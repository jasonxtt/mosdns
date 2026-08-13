# Rust matcher foundation evidence

Date: `2026-08-13`

This is a repeatable Linux+cgo measurement for the Slice 5 matcher fixtures.
It is evidence for the experimental bridge only; it does not change the
default Go backend or authorize a default switch.

## Machine and toolchain

- Host: `mos-test` / `10.0.0.91`, Linux x86_64
- Kernel: `7.0.9-x64v3-xanmod1`
- CPU: `Genuine Intel(R) 0000`, 4 online CPUs
- Memory: `4005900 kB`
- Go: `go1.24.4 linux/amd64`
- Rust/Cargo: `rustc 1.95.0`, `cargo 1.95.0`

The source was copied to the isolated directory
`/tmp/mosdns-rust-review-20260813`; no installed service or production host
was used.

## Fixtures and commands

The benchmark fixtures are generated in the repository test files, so they do
not depend on downloaded rules or private data:

- Domain: 128 `full`, 128 `domain`, 128 `regexp`, and 128 `keyword` rules;
  512 rules and 15,231 UTF-8 bytes after joining with newlines. Lookup
  alternates `full-0042.bench.example.` and a missing name.
- IP: 256 IPv4 `/24` prefixes under `192.0.0.0/16` and 256 IPv6 `/48`
  prefixes under `2001:db8::/32`; 512 prefixes and 8,065 joined bytes.
  Lookup cycles through two hits and two misses.

The static library and benchmark were run with:

```text
scripts/build-rust-cache.sh
CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust BENCHTIME=250ms COUNT=3 \
  scripts/benchmark-rust-matchers.sh
```

The benchmark's `go` subtests build and query the existing Go matcher. Its
`rust` subtests create/query/close the Rust handle through the tagged cgo
wrapper. `index_entries` comes from the Rust ABI `*_matcher_len` call.

Cold compile/load timing used an isolated Rust target and Go build cache:

```text
rm -rf rust/target
cargo build --manifest-path rust/Cargo.toml --package mosdns-runtime --release --locked
GOCACHE=/tmp/mosdns-rust-review-20260813/gocache CGO_ENABLED=1 \
  MOSDNS_MATCHER_BACKEND=rust go test -tags mosdns_rust \
  ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set \
  -run '^$' -count=1
```

Measured wall time was `8.730 s` for the cold Rust runtime static library and
`15.786 s` for the cold Go matcher package compile/test-binary load. The
experimental binary entrypoint was also run with
`SKIP_UI_BUILD=1 OUTPUT=/tmp/mosdns-rust-review-20260813/mosdns-rust-runtime
scripts/build-rust-experimental.sh`; with the Rust target warm, it completed
in `15.138 s` and produced a Linux amd64 ELF binary. The UI was intentionally
skipped for this isolated smoke artifact; CI's corresponding gate runs
`npm ci` and the same entrypoint with embedded UI enabled.

## Lookup and build results

Values are the three raw benchmark runs. Throughput is derived as
`1e9 / median ns/op`, not an independently sampled load-generator result.

| Fixture / operation | Go ns/op | Rust ns/op | Derived median throughput | Rust cgo calls/op |
| --- | ---: | ---: | ---: | ---: |
| Domain build/load | 679,827 / 632,756 / 646,944 | 1,155,286 / 1,158,650 / 1,228,081 | Go `1,545/s`; Rust `864/s` | not reported for build |
| Domain lookup | 3,227 / 3,118 / 3,129 | 2,903 / 2,902 / 3,007 | Go `319,591/s`; Rust `344,471/s` | `3.000` |
| IP build/load | 36,382 / 36,198 / 38,728 | 73,846 / 71,843 / 73,399 | Go `27,486/s`; Rust `13,624/s` | not reported for build |
| IP lookup | 38.71 / 43.63 / 39.62 | 223.3 / 234.7 / 226.7 | Go `25,239,778/s`; Rust `4,411,116/s` | `3.000` |

Snapshot/index indicators from the same run:

| Fixture | Input bytes | Rust `index_entries` | Go build B/op | Rust build Go-side B/op |
| --- | ---: | ---: | ---: | ---: |
| Domain | `15,231` | `512` | `1,035,680` | `16,400` |
| IP | `8,065` | `512` | `55,936` | `8,208` |

The Rust `*_matcher_len` ABI exposes accepted entry counts, not allocated heap
bytes. `fixture_bytes` is the newline-joined input size, and `B/op` is the Go
allocator view of the benchmark process; it is not Rust heap size. A reliable
per-snapshot RSS measurement is not available at this boundary, so no exact
Rust RSS or index-byte figure is claimed. A future native Rust benchmark must
measure allocator/heap and steady process RSS before using memory as a rollout
gate.

The transitional cgo cost is visible in the lookup rows: each Rust lookup
reports three cgo calls/op and includes Go `CString`/FFI wrapper work. On this
fixture the domain path is within the noise of the Go matcher, while the IP
path is about 5.2x slower than direct Go lookup. This is expected evidence for
the current per-plugin bridge, not a reason to add an architectural micro-
optimization; the migration plan keeps the bridge experimental until the
Rust-owned query path can remove this boundary.

## Interpretation

- Rust build/load is slower on both small fixtures, and Rust IP lookup is much
  slower through the transitional cgo wrapper.
- The benchmark is deliberately small and single-process; it does not claim
  production-scale QPS, p99 latency, CPU utilization, or Rust heap/RSS parity.
- Default Go builds and runtime selection remain unchanged. Any provider
  fan-out or `domain_mapper` migration requires a separately approved task;
  this evidence does not authorize it.
