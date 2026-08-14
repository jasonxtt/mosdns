# Rust Migration

The complete architecture and phase gates are in `docs/ai/rust-rewrite-plan.md`. That document overrides abbreviated notes here.

## Architecture

Use a strangler migration: Go continues to own YAML, plugin lifecycle, sequence hosting, WebUI/API, and unmigrated modules; Rust gradually owns bounded data-plane modules behind a versioned C ABI. The first module is cache, followed by matchers, DNS/query execution, sequence, transports, and servers only after the previous gates pass.

## Reuse policy

- Preferred Rust upstream: KixDNS at audited commit `2da3a2d` (2026-08-12).
- Prefer direct crates such as Moka when they provide the needed capability; otherwise classify KixDNS material as direct dependency, extracted code, adapted design, or rejected.
- Preserve attribution and GPL-3.0 obligations for copied/adapted source. Pin reviewed upstream revisions and audit every update.
- `/Users/tom/github/mosdns-rust-cache` is a compatibility reference for MosDNS bridge, dump, API, tests, and fallback—not a subtree to copy wholesale.

## Cache foundation constraints

- Use a concurrent O(1)-style cache design (Moka/sharding) and `Bytes`/raw-wire techniques; do not carry over the old global `Mutex` or linear L1 scan.
- Reuse audited KixDNS DNS wire, TTL, truncation, and ECS ideas only after golden parity against MosDNS semantics.
- Preserve `mosdns_cache_v2`, show/load/flush behavior, `domain_set`, exclusion, lazy TTL, metrics, and raw response paths.
- Keep ABI calls coarse-grained. Check ABI version/capabilities at startup and contain every panic.
- Default releases remain on the Go backend until correctness, safety, performance, and test-host gates are satisfied.

## Phase gate

Do not begin a later migration module merely because its Rust implementation is available upstream. Each module needs an approved Trellis task, frozen compatibility fixtures, parity results, performance evidence, and a rollback path.

## Scenario: cgo borrowed byte slices

### 1. Scope / Trigger

Any Go-to-Rust ABI call that passes one or more Go-owned byte slices through
cgo uses this contract. It prevents the runtime panic caused by passing C a
pointer to a Go struct that itself contains Go pointers.

### 2. Signatures

Expose each borrowed slice as a C-compatible `{const uint8_t *ptr, uint64_t
len}` value. Pass those slice descriptors **by value** to the C function; an
opaque integer handle and fixed-width timestamps may accompany them.

### 3. Contracts

- Go retains each backing slice for the complete call and invokes
  `runtime.KeepAlive` when lifetime is not otherwise obvious.
- Empty input is `{NULL, 0}`; non-empty input is `{live pointer, positive len}`.
- Rust validates pointer/length pairs before constructing a slice and never
  retains a borrowed view after returning.
- Rust-owned output uses the separate owned-buffer/release contract.

### 4. Validation & Error Matrix

- `NULL, positive len` -> `InvalidArgument`
- non-NULL, zero len -> `InvalidArgument`
- length not representable as `usize` -> `InvalidArgument`
- closed/unknown handle -> `Closed`
- caught panic -> `Panic`

### 5. Good/Base/Bad Cases

- Good: three `BorrowedSlice` values passed directly to `cache_store`.
- Base: an empty optional `domain_set` passed as `{NULL, 0}`.
- Bad: `&GoRequest{keyPtr, responsePtr}` passed to C, because the outer Go
  pointer references memory containing additional Go pointers.

### 6. Tests Required

- Rust ABI tests assert null/length status mapping and no panic escape.
- A real Linux+cgo integration test passes multiple non-empty slices in one
  call; mock-only coverage is insufficient.
- Go race tests cover concurrent operation and close ordering.

### 7. Wrong vs Correct

Wrong: `C.cache_store(handle, (*C.Request)(unsafe.Pointer(&goRequest)))` where
`goRequest` contains pointers into Go byte slices.

Correct: `C.cache_store(handle, keySlice, responseSlice, domainSetSlice, ...)`
with each descriptor passed by value and each backing slice kept alive until
the call returns.

## Scenario: provider matcher snapshot publication

### 1. Scope / Trigger

Any Go provider that owns a reloadable Rust domain/IP matcher and is called from
a DNS match hot path uses this publication contract. It prevents a large Rust
build or file write from turning the provider state lock into global hot-path
serialization.

### 2. Signatures

- Update entrypoints: provider `POST`/`flush`/`save` and `Close`.
- State: Go matcher snapshot plus optional Rust integer handle.
- Rust candidate: `BuildRustDomainMatcher(rules)` or
  `BuildRustIPMatcher(prefixes)`, returning `(matcher, error)`.

### 3. Contracts

- A provider-local `updateMu` serializes complete updates, persistence, and
  close; it is not used by `Match`.
- Parse and copy request-owned rules/prefixes, build the Rust candidate, and
  write configured files while the state `RWMutex` is unlocked.
- Acquire the state write lock only long enough to exchange every member of one
  generation: Go list/mix, published Go snapshot, and Rust handle.
