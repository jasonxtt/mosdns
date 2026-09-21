#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCENARIO="${SCENARIO:-}"
RUN_MODE="${RUN_MODE:-smoke}"
MOSDNS_BINARY="${MOSDNS_BINARY:-}"
RESULT_DIR="${RESULT_DIR:-}"
HELPER_BINARY="${HELPER_BINARY:-}"

if [[ -z "${MOSDNS_BINARY}" || -z "${SCENARIO}" ]]; then
  echo "MOSDNS_BINARY and SCENARIO are required" >&2
  exit 2
fi
if [[ ! -x "${MOSDNS_BINARY}" ]]; then
  echo "MOSDNS_BINARY is not executable: ${MOSDNS_BINARY}" >&2
  exit 2
fi
case "${SCENARIO}" in
  w1-udp|w1-tcp|w2|w3) ;;
  *) echo "unsupported SCENARIO: ${SCENARIO}" >&2; exit 2 ;;
esac
case "${RUN_MODE}" in
  smoke|pilot|official) ;;
  *) echo "unsupported RUN_MODE: ${RUN_MODE}" >&2; exit 2 ;;
esac

if [[ -z "${RESULT_DIR}" ]]; then
  RESULT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/phase5a-baseline.XXXXXX")"
else
  mkdir -p "${RESULT_DIR}"
fi
RESULT_DIR="$(cd "${RESULT_DIR}" && pwd)"
TMP_DIR="$(mktemp -d "${RESULT_DIR}/.run.XXXXXX")"
CONFIG_DIR="${ROOT_DIR}/tests/phase5a-baseline/configs"
WORKLOAD_DIR="${ROOT_DIR}/tests/phase5a-baseline/workloads"
HELPER_TMP="${TMP_DIR}/phase5a-baseline-helper"
SUT_PID=""
FIXTURE_PIDS=()

cleanup() {
  set +e
  if [[ -n "${SUT_PID}" ]] && kill -0 "${SUT_PID}" 2>/dev/null; then
    kill -TERM "${SUT_PID}" 2>/dev/null
    wait "${SUT_PID}" 2>/dev/null
  fi
  for pid in "${FIXTURE_PIDS[@]:-}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      kill -TERM "${pid}" 2>/dev/null
      wait "${pid}" 2>/dev/null
    fi
  done
}
trap cleanup EXIT INT TERM

if [[ -z "${HELPER_BINARY}" ]]; then
  (cd "${ROOT_DIR}" && go build -trimpath -o "${HELPER_TMP}" ./tests/phase5a-baseline/cmd/phase5a-baseline)
  HELPER_BINARY="${HELPER_TMP}"
fi
if [[ ! -x "${HELPER_BINARY}" ]]; then
  echo "HELPER_BINARY is not executable: ${HELPER_BINARY}" >&2
  exit 2
fi

SCENARIO_CONFIG=""
SUT_ADDR=""
TRANSPORT="udp"
WORKLOAD=""
FIXTURE_SPECS=()
case "${SCENARIO}" in
  w1-udp)
    SCENARIO_CONFIG="${CONFIG_DIR}/forward-udp.yaml"
    SUT_ADDR="127.0.0.1:15353"
    TRANSPORT="udp"
    WORKLOAD="${WORKLOAD_DIR}/forward.jsonl"
    FIXTURE_SPECS=("udp|127.0.0.1:15453|forward|${TMP_DIR}/forward.json")
    ;;
  w1-tcp)
    SCENARIO_CONFIG="${CONFIG_DIR}/forward-tcp.yaml"
    SUT_ADDR="127.0.0.1:15354"
    TRANSPORT="tcp"
    WORKLOAD="${WORKLOAD_DIR}/forward.jsonl"
    FIXTURE_SPECS=("tcp|127.0.0.1:15454|forward|${TMP_DIR}/forward.json")
    ;;
  w2)
    SCENARIO_CONFIG="${CONFIG_DIR}/cache.yaml"
    SUT_ADDR="127.0.0.1:15355"
    TRANSPORT="udp"
    WORKLOAD="${WORKLOAD_DIR}/cache.jsonl"
    FIXTURE_SPECS=("udp|127.0.0.1:15455|cache|${TMP_DIR}/cache.json")
    ;;
  w3)
    SCENARIO_CONFIG="${CONFIG_DIR}/routing.yaml"
    SUT_ADDR="127.0.0.1:15356"
    TRANSPORT="udp"
    WORKLOAD="${WORKLOAD_DIR}/routing.jsonl"
    FIXTURE_SPECS=(
      "udp|127.0.0.1:15456|route-a|${TMP_DIR}/route-a.json"
      "udp|127.0.0.1:15457|route-b|${TMP_DIR}/route-b.json"
      "udp|127.0.0.1:15458|route-c|${TMP_DIR}/route-c.json"
    )
    ;;
