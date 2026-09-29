#!/usr/bin/env bash
# Bounded real-process HTTP + DNS functional proof on the isolated mosdns-rust
# VM. Runs entirely on loopback with probed free ports, owns only its own
# temporary directory and process, and never touches the live service or
# /cus/mosdns.
#
# Usage: remote-functional.sh <path-to-mosdns-binary> <work-dir>
set -euo pipefail

BIN="${1:?usage: remote-functional.sh <binary> <work-dir>}"
WORK="${2:?usage: remote-functional.sh <binary> <work-dir>}"

pass=0
fail=0
check() {
  local name="$1" ok="$2" detail="${3:-}"
  if [ "$ok" = "1" ]; then
    pass=$((pass + 1))
    printf '[PASS] %s %s\n' "$name" "$detail"
  else
    fail=$((fail + 1))
    printf '[FAIL] %s %s\n' "$name" "$detail"
  fi
}

free_port() {
  python3 - "$1" <<'PY'
import socket, sys
kind = sys.argv[1]
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if kind == 'udp' else socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', 0))
print(s.getsockname()[1])
s.close()
PY
}

# A closed TCP session leaves TIME_WAIT on the local port. Like the product
# listener (Tokio sets SO_REUSEADDR), the probe must set SO_REUSEADDR so closed
# sessions permit the bind while a live listener still refuses it.
port_free() {
  python3 - "$1" "$2" <<'PY'
import socket, sys
port = int(sys.argv[1]); kind = sys.argv[2]
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if kind == 'udp' else socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    s.bind(('127.0.0.1', port))
    print('1')
except OSError:
    print('0')
finally:
    s.close()
PY
}

# The primary release assertion: no process is listening on the port any more.
listener_absent() {
  local port="$1" proto="$2"
  if [ "$proto" = "udp" ]; then
    [ -z "$(ss -lnuH "sport = :$port" 2>/dev/null)" ] && echo 1 || echo 0
  else
    [ -z "$(ss -lntH "sport = :$port" 2>/dev/null)" ] && echo 1 || echo 0
  fi
}

dns_rcode() {
  python3 - "$1" "$2" "$3" <<'PY'
import socket, sys
port = int(sys.argv[1]); labels = sys.argv[2].split(','); ident = int(sys.argv[3])
packet = bytearray(b'\x00\x00\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00')
packet[0] = (ident >> 8) & 0xFF
packet[1] = ident & 0xFF
for label in labels:
    packet.append(len(label))
    packet += label.encode()
packet += b'\x00\x00\x01\x00\x01'
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(5)
s.sendto(bytes(packet), ('127.0.0.1', port))
data, _ = s.recvfrom(65535)
s.close()
print(data[3] & 0x0F)
PY
}

body_of() {
  curl -s -o "$WORK/body" -w '%{http_code} %{content_type}' "$@"
}

start_host() {
  "$BIN" start -c "$WORK/config.yaml" >"$WORK/native-$1.log" 2>&1 &
  NATIVE_PID=$!
}

