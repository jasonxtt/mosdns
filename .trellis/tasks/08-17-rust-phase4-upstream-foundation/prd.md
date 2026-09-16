# Rust Phase 4 upstream transport foundation

> **Planning hold (revised 2026-08-18):** do not run `task.py start` or write
> implementation code from this task yet. Phase 3B must be implemented,
> root-reviewed, and archived first. This PRD has been realigned with the final
> goal: a pure Rust-native MosDNS host, not a permanent Go/Rust hybrid runtime.

## Goal

After the Phase 3B gate, establish a bounded **pure Rust** foundation for
DNS-over-UDP and DNS-over-TCP upstream exchange that can be consumed directly
by the future Rust sequence/host. No Go transport adapter, C ABI, backend
selector, Go pool-buffer ownership, or Go fallback is part of this phase.

The current Go upstream is behavior-discovery evidence only. Phase4 preserves
reviewed MosDNS product/protocol contracts and is free to replace Go internal
algorithms, allocation models, pool layouts, error strings, retry machinery,
and other implementation details.

## Confirmed facts

- Phase 3A query/wire foundation is archived and provides reusable pure Rust
  DNS/query types; its query C ABI is transitional and is not a template for
  Phase4.
- Phase 3B is the prerequisite Rust-native sequence/execution foundation and
  must complete before this task starts.
- The current live path remains Go-owned only because the Rust-native host does
  not exist yet. The `rust` branch is not intended for actual use before the
  full replacement is complete.
- The final migration order remains UDP/TCP upstream -> TLS/HTTPS ->
  QUIC/HTTP3 -> server listeners -> Rust-native host -> hybrid-scaffolding
  retirement.
- Current Go upstream code and tests cover response ID handling, UDP/TCP
  exchange, truncation fallback, cancellation/deadline, concurrency, idle
  close, framing, and connection recovery. These are evidence sources, not an
  instruction to copy the Go transport implementation.

## Compatibility policy

Every discovered Go behavior must be classified before Rust networking code is
written:

### Preserve — product/protocol contract

- caller query bytes are not mutated by exchange;
- the response corresponds to the request and exposes the original DNS query
  ID to the caller;
- UDP `TC=1` can fall back to TCP for the same upstream when that behavior is
  part of the configured MosDNS upstream semantics;
- TCP uses correct two-byte DNS length framing;
- cancellation/deadline terminates the Rust request and releases resources;
- `Close` is deterministic and prevents new work while allowing defined
  in-flight cleanup;
- concurrent queries cannot receive each other's responses;
- future configuration fields that control upstream behavior keep their
  user-visible meaning.

### Intentional Rust design / deviation

- use Rust-native owned/borrowed byte types (`Bytes`, `Vec<u8>`, or equivalent)
  rather than Go `*[]byte`/`pkg/pool` ownership;
- use a Rust async/runtime model with explicit cancellation rather than a
  blocking cgo request abandoned by Go;
- malformed/undersized DNS packets use one documented Rust validation policy
  rather than inheriting inconsistent UDP/TCP Go edge behavior;
- internal retry/backoff/concurrency limits may be redesigned when they are not
  exposed configuration contracts, provided timeout/cancellation and protocol
  behavior remain correct and measured.

### Implementation-only — do not port merely for parity

- Go `pkg/pool.GetBuf` / `pool.ReleaseBuf` lifecycle;
- exact internal QID-remap data structures;
- the current 4095-byte UDP receive buffer;
- the exact one-second retransmit implementation and ten-second internal
  ceiling unless research proves they are part of an externally configured
  contract;
- the current fixed pipeline limit of 64 when it is only an implementation
  constant;
- Go transport interfaces, error strings/types, connection structs, goroutine
  layout, and bridge/fallback behavior.

If source/docs review cannot determine whether a behavior is externally relied
upon, add a targeted characterization/research entry and classify it explicitly
before implementation.

## Required design decisions before implementation

### Rust async runtime ownership

- Select one shared Rust async runtime model for transport and later server
  work; do not create a second independent runtime per upstream or protocol.
- Define task spawning, timer/deadline ownership, cancellation, shutdown, and
  connection-pool lifecycle entirely in Rust.
- Do not add transport capability bits, C handles, or runtime FFI records merely
  for Go integration.

### Request cancellation and side-effect safety

- Define a Rust request state machine that can cancel timers/socket work and
  release all resources deterministically.
- Distinguish pre-send validation/construction errors from post-send network
  failures for retry decisions.
- There is no Go fallback. After a packet has been sent, any retry must be an
  explicitly reviewed Rust transport/protocol retry (for example UDP
  retransmission or TC -> TCP), never a hidden second execution through Go.

### Protocol/socket scope

The first implementation subset should remain narrow:

- direct numeric-IP UDP;
- plain TCP;
- DNS response validation/demultiplexing;
- cancellation/deadline;
- connection reuse needed for the preserved behavior;
- UDP truncation -> TCP fallback when configured by the preserved contract.

`SoMark`, `BindToDevice`, SOCKS5, hostname/bootstrap resolution, TLS/HTTPS,
QUIC/HTTP3 and server listeners must each be explicitly classified as later
work or required product contract before coding. Do not silently absorb them.