esac

"${HELPER_BINARY}" validate-binary --path "${MOSDNS_BINARY}" > "${RESULT_DIR}/sut.json"
for spec in "${FIXTURE_SPECS[@]}"; do
  IFS='|' read -r network addr upstream counter <<<"${spec}"
  "${HELPER_BINARY}" fixture --network "${network}" --addr "${addr}" --upstream-id "${upstream}" --counter "${counter}" > "${TMP_DIR}/${upstream}.stdout" 2> "${TMP_DIR}/${upstream}.stderr" &
  FIXTURE_PIDS+=("$!")
done

if [[ "${RUN_MODE}" == "official" ]]; then
  MANIFEST="${ROOT_DIR}/.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json"
  if [[ ! -f "${MANIFEST}" ]]; then
    echo "official mode requires run-manifest.json" >&2
    exit 2
  fi
  if ! rg -q '"official_frozen"[[:space:]]*:[[:space:]]*true' "${MANIFEST}"; then
    echo "official mode requires an immutable official_frozen manifest" >&2
    exit 2
  fi
fi

"${MOSDNS_BINARY}" start -c "${SCENARIO_CONFIG}" > "${TMP_DIR}/mosdns.stdout" 2> "${TMP_DIR}/mosdns.stderr" &
SUT_PID="$!"

sut_port="${SUT_ADDR##*:}"
for _ in $(seq 1 100); do
  if kill -0 "${SUT_PID}" 2>/dev/null && (echo >/dev/tcp/127.0.0.1/"${sut_port}") 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if ! kill -0 "${SUT_PID}" 2>/dev/null; then
  echo "SUT exited during startup" >&2
  exit 1
fi

duration="1s"
qps="20"
if [[ "${RUN_MODE}" == "pilot" ]]; then
  duration="5s"
  qps="100"
fi
if [[ "${RUN_MODE}" == "official" ]]; then
  duration="10s"
  qps="100"
fi

workload_scenario="w3"
if [[ "${SCENARIO}" == w1-* ]]; then
  workload_scenario="w1"
elif [[ "${SCENARIO}" == w2 ]]; then
  workload_scenario="w2"
fi

"${HELPER_BINARY}" run \
  --workload "${WORKLOAD}" \
  --scenario "${workload_scenario}" \
  --transport "${TRANSPORT}" \
  --addr "${SUT_ADDR}" \
  --stage "${RUN_MODE}-${SCENARIO}" \
  --qps "${qps}" \
  --duration "${duration}" \
  --deadline 500ms \
  --late-drain 100ms \
  --result "${RESULT_DIR}" \
  --sut-pid "${SUT_PID}"

cp "${TMP_DIR}/mosdns.stdout" "${RESULT_DIR}/sut.stdout.log"
cp "${TMP_DIR}/mosdns.stderr" "${RESULT_DIR}/sut.stderr.log"
for spec in "${FIXTURE_SPECS[@]}"; do
  IFS='|' read -r _ _ upstream counter <<<"${spec}"
  [[ -f "${counter}" ]] && cp "${counter}" "${RESULT_DIR}/fixture-${upstream}.json"
done
printf '%s\n' "scenario=${SCENARIO}" "run_mode=${RUN_MODE}" "config=${SCENARIO_CONFIG}" "workload=${WORKLOAD}" "sut_pid=${SUT_PID}" > "${RESULT_DIR}/run-metadata.txt"
