#!/usr/bin/env bash
set -euo pipefail

# Narrow W1 fresh-TCP fixture-direct calibration.  This is not an official
# candidate runner: the load generator talks directly to the deterministic
# fixture, while the script keeps an explicit fixture resource sample and a
# shell PID only as the helper's required non-fixture sampling anchor.
ROOT_DIR="${ROOT_DIR:-/root/mosdns-rust-phase5a-measurement-reliability-20260927/runner}"
HELPER_BINARY="${HELPER_BINARY:-/root/mosdns-rust-phase5a-measurement-reliability-20260927/bin/phase5a-baseline-helper}"
WORKLOAD="${WORKLOAD:-${ROOT_DIR}/tests/phase5a-baseline/workloads/forward.jsonl}"
RESULT_ROOT="${RESULT_ROOT:-/root/mosdns-rust-phase5a-measurement-reliability-20260927/results-slice2-w1-fixture-calibration}"
HARNESS_CPU_SET="${HARNESS_CPU_SET:-0}"
FIXTURE_ADDR="${FIXTURE_ADDR:-127.0.0.1:15454}"
RUN_PREFIX="${RUN_PREFIX:-slice2-w1-fixture}"
STAGE_DURATION_MS="${STAGE_DURATION_MS:-3000}"
REQUEST_DEADLINE_MS="${REQUEST_DEADLINE_MS:-500}"
LATE_DRAIN_MS="${LATE_DRAIN_MS:-100}"
COOLING_SECONDS="${COOLING_SECONDS:-5}"
REPEATS="${REPEATS:-2}"
NORMAL_QPS="${NORMAL_QPS:-200}"
COMMON_QPS="${COMMON_QPS:-300}"
NEAR_QPS="${NEAR_QPS:-350}"
OVERLOAD_QPS="${OVERLOAD_QPS:-400}"
PEAK_QPS="${PEAK_QPS:-500}"

if [[ ! -x "${HELPER_BINARY}" ]]; then
  echo "HELPER_BINARY is not executable: ${HELPER_BINARY}" >&2
  exit 2
fi
if [[ ! -f "${WORKLOAD}" ]]; then
  echo "WORKLOAD does not exist: ${WORKLOAD}" >&2
  exit 2
fi
if [[ -e "${RESULT_ROOT}" ]]; then
  echo "RESULT_ROOT must be fresh: ${RESULT_ROOT}" >&2
  exit 2
fi
if ! [[ "${REPEATS}" =~ ^[1-9][0-9]*$ && "${STAGE_DURATION_MS}" =~ ^[1-9][0-9]*$ && "${COOLING_SECONDS}" =~ ^[0-9]+$ ]]; then
  echo "REPEATS, STAGE_DURATION_MS, and COOLING_SECONDS are invalid" >&2
  exit 2
fi
if ! awk -v normal="${NORMAL_QPS}" -v common="${COMMON_QPS}" -v near="${NEAR_QPS}" -v overload="${OVERLOAD_QPS}" -v peak="${PEAK_QPS}" 'BEGIN { if (normal <= 0 || !(normal < common && common < near && near < overload) || peak < overload * 1.25) exit 1 }'; then
  echo "QPS ladder must be increasing and PEAK_QPS must be at least 1.25x OVERLOAD_QPS" >&2
  exit 2
fi

mkdir -p "${RESULT_ROOT}"
RESULT_ROOT="$(cd "${RESULT_ROOT}" && pwd)"
mkdir -p "${RESULT_ROOT}/attempts"

printf '%s\n' \
  "schema=slice2-w1-fixture-calibration-v1" \
  "helper_version=$("${HELPER_BINARY}" version)" \
  "helper_sha256=$(sha256sum "${HELPER_BINARY}" | awk '{print $1}')" \
  "workload_sha256=$(sha256sum "${WORKLOAD}" | awk '{print $1}')" \
  "harness_cpu_set=${HARNESS_CPU_SET}" \
  "fixture_addr=${FIXTURE_ADDR}" \
  "stage_duration_ms=${STAGE_DURATION_MS}" \
  "request_deadline_ms=${REQUEST_DEADLINE_MS}" \
  "late_drain_ms=${LATE_DRAIN_MS}" \
  "gomaxprocs=${GOMAXPROCS:-unset}" \
  "normal_qps=${NORMAL_QPS}" \
  "common_qps=${COMMON_QPS}" \
  "near_qps=${NEAR_QPS}" \
  "overload_qps=${OVERLOAD_QPS}" \
  "peak_qps=${PEAK_QPS}" \
  "repeats=${REPEATS}" \
  "cooling_seconds=${COOLING_SECONDS}" \
  "fixture_direct=true" \
  "sample_anchor_role=sut(shell-pid)" \
  "fixture_sample_role=fixture-1" \
  "started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  > "${RESULT_ROOT}/calibration-metadata.txt"

