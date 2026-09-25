# Slice 3 pilot v1 assessment

Measured 2026-09-25 on `mosdns-rust` with Rust-before `605c305…` and Rust-after
v1 commit `545ba29fe6c02d47db1ee1be40b781c04646cac0`. Candidate executable
SHA-256: `a20bac363de9fcf7ce21c2ef09f59a695d74ac8b7ace6f6126c87e6faec3370f`.
The build identity and frozen inputs are recorded in
[`slice3-candidate-identity.md`](slice3-candidate-identity.md).

## Result

The matrix completed all 27 planned attempts in balanced order. Runner exit
codes were 26 zero and one nonzero. Per-attempt fixture, helper, runner, and
binary input hashes were independently rechecked: 0 mismatches. All frozen
ports were free before each launched attempt, and the staged configurations
were restored to their frozen hashes after every run.

One attempt was invalid: W3 repetition 3 Rust-before audit-off dropped one
open-loop sender slot at each of 350 and 400 QPS (`1050/1051` and `1199/1200`).
The 400 QPS primary point therefore has only two valid paired repetitions and
is **inconclusive**. No replacement attempt was run. W1 and W2 primary stages
and W3 at 200 QPS had complete expected responses and passed their fixture
oracles. Across the 63 selected primary-stage rows, 62 were valid.

Four latency guardrails crossed the predeclared repeatability threshold:

| Comparison | Metric and point | Paired deltas | Guard | Result |
|---|---|---:|---:|---|
| W1 audit-on vs audit-off | p95, 200 QPS | −7.86%, +82.17%, +20.85% | 10.00% | Repeatable regression |
| W1 after audit-off vs before audit-off | p99, 400 QPS | −19.02%, +25.58%, +67.93% | 18.42% | Repeatable regression |
| W2 cold after audit-off vs before audit-off | p95, 200 QPS | +24.92%, +160.13%, −9.45% | 10.00% | Repeatable regression |
| W2 cold audit-on vs audit-off | p99, 200 QPS | +61.68%, −52.57%, +63.78% | 39.58% | Repeatable regression |

These are computed against the frozen median-of-paired-deltas and control
relative-MAD rules. V1 therefore does not pass the frozen performance gate and
is not submitted for Slice 3 review. Other p95/p99 comparisons did not cross
the repeatable-regression guard. The per-repetition p50/p95/p99, correct-on-time
counts, throughput, sampled CPU ticks, and sampled RSS values are in
[`slice3-v1-primary-measurements.tsv`](slice3-v1-primary-measurements.tsv);
all paired calculations and guards are in
[`slice3-v1-paired-assessments.tsv`](slice3-v1-paired-assessments.tsv).

Sampled RSS stayed below both frozen limits. The largest audit-off paired
increase was 204 KiB (8 MiB limit); the largest audit-on versus audit-off peak
increase was 1,916 KiB (32 MiB limit). CPU used 100 Hz process ticks, with
2–20 ticks measured per selected stage; comparisons whose paired difference
was below two ticks are marked inconclusive by the analysis.

Host load increased through W3 (one-minute load average rose from 0.29–0.59 at
the start of earlier W3 attempts to 1.09 after the final attempt on a
two-CPU host). This adds measurement uncertainty, but it does not erase the
four guard crossings or turn them into a pass. No capacity, overload, recovery,
or production performance claim is made.

## In-scope response

The fixed histogram implementation updated up to all 16 cumulative buckets
under the per-query observer mutex, even when detailed audit capture was off.
V1 exposed repeatable latency regressions, so the implementation was changed
to increment one internal bucket and produce the same cumulative public
snapshot only when a metrics snapshot is requested. The frozen bucket edges,
inclusive-boundary semantics, total count, and sum are unchanged. The full
workspace test suite and strict clippy pass after this change. This is a new
candidate build and a full second frozen matrix; no v1 attempt is replaced or
discarded.

## Frozen-manifest and raw-data integrity

The frozen manifest's routing workload digest is a transcription error. The
unchanged tracked file and Linux staged input hash to
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`; the
manifest records `…b1f1d…`. The manifest and sidecar remain unmodified. All
attempts used the same tracked workload bytes, with the actual digest present
in every runner input-hash file.

The complete v1 raw result tree, including 924 file hashes, remains on the
disk-backed Linux host at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results/`. The
raw-file hash manifest SHA-256 is
`17732389b572566bf320783e0e391d7addf0645e03b328e305305c0cad5c322b`; the
matching sidecar is retained alongside it and in the task research directory.
The TSV summaries, attempt order, raw hash manifest, and audit counts in the
task directory were copied from this run. The raw ledgers and logs remain on
the VM rather than being committed into the repository.
