# Rust Migration

The complete architecture and phase gates are in `docs/ai/rust-rewrite-plan.md`. That document overrides abbreviated notes here.

## Architecture

The final target is a **pure Rust-native MosDNS binary and runtime**, not a permanently hybrid Go/Rust process. The migration still uses a strangler sequence, but the Go shell and C ABI are transitional scaffolding created by the completed cache/matcher/query-foundation phases. Phase 3B and later foundations should compose as Rust crates for the future Rust host and must not add a Go adapter, backend selector, mirror, or fallback unless a separately reviewed requirement proves one is needed.

The module order remains cache -> matchers -> DNS/query execution -> sequence -> transports/servers -> Rust-native host -> retirement of the old hybrid scaffolding.

## Compatibility policy

Compatibility targets the MosDNS **product contract**, not the Go implementation:

- Preserve user-facing YAML/config syntax, plugin/sequence semantics, final DNS behavior, routing/audit outputs, WebUI/API workflows, persistent formats, and other explicitly frozen external behavior.
- Use current Go code/tests as behavior-discovery evidence when the product contract is unclear. Classify each discovered behavior as `preserve` or `intentional Rust deviation` before making it normative for Rust.
- Do not carry Go implementation details forward by default: Go interfaces, `map[uint32]any`, `ChainWalker` recursion, error strings, internal pool/buffer layouts, naming tricks, cgo handles, selectors, Go mirrors/fallback, and same-generation publication are not final-architecture requirements.
- Intentional Rust deviations are allowed for safety, determinism, correctness, or architectural quality when documented, tested, and shown not to break the frozen product contract.

Phase 0 Go baselines and parity fixtures remain valuable discovery evidence; they do not require every Go/Rust internal difference to be zero.

## Reuse policy

- Preferred Rust upstream: KixDNS at audited commit `2da3a2d` (2026-08-12).
- Prefer direct crates such as Moka when they provide the needed capability; otherwise classify KixDNS material as direct dependency, extracted code, adapted design, or rejected.
- Preserve attribution and GPL-3.0 obligations for copied/adapted source. Pin reviewed upstream revisions and audit every update.
- `/Users/tom/github/mosdns-rust-cache` is a compatibility reference for MosDNS bridge, dump, API, tests, and fallback—not a subtree to copy wholesale.

## Existing hybrid foundation constraints

The following constraints describe the already-built Phase 1/2/3A bridge and remain valid while that code exists. They are **not** a template for creating new Phase 3B+ Go/Rust boundaries.

## Cache foundation constraints

- Use a concurrent O(1)-style cache design (Moka/sharding) and `Bytes`/raw-wire techniques; do not carry over the old global `Mutex` or linear L1 scan.
- Reuse audited KixDNS DNS wire, TTL, truncation, and ECS ideas only after golden parity against MosDNS semantics.
- Preserve `mosdns_cache_v2`, show/load/flush behavior, `domain_set`, exclusion, lazy TTL, metrics, and raw response paths.
- Keep ABI calls coarse-grained. Check ABI version/capabilities at startup and contain every panic.
- Default releases remain on the Go backend until correctness, safety, performance, and test-host gates are satisfied.

## Phase gate

Do not begin a later migration module merely because its Rust implementation is available upstream. Each module needs an approved Trellis task, a frozen product-contract/deviation matrix, appropriate correctness/safety/performance evidence, and a rollback path. For Phase 3B+ pure Rust foundations, Go parity is optional discovery evidence rather than a mandatory runtime/fallback requirement.

After a complete Rust-native host exists, run a dedicated hybrid-scaffolding retirement gate before final replacement/release. Remove the early `MOSDNS_*_BACKEND` selectors, Go mirrors/fallback, cgo adapters/FFI handles and bridge-only test/build paths that are no longer needed, but only after equivalent Rust-native product-contract coverage exists.

## Historical transitional scenarios