{
  printf 'kernel=%s\n' "$(uname -a)"
  printf 'hostname=%s\n' "$(hostname -f)"
  printf 'online_cpus=%s\n' "$(getconf _NPROCESSORS_ONLN)"
  printf 'open_file_limit=%s\n' "$(ulimit -n)"
  printf 'ephemeral_port_range=%s\n' "$(cat /proc/sys/net/ipv4/ip_local_port_range)"
  printf 'tcp_tw_reuse=%s\n' "$(cat /proc/sys/net/ipv4/tcp_tw_reuse)"
  printf 'tcp_fin_timeout=%s\n' "$(cat /proc/sys/net/ipv4/tcp_fin_timeout)"
  printf 'go_version=%s\n' "$(go version)"
  printf 'gomaxprocs=%s\n' "${GOMAXPROCS:-unset}"
  printf 'fixture_pid_anchor=%s\n' "$$"
} > "${RESULT_ROOT}/environment.txt"

record_tcp_state() {
  local repeat="$1" phase="$2"
  local time_wait established
  time_wait="$(ss -tan state time-wait | awk 'NR > 1 {count++} END {print count+0}')"
  established="$(ss -tan state established | awk 'NR > 1 {count++} END {print count+0}')"
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${repeat}" "${phase}" "${time_wait}" "${established}" >> "${RESULT_ROOT}/tcp-state.tsv"
  cat /proc/net/sockstat >> "${RESULT_ROOT}/sockstat-${repeat}-${phase}.txt"
}

fixture_pid=""
stop_fixture() {
  if [[ -n "${fixture_pid}" ]] && kill -0 "${fixture_pid}" 2>/dev/null; then
    kill -TERM "${fixture_pid}" 2>/dev/null || true
    wait "${fixture_pid}" 2>/dev/null || true
  fi
  fixture_pid=""
}
trap stop_fixture EXIT INT TERM

start_fixture() {
  local counter="$1"
  taskset --cpu-list "${HARNESS_CPU_SET}" "${HELPER_BINARY}" fixture \
    --network tcp --addr "${FIXTURE_ADDR}" --upstream-id forward --counter "${counter}" \
    > "${RESULT_ROOT}/fixture-${RUN_PREFIX}.stdout" \
    2> "${RESULT_ROOT}/fixture-${RUN_PREFIX}.stderr" &
  fixture_pid="$!"
  for _ in $(seq 1 100); do
    if [[ -s "${counter}" ]] && kill -0 "${fixture_pid}" 2>/dev/null; then
      "${HELPER_BINARY}" verify-affinity --pid "${fixture_pid}" --expected "${HARNESS_CPU_SET}"
      return 0
    fi
    sleep 0.05
  done
  echo "fixture did not become ready" >&2
  return 1
}

printf 'repeat\tstage\tqps\tduration_ms\ttime_wait\testablished\trunner_status\n' > "${RESULT_ROOT}/attempt-status.tsv"
printf 'repeat\tstage\tqps\tduration_ms\tscheduled\tsent\treceived\tcorrect_on_time\tcorrect_late\ttimeout\ttransport_error\tsender_shortfall\tmax_lag_us\tp50_us\tp95_us\tp99_us\teffective_throughput_qps\tfixture_max_rss_kib\tfixture_max_fd\n' > "${RESULT_ROOT}/calibration-observations.tsv"
printf 'timestamp_utc\trepeat\tphase\ttime_wait\testablished\n' > "${RESULT_ROOT}/tcp-state.tsv"

