# Rust Migration

The complete architecture and phase gates are in `docs/ai/rust-rewrite-plan.md`. That document overrides abbreviated notes here.

## Architecture

The final target is a **pure Rust-native MosDNS binary and runtime**, not a permanently hybrid Go/Rust process. The migration still uses a strangler sequence, but the Go shell and C ABI are transitional scaffolding created by the completed cache/matcher/query-foundation phases. Phase 3B and later foundations should compose as Rust crates for the future Rust host and must not add a Go adapter, backend selector, mirror, or fallback unless a separately reviewed requirement proves one is needed.

The historical foundation order is cache -> matchers -> DNS/query execution ->
sequence -> transports. The revised roadmap brings Phase 5A minimal native host
and its UDP/TCP listeners forward after the current QUIC reuse task closes,
without waiting for all remaining Phase 4 work. Remaining transports/servers
compose with 5B full query features; 5C completes the control plane, 5D verifies
the full system, and Phase 6 retires hybrid scaffolding. No existing task scope
or implementation authorization is enlarged by that ordering.

Linux amd64 is the primary target. Full functionality/correctness and stability
are prerequisites; prioritize tail latency and sustainable useful throughput.
Memory is secondary and bounded performance-oriented tradeoffs are allowed.
Track library readiness, native integration and product acceptance separately
in `docs/rust/feature-coverage.md`; use
`docs/rust/performance-validation.md` for native performance evidence. Historical
hybrid thresholds and archived evidence are not rewritten.

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
crate uses `mosdns-dns-core` as its only MosDNS crate dependency and must not depend on
`sequence-core`, `mosdns-runtime`, Go, cgo, selectors, or fallback paths.

### 2. Signatures

- `Endpoint::new(SocketAddr, Transport) -> Result<Endpoint, UpstreamError>`
- `ExchangeRequest::new(&[u8]) -> Result<ExchangeRequest<'_>, UpstreamError>`
- `Upstream::prepare_exchange(ExchangeRequest<'_>, ExchangeContext) -> Result<PreparedExchange<'_>, UpstreamError>`
- `Upstream::exchange(ExchangeRequest<'_>, ExchangeContext) -> impl Future<Output = Result<ExchangeResponse, UpstreamError>>`
- `Upstream::close() -> impl Future<Output = CloseResult>`; the owner drains
  registered exchanges before exposing `Closed`
- `ExchangeContext::check_at(Instant, SideEffectState) -> Result<(), UpstreamError>`
- `ExchangeResponse { wire: Vec<u8>, request_id, response_id, transport, truncated }`
- `inspect_response_header(&[u8]) -> Result<ResponseHeader, HeaderError>`

### 3. Contracts

- Request input is borrowed and read-only; `ExchangeRequest` validates through
  `mosdns-dns-core::parse_query` and records the original transaction ID.
- The accepted foundation includes per-exchange UDP, fresh plain TCP and
  `UdpTcpPolicy`. All run on the caller's host-owned runtime; no runtime is
  created by an upstream. This scenario describes the UDP/TCP foundation;
  secure transport extensions have separate contracts and evidence. Read the
  handover/task records for their current status, not this historical scope.
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
  `Open -> Closing -> Closed` rejects new exchanges after Closing begins.
  Slice1 registration is serialized with the close admission transition;
  `close().await` waits for every in-flight guard to drop and repeated close
  calls converge without a hidden runtime or blocking wait.
- Wrong-peer and wrong-ID datagrams remain ignored while waiting. If one was
  observed before a deadline, caller cancellation, owner close, or receive
  failure, the terminal typed error retains minimal structured diagnostic flags
  without relabeling the primary cause. A later valid response succeeds
  without diagnostic state.
- After the expected-peer and response-ID checks, both valid TC observations
  and fully validated non-TC responses pass one synchronous response-commit
  gate under the same lifecycle lock as registration and `Open -> Closing`.
  Commit-before-close wins; close-before-commit returns `Closed(Sent)`.
- `dns-core` header inspection reads only the 12-byte header's QR, TXID, and
  TC; complete response/RR/OPT semantics remain in `dns-core` validation.
