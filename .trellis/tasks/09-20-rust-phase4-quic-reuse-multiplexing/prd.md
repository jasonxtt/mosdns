# Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. This task authorizes no implementation until the final
planning summary is explicitly approved, `task.py start` is run, and the
selected DSH executor receives a bounded slice assignment.

## Goal

为未来 Rust-native host 建立一个 QUIC-specific shared connection owner：DoQ
在一个已认证 QUIC connection 上为每个 query 打开独立的 concurrent
bidirectional stream，DoH3 在一个已认证 H3 connection 上复用 concurrent
request streams。复用必须由 validated `QuicReuseKey` 隔离，且不把旧的
serial TCP `ReuseOwner` 泛化成万能池。

用户价值：完成 QUIC outbound data plane 从 one-shot foundation 到真正可复用、
可并发、可有界关闭的 transport 基础，让后续 Rust-native host 可以使用 DoQ
和 DoH3，而不改变已有的 caller deadline、取消、服务身份、TLS policy、resolver
numeric target 或最终 response commit 契约。

## Confirmed repository facts

- `rust/upstream-core/src/quic.rs:1-66` and the `DoqUpstream` /
  `Doh3Upstream` definitions (`:210-217`, `:725-738`) intentionally
  implement fresh, one-shot connections. QUIC pooling, reuse, and multiplexing
  are explicitly deferred there; this task is the next independent scope.
- `Lifecycle` in `rust/upstream-core/src/lib.rs:585-826` serializes owner
  admission with `Open -> Closing`, provides shared liveness registration for
  children, and exposes the only crate-level final response commit gate.
- `ExchangeContext` / `ExchangeControl` and `SideEffectState` are already
  the shared absolute-deadline, caller-cancellation, owner-cancellation, and
  send-state vocabulary. The new owner must consume them rather than create a
  second control race.
- `rust/upstream-core/src/reuse.rs:1-27` defines the existing owner as
  serial-per-connection TCP/DoT/DoH reuse. Its `MAX_PENDING_PER_CONNECTION = 1`
  and idle pool are not a QUIC design; this task must use a separate owner and
  separate key/state model.
- `DoqEndpoint` and the one-shot DoQ path already enforce numeric dial plus
  separate `ServerIdentity`, exact `doq` ALPN, zeroed outbound DNS ID,
  stream FIN, response ID validation, and original-ID restoration.
- The one-shot DoH3 path already reuses `DohEndpoint::get_request_target`,
  exact `h3` ALPN, the bounded DoH response validator, and a tracked short-lived
  H3 driver. Its one-shot `H2ScopeLease`-style teardown is not a long-lived
  connection owner and must not be copied as the reuse abstraction.
- `ResolverComposition::doq_endpoint` in
  `rust/upstream-core/src/resolver/owner.rs:900-915` already consumes only
  `PublishedTarget::dial()` while preserving the caller's identity. DoH3 can
  use the existing `doh_endpoint` composition because its endpoint type is the
  existing `DohEndpoint`.
- `TlsPolicy` carries an explicit verification mode and opaque `roots_revision`
  for reuse identity; its current contract keeps 0-RTT early data and session
  resumption disabled.
- The locked QUIC/H3 graph is already present in `rust/upstream-core/Cargo.toml`;
  Slice 0 is model-only and should not add dependencies.

## Requirements

### R1. QUIC-specific reuse model

Add a dedicated `QuicReuseKey` and `QuicReuseOwner` boundary in
`rust/upstream-core`. The key must be constructed only from validated endpoint
and TLS inputs and must isolate at least:

- numeric `dial` address, including port;
- protocol/ALPN (`doq` versus `h3`) as a closed, non-arbitrary discriminator;
- canonical `ServerIdentity`;
- TLS verification mode and `TlsPolicy` roots revision;
- the validated DoH3 service authority where it is distinct from the identity.

The existing generic `ReuseOwner` / `ReuseKey` must not become a QUIC universal
pool. A connection entry is owned by the QUIC owner and is never handed across
keys.

### R2. DoQ stream multiplexing

For one validated DoQ key, reuse one authenticated QUIC connection and open one
fresh bidirectional stream per query. Each stream must independently preserve
the existing DoQ contract:

- two-byte big-endian length framing remains the existing shared helper;
- the outbound copy has DNS ID zeroed, while caller-owned bytes are untouched;
- request-side FIN and response-side FIN are required;
- peer response ID is validated as zero before restoring the caller's original
  ID;
