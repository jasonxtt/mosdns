# Rust cache foundation — technical design

## 1. System boundary

```text
YAML / sequence / HTTP API / metrics
                |
       Go cache plugin facade
          /             \
 default Go backend   experimental Rust bridge (Linux+cgo)
                            |
                    versioned C ABI
                            |
                 rust/cache-core staticlib
                 Moka + Bytes + DNS wire helpers
```

The Go `cache` plugin remains the sole MosDNS-facing owner. It parses existing arguments, registers API/metrics, owns lazy-update orchestration and selects a backend. Rust does not parse project YAML, register HTTP routes, or know about Vue/coremain.

## 2. Source layout

Proposed minimum layout:

```text
rust/
  Cargo.toml
  cache-core/
    Cargo.toml
    src/{lib,abi,cache,wire,dump}.rs
    LICENSES/ or NOTICE.md
plugin/executable/cache/
  backend.go                 internal backend contract only if needed by two implementations
  backend_rust_linux.go      cgo bridge behind explicit build tags
  backend_rust_stub.go       clear unsupported/disabled result
  testdata/parity/           canonical behavior fixtures
scripts/
  build-rust-cache.sh        deterministic staticlib/header build
docs/rust/
  cache-compatibility.md
  kixdns-reuse.md
  benchmarks/cache-foundation.md
```

Exact filenames may change if source inspection shows an existing convention, but ownership boundaries must not.

## 3. Cache representation

- Use `moka::sync::Cache` unless benchmarks prove the async variant is necessary. It provides bounded, concurrent lookup/eviction without a process-wide lock.
- Keys are owned byte strings derived from the exact current Go key contract, including qname normalization, qtype/qclass, flags, and ECS where enabled. Golden fixtures—not KixDNS behavior—define the result.
- Values hold immutable raw DNS bytes (`bytes::Bytes`), stored/expiry timestamps, and `domain_set` metadata. Raw bytes avoid shared mutable DNS objects and enable wire-level TTL/TXID patching.
- Cache size reports the Rust backend's actual entry count and is the source for the existing gauge when Rust is active.
- Lazy refresh coordination remains in Go for this slice so sequence execution and request context do not cross the ABI. Rust returns a hit classification and aged response.

## 4. DNS wire processing and KixDNS reuse

Start with a reuse ledger pinned to KixDNS `2da3a2d`:

| Capability | Initial decision |
|---|---|
| Moka cache / `Bytes` | direct crates plus adapted design |
| quick query/response parsing | extract/adapt after parity tests |
| TXID and TTL patching | extract/adapt after malformed-wire tests |
| UDP truncation / EDNS payload | defer until raw response integration test needs it |
| ECS prefix masking/key isolation | extract/adapt after MosDNS key fixtures |
| JSON pipeline/engine | reject |

Any copied/adapted file receives origin URL, commit, license, and local modification notes. If a thin wrapper adds no MosDNS-specific value, depend on the underlying crate instead.

## 5. ABI contract

The ABI is deliberately small and coarse-grained. Conceptual operations:

- `abi_version()` and `capabilities()`
- `cache_create(config_bytes, out_handle)` / `cache_close(handle)`
- `cache_lookup(handle, request_bytes, metadata, out_result)`
- `cache_store(handle, request_bytes, response_bytes, metadata)`
- `cache_flush(handle)` / `cache_len(handle)`
- `buffer_release(buffer)` when an owned return buffer is used

`mosdns_cache_v2` remains owned by the Go facade in this slice. The facade
fully parses and validates an import before mutating either backend, then uses
the coarse-grained store operation to mirror validated entries into Rust. This
avoids a second protobuf/gzip implementation and still exercises dump
export/import through the experimental binary's unchanged API.

Rules:

- C-compatible structs contain fixed-width integers, pointer+length views, opaque handles, and explicit ownership flags only.
- Returned owned buffers carry the exact allocation metadata required by their release routine, or use boxed slices whose release needs only pointer+length. Never reconstruct a `Vec` using an assumed capacity.
- Every exported function wraps its body in panic containment and converts failures to stable status codes. A thread-local/handle-scoped diagnostic may expose bounded error text for Go logging.
- Handle close is idempotent from Go's perspective; concurrent operations either complete safely or return a stable closed status. Go owns lifecycle synchronization so a freed raw pointer is never reused.
- The ABI header is generated/validated from the Rust definition, checked into the repository only if reproducible, and has an ABI conformance test.

## 6. Backend selection and fallback

- Normal builds do not require cgo/Rust and always use Go.
- Rust support is compiled only with an explicit build tag (final name chosen during implementation, e.g. `rustcache`) on supported Linux targets.
- While Rust remains experimental, every accepted store is also retained by the Go backend. This deliberate mirror keeps dump/API behavior and the one-way circuit-breaker fallback immediately usable; removing the mirror requires later performance evidence and a separately reviewed rollback design.
- Runtime selection is explicit and experimental; it must not overload existing YAML semantics without a compatibility review. Prefer an operator/environment override for the experiment so generated config remains unchanged.
- Startup ABI mismatch, missing capability, or create/import failure logs one structured warning and selects Go before traffic starts.
- A malformed request/response returns a Go error and uses the established Go path for that request where safe.
- An internal panic/closed/corruption status trips a one-way circuit breaker for that plugin instance: stop new Rust calls, log/count the transition, and continue with a fresh Go backend. The failed Rust instance is not trusted for dump writes.
- Existing dump import is transactional: parse/validate into a new backend and swap only after success. A failed Rust import leaves the original dump untouched and lets Go load it.

## 7. Compatibility and observability

- Existing Prometheus metric names and labels remain stable. Add backend/fallback diagnostics only as additive metrics after checking collision/cardinality.
- Existing show/load/flush HTTP paths remain implemented by the Go facade and return the current shapes.
- `mosdns_cache_v2` remains the persisted format. The mirrored Go facade is the single encoder/decoder; actual Linux+cgo fixtures verify that validated Go dumps populate Rust and malformed dumps mutate neither backend.
- Logs must identify plugin tag, requested/active backend, ABI version, operation, and fallback reason without logging raw DNS payloads.

## 8. Testing strategy

1. Capture Go golden fixtures before introducing Rust behavior.
2. Unit-test Rust cache, wire parser, ECS, dump, and ABI status mapping.
3. Run one table-driven Go parity harness against both backend implementations.
4. Fuzz/malformed tests cover DNS wire, dump blocks, pointer/length validation, and lifecycle ordering.
5. Concurrency tests cover lookup/store/flush/close; Go race tests cover facade/circuit breaker.
6. Linux+cgo integration builds and runs the actual static library, not a mock.
7. Benchmark the same replay workload on pinned hardware/settings and record QPS, latency percentiles, CPU, RSS, and allocations.

## 9. Build and release integration

Add Rust as an incremental experimental job/path. Do not replace `scripts/build-local.sh` or the existing release matrix while Go remains default. Experimental builds must still build both Vue UIs before the final Go link. Cross-target support is added only after the native Linux bridge is correct; no fake portability stubs that silently select Rust.

## 10. Rollout

1. Local tests and benchmarks.
2. Experimental artifact on `mos-test` with parity traffic and restart/dump/API checks.
3. Extended soak with Go fallback telemetry.
4. Separate approval/task to consider production opt-in.
5. Separate approval/task, after sustained evidence, to consider any default change.
