# Slice 3 candidate identity — V12

Captured on 2026-09-26 after the V12 Linux release build and helper validation,
before any V12 benchmark attempt. V12 keeps the executable ID in the in-flight
checkpoint until exchange completion, then resolves the upstream identity
once. If execution is interrupted while the exchange is pending, the drop hook
resolves the identity for the terminal attempt. The enabled audit record derives
`final_upstream` from the final response source, keeping the public record value
without retaining a second execution-owned string. Frozen inputs and thresholds
are unchanged.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `eddcb48057096f1f8562d55b0bbc6290bff35756` |
| Tracked `rust/` tree | `96421a4150af0521c8966520e1fd9342c4baf3f1` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v12/rust` |
| Tracked source files | 107; every file matched the committed source manifest before build |
| Source-file manifest | `slice3-v12-rust-source-files.sha256`; SHA-256 `d243be705f5dfcf7feafb05f12347af4d857d8c06101673ceffee2a3e2472157` |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` from `candidate-v12` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)`; Cargo 1.95.0 |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Build completed | `2026-09-25T22:25:57Z` (file mtime `22:25:57.983641184 +0000`) |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-v12/rust/target/release/mosdns` |
| Executable size | 2,306,464 bytes |
| Executable SHA-256 | `8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d` |
| Helper validation | helper v8 `validate-binary` passed with the same SHA-256 |

## Frozen pilot inputs

The V12 run uses the unchanged frozen inputs and helper from
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
manifest input used by V1 through V11.

| Candidate evidence | SHA-256 |
|---|---|
| Frozen 27-attempt order | `slice3-v12-attempt-order-plan.tsv`; `3ef50747b1e0fea29a2f46b35e92c420a11a1a285dddd31366b24730fed2a4a0` |
| V12 driver | `run-slice3-v12-matrix.sh`; `1e1df21fb1b6a8d13d424c350bb05526185b2a83ba39663d41770756ee974e3f` |
| Analyzer | `summarize-slice3-pilot-v2.py`; `3b3b4559b62d10088e45e41aa59b46bfa43b8897a8805e57edf08abbee622dc6` |

The result root for the one official run is the fresh disk-backed directory
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v12-run/`.
The driver retains all 27 attempts without replacement, on W1 TCP, W2 cold/warm
in one process, and W3, using 200/300/350/400 QPS plus the 200 QPS health stage,
3,000 ms per stage, a 500 ms request deadline, 100 ms late drain, W2
TTL/safety-margin 30,000/500 ms, SUT CPU 0, and harness CPU 1. A separate
preflight-only invocation uses `results-v12-preflight` and starts no benchmark
attempt. The driver checks helper v8 identity, binary validation, source
manifest, lockfile, frozen fixture/workload/overlay hashes, CPU sets, free
ports, and disk-backed result storage before any attempt.

No V12 benchmark attempt had started when this identity was captured.
