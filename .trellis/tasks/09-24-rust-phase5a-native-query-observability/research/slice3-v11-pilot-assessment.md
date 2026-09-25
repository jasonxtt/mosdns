# Slice 3 V11 pilot assessment

## Candidate and run identity

V11 source commit: `7b7a0822b3820b4b7d56a5270893ed3eb6f13c69`. The pinned Linux amd64 release binary is 2,307,104 bytes with SHA-256
`a50ac020785f9e55f42c2455690d5ceaedf0edd37e5d33e39fde649352e3f75c`; helper v8 accepted that exact binary. The source manifest contained 107 tracked Rust files and matched before build. The candidate identity, frozen 27-attempt plan, and driver are recorded in `slice3-candidate-v11-identity.md` and adjacent files.

The one official frozen matrix completed from 2026-09-25 21:47:45 UTC through 21:57:48 UTC. All 27 planned attempts ran once, each runner exit was 0, and there were no replacement attempts or `invalid-stages.tsv` files. The 63/63 primary rows were valid. All 54,000 scheduled requests were sent and received, and all 54,000 were correct on time. Late, wrong, protocol-error, transport-error, timeout, and sender-shortfall counts were all zero.

The 901-entry raw-file hash manifest has SHA-256
`533356cafd58da4684f428fb3812206beb8105793313d5d6cfd895b03c1e1372`.
The full ~61 MiB result tree remains on the Linux benchmark host at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v11-run`;
the manifest and sidecar were reverified there after the run. The report,
derived primary and paired TSVs, run audit, attempt order, candidate binaries'
validation JSON, and raw hash manifest are captured under
`slice3-v11-run/results-v11-run/`.

## Primary latency summaries

Median p50/p95/p99 across the three repetitions for each frozen primary row,
in microseconds:

| Scenario and load | Rust-before | V11 audit off | V11 audit on |
|---|---:|---:|---:|
| W1 TCP, 200 QPS | 286 / 544 / 863 | 355 / 818 / 1,580 | 337 / 634 / 876 |
| W1 TCP, 400 QPS | 223 / 508 / 934 | 256 / 601 / 1,300 | 263 / 577 / 1,199 |
| W2 cold, 200 QPS | 168 / 325 / 828 | 186 / 340 / 715 | 192 / 334 / 668 |
| W2 warm, 200 QPS | 172 / 516 / 835 | 162 / 303 / 465 | 167 / 315 / 484 |
| W2 warm, 400 QPS | 116 / 296 / 607 | 109 / 284 / 517 | 111 / 274 / 452 |
| W3, 200 QPS | 344 / 677 / 1,247 | 355 / 687 / 1,083 | 415 / 830 / 1,282 |
| W3, 400 QPS | 262 / 584 / 835 | 267 / 587 / 1,064 | 299 / 667 / 1,246 |

The frozen paired analyzer reports seven repeated p95/p99 guard crossings:

| Comparison | Metric | Paired deltas | Guard |
|---|---|---:|---:|
| W1 TCP 200, audit off vs Rust-before | p95 | +56.25%, +53.76%, -20.12% | 10.00% |
| W1 TCP 200, audit off vs Rust-before | p99 | +134.07%, +98.99%, -22.96% | 15.99% |
| W1 TCP 400, audit off vs Rust-before | p99 | -30.19%, +43.01%, +39.19% | 37.69% |
| W2 cold 200, audit off vs Rust-before | p99 | +18.84%, +26.29%, -18.29% | 11.35% |
| W3 200, audit on vs audit off | p95 | +20.82%, +27.50%, -3.84% | 10.48% |
| W3 400, audit off vs Rust-before | p99 | +36.02%, +3.47%, +32.17% | 10.00% |
| W3 400, audit on vs audit off | p95 | +12.27%, +23.06%, +15.89% | 15.33% |

The other paired p95/p99 comparisons did not cross their frozen guards. No CPU comparison crossed a guard; 13 of 14 CPU comparisons were inconclusive at the host's 100-Hz sampling resolution. The largest positive paired sampled-RSS deltas were +132 KiB for audit off versus Rust-before (8,192 KiB budget) and +1,964 KiB for audit on versus audit off (32,768 KiB budget).

## Assessment

V11 is not reviewable. It reduced the count of repeated latency guard crossings from V10's nine to seven, with no correctness, RSS-budget, or runner-validity failures, but the frozen acceptance gate still blocks review. Do not start the C2C review until the repeated guards are resolved or explicitly handled under the task gate.

Source inspection identifies two hypotheses for the next candidate, not proven causes. First, V11 moved `upstream_identity` lookup and string allocation from after the upstream exchange to immediately before awaiting it, so the allocation now lies directly on the request's upstream-dispatch critical path. Second, the audit-enabled success path stores the same upstream identity in the attempt record, `final_upstream`, and `ResponseSource::Upstream`; W3 has the greatest multi-leg audit detail. These are bounded places to reduce hot-path work while preserving the public observation values and cancellation facts. The next pinned candidate must keep every frozen input and guard unchanged and rerun the full matrix.