The scenarios below remain authoritative for the **existing** Phase 1/2/3A hybrid code until its retirement. Do not extend them into Phase 3B sequence-core, Phase 4 transport/server foundations, or the final Rust host unless an explicit future task says otherwise.

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
- A Go parse or persistence failure publishes nothing and leaves the complete
  old generation active; any unpublished Rust candidate is closed. A Rust-only
  build failure after a valid Go candidate and successful persistence publishes
  the new Go-only generation, returns the established success response, and
  retires the old Rust handle only after the replacement is visible. A match
  error may disable only the handle that returned the error; it must not close
  a concurrently published generation.
- Rust selection is opt-in; the default Go path and matcher order remain
  unchanged.

### 4. Validation & Error Matrix

- Rust-only build error with a valid Go candidate and successful persistence ->
  warning, new Go-only generation published, established success response, and
  retired Rust handle closed after publication.
- Go parse or file write error -> existing update error, unpublished candidate
  closed, old complete Go/Rust generation remains active.
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
- Rust-only build-failure tests assert Go-only publication and the established
  success response; Go parse/persistence-failure tests assert both old Go/Rust
  generation retention and no partial file/state publication.
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

## Scenario: query wire foundation parity and fail-safe fallback

### 1. Scope / Trigger

The Phase 3 query foundation parses DNS names and additional records in Rust
before the opt-in adapter compares the result with the Go oracle. These rules
prevent compression-base mistakes and prevent malformed additional data from
being published as a valid snapshot.

### 2. Signatures

- `parse_query(packet: &[u8]) -> Result<(QueryHeader, QuestionInfo), QueryError>`
- `extract_edns_at(packet: &[u8], extra_offset: usize) -> Result<Option<EdnsInfo>, EdnsParseError>`
- Go selection remains `MOSDNS_QUERY_BACKEND=rust`; the adapter exposes a Go
  oracle result plus typed `FallbackError` on Rust failure.

### 3. Contracts

- Query-name walking follows compression pointers with a 255-byte expanded
  wire-name budget; labels remain at most 63 bytes and pointer cycles are
  rejected before a handle is published.
- Additional-record owner pointers are resolved against the complete DNS
  packet and the absolute record offset. The standalone `extract_edns` helper
  is only for a relative standalone buffer and must not parse a real message.
- A non-OPT additional record is accepted only after owner, fixed fields,
  declared RDLENGTH, and body bounds are valid; then Rust returns typed
  `UnsupportedRecord`, and the opt-in adapter returns the same Go-oracle
  result rather than publishing a partial Rust result.
- Rust remains opt-in and default Go-only; the live Go query context is never
  mutated by a failed Rust attempt.

### 4. Validation & Error Matrix

- Expanded name >255 bytes or a compression cycle -> typed malformed query
  error; no snapshot handle or output is published.
- Pointer target outside the full packet -> typed malformed EDNS error; no
  partial EDNS result.
- Malformed non-OPT owner/fixed/body bounds -> malformed error, not
  `UnsupportedRecord` and not successful EDNS absence.
- Well-bounded non-OPT additional RR -> `UnsupportedRecord` in Rust and
  deterministic Go fallback in the adapter.
- Rust parser/ABI/runtime failure or result mismatch -> `FallbackError` and
  the same-generation Go oracle result.

### 5. Good/Base/Bad Cases

- Good: `extract_edns_at(full_packet, absolute_extra_offset)` resolves an
  in-message compressed owner and extracts a valid OPT record.
- Base: a standalone extra-record buffer uses `extract_edns(extra)` with
  relative pointers only.
- Bad: pass a subslice from a real packet to the standalone helper, accept an
  overlong expanded name, or treat an out-of-bounds non-OPT RDATA as EDNS
  absence.

### 6. Tests Required

- Rust tests assert overlong expanded names and multi-pointer cycles are
  rejected, full-packet additional-owner offsets are honored, and malformed
  non-OPT records produce no partial output.
