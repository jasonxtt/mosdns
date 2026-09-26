# Slice 3 V7 pilot assessment

Captured on 2026-09-26 from the V7 Linux run and its pinned raw artifacts.
V7 removed V6's repeated W1 TCP 400 QPS baseline crossings, but introduced or
retained ten other repeatable p95/p99 crossings and is not eligible for review.

## Candidate and run identity

- Source commit: `ad0097ea73374425652586b6fe6d06748309c362`; tracked `rust/`
  tree: `9ad33d3c4317f2d3bf221d7ce4f15e296d8ca38c`.
- Linux release binary SHA-256:
  `6c799a905cd4bc33da241238473bc449abc7ad077e430723f79d514a013bd56b`.
  Rust 1.95.0 and the committed lockfile were used; helper v8's
  `validate-binary` passed. Full identities and inputs are in
  `slice3-candidate-v7-identity.md`.
- All 27 scheduled attempts ran from `2026-09-25T19:06:34Z` through
  `2026-09-25T19:16:38Z`; all runner exits were zero. No attempt was replaced
  or rerun. Results used the distinct ext4-backed `results-v7/` directory on
  `mosdns-rust`; production `mos` was untouched.
- The 899-entry raw-file hash manifest passed remote
  `sha256sum --quiet -c`; the raw tree totals 901 files including the manifest
  and companion. Manifest SHA-256:
  `30d461d2c102075fbe0c18e9f64c6a0a2d45412c15b0ef0f48806ef30b6c0a43`.
  The downloaded manifest matches the companion digest.

## Validity and resource observations

The analyzer emitted 63 primary rows and 56 paired assessments. All primary
rows were valid; all 27 attempts exited zero. Wrong responses, protocol
errors, transport errors, timeouts, and sender shortfalls were zero. Maximum
sampled host load was 0.77 on the two-CPU VM.

Peak sampled RSS remained within the frozen guards: the largest paired
increase was +196 KiB for audit-off versus Rust-before and +2,276 KiB for
audit-on versus audit-off. Thirty-two CPU paired comparisons were limited by
the 100-Hz process-tick resolution, leaving CPU inconclusive.

## Frozen latency assessments

Ten paired assessments crossed their latency guards repeatedly:

| Workload | Rate | Comparison | Metric | Paired deltas | Guard | Above guard |
|---|---:|---|---|---|---:|---:|
| W1 TCP | 200 QPS | audit-off vs Rust-before | p95 | `21.9378%, -12.9663%, 10.7206%` | 10.00% | 2 of 3 |
| W1 TCP | 200 QPS | audit-off vs Rust-before | p99 | `71.8016%, -6.5351%, 18.3384%` | 11.10% | 2 of 3 |
| W2 cold | 200 QPS | audit-on vs audit-off | p95 | `27.1293%, 51.9031%, -35.4839%` | 17.67% | 2 of 3 |
| W2 cold | 200 QPS | audit-on vs audit-off | p99 | `78.1903%, 72.4390%, -65.8291%` | 10.00% | 2 of 3 |
| W2 warm | 200 QPS | audit-on vs audit-off | p95 | `201.2539%, 39.7059%, 20.7944%` | 12.35% | 3 of 3 |
| W2 warm | 200 QPS | audit-on vs audit-off | p99 | `387.5465%, 64.6684%, -37.7763%` | 62.76% | 2 of 3 |
| W2 warm | 400 QPS | audit-off vs Rust-before | p95 | `16.6102%, 23.1884%, -8.8957%` | 12.88% | 2 of 3 |
| W2 warm | 400 QPS | audit-off vs Rust-before | p99 | `45.0000%, 39.3365%, -9.8446%` | 37.69% | 2 of 3 |
| W2 warm | 400 QPS | audit-on vs audit-off | p95 | `28.1977%, -18.5294%, 17.1717%` | 10.00% | 2 of 3 |
| W2 warm | 400 QPS | audit-on vs audit-off | p99 | `54.5093%, -22.9592%, 21.8391%` | 16.67% | 2 of 3 |

The W1 TCP 400 QPS audit-off baseline crossings from V6 did not repeat, but
the rest of the comparisons above block review. Full per-stage and paired
values are in `slice3-v7-derived-primary-measurements.tsv` and
`slice3-v7-derived-paired-assessments.tsv`.

V7 stores completed observations in a boxed listener-owned slot, avoiding
completed-event checkpoint locks and reducing inline future state. The added
allocation and other shared observer work remain possible contributors to the
variable crossings; this matrix does not establish causality. Source review
also found that admission and terminal metrics each take the observer mutex
with audit disabled. V8 will remove the admission mutex acquisition by
atomically tracking in-flight queries and deriving admitted totals in the
metrics snapshot while terminal metrics remain serialized as before.
