# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. This document defines the implementation boundary; it
does not authorize code changes before the final planning summary is approved
and `task.py start` is run.

## 1. Boundary and ownership

The deliverable stays in `rust/upstream-core`. The existing one-shot
`DoqUpstream` and `Doh3Upstream` remain valid fresh-connection primitives and
their focused tests remain regression coverage. The new code is a sibling
QUIC-specific owner, planned as `src/quic_reuse.rs`, with only the minimum
re-exports needed by future Rust-native composition.

It must not modify the generic serial `ReuseOwner` into a cross-protocol pool.
The shared vocabulary is reused, not the old storage/lifecycle implementation:

- `Lifecycle` remains the only owner admission, close, in-flight registration,
  and final response commit gate.
- `ExchangeContext` / `ExchangeControl` remain the only absolute
  deadline and caller/owner cancellation race.
- `ServerIdentity`, `TlsPolicy`, `PublishedTarget`, and
  `DohEndpoint::get_request_target` remain authoritative.
- `tcp::write_frame` and the existing DNS response validators are reused;
  no second DNS framing or DoH target/response contract is introduced.

## 2. Validated key

Planned public model:

- `QuicProtocol::{Doq, Doh3}` is a closed enum whose ALPN mapping is fixed to
  `doq` or `h3`. Callers cannot pass arbitrary ALPN bytes.
- `QuicReuseKey` contains the numeric `SocketAddr`, protocol/ALPN
  discriminator, canonical identity text, TLS verification mode, TLS roots
  revision, and an optional DoH3 authority.
- Constructors are protocol-specific, for example
  `from_doq(&DoqEndpoint, &TlsPolicy)` and
  `from_doh3(&DohEndpoint, &TlsPolicy)`. They accept only already
  validated endpoints and policies, and reject no new endpoint semantics.
- DoQ has no HTTP authority. DoH3 authority is included because the existing
  `DohEndpoint` separates the HTTP origin authority from the numeric dial and
  a connection must not silently serve a different HTTP origin.
- TLS key material is never copied into the key. The verified/insecure mode plus
  the opaque `TlsPolicy::roots_revision` identifies the trust configuration.
  Cloning a policy preserves the revision; constructing a new verified policy
  intentionally prevents reuse with the old roots.
- A resolver refresh or A/AAAA selection change only changes the numeric dial
  field. It never rewrites identity, authority, or an established connection.

Key tests are pure and deterministic. They cover every isolation dimension,
including DoQ versus DoH3 on the same numeric address and two otherwise equal
verified policies with distinct roots revisions.

## 3. Owner and connection-entry state

Planned owner shape:

`QuicReuseOwner` owns:

1. an `Arc<Lifecycle>` and owner `TransportCancellation`;
2. a short-lock map from `QuicReuseKey` to one
   `Arc<QuicConnectionEntry>`;
3. task-local limits and an injected clock/maintenance seam for deterministic
   idle tests;
4. no generic TCP pool and no hidden production/config selector.

Each `QuicConnectionEntry` owns one physical QUIC connection for one key,
a local stream-slot semaphore, a last-used timestamp, a generation/health state,
and the endpoint handle that keeps Quinn's UDP endpoint driver alive. The entry
also owns a `Lifecycle::register_owned` liveness guard from insertion until
the entry is closed and fully drained. This makes an idle connection visible to
owner close without borrowing a lifetime across an async task.

The entry has protocol-specific payload:

- DoQ stores the cloned `quinn::Connection`. Query attempts call
  `open_bi` on that connection and never share stream halves.
- DoH3 stores the H3 sender template and a long-lived driver handle. Each query
  clones the sender (the pinned `h3 0.0.8` sender is cloneable), while the
  driver task owns and continuously polls the H3 `Connection`.
- The H3 driver state has a health flag and a shutdown command/notification. A
  driver terminal error marks the entry dead; it does not silently create a
  replacement for the request that was using it.

Connection creation is single-flight per key. A map entry is installed before
the async connect/build begins, and a `tokio::sync::OnceCell` or equivalent
entry-local state ensures concurrent first users await the same connection rather
than opening duplicate QUIC connections. A failed initialization is removed by
generation identity, so a later independent exchange may create a replacement.

