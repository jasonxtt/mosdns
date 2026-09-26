# Slice 3 candidate identity — V2

Captured on 2026-09-26 before the second Slice 3 matrix. This records the
histogram hot-path correction candidate and frozen inputs; it is identity
evidence, not a benchmark result.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `6aa1df3d9e63b92b99f7be0ece4c6c5b0244cd0a` |
| Tracked `rust/` tree | `2492a333c9ef94c12948e6852d8c404a40aa2e87` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v2/rust` |
| Source transfer check | All 107 tracked Rust files matched the committed local tree before build. |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T15:59:37Z` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v2/rust/target/release/mosdns` |
| Executable SHA-256 | `9506cb7ccad7a26b1c95fff51e59ecec38c77497fa3b0d4eb10236c285e05a6f` |
| Helper validation | `phase5a-baseline-helper validate-binary --path …/mosdns` passed with the same SHA-256. |

## Frozen pilot inputs

The runner, helper, base configurations, and workloads match the frozen
manifest. The actual tracked routing workload SHA-256 is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest records `…b1f1d…` for this one row; that transcription
error and its sidecar remain untouched. V1 and V2 use the same unchanged
tracked workload bytes. The expected audit-on overlay hashes remain those in
the frozen manifest and V1 identity record. The runner uses `pilot` mode as
specified by the frozen manifest because its current digest differs from the
archived official matrix driver.

The exact runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`.
Rust-before remains the frozen executable and SHA-256 from
`performance-manifest.md`.

The second matrix uses a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v2/`.
It follows the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder
plus the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline,
100 ms late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness
CPU 1. Only W1 TCP, W2 cold/warm in one process, and W3 are run. No V2 attempt
had started when this identity was written.

Before the first attempt, remote preflight rechecked the runner, helper,
lockfile, candidate binary, all four base YAML files, all three workload files,
and the build toolchain. Their hashes matched the frozen values, including the
documented routing workload transcription correction. The three audit-on
overlay hashes, port checks, and restore trap are recorded in the V2 run audit.
