# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. This document defines the implementation boundary; it
does not authorize code changes before the final planning summary is approved
and `task.py start` is run.

## 0. Pre-start gates (R0, blocking)

R0 is a blocking predecessor of Slice 1 and is part of Slice 0's exit criteria.
It converts three currently implicit assumptions into pinned, executable
contracts. None of them may be deferred to a later task.

### 0.1 R0a — H3 request cancellation is core

Define the per-phase local cancellation contract for one H3 request stream,
against the pinned `h3 0.0.8` / `h3-quinn 0.0.10` API, for exactly these phases:

1. **before send** — no request byte exists; the stream is simply dropped;
2. **after request FIN** — the request is complete; only the receive side is
   torn down;
3. **during response head** — a read future in the pinned h3-quinn layer may
   already hold the internal receive stream;
4. **during body read** — same hazard as (3), on the data path.

The pinned hazard is explicit and must be honored: `h3_quinn::RecvStream::poll_data`
takes the `Option<quinn::RecvStream>` into its in-flight `read_chunk_fut` and only
restores it when that future completes; a local control decision drops that
future, leaving the option `None`, and a subsequent
`h3::client::RequestStream::stop_sending` reaches
`self.stream.as_mut().unwrap()` and panics. Therefore:

- R0a must decide, per phase, whether an active stop is safe or whether the
  contract is drop-only teardown.
- For any phase where an active stop is not safe through the pinned API, the
  documented contract is: return the unchanged typed control error and drop the
  `RequestStream`, letting Quinn's `RecvStream::Drop` stop unread receive data.
  That is a deliberate, tested contract, not an unimplemented feature.
- R0a must include a Slice 0 test that enumerates the four phases and asserts
  (a) only the offending stream is affected, (b) no panic occurs, and (c) the
  shared H3 connection and driver remain healthy.
- A dependency change is never the remedy. If the pinned API cannot satisfy R0a
  for a phase, that phase is recorded as drop-only and the task stops for
  re-review.

### 0.2 R0b — connection-level versus stream-level error classification is core

Produce one authoritative classification table over the pinned error vocabulary
and make every later slice consume it. Each error maps to exactly one class:

- **stream-local** — the query fails; the entry stays healthy and reusable;
- **entry-terminal** — the exact key+generation is marked dead and evicted; no
  query is replayed.

The table must be derived from the pinned sources, not from a guess about a
`Result` shape, and must cover at least: DoQ stream reset/stop, DoQ framing and
response-validation failures, DoQ `open_bi`/write/read failures, H3 request
stream errors, H3 driver `poll_close` terminal outcomes, and `quinn::Connection`
`closed`/`close` outcomes. A `SendRequest`-level failure is not by itself proof
that the physical connection is dead, and a stream-level protocol error is not
by itself proof that it is healthy. R0b also records which errors are
`NotSent` versus `Sent`/`MaybeSent` for `SideEffectState` purposes, and that
classification must not be inferred from the eviction class.

### 0.3 R0c — pinned API assumptions are frozen

Every API fact in `research/quic-reuse-evidence.md` for `quinn 0.11.7`,
`h3 0.0.8`, and `h3-quinn 0.0.10` is verified in R0 against the vendored locked
sources and pinned to an exact file and line range, with an explicit
holds/does-not-hold result. The dependency graph stays exactly as locked. If a
recorded assumption does not hold, R0 records the mismatch and the task stops
for re-review; adding, removing, or bumping a dependency to satisfy the
contract is forbidden.

### 0.4 R0 exit criteria

R0 is closed only when: the four-phase cancellation contract is documented and
tested at the model level; the classification table is complete and referenced
by the DoQ/H3 flow sections; the pinned assumptions are verified with source
citations; and the diff contains no dependency change.

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
generation identity (it never became `Active` and has no drain to await), so a
later independent exchange may create a replacement. This early removal is
distinct from the `Active -> Closing -> Drained/Failed` path in section 7.1,
where an entry that served leases is removed only after its drain completes.

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
2. Enter one **atomic admission section** (see 4.1): transition dead/expired
   entries to `Closing` (they stay discoverable until `Drained`/`Failed` per
   4.2 and 7.1), check the owner-entry bound, and reserve the key's
   placeholder/generation.
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

