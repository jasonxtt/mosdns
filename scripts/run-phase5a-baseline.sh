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
MANIFEST_PATH="${MANIFEST_PATH:-}"
SUT_CPU_SET="${SUT_CPU_SET:-}"
HARNESS_CPU_SET="${HARNESS_CPU_SET:-}"
SUT_STARTUP_MARGIN="${SUT_STARTUP_MARGIN:-3}"
RUN_ID="${RUN_ID:-${SCENARIO}-${RUN_MODE}-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
FIXTURE_SESSION_ID="${FIXTURE_SESSION_ID:-${RUN_ID}-fixtures}"
CANDIDATE="${CANDIDATE:-}"
REPETITION="${REPETITION:-}"
PAIR_POSITION="${PAIR_POSITION:-}"
STAGE_DURATION_MS="${STAGE_DURATION_MS:-}"
NORMAL_REFERENCE_QPS="${NORMAL_REFERENCE_QPS:-}"
COMMON_LOAD_QPS="${COMMON_LOAD_QPS:-}"
NEAR_SATURATION_QPS="${NEAR_SATURATION_QPS:-}"
OVERLOAD_QPS="${OVERLOAD_QPS:-}"
REQUEST_DEADLINE_MS="${REQUEST_DEADLINE_MS:-500}"
LATE_DRAIN_MS="${LATE_DRAIN_MS:-100}"
RECOVERY_MINIMUM_SAMPLES="${RECOVERY_MINIMUM_SAMPLES:-}"
RECOVERY_P95_CEILING_US="${RECOVERY_P95_CEILING_US:-}"
RECOVERY_P99_CEILING_US="${RECOVERY_P99_CEILING_US:-}"
MEASUREMENT_PROFILE="${PHASE5A_MEASUREMENT_PROFILE:-legacy}"
case "${MEASUREMENT_PROFILE}" in
  legacy) ;;
  m2|m3)
    if [[ "${GOMAXPROCS:-}" != 1 ]]; then
      echo "${MEASUREMENT_PROFILE} measurement requires GOMAXPROCS=1" >&2
      exit 2
    fi
    if [[ "${CANDIDATE}" != rust || "${RUN_MODE}" != pilot ]]; then
      echo "${MEASUREMENT_PROFILE} measurement supports only isolated Rust pilot runs" >&2
      exit 2
    fi
    ;;
  *) echo "unsupported measurement profile: ${MEASUREMENT_PROFILE}" >&2; exit 2 ;;
esac

measurement_stage_specs() {
  printf '%s\n' "normal-reference:${NORMAL_REFERENCE_QPS}"
  if [[ "${MEASUREMENT_PROFILE}" != m3 ]]; then
    printf '%s\n' "common-load:${COMMON_LOAD_QPS}" "near-saturation:${NEAR_SATURATION_QPS}"
  fi
  printf '%s\n' "overload:${OVERLOAD_QPS}"
  if [[ "${MEASUREMENT_PROFILE}" != m3 ]]; then
    printf '%s\n' "recovery:${NORMAL_REFERENCE_QPS}"
  fi
}
if [[ "${MEASUREMENT_PROFILE}" == m3 ]]; then
  if [[ "${STAGE_DURATION_MS}" != 25000 || "${NORMAL_REFERENCE_QPS}" != 200 || "${OVERLOAD_QPS}" != 400 || "${REQUEST_DEADLINE_MS}" != 500 || "${LATE_DRAIN_MS}" != 100 ]]; then
    echo "m3 requires reviewed 25000ms, 200/400 QPS, 500ms deadline and 100ms drain" >&2; exit 2
  fi
  if [[ "${SCENARIO}" == w2 && ( "${W2_WARM_LIFECYCLE:-}" != independent-prefilled || "${W2_CACHE_TTL_MS:-}" != 30000 || "${W2_TTL_SAFETY_MARGIN_MS:-}" != 500 ) ]]; then
    echo "m3 W2 requires independent-prefilled lifecycle and frozen TTL settings" >&2; exit 2
  fi
fi
# Read-only plan output uses the same stage list as execution, before any SUT starts.
if [[ "${PHASE5A_PLAN_ONLY:-0}" == 1 ]]; then
  measurement_stage_specs
  exit 0
fi

if [[ -n "${HARNESS_CPU_SET}" && "${PHASE5A_HARNESS_PINNED:-0}" != "1" ]]; then
  command -v taskset >/dev/null 2>&1 || { echo "HARNESS_CPU_SET requires taskset" >&2; exit 2; }
  exec taskset --cpu-list "${HARNESS_CPU_SET}" env PHASE5A_HARNESS_PINNED=1 bash "$0" "$@"
fi

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
if [[ "${SCENARIO}" == "w2" ]]; then
  W2_WARM_LIFECYCLE="${W2_WARM_LIFECYCLE:-same-process}"
else
  W2_WARM_LIFECYCLE="${W2_WARM_LIFECYCLE:-not-applicable}"
fi
case "${RUN_MODE}" in
  smoke|pilot|official) ;;
  *) echo "unsupported RUN_MODE: ${RUN_MODE}" >&2; exit 2 ;;
esac
if [[ "${RUN_MODE}" == "official" && -z "${MANIFEST_PATH}" ]]; then
  echo "official mode requires an explicit MANIFEST_PATH to a reviewed, compatible manifest" >&2
  exit 2
fi