- Slice1 UDP binds one fresh ephemeral socket per exchange, sends the borrowed
  query exactly once, validates the configured peer and response ID, ignores
  wrong-peer/wrong-ID datagrams, and returns an owned response. It uses the
  full legal datagram capacity rather than the Go 4095-byte buffer; no shared
  demux, retransmission, generic retry, pool, reuse, pipeline or production
  wiring is part of this primitive.
- TCP opens one stream per exchange; `tcp::write_frame` / `read_frame` own
  exact two-byte framing and partial I/O. It validates QR, original ID and the
  complete DNS response, including TCP TC responses, before final commit.
- `UdpTcpPolicy::new(Endpoint)` expects a UDP endpoint and uses the same
  numeric address/port for TCP. `exchange(ExchangeRequest, ExchangeContext)`
  permits exactly one TCP attempt after a valid UDP TC observation, with the
  same borrowed query, original ID and absolute deadline. UDP errors never
  trigger fallback. Its `close().await` drains both owners.
- A TCP-leg error retains `UpstreamError::TcpFallback { prior, cause }`;
  prior records the UDP TC observation and overall Sent state. Pre-fallback
  cancellation/deadline remains a direct typed control error with Sent state.
- TCP final response commit checks owner close, caller cancellation, original
  deadline, then success under the lifecycle lock. These checks also apply to
  the final TCP leg of the composite; do not reset the deadline on fallback.

### 4. Validation & Error Matrix

- Empty/malformed query -> `InvalidRequest`, before exchange preparation.
- TCP query longer than `u16::MAX` -> `FrameTooLarge`, before any send.
- Port zero -> `InvalidEndpoint`; UDP local bind/setup and TCP connect/setup ->
  `Connect` with `NotSent`.
- Cancellation at any tracked state -> `Cancelled(state)`; an expired
  uncancelled context -> `DeadlineExceeded(state)`.
- Owner Closing/Closed -> `Closed(state)`; `Runtime(state)` never introduces
  an `Unknown` marker.
- Close completion is guarded to `Closing -> Closed`; a public completion call
  from `Open` returns an explicit no-op result and leaves the owner `Open`.
- Header shorter than 12 bytes -> `HeaderError::TooShort`; QR clear ->
  `HeaderError::NotResponse`.
- TCP zero prefix/EOF/partial frame/wrong ID/malformed DNS -> typed terminal
  error; never reuse or retry the stream. Partial writes are conservatively
  MaybeSent; complete write followed by read failure is Sent.
- A valid non-TC UDP response returns with zero TCP connections; valid TC
  permits at most one; malformed UDP permits zero.

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
- `slice2_tcp.rs` verifies exact framing, partial I/O, EOF, size, ID,
  cancellation and fresh-stream isolation. `slice3_policy.rs` verifies the
  TCP connection count, unchanged query/deadline, prior TC errors, cancellation
  between legs and deterministic close/drain.
- Race tests use explicit server/client handshakes and bounded waits, not
  equal sleeps as evidence of ordering. Foundation acceptance is recorded in
  `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/implement.md`.

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

## Scenario: Phase 4 DoQ post-open cancellation and response-FIN contract

### 1. Scope / Trigger

Use this contract for a fresh one-shot DoQ exchange after `open_bi` succeeds.
It captures the cancellation lifecycle and the missing-response-FIN boundary
because Quinn 0.11.7 queues `STOP_SENDING` but exposes no awaitable flush or
peer-observation future.

### 2. Signatures

- `DoqUpstream::exchange(...) -> Future<Output = Result<SecureResponse, SecureError>>`
  owns one `RecvStream` after `open_bi` and returns the existing typed error
  vocabulary with its original `SideEffectState`.
- `PreparedDoq::settle_after_open(&mut RecvStream, Result<T, SecureError>)`
  is the single post-open local-control settlement path for request write,
  request FIN, and response read.
- `RecvStream::stop(VarInt::from_u32(DOQ_REQUEST_CANCELLED))` uses
  `DOQ_REQUEST_CANCELLED = 0x3`; its `ClosedStream` result is not allowed to
  replace the original local-control error.
