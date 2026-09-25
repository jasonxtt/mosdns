# Slice 3 candidate identity

Captured on 2026-09-25 at 15:05 UTC, before the first Slice 3 pilot attempt.
This records the exact Rust-after executable and the inputs selected for the
frozen low/moderate-load pilot. It is identity evidence, not a benchmark result.

## Candidate build

| Item | Identity |
|---|---|
| Source commit | `545ba29fe6c02d47db1ee1be40b781c04646cac0` |
| Tracked `rust/` tree | `40f5c730d79e43c550487699e9fd831073b63fa8` |
| Staged Rust source | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-src/rust` |
| Source transfer check | SHA-256 of every tracked file under `rust/` matched the committed local tree before the run. |
| Build command | `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Lockfile SHA-256 | `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Executable | `/root/mosdns-rust-phase5a-native-query-observability-545ba29/candidate-src/rust/target/release/mosdns` |
| Executable SHA-256 | `a20bac363de9fcf7ce21c2ef09f59a695d74ac8b7ace6f6126c87e6faec3370f` |
| Helper validation | `phase5a-baseline-helper validate-binary --path …/mosdns` passed with the same SHA-256. |

## Pilot support inputs

| Input | SHA-256 |
|---|---|
| `scripts/run-phase5a-baseline.sh` | `dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f9410e1e3d8` |
| `tests/phase5a-baseline/cmd/phase5a-baseline/main.go` | `2dec4788eaa3dad251061e380cbd8c32c9122285b852acf8b9a0b2eb81f64da1` |
| Linux `phase5a-baseline-helper` v8 | `df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065` |
| `forward-udp.yaml` | `f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729` |
| `forward-tcp.yaml` | `1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1` |
| `cache.yaml` | `7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7` |
| `routing.yaml` | `66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651` |
| `forward.jsonl` workload | `32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2` |
| `cache.jsonl` workload | `7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed` |
| `routing.jsonl` workload | `dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1` |
| W1 TCP audit-on overlay | `72db879e9fbee4fb87400b54da31dfd41f82f6766dbe6081ab3d4690e42df24c` |
| W2 audit-on overlay | `c973586aee0f0381d96256afb305ef773f240a4aab7033303990c013d4cf7158` |
| W3 audit-on overlay | `0cc96555e135529a9acb7b49dbbefdf94d1d8b9e8cf2c2d4c37a980e099521b5` |

The three overlays only replace the frozen listener setting
`enable_audit: false` with `enable_audit: true`. Before each audit-on attempt,
the selected staged YAML is replaced by its hash-verified overlay and restored
immediately afterward. The runner records its active fixture, runner, helper,
and candidate hashes in each attempt's `input-hashes.sha256`.

The frozen manifest's routing workload row contains a transcription error: it
records `dbb582dc9d623af54e501639b7a52538b1f1d9e493fa0f0f6a4f4714ea0176c1`,
while SHA-256 of the unchanged tracked workload in both the local checkout and
Linux stage is
`dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`. The
tracked file is clean and both copies match byte-for-byte. The frozen manifest
and its sidecar remain untouched; raw runner hashes record the actual input.
The initial driver preflight detected this mismatch and stopped before any
pilot attempt started.

## Host and result location

- Host: SSH alias `mosdns-rust`, Linux amd64, kernel
  `7.0.9-x64v3-xanmod1`, two online CPUs (`0-1`), Rust 1.95.0, ext4 root,
  4,102,578,176 bytes total memory, 1,024 open-file limit.
- Identity recheck time: `2026-09-25T15:05:26Z`; observed load average was
  `0.24 / 0.24 / 0.19`. Per-attempt `/proc/loadavg` samples are retained with
  the raw results.
- Candidate pilot results are written under
  `/root/mosdns-rust-phase5a-native-query-observability-545ba29/results/` on
  the disk-backed root filesystem. Each result directory retains the runner's
  raw ledgers, stage summaries, counters/events, process resource samples,
  hashes, logs, and invalid-stage reasons.
- The Rust-before comparator remains the frozen binary and SHA in
  `performance-manifest.md`; only Rust-after uses this candidate executable.
- No pilot run had started when this identity record was written. The first
  driver invocation performed the frozen-input checks, caught the manifest
  transcription error above, and exited before launching the runner.