- Query ABI regression tests exercise the compressed additional-owner fixture
  and handle creation failure on malformed wire.
- Go adapter tests assert typed fallback and Go-oracle parity for unsupported
  or malformed Rust input; Linux+cgo tests repeat the real boundary.

### 7. Wrong vs Correct

#### Wrong

```rust
extract_edns(&packet[extra_offset..])
```

for a real packet whose compression pointer targets bytes before the subslice,
or silently returning EDNS-absent for an unchecked non-OPT record.

#### Correct

```rust
extract_edns_at(&packet, extra_offset)
```

validate all declared bounds, return `UnsupportedRecord` only for a bounded
non-OPT record, and let the Go adapter publish the oracle fallback.

## Scenario: pure Rust sequence execution foundation

### 1. Scope / Trigger

Phase 3B sequence work uses `rust/sequence-core` as an isolated pure Rust
library for the future Rust-native host. It is not a Go adapter, runtime
export, backend selector, or production request-path switch. The crate may
reuse typed `mosdns-dns-core` query/response atoms, but it must not depend on
Go, cgo, plugin registries, listeners, upstreams, or network I/O.

### 2. Signatures

- `ExecutionState::new(QueryHeader, QuestionInfo) -> ExecutionState`
- `ProgramSpec::validate(self) -> Result<ValidatedProgram, ProgramError>`
- `execute(&ValidatedProgram, SequenceId, &mut ExecutionState, &mut ExecutionControl) -> Result<ExecutionCompletion, ExecutionError>`
- `Matcher::evaluate(&self, &ExecutionState) -> Result<MatchOutcome, MatcherError>`
- `Executor::execute(&self, &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError>`
- `ExecutionState::inspect_response(&self, &impl ResponseInspector) -> Result<Option<ResponseInspection>, ResponseError>`

### 3. Contracts

- `ExecutionState` is closed and caller-owned: owned query/question data,
  deterministic `BTreeSet<u32>` marks, `u64` fast flags, `None`/`Raw`/
  `Synthesized` response state, and typed optional routing/audit strings.
  There is no `map[uint32]any`, generic values field, Go pointer, or callback
  escape hatch.
- Raw responses retain the complete owned wire. Valid inspection returns only
  the existing `dns-core` TTL observation and does not consume the wire.
  Malformed raw wire returns `MalformedRawResponse` and remains owned as
  `Raw` until explicitly cleared. Synthesized reject RCODEs accept
  `0..=0x0fff`, with the default reject value `REFUSED` (`5`).
- `ProgramSpec` is the only unvalidated input. Validation assigns stable IDs,
  rejects duplicate names/unknown kinds/missing targets/invalid RCODEs, and
  resolves `goto`/`jump`/`try` before execution. Missing and empty executable
  lists are legal no-ops; multi-exec lists become explicit synthetic
  `Inline(SequenceId)` scopes. Synthetic inline sequences are not direct
  symbolic targets.
- Matchers are immutable readers with one typed `StateMutation` channel. The
  dispatcher applies mutation, reverses only the boolean, then applies
  positive typed metadata. Metadata writes `domain_set` once for anonymous
  qname, `switch6`/AAAA, and `switch5`/SOA/PTR/HTTPS; reversed matches never
  claim the positive label.
- The engine uses an explicit continuation/scope stack. `jump` pushes a
  continuation, `goto` replaces the current scope continuation, `return`
  resumes or completes the current scope, and `accept`/`reject` complete only
  the current scope. Inline fall-through/return/accept/reject resume the outer
  next rule; inline `exit` propagates unless a nested `try` catches it.
- One root `ExecutionControl` owns shared fuel and cancellation. Every
  matcher/executable dispatch checks cancellation before fuel; nested `try`
  never resets either. The observable priority is
  `Cancelled > BudgetExceeded > ordinary matcher/executor error > Exit`.
  `try` converts only `Exit` into normal continuation; all other errors
  propagate, and caller-owned state remains observable on every result.