No async wait occurs while the owner map lock is held. Map operations are short,
and all network, stream, driver, and close awaits happen after the relevant entry
or snapshot has been acquired.

## 4. Bounds and admission

The planning constants are task-local and non-configurable:

- `MAX_STREAMS_PER_CONNECTION = 32`;
- `MAX_CONNECTIONS_PER_OWNER = 8`;
- `QUIC_IDLE_TIMEOUT = 30s` (lazy maintenance, not a background timer);
- one physical connection per key.

Admission sequence:

1. Register the exchange through the owner's `Arc<Lifecycle>`, rejecting
   `Closed(NotSent)` before any network operation.
2. Prune dead/expired entries and check the owner-entry bound.
3. Get or single-flight-create the exact key entry.
4. Take a stream permit with a non-queueing operation. If local permits are
   exhausted, return a typed `Backpressure` error with `NotSent`; do not
   grow an owner queue or open a second same-key connection.
5. Race QUIC/H3 stream opening against the exchange's original
   `ExchangeControl` and absolute deadline. If the peer has exhausted its
   advertised stream credit, the open future may remain pending only under this
   bounded caller-owned race.
6. On every exit, release the stream permit exactly once. Update idle time only
   when the entry has no active permits and remains healthy.

Idle expiry is lazy: every lookup/admission and an explicit test/maintenance
method checks the injected clock. Expired entries are removed from the map before
their async close/drain is awaited. No hidden reaper task is needed.

## 5. DoQ exchange flow

For a leased DoQ entry:

1. Check owner/caller/deadline with `SideEffectState::NotSent`.
2. Open one fresh bidirectional stream on the shared connection.
3. Copy the caller query, zero its ID, and use the existing two-byte stream
   framing helper to write it.
4. Finish the request side. If local cancellation wins after the stream exists,
   stop only that stream with the existing DoQ cancellation behavior.
5. Read exactly one framed response through the peer FIN, validate peer wire ID
   zero, restore the original caller ID, and run the existing DNS response
   validator.
6. Apply `Lifecycle::commit_final_response` with the original caller
   cancellation/deadline before releasing the permit.
7. Return the connection to the entry if healthy. A stream reset, malformed
   response, or local cancellation is stream-local; a QUIC connection error or
   closed handle marks the entry dead and removes that generation.

A failed current query is never replayed. A replacement is available only to a
later independent exchange, and only after the old generation has been
evicted/closed.

## 6. DoH3 exchange and long-lived driver

Connection setup is one-time per entry:

1. Build the exact `TlsPolicy`-derived client config offering only `h3`.
2. Open the numeric QUIC connection using the endpoint's separate identity.
3. Build H3 once, which creates control/QPACK streams and returns the driver plus
   cloneable `SendRequest`.
4. Register the driver as an owned child before the entry becomes reusable.
5. Spawn it through the caller's Tokio runtime with an explicit join/drain
   handle. The task loops `poll_close` until normal or terminal connection
   completion and publishes health without detaching.

For each request:

1. Clone the entry's H3 sender template and build the existing DoH GET target
   from `DohEndpoint::get_request_target`.
2. Open one H3 request stream, send exactly the existing headers, and finish the
   request side.
3. Read/validate the response using the existing bounded DoH validator and
   restore only that request's original ID.
4. Race every request phase against the original control/deadline.
5. Commit through the shared lifecycle gate, then release only that request's
   stream permit.

A query cancellation resets/stops only its request stream where the h3 0.0.8
API can do so safely. If cancellation wins while h3-quinn owns an internal
read future, the implementation must not call the known panic-prone
`stop_sending` path on a missing internal stream; it drops/resets only the
request stream using a safe phase-aware helper and proves through loopback that
the shared H3 driver and another request remain healthy. This is a stream-local
teardown requirement, not permission to close the connection.

The H3 driver is not reused from `H2ScopeLease`: the H2 lease assumes one
exchange and can seal/abort at response completion, while this driver must remain
alive across requests. A dedicated QUIC driver handle must make admission,
shutdown, and drain idempotent.

## 7. Close, dead-entry eviction, and final commit

Owner close has two phases:

