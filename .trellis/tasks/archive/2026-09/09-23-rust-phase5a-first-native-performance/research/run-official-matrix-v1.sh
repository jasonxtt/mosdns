#!/usr/bin/env bash
set -euo pipefail

task_root="${PHASE5A_TASK_ROOT:-/root/mosdns-rust-phase5a-first-native-performance-605c305}"
tool_root="${task_root}/src-rust-v9"
manifest_path="${task_root}/official-manifest-v1.json"
helper_path="${task_root}/bin/official-v1/phase5a-baseline-helper"
runner_path="${tool_root}/scripts/run-phase5a-baseline.sh"
driver_path="${task_root}/evidence-official-v1/run-official-matrix-v1.sh"
results_root="${task_root}/results/official-v1"
mode="${1:-}"

case "${mode}" in
  --validate-only|--dry-run|--execute) ;;
  *)
    echo "usage: $0 --validate-only|--dry-run|--execute" >&2
    exit 2
    ;;
esac

for path in "${manifest_path}" "${helper_path}" "${runner_path}" "${driver_path}"; do
  if [[ ! -f "${path}" ]]; then
    echo "missing frozen matrix input: ${path}" >&2
    exit 2
  fi
done

manifest_sha256="$(sha256sum "${manifest_path}" | awk '{print $1}')"
actual_driver_sha256="$(sha256sum "${driver_path}" | awk '{print $1}')"
expected_driver_sha256="$(python3 - "${manifest_path}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
print(manifest["matrix_driver"]["sha256"])
PY
)"
if [[ "${actual_driver_sha256}" != "${expected_driver_sha256}" ]]; then
  echo "matrix driver hash mismatch: expected=${expected_driver_sha256} actual=${actual_driver_sha256}" >&2
  exit 2
fi

mapfile -t tuples < <(python3 - "${manifest_path}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
for scenario in ("w1-udp", "w1-tcp", "w2", "w3"):
    if scenario not in manifest["scenarios"]:
        raise SystemExit(f"missing frozen scenario: {scenario}")
    for pair in manifest["pair_schedule"]:
        for position, candidate in enumerate(pair["order"], 1):
            print(f"{scenario}\t{pair['repetition']}\t{position}\t{candidate}")
PY
)
if [[ "${#tuples[@]}" -ne 24 ]]; then
  echo "frozen matrix has ${#tuples[@]} candidate tuples, want 24" >&2
  exit 2
fi

scenario_values() {
  python3 - "${manifest_path}" "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
p = manifest["scenarios"][sys.argv[2]]
for key in (
    "stage_duration_ms", "normal_reference_qps", "common_load_qps",
    "near_saturation_qps", "overload_qps", "request_deadline_ms",
    "late_drain_ms", "w2_cache_ttl_ms", "w2_ttl_safety_margin_ms",
):
    print(p[key])
print(p.get("w2_warm_lifecycle") or "not-applicable")
print(p["harness_cpu_set"])
print(p["sut_cpu_set"])
print(p["recovery_minimum_samples"])
print(p["recovery_p95_ceiling_us"])
print(p["recovery_p99_ceiling_us"])
PY
}

candidate_binary() {
  python3 - "${manifest_path}" "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
print(manifest["candidates"][sys.argv[2]]["binary_path"])
PY
}

manifest_value() {
  python3 - "${manifest_path}" "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
print(manifest["environment"][sys.argv[2]])
PY
}

verify_tuple() {
  local scenario="$1" repetition="$2" position="$3" candidate="$4"
  local -a values
  mapfile -t values < <(scenario_values "${scenario}")
  local sut_path
  sut_path="$(candidate_binary "${candidate}")"
  "${helper_path}" verify-manifest \
    --manifest "${manifest_path}" --sha256 "${manifest_sha256}" \
    --repo-root "${tool_root}" --helper "${helper_path}" --runner "${runner_path}" \
    --sut "${sut_path}" --candidate "${candidate}" --scenario "${scenario}" \
    --repetition "${repetition}" --position "${position}" \
    --stage-duration-ms "${values[0]}" \
    --normal-reference-qps "${values[1]}" --common-load-qps "${values[2]}" \
    --near-saturation-qps "${values[3]}" --overload-qps "${values[4]}" \
    --deadline-ms "${values[5]}" --late-drain-ms "${values[6]}" \
    --w2-cache-ttl-ms "${values[7]}" --w2-ttl-safety-margin-ms "${values[8]}" \
    --w2-warm-lifecycle "${values[9]}" \
    --recovery-minimum-samples "${values[12]}" \
    --recovery-p95-ceiling-us "${values[13]}" --recovery-p99-ceiling-us "${values[14]}" \
    --harness-cpu-set "${values[10]}" --sut-cpu-set "${values[11]}" \
    --host-alias "$(manifest_value host_alias)" \
    --rust-toolchain "$(manifest_value rust_toolchain)"
}

