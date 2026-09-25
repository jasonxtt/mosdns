# Slice 3 candidate identity — V7

Captured on 2026-09-26 after the V7 Linux release build and before the seventh
Slice 3 matrix. This pins the boxed terminal-observation candidate and frozen
inputs; no V7 benchmark attempt had started when this identity was captured.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `ad0097ea73374425652586b6fe6d06748309c362` |
| Tracked `rust/` tree | `9ad33d3c4317f2d3bf221d7ce4f15e296d8ca38c` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v7/rust` |
| Tracked source files | 107; all matched the committed source manifest before build |
| Source-file manifest SHA-256 | `86e6c3c9a40846cf0a19ad9a4c6253d0a642bbfd1381ee79a129f3aacc56d43f` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T19:04:53Z` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v7/rust/target/release/mosdns` |
| Executable SHA-256 | `6c799a905cd4bc33da241238473bc449abc7ad077e430723f79d514a013bd56b` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The runner, helper, base configurations, overlays, and workloads match the
frozen manifest. The actual tracked routing workload SHA-256 is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest's transcription error for that row and its sidecar remain
untouched. V1 through V7 use the same tracked workload bytes. The runner uses
`pilot` mode as specified by the frozen manifest because its digest differs
from the archived official matrix driver.

The runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`.
Rust-before remains the frozen executable with SHA-256
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

The seventh matrix uses a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v7/`.
It follows the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder
plus the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline,
100 ms late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness
CPU 1. Only W1 TCP, W2 cold/warm in one process, and W3 are run. Completed
facts move into a boxed listener-owned slot; terminal finalization uses no
completed-event checkpoint lock, while interrupted execution continues to
use the shared checkpoint.

Before the first attempt, the V7 driver rechecks the runner, helper, frozen
Rust-before binary, candidate binary, lockfile, base YAML files, workloads,
audit-on overlays, and toolchain. Each attempt records port availability and
restores fixture configs; production service `mos` is not contacted.