- `classify_read_error(ReadToEndError)` maps both
  `ReadError::Reset(_)` and `ReadError::ConnectionLost(_)` without a normal
  response STREAM FIN to `DoqProtocolMissingResponseFin`.

### 3. Contracts

- After `open_bi`, owner close, caller cancellation, or the shared deadline
  that wins any write/FIN/read phase must call `RecvStream::stop(0x3)` and
  immediately return the original typed control error; it must not bypass the
  final response commit gate.
- Production code must not claim that `STOP_SENDING` was flushed or observed by
  the peer. Do not substitute `yield_now`, sleep, polling, a short timeout, or
  `connection.close`/`endpoint.close` plus `wait_idle` as a transport barrier.
- A debug-only `DoqStopPause` seam may park an integration-test exchange after
  the production `stop` call while the real Quinn driver runs. It is an
  observation seam, not a production flush mechanism, and must be absent from
  release builds.
- A response is committed only after a normal response STREAM FIN. Reset or
  connection loss before that FIN is terminal `DoqProtocolMissingResponseFin`
  with `Sent`; no retry, fallback, or commit is permitted. Connection close
  after a successfully committed response is ordinary one-shot teardown, not
  cancellation synchronization.

### 4. Validation & Error Matrix

- local control before `open_bi` -> existing connect/control error,
  `NotSent`, and no `RecvStream::stop` call;
- local control after `open_bi` -> one best-effort `stop(0x3)`, unchanged typed
  control error and side-effect state;
- `stop` returns `ClosedStream` -> ignore that transport result and preserve the
  original typed control error;
- response reset or connection loss before normal FIN ->
  `DoqProtocolMissingResponseFin`, `Sent`, no commit and no retry;
- release build -> no pause installer or pause path; tests must not depend on
  the debug-only seam;
- test observation -> use explicit readiness/arrival/release handshakes and
  bounded waits, never equal sleeps or scheduler-yield ordering.

### 5. Good/Base/Bad Cases

- Good: `settle_after_open` is called for all three phases, calls `stop(0x3)`
  on local control, and returns the original error while an integration test
  observes `SendStream::stopped()` through the debug seam.
- Base: a normal response FIN reaches the commit gate, then one-shot teardown
  closes the connection and endpoint.
- Bad: close the endpoint immediately after `stop` and call `wait_idle` to
  imply that the peer saw `STOP_SENDING`, or classify `ConnectionLost` as a
  generic receive error and commit bytes without FIN.

### 6. Tests Required

- Keep separate response-wait and write-phase cancellation tests; assert peer
  stop code `0x3`, original typed errors, `in_flight == 0`, and one accepted
  connection.
- Add deterministic complete-response and partial-response connection-loss
  fixtures; assert missing-FIN, `Sent`, no commit, request FIN, and restored
  wire-ID behavior. Retain the stream-reset fixture.
- Run focused Slice 1 tests, repeated cancellation stress, debug/release seam
  checks, upstream-core all-target tests, workspace tests, fmt, clippy, and
  `git diff --check`.

### 7. Wrong vs Correct

#### Wrong

```rust
recv.stop(REQUEST_CANCELLED);
connection.close(0.into(), b"");
endpoint.close(0.into(), b"");
endpoint.wait_idle().await; // not a STOP_SENDING flush or peer ACK
```

#### Correct

```rust
let result = race_control(...).await;
if is_local_control_error(&result_error) {
    let _ = recv.stop(DOQ_REQUEST_CANCELLED.into());
}
result // preserve the original typed error and side-effect state
```

## Scenario: pure Rust Phase 4 DoH3 one-shot foundation

### 1. Scope / Trigger

Use this contract for the Slice 2 DoH3 primitive in `rust/upstream-core`. It
adds one fresh, authenticated HTTP/3 exchange on the caller-owned runtime while
preserving the existing DoH response contract and lifecycle vocabulary. It does
not add pooling, retry/fallback, host wiring, resolver composition, 0-RTT,
resumption, migration, or the Slice 3 QUIC/H3 stream-error matrix.

### 2. Signatures

- `Doh3Upstream::new(endpoint: DohEndpoint, tls: TlsPolicy) ->
  Result<Doh3Upstream, SecureError>` validates an exact `h3` ALPN policy before
  any socket work.
