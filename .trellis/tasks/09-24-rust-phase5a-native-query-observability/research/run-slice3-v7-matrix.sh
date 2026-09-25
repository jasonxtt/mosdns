#!/usr/bin/env bash
set -euo pipefail

BASE=/root/mosdns-rust-phase5a-native-query-observability-545ba29
RESULT_ROOT="${BASE}/results-v7"
RUNNER="${BASE}/runner-root/scripts/run-phase5a-baseline.sh"
CONFIG_DIR="${BASE}/runner-root/tests/phase5a-baseline/configs"
HELPER=/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/phase5a-baseline-helper
BEFORE=/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust
AFTER="${BASE}/candidate-v7/rust/target/release/mosdns"
OVERLAY_DIR="${BASE}/audit-on-overlays"
PLAN="${BASE}/candidate-v7/slice3-v7-attempt-order-plan.tsv"
INPUT_DIR="${BASE}/candidate-v7"
AUDIT="${RESULT_ROOT}/run-audit.txt"
ORDER="${RESULT_ROOT}/attempt-order.tsv"
ORIGINALS="${RESULT_ROOT}/.input-originals"

if [[ -e "${RESULT_ROOT}" ]] && [[ -n "$(find "${RESULT_ROOT}" -mindepth 1 -print -quit)" ]]; then
  echo "refusing to overwrite non-empty V7 result directory: ${RESULT_ROOT}" >&2
  exit 2
fi
mkdir -p "${RESULT_ROOT}" "${ORIGINALS}"
cp "${INPUT_DIR}/slice3-candidate-v7-identity.md" "${RESULT_ROOT}/candidate-v7-identity.md"
cp "${INPUT_DIR}/slice3-v7-attempt-order-plan.tsv" "${RESULT_ROOT}/attempt-order-plan.tsv"
cp "${INPUT_DIR}/run-slice3-v7-matrix.sh" "${RESULT_ROOT}/run-slice3-v7-matrix.sh"
cp "${INPUT_DIR}/summarize-slice3-pilot-v2.py" "${RESULT_ROOT}/summarize-slice3-pilot-v2.py"
cp "${INPUT_DIR}/performance-manifest.md" "${RESULT_ROOT}/performance-manifest.md"
cp "${INPUT_DIR}/performance-manifest.sha256" "${RESULT_ROOT}/performance-manifest.sha256"
[[ "$(sha256sum "${INPUT_DIR}/performance-manifest.md" | awk '{print $1}')" == \
  "b687be8014a78008127d9df818123c839b69ffdda4764eb99c174b9e085965a8" ]]
[[ "$(cat "${INPUT_DIR}/performance-manifest.sha256")" == \
  "b687be8014a78008127d9df818123c839b69ffdda4764eb99c174b9e085965a8  .trellis/tasks/09-24-rust-phase5a-native-query-observability/research/performance-manifest.md" ]]
cp "${CONFIG_DIR}/forward-udp.yaml" "${ORIGINALS}/forward-udp.yaml"
cp "${CONFIG_DIR}/forward-tcp.yaml" "${ORIGINALS}/forward-tcp.yaml"
cp "${CONFIG_DIR}/cache.yaml" "${ORIGINALS}/cache.yaml"
cp "${CONFIG_DIR}/routing.yaml" "${ORIGINALS}/routing.yaml"

restore_configs() {
  cp "${ORIGINALS}/forward-udp.yaml" "${CONFIG_DIR}/forward-udp.yaml"
  cp "${ORIGINALS}/forward-tcp.yaml" "${CONFIG_DIR}/forward-tcp.yaml"
  cp "${ORIGINALS}/cache.yaml" "${CONFIG_DIR}/cache.yaml"
  cp "${ORIGINALS}/routing.yaml" "${CONFIG_DIR}/routing.yaml"
}
trap restore_configs EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

check_hash() {
  local expected="$1" path="$2" actual
  actual="$(sha256sum "${path}" | awk '{print $1}')"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "SHA-256 mismatch: ${path}: expected=${expected} actual=${actual}" >&2
    return 1
  fi
}

