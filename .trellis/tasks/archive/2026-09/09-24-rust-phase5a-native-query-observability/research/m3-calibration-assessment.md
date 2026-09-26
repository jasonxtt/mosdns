# M3 fixed W1 gate: unqualified

Reviewed head `6283727fc84eed889f9a23a916015af0ed1e8b1d` ran once,
2026-09-26 02:49:47–03:05:15 UTC (see run audit for exact timestamps).
Both fixed W1 batches completed all 18 attempts, exit zero. All 36 primary
rows were valid, all 270,000 requests sent/received/correct on time, and all
error/late/shortfall counters zero. Both batches repeated the identical-binary
TCP200 p99 guard. Twelve comparison/metric rows had individual crossings.
Only 1/8 six-pair equivalence intervals qualified. M3 is unqualified.

The driver stopped at the declared W1 gate, collecting no W2/W3 or candidate
data. Full 27-attempt plans remain preserved with unexecuted rows. V12 remains
failed and A5 unmet; no longer-window result is used to excuse its overhead.

Remote full raw: `mosdns-rust:/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m3-calibration`,
84 MiB, 431 files, verified manifest SHA
`8ac2c308a079ea79b562fbf7084e8e43a05afebba77dc6e647b8b079480015b0`.
102 selected files verified locally against it in `m3-calibration-results/`.
Qualification lists every paired ratio, guard failure and confidence interval.

Longer windows did not solve the measurement limitation. Archived resource
samples show the Go load generator and TCP fixture consuming CPU1 concurrently;
their sampled RSS peaks are roughly 13–18 MiB. They allocate new socket/DNS/
ledger objects per query with default garbage collection. GC is a plausible
source of harness-side disturbance, not an established cause. A prospective
bounded experiment may remove automatic GC only from these test processes,
capture GC traces and resource peaks, and retain the same Rust SUT, traffic,
latency definition, 25-second windows and unchanged qualification thresholds.
It requires separate review before data; no M3 resampling.