### 4.1 Atomic multi-key admission

`MAX_CONNECTIONS_PER_OWNER = 8` is a **cross-key** invariant, so it is enforced
by a single owner-map critical section that performs all of the following with
**no `await` inside the section**:

1. lookup of the requested key;
2. transition of dead and idle-expired entries to `Closing` within this section
   (never removal: the entry stays discoverable until `Drained`/`Failed`, per
   4.2 and 7.1, and its async drain is awaited after the lock is dropped);
3. the capacity check against `MAX_CONNECTIONS_PER_OWNER` (a same-key hit does
   not consume new capacity), counting entries that are still `Closing` because
   their slot is not free until they reach `Drained`/`Failed`;
4. placeholder/generation reservation for the key being admitted, so a
   concurrent admission for another key observes the reserved slot.

The owner map guard is `std::sync::Mutex` (or an equivalent non-async lock) and
is dropped before any connect, stream, driver, or close await. The following are
explicitly non-conforming:

- a check-then-insert split across two lock acquisitions;
- releasing the lock between the capacity check and the placeholder insert;
- a per-key lock that leaves the global count racy;
- computing the count from a snapshot and inserting afterwards;
- holding the map lock across an `await`, which would serialize the owner.

A failed initialization releases the reserved slot by key+generation identity so
a later independent exchange may admit a replacement.

Deterministic test (Slice 0, no socket): spawn many concurrent admissions for
distinct keys against a barrier, with the entry count observed under the same
lock. Assert that live entries never exceed `MAX_CONNECTIONS_PER_OWNER`, that the
admitted-key count reaches the cap exactly, that every excess admission receives
the typed capacity result, and that the same-key reservation is not
double-counted. The test must also cover an entry that is idle-expired or closed
concurrently: while it is `Closing` it still occupies its slot, so a new key is
not admitted until it reaches `Drained`/`Failed`. The test must fail if the
admission section is split, if the lock is dropped before the placeholder insert,
or if a `Closing` entry's slot is reused before its drain completes.

### 4.2 Idle expiry

Idle expiry is lazy: every lookup/admission and an explicit test/maintenance
method checks the injected clock while holding the owner-map lock. An expired
entry is transitioned to `Closing` in that same lock section, not removed:
because only an `Active` entry is leasable, a `Closing` entry is already
unleasable. The `Closing` entry stays discoverable while its async close/drain
runs and is removed from the map only after it reaches `Drained` or `Failed`,
per section 7.1. The lazy maintenance pass then awaits the entry's shared
teardown completion (section 7.2), so a concurrent close caller joins the same
teardown instead of racing a vanished entry. No hidden reaper task is needed.

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

A query cancellation resets/stops only its request stream **as permitted by the
R0a per-phase contract in section 0.1**. If cancellation wins while h3-quinn owns
an internal read future, the request-stream receive side is in the pinned
`None`-option hazard state, and the implementation must not call
`RequestStream::stop_sending`, which unwraps that option and panics. For such a
phase the contract is drop-only teardown: return the unchanged typed control
error and drop the `RequestStream`, letting Quinn's `RecvStream::Drop` stop
unread receive data. Every phase is proven through loopback to leave the shared
H3 driver and another concurrent request healthy. This is a stream-local teardown
requirement, not permission to close the connection, and it is not an
optional/deferred item: R0a must close it before Slice 1.

The H3 driver is not reused from `H2ScopeLease`: the H2 lease assumes one
exchange and can seal/abort at response completion, while this driver must remain
alive across requests. A dedicated QUIC driver handle must make admission,
shutdown, and drain idempotent.

## 7. Close, dead-entry eviction, and final commit

### 7.1 Entry lifecycle

Every `QuicConnectionEntry` exposes an explicit, observable lifecycle:

```text
Active ──close/evict──▶ Closing ──drain complete──▶ Drained
                                  └─drain terminal──▶ Failed
```