for repeat in $(seq 1 "${REPEATS}"); do
  attempt_root="${RESULT_ROOT}/attempts/repeat-${repeat}"
  mkdir -p "${attempt_root}"
  counter="${attempt_root}/fixture-forward.json"
  start_fixture "${counter}"
  record_tcp_state "${repeat}" start

  stage_plan=(
    "normal-reference|${NORMAL_QPS}"
    "common-load|${COMMON_QPS}"
    "near-saturation|${NEAR_QPS}"
    "overload|${OVERLOAD_QPS}"
    "recovery|${NORMAL_QPS}"
    "peak-envelope|${PEAK_QPS}"
  )
  for stage_spec in "${stage_plan[@]}"; do
    IFS='|' read -r stage qps <<<"${stage_spec}"
    stage_dir="${attempt_root}/${stage}"
    mkdir -p "${stage_dir}"
    baseline="${stage_dir}/counter-before.json"
    cp "${counter}" "${baseline}"
    record_tcp_state "${repeat}" "before-${stage}"
    run_status=0
    if taskset --cpu-list "${HARNESS_CPU_SET}" "${HELPER_BINARY}" run \
      --workload "${WORKLOAD}" --scenario w1 --transport tcp --addr "${FIXTURE_ADDR}" \
      --stage "${stage}" --qps "${qps}" --duration "${STAGE_DURATION_MS}ms" \
      --deadline "${REQUEST_DEADLINE_MS}ms" --late-drain "${LATE_DRAIN_MS}ms" \
      --run-id "${RUN_PREFIX}-r${repeat}" --fixture-session-id "${RUN_PREFIX}-fixture-r${repeat}" \
      --result "${stage_dir}" --sut-pid "$$" --fixture-pid "${fixture_pid}" \
      --request-ledger "${stage_dir}/requests.jsonl" --fail-on-error \
      > "${stage_dir}/runner.stdout.log" 2> "${stage_dir}/runner.stderr.log"; then
      run_status=0
    else
      run_status=$?
    fi
    record_tcp_state "${repeat}" "after-${stage}"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "${repeat}" "${stage}" "${qps}" "${STAGE_DURATION_MS}" "$(awk 'NR > 1 {count++} END {print count+0}' < <(ss -tan state time-wait))" "$(awk 'NR > 1 {count++} END {print count+0}' < <(ss -tan state established))" "${run_status}" >> "${RESULT_ROOT}/attempt-status.tsv"
    if [[ "${run_status}" -ne 0 ]]; then
      echo "calibration stage failed: repeat=${repeat} stage=${stage} status=${run_status}" >&2
      exit "${run_status}"
    fi
    "${HELPER_BINARY}" verify-samples --stage-result "${stage_dir}/stages.jsonl" --stage "${stage}" --expected-fixtures 1
    "${HELPER_BINARY}" verify-sender --stage-result "${stage_dir}/stages.jsonl" --stage "${stage}"
    "${HELPER_BINARY}" verify-counters --scenario w1 --workload "${WORKLOAD}" --counter "${counter}" --baseline "${baseline}" --stage-result "${stage_dir}/stages.jsonl" --stage "${stage}" --expect-delta
  done
  stop_fixture
  record_tcp_state "${repeat}" stopped
  if [[ "${repeat}" -lt "${REPEATS}" ]]; then
    sleep "${COOLING_SECONDS}"
    record_tcp_state "${repeat}" cooled
  fi
done

python3 - "${RESULT_ROOT}" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
out = root / "calibration-observations.tsv"
rows = []
for stage_path in sorted(root.glob("attempts/repeat-*/**/stages.jsonl")):
    repeat = stage_path.parts[-3].split("-")[-1]
    with stage_path.open() as stream:
        stage = json.loads(stream.readline())
    resources = []
    resource_path = stage_path.parent / "resource-samples.jsonl"
    with resource_path.open() as stream:
        for line in stream:
            sample = json.loads(line)
            if sample.get("Role") == "fixture-1" or sample.get("role") == "fixture-1":
                resources.append(sample)
    def value(key):
        return stage.get(key, stage.get({
            "TargetQPS": "target_qps", "DurationMS": "duration_ms", "Counters": "counters",
            "SenderLagMaxUS": "sender_lag_max_us", "P50US": "p50_us", "P95US": "p95_us",
            "P99US": "p99_us", "EffectiveThroughput": "effective_throughput_qps",
        }.get(key, key)))
    counters = value("Counters") or {}
    def counter(key):
        return counters.get(key, counters.get({
            "Scheduled": "scheduled", "Sent": "sent", "Received": "received",
            "CorrectOnTime": "correct_on_time", "CorrectLate": "correct_late",
            "Timeout": "timeout", "TransportError": "transport_error",
            "SenderShortfall": "sender_shortfall",
        }.get(key, key), 0))
    def sample_max(key):
        aliases = {"RSSKiB": "rss_kib", "FDCount": "fd_count"}
        return max((item.get(key, item.get(aliases[key], 0)) for item in resources), default=0)
    rows.append([
        repeat, stage.get("Stage", stage.get("stage", "")), value("TargetQPS"), value("DurationMS"),
        counter("Scheduled"), counter("Sent"), counter("Received"), counter("CorrectOnTime"),
        counter("CorrectLate"), counter("Timeout"), counter("TransportError"), counter("SenderShortfall"),
        value("SenderLagMaxUS"), value("P50US"), value("P95US"), value("P99US"),
        value("EffectiveThroughput"), sample_max("RSSKiB"), sample_max("FDCount"),
    ])
with out.open("a") as stream:
    for row in rows:
        stream.write("\t".join(str(item) for item in row) + "\n")
PY

record_tcp_state all finished
printf 'finished_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "${RESULT_ROOT}/calibration-metadata.txt"
cd "${RESULT_ROOT}"
find . -type f ! -name 'raw-file-hashes.sha256' -print0 | sort -z | xargs -0 sha256sum > raw-file-hashes.sha256
sha256sum raw-file-hashes.sha256 > raw-file-hashes.sha256.sha256
sha256sum --quiet -c raw-file-hashes.sha256