- Readers hold the state read lock through the Rust match call. Close the old
  handle only after releasing the write lock, so prior readers have exited.
- Build or persistence failure publishes nothing and leaves the complete old
  generation active. A match error may disable only the handle that returned
  the error; it must not close a concurrently published generation.
- Rust selection is opt-in; the default Go path and matcher order remain
  unchanged.

### 4. Validation & Error Matrix

- Rust build error -> update error, old Go/Rust generation remains active.
- File write error -> update error, candidate is closed, old generation remains
  active.
- Concurrent update -> updates are serialized and each response publishes one
  whole generation; no cross-generation Go/Rust combination is observable.
- Match during candidate build/write -> old snapshot can complete without
  waiting for the off-path work.
- Close during/after update -> update serialization and handle identity checks
  prevent closing another generation; repeated close is harmless.

### 5. Good/Base/Bad Cases

- Good: build `tmpList`/`tmpMix` and Rust handle off-path, then swap all fields
  under one short state lock and close the old handle after unlock.
- Base: a disabled backend returns no handle and the established Go matcher is
  still published.
- Bad: hold the state mutex during Rust compilation or disk I/O, or publish Go
  state before the Rust candidate is ready.

### 6. Tests Required

- A controllably blocked builder proves an old `Match` completes during the
  candidate build and that the new generation is published afterward.
- Build/persistence failure tests assert both Go and Rust old-generation
  behavior and no partial file/state publication.
- Race tests cover concurrent match, update, and repeated close.
- Linux+cgo tests directly assert Rust positive and negative results, while
  separate whole-matcher tests retain fallback and consumer semantics.

### 7. Wrong vs Correct

Wrong: acquire the provider state lock, compile a large Rust matcher, write the
rule file, then publish fields one at a time.

Correct: serialize the update with `updateMu`, build and persist immutable
request-local candidates without the state lock, exchange the complete
generation in one short critical section, then retire the exact old handle.

## Scenario: matcher Phase 2 Linux gate and evidence harness

### 1. Scope / Trigger

The provider/mapper expansion crosses Go packages, the Rust static library,
cgo build tags, CI, benchmark output, and an isolated process smoke. Keep the
same opt-in and rollback contract at every boundary.

### 2. Signatures

- `scripts/benchmark-rust-matchers.sh` accepts `BENCHTIME` and `COUNT`, and
  runs tagged Linux+cgo domain, IP, and valued-mapper benchmarks.
- `scripts/smoke-rust-matcher-mos-test.sh` requires executable
  `MOSDNS_RUST_BINARY` and optionally accepts `MOSDNS_GO_ONLY_BINARY`.
- Rust selection remains `MOSDNS_MATCHER_BACKEND=rust`; an unset/other value
  selects the Go path.

### 3. Contracts

- Default Go CI runs without Rust tags, a Rust toolchain, cgo, or the runtime
  environment variable.
- Linux matcher gates use `CGO_ENABLED=1`, `-tags mosdns_rust`, and the same
  provider/mapper package list for normal and race tests.
- Benchmarks report fixture bytes/rules or prefixes, build and lookup timing,
  Go-observed allocations, logical valued-result bytes, and cgo calls/op.
- Smoke uses temporary files, random high loopback ports, public audit output,
  and process cleanup; it never uses port 53 or an installed service.

### 4. Validation & Error Matrix

- Missing/non-executable binary -> smoke exits before starting a process.
- Rust static library or cgo build failure -> Linux gate fails; default Go job
  remains independently runnable.
- Rust-disabled/no-cgo binary with `MOSDNS_MATCHER_BACKEND=rust` -> provider
  keeps the Go generation and smoke must observe identical answers.
- Malformed reload -> HTTP 400 and previous valid generation remains active.
- Leaked process/temp directory or port-53 reference -> smoke fails cleanup or
  configuration checks.

### 5. Good/Base/Bad Cases

- Good: build the Rust artifact and a separate no-cgo Go-only fallback, then
  run the same temporary config through both binaries and compare answers plus
  mapper source metadata.
- Base: macOS runs default/stub and pure Rust gates; Linux+cgo evidence comes
  from CI or isolated `mos-test`.
- Bad: run the smoke against the installed service, bind port 53, or report
  `B/op` as Rust heap/RSS bytes.

### 6. Tests Required

- `go test ./...`, focused provider/mapper race, `go build ./...`, and
  `go vet ./...` without Rust selection.
- Rust fmt, all-target tests, checked-in header/valued ABI, clippy, and release
  build; tagged Linux+cgo provider/mapper normal and race tests.
- Three-run fixed-fixture benchmark and isolated smoke with reload, overlap,
  audit source metadata, malformed input, fallback, concurrency, and restart.

### 7. Wrong vs Correct

Wrong: run only macOS stub tests and call the cgo/ABI or process smoke gates
passed, or reuse a production config/port for the benchmark.

Correct: run the explicit Linux+cgo commands and temporary-port smoke, record
the transitional cgo/result-size limitations, and keep `MOSDNS_MATCHER_BACKEND`
opt-in with the Go fallback available.
