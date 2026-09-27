# Reviewed execution plan v1 — Slice 0 / G0

Status: `Slice 0 C2C PASS; G0 profiler remediation PASS; Slice 1 offline work
admitted; G2 pending`. This plan freezes method and safety ceilings only; it
contains no candidate result and no performance claim. The host now has an
accepted software-event profiler fallback; hardware-PMU limitations remain
explicit.

## Identity and scope

- Task: `rust-phase5a-measurement-reliability`
- Branch/base: `rust` / `rust`
- Planning source HEAD: `5478015f7998be5335a7019915af558da5c74b4b`
- Go reference source: `5b1eca69e0668ad1ddb6db88c0f39202557d5b98`
- Official scope: W1 fresh TCP connection/request and W2 UDP two-key
  warm-cache only; W1 UDP and W3 are correctness regressions; W2 cold is
  correctness-only. Audit stays disabled.
- Allowed product-side edits: only
  `tests/phase5a-baseline/cmd/phase5a-baseline/`, its README, the narrow
  `scripts/run-phase5a-reliability.sh`, this task's research, and the new
  measurement report. No Rust runtime/Send, Go business code, API/WebUI,
  production, hybrid retirement, or generic load framework.

## Frozen safety ceilings

These are upper bounds and stop rules, not target capacity or SLA values.
Actual offered points and technical latency/percentage thresholds must be
formed from the Go reference pilot plus calibration and sampling error, before
any Rust candidate result is inspected.

| Budget | Frozen ceiling / rule |
| --- | --- |
| W1 pilot | One single-point pilot per candidate/session, maximum 30 s stage, 500 ms request service budget, 100 ms late collection, 5 s assessment window. Pilot is not an official pair. |
| W2 pilot | One independent-prefill point per candidate/session, maximum 10 s measured stage; per-key prefill-to-final response must remain below 30 s TTL minus 500 ms safety margin and request budget. No recovery claim. |
| Official matrix | 2–4 common points per scenario, exactly 3 prearranged Go/Rust pairs per point, interleaved order frozen before launch; at most one reviewed full-matrix retry. All attempts remain archived. |
| Profiles | At most 2 points per candidate/scenario, in separate runs from official latency. A profile is invalid when the required process-directed profiler or symbols are unavailable. The current host accepts process-directed `perf` software-event call-chain sampling; hardware PMU counters are unavailable and must be disclosed. |
| CPU claim | SUT and generator/fixture must use disjoint masks on CPUs 0 and 1 when execution is permitted; no multi-core scaling claim. If actual affinity or isolation cannot be verified, stop. |
| FD / in-flight | Start with a hard task-side ceiling of 512 FDs and 256 in-flight fresh-TCP requests, leaving at least 512 FD slots for the host/service and non-request evidence. Recalculate only from measured owned-process usage during G1; never silently raise it. |
| RSS | Start with a 256 MiB cap per generator/fixture role and 768 MiB combined harness/fixture task budget. SUT RSS is sampled and reported separately. Breach stops the attempt and invalidates load. |
| Disk | Use a task-owned directory on ext4, never `/tmp`; reserve at least 1 GiB free on the filesystem and cap new raw/derived evidence for this task at 512 MiB per attempt root. Do not delete unrelated files to satisfy the floor. |
| Evidence queue | The implementation must use finite queue/task/log buffers; exact record/byte limits are part of Slice 1's reviewed schema and must fit the disk/RSS ceilings. Queue-full, writer error, or non-error blocked sink is harness failure, never service overload. |
| Timeouts | Preflight ≤60 s; each bounded stage includes its service/late budgets plus ≤30 s drain/cleanup; owned process shutdown and port rebind must complete within 60 s. A non-reaped PID/start identity is cleanup failure. |
| Stop conditions | Wrong/duplicate response, protocol mismatch, identity/hash mismatch, source drift, queue/evidence loss, missing required sample, port/FD/RSS/disk cap, sender shortfall, fixture saturation, or unowned process causes immediate stop and preserved invalid evidence. |

The 256 in-flight and 512-FD ceilings are conservative host-safety bounds
derived from the observed 1024 FD limit, not a claim that the sender can sustain
the corresponding rate. G1 must measure sender/fixture headroom, and W1 G2
must additionally qualify the full fresh-TCP ladder's cumulative connections,
TIME_WAIT/reuse behavior, errno and cooling—not only a peak-rate point.

## Threshold and identity rules

1. Freeze Go-only reference pilot inputs and sampling error first. Numeric
   normal-band, on-time-rate, tail-latency and recovery thresholds are then
   written into the official manifest. Rust results cannot alter them.
2. `planned_at` is the monotonic stage start plus slot offset;
   `service_deadline = planned_at + request_deadline`; connect, queue, full
   write and read consume the same absolute budget. `collection_deadline` is
   only for a request whose complete DNS frame was already accepted by the
   send API. It cannot create an expired new send.
3. W1 generator qualification covers the entire official ladder duration and
   cumulative fresh connection count, with the same destination layout,
   source/network strategy, port range and reuse settings. Any change
   invalidates the qualification.
4. W1 recovery requires objective valid-load overload first, then the same
   PID/start returning to the reference budget for three consecutive windows.
   Without the first condition the result is
   `indeterminate-no-overload-evidence`. W2 independent warm points never
   supply same-process recovery evidence.
5. Three latency views are mandatory in raw/summary: planned-slot-to-finish,
   dispatch-to-finish, and write-start-to-finish. Each records its start offset,
   sample count and failure denominator; requests that never reach dispatch or
   write never get invented samples.

## G0 evidence package and review boundary

The G0 package consists of:

- `slice0-environment-preflight.md` — local identity, remote read-only
  preflight, source/tool availability and the initial profiler gap.
- this execution plan — finite ceilings, stop rules, identity and threshold
  formation.
- the existing task planning files and their SHA-256 values from revision 3.
- an exact command/hash index produced after this file is finalized.

The initial G0 C2C review returned `FINAL: PASS` after the transcript
remediation recorded in `research/c2c-slice0-review.md`. At that point it
permitted only the already-authorized Slice 1 helper work because the host had
no process profiler. The follow-up remediation below was separately reviewed
and now resolves that capability gate; it does not itself create a hotspot or
capacity result. A5 remains incomplete until the separate profile runs and
their evidence are accepted.

## Profiler remediation follow-up

The initially missing profiler packages were installed on `mosdns-rust` after
explicit user authorization without lowering `perf_event_paranoid`. The host
now supports process-directed software-event call-chain sampling with
`perf record -e cpu-clock`; hardware `cycles`/`instructions` counters returned
zero in the controlled check. See
`research/slice0-profiler-remediation.md` and the accepted
`research/c2c-slice0-profiler-review.md`. G0 now accepts the software-event
profile path with its disclosed PMU limitation; this still produces no hotspot
or capacity conclusion by itself.

## Unchanged historical references

The existing baseline's fixed workload identities remain the current values:

```text
7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed  tests/phase5a-baseline/workloads/cache.jsonl
32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2  tests/phase5a-baseline/workloads/forward.jsonl
dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1  tests/phase5a-baseline/workloads/routing.jsonl
```

These are inputs only; no archived result is reinterpreted and no historical
hash is reused for a newly built binary.