wait_api() {
  for _ in $(seq 1 100); do
    if curl -s -m 1 -o /dev/null "http://127.0.0.1:$API_PORT/api/v1/special-groups"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

rm -rf -- "$WORK"
mkdir -p "$WORK/rules"

DNS_PORT="$(free_port udp)"
API_PORT="$(free_port tcp)"
printf 'seed-blocked.example\n' >"$WORK/rules/blocklist.txt"

sed -e "s/__API_PORT__/$API_PORT/" -e "s/__DNS_PORT__/$DNS_PORT/" \
  >"$WORK/config.yaml" <<'YAML'
log:
  level: error
api:
  http: "127.0.0.1:__API_PORT__"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $blocklist
        exec: reject 3
      - exec: reject 0
  - tag: blocklist
    type: domain_set
    args:
      files:
        - rules/blocklist.txt
  - tag: explist
    type: domain_set
    args:
      exps:
        - exps.example
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:__DNS_PORT__"
      enable_audit: false
YAML

printf '== isolated ports: dns=%s api=%s\n' "$DNS_PORT" "$API_PORT"
printf '== native log: %s\n' "$WORK/native-first.log"

NATIVE_PID=""
trap '[ -n "$NATIVE_PID" ] && kill "$NATIVE_PID" 2>/dev/null || true' EXIT
start_host first
if ! wait_api; then
  check "the native host serves its scoped API" 0 "no answer on 127.0.0.1:$API_PORT"
  cat "$WORK/native-first.log"
  exit 1
fi
check "the native host serves its scoped API" 1 "127.0.0.1:$API_PORT"

status="$(body_of "http://127.0.0.1:$API_PORT/api/v1/special-groups")"
check "GET /api/v1/special-groups" "$([ "$status" = "200 application/json" ] && [ "$(cat "$WORK/body")" = "[]" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"

status="$(body_of "http://127.0.0.1:$API_PORT/plugins/blocklist/show?limit=10000")"
check "GET /show ignores the query string" "$([ "$status" = "200 text/plain; charset=utf-8" ] && [ "$(cat "$WORK/body")" = "seed-blocked.example" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"

status="$(body_of "http://127.0.0.1:$API_PORT/plugins/absent/show")"
check "unknown tag fails explicitly" "$([ "$status" = "404 text/plain; charset=utf-8" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"

status="$(body_of "http://127.0.0.1:$API_PORT/plugins/explist/show")"
check "ineligible query-only tag is rejected" "$([ "${status%% *}" = "400" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"

status="$(body_of -X POST --data '' "http://127.0.0.1:$API_PORT/plugins/blocklist/show")"
check "wrong method is 405" "$([ "${status%% *}" = "405" ] && echo 1 || echo 0)" "status=${status%% *}"

status="$(body_of -X POST -H 'Content-Type: application/json' --data '{' "http://127.0.0.1:$API_PORT/plugins/blocklist/post")"
check "malformed JSON is 400 invalid JSON" "$([ "${status%% *}" = "400" ] && [ "$(cat "$WORK/body")" = "invalid JSON" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"
check "a malformed POST leaves the file unchanged" "$([ "$(cat "$WORK/rules/blocklist.txt")" = "seed-blocked.example" ] && echo 1 || echo 0)" "$(cat "$WORK/rules/blocklist.txt")"

status="$(body_of -X POST -H 'Content-Type: application/json' --data '{"values":["vm-added.example","regexp:["]}' "http://127.0.0.1:$API_PORT/plugins/blocklist/post")"
check "POST publishes the accepted rules and reports the count" "$([ "${status%% *}" = "200" ] && [ "$(cat "$WORK/body")" = "domain_set replaced with 1 entries" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"
check "the committed file holds only accepted rules" "$([ "$(cat "$WORK/rules/blocklist.txt")" = "vm-added.example" ] && echo 1 || echo 0)" "$(cat "$WORK/rules/blocklist.txt")"

status="$(body_of "http://127.0.0.1:$API_PORT/plugins/blocklist/show")"
check "GET /show reflects the new generation" "$([ "$(cat "$WORK/body")" = "vm-added.example" ] && echo 1 || echo 0)" "$(cat "$WORK/body")"

status="$(body_of "http://127.0.0.1:$API_PORT/plugins/blocklist/save")"
check "GET /save returns an empty 200" "$([ "${status%% *}" = "200" ] && [ ! -s "$WORK/body" ] && echo 1 || echo 0)" "$status body=$(cat "$WORK/body")"

before_rcode="$(dns_rcode "$DNS_PORT" vm-added,example 4660)"
other_rcode="$(dns_rcode "$DNS_PORT" other,example 4661)"
check "the next real DNS query reflects the publication" "$([ "$before_rcode" = "3" ] && echo 1 || echo 0)" "vm-added.example rcode=$before_rcode"
check "an unrelated name is still not blocked" "$([ "$other_rcode" = "0" ] && echo 1 || echo 0)" "other.example rcode=$other_rcode"

kill "$NATIVE_PID"
wait "$NATIVE_PID" 2>/dev/null || true
NATIVE_PID=""
sleep 0.5

start_host restart
if ! wait_api; then
  check "the restarted host serves the committed generation" 0 "no answer on 127.0.0.1:$API_PORT"
  cat "$WORK/native-restart.log"
  exit 1
fi
status="$(body_of "http://127.0.0.1:$API_PORT/plugins/blocklist/show")"
check "a restart keeps the committed generation" "$([ "$(cat "$WORK/body")" = "vm-added.example" ] && echo 1 || echo 0)" "$(cat "$WORK/body")"
after_rcode="$(dns_rcode "$DNS_PORT" vm-added,example 4662)"
check "a restart keeps the DNS effect" "$([ "$after_rcode" = "3" ] && echo 1 || echo 0)" "vm-added.example rcode=$after_rcode"

kill "$NATIVE_PID"
wait "$NATIVE_PID" 2>/dev/null || true
NATIVE_PID=""
sleep 0.5

check "no listener remains on the management port" "$(listener_absent "$API_PORT" tcp)" "port $API_PORT"
check "no listener remains on the DNS port" "$(listener_absent "$DNS_PORT" udp)" "port $DNS_PORT"
check "the management port is bindable again (SO_REUSEADDR)" "$(port_free "$API_PORT" tcp)" "port $API_PORT"
check "the DNS port is bindable again" "$(port_free "$DNS_PORT" udp)" "port $DNS_PORT"

printf '\nfunctional checks: %s passed, %s failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