- `Doh3Upstream::exchange(request: ExchangeRequest<'_>, context:
  ExchangeContext) -> Result<SecureResponse, SecureError>` performs one fresh
  connection and one GET; the result is `SecureTransport::Doh3` with
  `SecureHttpVersion::Http3`.
- The H3 connection driver is registered through the existing
  `H2ScopeLease::spawn` child registry before request bytes are sent; the
  response candidate is committed only after `H2ScopeLease::finish()` drains
  that child.

### 3. Contracts

- QUIC dials `DohEndpoint::dial()` numerically, authenticates
  `DohEndpoint::identity()`, and offers exactly ALPN `h3`. The service
  authority and request target remain `DohEndpoint::authority()` and
  `DohEndpoint::get_request_target(request)`; the dial address must never leak
  into `:authority` or `:path`.
- Each exchange sends exactly one `GET` with `Accept:
  application/dns-message`, no body, `User-Agent`, or request
  `Content-Encoding`, then sends request-side FIN. There is no HTTP/1.1,
  HTTP/2, retry, fallback, or pooled reuse path.
- Response validation is shared with H1/H2: status `200`, parsed head at most
  16 KiB, at most 64 headers, `Content-Length` at most 65535 and exact when
  present, absent/`identity` content encoding, and a case-insensitive
  `application/dns-message` media type with optional parameters. The complete
  body is bounded at 65535 bytes, is DNS-validated, and restores only the
  caller's original ID.
- The H3 driver is owned work: no detached task or hidden runtime. Teardown
  seals admission, aborts tracked children, and drains them before the commit
  gate. One-shot connection/endpoint close is success-path teardown after
  commit, never a cancellation or `STOP_SENDING` flush substitute.

### 4. Validation & Error Matrix

| Condition | Required result |
|---|---|
| invalid request target or closed owner before I/O | typed `NotSent`; no QUIC connection |
| connect, TLS, ALPN, or H3 control-stream setup failure | typed connect/TLS failure with `NotSent`; no fallback |
| non-200, wrong/missing media type, or non-identity encoding | typed DoH protocol error; no commit |
| more than 64 headers, head over the H3 advertised bound, or declared body over 65535 | typed response-head/body-size error; no commit |
| early body end, length mismatch, invalid DNS wire, or response ID mismatch | typed incomplete/malformed response; no commit |
| owner close, caller cancellation, deadline, or dropped exchange before commit | existing typed control/transport error wins; child and in-flight registration drain to zero |

### 5. Good/Base/Bad Cases

- Good: build an absolute HTTPS URI only to derive the correct H3 pseudo-headers,
  reuse the existing DoH target encoder, register the driver before the GET,
  drain before `commit_final_response`, and return `Doh3`/`Http3` metadata.
- Base: use a loopback H3/TLS fixture with an ephemeral port, exact `h3` ALPN,
  one request per connection, and an independent TCP probe to prove no fallback.
- Bad: spawn an untracked H3 driver, commit before it drains, invent a second
  DNS GET encoder, use the numeric dial as authority, or close/wait-idle as a
  cancellation flush barrier.

### 6. Tests Required

- Focused loopback tests assert authority/path/header bytes, request FIN, ID
  zeroing/restoration, one request and one fresh connection per exchange, and
  `Doh3`/`Http3` metadata.
- Negative tests assert typed status/media/encoding/head/body errors, early EOF
  and length mismatch, ALPN/TLS/identity failures as `NotSent`, and no TCP or
  protocol fallback.
- Ownership tests assert owner close, caller cancellation, dropped futures, and
  exchange-after-close leave `in_flight == 0`, no second connection, and no
  late committed response. Slice 3 owns deadline precedence and explicit H3
  stream-error-code mappings.
- Run the focused target in debug and release, upstream-core all-target tests,
  workspace clippy/test gates, fmt, `task.py validate`, and `git diff --check`.

### 7. Wrong vs Correct

#### Wrong

```rust
let (driver, mut sender) = builder.build(quic).await?;
tokio::spawn(driver); // not owned by the exchange scope
let response = read_and_validate(&mut sender).await?;
commit_final_response(response); // driver may still be alive
```