- stream-local cancellation terminates only that stream and does not close a
  healthy shared connection;
- a connection-level failure marks the connection dead and makes it eligible
  for replacement; the current query is never silently replayed.

Concurrent queries must have independent stream ownership and must not require
DNS-ID rewriting or response demultiplexing.

### R3. DoH3 connection reuse

For one validated DoH3 key, retain one authenticated H3 connection and one
long-lived owned H3 driver. Each query clones/leases the H3 request sender and
opens its own H3 request stream. Request target, `:authority`, headers, body
limits, status/media-type/encoding checks, and original-ID restoration remain the
existing DoH contract.

The long-lived driver must be explicitly owned and continuously driven. It may
not be detached, silently replaced by a one-shot `H2ScopeLease`, or allowed to
outlive owner close. Owner close must stop admission, cancel/finish request
streams, send the driver's shutdown signal, and drain the driver and all
request streams before the owner reports closed.

### R4. Lifecycle and final commit

Reuse the existing `Lifecycle`, `ExchangeContext`, `ExchangeControl`, absolute
deadline, caller cancellation, owner cancellation, `ServerIdentity`,
`TlsPolicy`, resolver `PublishedTarget`, and
`Lifecycle::commit_final_response` contracts.

An exchange remains registered until its validated response has either committed
or returned a terminal error and its stream permit has been released. A
validated response is only a candidate until the existing final commit gate
accepts it. Owner close wins over a late response; cancellation and deadline
retain their existing side-effect state and precedence.

### R5. Bounded concurrency and backpressure

Define QUIC-specific task-local bounds. They are implementation constants, not
YAML/API compatibility, and must not copy the Go implementation's concurrency
number:

- at most one live QUIC connection per validated key in this task;
- a finite local stream-slot bound per connection;
- a finite owner connection-entry bound for resolver/address/key churn;
- an explicit idle-expiry policy;
- no unbounded owner queue. Exhausted local slots return a typed, pre-send
  backpressure error; a peer-advertised stream limit may keep `open_bi` / H3
  stream admission pending only under the caller's original control/deadline
  race.

The planning design proposes `MAX_STREAMS_PER_CONNECTION = 32`,
`MAX_CONNECTIONS_PER_OWNER = 8`, and a 30-second lazy idle expiry. These are
task-local calibration choices, not product contract or Go parity claims.

### R6. Failure, replacement, and teardown

- Handshake/setup failure before DNS application bytes is `NotSent`; the failed
  entry is removed so a later independent exchange can establish a replacement.
- A stream-local reset, malformed response, or local query cancellation does
  not evict a healthy connection merely because the query failed.
- A QUIC connection close, H3 driver terminal failure, endpoint failure, or
  unusable shared transport evicts the exact key/generation. No current query
  is replayed after bytes may have been sent.
- Idle expiry and explicit close remove the entry before asynchronous drain so
  no new lease can race back into a closing connection.
- Concurrent `close()` calls are idempotent and converge through the existing
  lifecycle state machine. No response may commit after close wins.

### R7. Resolver composition

The reuse key must be built from the numeric address selected by an existing
`PublishedTarget` snapshot. A resolver A/AAAA selection change produces a new
key for a new dial; it does not rewrite `ServerIdentity`, DoH3 authority, or an
already-established connection. No Happy Eyeballs, cross-address race, or
resolver redesign is included.

### R8. Pure Rust and no production wiring

The task stays inside `rust/upstream-core` plus focused Rust tests and planning
evidence. It adds no Go/cgo/FFI/C ABI, backend selector, fallback, host/config,
plugin/sequence wiring, API/WebUI, listener, or production/default selection.

## Acceptance criteria

- [ ] A1. Slice 0 has deterministic key tests proving that different numeric
      dials, identities, ALPN/protocols, DoH3 authorities, TLS modes, or roots
      revisions cannot share an entry, while a cloned identical policy and
      endpoint produce the same key. No network I/O is used in Slice 0.
- [ ] A2. DoQ loopback accepts one QUIC connection for multiple concurrent
      queries, observes one independent bidirectional stream per query, and
      returns the matching marker/original ID for every response. No wire-ID
      rewrite or response cross-talk occurs.
