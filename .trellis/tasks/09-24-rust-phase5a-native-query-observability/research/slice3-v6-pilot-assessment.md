# Slice 3 V6 pilot assessment

Captured on 2026-09-26 from the V6 Linux run and its pinned raw artifacts.
V6 cleared the W2 cold 200 QPS crossings from V5 but retains four repeatable
high-rate audit-off latency crossings and is not eligible for review.

## Candidate and run identity

- Source commit: `cd96b0adf76767cfede5f20a929ad7ad36e0ce44`; tracked `rust/`
  tree: `246910bbbe1a59612e9dbde0d2256231f2c3bc76`.
- Linux release binary SHA-256:
  `fe553e2666a9b4dfeb803a6489cadd9f9ad53e85f60f2a6bb5f68e357beec91e`.
  Rust 1.95.0 and the committed lockfile were used; helper v8's
  `validate-binary` passed. See `slice3-candidate-v6-identity.md` for the
  complete pinned build and fixture identities.
- All 27 scheduled attempts ran from `2026-09-25T18:40:39Z` through
  `2026-09-25T18:50:42Z`; all runner exits were zero. There were no replacement
  or rerun attempts. Results used the separate ext4-backed `results-v6/`
  directory on `mosdns-rust`; production `mos` was untouched.
- The 899-entry raw-file hash manifest was verified on the remote host with
  `sha256sum --quiet -c`; the full raw tree totals 901 files including the
  manifest and its companion. The manifest SHA-256 is
  `1ab82e212c6e2651993a0c2219abf29ddc58b95581e8119acfbfb7f046b2844f`; the
  copied local manifest matches the companion digest.

## Validity and resource observations

The analyzer emitted 63 primary rows and 56 paired assessments. All 63
primary rows were valid, and every planned attempt exited zero. Wrong
responses, protocol errors, transport errors, timeouts, and sender shortfalls
were zero. The maximum sampled host load was 0.59 on the two-CPU VM.

Peak sampled RSS remained under the frozen budgets: the largest paired increase
was +168 KiB for audit-off versus Rust-before and +2,228 KiB for audit-on
versus audit-off. Thirty CPU paired comparisons were limited by the VM's
100-Hz tick resolution, so the CPU guard remains inconclusive.

## Frozen latency assessments

Four paired assessments crossed their latency guards repeatedly:

| Workload | Rate | Metric | Paired deltas | Guard | Repetitions above guard |
|---|---:|---|---|---:|---:|
| W1 TCP | 400 QPS | p95, after-off vs Rust-before | `57.3566%, -2.4540%, 31.7526%` | 10.00% | 2 of 3 |
| W1 TCP | 400 QPS | p99, after-off vs Rust-before | `109.8485%, 23.6041%, 86.5155%` | 12.69% | 3 of 3 |
| W2 warm | 400 QPS | p95, after-off vs Rust-before | `16.6065%, 21.3559%, 52.2901%` | 10.83% | 3 of 3 |
| W2 warm | 400 QPS | p99, after-off vs Rust-before | `152.3677%, 66.7269%, 98.3640%` | 26.18% | 3 of 3 |

The other frozen p95/p99 pairs did not cross their guards repeatably,
including W2 cold 200 QPS audit-off versus Rust-before. Audit-on versus
audit-off comparisons had no repeatable p95/p99 crossing. Full per-stage and
paired values are in `slice3-v6-derived-primary-measurements.tsv` and
`slice3-v6-derived-paired-assessments.tsv`.

V6 restores the completed-event write and read through the shared execution
checkpoint. Their mutex cost is a plausible contributor to the W1 and W2
warm high-rate audit-off crossings, but the matrix does not establish that
causally. The next candidate will keep the completed observation in a boxed
listener-owned slot: this retains V5's lock-free successful finalization path
while reducing the guard's inline state. This is a new hypothesis and requires
another full pinned matrix.
