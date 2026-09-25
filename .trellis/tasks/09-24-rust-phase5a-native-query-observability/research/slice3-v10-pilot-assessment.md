# Slice 3 V10 pilot assessment

Captured on 2026-09-26 from the completed V10 Linux run and pinned result
artifacts. All primary measurements are valid and request correctness passed,
but nine paired p95/p99 comparisons crossed frozen regression guards. V10 is
not eligible for review.

## Candidate and run identity

- Source commit: `65a31ff57046e0047006ea401aa023519c658c16`; tracked `rust/`
  tree: `6275e691d181aa5c9dc7b3f499c3a4fba32bc9c0`.
- Release binary SHA-256:
  `bb13371306d6a26228bcca7e496e8f1354e1d9ac0ecfa2ef36bb5c7b01dba2bd`.
  Rust 1.95.0 and the committed lockfile were used; helper v8 validation
  passed. See `slice3-candidate-v10-identity.md` for the complete identity.
- The frozen 27-attempt matrix ran from `2026-09-25T21:07:00Z` through
  `2026-09-25T21:17:04Z`; 26 runners exited zero and one exited 1. No attempt
  was replaced or rerun. Results remain in the ext4-backed
  `results-v10-run/` directory on `mosdns-rust`; production `mos` was
  untouched.
- The one runner failure was W1 TCP repetition 3, Rust-before. Its 350 QPS
  `near-saturation` supporting stage dropped one scheduled slot, and its
  `recovery` supporting stage failed the same-process recovery criterion. The
  200 and 400 QPS primary rows for that attempt remain valid; both invalid
  supporting-stage records and logs are preserved in
  `slice3-v10-invalid-w1-tcp-r3-before-off/`.
- All 63/63 primary rows were valid. Their 54,000 scheduled requests were
  sent, received, and correct on time; late/wrong responses, protocol or
  transport errors, timeouts, and sender shortfalls were all zero in those
  rows. Maximum sampled host load across the primary rows was 0.63 on the
  two-CPU VM.
- The complete raw-tree SHA-256 manifest contains 902 entries and passed
  verification on Linux. Its digest is
  `c862ae80466ee8255166a325486e20dec5c592614f6308abe1486894cd15b309`; the
  companion file records that same digest. The complete 61 MiB raw result
  tree remains at the remote path above. Its run audit, attempt order, derived
  TSVs, binary identities, manifest, and companion digest are copied beside
  this report.

## Median latencies

Values are medians across the three repetitions, in microseconds. Each valid
primary run delivered all 600 scheduled requests at 200 QPS and all 1,200 at
400 QPS on time.

| Workload / rate | Rust-before p50/p95/p99 | Audit off p50/p95/p99 | Audit on p50/p95/p99 |
|---|---:|---:|---:|
| W1 TCP / 200 QPS | 282 / 579 / 974 | 303 / 570 / 720 | 273 / 538 / 648 |
| W1 TCP / 400 QPS | 218 / 449 / 670 | 225 / 518 / 848 | 226 / 458 / 746 |
| W2 cold / 200 QPS | 157 / 312 / 420 | 150 / 308 / 408 | 161 / 321 / 530 |
| W2 warm / 200 QPS | 159 / 305 / 425 | 178 / 339 / 516 | 173 / 319 / 702 |
| W2 warm / 400 QPS | 135 / 301 / 518 | 144 / 359 / 864 | 143 / 335 / 838 |
| W3 / 200 QPS | 402 / 801 / 1,279 | 466 / 811 / 1,525 | 451 / 869 / 1,193 |
| W3 / 400 QPS | 306 / 698 / 1,320 | 423 / 1,053 / 2,922 | 266 / 605 / 1,031 |

Peak paired sampled RSS increases were +68 KiB for audit-off versus
Rust-before and +2,160 KiB for audit-on versus audit-off, below the frozen
limits. Thirteen of 14 CPU comparisons were inconclusive at 100-Hz process-tick
resolution; the remaining comparison did not cross its frozen guard.

## Frozen guard crossings

| Workload | Rate | Comparison | Metric | Paired deltas | Guard | Above guard |
|---|---:|---|---|---|---:|---:|
| W1 TCP | 400 QPS | audit-off vs Rust-before | p95 | `31.8945%, -24.3144%, 15.3675%` | 14.25% | 2/3 |
| W1 TCP | 400 QPS | audit-off vs Rust-before | p99 | `76.5672%, -40.2351%, 28.4848%` | 10.00% | 2/3 |
| W2 cold | 200 QPS | audit-on vs audit-off | p99 | `30.6373%, 23.8318%, 15.5388%` | 10.00% | 3/3 |
| W2 warm | 200 QPS | audit-off vs Rust-before | p95 | `9.4937%, 11.1475%, 12.5436%` | 10.00% | 2/3 |
| W2 warm | 200 QPS | audit-on vs audit-off | p99 | `46.1240%, -29.8482%, 64.4028%` | 29.84% | 2/3 |
| W2 warm | 400 QPS | audit-off vs Rust-before | p95 | `3.6545%, 18.8119%, 20.0669%` | 10.00% | 2/3 |
| W2 warm | 400 QPS | audit-off vs Rust-before | p99 | `11.6959%, 66.7954%, 46.9773%` | 10.00% | 3/3 |
| W3 | 400 QPS | audit-off vs Rust-before | p95 | `68.3381%, 57.8711%, -19.3421%` | 10.00% | 2/3 |
| W3 | 400 QPS | audit-off vs Rust-before | p99 | `121.6995%, 170.3030%, -34.9528%` | 10.00% | 2/3 |

The detailed measurements are in `slice3-v10-derived-primary-measurements.tsv`
and `slice3-v10-derived-paired-assessments.tsv`; the raw analyzer output is
`slice3-v10-analysis-summary.txt`. V10 gated audit-only route and provenance
materialization and reserved the bounded W3 attempt vector, but the frozen
matrix still blocks review. Keep the thresholds unchanged and use another
source-backed candidate before requesting review.
