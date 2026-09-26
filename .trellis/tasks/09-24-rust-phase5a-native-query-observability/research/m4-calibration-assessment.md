# M4 fixed W1 controls: unqualified; acceptance remains closed

Approved head `2a56052cd9ef868eb4873964c3c38ad97f84173e` ran once,
2026-09-26 03:15:38–03:31:06 UTC (exact run audit retained).
All18 runner exits zero, all36 primary measurements valid, all270000
scheduled/sent/received/correct-on-time requests reconciled, all late/wrong/
protocol/transport/timeout/shortfall counters zero. All18 GC evidence gates
passed: zero traces, complete sampled Go-role coverage, maximum50548KiB
(49.36MiB), below256MiB. Thus the intended bounded helper profile executed.

Nevertheless, batch1 repeated TCP400 off-slot versus before-slot p99 guard;
batch2 had no repeated latency guard but had two individual crossings.
Eight comparison/metric rows across both batches had individual crossings.
Only1/8 six-pair equivalence intervals qualified. The other seven intervals
remain too wide/outside the unchanged10% margin. The prospective W1 gate
failed and stopped before W2/W3, with full unexecuted plans retained.

Full raw remains at
`mosdns-rust:/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m4-calibration`,
84MiB/450 files. Complete manifest verified remotely, SHA
`85c25fbe25b859f4677d7d74a3df7a6ef09c75a9dcb5c67990e6fa4b271cea3e`.
120 selected files verified locally against it in `m4-calibration-results/`.
Qualification includes all ratios, individual failures and interval bounds;
GC evidence includes source-log hashes and sampled peaks per role.

Automatic helper GC is not a sufficient explanation of the remaining
variability. These observations do not prove the VM is its sole cause, prove
V12 free of overhead, or qualify Rust production performance. No retry,
replacement, filtered sample, widened margin or candidate acceptance follows.
M2/M3/M4 remain unqualified, V12 remains failed, A5 remains unmet.

Further useful execution needs a materially isolated test environment or a
host-interference investigation, followed by a fresh prospective protocol
review and calibration. `mos-test` was unreachable in bounded read-only
connection checks; no alternative exclusive Linux host is currently supplied.
The user has been asked for a reachable exclusive host or restored mos-test.
Existing production/test services were not changed. Task remains in progress,
not finished or archived. Do not run another copy of M4 until it happens to pass.
