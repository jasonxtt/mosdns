#!/usr/bin/env bash
set -euo pipefail

BINARY=${MOSDNS_RUST_BINARY:?set MOSDNS_RUST_BINARY to the experimental Linux binary}
RUST_BINARY="${BINARY}"
GO_ONLY_BINARY=${MOSDNS_GO_ONLY_BINARY:-}

if [[ ! -x "${BINARY}" ]]; then
	echo "experimental binary is not executable: ${BINARY}" >&2
	exit 1
fi
if [[ -n "${GO_ONLY_BINARY}" && ! -x "${GO_ONLY_BINARY}" ]]; then
	echo "Go-only fallback binary is not executable: ${GO_ONLY_BINARY}" >&2
	exit 1
fi
for command_name in curl dig python3; do
	if ! command -v "${command_name}" >/dev/null 2>&1; then
		echo "missing required command: ${command_name}" >&2
		exit 1
	fi
done

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/mosdns-rust-matcher-smoke.XXXXXX")
PID=""

cleanup_server() {
	if [[ -n "${PID}" ]] && kill -0 "${PID}" >/dev/null 2>&1; then
		kill -TERM "${PID}" >/dev/null 2>&1 || true
		for _ in $(seq 1 50); do
			if ! kill -0 "${PID}" >/dev/null 2>&1; then
				break
			fi
			sleep 0.1
		done
		kill -KILL "${PID}" >/dev/null 2>&1 || true
		wait "${PID}" >/dev/null 2>&1 || true
	fi
	PID=""
}

cleanup() {
	status=$?
	cleanup_server
	rm -rf -- "${ROOT}"
	echo "mos-test matcher smoke cleanup complete"
	exit "${status}"
}
trap cleanup EXIT

read -r API_PORT DNS_PORT < <(python3 - <<'PY'
import socket

ports = []
for _ in range(2):
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    ports.append(str(sock.getsockname()[1]))
    sock.close()
print(*ports)
PY
)

DOMAIN_FILE="${ROOT}/domain.txt"
IP_FILE="${ROOT}/ip.txt"
MAPPER_BASE_FILE="${ROOT}/mapper-base.txt"
MAPPER_OVERLAP_FILE="${ROOT}/mapper-overlap.txt"
CONFIG="${ROOT}/mosdns.yaml"
LOG="${ROOT}/mosdns.log"
printf 'full:old.example\n' >"${DOMAIN_FILE}"
printf '127.0.0.0/8\n' >"${IP_FILE}"
printf 'domain:mapper.example\n' >"${MAPPER_BASE_FILE}"
printf 'keyword:overlap\n' >"${MAPPER_OVERLAP_FILE}"
cat >"${CONFIG}" <<EOF
log:
  level: warn

api:
  http: "127.0.0.1:${API_PORT}"

plugins:
  - tag: smoke_domain
    type: domain_set
    args:
      files:
        - "${DOMAIN_FILE}"

  - tag: smoke_ip
    type: ip_set
    args:
      files:
        - "${IP_FILE}"

  - tag: smoke_mapper_base
    type: domain_set
    args:
      files:
        - "${MAPPER_BASE_FILE}"

  - tag: smoke_mapper_overlap
    type: domain_set
    args:
      files:
        - "${MAPPER_OVERLAP_FILE}"

  - tag: smoke_mapper
    type: domain_mapper
    args:
      rules:
        - tag: smoke_mapper_base
          mark: 3
          output_tag: mapper-base
        - tag: smoke_mapper_overlap
          mark: 7
          output_tag: mapper-overlap

  - tag: smoke_sequence
    type: sequence
    args:
      - exec: "\$smoke_mapper"
      - matches:
          - fast_mark 3
          - fast_mark 7
        exec:
          - "black_hole 192.0.2.4"
          - "exit"
      - matches: fast_mark 3
        exec:
          - "black_hole 192.0.2.5"
          - "exit"
      - matches: fast_mark 7
        exec:
          - "black_hole 192.0.2.6"
          - "exit"
      - matches:
          - "qname \$smoke_domain"
        exec:
          - "black_hole 192.0.2.1"
          - "exit"
      - matches:
          - "qname full:ip-hit.example"
          - "client_ip \$smoke_ip"
        exec:
          - "black_hole 192.0.2.2"
          - "exit"
      - exec: "black_hole 192.0.2.3"

  - tag: smoke_udp
    type: udp_server
    args:
      entry: smoke_sequence
      enable_audit: true
      listen: "127.0.0.1:${DNS_PORT}"
EOF

API_URL="http://127.0.0.1:${API_PORT}"

start_server() {
	local backend=$1
	cleanup_server
	: >"${LOG}"
	MOSDNS_MATCHER_BACKEND="${backend}" "${BINARY}" start -c "${CONFIG}" >"${LOG}" 2>&1 &
	PID=$!
	for _ in $(seq 1 100); do
		if ! kill -0 "${PID}" >/dev/null 2>&1; then
			sed -n '1,160p' "${LOG}" >&2 || true
			return 1
		fi
		if curl --max-time 1 -fsS "${API_URL}/api/v1/system/health" >/dev/null 2>&1; then
			return 0
		fi
		sleep 0.1
	done
	sed -n '1,160p' "${LOG}" >&2 || true
	return 1
}

