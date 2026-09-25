# Slice 3 candidate identity — V9

Captured on 2026-09-26 after the V9 Linux release build and helper validation,
before the ninth Slice 3 matrix. No V9 benchmark attempt had started when this
identity was captured.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `f22c558365f1fc929752ecf214b29d510118d619` |
| Tracked `rust/` tree | `15ea7e46304950ba1df8c7b5f60cd04cbaa628a2` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v9/rust` |
| Tracked source files | 107; every file matched the committed source manifest before build |
| Source-file manifest | `slice3-v9-rust-source-files.sha256`; SHA-256 `788ef899698077e85f0f72a45b8938bb4693789c6647241a2ee30097d8088810` |
| Build command | `cargo build --manifest-path Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` from `candidate-v9/rust` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T20:15:01Z` (file mtime `20:15:01.892053919 +0000`) |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v9/rust/target/release/mosdns` |
| Executable size | 2,303,968 bytes |
| Executable SHA-256 | `b0917f6fd57eb2f57996aa7125a60a42871edfd67c789bafd31e253c9ffd37b7` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The runner, helper, base configurations, audit-on overlays, workloads, and
manifest match the earlier frozen inputs. The tracked routing workload
SHA-256 is `dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`.
The frozen manifest's transcription error for that row and its sidecar remain
untouched. V1 through V9 use the same tracked workload bytes. The runner uses
`pilot` mode, as specified by the frozen manifest because its digest differs
from the archived official matrix driver.

The V9 driver SHA-256 is
`cb97c4bf3e718eaa432182dae8836c1b10df1947187e19c444cdc669395b75a9`; the
frozen runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; and the
Linux helper v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`. The
Rust-before executable remains pinned at
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.

The ninth matrix has a distinct disk-backed result directory,
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v9/`.
It uses the frozen 27-attempt balanced order, 200/300/350/400 QPS ladder plus
the 200 QPS health stage, 3,000 ms per stage, 500 ms request deadline, 100 ms
late drain, W2 TTL/safety margin 30,000/500 ms, SUT CPU 0, and harness CPU 1.
Only W1 TCP, W2 cold/warm in one process, and W3 are run. The source manifest is
verified again by the driver before any attempt. V9 removes the per-query
shared checkpoint reference count and lock, and stores completed observations
in the checkpoint's existing heap allocation.

Before each attempt, the V9 driver rechecks the runner, helper, frozen
Rust-before binary, candidate binary, source manifest and every manifested Rust
file, lockfile, base YAML files, workloads, audit-on overlays, and toolchain.
Each attempt records port availability and restores fixture configs;
production service `mos` is not contacted.
