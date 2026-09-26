# M4 prospective bounded harness GC control

User authorization and reviewer remain the existing measurement-correction
scope and designated 002reviewer. M2 and M3 stay unqualified, V12 stays failed,
A5 stays unmet. This unit requests only fresh fixed calibration permission.

## One intervention, unchanged decision rule

M3's two W1 batches were fully correct, yet only 1/8 equivalence intervals
qualified. A longer observation window alone was insufficient. Source shows
per-query socket/DNS/ledger allocation in the Go generator/fixtures sharing
CPU1; sampled RSS peaks were 13–18MiB. Default Go automatic GC can disturb
timing, but no causal diagnosis has been established. Test this bounded
intervention prospectively, not a guarantee or a candidate optimization.

Set GOGC=off and GODEBUG=gctrace=1 for the isolated test runner/helpers/
fixtures, with GOMEMLIMIT explicitly removed. The Rust SUT has no Go runtime
and is unchanged. Existing system services retain their environment. Go's
[GC guide](https://go.dev/doc/gc-guide) documents that disabling automatic GC
requires avoiding an applicable memory limit; it increases memory use.
Each helper's measured stage lasts25 seconds, each attempt is finite, and
fresh processes are terminated after its prescribed stages. Preflight needs
at least2GiB MemAvailable. Sampled Go-role peak RSS must be positive and
at most256MiB per role, all roles covered, or calibration fails. This is a
measurement resource limit, not a hard RSS allocator cap or Rust memory claim.
No live GC telemetry or additional per-query sampler is introduced.

After each attempt, a pinned offline collector retains hashes and GC-trace
counts from generator/verifier stderr and every fixture stderr, plus sampled
Go-role RSS peaks from all stages. Missing logs/resources fail qualification.
Any GC trace or role over the limit fails it. All raw logs remain in the raw
manifest, including empty logs. Absence of traces is an observation under the
fixed environment, not proof of general VM stability or GC's prior causality.

All M3 settings remain: helperv9, original baseline binary and frozen configs/
workloads, CPU0 SUT/CPU1 harness, GOMAXPROCS1, two25-second primary points at
200/400QPS,500ms deadline/100ms drain, W2 unique cold200 and independent warm
prefill with original TTL30000/margin500 per-key check, exact DNS/counter/
routing-event/latency semantics, recovery indeterminate. Legacy/M2/M3 remain
available and keep their profile semantics; their remote sources stay frozen.

Two fixed27 plans from the V12 balanced order, sealed before traffic. W1's18
slots execute first. Both batches must have exact36 valid primary rows,
270000 correct-on-time requests, all runner exits0, all profile/binary/window/
GC/resource evidence correct, zero individual old p95/p99 guard crossings,
all eight six-pair90% t(5) log equivalence intervals inside ±log1.10. Only if
W1 qualifies run remaining36 slots, then require exact126 primary rows and
all28 intervals under that same rule. No retries, replacements, exclusions,
order changes, widened margins, candidate data or old-result reinterpretation.
Full qualify permits only preparing a separately reviewed V12 supplement.

## Evidence and execution boundary

Fresh `measurement-v4` tools and `results-m4-calibration` results. Exact
runner/helper/source/config/workload/analyzer/qualifier/collector/protocol
hashes checked by driver. Pin source parent/head and zero-attempt preflight
after scoped review PASS, then execute driver once. Archive complete raw,
manifest, selected identity/derived/GC evidence, every failed or unexecuted
slot. Existing SUT correctness/resources and CPU tick uncertainty remain.

Local full helper tests, preserved M2/M3 tests and M4 synthetic gate/trace/
resource/profile tests pass; shell syntax checks pass. Linux validation must
pass before traffic. No M4 measured attempts exist at review submission.
Any failure requires reporting this exact result and a separate prospective
review, never resampling this revision until it passes.