- **Active** — admitted, may serve leases.
- **Closing** — admission stopped for this key. The entry **remains in the owner
  map and discoverable** until it reaches `Drained` or `Failed`, so a concurrent
  close caller or a maintenance pass can observe and join the in-progress
  teardown rather than racing a vanished entry. `Closing` is not removed from the
  map on entry; removal happens only at `Drained`/`Failed` (section 7.1).
- **Drained** — the H3 driver has stopped, all request streams have ended, the
  QUIC connection/endpoint handle has been released, and the entry's
  `Lifecycle` liveness registration has been dropped.
- **Failed** — the drain could not complete gracefully and the entry was made
  terminal by an explicit bounded force path (see 7.3). `Failed` is also a
  complete terminal state: it still releases the liveness registration and never
  leaves a detached driver.

`Drained` and `Failed` are the only states from which the entry is removed from
the map. Both are idempotent terminals.

### 7.2 Shared idempotent teardown completion

All close paths for an entry (owner close, idle expiry, dead-entry eviction,
protocol-terminal failure) converge on **one shared teardown future per entry**:

- The entry stores a shared completion primitive (for example an
  `Arc<tokio::sync::OnceCell<TeardownOutcome>>` or a stored shared future/notify
  pair) created when the entry first transitions to `Closing`.
- The **first** caller to observe `Closing` becomes the driver of the teardown;
  every other caller awaits the same primitive. No caller starts a second
  teardown and no caller observes a half-finished state.
- Concurrent `close()` calls on the owner therefore all await the same per-entry
  completion, and the owner-level `Lifecycle::finish_close` still gates on the
  remaining registrations.
- Teardown is idempotent: a second transition through the same entry returns the
  recorded `TeardownOutcome` without re-running shutdown, re-closing the QUIC
  connection, or double-releasing a guard.

### 7.3 Abort safety: the driver survives the first close caller

Aborting the future of the first close caller must not detach the H3 driver, drop
its `JoinHandle` without supervision, or lose the entry's liveness hold. The
design requirement is:

- The driver task is **owned by the entry**, not by the close caller's future.
  Dropping a close caller's future drops only that caller's await on the shared
  completion; it does not drop the driver handle, the shutdown sender, or the
  entry's `SharedInFlightGuard`.
