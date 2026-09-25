# Slice 3 V8 pilot assessment

Captured on 2026-09-26 from the V8 Linux run and its pinned raw artifacts.
V8 completed the matrix but retained eight repeatable p95/p99 guard crossings
and one invalid primary row, so it is not eligible for review.

## Candidate and run identity

- Source commit: `b7c4935e176809bdd3fc46fbea0cca13887a89f4`; tracked `rust/`
  tree: `d59a19d4f6e8bd5ae22c620cba66e93c9331063c`.
- Linux release binary SHA-256:
  `e9e92965d724438ea973f31e560432fc154c419ecc0be2133659607078176fda`.
  Rust 1.95.0 and the committed lockfile were used; helper v8's
  `validate-binary` passed. See `slice3-candidate-v8-identity.md` for full
  source, runner, helper, binary, and fixture identities.
- All 27 scheduled attempts ran from `2026-09-25T19:38:04Z` through
  `2026-09-25T19:48:08Z`; 25 runner exits were zero and two were one. No
  replacement or rerun attempt was made. Results used the separate ext4-backed
  `results-v8/` directory on `mosdns-rust`; production `mos` was untouched.
- The raw tree totals 901 files. The 899-entry raw-file hash manifest passed
  remote `sha256sum --quiet -c`; its SHA-256 is
  `fa14246231c100634173dbd94be208d7b4960b45087322e97b4bc188e56f3821`. The
  copied local manifest matches its companion digest.

## Validity and resource observations

The analyzer emitted 63 primary rows and 56 paired assessments; 62 primary
rows were valid. W2 warm repetition 3 Rust-before missed one open-loop slot in
the 400 QPS overload stage (1,199/1,200 sent); that primary row is invalid and
there was no replacement. The other runner exit 1 was W3 repetition 1
audit-on, whose 350 QPS supporting near-saturation stage missed one slot; its
200 and 400 QPS primary rows remained valid. Wrong responses, protocol errors,
transport errors, and timeouts were zero. Maximum sampled host load was 0.65
on the two-CPU VM.

Peak sampled RSS remained within budget: the largest paired increase was
+172 KiB for audit-off versus Rust-before and +2,324 KiB for audit-on versus
audit-off. Twenty-nine CPU paired comparisons were limited by the 100-Hz
process-tick resolution, so CPU remains inconclusive. The W2 warm 400 QPS
audit-off versus Rust-before comparison has only two valid pairs and is
inconclusive under the frozen rule.

## Frozen latency assessments

Eight paired assessments crossed their latency guards repeatedly:

| Workload | Rate | Comparison | Metric | Paired deltas | Guard | Above guard |
|---|---:|---|---|---|---:|---:|
| W1 TCP | 200 QPS | audit-on vs audit-off | p95 | `15.8273%, 19.9637%, -4.6763%` | 10.00% | 2 of 3 |
| W1 TCP | 200 QPS | audit-on vs audit-off | p99 | `16.7579%, 12.8009%, -14.3333%` | 10.00% | 2 of 3 |
| W1 TCP | 400 QPS | audit-off vs Rust-before | p95 | `35.9606%, -24.9564%, 50.2222%` | 19.56% | 2 of 3 |
| W1 TCP | 400 QPS | audit-off vs Rust-before | p99 | `44.6659%, -39.7869%, 62.5869%` | 31.42% | 2 of 3 |
| W2 warm | 200 QPS | audit-on vs audit-off | p99 | `-8.8583%, 27.8215%, 40.4762%` | 18.57% | 2 of 3 |
| W3 | 200 QPS | audit-off vs Rust-before | p99 | `31.7116%, 35.4102%, -0.0883%` | 10.00% | 2 of 3 |
| W3 | 400 QPS | audit-off vs Rust-before | p95 | `72.4490%, 31.0576%, -4.6584%` | 17.39% | 2 of 3 |
| W3 | 400 QPS | audit-off vs Rust-before | p99 | `115.0980%, 57.6667%, 42.3077%` | 21.57% | 3 of 3 |

The W2 warm 400 QPS audit-off baseline comparison is inconclusive because of
its invalid pair and is not included in the eight crossings. Full per-stage
and paired values are in `slice3-v8-derived-primary-measurements.tsv` and
`slice3-v8-derived-paired-assessments.tsv`.

V8 removes the admission metrics mutex, but each request still allocates and
clones an `Arc<Mutex<ExecutionCheckpoint>>` used only if its execution future
is interrupted. That per-query checkpoint handle and allocation are a
remaining candidate cost, not a demonstrated cause of the measured
crossings. V9 will make the checkpoint listener-owned boxed state and lend it
mutably to execution; cancellation Drop can publish partial facts directly,
while successful requests avoid the Arc, mutex, and clone. This preserves the
bounded event and snapshot contract and requires a new full matrix.