### 4. Validation & Error Matrix

- Duplicate sequence/fixture name -> `ProgramError::Duplicate*Name`; no
  program is exposed to execution.
- Missing `goto`/`jump` sequence or `try`/fixture target -> typed missing-target
  error; no partial state mutation occurs during validation.
- Unknown matcher/executable or reject RCODE above `0x0fff` -> typed program
  error before execution.
- Malformed raw response -> `ResponseError::MalformedRawResponse`; raw bytes
  remain in the state.
- Cancellation at a dispatch boundary -> `ExecutionError::Cancelled`, taking
  priority over exhausted fuel.
- Cyclic `goto`/`jump`/nested `try` with exhausted shared fuel ->
  `ExecutionError::BudgetExceeded`, never recursion overflow or hang.
- Matcher/executor failure -> typed `ExecutionError` with mutations from
  earlier completed dispatches retained; `try` does not swallow it.

### 5. Good/Base/Bad Cases

- Good: validate a multi-exec rule, observe `Inline(SequenceId)`, run a
  `jump`/`return`, and see the outer rule continue in declaration order.
- Base: a no-matcher/no-exec rule is a legal no-op; repeated matcher and
  fixture kinds remain ordered and can reuse one fixture target.
- Bad: parse Go matcher-name strings, copy `ChainWalker` recursion, add a
  generic state map, or create a cgo/ABI/selector/fallback seam in
  `sequence-core`.

### 6. Tests Required

- State tests assert owned query data, sorted marks, closed routing fields,
  exact response replacement/clear transitions, complete-wire retention,
  TTL-only non-consuming inspection, malformed-wire retention, and RCODE
  boundaries.
- Program tests assert no-op/multi-exec normalization, inline target
  isolation, duplicate-name/unknown-kind/target validation, repeated kinds,
  and no state mutation before successful validation.
- Dispatcher tests assert declaration order, false/error short-circuit,
  mutation ordering, reverse semantics, write-once metadata, and qtype labels.
- Control tests assert accept/reject/return/exit, goto/jump continuations,
  inline scope behavior, `try` sequence/fixture targets, and propagation of
  ordinary errors/cancellation/budget exhaustion.
- Safety tests assert cyclic termination, shared nested fuel/cancellation,
  cancellation priority, caller-owned state after every completion/error, and
  no ABI/live-wiring surface. Run the Rust workspace gates and the unchanged
  Go default/race/vet/build plus `CGO_ENABLED=0` gates.

### 7. Wrong vs Correct

#### Wrong

```rust
// A recursive Go-style walker or a generic plugin-value escape hatch.
fn run(next: &mut ChainWalker, values: &mut HashMap<u32, Box<dyn Any>>) { /* ... */ }
```

#### Correct

```rust
let result = execute(&program, entry, &mut state, &mut control);
// State stays borrowed and observable; nested try shares the root control.
```

## Scenario: pure Rust Phase 4 upstream contract boundary

### 1. Scope / Trigger

Phase 4 transport foundations use `rust/upstream-core` as a pure Rust sibling
of `rust/sequence-core`. The future host composes both crates; the transport
crate depends on `mosdns-dns-core` only and must not depend on
`sequence-core`, `mosdns-runtime`, Go, cgo, selectors, or fallback paths.

### 2. Signatures

- `Endpoint::new(SocketAddr, Transport) -> Result<Endpoint, UpstreamError>`
- `ExchangeRequest::new(&[u8]) -> Result<ExchangeRequest<'_>, UpstreamError>`
- `Upstream::prepare_exchange(ExchangeRequest<'_>, ExchangeContext) -> Result<PreparedExchange<'_>, UpstreamError>`
- `ExchangeContext::check_at(Instant, SideEffectState) -> Result<(), UpstreamError>`
- `ExchangeResponse { wire: Vec<u8>, request_id, response_id, transport, truncated }`
- `inspect_response_header(&[u8]) -> Result<ResponseHeader, HeaderError>`

