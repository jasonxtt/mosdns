# M3 prospective longer-window controls

## Scope

User authorization remains the measurement correction within the active
Phase 5A query-observability task. M2 is unqualified (54 fixed attempts,
126 valid primary rows, 108,000 correct on time, only 1/28 equivalent latency
intervals). Preserve all M2/V12 verdicts. This revision asks the designated
002reviewer to approve fresh calibration only, not candidate acceptance/A5.
No Rust source or binary changes, new task, production deployment or finish.

## Prospective design, before data

Keep the original 10% practical equivalence margin and every individual old
latency guard check; no retrospective budget widening. The 3-second window
gives only about 6/12 tail observations at p99 at 200/400 QPS. Increase it to
25 seconds (5,000/10,000 requests, about 50/100 tail observations). This
motivates a precision experiment, not a proven cause or guaranteed improvement.
Latency remains connection-start through full response; sockets, deadlines,
sender schedule, DNS workload/config/answers, counters and routing event
oracles stay frozen. Helper v9, CPU0 SUT/CPU1 harness, GOMAXPROCS=1, ext4,
the same isolated 2-vCPU benchmark VM and original Rust-before binary apply.
CPU 100-Hz uncertainty and RSS restrictions remain disclosed.

Measure only the original primary stages normal-reference200 and overload400;
omit common300/near350/recovery200 from this calibration. The historical
overload label is not evidence of capacity. Report recovery indeterminate;
this experiment cannot replace recovery/host acceptance gates. W2 cold200
uses existing unique-key generation. Each W2 warm point uses an independent
prefilled process, original 30,000ms TTL and 500ms margin, and per-key TTL
verification. A 25-second warm window leaves 4.5 seconds for prefill age;
any actual TTL breach invalidates the point, never raises TTL. W1/W3 keep
their two primary stages in one process. W3 final event-tail oracle now checks
overload, the actual last measured stage. Legacy/M2 retain five stages.

Freeze two complete 27-attempt plans from the original balanced V12 order
before data. First execute their W1 slots only: nine attempts in each batch,
36 primary measurements total. Compute the fixed W1 gate using all six
paired repetitions. If any validity/guard/equivalence condition fails, stop
with no W2/W3 data and preserve unexecuted plan rows. If W1 qualifies, execute
the remaining W2/W3 slots in original relative order, then full gate. This
conditional continuation is prospective resource budgeting, not resampling
or choosing favorable data. No reruns, replacements, exclusions or alternate
orders. Plans, all tools and inputs are frozen before first traffic.

Full qualification requires 126 valid primary rows, each with exactly
25*QPS scheduled=sent=received=correct_on_time and zero error/late/shortfall,
all 54 runner exits zero, exact Rust-before identity, m3 profile and settings.
No individual p95/p99 pair may exceed its original guard in either batch.
Every one of 28 six-pair log-ratio intervals must fall strictly inside
[-log(1.10),+log(1.10)], two-sided90% t(5)=2.01504837333302, sample SD/sqrt6.
W1 subset applies the identical rule to its eight intervals. Units are
whole attempts, not thousands of correlated requests. Approximate normality
and independence assumptions remain limitations; publish all ratios/drift.
Missing/duplicate rows, settings errors and missing identities fail closed.

## Freeze and validation

Fresh remote tools `measurement-v3` and results `results-m3-calibration`;
never overwrite M2. Exact runner/helper/config/workload/analyzer hashes are
checked by the driver. Review the committed parent/head; after PASS capture
zero-attempt preflight, hashes, full plans, disk capacity and source commit
before running the same driver once. Keep full raw and hashed selected evidence.
Existing v9 binary SHA
`254500ec00527850f0f137bcc7e9ac26ff4953d6600bfc00eb7a7fbc2afc436d`.
Baseline SHA
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

macOS full helper tests passed; eight preserved M2 qualification tests and
13 M3/compatibility tests passed; both shell scripts pass syntax checks.
M3 tests exercise the actual read-only execution stage planner, fail malformed
settings, fail one guard crossing, qualify fixed W1/full synthetic controls,
and fail missing rows/short windows. Linux helper/runner tests must pass before
measured calibration. No measured M3 attempt has run at protocol submission.

Only a qualified complete M3 matrix permits preparing a separately reviewed
V12 acceptance supplement with contemporaneous controls and audit-on data.
Calibration PASS alone cannot award A5. A failed M3 remains failed and requires
another explicitly prospective reviewed revision, never retry-until-PASS.
