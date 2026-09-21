#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCENARIO="${SCENARIO:-}"
RUN_MODE="${RUN_MODE:-smoke}"
MOSDNS_BINARY="${MOSDNS_BINARY:-}"
RESULT_DIR="${RESULT_DIR:-}"
HELPER_BINARY="${HELPER_BINARY:-}"
OFFERED_QPS="${OFFERED_QPS:-}"
MANIFEST_SHA256="${MANIFEST_SHA256:-}"
SUT_CPU_SET="${SUT_CPU_SET:-}"
HARNESS_CPU_SET="${HARNESS_CPU_SET:-}"

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

stop_sut() {
  if [[ -n "${SUT_PID}" ]] && kill -0 "${SUT_PID}" 2>/dev/null; then
    kill -TERM "${SUT_PID}" 2>/dev/null || true
    wait "${SUT_PID}" 2>/dev/null || true
  fi
  SUT_PID=""
}

stop_fixtures() {
  local pid
  for pid in "${FIXTURE_PIDS[@]:-}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      kill -TERM "${pid}" 2>/dev/null || true
    fi
  done
  for pid in "${FIXTURE_PIDS[@]:-}"; do
    if [[ -n "${pid}" ]]; then
      wait "${pid}" 2>/dev/null || true
    fi
  done
  FIXTURE_PIDS=()
}