- The shutdown signal is delivered by state stored in the entry (a shutdown
  command channel/`Notify` plus the entry's health flag), not by a value moved
  into the close caller's future. Once `Closing` is entered, the shutdown signal
  stays pending until the entry observes it, even if no close caller is alive.
- If the aborted caller was the only driver of the teardown, a later close caller
  or the owner's own drain pass re-drives the same shared completion. Because the
  `Closing` entry remains discoverable in the map (7.1), the owner's close path
  re-scan finds it and completes the drain. An entry can therefore not be
  stranded in `Closing`.
- The liveness registration (`Lifecycle::register_owned`/`register_shared` guard)
  is held by the entry and is released only at `Drained`/`Failed`. It is never
  moved into a droppable caller future, so an aborted close caller cannot make
  `Lifecycle` report drained while the driver is still running.
- The driver task is spawned through the entry's owned scope with a stored
  join/drain handle. It is never `tokio::spawn`-and-forget and never detached.

To keep this bounded, the abort path and the normal path share one rule: **no
teardown progress depends on any single caller future being polled**. Progress
depends on the entry state, the shutdown command, and the driver being driven by
whichever owner/close pass is currently alive; if none is, the next close or
drain pass re-enters from `Closing`.

### 7.4 Owner close sequence

Owner close has three phases:

1. `begin_close` transitions the lifecycle and cancels the owner token. It
   atomically rejects new exchanges.
2. Under the owner map lock, every entry is transitioned to `Closing`
   (`Active -> Closing`) but **is not removed**. The transition is the
   placeholder/reference used by the shared completion; no exchange can
   reacquire a `Closing` entry because the lease path rejects non-`Active`
   entries.
3. Every `Closing` entry's shared teardown completion is awaited. H3 entries send
   the driver's shutdown command, cancel request streams, close/force-close the
   QUIC connection as needed, and await the owned driver task. When the entry
   reaches `Drained`/`Failed`, it is removed from the map and its liveness guard
   is dropped. All exchange registrations then drain through `Lifecycle` before
   `finish_close`.

If an entry is already `Drained`/`Failed`, close is idempotent and only waits for
the owner-level registrations. If a peer never cooperates, local stream
cancellation and an explicit QUIC close code provide the bounded connection
teardown; the owner does not wait forever for a peer response.

### 7.5 Eviction and final commit

Connection-level failure eviction uses key plus generation identity. A stale
failure callback cannot remove a newer replacement. Stream-local protocol errors
do not evict unless the connection/driver health state also proves terminal, per
the R0b classification table in section 0.2 and section 9.

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

The **Entry** column is produced by the R0b classification table (section 0.2),
not by each call site. Any error the table does not classify is a Slice 0 gap
that must be closed before Slice 1, not a runtime guess. The `NotSent` versus
`Sent`/`MaybeSent` `SideEffectState` decision is independent of the eviction class:
a stream-local failure can be `Sent` (the request is fully written) and a
connection-terminal failure can be `NotSent` (the handshake never completed).

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT, session resumption,
connection migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks,
interface/source binding, UDP retransmission, listeners, config, plugins,
sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or
general-purpose pool abstraction enters this task.

The QUIC/H3 dependency graph stays locked. Any proposed dependency change is a
Slice 0 blocker and requires a revised plan/review before implementation. The
same rule applies to R0a/R0b/R0c: a pinned API that cannot satisfy the contract
stops the task for re-review and is never worked around by adding, removing, or
bumping a dependency.

## 11. Test architecture

- Slice 0 uses pure key/limit/entry-state tests and proves no socket or QUIC I/O.
  It must include the three deterministic concurrency/contract tests introduced
  above: the multi-key atomic admission cap test (4.1), the
  aborted-at-barrier/concurrent-close shared-teardown test (7.2/7.3), and the
  four-phase R0a cancellation contract test (0.1). All three run without network
  I/O and must fail if the corresponding contract is removed.
- Slice 1 reuses the existing in-process DoQ fixture style, adds connection
  accept counting, concurrent stream markers, one-stream cancellation, and
  dead-connection replacement evidence.
- Slice 2 extends the H3 fixture to count one connection and multiple request
  stream IDs, hold/release selected responses, verify the driver remains alive,
  and exercise request cancellation without killing another stream, including the
  R0a drop-only phases.
- Slice 3 adds peer stream-budget fixtures, local bound/backpressure tests,
  idle-clock tests, concurrent close tests, resolver A/AAAA-to-key tests, and
  bounded stress.
- Every slice asserts connection count, stream/request count, lifecycle count,
  final commit behavior, and no late success where the relevant boundary is
  exercised.

### 11.1 Aborted-at-barrier / concurrent-close test

This test is the executable form of 7.2/7.3 and is a Slice 0 deliverable:

1. Build an entry with a deterministic teardown barrier (a gate the driver/
   teardown path must pass through before it can reach `Drained`). No socket is
   opened.
2. Spawn concurrent `close()` callers for the same owner/entry.
3. Abort the **first** close caller's future while it is parked at the barrier.
4. Assert, at the barrier: the entry is still discoverable as `Closing`; the
   driver/`JoinHandle` is still owned (not detached); the liveness registration
   is still held; and no teardown completion has been reported.
5. Release the barrier and let a second close caller (or the owner's own drain
   pass) proceed.
6. Assert: exactly one teardown runs; all close callers observe the same
   `Drained` outcome; the entry is removed only after `Drained`; the liveness
   registration reaches zero; and no late response commits.

The test must fail if the teardown future is moved into the first caller, if the
driver handle is owned by that caller, or if a `Closing` entry is removed from
the map in any state other than `Drained`/`Failed`.

## 12. Rollback

Rollback is a task-scoped revert of the new QUIC reuse module, exports, tests, and
any narrowly required lifecycle/error additions. Existing one-shot QUIC,
secure, resolver, and generic TCP reuse paths remain unchanged. Unrelated dirty
files are not staged or reverted.