1. `begin_close` transitions the lifecycle and cancels the owner token.
   It atomically rejects new exchanges. Under the owner map lock, all entries are
   removed/marked closing so no exchange can reacquire one.
2. Snapshot entries are asked to cancel request streams and close their QUIC
   connection/endpoint. H3 entries send the driver's shutdown command and await
   the owned driver task; all entry liveness guards and exchange registrations
   then drain through `Lifecycle` before `finish_close`.

If an entry is already dead, close is idempotent and only waits for its driver,
endpoint, and guards to disappear. If a peer never cooperates, local stream
cancellation and an explicit QUIC close code provide the bounded connection
teardown; the owner does not wait forever for a peer response.

Connection-level failure eviction uses key plus generation identity. A stale
failure callback cannot remove a newer replacement. Stream-local protocol errors
do not evict unless the connection/driver health state also proves terminal.

The final response commit occurs before stream permit release. Therefore:

- a successful commit remains a success even if close starts immediately after it;
- close/cancel/deadline winning before the commit returns a typed error;
- no entry can be returned to the pool before the commit decision is complete.

## 8. Resolver composition

Use the existing resolver snapshot as a read-only input:

- DoQ uses `ResolverComposition::doq_endpoint(PublishedTarget, ServerIdentity)`.
- DoH3 uses `ResolverComposition::doh_endpoint(PublishedTarget, service_url)`
  and the existing `DohEndpoint`.
- The owner builds a key from the resulting endpoint and policy. A selected A or
  AAAA address therefore produces a distinct key without copying resolver state
  into the owner or changing the authenticated service identity.
- Resolver refresh does not proactively close an established connection. The
  old key remains valid until idle/dead/owner teardown; a new selected target
  creates or reuses its own key.

No resolver algorithm, fallback, cross-family race, or socket policy is added.

## 9. Error and state matrix

| Event | Current exchange | Entry | Reuse |
| --- | --- | --- | --- |
| invalid key/zero port | typed pre-I/O error, `NotSent` | none | no entry |
| local stream slots full | typed backpressure, `NotSent` | healthy | keep |
| handshake/build failure | typed connect/TLS, `NotSent` | evict exact generation | later call may replace |
| stream open/send uncertainty | typed existing send/connect error | keep unless connection is terminal | no replay |
| peer stream reset / malformed response | typed DoQ/DoH3 terminal error, usually `Sent` | keep if connection healthy | reusable |
| caller cancel / deadline | existing typed control error | keep if connection healthy | reusable |
| owner close | `Closed` with existing side-effect state | close and drain | unavailable |
| connection close / H3 driver failure | typed connection error | evict exact generation | later call may replace |
| validated response | candidate until final commit | keep after permit release | reusable only after commit |

The implementation must preserve the existing closed side-effect vocabulary; it
must not turn stream-local failure into an implicit retry or cross-protocol
fallback.

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT, session resumption,
connection migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks,
interface/source binding, UDP retransmission, listeners, config, plugins,
sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or
general-purpose pool abstraction enters this task.

The QUIC/H3 dependency graph stays locked. Any proposed dependency change is a
Slice 0 blocker and requires a revised plan/review before implementation.

## 11. Test architecture

- Slice 0 uses pure key/limit/entry-state tests and proves no socket or QUIC I/O.
- Slice 1 reuses the existing in-process DoQ fixture style, adds connection
  accept counting, concurrent stream markers, one-stream cancellation, and
  dead-connection replacement evidence.
- Slice 2 extends the H3 fixture to count one connection and multiple request
  stream IDs, hold/release selected responses, verify the driver remains alive,
  and exercise request cancellation without killing another stream.
- Slice 3 adds peer stream-budget fixtures, local bound/backpressure tests,
  idle-clock tests, concurrent close tests, resolver A/AAAA-to-key tests, and
  bounded stress.
- Every slice asserts connection count, stream/request count, lifecycle count,
  final commit behavior, and no late success where the relevant boundary is
  exercised.

## 12. Rollback

Rollback is a task-scoped revert of the new QUIC reuse module, exports, tests, and
any narrowly required lifecycle/error additions. Existing one-shot QUIC,
secure, resolver, and generic TCP reuse paths remain unchanged. Unrelated dirty
files are not staged or reverted.
