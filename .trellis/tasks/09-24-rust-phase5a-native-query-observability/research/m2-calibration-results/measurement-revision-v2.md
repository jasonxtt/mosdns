# Measurement correction M2 — reviewed calibration before acceptance

## Authorization and review boundary

On 2026-09-26 the user authorized correction and review of the measurement
scheme, establishment of stable controls, then resumption of acceptance.
Remain inside this query-observability task. Authorized units: (1) harness
correction and calibration protocol; (2) run and assess fixed controls;
(3) only after control qualification, pin and run a prospective V12 acceptance
protocol; (4) scoped A1–A6 review. No new task, archive, or deployment.

Reviewer: `codex://threads/01a0d43d-d0aa-7401-af0f-2ca3a45ba519`
(`002reviewer`), verified through native `read_thread`. This first review
submits only unit 1 and asks whether unit 2 may execute. It cannot award A5 or
approve runtime observability. A future acceptance protocol needs its own
committed identity and review before candidate results are collected.

## Identified instrumentation issues

The v8 sampler invokes `getconf CLK_TCK` separately for every target every
second during measured load. The runner pins all Go helper/fixture processes
to CPU 1 but does not pin their Go scheduler parallelism. Both are avoidable
sources of disturbance; neither has been established as the sole source of
the identical-binary false regressions.

Helper v9 resolves CLK_TCK once, before the stage start timestamp and resource
sampling; invalid/missing CLK_TCK fails the stage before load instead of
silently assuming 100. All sampled roles use that resolved frequency without
forking a process. Query traffic, sockets, correctness, resource interval,
counter and event oracles, ledger writes, and latency definition are unchanged.
The runner accepts v8 for archived protocols and v9 for this explicitly pinned
protocol. M2 sets `PHASE5A_MEASUREMENT_PROFILE=m2` and `GOMAXPROCS=1` for its
Go helpers/fixtures; the runner rejects missing/wrong GOMAXPROCS and records
both keys in each `environment.txt`. It never changes the Rust SUT.

## Fixed calibration experiment

Use only the archived Rust-before executable in every pairing slot, with
the original audit-disabled YAML. Labels are analysis slots, never claims of
an enabled-audit or candidate comparison. Keep the same 2-vCPU VM, CPU 0 SUT,
CPU 1 harness/fixtures, ext4 results, exact frozen YAML/workloads, 200/300/350/400
QPS, 3,000 ms stages, 500 ms deadline, 100 ms drain, 30,000 ms W2 TTL and 500 ms
TTL margin, same-process W2 warm lifecycle, and source/oracle semantics.

Run two fresh, fixed 27-attempt batches (six paired repetitions altogether).
Use the V12 balanced order in each batch; both plans and all tool hashes are
pinned before the first attempt. No reruns or replacements. Preserve all
supporting stages and rejected attempts. Before running, pin v9 source/build
identity, Linux executable hash, runner hash, driver hashes, analyzer hashes,
and original input hashes in the run supplement. The helper was built with
`GOTOOLCHAIN=go1.26.4 go build -trimpath` on Linux amd64.

Each batch must pass the original 63/63 primary correctness/validity gates.
Report all original paired guard outputs, not just aggregate summaries. To
qualify M2, no individual p95/p99 pair in either batch may exceed its original
guard (`pairs_above_guard` must be zero for every latency assessment).
Additionally, for every primary scenario/load and both slot comparisons,
the six paired log latency ratios must have a two-sided 90% Student-t interval
wholly within [-log(1.10), +log(1.10)]. Use t(5)=2.01504837333302, sample
standard deviation, SE=s/sqrt(6), and the six whole-attempt pairs as units;
do not treat thousands of correlated requests as independent repetitions.
This is an equivalence check at the 10% practical margin, not an overhead
claim. It is subject to independence/approximately normal log-ratio assumptions;
report per-pair values and drift rather than implying a general VM guarantee.

If a primary attempt is invalid, any individual original latency guard crosses, or any
equivalence interval is too wide/outside the margin, M2 is unqualified.
Archive the result and revise/review the next protocol; do not resample M2
until it passes. Longer observation windows or a stronger block design would
require a separately reviewed prospective revision. Do not widen the 10%
margin based on these measurements. CPU's 100-Hz uncertainty and RSS budgets
remain disclosed; they cannot qualify latency.

## Prospective acceptance boundary

If and only if the designated reviewer approves unit 1 and all fixed controls
qualify unit 2, prepare a separately reviewed acceptance supplement for the
exact V12 binary. It must preserve correctness/oracles and practical budgets,
include audit-on measurement and a contemporaneous identical-binary control,
state uncertainty and the decision rule before data, and preserve the original
V12 failed result. A calibration PASS alone never awards candidate PASS.

## Unit 1 validation

Linux regression test first failed because sampling spawned the fake `getconf`
executable. After correction, the complete helper package passed on Linux
and macOS; `go vet` passed on macOS. Tests cover no subprocess during per-role
sampling and valid, zero, negative, malformed, and missing CLK_TCK outputs.
Linux v9 SHA-256:
`254500ec00527850f0f137bcc7e9ac26ff4953d6600bfc00eb7a7fbc2afc436d`.
No Rust runtime source changed in this unit; no runtime gate is being rerun or
claimed from the harness tests.
