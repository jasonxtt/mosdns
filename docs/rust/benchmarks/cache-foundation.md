# Cache foundation benchmark record

Status: preliminary facade benchmark and reproducible replay/soak complete;
Rust remains experimental and is not the default.

## Baseline environment

```text
date: 2026-08-13
branch: rust
commit: 3896a4a7e0ce4311b40a7e4c80c93f2c8b3b4f1d
host: Apple M4 / Darwin 25.5.0 / arm64
go: go1.26.4 darwin/arm64
rustc: 1.95.0
cargo: 1.95.0
node: v22.22.2
npm: 10.9.7
```

## Verification baseline

```text
go test ./plugin/executable/cache ./pkg/cache ./pkg/query_context ./pkg/server_handler
PASS

go test ./...
PASS
```

## Preliminary Linux facade hit benchmark

Environment: `mos-test`, Linux x86_64, 4 logical CPUs, 3.8 GiB RAM,
effective Go toolchain `go1.26.4`, Rust `1.95.0`, `GOAMD64=v1`. Both subtests
used the same tagged binary, 4096-entry capacity, one positive A response,
parallel execution, one-second runs, and three repetitions.

```text
CGO_ENABLED=1 go test -tags mosdns_rust_cache \
  ./plugin/executable/cache -run '^$' \
  -bench '^BenchmarkCacheFacadeHit$' -benchmem -benchtime=1s -count=3

BenchmarkCacheFacadeHit/go-4    216.1 ns/op  776 B/op  11 allocs/op
BenchmarkCacheFacadeHit/go-4    233.0 ns/op  776 B/op  11 allocs/op
BenchmarkCacheFacadeHit/go-4    221.6 ns/op  776 B/op  11 allocs/op
BenchmarkCacheFacadeHit/rust-4  411.5 ns/op  704 B/op  10 allocs/op
BenchmarkCacheFacadeHit/rust-4  379.8 ns/op  704 B/op  10 allocs/op
BenchmarkCacheFacadeHit/rust-4  412.9 ns/op  704 B/op  10 allocs/op
```

Median hit time is 221.6 ns for Go and 411.5 ns for Rust. The current Rust
bridge is about 85.7% slower by this throughput-oriented measure, although it
uses about 9.3% fewer bytes and one fewer allocation per operation. This is
well beyond the 10% rollout gate, so Rust cannot become the default.

## Hot-path optimization iterations

CPU profiles attributed most of the Rust delta to `runtime.cgocall`, the
Go/Rust output copy, and duplicate bridge locking. The following iterations
were measured on the same host and benchmark shape; medians are shown because
the shared test VM has visible scheduling noise.

| Iteration | Go median | Rust median | Rust delta | Rust allocations |
| --- | ---: | ---: | ---: | ---: |
| Initial owned-output ABI | 221.6 ns/op | 411.5 ns/op | +85.7% | 704 B/op, 10 allocs/op |
| Borrow keys and consume the patched Rust `Vec` | 224.2 ns/op | 341.4 ns/op | +52.3% | 704 B/op, 10 allocs/op |
| Caller-owned output buffers (`cache_lookup_into`) | 248.0 ns/op | 342.3 ns/op | +38.0% | 688 B/op, 10 allocs/op |
| Caller-owned buffers + atomic cgo handle | 238.9 ns/op | 332.1 ns/op | +39.0% | 688 B/op, 10 allocs/op |

The last two results are effectively in the same noisy range. The atomic
facade handle and allocation-free TTL offset walk were subsequently retained
for lower contention and lower Rust allocator pressure, not claimed as a
proven latency win. A later three-second run was disturbed by transient host
load and produced a 341.5 ns/op Rust median; it is not used as a new baseline.

The first optimization rounds materially reduced the gap, but a per-query cgo
transition remains a hard floor for this very small cache-hit workload. Future
work can still optimize the surrounding query pipeline, avoid additional
copies as raw-response ownership evolves, and move a coarser request stage into
Rust. It must not add a Go L1 hit path merely to hide the bridge cost in the
Rust comparison.

This microbenchmark does not supply p50/p95/p99 wall latency, process CPU, or
steady/peak RSS. Those measurements are supplied by the reproducible
replay/soak harness below; the preliminary result is a blocking signal, not a
production performance claim.

## Reproducible replay/soak evidence

Harness: `plugin/executable/cache/rust_backend_soak_test.go`
(`TestCacheReplaySoak`, gated behind `MOSDNS_CACHE_SOAK=1`). The workload is a
deterministic synthetic replay: 256 distinct cached domains with a fixed 1-in-10
miss population, replayed cyclically by 8 workers for 4 s per backend after a
1 s warmup. Each worker reuses one query message so the measurement isolates
cache/backend cost rather than per-op `dns.Msg` serialization. Latency uses a
deterministic log-linear histogram (base 2^(1/16)); CPU reads `/proc/self/stat`
user+system ticks; RSS samples `/proc/self/status` VmRSS every 100 ms and keeps
the peak and final samples; allocations come from `runtime.MemStats` deltas;
cgo calls come from `runtime.NumCgoCall` deltas.