check_hash dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8 "${RUNNER}"
check_hash 2dec4788eaa3dad251061e380cbd8c32c9122285b852acf8b9a0b2eb81f64da1 "${BASE}/runner-root/tests/phase5a-baseline/cmd/phase5a-baseline/main.go"
check_hash df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065 "${HELPER}"
check_hash 370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa "${BEFORE}"
check_hash 6c799a905cd4bc33da241238473bc449abc7ad077e430723f79d514a013bd56b "${AFTER}"
check_hash d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca "${BASE}/candidate-v7/rust/Cargo.lock"
check_hash f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729 "${CONFIG_DIR}/forward-udp.yaml"
check_hash 1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1 "${CONFIG_DIR}/forward-tcp.yaml"
check_hash 7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7 "${CONFIG_DIR}/cache.yaml"
check_hash 66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651 "${CONFIG_DIR}/routing.yaml"
check_hash 32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2 "${BASE}/runner-root/tests/phase5a-baseline/workloads/forward.jsonl"
check_hash 7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed "${BASE}/runner-root/tests/phase5a-baseline/workloads/cache.jsonl"
check_hash dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1 "${BASE}/runner-root/tests/phase5a-baseline/workloads/routing.jsonl"
check_hash 72db879e9fbee4fb87400b54da31dfd41f82f6766dbe6081ab3d4690e42df24c "${OVERLAY_DIR}/forward-tcp.yaml"
check_hash c973586aee0f0381d96256afb305ef773f240a4aab7033303990c013d4cf7158 "${OVERLAY_DIR}/cache.yaml"
check_hash 0cc96555e135529a9acb7b49dbbefdf94d1d8b9e8cf2c2d4c37a980e099521b5 "${OVERLAY_DIR}/routing.yaml"
[[ "$(rustc --version)" == "rustc 1.95.0 (59807616e 2026-04-14)" ]]
[[ "$("${HELPER}" version)" == "phase5a-baseline-helper/v8" ]]
RESULT_FS="$(df -PT "${RESULT_ROOT}" | awk 'NR == 2 {print $2}')"
case "${RESULT_FS}" in
  tmpfs|ramfs|"") echo "V7 results must use a disk-backed filesystem (found ${RESULT_FS:-unknown})" >&2; exit 2 ;;
esac
"${HELPER}" validate-binary --path "${BEFORE}" > "${RESULT_ROOT}/before-binary.json"
"${HELPER}" validate-binary --path "${AFTER}" > "${RESULT_ROOT}/after-binary.json"
"${HELPER}" verify-cpu-sets --harness 1 --sut 0

for name in forward-tcp cache routing; do
  config="${ORIGINALS}/${name}.yaml"
  overlay="${OVERLAY_DIR}/${name}.yaml"
  if [[ "$(grep -c 'enable_audit: false' "${config}")" != 1 ]]; then
    echo "expected one audit-off setting in ${config}" >&2
    exit 2
  fi
  if ! diff -u <(sed 's/enable_audit: false/enable_audit: true/' "${config}") "${overlay}"; then
    echo "audit-on overlay contains changes beyond the frozen listener flag: ${overlay}" >&2
    exit 2
  fi
done

printf 'preflight_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${AUDIT}"
printf 'host=%s\nkernel=%s\nonline_cpus=%s\nmem_bytes=%s\nopen_file_limit=%s\nresult_filesystem=%s\nloadavg=%s\n' \
  "$(hostname -f)" "$(uname -a)" "$(getconf _NPROCESSORS_ONLN)" \
  "$(free -b | awk '/^Mem:/ {print $2}')" "$(ulimit -n)" "${RESULT_FS}" "$(cat /proc/loadavg)" >> "${AUDIT}"
printf 'runner_sha256=%s\nhelper_sha256=%s\nbefore_sha256=%s\nafter_sha256=%s\n' \
  "$(sha256sum "${RUNNER}" | awk '{print $1}')" \
  "$(sha256sum "${HELPER}" | awk '{print $1}')" \
  "$(sha256sum "${BEFORE}" | awk '{print $1}')" \
  "$(sha256sum "${AFTER}" | awk '{print $1}')" >> "${AUDIT}"
printf '%s\n' 'mode=pilot (the frozen manifest specifies pilot mode because the archived official driver digest differs)' \
  'matrix=27 balanced attempts; W1 TCP, W2 same-process cold/warm, W3' \
  'stages=200,300,350,400 QPS plus 200 QPS health; 3000 ms each; deadline 500 ms; late drain 100 ms' \
  'w2_ttl_ms=30000; w2_ttl_safety_margin_ms=500; sut_cpu=0; harness_cpu=1' \
  'production_service=untouched; no connection to mosdns production service' >> "${AUDIT}"
printf 'driver_sha256=%s\nplan_sha256=%s\nidentity_sha256=%s\nanalysis_script_sha256=%s\nmanifest_sha256=%s\n' \
  "$(sha256sum "${RESULT_ROOT}/run-slice3-v7-matrix.sh" | awk '{print $1}')" \
  "$(sha256sum "${RESULT_ROOT}/attempt-order-plan.tsv" | awk '{print $1}')" \
  "$(sha256sum "${RESULT_ROOT}/candidate-v7-identity.md" | awk '{print $1}')" \
  "$(sha256sum "${RESULT_ROOT}/summarize-slice3-pilot-v2.py" | awk '{print $1}')" \
  "$(sha256sum "${RESULT_ROOT}/performance-manifest.md" | awk '{print $1}')" >> "${AUDIT}"
printf 'scenario\trepetition\tvariant\tpair_position\tresult_dir\trunner_exit\n' > "${ORDER}"