### 3. Contracts

- Request input is borrowed and read-only; `ExchangeRequest` validates through
  `mosdns-dns-core::parse_query` and records the original transaction ID.
- Slice0 prepares exchange state only. It performs no socket, runtime, or
  network operation; later slices own UDP/TCP I/O and use one host-owned
  runtime.
- `ExchangeResponse` owns its complete returned wire. No Go pool or FFI
  release is part of the API.
- A prepared exchange keeps caller cancellation and upstream-owner shutdown
  as separate wakeable tokens. Owner shutdown wins the terminal check and
  returns `Closed(state)`; caller cancellation returns `Cancelled(state)`.
  The transport crate may expose async cancellation futures for later socket
  selection, but it must not create a hidden Tokio runtime.
- `SideEffectState` is closed: `NotSent`, `MaybeSent`, `Sent`. Connect/setup,
  invalid request/endpoint, and outbound frame-too-large are `NotSent`;
  runtime errors retain the last tracked state.
- Cancellation is checked before the absolute deadline, so it wins a tie.
  `Open -> Closing -> Closed` rejects new exchanges after Closing begins and
  repeated close is harmless.
- `dns-core` header inspection reads only the 12-byte header's QR, TXID, and
  TC; complete response/RR/OPT semantics remain in `dns-core` validation.

### 4. Validation & Error Matrix

- Empty/malformed query -> `InvalidRequest`, before exchange preparation.
- TCP query longer than `u16::MAX` -> `FrameTooLarge`, before any send.
- Port zero -> `InvalidEndpoint`; TCP connect/setup -> `Connect` with
  `NotSent`.
- Cancellation at any tracked state -> `Cancelled(state)`; an expired
  uncancelled context -> `DeadlineExceeded(state)`.
- Owner Closing/Closed -> `Closed(state)`; `Runtime(state)` never introduces
  an `Unknown` marker.
- Close completion is guarded to `Closing -> Closed`; a public completion call
  from `Open` returns an explicit no-op result and leaves the owner `Open`.
- Header shorter than 12 bytes -> `HeaderError::TooShort`; QR clear ->
  `HeaderError::NotResponse`.

### 5. Good/Base/Bad Cases

- Good: validate a borrowed query, retain its original ID, then return an
  owned response wire; use the same absolute deadline for every later phase.
- Base: use `inspect_response_header` for TC routing, then pass only a complete
  non-TC wire to full DNS validation.
- Bad: import `sequence-core::CancellationToken`, create a per-upstream Tokio
  runtime, mutate the caller query, add a second RR/OPT parser, or treat a
  post-send error as safely retryable by default.

### 6. Tests Required

- Public contract tests reject empty/malformed/unframeable queries before any
  socket boundary, assert query nonmutation and original ID, and verify owned
  response bytes.
- Error tests assert the closed side-effect enum, Connect=`NotSent`, runtime
  state preservation, distinct cancellation/deadline errors, and cancellation
  precedence at a deadline tie.
- Lifecycle tests assert Open/Closing/Closed, idempotent close, and rejection
  of new exchanges after Closing and Closed.
- `dns-core` tests assert minimum header validity, QR, TXID, TC, and no RR/OPT
  parsing in the helper. Manifest/tree checks assert the sibling dependency
  direction and absence of sequence/runtime/FFI dependencies.

### 7. Wrong vs Correct

#### Wrong

```rust
// Transport imports sequence policy or a transitional runtime to obtain
// cancellation and starts a hidden executor before validating the query.
```

#### Correct

```rust
let request = ExchangeRequest::new(query)?;
let context = ExchangeContext::new(deadline, transport_cancellation);
let prepared = upstream.prepare_exchange(request, context)?;
// Slice0 has performed only pure validation; later slices own socket I/O.
```
