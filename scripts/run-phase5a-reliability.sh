#!/usr/bin/env bash
set -euo pipefail

# Slice 1 only: bounded loopback reliability helper. This runner never starts
# MosDNS, connects over SSH, launches a pilot, or enables an official profile.
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKLOAD="${WORKLOAD:-}"
SCENARIO="${SCENARIO:-}"
TRANSPORT="${TRANSPORT:-udp}"
ADDR="${ADDR:-127.0.0.1:15353}"
RESULT_DIR="${RESULT_DIR:-}"
HELPER_BINARY="${HELPER_BINARY:-}"
SLOTS="${SLOTS:-1}"
TARGET_QPS="${TARGET_QPS:-1}"
REQUEST_DEADLINE_MS="${REQUEST_DEADLINE_MS:-500}"
LATE_DRAIN_MS="${LATE_DRAIN_MS:-100}"
WORKERS="${WORKERS:-1}"
IN_FLIGHT="${IN_FLIGHT:-1}"
DISPATCH_QUEUE="${DISPATCH_QUEUE:-8}"
EVIDENCE_QUEUE="${EVIDENCE_QUEUE:-8}"
RECORD_BYTES="${RECORD_BYTES:-65536}"
CLEANUP_TIMEOUT_MS="${CLEANUP_TIMEOUT_MS:-1000}"
RUN_ID="${RUN_ID:-reliability-${SCENARIO:-unset}-$(date -u +%Y%m%dT%H%M%SZ)-$$}"

if [[ -z "${WORKLOAD}" || -z "${SCENARIO}" || -z "${RESULT_DIR}" ]]; then
  echo "WORKLOAD, SCENARIO, and RESULT_DIR are required" >&2
  exit 2
fi
case "${SCENARIO}" in
  w1|w2|w3) ;;
  *) echo "unsupported SCENARIO: ${SCENARIO}" >&2; exit 2 ;;
esac
case "${TRANSPORT}" in
  udp|tcp) ;;
  *) echo "unsupported TRANSPORT: ${TRANSPORT}" >&2; exit 2 ;;
esac

mkdir -p "${RESULT_DIR}"
RESULT_DIR="$(cd "${RESULT_DIR}" && pwd)"
if [[ -n "$(find "${RESULT_DIR}" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
  echo "RESULT_DIR must be fresh and empty: ${RESULT_DIR}" >&2
  exit 2
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/phase5a-reliability.XXXXXX")"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT INT TERM

if [[ -z "${HELPER_BINARY}" ]]; then
  HELPER_BINARY="${TMP_DIR}/phase5a-baseline-helper"
  (cd "${ROOT_DIR}" && go build -trimpath -o "${HELPER_BINARY}" ./tests/phase5a-baseline/cmd/phase5a-baseline)
fi
if [[ ! -x "${HELPER_BINARY}" ]]; then
  echo "HELPER_BINARY is not executable: ${HELPER_BINARY}" >&2
  exit 2
fi

exec "${HELPER_BINARY}" reliability-run \
  --workload "${WORKLOAD}" \
  --scenario "${SCENARIO}" \
  --transport "${TRANSPORT}" \
  --addr "${ADDR}" \
  --result "${RESULT_DIR}" \
  --run-id "${RUN_ID}" \
  --slots "${SLOTS}" \
  --target-qps "${TARGET_QPS}" \
  --request-deadline-ms "${REQUEST_DEADLINE_MS}" \
  --late-drain-ms "${LATE_DRAIN_MS}" \
  --workers "${WORKERS}" \
  --in-flight "${IN_FLIGHT}" \
  --dispatch-queue "${DISPATCH_QUEUE}" \
  --evidence-queue "${EVIDENCE_QUEUE}" \
  --record-bytes "${RECORD_BYTES}" \
  --cleanup-timeout-ms "${CLEANUP_TIMEOUT_MS}"