echo "manifest_sha256=${manifest_sha256}"
echo "matrix_driver_sha256=${actual_driver_sha256}"
for tuple in "${tuples[@]}"; do
  IFS=$'\t' read -r scenario repetition position candidate <<<"${tuple}"
  verify_tuple "${scenario}" "${repetition}" "${position}" "${candidate}"
  printf 'manifest_tuple_validated\t%s\t%s\t%s\t%s\n' "${scenario}" "${repetition}" "${position}" "${candidate}"
done

if [[ "${mode}" == "--validate-only" ]]; then
  echo "validated_candidate_tuples=${#tuples[@]}"
  exit 0
fi

if [[ "${mode}" == "--dry-run" ]]; then
  printf 'scenario\trepetition\tposition\tcandidate\tbinary\tresult_dir\n'
  for tuple in "${tuples[@]}"; do
    IFS=$'\t' read -r scenario repetition position candidate <<<"${tuple}"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "${scenario}" "${repetition}" "${position}" "${candidate}" \
      "$(candidate_binary "${candidate}")" \
      "${results_root}/${scenario}/repetition-${repetition}/${candidate}"
  done
  exit 0
fi

if [[ -e "${results_root}" ]]; then
  echo "refusing to overwrite existing official results: ${results_root}" >&2
  exit 2
fi
command -v taskset >/dev/null 2>&1 || { echo "taskset is required" >&2; exit 2; }
mkdir -p "${results_root}"
status_path="${results_root}/attempt-status.tsv"
printf 'scenario\trepetition\tposition\tcandidate\tstart_utc\tend_utc\trunner_exit\tloadavg_before\tloadavg_after\n' > "${status_path}"

runner_failures=0
for tuple in "${tuples[@]}"; do
  IFS=$'\t' read -r scenario repetition position candidate <<<"${tuple}"
  mapfile -t values < <(scenario_values "${scenario}")
  sut_path="$(candidate_binary "${candidate}")"
  run_dir="${results_root}/${scenario}/repetition-${repetition}/${candidate}"
  if [[ -e "${run_dir}" ]]; then
    echo "refusing to overwrite candidate result: ${run_dir}" >&2
    exit 2
  fi
  mkdir -p "${run_dir}"
  started="$(date -u +%FT%TZ)"
  load_before="$(awk '{printf "%s,%s,%s", $1, $2, $3}' /proc/loadavg)"
  warm_lifecycle="${values[9]}"
  if [[ "${scenario}" != "w2" ]]; then warm_lifecycle=not-applicable; fi
  if env \
    MOSDNS_BINARY="${sut_path}" SCENARIO="${scenario}" RUN_MODE=official \
    RESULT_DIR="${run_dir}" HELPER_BINARY="${helper_path}" \
    CANDIDATE="${candidate}" REPETITION="${repetition}" PAIR_POSITION="${position}" \
    MANIFEST_PATH="${manifest_path}" MANIFEST_SHA256="${manifest_sha256}" \
    STAGE_DURATION_MS="${values[0]}" NORMAL_REFERENCE_QPS="${values[1]}" \
    COMMON_LOAD_QPS="${values[2]}" NEAR_SATURATION_QPS="${values[3]}" \
    OVERLOAD_QPS="${values[4]}" REQUEST_DEADLINE_MS="${values[5]}" \
    LATE_DRAIN_MS="${values[6]}" W2_CACHE_TTL_MS="${values[7]}" \
    W2_TTL_SAFETY_MARGIN_MS="${values[8]}" W2_WARM_LIFECYCLE="${warm_lifecycle}" \
    RECOVERY_MINIMUM_SAMPLES="${values[12]}" RECOVERY_P95_CEILING_US="${values[13]}" \
    RECOVERY_P99_CEILING_US="${values[14]}" HARNESS_CPU_SET="${values[10]}" \
    SUT_CPU_SET="${values[11]}" TEST_HOST_ALIAS="$(manifest_value host_alias)" \
    RUST_TOOLCHAIN_VERSION="$(manifest_value rust_toolchain)" \
    taskset --cpu-list "${values[10]}" "${runner_path}" \
    > "${run_dir}/driver-console.log" 2>&1; then
    runner_exit=0
  else
    runner_exit=$?
    runner_failures=$((runner_failures + 1))
  fi
  ended="$(date -u +%FT%TZ)"
  load_after="$(awk '{printf "%s,%s,%s", $1, $2, $3}' /proc/loadavg)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${scenario}" "${repetition}" "${position}" "${candidate}" \
    "${started}" "${ended}" "${runner_exit}" "${load_before}" "${load_after}" >> "${status_path}"
  printf 'official_attempt_complete\t%s\t%s\t%s\t%s\texit=%s\n' \
    "${scenario}" "${repetition}" "${position}" "${candidate}" "${runner_exit}"
done

echo "completed_candidate_attempts=${#tuples[@]}"
echo "runner_nonzero_exits=${runner_failures}"
echo "attempt_status=${status_path}"