#### Correct

```rust
let scope = H2ScopeLease::new(liveness, owner_cancel, caller_cancel);
scope.spawn(drive_h3_connection(driver)); // before request bytes
let candidate = run_one_get(...).await?;
scope.finish().await; // seal, abort, and drain the tracked driver
commit_final_response(...)?;
```

## Scenario: pure Rust Phase 4 endpoint-resolution foundation

### 1. Scope / Trigger

Use this contract for the single-family endpoint-resolution foundation in
`rust/upstream-core`. It resolves a configured hostname through a numeric
bootstrap peer and hands a numeric destination to the existing transport
constructors. It is a pure Rust sibling and must not add host/config/plugin
wiring, Go/cgo boundaries, backend selectors, fallback paths, or production
service changes.

Native dual-stack resolution, address ordering/racing, per-family failure
memory, QUIC/HTTP3 interaction, and the follow-up task that owns them are out
of scope for this foundation.

### 2. Signatures

- `ResolutionTarget::new(host, port, family)` validates and normalizes a
  hostname or validates a numeric literal; `numeric_address()` exposes the
  already-usable dial address.
- `BootstrapEndpoint::new(host, port)` accepts only a numeric bootstrap peer.
- `ConfigVersion::from_u8(0 | 4 | 6)` maps to the reviewed single family;
  `0` and `4` select IPv4/A, while `6` selects IPv6/AAAA.
- `BootstrapResolver::new(target, bootstrap, policy, clock)` uses the
  unpredictable OS ID source for hostname resolution and returns a typed error
  when that source is unavailable.
- `BootstrapResolver::resolve(context)` preserves the caller's absolute
  deadline and returns an owned `PublishedTarget`; `ResolverComposition` hands
  its numeric address to UDP/TCP, DoT, or DoH constructors without changing
  secure service identity.
- `ResolverState` exposes `published`, `last_expired`, and `last_error` for
  diagnostics; publication and refresh mutations remain crate-private and are
  owned by `BootstrapResolver`.

### 3. Contracts

- A numeric target bypasses DNS, does not open a bootstrap socket, and does not
  probe or draw from the OS entropy source. Numeric construction therefore
  remains usable even when an injected or host ID source would fail.
- A hostname target fails closed when unpredictable transaction IDs are not
  available; it never substitutes a predictable ID. Deterministic ID sources
  are test seams only.
- The bootstrap peer's address family and the target's answer family are
  independent. An IPv4 bootstrap may answer AAAA and an IPv6 bootstrap may
  answer A; do not add a peer/answer-family equality invariant.
- Bootstrap UDP uses one caller-owned runtime, one connected ephemeral socket,
  one absolute deadline, bounded retransmission, and no system resolver,
  hidden runtime, detached task, TCP fallback, or deadline extension.
- Correlation and DNS wire failures remain typed. In particular,
  `ResolverWireError::Truncated` maps to `ResolverError::Truncated` and is a
  terminal result for this foundation; it must not be relabeled as malformed
  or trigger TCP bootstrap fallback.
- Publication is complete-or-nothing. A failed refresh preserves the prior
  published value, an expired value is never served as fresh, and lifecycle
  close/cancellation can prevent a late publication.
- Public state handles are observation-only. External callers cannot publish,
  manufacture freshness, or clear refresh diagnostics; state-model tests that
  need mutation live inside the resolver crate.
- Resolution composes only after a numeric destination exists. UDP/TCP use the
  numeric dial address, while DoT SNI and DoH URL authority/path remain the
  configured service identity. No YAML/API/WebUI or live sequence ownership is
  introduced here.

### 4. Validation & Error Matrix

| Condition | Required result |
|---|---|
| numeric target with unavailable entropy | immediate numeric publication; no RNG/DNS dependency |
| hostname with unavailable entropy | `UnpredictableIdsUnavailable`; no predictable fallback |
| bootstrap hostname or zero port | typed validation error before I/O |
| unsupported `bootstrap_version` | `UnsupportedConfigVersion`; no silent default |
| correlated TC response | `ResolverError::Truncated`; terminal, no TCP attempt |
| malformed/wrong-ID/wrong-question response | typed terminal/mismatch handling; no publication |
| failed refresh with prior value | prior value retained; failure recorded diagnostically |
| owner close or caller cancellation | typed control error; no late success/publication |

