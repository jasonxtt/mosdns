# Slice 3 V9 pilot assessment

Captured on 2026-09-26 from the V9 Linux run and pinned raw artifacts. V9
completed the frozen matrix and all primary rows were valid, but 12 paired
measurements crossed frozen regression guards repeatedly. V9 is not eligible
for review.

## Candidate and run identity

- Source commit: `f22c558365f1fc929752ecf214b29d510118d619`; tracked `rust/`
  tree: `15ea7e46304950ba1df8c7b5f60cd04cbaa628a2`.
- Release binary SHA-256:
  `b0917f6fd57eb2f57996aa7125a60a42871edfd67c789bafd31e253c9ffd37b7`.
  Rust 1.95.0 and the committed lockfile were used; helper v8 validation
  passed. Full identities are in `slice3-candidate-v9-identity.md`.
- All 27 scheduled attempts ran from `2026-09-25T20:22:31Z` through
  `2026-09-25T20:32:34Z`; every runner exit was zero. No benchmark attempt was
  replaced or rerun. Results are in the separate ext4-backed
  `results-v9-run/` directory on `mosdns-rust`; production `mos` was untouched.
- The raw tree has 901 files. Its complete SHA-256 manifest passed on Linux;
  manifest digest `60b7058a9a51d8da38f1cd13720fdae8ac5619fe136c17fb085adced6ea0fa71`.
  The companion digest file matches the copied local manifest.

## Validity and observed measurements

The frozen analyzer emitted 63 primary rows and 56 paired assessments; all
63 primary rows were valid. Across those rows, the 54,000 scheduled requests
were sent, received, and correct on time. There were no late or wrong
responses, protocol or transport errors, timeouts, or sender shortfalls. The
maximum sampled host load was 0.60 on the two-CPU VM.

Median p50/p95/p99 latency values (microseconds) across the three repetitions
are shown below. Per-run correct-on-time throughput matched the offered rate in
each group: 600/600 at 200 QPS and 1,200/1,200 at 400 QPS.

| Workload / rate | Rust-before | Audit off | Audit on |
|---|---:|---:|---:|
| W1 TCP / 200 QPS | 281 / 554 / 820 | 277 / 548 / 786 | 295 / 615 / 1,045 |
| W1 TCP / 400 QPS | 203 / 431 / 681 | 222 / 491 / 852 | 242 / 552 / 950 |
| W2 cold / 200 QPS | 155 / 296 / 449 | 150 / 299 / 419 | 170 / 352 / 594 |
| W2 warm / 200 QPS | 179 / 336 / 514 | 193 / 361 / 579 | 190 / 358 / 490 |
| W2 warm / 400 QPS | 94 / 233 / 395 | 111 / 272 / 415 | 98 / 253 / 379 |
| W3 / 200 QPS | 384 / 752 / 1,312 | 501 / 991 / 1,737 | 421 / 788 / 1,410 |
| W3 / 400 QPS | 263 / 593 / 929 | 350 / 825 / 1,365 | 336 / 807 / 1,705 |

Peak paired RSS deltas remained below the frozen limits: +208 KiB for audit-off
versus Rust-before and +2,324 KiB for audit-on versus audit-off. Thirteen of
14 CPU comparison groups were inconclusive at the VM's 100-Hz process-tick
resolution. The remaining W3 400 QPS audit-off comparison crossed its CPU
guard in 2/3 pairs (`18.1818%, 40.0000%, 30.0000%`; 25.00% guard).

## Frozen guard crossings

| Workload | Rate | Comparison | Metric | Paired deltas | Guard | Above guard |
|---|---:|---|---|---|---:|---:|
| W1 TCP | 200 QPS | audit-on vs audit-off | p95 | `27.7259%, 3.4672%, 22.5100%` | 16.79% | 2/3 |
| W1 TCP | 200 QPS | audit-on vs audit-off | p99 | `121.5933%, -1.0178%, 63.7931%` | 37.66% | 2/3 |
| W1 TCP | 400 QPS | audit-off vs Rust-before | p95 | `13.9211%, 29.9304%, -2.5463%` | 10.00% | 2/3 |
| W1 TCP | 400 QPS | audit-off vs Rust-before | p99 | `26.5973%, 89.5742%, -9.6774%` | 10.00% | 2/3 |
| W2 cold | 200 QPS | audit-on vs audit-off | p95 | `4.6823%, 22.2222%, 18.4211%` | 10.00% | 2/3 |
| W2 cold | 200 QPS | audit-on vs audit-off | p99 | `18.2482%, 24.7899%, 52.5060%` | 10.00% | 3/3 |
| W2 warm | 400 QPS | audit-off vs Rust-before | p99 | `-4.3038%, 33.7500%, 21.7009%` | 10.00% | 2/3 |
| W3 | 200 QPS | audit-off vs Rust-before | p95 | `48.9362%, 22.0443%, 0.6766%` | 10.00% | 2/3 |
| W3 | 200 QPS | audit-off vs Rust-before | p99 | `60.3524%, 35.5972%, -23.2470%` | 10.00% | 2/3 |
| W3 | 400 QPS | audit-off vs Rust-before | p95 | `39.1231%, 45.2991%, 25.9319%` | 10.00% | 3/3 |
| W3 | 400 QPS | audit-off vs Rust-before | p99 | `30.4688%, 98.8359%, 46.9322%` | 15.07% | 3/3 |
| W3 | 400 QPS | audit-off vs Rust-before | CPU/query | `18.1818%, 40.0000%, 30.0000%` | 25.00% | 2/3 |

The full primary and paired values are in
`slice3-v9-derived-primary-measurements.tsv` and
`slice3-v9-derived-paired-assessments.tsv`. The V9 change removed shared
checkpoint locking and stored the completed observation in the existing
checkpoint allocation, but this matrix did not clear the frozen review gate.