if [[ "${RUN_MODE}" != "smoke" ]]; then
  if [[ -z "${RESULT_DIR}" || -z "${HELPER_BINARY}" || -z "${CANDIDATE}" || -z "${REPETITION}" || -z "${PAIR_POSITION}" ]]; then
    echo "pilot/official mode requires RESULT_DIR, HELPER_BINARY, CANDIDATE, REPETITION, and PAIR_POSITION" >&2
    exit 2
  fi
  if [[ "${CANDIDATE}" != "go" && "${CANDIDATE}" != "rust" ]]; then
    echo "CANDIDATE must be go or rust" >&2
    exit 2
  fi
  if ! [[ "${REPETITION}" =~ ^[1-9][0-9]*$ ]]; then
    echo "REPETITION must be a positive integer" >&2
    exit 2
  fi
  if [[ "${PAIR_POSITION}" != "1" && "${PAIR_POSITION}" != "2" ]]; then
    echo "PAIR_POSITION must be 1 or 2" >&2
    exit 2
  fi
fi
if [[ -z "${RESULT_DIR}" ]]; then
  RESULT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/phase5a-baseline.XXXXXX")"
else
  mkdir -p "${RESULT_DIR}"
fi
RESULT_DIR="$(cd "${RESULT_DIR}" && pwd)"
if [[ "${RUN_MODE}" != "smoke" ]]; then
  case "${RESULT_DIR}" in
    /tmp|/tmp/*) echo "pilot/official RESULT_DIR must not use /tmp" >&2; exit 2 ;;
  esac
  result_fs="$(df -PT "${RESULT_DIR}" | awk 'NR == 2 {print $2}')"
  case "${result_fs}" in
    tmpfs|ramfs|"") echo "pilot/official RESULT_DIR must use a disk-backed filesystem (found ${result_fs:-unknown})" >&2; exit 2 ;;
  esac
fi
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
HELPER_BINARY="$(cd "$(dirname "${HELPER_BINARY}")" && pwd)/$(basename "${HELPER_BINARY}")"
if [[ "${RUN_MODE}" != "smoke" ]]; then
  helper_version="$("${HELPER_BINARY}" version)"
  if [[ "${helper_version}" != "phase5a-baseline-helper/v8" && "${helper_version}" != "phase5a-baseline-helper/v9" ]]; then
    echo "unsupported helper version: ${helper_version}" >&2
    exit 2
  fi
  if [[ "${MEASUREMENT_PROFILE}" != legacy && "${helper_version}" != "phase5a-baseline-helper/v9" ]]; then
    echo "${MEASUREMENT_PROFILE} measurement requires helper v9" >&2
    exit 2
  fi
fi

SCENARIO_CONFIG=""
SUT_ADDR=""
TRANSPORT="udp"
WORKLOAD=""
FIXTURE_SPECS=()
FIXTURE_EVENT_JOURNAL=""
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
    FIXTURE_EVENT_JOURNAL="${TMP_DIR}/routing-events.jsonl"
    ;;
esac

rate_multiple() {
  awk -v rate="$1" -v multiplier="$2" 'BEGIN { if (rate !~ /^[0-9]+([.][0-9]+)?$/ || rate <= 0) exit 1; printf "%.10g", rate * multiplier }'
}

if [[ "${RUN_MODE}" == "smoke" ]]; then
  STAGE_DURATION_MS="${STAGE_DURATION_MS:-1000}"
  NORMAL_REFERENCE_QPS="${NORMAL_REFERENCE_QPS:-${OFFERED_QPS:-20}}"
  COMMON_LOAD_QPS="${COMMON_LOAD_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 2)}"
  NEAR_SATURATION_QPS="${NEAR_SATURATION_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 3)}"
  OVERLOAD_QPS="${OVERLOAD_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 5)}"
elif [[ "${RUN_MODE}" == "pilot" ]]; then
  STAGE_DURATION_MS="${STAGE_DURATION_MS:-3000}"
  NORMAL_REFERENCE_QPS="${NORMAL_REFERENCE_QPS:-${OFFERED_QPS:-20}}"
  COMMON_LOAD_QPS="${COMMON_LOAD_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 2)}"
  NEAR_SATURATION_QPS="${NEAR_SATURATION_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 3)}"
  OVERLOAD_QPS="${OVERLOAD_QPS:-$(rate_multiple "${NORMAL_REFERENCE_QPS}" 5)}"
  RECOVERY_MINIMUM_SAMPLES="${RECOVERY_MINIMUM_SAMPLES:-1}"
  RECOVERY_P95_CEILING_US="${RECOVERY_P95_CEILING_US:-9223372036854775807}"
  RECOVERY_P99_CEILING_US="${RECOVERY_P99_CEILING_US:-9223372036854775807}"
fi
W2_CACHE_TTL_MS="${W2_CACHE_TTL_MS:-30000}"
W2_TTL_SAFETY_MARGIN_MS="${W2_TTL_SAFETY_MARGIN_MS:-500}"
W2_CACHE_TTL="${W2_CACHE_TTL_MS}ms"
W2_TTL_SAFETY_MARGIN="${W2_TTL_SAFETY_MARGIN_MS}ms"
if [[ "${RUN_MODE}" != "smoke" ]]; then
  TEST_HOST_ALIAS="${TEST_HOST_ALIAS:-mosdns-rust}"
  RUST_TOOLCHAIN_VERSION="${RUST_TOOLCHAIN_VERSION:-$(rustc --version)}"
fi
case "${W2_WARM_LIFECYCLE}" in
  same-process|independent-prefilled|not-applicable) ;;
  *) echo "unsupported W2_WARM_LIFECYCLE: ${W2_WARM_LIFECYCLE}" >&2; exit 2 ;;
esac
if ! awk -v duration="${STAGE_DURATION_MS}" -v normal="${NORMAL_REFERENCE_QPS}" -v common="${COMMON_LOAD_QPS}" -v near="${NEAR_SATURATION_QPS}" -v overload="${OVERLOAD_QPS}" -v deadline="${REQUEST_DEADLINE_MS}" -v drain="${LATE_DRAIN_MS}" -v ttl="${W2_CACHE_TTL_MS}" -v margin="${W2_TTL_SAFETY_MARGIN_MS}" 'BEGIN { if (duration !~ /^[0-9]+$/ || duration <= 0 || normal !~ /^[0-9]+([.][0-9]+)?$/ || common !~ /^[0-9]+([.][0-9]+)?$/ || near !~ /^[0-9]+([.][0-9]+)?$/ || overload !~ /^[0-9]+([.][0-9]+)?$/ || normal <= 0 || !(normal < common && common < near && near < overload) || deadline !~ /^[0-9]+$/ || deadline <= 0 || drain !~ /^[0-9]+$/ || drain < 0 || ttl !~ /^[0-9]+$/ || ttl <= 0 || margin !~ /^[0-9]+$/ || margin < 0 || margin >= ttl) exit 1 }'; then
  echo "invalid stage rates, duration, deadline, or W2 TTL parameters" >&2
  exit 2
fi

if [[ "${RUN_MODE}" == "official" ]]; then
  MANIFEST="${MANIFEST_PATH}"
  if [[ "${MANIFEST}" != /* ]]; then
    MANIFEST="${ROOT_DIR}/${MANIFEST}"
  fi
  if [[ ! -f "${MANIFEST}" ]]; then
    echo "official mode requires an existing MANIFEST_PATH: ${MANIFEST}" >&2
    exit 2
  fi
  if [[ -z "${MANIFEST_SHA256}" || -z "${CANDIDATE}" || -z "${REPETITION}" || -z "${PAIR_POSITION}" || -z "${STAGE_DURATION_MS}" || -z "${NORMAL_REFERENCE_QPS}" || -z "${COMMON_LOAD_QPS}" || -z "${NEAR_SATURATION_QPS}" || -z "${OVERLOAD_QPS}" || -z "${RECOVERY_MINIMUM_SAMPLES}" || -z "${RECOVERY_P95_CEILING_US}" || -z "${RECOVERY_P99_CEILING_US}" ]]; then
    echo "official mode requires a complete candidate, pair, stage, and recovery plan" >&2
    exit 2
  fi
  if [[ -z "${HARNESS_CPU_SET}" || -z "${SUT_CPU_SET}" ]]; then
    echo "pilot/official mode requires separate HARNESS_CPU_SET and SUT_CPU_SET" >&2
    exit 2
  fi
  if [[ -z "${REPETITION}" || -z "${PAIR_POSITION}" ]]; then
    echo "official mode requires REPETITION and PAIR_POSITION" >&2
    exit 2
  fi
  actual_manifest_sha256="$(sha256sum "${MANIFEST}" | awk '{print $1}')"
  manifest_args=(verify-manifest --manifest "${MANIFEST}" --sha256 "${MANIFEST_SHA256}" --repo-root "${ROOT_DIR}" --helper "${HELPER_BINARY}" --runner "${ROOT_DIR}/scripts/run-phase5a-baseline.sh" --sut "${MOSDNS_BINARY}" --candidate "${CANDIDATE}" --scenario "${SCENARIO}" --repetition "${REPETITION}" --position "${PAIR_POSITION}" --stage-duration-ms "${STAGE_DURATION_MS}" --normal-reference-qps "${NORMAL_REFERENCE_QPS}" --common-load-qps "${COMMON_LOAD_QPS}" --near-saturation-qps "${NEAR_SATURATION_QPS}" --overload-qps "${OVERLOAD_QPS}" --deadline-ms "${REQUEST_DEADLINE_MS}" --late-drain-ms "${LATE_DRAIN_MS}" --w2-cache-ttl-ms "${W2_CACHE_TTL_MS}" --w2-ttl-safety-margin-ms "${W2_TTL_SAFETY_MARGIN_MS}" --recovery-minimum-samples "${RECOVERY_MINIMUM_SAMPLES}" --recovery-p95-ceiling-us "${RECOVERY_P95_CEILING_US}" --recovery-p99-ceiling-us "${RECOVERY_P99_CEILING_US}" --w2-warm-lifecycle "${W2_WARM_LIFECYCLE}" --harness-cpu-set "${HARNESS_CPU_SET}" --sut-cpu-set "${SUT_CPU_SET}" --host-alias "${TEST_HOST_ALIAS}" --rust-toolchain "${RUST_TOOLCHAIN_VERSION}")
  "${HELPER_BINARY}" "${manifest_args[@]}"
  printf '%s  %s\n' "${actual_manifest_sha256}" "${MANIFEST}" > "${RESULT_DIR}/manifest.sha256"
fi

"${HELPER_BINARY}" validate-binary --path "${MOSDNS_BINARY}" > "${RESULT_DIR}/sut.json"
sha256sum \
  "${CONFIG_DIR}/forward-udp.yaml" "${CONFIG_DIR}/forward-tcp.yaml" "${CONFIG_DIR}/cache.yaml" "${CONFIG_DIR}/routing.yaml" \
  "${WORKLOAD_DIR}/forward.jsonl" "${WORKLOAD_DIR}/cache.jsonl" "${WORKLOAD_DIR}/routing.jsonl" \
  "${ROOT_DIR}/scripts/run-phase5a-baseline.sh" "${ROOT_DIR}/tests/phase5a-baseline/cmd/phase5a-baseline/main.go" \
  "${HELPER_BINARY}" "${MOSDNS_BINARY}" > "${RESULT_DIR}/input-hashes.sha256"
"${HELPER_BINARY}" version > "${RESULT_DIR}/helper-version.txt"
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

record_affinity() {
  local role="$1" pid="$2" requested="$3" actual
  if [[ ! -r "/proc/${pid}/status" ]]; then
    echo "cannot inspect affinity for ${role} PID ${pid}" >&2
    return 1
  fi
  actual="$(awk '/^Cpus_allowed_list:/ { print $2 }' "/proc/${pid}/status")"
  if [[ -z "${actual}" ]]; then
    echo "missing Cpus_allowed_list for ${role} PID ${pid}" >&2
    return 1
  fi
  printf '%s\t%s\t%s\t%s\n' "${role}" "${pid}" "${requested}" "${actual}" >> "${RESULT_DIR}/affinity.tsv"
}

start_fixtures() {
  local spec network addr upstream counter
  for spec in "${FIXTURE_SPECS[@]}"; do
    IFS='|' read -r network addr upstream counter <<<"${spec}"
    fixture_args=(fixture --network "${network}" --addr "${addr}" --upstream-id "${upstream}" --counter "${counter}")
    if [[ -n "${FIXTURE_EVENT_JOURNAL}" ]]; then
      fixture_args+=(--event-journal "${FIXTURE_EVENT_JOURNAL}")
    fi
    "${HELPER_BINARY}" "${fixture_args[@]}" >> "${TMP_DIR}/${upstream}.stdout" 2>> "${TMP_DIR}/${upstream}.stderr" &
    FIXTURE_PIDS+=("$!")
  done
  for spec in "${FIXTURE_SPECS[@]}"; do
    IFS='|' read -r _ _ _ counter <<<"${spec}"
    wait_for_counter_file "${counter}"
  done
  if [[ -n "${HARNESS_CPU_SET}" ]]; then
    "${HELPER_BINARY}" verify-affinity --pid "$$" --expected "${HARNESS_CPU_SET}"
    record_affinity runner "$$" "${HARNESS_CPU_SET}"
    for pid in "${FIXTURE_PIDS[@]}"; do
      "${HELPER_BINARY}" verify-affinity --pid "${pid}" --expected "${HARNESS_CPU_SET}"
      record_affinity fixture "${pid}" "${HARNESS_CPU_SET}"
    done
  fi
}

start_sut() {
  if [[ -n "${SUT_CPU_SET}" && -n "${HARNESS_CPU_SET}" ]]; then
    "${HELPER_BINARY}" verify-cpu-sets --harness "${HARNESS_CPU_SET}" --sut "${SUT_CPU_SET}"
  fi
  if [[ -n "${SUT_CPU_SET}" ]]; then
    command -v taskset >/dev/null 2>&1 || { echo "SUT_CPU_SET requires taskset" >&2; return 2; }
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
    sleep "${SUT_STARTUP_MARGIN}"
    if ! kill -0 "${SUT_PID}" 2>/dev/null; then
      echo "SUT exited during startup" >&2
      return 1
    fi
    if [[ -n "${SUT_CPU_SET}" ]]; then
      "${HELPER_BINARY}" verify-affinity --pid "${SUT_PID}" --expected "${SUT_CPU_SET}"
      record_affinity sut "${SUT_PID}" "${SUT_CPU_SET}"
    fi
    return 0
  fi
  for _ in $(seq 1 100); do
    if kill -0 "${SUT_PID}" 2>/dev/null && (echo >/dev/tcp/127.0.0.1/"${sut_port}") 2>/dev/null; then
      if [[ -n "${SUT_CPU_SET}" ]]; then
        "${HELPER_BINARY}" verify-affinity --pid "${SUT_PID}" --expected "${SUT_CPU_SET}"
        record_affinity sut "${SUT_PID}" "${SUT_CPU_SET}"
      fi
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
  local target_dir="${1:-${RESULT_DIR}}"
  mkdir -p "${target_dir}"
  local spec upstream counter
  for spec in "${FIXTURE_SPECS[@]}"; do
    IFS='|' read -r _ _ upstream counter <<<"${spec}"
    if [[ ! -f "${counter}" ]]; then
      echo "missing final fixture counter for ${upstream}" >&2
      return 1
    fi
    cp "${counter}" "${target_dir}/fixture-${upstream}.json"
  done
}

verify_counter_delta() {
  local counter_path="$1"
  local baseline_path="$2"
  local scenario="$3"
  local result_path="$4"
  local stage_name="$5"
  local expect_delta="${6:-true}"
  local verify_args=(verify-counters --scenario "${scenario}" --workload "${WORKLOAD}" --counter "${counter_path}" --baseline "${baseline_path}" --stage-result "${result_path}/stages.jsonl" --stage "${stage_name}")
  if [[ "${expect_delta}" == "true" ]]; then
    verify_args+=(--expect-delta)
  fi
  for _ in $(seq 1 50); do
    if "${HELPER_BINARY}" "${verify_args[@]}"; then
      return 0
    fi
    sleep 0.1
  done
  "${HELPER_BINARY}" "${verify_args[@]}"
}

verify_sample_coverage() {
  local stage_dir="$1" stage_name="$2"
  "${HELPER_BINARY}" verify-samples --stage-result "${stage_dir}/stages.jsonl" --stage "${stage_name}" --expected-fixtures "${#FIXTURE_SPECS[@]}"
}

verify_sender_schedule() {
  local stage_dir="$1" stage_name="$2"
  "${HELPER_BINARY}" verify-sender --stage-result "${stage_dir}/stages.jsonl" --stage "${stage_name}"
}

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
EVENT_ARGS=()
if [[ -n "${FIXTURE_EVENT_JOURNAL}" ]]; then
  EVENT_ARGS+=(--event-journal "${FIXTURE_EVENT_JOURNAL}")
fi
if [[ "${RUN_MODE}" == "pilot" || "${RUN_MODE}" == "official" ]]; then
  if [[ -z "${HARNESS_CPU_SET}" || -z "${SUT_CPU_SET}" ]]; then
    echo "pilot/official mode requires separate HARNESS_CPU_SET and SUT_CPU_SET" >&2
    exit 2
  fi
  if ! "${HELPER_BINARY}" verify-cpu-sets --harness "${HARNESS_CPU_SET}" --sut "${SUT_CPU_SET}"; then
    exit 2
  fi
fi

RUN_INVALID=0
CONTINUOUS_WARM_INVALID=0
SESSION_RUN_ID="${RUN_ID}"
SESSION_FIXTURE_ID="${FIXTURE_SESSION_ID}"
record_invalid() {
  local stage="$1" reason="$2"
  RUN_INVALID=1
  printf '%s\t%s\n' "${stage}" "${reason}" >> "${RESULT_DIR}/invalid-stages.tsv"
}

run_one_stage() {
  local stage_name="$1" stage_qps="$2" stage_dir="$3" ledger_path="$4" one_pass="$5" stage_mode="$6"
  local baseline="" counter="" upstream="" spec
  mkdir -p "${stage_dir}"
  if [[ "${SCENARIO}" != "w3" ]]; then
    IFS='|' read -r _ _ upstream counter <<<"${FIXTURE_SPECS[0]}"
    baseline="${TMP_DIR}/counter-before-${stage_name}.json"
    cp "${counter}" "${baseline}"
  fi
  local run_args=(run --workload "${WORKLOAD}" --scenario "${workload_scenario}" --transport "${TRANSPORT}" --addr "${SUT_ADDR}" --stage "${stage_name}" --qps "${stage_qps}" --duration "${STAGE_DURATION_MS}ms" --deadline "${REQUEST_DEADLINE_MS}ms" --late-drain "${LATE_DRAIN_MS}ms" --run-id "${SESSION_RUN_ID}" --fixture-session-id "${SESSION_FIXTURE_ID}" --result "${stage_dir}" --sut-pid "${SUT_PID}" --request-ledger "${ledger_path}")
  if [[ -n "${FIXTURE_EVENT_JOURNAL}" ]]; then
    run_args+=(--event-journal "${FIXTURE_EVENT_JOURNAL}")
  fi
  if [[ "${one_pass}" == "true" ]]; then
    run_args+=(--one-pass --fail-on-error)
  fi
  for fixture_pid in "${FIXTURE_PIDS[@]}"; do
    run_args+=(--fixture-pid "${fixture_pid}")
  done
  if ! "${HELPER_BINARY}" "${run_args[@]}"; then
    record_invalid "${stage_name}" "load helper failed"
    return 0
  fi
  if ! verify_sample_coverage "${stage_dir}" "${stage_name}"; then
    record_invalid "${stage_name}" "missing SUT/load-generator/fixture resource samples"
  fi
  if ! verify_sender_schedule "${stage_dir}" "${stage_name}"; then
    record_invalid "${stage_name}" "sender shortfall or scheduled query not sent"
  fi
  if [[ "${stage_mode}" == "cold" ]] && ! "${HELPER_BINARY}" verify-stage --stage-result "${stage_dir}/stages.jsonl" --stage "${stage_name}"; then
    record_invalid "${stage_name}" "cold stage correctness gate failed"
  fi
  if [[ -n "${counter}" ]]; then
    local expected_delta="true"
    if [[ "${SCENARIO}" == "w2" && "${stage_mode}" != "cold" ]]; then
      expected_delta="false"
    fi
    if ! verify_counter_delta "${counter}" "${baseline}" "${workload_scenario}" "${stage_dir}" "${stage_name}" "${expected_delta}"; then
      if [[ "${SCENARIO}" == "w2" && "${RUN_MODE}" == "pilot" && "${stage_mode}" == "warm" ]]; then
        CONTINUOUS_WARM_INVALID=1
        printf '%s\t%s\n' "${stage_name}" "fixture counter delta mismatch" >> "${RESULT_DIR}/w2-continuous-warm-issues.tsv"
      else
        record_invalid "${stage_name}" "fixture counter delta mismatch"
      fi
    fi
  fi
  if [[ "${SCENARIO}" == "w3" ]]; then
    if ! "${HELPER_BINARY}" verify-routing-events --workload "${WORKLOAD}" --request-ledger "${ledger_path}" \
      --event-journal "${FIXTURE_EVENT_JOURNAL}" --stage-result "${stage_dir}/stages.jsonl" --stage "${stage_name}"; then
      record_invalid "${stage_name}" "per-request route event verification failed"
    fi
  fi
  if ! kill -0 "${SUT_PID}" 2>/dev/null; then
    record_invalid "${stage_name}" "SUT process exited during stage"
  fi
}

run_continuous_sequence() {
  local stage_dir="$1" ledger_path="$2" use_one_pass="$3"
  local stage_spec stage stage_qps
  while IFS= read -r stage_spec; do
    stage="${stage_spec%%:*}"; stage_qps="${stage_spec#*:}"
    run_one_stage "${stage}" "${stage_qps}" "${stage_dir}" "${ledger_path}" "${use_one_pass}" warm
    use_one_pass=false
  done < <(measurement_stage_specs)
  if [[ "${MEASUREMENT_PROFILE}" == m3 ]]; then
    printf '%s\n' "status=indeterminate" "mode=indeterminate-no-overload-evidence" "reason=M3 primary latency points only; no recovery measurement" > "${RESULT_DIR}/service-recovery-assessment.txt"
    return
  fi
  if ! "${HELPER_BINARY}" verify-continuous --stage-result "${stage_dir}/stages.jsonl" --run-id "${SESSION_RUN_ID}" \
    --minimum-samples "${RECOVERY_MINIMUM_SAMPLES}" --p95-ceiling-us "${RECOVERY_P95_CEILING_US}" --p99-ceiling-us "${RECOVERY_P99_CEILING_US}" \
    > "${RESULT_DIR}/service-recovery-assessment.txt"; then
    record_invalid "recovery" "same-process recovery criteria failed"
  fi
}

run_w2_independent_points() {
  local stage stage_qps stage_dir ledger_path prefill_dir prefill_before prefill_after
  mkdir -p "${RESULT_DIR}/w2-warm-independent"
  printf '%s\n' "indeterminate: W2 warm stages use independent prefilled SUT sessions; no same-process recovery claim" > "${RESULT_DIR}/w2-warm-independent/recovery-status.txt"
  printf '%s\n' "status=indeterminate" "mode=indeterminate-no-overload-evidence" "reason=independent-prefilled W2 sessions; no same-process recovery is measured" > "${RESULT_DIR}/service-recovery-assessment.txt"
  while IFS= read -r stage_spec; do
    stage="${stage_spec%%:*}"
    stage_qps="${stage_spec#*:}"
    stage_dir="${RESULT_DIR}/w2-warm-independent/${stage}"
    ledger_path="${stage_dir}/requests.jsonl"
    prefill_dir="${stage_dir}/prefill"
    mkdir -p "${prefill_dir}"
    SESSION_RUN_ID="${RUN_ID}-w2-independent-${stage}"
    SESSION_FIXTURE_ID="${FIXTURE_SESSION_ID}-w2-independent-${stage}"
    start_fixtures
    start_sut
    IFS='|' read -r _ _ _ CACHE_COUNTER <<<"${FIXTURE_SPECS[0]}"
    prefill_before="${TMP_DIR}/w2-independent-${stage}-before.json"
    cp "${CACHE_COUNTER}" "${prefill_before}"
    local prefill_args=(run --workload "${WORKLOAD}" --scenario w2 --transport udp --addr "${SUT_ADDR}" --stage warm-prefill --qps "${NORMAL_REFERENCE_QPS}" --duration "${STAGE_DURATION_MS}ms" --deadline "${REQUEST_DEADLINE_MS}ms" --late-drain "${LATE_DRAIN_MS}ms" --run-id "${SESSION_RUN_ID}" --fixture-session-id "${SESSION_FIXTURE_ID}" --one-pass --fail-on-error --result "${prefill_dir}" --sut-pid "${SUT_PID}" --request-ledger "${ledger_path}")
    for fixture_pid in "${FIXTURE_PIDS[@]}"; do
      prefill_args+=(--fixture-pid "${fixture_pid}")
    done
    if ! "${HELPER_BINARY}" "${prefill_args[@]}"; then
      record_invalid "${stage}-prefill" "independent warm prefill failed"
    elif ! verify_sample_coverage "${prefill_dir}" warm-prefill; then
      record_invalid "${stage}-prefill" "missing SUT/load-generator/fixture resource samples"
    elif ! verify_counter_delta "${CACHE_COUNTER}" "${prefill_before}" w2 "${prefill_dir}" warm-prefill true; then
      record_invalid "${stage}-prefill" "independent warm prefill counter delta mismatch"
    fi
    prefill_after="${TMP_DIR}/w2-independent-${stage}-prefilled.json"
    cp "${CACHE_COUNTER}" "${prefill_after}"
    run_one_stage "${stage}" "${stage_qps}" "${stage_dir}" "${ledger_path}" false independent
    stop_sut
    stop_fixtures
    copy_fixture_counters "${stage_dir}"
    if ! "${HELPER_BINARY}" verify-counters --scenario w2 --workload "${WORKLOAD}" --counter "${stage_dir}/fixture-cache.json" --baseline "${prefill_after}"; then
      record_invalid "${stage}" "final independent warm counter delta mismatch"
    fi
    if ! "${HELPER_BINARY}" verify-warm-ttl --workload "${WORKLOAD}" --request-ledger "${ledger_path}" \
      --prefill-stage warm-prefill --warm-stage "${stage}" --ttl "${W2_CACHE_TTL}" --safety-margin "${W2_TTL_SAFETY_MARGIN}"; then
      record_invalid "${stage}" "independent W2 warm point exceeded per-key TTL or correctness gate"
    fi
  done < <(measurement_stage_specs)
}

start_fixtures
if [[ "${SCENARIO}" == "w2" ]]; then
  IFS='|' read -r _ _ _ CACHE_COUNTER <<<"${FIXTURE_SPECS[0]}"
  COLD_DIR="${RESULT_DIR}/w2-cold"
  WARM_DIR="${RESULT_DIR}/w2-warm"
  mkdir -p "${COLD_DIR}" "${WARM_DIR}"

  SESSION_RUN_ID="${RUN_ID}-w2-cold"
  SESSION_FIXTURE_ID="${FIXTURE_SESSION_ID}-cold"
  start_sut
  if [[ "${RUN_MODE}" == "smoke" ]]; then COLD_ONE_PASS=true; else COLD_ONE_PASS=false; fi
  run_one_stage "${RUN_MODE}-w2-cold" "${NORMAL_REFERENCE_QPS}" "${COLD_DIR}" "${COLD_DIR}/requests.jsonl" "${COLD_ONE_PASS}" cold
  stop_sut
  stop_fixtures
  copy_fixture_counters "${COLD_DIR}"
  if ! "${HELPER_BINARY}" verify-session-counters --scenario w2 --workload "${WORKLOAD}" --counter "${COLD_DIR}/fixture-cache.json" \
    --stage-result "${COLD_DIR}/stages.jsonl" --run-id "${SESSION_RUN_ID}"; then
    record_invalid w2-cold "final cold session counters mismatch"
  fi

if [[ "${RUN_MODE}" != "smoke" && "${W2_WARM_LIFECYCLE}" == "independent-prefilled" ]]; then
    run_w2_independent_points
  else
    start_fixtures
    SESSION_RUN_ID="${RUN_ID}-w2-warm"
    SESSION_FIXTURE_ID="${FIXTURE_SESSION_ID}-warm"
    start_sut
    PREFILL_DIR="${WARM_DIR}/prefill"
    mkdir -p "${PREFILL_DIR}"
    cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-before-counter.json"
    PREFILL_ARGS=(run --workload "${WORKLOAD}" --scenario w2 --transport udp --addr "${SUT_ADDR}" --stage warm-prefill --qps "${NORMAL_REFERENCE_QPS}" --duration "${STAGE_DURATION_MS}ms" --deadline "${REQUEST_DEADLINE_MS}ms" --late-drain "${LATE_DRAIN_MS}ms" --run-id "${SESSION_RUN_ID}" --fixture-session-id "${SESSION_FIXTURE_ID}" --one-pass --fail-on-error --result "${PREFILL_DIR}" --sut-pid "${SUT_PID}" --request-ledger "${WARM_DIR}/requests.jsonl")
    for fixture_pid in "${FIXTURE_PIDS[@]}"; do
      PREFILL_ARGS+=(--fixture-pid "${fixture_pid}")
    done
    if ! "${HELPER_BINARY}" "${PREFILL_ARGS[@]}"; then
      record_invalid warm-prefill "prefill helper failed"
    elif ! verify_sample_coverage "${PREFILL_DIR}" warm-prefill; then
      record_invalid warm-prefill "missing SUT/load-generator/fixture resource samples"
    elif ! "${HELPER_BINARY}" verify-stage --stage-result "${PREFILL_DIR}/stages.jsonl" --stage warm-prefill; then
      record_invalid warm-prefill "prefill correctness gate failed"
    elif ! verify_counter_delta "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-before-counter.json" w2 "${PREFILL_DIR}" warm-prefill true; then
      record_invalid warm-prefill "prefill upstream delta mismatch"
    fi
    cp "${CACHE_COUNTER}" "${TMP_DIR}/w2-prefill-counter.json"

    SESSION_RUN_ID="${RUN_ID}-w2-warm"
    if [[ "${RUN_MODE}" == "smoke" ]]; then
      run_one_stage "${RUN_MODE}-w2-warm" "${NORMAL_REFERENCE_QPS}" "${WARM_DIR}" "${WARM_DIR}/requests.jsonl" true warm
    else
      run_continuous_sequence "${WARM_DIR}" "${WARM_DIR}/requests.jsonl" false
    fi
    stop_sut
    stop_fixtures
    copy_fixture_counters "${WARM_DIR}"
    warm_stages=()
    if [[ "${RUN_MODE}" == "smoke" ]]; then
      warm_stages+=(--warm-stage "${RUN_MODE}-w2-warm")
    else
      warm_stages+=(--warm-stage normal-reference --warm-stage common-load --warm-stage near-saturation --warm-stage overload --warm-stage recovery)
    fi
    if ! "${HELPER_BINARY}" verify-warm-ttl --workload "${WORKLOAD}" --request-ledger "${WARM_DIR}/requests.jsonl" \
      --prefill-stage warm-prefill "${warm_stages[@]}" --ttl "${W2_CACHE_TTL}" --safety-margin "${W2_TTL_SAFETY_MARGIN}"; then
      printf '%s\n' "indeterminate: at least one response exceeded the frozen per-key TTL safety window" > "${WARM_DIR}/recovery-status.txt"
      if [[ "${RUN_MODE}" == "pilot" ]]; then
        run_w2_independent_points
      else
        record_invalid w2-warm "per-key TTL window exceeded; same-process warm result is ineligible"
      fi
    elif [[ "${CONTINUOUS_WARM_INVALID}" -ne 0 ]]; then
      record_invalid w2-warm "cache counter changed during a TTL-eligible warm stage"
      printf '%s\n' "invalid: TTL passed but one or more warm stages reached the fixture" > "${WARM_DIR}/recovery-status.txt"
    else
      printf '%s\n' "TTL-eligible" > "${WARM_DIR}/recovery-status.txt"
      if ! "${HELPER_BINARY}" verify-counters --scenario w2 --workload "${WORKLOAD}" --counter "${WARM_DIR}/fixture-cache.json" --baseline "${TMP_DIR}/w2-prefill-counter.json"; then
        record_invalid w2-warm "final warm cache counter delta mismatch"
      fi
    fi
  fi
else
  SESSION_RUN_ID="${RUN_ID}"
  SESSION_FIXTURE_ID="${FIXTURE_SESSION_ID}"
  start_sut
  LEDGER_PATH="${RESULT_DIR}/requests.jsonl"
  if [[ "${RUN_MODE}" == "smoke" ]]; then
    run_one_stage "${RUN_MODE}-${SCENARIO}" "${NORMAL_REFERENCE_QPS}" "${RESULT_DIR}" "${LEDGER_PATH}" true measured
  else
    run_continuous_sequence "${RESULT_DIR}" "${LEDGER_PATH}" false
  fi
  stop_sut
fi

stop_sut
stop_fixtures
if [[ "${SCENARIO}" != "w2" ]]; then
  copy_fixture_counters
fi
if [[ "${SCENARIO}" == "w3" ]]; then
  cp "${FIXTURE_EVENT_JOURNAL}" "${RESULT_DIR}/fixture-routing-events.jsonl"
  if [[ "${RUN_MODE}" == "smoke" ]]; then LAST_STAGE="${RUN_MODE}-${SCENARIO}"; elif [[ "${MEASUREMENT_PROFILE}" == m3 ]]; then LAST_STAGE=overload; else LAST_STAGE=recovery; fi
  if ! "${HELPER_BINARY}" verify-event-journal --event-journal "${RESULT_DIR}/fixture-routing-events.jsonl" --stage-result "${RESULT_DIR}/stages.jsonl" --last-stage "${LAST_STAGE}"; then
    record_invalid events "fixture event journal has a tail mismatch"
  fi
  "${HELPER_BINARY}" verify-counters --scenario w3 --workload "${WORKLOAD}" \
    --route-a "${RESULT_DIR}/fixture-route-a.json" --route-b "${RESULT_DIR}/fixture-route-b.json" --route-c "${RESULT_DIR}/fixture-route-c.json" \
    --event-journal "${RESULT_DIR}/fixture-routing-events.jsonl" || record_invalid counters "aggregate W3 counters mismatch"
elif [[ "${SCENARIO}" == w1-* ]]; then
  "${HELPER_BINARY}" verify-session-counters --scenario w1 --workload "${WORKLOAD}" --counter "${RESULT_DIR}/fixture-forward.json" \
    --stage-result "${RESULT_DIR}/stages.jsonl" --run-id "${SESSION_RUN_ID}" || record_invalid counters "final W1 session counters mismatch"
fi

cp "${TMP_DIR}/mosdns.stdout" "${RESULT_DIR}/sut.stdout.log"
cp "${TMP_DIR}/mosdns.stderr" "${RESULT_DIR}/sut.stderr.log"
printf '%s\n' "scenario=${SCENARIO}" "run_mode=${RUN_MODE}" "config=${SCENARIO_CONFIG}" "workload=${WORKLOAD}" > "${RESULT_DIR}/run-metadata.txt"
printf '%s\n' "run_id=${RUN_ID}" "fixture_session_id=${FIXTURE_SESSION_ID}" "candidate=${CANDIDATE}" "repetition=${REPETITION}" "pair_position=${PAIR_POSITION}" >> "${RESULT_DIR}/run-metadata.txt"
printf '%s\n' "stage_duration_ms=${STAGE_DURATION_MS}" "normal_reference_qps=${NORMAL_REFERENCE_QPS}" "common_load_qps=${COMMON_LOAD_QPS}" "near_saturation_qps=${NEAR_SATURATION_QPS}" "overload_qps=${OVERLOAD_QPS}" >> "${RESULT_DIR}/run-metadata.txt"
printf '%s\n' "request_deadline_ms=${REQUEST_DEADLINE_MS}" "late_drain_ms=${LATE_DRAIN_MS}" "sut_cpu_set=${SUT_CPU_SET}" "harness_cpu_set=${HARNESS_CPU_SET}" "sut_startup_margin=${SUT_STARTUP_MARGIN}" >> "${RESULT_DIR}/run-metadata.txt"
if [[ "${RUN_MODE}" != "smoke" ]]; then
  {
    printf 'measurement_profile=%s\ngomaxprocs_environment=%s\n' "${MEASUREMENT_PROFILE}" "${GOMAXPROCS:-unset}"
    printf 'ssh_host_alias=%s\n' "${TEST_HOST_ALIAS}"
    printf 'hostname=%s\n' "$(hostname -f)"
    printf 'kernel=%s\n' "$(uname -a)"
    printf 'online_cpus=%s\n' "$(getconf _NPROCESSORS_ONLN)"
    printf 'memory=%s\n' "$(free -b | awk '/^Mem:/ {print $2}')"
    printf 'disk_filesystem=%s\n' "$(df -PT "${RESULT_DIR}" | awk 'NR == 2 {print $2}')"
    printf 'open_file_limit=%s\n' "$(ulimit -n)"
    printf 'go_base_launcher=%s\n' "$(go version)"
    printf 'go_project_selected_toolchain=%s\n' "$(cd "${ROOT_DIR}" && go version)"
    printf 'go_helper_build_toolchain=%s\n' "$(go version -m "${HELPER_BINARY}" | awk -F': ' 'NR == 1 {print $2}')"
    if [[ "${CANDIDATE}" == "go" ]]; then
      printf 'go_candidate_build_toolchain=%s\n' "$(go version -m "${MOSDNS_BINARY}" | awk -F': ' 'NR == 1 {print $2}')"
    else
      printf 'go_candidate_build_toolchain=not-applicable\n'
    fi
    printf 'rust_toolchain=%s\n' "${RUST_TOOLCHAIN_VERSION}"
    printf 'cpu_cgroup=%s\n' "$(tr '\n' ';' < /proc/self/cgroup)"
    printf 'harness_requested_cpus=%s\n' "${HARNESS_CPU_SET}"
    printf 'sut_requested_cpus=%s\n' "${SUT_CPU_SET}"
  } > "${RESULT_DIR}/environment.txt"
fi
if [[ "${SCENARIO}" == "w2" ]]; then
  printf '%s\n' "w2_cache_ttl=${W2_CACHE_TTL}" "w2_ttl_safety_margin=${W2_TTL_SAFETY_MARGIN}" "w2_warm_lifecycle=${W2_WARM_LIFECYCLE}" >> "${RESULT_DIR}/run-metadata.txt"
fi
if [[ "${RUN_MODE}" == "official" ]]; then
  printf '%s\n' "manifest_sha256=${MANIFEST_SHA256}" >> "${RESULT_DIR}/run-metadata.txt"
fi
if [[ "${RUN_MODE}" != "smoke" ]]; then
  printf '%s\n' "recovery_assessment_mode=indeterminate-no-overload-evidence" >> "${RESULT_DIR}/run-metadata.txt"
fi
if [[ "${RUN_INVALID}" -ne 0 ]]; then
  exit 1
fi