### Ownership

- Rust transport owns its sockets, connection pool/runtime state and any bytes
  it retains beyond a call boundary.
- Query input ownership/borrowing and returned response ownership must be
  explicit in Rust types.
- No Go pointer, `pkg/pool` buffer, cgo handle or Go callback appears in the
  transport API.

### KixDNS research

Add a research ledger for pinned KixDNS commit
`2da3a2d59466e996a0f846c3e7e504970b878b06` covering async runtime, UDP
multiplexing, TCP pooling, cancellation, recovery, timeout/retry policy and
socket abstraction. Classify each reusable element as direct dependency,
adapted design, extracted code, or rejected, and compare it against the frozen
MosDNS product contract rather than Go internals.

## Initial Rust-native boundary

The intended dependency shape is conceptually:

```text
rust/dns-core
     |
rust/sequence-core
     |
rust/upstream-core  -> shared Rust async runtime
     |
future Rust server/host
```

`upstream-core` exposes a pure Rust request/response/cancellation API. It does
not export C symbols and does not integrate with `pkg/upstream.NewUpstream`.
The future Rust host will construct/configure these transports directly after
configuration ownership moves to Rust.

## Requirements

### R1 — Freeze product/protocol behavior, not Go internals

Create a reviewed compatibility/deviation matrix before networking code. Go
source/tests are discovery evidence. Rust tests are normative only after each
behavior is classified as preserve, intentional deviation, or
implementation-only.

### R2 — Preserve UDP/TCP externally meaningful semantics

Rust tests must cover query non-mutation, original response ID, concurrent
response isolation, UDP TC -> TCP fallback, correct TCP framing,
cancellation/deadline, connection recovery/reuse where required, and
predictable close/shutdown.

### R3 — Pure Rust ownership and cancellation

The transport API and runtime lifecycle are Rust-native. No cgo, C header,
Go pool ownership, `MOSDNS_UPSTREAM_BACKEND`, Go fallback, or Go-side request
state machine may be introduced.

### R4 — Side-effect-safe Rust retry policy

The design must explicitly distinguish protocol-approved retries from arbitrary
re-execution. A post-send error/cancellation must never cause an unreviewed
second request path. Cancellation has priority over starting new retry work.

### R5 — Complete Phase 3B first

This task cannot start until Phase 3B sequence/execution ownership is
implemented, root-reviewed and archived. Phase4 should consume the Rust-native
execution/query abstractions rather than add another hybrid bridge.

### R6 — Keep Phase 4 bounded

Do not modify Go production listeners, `EntryHandler`, Go sequence execution,
Go `NewUpstream`, config/WebUI/API, deployment, or default `main` behavior.
Do not implement TLS/HTTPS, QUIC/HTTP3 or server listeners in this first
UDP/TCP foundation unless a separately reviewed scope change authorizes it.

## Acceptance criteria

- [ ] `design.md` and `implement.md` are root-reviewed before `task.py start`.
- [ ] A reviewed compatibility/deviation matrix separates MosDNS
      product/protocol contracts from Go implementation details.
- [ ] A pure Rust upstream core passes malformed-input, ownership,
      demultiplexing, concurrency, cancellation/deadline, retry/fallback-to-TCP,
      connection lifecycle and deterministic-close tests.
- [ ] Query input is not mutated and returned responses expose the correct
      original request ID under concurrent UDP/TCP traffic.
- [ ] UDP `TC=1` -> TCP behavior and TCP DNS framing match the frozen product
      contract without requiring the Go transport implementation.
- [ ] One shared Rust async runtime/lifecycle model is documented and tested;
      no transport C ABI/capability/handle namespace is added.
- [ ] KixDNS transport research is pinned and classified before reuse.
- [ ] Rust fmt/test/clippy/release and isolated Linux network tests pass; the
      existing Go repository tests/build remain green because Phase4 does not
      change Go production code.
- [ ] No Go pool-buffer ownership, Go fallback/selector, server listener,
      configuration/WebUI, or production deployment change is introduced.

## Planning gate status

This PRD remains **NO-GO/deferred** until Phase3B is completed and archived.
After that, create/review `design.md`, `implement.md`, and the KixDNS transport
research ledger. Planning must freeze the Rust async runtime, cancellation and
retry state machine, protocol/socket compatibility matrix, byte ownership,
connection lifecycle, and test strategy before start.

No transport ABI, Go adapter or production selector is expected in the future
design. If a later task ever proposes one, it requires a separate explicit
architecture review because it conflicts with the current Rust-native target.

## Out of scope

- TLS/HTTPS, QUIC/HTTP3, DoH/DoT/DoQ and server listeners;
- Go `EntryHandler`, Go sequence, Go upstream selection, Go pool lifecycle,
  configuration, WebUI/API, metrics/audit schema or deployment;
- any new Go↔Rust fallback/selector/cgo transport path;
- enabling an incomplete Rust branch for production;
- removing the already-existing Phase1/2/3A hybrid scaffolding before the
  Rust-native host exists — that cleanup belongs to the later retirement gate.
