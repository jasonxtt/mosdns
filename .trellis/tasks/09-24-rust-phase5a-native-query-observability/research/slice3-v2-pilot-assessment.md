# Slice 3 pilot V2 assessment

Measured 2026-09-25 on the isolated `mosdns-rust` Linux host with Rust-before
commit `605c30577b79d397b5695618dbd2980e550ca6f3` and Rust-after V2 commit
`6aa1df3d9e63b92b99f7be0ece4c6c5b0244cd0a`. V2 executable SHA-256:
`9506cb7ccad7a26b1c95fff51e59ecec38c77497fa3b0d4eb10236c285e05a6f`.
Candidate identity, frozen inputs, attempt order, per-stage measurements, and
paired guard calculations are recorded in the adjacent V2 artifacts.
The complete Linux result tree remains at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v2/`;
its 901-file SHA-256 manifest is
`slice3-v2-raw-file-hashes.sha256` with digest
`255f54df8807ad8adf209b04a29e25f3f50a2baceb6c7553a6348cc0aa3b7f36`.
Remote `sha256sum --quiet -c raw-file-hashes.sha256` verified every listed
file before the manifest was copied into this task directory.

## Result

All 27 planned attempts completed with runner exit code 0. All 63 selected
primary-stage rows passed sender, response, deadline, and W1/W2/W3 fixture
oracles. No attempt was replaced. The first summary invocation failed because
the V2 attempt-order file includes a column header while the original analyzer
accepted only headerless rows. The original attempt-order file and analyzer
were preserved; a V2 analyzer that skips the header generated the summaries
without changing measurement data or rerunning any attempt.

V2 still crosses the frozen performance gate and is not submitted for review.
The repeated latency regressions are:

| Comparison | Point | Paired p95 or p99 deltas | Frozen guard |
|---|---|---:|---:|
| Audit-on vs audit-off | W1 TCP p95, 200 QPS | +49.22%, −9.68%, +19.54% | 10.00% |
| Audit-on vs audit-off | W1 TCP p99, 200 QPS | +77.69%, −25.88%, +31.27% | 14.53% |
| After-off vs before-off | W1 TCP p95, 400 QPS | +14.29%, +15.04%, −3.46% | 10.00% |
| After-off vs before-off | W1 TCP p99, 400 QPS | +58.88%, +28.19%, +2.77% | 10.00% |
| After-off vs before-off | W2 cold p99, 200 QPS | +22.98%, +52.75%, +27.54% | 10.64% |
| Audit-on vs audit-off | W2 warm p95, 200 QPS | +26.81%, +13.74%, +12.46% | 10.00% |
| Audit-on vs audit-off | W2 warm p99, 200 QPS | +118.38%, +60.81%, +14.32% | 22.99% |
| After-off vs before-off | W2 warm p95, 400 QPS | +14.54%, +24.18%, +7.84% | 10.00% |
| After-off vs before-off | W2 warm p99, 400 QPS | +28.88%, +46.83%, +9.09% | 18.73% |
| Audit-on vs audit-off | W2 warm p95, 400 QPS | +55.77%, +51.82%, +1.09% | 10.91% |
| Audit-on vs audit-off | W2 warm p99, 400 QPS | +162.97%, +14.76%, +8.08% | 13.21% |
| Audit-on vs audit-off | W3 p95, 400 QPS | +37.31%, +17.12%, +34.67% | 22.92% |
| Audit-on vs audit-off | W3 p99, 400 QPS | +118.91%, +21.25%, +147.96% | 10.00% |

The W3 audit-on vs audit-off CPU-per-correct-query comparison at 400 QPS also
crossed its frozen guard: +36.36%, +33.33%, and +33.33% against 25.00%; all
three paired process-tick differences exceed the two-tick resolution limit.
Other CPU comparisons are generally inconclusive at 100-Hz process-tick
resolution and remain labeled that way in the paired-assessment TSV.

Sampled RSS stayed within both frozen budgets. The largest audit-off paired
increase was 276 KiB against the 8 MiB minimum guard. The largest audit-on
versus audit-off increase was 1,908 KiB against 32 MiB. These short samples do
not establish long-term retention growth.

The host load stayed low during the matrix: initial load averages were
`0.04 / 0.09 / 0.12`; the largest recorded one-minute average was `0.72` on a
two-CPU host. Load does not account for all observed variation, and it does
not remove the repeated guard crossings. No capacity, overload, recovery, or
production performance claim is made.

## Candidate response

Changing cumulative histogram updates to one exclusive bucket update reduced
that specific hot-path work, but did not clear the full regression gate. V2
results point to avoidable per-request copying between listener execution
facts, the cancellation checkpoint, and the retained audit record. The
follow-up source change now moves completed execution facts through the shared
checkpoint and into metrics/audit retention without cloning them on the normal
send path. It preserves the checkpoint's interrupted-execution behavior,
public snapshot contents, and fixed retention boundary. The focused
native-host suite, workspace tests, formatting, and strict clippy passed for
this local correction. A new pinned Linux build and complete frozen matrix are
still required; V2 remains intact as a failed candidate.
