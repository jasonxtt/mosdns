#!/usr/bin/env bash
set -euo pipefail
BASE=/root/mosdns-rust-phase5a-native-query-observability-545ba29
ROOT=${BASE}/results-self-control-w1
INPUT=${BASE}/self-control-w1
RUNNER=${BASE}/runner-root/scripts/run-phase5a-baseline.sh
BEFORE=/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust
HELPER=/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/phase5a-baseline-helper
CONFIG=${BASE}/runner-root/tests/phase5a-baseline/configs/forward-tcp.yaml
check_hash() {
  [[ "$(sha256sum "$2" | awk '{print $1}')" == "$1" ]]
}
[[ ! -e ${ROOT} ]]
check_hash dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8 "${RUNNER}"
check_hash 370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa "${BEFORE}"
check_hash df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065 "${HELPER}"
check_hash 1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1 "${CONFIG}"
check_hash 3b3b4559b62d10088e45e41aa59b46bfa43b8897a8805e57edf08abbee622dc6 "${INPUT}/summarize-slice3-pilot-v2.py"
"${HELPER}" verify-cpu-sets --harness 1 --sut 0
mkdir -p "${ROOT}"
cp "${INPUT}/run-slice3-self-control-w1.sh" "${INPUT}/slice3-self-control-protocol.md" "${INPUT}/summarize-slice3-pilot-v2.py" "${ROOT}/"
"${HELPER}" validate-binary --path "${BEFORE}" > "${ROOT}/same-binary.json"
printf 'diagnostic=all slots identical Rust-before binary and audit-disabled YAML\nstart_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${ROOT}/run-audit.txt"
sha256sum "${BEFORE}" "${CONFIG}" "${RUNNER}" "${HELPER}" "${ROOT}/run-slice3-self-control-w1.sh" "${ROOT}/slice3-self-control-protocol.md" >> "${ROOT}/run-audit.txt"
printf 'scenario\trepetition\tvariant\tpair_position\tresult_dir\trunner_exit\n' > "${ROOT}/attempt-order.tsv"
awk -F '\t' '$1 == "w1-tcp" {print $1 "\t" $2 "\t" $3 "\t" $4}' "${BASE}/candidate-v12/slice3-v12-attempt-order-plan.tsv" > "${ROOT}/slot-order.tsv"
[[ "$(wc -l < "${ROOT}/slot-order.tsv")" -eq 9 ]]
while IFS=$'\t' read -r scenario repetition variant position; do
  result=${ROOT}/${scenario}-r${repetition}-${variant}
  mkdir -p "${result}"
  printf 'loadavg_before_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" > "${result}/loadavg.tsv"
  set +e
  RUN_MODE=pilot SCENARIO=w1-tcp MOSDNS_BINARY="${BEFORE}" RESULT_DIR="${result}" \
  HELPER_BINARY="${HELPER}" CANDIDATE=rust REPETITION="${repetition}" PAIR_POSITION="${position}" \
  RUN_ID="self-control-w1-r${repetition}-${variant}" STAGE_DURATION_MS=3000 \
  NORMAL_REFERENCE_QPS=200 COMMON_LOAD_QPS=300 NEAR_SATURATION_QPS=350 OVERLOAD_QPS=400 \
  REQUEST_DEADLINE_MS=500 LATE_DRAIN_MS=100 W2_CACHE_TTL_MS=30000 W2_TTL_SAFETY_MARGIN_MS=500 \
  W2_WARM_LIFECYCLE=not-applicable SUT_CPU_SET=0 HARNESS_CPU_SET=1 TEST_HOST_ALIAS=mosdns-rust \
  "${RUNNER}" > "${result}/pilot.stdout.log" 2> "${result}/pilot.stderr.log"
  status=$?
  set -e
  printf 'loadavg_after_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" >> "${result}/loadavg.tsv"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "${scenario}" "${repetition}" "${variant}" "${position}" "${result}" "${status}" >> "${ROOT}/attempt-order.tsv"
  printf 'slot_utc=%s repetition=%s slot=%s runner_exit=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${repetition}" "${variant}" "${status}" | tee -a "${ROOT}/run-audit.txt"
  check_hash 1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1 "${CONFIG}"
done < "${ROOT}/slot-order.tsv"
printf 'finished_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "${ROOT}/run-audit.txt"
python3 "${ROOT}/summarize-slice3-pilot-v2.py" "${ROOT}" | tee "${ROOT}/analysis-summary.txt"
cd "${ROOT}"
find . -type f ! -name 'raw-file-hashes.sha256' ! -name 'raw-file-hashes.sha256.sha256' -print0 | sort -z | xargs -0 sha256sum > raw-file-hashes.sha256
sha256sum raw-file-hashes.sha256 > raw-file-hashes.sha256.sha256
sha256sum --quiet -c raw-file-hashes.sha256
