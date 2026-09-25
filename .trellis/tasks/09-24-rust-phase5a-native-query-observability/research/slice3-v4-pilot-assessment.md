# Slice 3 V4 pilot assessment

Captured on 2026-09-26 from the V4 Linux run and its pinned raw artifacts.
V4 reduced the V3 audit-on tail-latency crossings but still fails the frozen
performance gate and is not eligible for review.

## Candidate and run identity

- Source commit: `aa9a6c4316feec1a9911826208ab9d872785b6fa`; tracked `rust/` tree:
  `ecbe0d49dd6dd41b172020e6db580cb25fa8cad4`.
- Linux release binary SHA-256:
  `0f10153558d7ff85367d0a81ea985ae060539759124fe8a9ec3605c9157524bd`.
- The build used Rust 1.95.0, the committed lockfile, and the frozen runner,
  helper, configurations, overlays, and W1/W2/W3 workload inputs. Exact
  identity and pre-run checks are in `slice3-candidate-v4-identity.md` and
  `slice3-v4-run-audit.txt`.
- All 27 planned attempts ran from `2026-09-25T17:41:00Z` through
  `2026-09-25T17:51:03Z`; 26 runner exits were zero and no replacement or
  rerun attempt was made. Results used the separate ext4-backed
  `results-v4/` directory on `mosdns-rust`; production `mos` was untouched.
- The remote 901-file raw tree passed `sha256sum --quiet -c
  raw-file-hashes.sha256`; its manifest SHA-256 is
  `699b28e6dfe916fb9714d5e623630a8978a647d1257ed403ebfd81d81c084579`, matching
  the copied companion digest. The local manifest copy hashes to the same
  value. The checked manifest and companion value are preserved alongside this
  report.

## Validity and resource observations

The analyzer emitted 63 primary measurement rows, 62 valid, and 56 paired
assessments. W1 TCP repetition 3 audit-off had one scheduled overload slot that
was not sent; the recovery stage was therefore indeterminate. That 400 QPS
primary row is invalid. Wrong responses, protocol errors, transport errors,
and timeouts were zero across the primary rows. The maximum observed
one-minute load in those primary stage samples was 0.60; the test VM had two
online CPUs and pinned the SUT to CPU 0 and harness to CPU 1.

Peak sampled RSS stayed under the frozen guards. The largest audit-off versus
Rust-before delta was +172 KiB (8 MiB guard); the largest audit-on versus
audit-off delta was +636 KiB (32 MiB guard). There were no repeatable CPU guard
crossings. CPU comparisons were inconclusive because the 100-Hz tick samples
differed by fewer than two ticks or fewer than three valid pairs remained.

## Frozen latency assessments

Two paired assessments crossed their latency guards repeatedly:

| Workload | Rate | Comparison | Metric | Repetitions above guard |
|---|---:|---|---|---:|
| W1 TCP | 200 QPS | after-off vs Rust-before | p95 | 2 of 3 |
| W2 cold | 200 QPS | audit-on vs after-off | p99 | 2 of 3 |

The W1 paired deltas were `19.5164%, -6.9149%, 17.5414%` against a 10%
guard. The W2 paired deltas were `13.3489%, -9.8592%, 44.6970%` against a
12.47% guard. All W3 audit-on 400 QPS p95/p99 comparisons were improvements
relative to audit-off, removing V3's repeated W3 audit-on crossings. The two
remaining repeated crossings still fail the gate. Full values are in
`slice3-v4-derived-primary-measurements.tsv` and
`slice3-v4-derived-paired-assessments.tsv`.

The normal successful-request path still wrote completed facts into the
cancellation checkpoint and then read them back before recording. The next
candidate has the listener guard retain those completed facts directly; its
Drop path uses the owned event if send is pending and consults the shared
checkpoint only when execution itself was interrupted. The audit ring also
reserves up to 1,024 records at assembly time when capture is enabled, avoiding
its early growth reallocations while keeping audit-off allocation at zero.