- [ ] A3. Canceling or timing out one in-flight DoQ stream produces only that
      query's typed control error, leaves a healthy connection usable, and
      permits a later query to reuse the same connection. A connection-level
      failure is instead evicted and the next independent query establishes a
      replacement without replaying the failed query.
- [ ] A4. DoH3 loopback accepts one QUIC/H3 connection for multiple concurrent
      GET request streams, preserves `:authority`/path and the existing DoH
      response contract, restores each original ID, and proves the driver stays
      alive between requests.
- [ ] A5. DoH3 owner close drains the long-lived driver and every request
      stream, rejects new admissions, converges under concurrent/repeated close,
      and leaves zero registered child/stream residue. A canceled request does
      not close a healthy connection used by another request.
- [ ] A6. Local stream-slot and owner-entry bounds are finite and observable;
      exhaustion returns a typed pre-send backpressure result without an
      unbounded queue. Peer stream-limit exhaustion is controlled by the caller's
      original deadline/cancellation race.
- [ ] A7. Idle expiry removes an unused connection according to the injected
      clock/maintenance path, while a connection used before expiry remains
      reusable. Expired/dead entries cannot be returned after owner close.
- [ ] A8. Existing lifecycle precedence and final commit behavior remain
      observable for DoQ and DoH3: owner close, caller cancellation, deadline,
      and a successful final response cannot be reordered into a late success.
- [ ] A9. Resolver composition tests prove A/AAAA `PublishedTarget::dial()`
      feeds the key while the validated identity/authority remains unchanged;
      no hostname enters the socket dial path.
- [ ] A10. All existing UDP/TCP/DoT/DoH/resolver/reuse/one-shot QUIC tests stay
      green. Focused tests, warnings-denied clippy, formatting, locked
      dependency inspection, `git diff --check`, task validation, and the
      isolated Linux/MSRV plus bounded stress gate pass. The selected web
      reviewer returns an explicit scoped PASS before task archive.
- [ ] A11. Diff inspection confirms no 0-RTT, TLS resumption, connection
      migration, Happy Eyeballs, socket policy, UDP retransmission, listener,
      YAML/config, plugin/sequence, API/WebUI, production wiring, or Go/cgo/FFI
      changes entered this task.

## Out of scope

- 0-RTT and TLS session resumption.
- QUIC connection migration.
- Happy Eyeballs, cross-address racing, or cross-family retry.
- TCP/DoT 64-way pipeline work.
- SOCKS5, `SO_MARK`, `SO_BINDTODEVICE`, local source binding, or any socket
  policy.
- UDP retransmission.
- Server listeners, including inbound DoQ or DoH3.
- YAML/config, plugin/sequence wiring, API/WebUI, production/default wiring,
  deployment, Go/cgo/FFI/C ABI, selectors, mirrors, or fallbacks.
- New QUIC tuning configuration, metrics/audit wiring, or a general-purpose
  transport pool abstraction.

## Key decisions and risks

- The owner is QUIC-specific and key-driven; it is not a specialization of the
  serial TCP `ReuseOwner`.
- One key has one physical connection in this task. Concurrency is stream-level
  and bounded; a later task may consider multiple connections per key only with
  a separate policy review.
- DoQ stream association is structural, so no ID demux is needed. DoH3 stream
  association is structural through the H3 request stream handle.
- H3 driver ownership is the highest lifecycle risk. The implementation must
  prove driver admission, shutdown, stream cancellation, and drain with real
  loopback tests; one-shot H2 scope machinery is evidence, not the new owner.
- The exact task-local bounds are not product behavior. If implementation
  evidence shows the proposed values are unsuitable, the slice may revise the
  constants without changing the public contract, but it must keep the bounds
  finite and the backpressure/close semantics explicit.

## Blocking open questions

None. The latest user decision fixed the task name, four-slice scope, exclusions,
reuse-key dimensions, lifecycle vocabulary, and DSH/web review routing. The
proposed numeric bounds are implementation-level choices recorded for review,
not unresolved product behavior.

## Planning notes

- Dependencies already exist and were audited by the archived
  `09-18-rust-phase4-quic-http3-doq-foundation` task; no new dependency is
  expected.
- Preserve unrelated dirty files and `.DS_Store` files. Do not use
  `git add -A`. Trellis auto-commit remains disabled.
- The old CI task mentioned in the user's bookkeeping note is already fully
  archived by HEAD `eb40379`; only the handover document needed correction.
