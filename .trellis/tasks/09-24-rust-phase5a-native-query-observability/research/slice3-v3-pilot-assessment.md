# Slice 3 V3 pilot assessment

Captured on 2026-09-26 from the V3 Linux run and its pinned raw artifacts.
This is a performance-gate assessment; V3 does not pass the frozen latency
guards and is not eligible for review.

## Candidate and run identity

- Source commit: `0f695ca2fd9e4a63afa485bdaefe62af1f534f9e`; tracked `rust/` tree:
  `c3c5019bfc30916a6e92aa605502bc5d59121867`.
- Linux release binary SHA-256:
  `ea111683b9cca331eafa0d35455f27a750b2bbd130546e42fe9cf656440d205c`.
- The build used Rust 1.95.0, the committed lockfile, and the frozen runner,
  helper, configurations, overlays, and W1/W2/W3 workload inputs. The exact
  identity and pre-run checks are in `slice3-candidate-v3-identity.md` and
  `slice3-v3-run-audit.txt`.
- All 27 planned attempts ran from `2026-09-25T17:07:24Z` through
  `2026-09-25T17:17:27Z`; 25 runner exits were zero and no replacement or
  rerun attempt was made. Results used the separate ext4-backed
  `results-v3/` directory on `mosdns-rust`; production `mos` was untouched.
- The remote 901-file raw tree passed `sha256sum --quiet -c
  raw-file-hashes.sha256`; its manifest SHA-256 is
  `b2236d6cf3a16c5abf27b84ab23d57b3b2a57e8a9e80aa3248a588bdcdb8cead`, matching
  the copied companion digest. The local manifest copy hashes to the same
  value. The checked manifest and companion value are preserved alongside this
  report.

## Validity and resource observations

The analyzer emitted 63 primary measurement rows, 62 valid, and 56 paired
assessments. Two W1 attempts contain invalid stages:

- W1 TCP repetition 1 Rust-before: one scheduled near-saturation slot was not
  sent; the recovery stage was therefore indeterminate.
- W1 TCP repetition 3 audit-on: one scheduled overload slot was not sent; the
  recovery stage was therefore indeterminate.

The primary table marks the W1 TCP repetition 3 audit-on overload row invalid.
Across the primary rows, wrong responses, protocol errors, transport errors,
and timeouts were all zero. The maximum observed one-minute load in those
primary stage samples was 0.68; the test VM had two online CPUs and pinned the
SUT to CPU 0 and harness to CPU 1.

Peak sampled RSS stayed under the frozen guards. The largest audit-off versus
Rust-before delta was +188 KiB (8 MiB guard); the largest audit-on versus
audit-off delta was +496 KiB (32 MiB guard). There were no repeatable CPU guard
crossings. CPU comparisons were inconclusive because the 100-Hz tick samples
differed by fewer than two ticks in 13 assessments; the remaining CPU pair was
inconclusive because fewer than three valid pairs remained.

## Frozen latency assessments

Six paired assessments crossed the predeclared latency guards repeatedly:

| Workload | Rate | Comparison | Metric | Repetitions above guard |
|---|---:|---|---|---:|
| W1 TCP | 400 QPS | after-off vs Rust-before | p99 | 2 of 3 |
| W2 warm | 200 QPS | after-off vs Rust-before | p99 | 2 of 3 |
| W2 warm | 400 QPS | audit-on vs after-off | p95 | 2 of 3 |
| W2 warm | 400 QPS | audit-on vs after-off | p99 | 3 of 3 |
| W3 | 400 QPS | audit-on vs after-off | p95 | 3 of 3 |
| W3 | 400 QPS | audit-on vs after-off | p99 | 3 of 3 |

The other latency rows did not show repeatable crossings, but these six are
sufficient to fail the frozen performance gate. Full per-stage values and
paired calculations are in `slice3-v3-derived-primary-measurements.tsv` and
`slice3-v3-derived-paired-assessments.tsv`; the exact order and all attempt
statuses are in `slice3-v3-attempt-order.tsv`.

Source inspection found that V3 materialized the audit record while holding
the shared observer-state mutex. The follow-up candidate moves record
construction outside that mutex while preserving atomic metric updates and
bounded audit insertion. This is a targeted hypothesis for the V3 audit-on
tail-latency crossings; the follow-up must pass a newly pinned full matrix
before Slice 3 can be submitted for review.