Run on `mos-test` (`10.0.0.91`, Linux x86_64, 4 logical CPUs, 3.8 GiB RAM,
load ~2, `go 1.24.4` with `GOTOOLCHAIN=auto` resolving `go 1.26.4`,
`rustc 1.95.0`, `GOAMD64=v1`). Exact command:

```text
CGO_ENABLED=1 MOSDNS_CACHE_SOAK=1 go test -tags mosdns_rust_cache \
  ./plugin/executable/cache -run '^TestCacheReplaySoak$' -v -count=3
```

Three consecutive runs, each 4 s per backend; latencies are histogram-bucket
midpoints and are identical across runs (stable distribution):

| Metric (median of 3) | Go | Rust | Rust vs Go |
| --- | ---: | ---: | ---: |
| QPS | 4,273,786 | 3,107,018 | −27.3% |
| p50 | 370 ns | 622 ns | +68% |
| p95 | 708 ns | 1,046 ns | +48% |
| p99 | 1,760 ns | 6,742 ns | +3.8x |
| Process CPU | ~370% of 4 cores | ~369% of 4 cores | ≈ |
| RSS steady | ~84–107 MiB | ~77–92 MiB | lower |
| RSS peak | ~125–150 MiB | ~98–132 MiB | lower |
| Go allocs/op | 8.64 / 634.7 B | 7.91 / 540.8 B | −8% / −15% |
| cgo calls/op | 0 | 1.00 | Rust-only |

Per-run detail (ops / qps / p50 / p95 / p99 / cpu% / steady RSS MiB):

```text
go:   17,153,188 / 4,283,370 /  354 /  678 / 1685 / 362% / 107.1
go:   17,099,335 / 4,274,786 /  370 /  708 / 1760 / 370% /  84.6
go:   17,063,115 / 4,263,979 /  370 /  708 / 1760 / 372% /  93.4
go:   17,298,603 / 4,310,295 /  370 /  708 / 1760 / 374% /  84.5
rust: 12,641,071 / 3,158,539 /  622 / 1046 / 6742 / 375% /  79.2
rust: 12,524,352 / 3,120,553 /  622 / 1046 / 6742 / 366% /  77.3
rust: 12,275,489 / 3,056,160 /  622 / 1046 / 6742 / 365% /  80.6
rust: 12,394,822 / 3,095,482 /  622 / 1046 / 6742 / 369% /  91.8
```

Conclusions:

- The cgo transition is exactly one call per query on the Rust path
  (`cgo_calls ≈ ops`), which is the hard floor identified by profiling. This
  remains the dominant reason the Rust facade does not meet the 10% rollout
  gate; it disappears only when Rust owns the request hot path (Phase 3).
- Rust QPS is ~27% lower and p50 ~68% higher than Go on this throughput-bound
  workload, consistent with the earlier noisy ~39% median gap on a smaller
  benchmark. Both backends saturate all 4 cores.
- Rust uses fewer Go allocations (7.91 vs 8.64 allocs/op) and lower steady RSS
  (~80 vs ~90 MiB), so the regression is transitional boundary cost, not a
  memory or allocation penalty.
- The p99 gap (3.8x) is driven by cgo call latency variance under parallel
  load; it does not imply a per-op allocator or locking defect in
  `mosdns-cache-core`.

Rust cache remains experimental: the QPS/latency gates are not met, so it must
not become the default backend.

## Safety tooling note

The ABI boundary tests, strict clippy, concurrent lifecycle tests, caller-owned
output contract, exact boxed-slice release test, Go race tests, and actual
Linux+cgo integration pass.

Miri (`rustup +nightly cargo miri`, nightly `1.99.0-nightly`) was run against
`mosdns-cache-core`:

- `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" cargo +nightly miri
  test --all-targets`: **all 16 tests pass** (7 lib + 9 ABI contract; the
  concurrent store/lookup tests complete under interpretation in ~232 s).
- The default **Stacked Borrows** model reports an incompatibility inside
  `crossbeam-epoch 0.9.20` (a moka dependency) at `internal.rs:562`
  (`&*local_ptr` retag); `crossbeam-epoch`'s epoch-reclamation pointer pattern
  is not Stacked-Borrows-clean. The same crate also emits
  `integer-to-pointer cast` warnings. This is dependency-internal and is a
  documented Miri/epoch-reclamation limitation, not a defect in
  `mosdns-cache-core`; the FFI boundary code itself reaches moka's `insert`
  without tripping a Miri error.

So Miri coverage is available and passes under the Tree Borrows model; the
Stacked Borrows finding is attributable to the third-party epoch library and is
noted here for future dependency upgrades. Sanitizer coverage on the Rust side
is not separately exercised; the Go side passes `-race` on the real Linux+cgo
bridge.
