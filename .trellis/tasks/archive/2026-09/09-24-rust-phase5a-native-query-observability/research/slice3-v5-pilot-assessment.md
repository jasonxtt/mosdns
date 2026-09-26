# Slice 3 V5 pilot assessment

Captured on 2026-09-26 from the V5 Linux run and its pinned raw artifacts.
V5 reduced the W1 after-off latency crossings and the W2 audit-on overhead, but
it retains three repeatable latency guard crossings and is not eligible for
review.

## Candidate and run identity

- Source commit: `f73d0967ce3325e35bef46c8b0ef7d9801eb797e`; tracked `rust/` tree:
  `6f7094d5b9d567ae6697fb8ee86ac81e5ddc6473`.
- Linux release binary SHA-256:
  `0d11f89837b7de972cd4d8d080932de50ec7d77708f0a1d46bd50c98a149e42f`.
- The build used Rust 1.95.0, the committed lockfile, and the frozen runner,
  helper, configurations, overlays, and W1/W2/W3 workload inputs. Exact
  identity and pre-run checks are in `slice3-candidate-v5-identity.md` and
  `slice3-v5-run-audit.txt`.
- All 27 planned attempts ran from `2026-09-25T18:13:02Z` through
  `2026-09-25T18:23:05Z`; 26 runner exits were zero and no replacement or
  rerun attempt was made. Results used the separate ext4-backed
  `results-v5/` directory on `mosdns-rust`; production `mos` was untouched.
- The remote 901-file raw tree passed `sha256sum --quiet -c
  raw-file-hashes.sha256`; its manifest SHA-256 is
  `3ae43b69531bfdf01dd8280e6c0dc50ae9beb5bc016bc9a0aa7cdcd08d28e51d`, matching
  the copied companion digest. The local manifest copy hashes to the same
  value. The checked manifest and companion value are preserved alongside this
  report.

## Validity and resource observations

The analyzer emitted 63 primary measurement rows, 62 valid, and 56 paired
assessments. W2 cold repetition 1 after-off had one scheduled 400 QPS overload
slot that was not sent; recovery was therefore indeterminate. The primary
400 QPS overload row is invalid. Wrong responses, protocol errors, transport
errors, and timeouts were zero across primary rows. The maximum observed
one-minute load in primary stage samples was 0.74; the test VM had two online
CPUs and pinned the SUT to CPU 0 and harness to CPU 1.

Peak sampled RSS stayed under the frozen guards. The largest audit-off versus
Rust-before delta was +144 KiB (8 MiB guard); the largest audit-on versus
audit-off delta was +488 KiB (32 MiB guard). There were no repeatable CPU guard
crossings. Ten CPU comparisons were inconclusive because the 100-Hz tick
samples differed by fewer than two ticks; two had fewer than three valid pairs.

## Frozen latency assessments

Three paired assessments crossed their latency guards repeatedly:

| Workload | Rate | Comparison | Metric | Repetitions above guard |
|---|---:|---|---|---:|
| W2 cold | 200 QPS | after-off vs Rust-before | p95 | 3 of 3 |
| W2 cold | 200 QPS | after-off vs Rust-before | p99 | 3 of 3 |
| W3 | 200 QPS | audit-on vs after-off | p99 | 2 of 3 |

The W2 cold p95 deltas were `21.7391%, 15.4882%, 11.2211%` against a 10%
guard. W2 cold p99 deltas were `129.1954%, 75.1256%, 18.3932%` against a
17.01% guard. W3 200 QPS audit-on versus after-off p99 deltas were
`28.0568%, -37.0857%, 12.1466%` against a 10% guard. Full per-stage and paired
values are in `slice3-v5-derived-primary-measurements.tsv` and
`slice3-v5-derived-paired-assessments.tsv`.

The V5 listener guard stores the completed `TerminalObservation` inline and
lives across the send await. This avoids checkpoint locks but increases the
listener future's inline state. Source inspection makes future size a plausible
cause of the W2 cold audit-off regression; this is a hypothesis, not a measured
causal result. The follow-up keeps completed facts in the heap-backed execution
checkpoint while preserving V4's audit-record construction outside the shared
observer lock and V5's bounded audit-ring reservation. A new pinned matrix is
required to test that combination.
