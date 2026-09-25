# Slice 3 candidate identity — V10

Captured on 2026-09-26 after the V10 Linux release build and helper validation,
before any V10 benchmark attempt. This candidate uses the frozen low/moderate
pilot plan; the V9 guard failures remain unresolved until this matrix reports
otherwise.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `65a31ff57046e0047006ea401aa023519c658c16` |
| Tracked `rust/` tree | `6275e691d181aa5c9dc7b3f499c3a4fba32bc9c0` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v10/rust` |
| Tracked source files | 107; every file matched the committed source manifest before build |
| Source-file manifest | `slice3-v10-rust-source-files.sha256`; SHA-256 `ac5e49c9ce6a01c0559bf1d504860737c69348d438a4a0751340050b32b270f2` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` from `candidate-v10` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)`; Cargo 1.95.0 |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T21:02:28Z` (file mtime `21:02:28.869771972 +0000`) |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v10/rust/target/release/mosdns` |
| Executable size | 2,304,904 bytes |
| Executable SHA-256 | `bb13371306d6a26228bcca7e496e8f1354e1d9ac0ecfa2ef36bb5c7b01dba2bd` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The V10 run uses the unchanged frozen inputs and helper from
`performance-manifest.md` (SHA-256
`b687be8014a78008127d9df818123c839b69ffdda4764eb99c174b9e085965a8`). The
runner SHA-256 is
`dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8`; helper
v8 SHA-256 is
`df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065`; and
Rust-before SHA-256 is
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`. The
baseline manifest's historical routing-workload transcription error and its
sidecar remain untouched; staged routing workload bytes match the frozen
manifest input used by V1 through V9.

| Candidate evidence | SHA-256 |
|---|---|
| Frozen 27-attempt order | `slice3-v10-attempt-order-plan.tsv`; `6efa44160e7aae671d5a71bff46c1a262302b95047f6231c6d5ec6a4cdb903e7` |
| V10 driver | `run-slice3-v10-matrix.sh`; `af1105c8f1525199b1f07c06da67d76fe48bb5973c0c607e7fec4ecccc0ddb09` |
| Analyzer | `summarize-slice3-pilot-v2.py`; `3b3b4559b62d10088e45e41aa59b46bfa43b8897a8805e57edf08abbee622dc6` |

The result root for the one official run is the fresh disk-backed directory
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v10-run/`.
The driver retains all 27 attempts without replacement, on W1 TCP, W2 cold/warm
in one process, and W3, using 200/300/350/400 QPS plus the 200 QPS health stage,
3,000 ms per stage, a 500 ms request deadline, 100 ms late drain, W2
TTL/safety-margin 30,000/500 ms, SUT CPU 0, and harness CPU 1. A separate
preflight-only invocation uses `results-v10-preflight` and starts no benchmark
attempt. The driver checks the helper v8 identity, binary validation, source
manifest, lockfile, frozen fixture/workload/overlay hashes, CPU sets, free
ports, and disk-backed result storage before any attempt.

No V10 benchmark attempt had started when this identity was captured.
