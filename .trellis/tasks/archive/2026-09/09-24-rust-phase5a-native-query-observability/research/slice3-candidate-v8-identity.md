# Slice 3 candidate identity — V8

Captured on 2026-09-26 after the V8 Linux release build and before the eighth
Slice 3 matrix. This pins the atomic admission-counter candidate and frozen
inputs; no V8 benchmark attempt had started when this identity was captured.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `b7c4935e176809bdd3fc46fbea0cca13887a89f4` |
| Tracked `rust/` tree | `d59a19d4f6e8bd5ae22c620cba66e93c9331063c` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v8/rust` |
| Tracked source files | 107; all matched the committed source manifest before build |
| Source-file manifest SHA-256 | `c8f7b363d531c9d52691b984a640dd01c932683137fdc6fe398759e1030fd5b5` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T19:36:34Z` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v8/rust/target/release/mosdns` |
| Executable SHA-256 | `e9e92965d724438ea973f31e560432fc154c419ecc0be2133659607078176fda` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The runner, helper, base configurations, overlays, and workloads match the
frozen manifest. The actual tracked routing workload SHA-256 is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest's transcription error for that row and its sidecar remain
untouched. V1 through V8 use the same tracked workload bytes. The runner uses
`pilot` mode as specified by the frozen manifest because its digest differs
from the archived official matrix driver.

The runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`.
Rust-before remains the frozen executable with SHA-256
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

The eighth matrix uses a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v8/`.
It follows the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder
plus the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline,
100 ms late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness
CPU 1. Only W1 TCP, W2 cold/warm in one process, and W3 are run. Admission
increments a sequentially consistent atomic in-flight counter without taking
the observer mutex; admitted totals are derived as completed plus in-flight
inside a mutex-consistent metrics snapshot.

Before the first attempt, the V8 driver rechecks the runner, helper, frozen
Rust-before binary, candidate binary, lockfile, base YAML files, workloads,
audit-on overlays, and toolchain. Each attempt records port availability and
restores fixture configs; production service `mos` is not contacted.