cleanup() {
  set +e
  stop_sut
  stop_fixtures
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
  if [[ -z "${OFFERED_QPS}" || -z "${MANIFEST_SHA256}" ]]; then
    echo "official mode requires OFFERED_QPS and MANIFEST_SHA256" >&2
    exit 2
  fi
  actual_manifest_sha256="$(sha256sum "${MANIFEST}" | awk '{print $1}')"
  if [[ "${actual_manifest_sha256}" != "${MANIFEST_SHA256}" ]]; then
    echo "manifest SHA-256 mismatch: expected ${MANIFEST_SHA256}, got ${actual_manifest_sha256}" >&2
    exit 2
  fi
  printf '%s  %s\n' "${actual_manifest_sha256}" "${MANIFEST}" > "${RESULT_DIR}/manifest.sha256"
fi

"${HELPER_BINARY}" validate-binary --path "${MOSDNS_BINARY}" > "${RESULT_DIR}/sut.json"
for spec in "${FIXTURE_SPECS[@]}"; do
  IFS='|' read -r network addr upstream counter <<<"${spec}"
  "${HELPER_BINARY}" fixture --network "${network}" --addr "${addr}" --upstream-id "${upstream}" --counter "${counter}" > "${TMP_DIR}/${upstream}.stdout" 2> "${TMP_DIR}/${upstream}.stderr" &
  FIXTURE_PIDS+=("$!")
done

wait_for_counter_file() {
  local counter_path="$1"
  for _ in $(seq 1 100); do
    if [[ -s "${counter_path}" ]]; then
      return 0
    fi
    sleep 0.05
  done
  echo "counter did not become ready: ${counter_path}" >&2
  return 1
}

for spec in "${FIXTURE_SPECS[@]}"; do
  IFS='|' read -r _ _ _ counter <<<"${spec}"
  wait_for_counter_file "${counter}"
done

start_sut() {
  if [[ -n "${SUT_CPU_SET}" ]]; then
    taskset --cpu-list "${SUT_CPU_SET}" "${MOSDNS_BINARY}" start -c "${SCENARIO_CONFIG}" >> "${TMP_DIR}/mosdns.stdout" 2>> "${TMP_DIR}/mosdns.stderr" &
  else
    "${MOSDNS_BINARY}" start -c "${SCENARIO_CONFIG}" >> "${TMP_DIR}/mosdns.stdout" 2>> "${TMP_DIR}/mosdns.stderr" &
  fi
  SUT_PID="$!"
  # Startup is outside every measured stage. Probe the TCP listener as a
  # bounded readiness barrier when the scenario uses TCP. UDP has no
  # connectable readiness signal, so use a bounded startup margin there.
  sut_port="${SUT_ADDR##*:}"
  if [[ "${TRANSPORT}" != "tcp" ]]; then
    sleep 1
    if ! kill -0 "${SUT_PID}" 2>/dev/null; then
      echo "SUT exited during startup" >&2
      return 1
    fi
    return 0
  fi
  for _ in $(seq 1 100); do
    if kill -0 "${SUT_PID}" 2>/dev/null && (echo >/dev/tcp/127.0.0.1/"${sut_port}") 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  if ! kill -0 "${SUT_PID}" 2>/dev/null; then
    echo "SUT exited during startup" >&2
  else
    echo "SUT did not become ready during startup" >&2
  fi
  return 1
}

copy_fixture_counters() {
  local spec upstream counter
  for spec in "${FIXTURE_SPECS[@]}"; do
    IFS='|' read -r _ _ upstream counter <<<"${spec}"
    if [[ ! -f "${counter}" ]]; then
      echo "missing final fixture counter for ${upstream}" >&2
      return 1
    fi
    cp "${counter}" "${RESULT_DIR}/fixture-${upstream}.json"
  done
}

verify_counter_delta() {
  local counter_path="$1"
  local baseline_path="$2"
  local stage_result_path="${3:-}"
  local stage_name="${4:-}"
  local verify_args=(verify-counters --scenario w2 --workload "${WORKLOAD}" --counter "${counter_path}" --baseline "${baseline_path}" --expect-delta)
  if [[ -n "${stage_result_path}" ]]; then
    verify_args+=(--stage-result "${stage_result_path}" --stage "${stage_name}")
  fi
  for _ in $(seq 1 50); do
    if "${HELPER_BINARY}" "${verify_args[@]}"; then
      return 0
    fi
    sleep 0.1
  done
  "${HELPER_BINARY}" "${verify_args[@]}"
}

duration="1s"
qps="20"
if [[ "${RUN_MODE}" == "pilot" ]]; then
  duration="5s"
  qps="100"
fi
if [[ "${RUN_MODE}" == "official" ]]; then
  duration="10s"
  qps="${OFFERED_QPS}"
elif [[ -n "${OFFERED_QPS}" ]]; then
  qps="${OFFERED_QPS}"
fi

workload_scenario="w3"
if [[ "${SCENARIO}" == w1-* ]]; then
  workload_scenario="w1"
elif [[ "${SCENARIO}" == w2 ]]; then
  workload_scenario="w2"
fi

ONE_PASS_ARGS=()
if [[ "${RUN_MODE}" == "smoke" ]]; then
  ONE_PASS_ARGS+=(--one-pass)
fi

start_sut
if [[ "${SCENARIO}" == w2 ]]; then
  IFS='|' read -r _ _ _ CACHE_COUNTER <<<"${FIXTURE_SPECS[0]}"
  cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-cold-before-counter.json"
  "${HELPER_BINARY}" run --workload "${WORKLOAD}" --scenario w2 --transport udp --addr "${SUT_ADDR}" \
    --stage "${RUN_MODE}-w2-cold" --qps "${qps}" --duration "${duration}" --deadline 500ms --late-drain 100ms \
    "${ONE_PASS_ARGS[@]}" --fail-on-error --result "${RESULT_DIR}" --sut-pid "${SUT_PID}"
  stop_sut
  verify_counter_delta "${CACHE_COUNTER}" "${TMP_DIR}/w2-cold-before-counter.json"
  cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-cold-after-counter.json"

  start_sut
  cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-before-counter.json"
  PREFILL_DIR="${TMP_DIR}/prefill"
  "${HELPER_BINARY}" run --workload "${WORKLOAD}" --scenario w2 --transport udp --addr "${SUT_ADDR}" \
    --stage warm-prefill --qps "${qps}" --duration 1s --deadline 500ms --late-drain 100ms \
    --one-pass --fail-on-error --result "${PREFILL_DIR}" --sut-pid "${SUT_PID}"
  verify_counter_delta "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-before-counter.json" "${PREFILL_DIR}/stages.jsonl" warm-prefill
  cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-counter.json"
  "${HELPER_BINARY}" run --workload "${WORKLOAD}" --scenario w2 --transport udp --addr "${SUT_ADDR}" \
    --stage "${RUN_MODE}-w2-warm" --qps "${qps}" --duration "${duration}" --deadline 500ms --late-drain 100ms \
    --one-pass --fail-on-error --result "${RESULT_DIR}" --sut-pid "${SUT_PID}"
else
  "${HELPER_BINARY}" run --workload "${WORKLOAD}" --scenario "${workload_scenario}" --transport "${TRANSPORT}" \
    --addr "${SUT_ADDR}" --stage "${RUN_MODE}-${SCENARIO}" --qps "${qps}" --duration "${duration}" \
    --deadline 500ms --late-drain 100ms --fail-on-error --result "${RESULT_DIR}" --sut-pid "${SUT_PID}"
fi

stop_sut
stop_fixtures
copy_fixture_counters

if [[ "${SCENARIO}" == w2 ]]; then
  "${HELPER_BINARY}" verify-counters --scenario w2 --workload "${WORKLOAD}" --counter "${RESULT_DIR}/fixture-cache.json" --baseline "${TMP_DIR}/w2-prefill-counter.json"
elif [[ "${SCENARIO}" == w3 ]]; then
  "${HELPER_BINARY}" verify-counters --scenario w3 --workload "${WORKLOAD}" \
    --route-a "${RESULT_DIR}/fixture-route-a.json" --route-b "${RESULT_DIR}/fixture-route-b.json" --route-c "${RESULT_DIR}/fixture-route-c.json"
else
  "${HELPER_BINARY}" verify-counters --scenario w1 --workload "${WORKLOAD}" --counter "${RESULT_DIR}/fixture-forward.json"
fi

cp "${TMP_DIR}/mosdns.stdout" "${RESULT_DIR}/sut.stdout.log"
cp "${TMP_DIR}/mosdns.stderr" "${RESULT_DIR}/sut.stderr.log"
printf '%s\n' "scenario=${SCENARIO}" "run_mode=${RUN_MODE}" "config=${SCENARIO_CONFIG}" "workload=${WORKLOAD}" > "${RESULT_DIR}/run-metadata.txt"
printf '%s\n' "offered_qps=${qps}" "sut_cpu_set=${SUT_CPU_SET}" "harness_cpu_set=${HARNESS_CPU_SET}" >> "${RESULT_DIR}/run-metadata.txt"
