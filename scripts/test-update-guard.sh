#!/usr/bin/env bash
set -euo pipefail

BINARY="${1:?path to a v0.7.0-or-newer mosdns binary is required}"
TARGET_VERSION="$("${BINARY}" version)"
PIDS=()

wait_for_health() {
  local url="$1"
  for _ in $(seq 1 60); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

json_field() {
  python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"
}

prepare_runtime() {
  local root="$1"
  local port="$2"
  mkdir -p "${root}/webinfo" "${root}/update/txn"
  cp "${BINARY}" "${root}/mosdns"
  cp "${BINARY}" "${root}/update/txn/mosdns.previous"
  chmod 755 "${root}/mosdns" "${root}/update/txn/mosdns.previous"
  cat >"${root}/config.yaml" <<EOF
log:
  level: error
api:
  http: "127.0.0.1:${port}"
plugins: []
EOF
  cat >"${root}/webinfo/config_update_state.json" <<'EOF'
{
  "format": 1,
  "applied_schema": 3,
  "required_schema": 3,
  "status": "success",
  "package_id": "main-config-schema-3"
}
EOF
}

run_success_case() {
  local root="$1"
  local port=19091
  local health="http://127.0.0.1:${port}/api/v1/system/health"
  prepare_runtime "${root}" "${port}"
  cp "${BINARY}" "${root}/update/txn/mosdns.candidate"
  chmod 755 "${root}/update/txn/mosdns.candidate"
  python3 - "${root}" "${health}" "${TARGET_VERSION}" <<'PY' >"${root}/update/txn/transaction.json"
import json, sys
root, health, target_version = sys.argv[1:]
binary = root + "/mosdns"
json.dump({
    "format": 1,
    "status": "staged",
    "target_version": target_version,
    "target_signature": "guard-success-test",
    "required_config_schema": 3,
    "config_package_id": "main-config-schema-3",
    "config_base_dir": root,
    "config_path": root + "/config.yaml",
    "executable_path": binary,
    "candidate_path": root + "/update/txn/mosdns.candidate",
    "previous_binary_path": root + "/update/txn/mosdns.previous",
    "health_url": health,
    "original_args": [binary, "start", "-d", root, "-c", root + "/config.yaml"],
    "created_at": "2026-07-12T00:00:00+08:00",
}, sys.stdout)
PY

  "${root}/update/txn/mosdns.previous" update-guard --transaction "${root}/update/txn/transaction.json" &
  local pid=$!
  PIDS+=("${pid}")
  wait_for_health "${health}"
  for _ in $(seq 1 80); do
    if [ ! -e "${root}/update/txn/transaction.json" ]; then
      break
    fi
    sleep 0.25
  done
  test ! -e "${root}/update/txn/transaction.json"
  test "$(curl -fsS "${health}" | json_field version)" = "${TARGET_VERSION}"
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
}

run_rollback_case() {
  local root="$1"
  local port=19092
  local health="http://127.0.0.1:${port}/api/v1/system/health"
  prepare_runtime "${root}" "${port}"
  cat >"${root}/update/txn/mosdns.candidate" <<'EOF'
#!/bin/sh
exit 42
EOF
  chmod 755 "${root}/update/txn/mosdns.candidate"
  python3 - "${root}" "${health}" <<'PY' >"${root}/update/txn/transaction.json"
import json, sys
root, health = sys.argv[1:]
binary = root + "/mosdns"
json.dump({
    "format": 1,
    "status": "staged",
    "target_version": "v9.9.9",
    "target_signature": "guard-rollback-test",
    "required_config_schema": 3,
    "config_package_id": "main-config-schema-3",
    "config_base_dir": root,
    "config_path": root + "/config.yaml",
    "executable_path": binary,
    "candidate_path": root + "/update/txn/mosdns.candidate",
    "previous_binary_path": root + "/update/txn/mosdns.previous",
    "health_url": health,
    "original_args": [binary, "start", "-d", root, "-c", root + "/config.yaml"],
    "created_at": "2026-07-12T00:00:00+08:00",
}, sys.stdout)
PY

  "${root}/update/txn/mosdns.previous" update-guard --transaction "${root}/update/txn/transaction.json" &
  local pid=$!
  PIDS+=("${pid}")
  wait_for_health "${health}"
  for _ in $(seq 1 80); do
    if [ "$(json_field status <"${root}/update/txn/transaction.json" 2>/dev/null || true)" = "rolled_back" ]; then
      break
    fi
    sleep 0.25
  done
  test "$(curl -fsS "${health}" | json_field version)" = "${TARGET_VERSION}"
  test "$(json_field status <"${root}/update/txn/transaction.json")" = "rolled_back"
  test "$("${root}/mosdns" version)" = "${TARGET_VERSION}"
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
}

SUCCESS_ROOT="$(mktemp -d)"
ROLLBACK_ROOT="$(mktemp -d)"
cleanup() {
  for pid in "${PIDS[@]}"; do
    kill "${pid}" 2>/dev/null || true
  done
  rm -rf "${SUCCESS_ROOT}" "${ROLLBACK_ROOT}"
}
trap cleanup EXIT

run_success_case "${SUCCESS_ROOT}"
run_rollback_case "${ROLLBACK_ROOT}"
echo "update guard success and rollback smoke tests passed"
