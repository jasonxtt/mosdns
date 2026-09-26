#!/usr/bin/env bash
set -euo pipefail
BASE=/root/mosdns-rust-phase5a-native-query-observability-545ba29
INPUT=${BASE}/measurement-v2
ROOT=${RESULT_ROOT:-${BASE}/results-m2-calibration}
RUNNER=${INPUT}/repo/scripts/run-phase5a-baseline.sh
HELPER=${INPUT}/phase5a-baseline-helper-v9
BEFORE=/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust
CONFIGS=${INPUT}/repo/tests/phase5a-baseline/configs
check_hash() { [[ "$(sha256sum "$2" | awk '{print $1}')" == "$1" ]]; }
[[ ! -e ${ROOT} ]]
check_hash ba864a787b639aa293a8e4a9981be03ca80d7a874f6cc182ed1a5889d77ad021 "${RUNNER}"
check_hash 397bdacf708a47894362d42b0cc12575a6c41ca34714db8d3d0be393ccbd654a "${INPUT}/repo/tests/phase5a-baseline/cmd/phase5a-baseline/main.go"
check_hash 254500ec00527850f0f137bcc7e9ac26ff4953d6600bfc00eb7a7fbc2afc436d "${HELPER}"
check_hash 370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa "${BEFORE}"
check_hash 3ef50747b1e0fea29a2f46b35e92c420a11a1a285dddd31366b24730fed2a4a0 "${BASE}/candidate-v12/slice3-v12-attempt-order-plan.tsv"
check_hash 3b3b4559b62d10088e45e41aa59b46bfa43b8897a8805e57edf08abbee622dc6 "${INPUT}/summarize-slice3-pilot-v2.py"
check_inputs() {
  check_hash f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729 "${CONFIGS}/forward-udp.yaml"
  check_hash 1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1 "${CONFIGS}/forward-tcp.yaml"
  check_hash 7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7 "${CONFIGS}/cache.yaml"
  check_hash 66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651 "${CONFIGS}/routing.yaml"
  check_hash 32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2 "${INPUT}/repo/tests/phase5a-baseline/workloads/forward.jsonl"
  check_hash 7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed "${INPUT}/repo/tests/phase5a-baseline/workloads/cache.jsonl"
  check_hash dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1 "${INPUT}/repo/tests/phase5a-baseline/workloads/routing.jsonl"
}
check_inputs
[[ "$("${HELPER}" version)" == phase5a-baseline-helper/v9 ]]
"${HELPER}" verify-cpu-sets --harness 1 --sut 0
mkdir -p "${ROOT}"
[[ "$(df -PT "${ROOT}" | awk 'NR==2 {print $2}')" == ext4 ]]
cp "${INPUT}/measurement-revision-v2.md" "${INPUT}/run-m2-calibration.sh" "${INPUT}/summarize-slice3-pilot-v2.py" "${ROOT}/"
printf 'mode=identical-baseline calibration; all slot labels use audit off\nGOMAXPROCS=1\nsource_commit=27cfc20d3bd27ba90ec67454ab343978d0cb7fed\nstart_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${ROOT}/run-audit.txt"
sha256sum "${RUNNER}" "${HELPER}" "${BEFORE}" "${ROOT}/run-m2-calibration.sh" "${ROOT}/measurement-revision-v2.md" >> "${ROOT}/run-audit.txt"
for batch in batch1 batch2; do
  mkdir -p "${ROOT}/${batch}"
  awk -F '\t' -v root="${ROOT}/${batch}" 'BEGIN {OFS="\t"} {print $1,$2,$3,$4,root "/" $1 "-r" $2 "-" $3}' "${BASE}/candidate-v12/slice3-v12-attempt-order-plan.tsv" > "${ROOT}/${batch}/attempt-order-plan.tsv"
  [[ "$(wc -l < "${ROOT}/${batch}/attempt-order-plan.tsv")" -eq 27 ]]
  printf 'scenario\trepetition\tvariant\tpair_position\tresult_dir\trunner_exit\n' > "${ROOT}/${batch}/attempt-order.tsv"
  sha256sum "${ROOT}/${batch}/attempt-order-plan.tsv" >> "${ROOT}/run-audit.txt"
done
if [[ ${PREFLIGHT_ONLY:-0} == 1 ]]; then
  printf 'preflight_only=1\n' >> "${ROOT}/run-audit.txt"
  exit 0
fi
for batch in batch1 batch2; do
  while IFS=$'\t' read -r scenario repetition variant position result; do
    check_inputs
    mkdir -p "${result}"
    printf 'loadavg_before_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" > "${result}/loadavg.tsv"
    warm=not-applicable
    if [[ ${scenario} == w2 ]]; then warm=same-process; fi
    set +e
    PHASE5A_MEASUREMENT_PROFILE=m2 GOMAXPROCS=1 RUN_MODE=pilot SCENARIO="${scenario}" MOSDNS_BINARY="${BEFORE}" RESULT_DIR="${result}" \
    HELPER_BINARY="${HELPER}" CANDIDATE=rust REPETITION="${repetition}" PAIR_POSITION="${position}" \
    RUN_ID="m2-${batch}-${scenario}-r${repetition}-${variant}" STAGE_DURATION_MS=3000 \
    NORMAL_REFERENCE_QPS=200 COMMON_LOAD_QPS=300 NEAR_SATURATION_QPS=350 OVERLOAD_QPS=400 \
    REQUEST_DEADLINE_MS=500 LATE_DRAIN_MS=100 W2_CACHE_TTL_MS=30000 W2_TTL_SAFETY_MARGIN_MS=500 \
    W2_WARM_LIFECYCLE="${warm}" SUT_CPU_SET=0 HARNESS_CPU_SET=1 TEST_HOST_ALIAS=mosdns-rust \
    "${RUNNER}" > "${result}/pilot.stdout.log" 2> "${result}/pilot.stderr.log"
    runner_exit=$?
    set -e
    printf 'loadavg_after_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" >> "${result}/loadavg.tsv"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "${scenario}" "${repetition}" "${variant}" "${position}" "${result}" "${runner_exit}" >> "${ROOT}/${batch}/attempt-order.tsv"
    printf 'attempt_utc=%s batch=%s scenario=%s repetition=%s slot=%s runner_exit=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${batch}" "${scenario}" "${repetition}" "${variant}" "${runner_exit}" | tee -a "${ROOT}/run-audit.txt"
    check_inputs
  done < "${ROOT}/${batch}/attempt-order-plan.tsv"
  python3 "${ROOT}/summarize-slice3-pilot-v2.py" "${ROOT}/${batch}" | tee "${ROOT}/${batch}/analysis-summary.txt"
done
printf 'finished_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "${ROOT}/run-audit.txt"
cd "${ROOT}"
find . -type f ! -name 'raw-file-hashes.sha256' ! -name 'raw-file-hashes.sha256.sha256' -print0 | sort -z | xargs -0 sha256sum > raw-file-hashes.sha256
sha256sum raw-file-hashes.sha256 > raw-file-hashes.sha256.sha256
sha256sum --quiet -c raw-file-hashes.sha256