ports=(15353 15354 15355 15356 15453 15454 15455 15456 15457 15458)
check_ports_free() {
  local busy
  busy="$(ss -H -lntu | awk '{print $5}' | awk -F: '{print $NF}' | sort -u)"
  for port in "${ports[@]}"; do
    if grep -Fxq "${port}" <<<"${busy}"; then
      echo "port ${port} is already listening" >&2
      return 1
    fi
  done
  printf 'ports_free_utc=%s ports=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${ports[*]}"
}

while IFS=$'\t' read -r scenario repetition variant position result_dir; do
  [[ -n "${scenario}" ]] || continue
  if [[ "${result_dir}" != "${RESULT_ROOT}/"* ]]; then
    echo "attempt result escaped V7 result root: ${result_dir}" >&2
    exit 2
  fi
  case "${scenario}:${variant}" in
    w1-tcp:*) config="${CONFIG_DIR}/forward-tcp.yaml"; overlay="${OVERLAY_DIR}/forward-tcp.yaml" ;;
    w2:*) config="${CONFIG_DIR}/cache.yaml"; overlay="${OVERLAY_DIR}/cache.yaml" ;;
    w3:*) config="${CONFIG_DIR}/routing.yaml"; overlay="${OVERLAY_DIR}/routing.yaml" ;;
    *) echo "unsupported planned attempt: ${scenario} ${variant}" >&2; exit 2 ;;
  esac
  restore_configs
  if [[ "${variant}" == "after_on" ]]; then
    cp "${overlay}" "${config}"
  fi
  check_ports_free > "${result_dir}.port-preflight.tmp"
  mkdir -p "${result_dir}"
  mv "${result_dir}.port-preflight.tmp" "${result_dir}/port-check.txt"
  printf 'loadavg_before_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" > "${result_dir}/loadavg.tsv"
  if [[ "${variant}" == "before_off" ]]; then
    binary="${BEFORE}"
  else
    binary="${AFTER}"
  fi
  warm_lifecycle=not-applicable
  if [[ "${scenario}" == "w2" ]]; then
    warm_lifecycle=same-process
  fi
  run_id="phase5a-v7-${scenario}-r${repetition}-${variant}-$(date -u +%Y%m%dT%H%M%SZ)"
  set +e
  RUN_MODE=pilot \
  SCENARIO="${scenario}" \
  MOSDNS_BINARY="${binary}" \
  RESULT_DIR="${result_dir}" \
  HELPER_BINARY="${HELPER}" \
  CANDIDATE=rust \
  REPETITION="${repetition}" \
  PAIR_POSITION="${position}" \
  RUN_ID="${run_id}" \
  STAGE_DURATION_MS=3000 \
  NORMAL_REFERENCE_QPS=200 \
  COMMON_LOAD_QPS=300 \
  NEAR_SATURATION_QPS=350 \
  OVERLOAD_QPS=400 \
  REQUEST_DEADLINE_MS=500 \
  LATE_DRAIN_MS=100 \
  W2_CACHE_TTL_MS=30000 \
  W2_TTL_SAFETY_MARGIN_MS=500 \
  W2_WARM_LIFECYCLE="${warm_lifecycle}" \
  SUT_CPU_SET=0 \
  HARNESS_CPU_SET=1 \
  TEST_HOST_ALIAS=mosdns-rust \
    "${RUNNER}" > "${result_dir}/pilot.stdout.log" 2> "${result_dir}/pilot.stderr.log"
  runner_exit=$?
  set -e
  restore_configs
  printf 'loadavg_after_utc=%s\n%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" >> "${result_dir}/loadavg.tsv"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "${scenario}" "${repetition}" "${variant}" "${position}" "${result_dir}" "${runner_exit}" >> "${ORDER}"
  printf 'attempt_utc=%s scenario=%s repetition=%s variant=%s pair_position=%s runner_exit=%s result=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${scenario}" "${repetition}" "${variant}" "${position}" "${runner_exit}" "${result_dir}" | tee -a "${AUDIT}"
  check_hash f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729 "${CONFIG_DIR}/forward-udp.yaml"
  check_hash 1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1 "${CONFIG_DIR}/forward-tcp.yaml"
  check_hash 7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7 "${CONFIG_DIR}/cache.yaml"
  check_hash 66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651 "${CONFIG_DIR}/routing.yaml"
done < "${PLAN}"

restore_configs
printf 'matrix_finished_utc=%s\nloadavg=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(cat /proc/loadavg)" >> "${AUDIT}"
python3 "${RESULT_ROOT}/summarize-slice3-pilot-v2.py" "${RESULT_ROOT}" | tee "${RESULT_ROOT}/analysis-summary.txt"
(
  cd "${RESULT_ROOT}"
  find . -type f ! -name 'raw-file-hashes.sha256' ! -name 'raw-file-hashes.sha256.sha256' -print0 \
    | sort -z \
    | xargs -0 sha256sum > raw-file-hashes.sha256
  sha256sum raw-file-hashes.sha256 > raw-file-hashes.sha256.sha256
  sha256sum --quiet -c raw-file-hashes.sha256
)
