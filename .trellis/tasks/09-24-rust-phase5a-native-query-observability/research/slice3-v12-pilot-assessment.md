# Slice 3 V12 pilot assessment

## Candidate and run identity

V12 source commit: `eddcb48057096f1f8562d55b0bbc6290bff35756`. The pinned
Linux amd64 release binary is 2,306,464 bytes with SHA-256
`8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d`;
helper v8 accepted that exact binary. The source manifest contained 107
tracked Rust files and matched before build. The candidate identity, frozen
27-attempt plan, and driver are recorded in `slice3-candidate-v12-identity.md`
and adjacent files.

The one official frozen matrix completed from 2026-09-25 22:29:51 UTC through
22:39:55 UTC. All 27 planned attempts ran once, each runner exit was 0, and
there were no replacement attempts or `invalid-stages.tsv` files. The 63/63
primary rows were valid. All 54,000 scheduled requests were sent and received,
and all 54,000 were correct on time. Late, wrong, protocol-error,
transport-error, timeout, and sender-shortfall counts were all zero.

The 901-entry raw-file hash manifest has SHA-256
`52b9b1be2dc59ada84beb304b317f00e58b1b0ecbade1cd544d3c4cb51e26c34`.
The full ~61 MiB result tree remains on the Linux benchmark host at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v12-run`;
the manifest and sidecar were reverified there after the run. The report,
derived primary and paired TSVs, run audit, attempt order, binary validation
JSON, and raw hash manifest are captured under `slice3-v12-run/results-v12-run/`.

## Primary latency summaries

Median p50/p95/p99 across the three repetitions for each frozen primary row,
in microseconds:

| Scenario and load | Rust-before | V12 audit off | V12 audit on |
|---|---:|---:|---:|
| W1 TCP, 200 QPS | 269 / 516 / 664 | 316 / 617 / 1,288 | 291 / 546 / 890 |
| W1 TCP, 400 QPS | 201 / 445 / 939 | 199 / 443 / 689 | 230 / 543 / 1,277 |
| W2 cold, 200 QPS | 158 / 317 / 512 | 161 / 323 / 449 | 161 / 314 / 417 |
| W2 warm, 200 QPS | 186 / 330 / 599 | 193 / 374 / 681 | 173 / 337 / 494 |
| W2 warm, 400 QPS | 100 / 269 / 486 | 111 / 282 / 484 | 104 / 256 / 403 |
| W3, 200 QPS | 427 / 823 / 1,116 | 433 / 795 / 1,123 | 332 / 695 / 1,099 |
| W3, 400 QPS | 268 / 637 / 1,049 | 265 / 625 / 1,053 | 267 / 599 / 947 |

The frozen paired analyzer reports five repeated p95/p99 guard crossings:

| Comparison | Metric | Paired deltas | Guard |
|---|---|---:|---:|
| W1 TCP 200, audit off vs Rust-before | p95 | +20.04%, -18.80%, +40.70% | 10.00% |
| W1 TCP 200, audit off vs Rust-before | p99 | +94.56%, -31.66%, +129.67% | 10.00% |
| W1 TCP 400, audit on vs audit off | p95 | +60.42%, +22.57%, -8.14% | 10.00% |
| W1 TCP 400, audit on vs audit off | p99 | +85.34%, +79.32%, -3.80% | 28.74% |
| W2 warm 200, audit off vs Rust-before | p95 | +18.14%, -7.27%, +24.25% | 17.58% |

The other paired p95/p99 comparisons did not cross their frozen guards. No
CPU comparison crossed a guard; all 14 CPU comparisons were inconclusive at
the host's 100-Hz sampling resolution. The largest positive paired sampled-RSS
deltas were +192 KiB for audit off versus Rust-before (8,192 KiB budget) and
+1,884 KiB for audit on versus audit off (32,768 KiB budget).

## Assessment

V12 is not reviewable. The repeated latency guard count fell from V11's seven
to five, and the W3 comparisons no longer repeat, but the frozen acceptance
gate still blocks review. The W1 TCP 400 audit-on p95/p99 crossings appeared
in V12. No causal attribution is established from these measurements. A
corrective candidate would need a newly pinned source and binary identity and
the complete frozen matrix; neither the thresholds nor the failed attempts
may be selectively replaced.
