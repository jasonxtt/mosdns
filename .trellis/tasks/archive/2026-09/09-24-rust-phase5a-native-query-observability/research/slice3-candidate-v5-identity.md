# Slice 3 candidate identity — V5

Captured on 2026-09-26 before the fifth Slice 3 matrix. This records the
listener-owned terminal facts candidate and frozen inputs; it is identity
evidence, not a benchmark result.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `f73d0967ce3325e35bef46c8b0ef7d9801eb797e` |
| Tracked `rust/` tree | `6f7094d5b9d567ae6697fb8ee86ac81e5ddc6473` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v5/rust` |
| Tracked source files | 107; all matched local committed source before build |
| Source-file manifest SHA-256 | `2f28b4f6fadf8c810ac6936626c8740fd331abb4d565267a8b8af542d6dc3528` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T18:11:27Z` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v5/rust/target/release/mosdns` |
| Executable SHA-256 | `0d11f89837b7de972cd4d8d080932de50ec7d77708f0a1d46bd50c98a149e42f` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The runner, helper, base configurations, and workloads match the frozen
manifest. The actual tracked routing workload SHA-256 is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest records `…b1f1d…` for this one row; that transcription
error and its sidecar remain untouched. V1 through V5 use the same unchanged
tracked workload bytes. The expected audit-on overlay hashes remain those in
the frozen manifest and V1 identity record. The runner uses `pilot` mode as
specified by the frozen manifest because its current digest differs from the
archived official matrix driver.

The runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`.
Rust-before remains the frozen executable with SHA-256
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

The fifth matrix uses a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v5/`.
It follows the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder
plus the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline,
100 ms late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness
CPU 1. Only W1 TCP, W2 cold/warm in one process, and W3 are run. No V5 attempt
had started when this identity was written.

The normal successful path transfers final facts directly into the listener
guard and avoids writing and reading the completed-event checkpoint. The Drop
path uses that owned event if send is interrupted and falls back to the shared
execution checkpoint when execution itself is interrupted. When audit is
enabled, the ring reserves at most 1,024 initial entries; audit-off reserves
none.

Before the first attempt, remote preflight rechecks the runner, helper,
lockfile, candidate binary, all four base YAML files, all three workload
files, and the build toolchain. Their hashes must match the frozen values,
including the documented routing workload transcription correction. The
three audit-on overlay hashes, per-attempt port checks, and config restore
trap are checked by the V5 driver and recorded in its run audit.