### 5. Good/Base/Bad Cases

- Good: construct numeric targets without touching `OsIdSource`, and let only
  the owner mutate its read-only shared state after the lifecycle commit gate.
- Base: use deterministic IDs and an injected clock in tests, while production
  uses the OS source and caller-owned time/runtime controls.
- Bad: probe entropy unconditionally in `BootstrapResolver::new`, collapse TC
  into a generic malformed error, expose public state mutation, call the system
  resolver, or add a TCP fallback to the bootstrap leg.

### 6. Tests Required

- Public tests cover numeric bypass with a failing ID source, hostname
  fail-closed behavior, typed TC handling, and the absence of external state
  mutation.
- Resolver state-model tests remain crate-internal and cover fresh/expired
  publication and refresh-failure preservation.
- Loopback tests use ephemeral high ports, explicit handshakes, bounded waits,
  injected clocks/IDs, and the existing upstream lifecycle vocabulary; they do
  not modify an installed service or use port 53.
- The focused Linux/MSRV resolver gate runs on the isolated Debian test VM via
  the repository SSH alias. Mac-host Docker/Colima is not a substitute for
  this evidence. Record any pre-existing Clippy findings separately and do not
  claim an unrun full-workspace gate.

### 7. Wrong vs Correct

#### Wrong

```rust
// Every constructor probes entropy, TC becomes a generic parse error, and an
// Arc<ResolverState> lets integration callers publish arbitrary destinations.
```

#### Correct

```rust
if target.is_numeric() {
    // No ID draw or bootstrap socket: publish the validated numeric address.
} else if !OsIdSource.is_available() {
    return Err(ResolverError::UnpredictableIdsUnavailable);
}
// Only the owner commits state; a correlated TC reply is terminal.
```

## Scenario: Phase 4 resolver composition into a DoQ endpoint

### 1. Scope / Trigger

Use this contract when a completed `PublishedTarget` is handed to the one-shot
DoQ transport. The composition boundary is read-only: it consumes the
published numeric destination and a caller-owned service identity, without
adding resolver refresh, host/config wiring, pooling, fallback, or listeners.

### 2. Signatures

- `ResolverComposition::doq_endpoint(
  published: &PublishedTarget,
  identity: &ServerIdentity,
) -> Result<DoqEndpoint, ResolverError>`
- `DoqEndpoint::new(dial: SocketAddr, identity: ServerIdentity) ->
  Result<DoqEndpoint, SecureError>` remains the construction and zero-port
  validation boundary.

### 3. Contracts

- `doq_endpoint` passes exactly `PublishedTarget::dial()` as the numeric QUIC
  destination and clones the caller's `ServerIdentity` unchanged.
- Resolver output never rewrites SNI/certificate identity to the selected IPv4
  or IPv6 address, and the hostname never enters the socket dial path.
- A/AAAA selection is already complete before composition; the helper performs
  no DNS lookup, bootstrap I/O, address racing, retry, fallback, or mutation of
  resolver state.
- Existing `endpoint`, `dot_endpoint`, and `doh_endpoint` contracts remain
  unchanged; a future DoH3 consumer continues to reuse `DohEndpoint`.

### 4. Validation & Error Matrix

| Condition | Required result |
|---|---|
| fresh A or AAAA `PublishedTarget` | `DoqEndpoint` dials its numeric address and retains the supplied identity |
| numeric literal publication | same read-only composition; no bootstrap traffic |
| zero published port | `ResolverError::ZeroPort`; no endpoint is returned |
| identity differs from dial address | identity remains the caller's validated name; no numeric substitution |
| resolver refresh/selection not complete | composition is not attempted; caller must provide a published target |

### 5. Good/Base/Bad Cases

- Good: `DoqEndpoint::new(published.dial(), identity.clone())` and a typed
  `SecureError` to `ResolverError` mapping at the resolver boundary.
