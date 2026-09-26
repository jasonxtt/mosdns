# Slice 3 candidate identity — V6

Captured on 2026-09-26 after the V6 Linux release build and before the sixth
Slice 3 matrix. This pins the candidate and frozen inputs; no V6 benchmark
attempt had started when this identity was captured.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `cd96b0adf76767cfede5f20a929ad7ad36e0ce44` |
| Tracked `rust/` tree | `246910bbbe1a59612e9dbde0d2256231f2c3bc76` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v6/rust` |
| Tracked source files | 107; all matched the committed source manifest before build |
| Source-file manifest SHA-256 | `15afe50832f87c831a4398810e885aa53ef2e20b807d538853e9be88dfd9abda` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T18:38:51Z` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v6/rust/target/release/mosdns` |
| Executable SHA-256 | `fe553e2666a9b4dfeb803a6489cadd9f9ad53e85f60f2a6bb5f68e357beec91e` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The runner, helper, base configurations, overlays, and workloads match the
frozen manifest. The actual tracked routing workload SHA-256 is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest records a transcription error for that one row; the
manifest and its sidecar remain untouched. V1 through V6 use the same unchanged
tracked workload bytes. The runner uses `pilot` mode as specified by the
frozen manifest because its current digest differs from the archived official
matrix driver.

The runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`.
Rust-before remains the frozen executable with SHA-256
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

The sixth matrix uses a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v6/`.
It follows the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder
plus the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline,
100 ms late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness
CPU 1. Only W1 TCP, W2 cold/warm in one process, and W3 are run. V6 restores
the heap-backed execution checkpoint for completed terminal facts while
keeping audit-record construction outside the observer lock and reserving at
most 1,024 initial audit-ring entries when capture is enabled.

Before the first attempt, the V6 driver rechecks the runner, helper, frozen
Rust-before binary, candidate binary, lockfile, base YAML files, workloads,
audit-on overlays, and toolchain. Each attempt records port availability and
restores fixture configs; production service `mos` is not contacted.