post_json() {
	local plugin=$1
	local body=$2
	curl --max-time 5 -fsS -X POST -H 'Content-Type: application/json' \
		-d "${body}" "${API_URL}/plugins/${plugin}/post" >/dev/null
}

expect_answer() {
	local name=$1
	local expected=$2
	local answer
	answer=$(dig +time=1 +tries=1 +short "@127.0.0.1" -p "${DNS_PORT}" "${name}" A | tr -d '\r' | sed '/^$/d' | head -n 1)
	if [[ "${answer}" != "${expected}" ]]; then
		echo "DNS answer mismatch for ${name}: got '${answer}', want '${expected}'" >&2
		return 1
	fi
}

audit_has_mapper_sources() {
	python3 - "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    logs = json.load(handle)
for log in logs:
    query_name = str(log.get("query_name", "")).rstrip(".").lower()
    sources = str(log.get("matched_rule_source", ""))
    if query_name == "overlap.mapper.example" and "smoke_mapper_base" in sources and "smoke_mapper_overlap" in sources:
        raise SystemExit(0)
raise SystemExit(1)
PY
}

start_server rust
expect_answer old.example. 192.0.2.1
expect_answer initial-miss.example. 192.0.2.3
expect_answer ip-hit.example. 192.0.2.2
expect_answer overlap.mapper.example. 192.0.2.4
expect_answer mapper.example. 192.0.2.5

curl --max-time 5 -fsS -X POST "${API_URL}/api/v1/audit/clear" >/dev/null
expect_answer overlap.mapper.example. 192.0.2.4
for _ in $(seq 1 30); do
	if curl --max-time 5 -fsS "${API_URL}/api/v1/audit/logs" >"${ROOT}/audit.json" && audit_has_mapper_sources "${ROOT}/audit.json"
	then
		break
	fi
	sleep 0.1
done
if ! audit_has_mapper_sources "${ROOT}/audit.json"
then
	echo "mapper source metadata was not observed in the audit log" >&2
	exit 1
fi

post_json smoke_domain '{"values":["full:new.example"]}'
expect_answer new.example. 192.0.2.1
expect_answer old.example. 192.0.2.3

post_json smoke_mapper_base '{"values":["domain:new-mapper.example"]}'
for _ in $(seq 1 30); do
	if expect_answer overlap.new-mapper.example. 192.0.2.4 2>/dev/null && expect_answer overlap.mapper.example. 192.0.2.6 2>/dev/null; then
		break
	fi
	sleep 0.1
done
expect_answer overlap.new-mapper.example. 192.0.2.4
expect_answer overlap.mapper.example. 192.0.2.6

post_json smoke_ip '{"values":[]}'
expect_answer ip-hit.example. 192.0.2.3
post_json smoke_ip '{"values":["127.0.0.1/32"]}'
expect_answer ip-hit.example. 192.0.2.2

invalid_status=$(curl --max-time 5 -sS -o "${ROOT}/invalid.body" -w '%{http_code}' \
	-X POST -H 'Content-Type: application/json' -d '{' "${API_URL}/plugins/smoke_domain/post")
if [[ "${invalid_status}" != "400" ]]; then
	echo "invalid reload status = ${invalid_status}, want 400" >&2
	exit 1
fi
expect_answer new.example. 192.0.2.1

if [[ -n "${GO_ONLY_BINARY}" ]]; then
	BINARY="${GO_ONLY_BINARY}"
	start_server rust
	expect_answer new.example. 192.0.2.1
	expect_answer ip-hit.example. 192.0.2.2
	expect_answer overlap.new-mapper.example. 192.0.2.4
	BINARY="${RUST_BINARY}"
fi

query_failures="${ROOT}/query_failures"
: >"${query_failures}"
(
	for _ in $(seq 1 80); do
		if ! expect_answer new.example. 192.0.2.1; then
			echo failure >>"${query_failures}"
		fi
	done
) &
QUERY_PID=$!
for i in $(seq 1 20); do
	post_json smoke_domain "{\"values\":[\"full:new.example\",\"full:alternate-${i}.example\"]}"
done
wait "${QUERY_PID}"
if [[ -s "${query_failures}" ]]; then
	echo "concurrent query/reload had $(wc -l <"${query_failures}") failed queries" >&2
	exit 1
fi

start_server go
expect_answer new.example. 192.0.2.1
expect_answer ip-hit.example. 192.0.2.2
expect_answer overlap.new-mapper.example. 192.0.2.4

start_server rust
expect_answer new.example. 192.0.2.1
expect_answer ip-hit.example. 192.0.2.2
expect_answer overlap.new-mapper.example. 192.0.2.4

if grep -Eq '(^|[^0-9])(:|[[:space:]])53([^0-9]|$)' "${CONFIG}"; then
	echo "smoke config unexpectedly references port 53" >&2
	exit 1
fi
if grep -Eqi 'panic|fatal error' "${LOG}"; then
	echo "smoke log contains panic/fatal error" >&2
	grep -Ein 'panic|fatal error' "${LOG}" >&2 || true
	exit 1
fi

echo "mos-test matcher smoke passed: backend=rust/go-only-fallback/go/rust, api=${API_PORT}, dns=${DNS_PORT}, root=${ROOT}"