- Base: deterministic A and AAAA loopback fixtures prove the selected family
  reaches `dial()` while the DNS name remains unchanged.
- Bad: resolve the service name again during construction, put the numeric
  address in SNI, mutate `PublishedTarget`, or add connection policy to this
  read-only helper.

### 6. Tests Required

- Public dual-stack tests cover an A-preferred selection and an AAAA-only
  selection, asserting numeric dial address, preserved identity, and the
  absence of numeric identity substitution.
- Existing endpoint and resolver tests remain green, including zero-port
  validation and numeric-target DNS bypass.
- Focused Linux/MSRV evidence uses the isolated Debian VM and an ephemeral
  loopback fixture; it must not mutate installed services or bind port 53.

### 7. Wrong vs Correct

#### Wrong

```rust
// Re-resolves the identity or authenticates the selected address itself.
let endpoint = DoqEndpoint::new(resolve(identity.dns_name())?, identity);
```

#### Correct

```rust
let endpoint = DoqEndpoint::new(published.dial(), identity.clone())
    .map_err(|_| ResolverError::ZeroPort)?;
```

## Scenario: Phase 5A native query observation hot path

### 1. Scope / Trigger

Use this contract when native-host request execution feeds the Phase 5A
observer. The sole supported listener's audit flag controls detailed record
capture; basic metrics remain active in either mode.

### 2. Signatures

- `ExecutionCheckpoint::new(capture_audit_details: bool)` carries the
  admission-time capture decision into request execution.
- `QueryObserver::metrics_snapshot()` and `audit_snapshot()` return read-only
  host-owned snapshots.

### 3. Contracts

- With detailed capture disabled, do not materialize per-query sequence,
  response-source identity, or failure-provenance strings. Keep response
  code, cache disposition, lifecycle, and configured upstream attempt identity
  plus outcome available to basic metrics.
- Upstream-attempt identity is required for bounded per-configured-upstream
  metric aggregation; question names, client addresses, and trace identifiers
  never become metric labels.
- With capture enabled, preserve the same executed route, ordered attempts,
  final response source, sequence, and failure provenance as the execution
  result; the observer flag must not change DNS wire or forwarding behavior.
- W1/W2 keep the zero-or-one attempt list inline. On a second W3 leg, promote
  it to a vector with capacity hinted from the configured forward count.
- Move the in-flight upstream identity into the terminal attempt after the
  exchange; do not resolve and allocate the same identity again. Keep it in
  the cancellation checkpoint while the exchange is pending.

### 4. Validation & Error Matrix

| Condition | Required result |
|---|---|
| audit disabled, successful upstream response | no audit record or audit-only route strings; response/cache/lifecycle and upstream metrics remain correct |
| audit enabled | existing route, cache, ordered-attempt, and provenance fields remain intact |
| W2 cache hit | cache-hit metric, no upstream attempt |
| W3 forwarding | each configured leg contributes its actual ordered attempt outcome; a second leg promotes inline storage with bounded reserved capacity |

### 5. Good/Base/Bad Cases

- Good: branch at the execution fact-construction boundary using the
  checkpoint's captured audit flag, while recording metric-required attempts.
- Base: assert disabled-audit metrics and empty retention through a real
  observer admission/finish guard; retain enabled-audit W1/W2/W3 coverage.
- Bad: build route/provenance strings on every request and discard them only
  after the observer notices that audit capture is disabled.

### 6. Tests Required

- Disabled-audit execution must retain correct lifecycle, response-code,
  cache, and configured-upstream attempt metrics while retaining no audit row.
- The attempt list keeps its first entry inline and promotes on a second entry
  without changing attempt order or outcomes.
- Enabled-audit execution and cancellation tests must preserve the detailed
  route and failure fields.
- Frozen Linux evidence must confirm that the candidate reduces the intended
  audit-off overhead without changing correctness or the predeclared guards.

### 7. Wrong vs Correct

Wrong: create an upstream `ResponseSource` and final sequence for every query,
then rely on the observer to drop them when detailed audit is disabled.

Correct: carry the admission-time capture bit into execution and only build
audit-only strings when it is set; always retain the bounded attempt facts
needed by metrics.
