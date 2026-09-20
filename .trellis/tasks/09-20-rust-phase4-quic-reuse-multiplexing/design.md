# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only; no code change before the summary is approved and `task.py start` runs.

## 0. Pre-start gates (R0, blocking)

R0 blocks Slice 1 and is part of Slice 0's exit criteria; its three pinned contracts must not be deferred.

### 0.1 R0a — H3 request cancellation is core

Define the per-phase local cancellation contract for one H3 request stream against
the pinned `h3 0.0.8` / `h3-quinn 0.0.10` API for exactly four phases: (1) **before
send** — no request byte exists, the stream is simply dropped; (2) **after request
FIN** — the request is complete, only the receive side is torn down; (3) **during
response head** — a read future in the pinned h3-quinn layer may already hold the
internal receive stream; (4) **during body read** — same hazard as (3), on the data
path.

The pinned hazard must be honored: `h3_quinn::RecvStream::poll_data` takes the
`Option<quinn::RecvStream>` into its in-flight `read_chunk_fut` and restores it only
when that future completes; a local control decision drops that future, leaving the
option `None`, and a subsequent `h3::client::RequestStream::stop_sending` reaches
`self.stream.as_mut().unwrap()` and panics. R0a therefore decides per phase whether
an active stop is safe or the contract is drop-only; where an active stop is unsafe,
return the unchanged typed control error and drop the `RequestStream`, letting
Quinn's `RecvStream::Drop` stop unread receive data (a deliberate, tested contract).

**Slice 0 proves the decision/state model, not the pinned stack.** It has no socket and no QUIC/H3 I/O; its four-phase test asserts only that (a) the model never selects the pinned `Option::None` `stop_sending` path, (b) the transition is per-phase and deterministic, and (c) the logical shared-entry state stays healthy. **The real pinned-stack H3 proof belongs to Slice 2/A5**, which owns the pinned `h3`/`h3-quinn` loopback fixture proving canceling one real H3 request leaves the real connection, driver, and another concurrent request healthy; Slice 0 must not present model evidence as real H3 evidence. A dependency change is never the remedy: a phase the pinned API cannot satisfy is drop-only and stops the task for re-review.

### 0.2 R0b — connection-level versus stream-level error classification is core

Produce one authoritative table over the pinned error vocabulary; every later slice consumes it. Each error maps to exactly one class: **stream-local** (the query fails; the entry stays healthy and reusable) or **entry-terminal** (the exact key+generation is logically deactivated by `Active -> Closing`, immediately unleasable, physical removal only at `Drained`/`Failed`, no query replayed).

Derived from the pinned sources, not a `Result` guess, it covers at least DoQ
stream reset/stop, DoQ framing and response-validation failures, DoQ
`open_bi`/write/read failures, H3 request stream errors, H3 driver `poll_close`
terminal outcomes, and `quinn::Connection` `closed`/`close` outcomes. A
`SendRequest`-level failure does not prove the connection is dead, and a stream
protocol error does not prove it is healthy. R0b also records `NotSent` versus
`Sent`/`MaybeSent` for `SideEffectState`, independent of the deactivation class.

### 0.3 R0c — pinned API assumptions are frozen

Every fact in `research/quic-reuse-evidence.md` for `quinn 0.11.7`, `h3 0.0.8`, and `h3-quinn 0.0.10` is verified in R0 against the locked local registry source and pinned to an exact file/line range with a holds/does-not-hold result. The graph stays exactly as locked; an assumption that does not hold stops the task for re-review, and adding, removing, or bumping a dependency is forbidden.

### 0.4 R0 exit criteria

R0 closes only when the four-phase decision/state model is documented and exercised (not by real H3 I/O), the classification table is complete and referenced by the DoQ/H3 flow sections, the pinned assumptions are verified with locked-local-registry citations, and the diff has no dependency change. It does **not** close the real pinned-stack H3 health proof, a Slice 2/A5 obligation that must not be claimed as R0 evidence.

## 1. Boundary and ownership

The deliverable stays in `rust/upstream-core`. The existing one-shot `DoqUpstream` and `Doh3Upstream` remain valid fresh-connection primitives and regression coverage; the new code is a sibling QUIC-specific owner, planned as `src/quic_reuse.rs`, with only the minimum re-exports needed by future Rust-native composition. It must not turn the generic serial `ReuseOwner` into a cross-protocol pool: the shared vocabulary is reused, not the old storage/lifecycle implementation:

- `Lifecycle` remains the only owner admission, close, in-flight registration, and final response commit gate.
- `ExchangeContext` / `ExchangeControl` remain the only absolute deadline and caller/owner cancellation race.
- `ServerIdentity`, `TlsPolicy`, `PublishedTarget`, and `DohEndpoint::get_request_target` remain authoritative.
- `tcp::write_frame` and the existing DNS response validators are reused; no second DNS framing or DoH target/response contract.

## 2. Validated key

`QuicProtocol::{Doq, Doh3}` is a closed enum fixing ALPN to `doq` or `h3`; callers cannot pass arbitrary ALPN bytes. `QuicReuseKey` holds the numeric `SocketAddr`, protocol/ALPN discriminator, canonical identity text, TLS verification mode, TLS roots revision, and optional DoH3 authority. Protocol-specific constructors such as `from_doq(&DoqEndpoint, &TlsPolicy)` and `from_doh3(&DohEndpoint, &TlsPolicy)` accept only validated endpoints and policies, adding no endpoint semantics. DoQ has no HTTP authority; DoH3 authority is keyed because `DohEndpoint` separates the HTTP origin authority from the numeric dial and a connection must not silently serve a different origin. TLS key material is never copied into the key: verified/insecure mode plus the opaque `TlsPolicy::roots_revision` identifies trust; cloning preserves the revision, and a new verified policy intentionally prevents reuse with old roots. A resolver refresh or A/AAAA selection change changes only the numeric dial field and never rewrites identity, authority, or an established connection.

Key tests are pure and deterministic, covering every isolation dimension including DoQ versus DoH3 on one numeric address and two equal verified policies with distinct roots revisions.

## 3. Owner and connection-entry state

`QuicReuseOwner` owns an `Arc<Lifecycle>` plus owner `TransportCancellation`; a short-lock map from `QuicReuseKey` to one `Arc<QuicConnectionEntry>`; task-local limits and an injected clock/maintenance seam for deterministic idle tests; and no generic TCP pool or hidden production/config selector.

Each `QuicConnectionEntry` owns one physical QUIC connection for one key, a local stream-slot semaphore, a last-used timestamp, a generation/health state, the endpoint handle keeping Quinn's UDP driver alive, and a `Lifecycle::register_owned` liveness guard from insertion until close and full drain, making an idle connection visible to owner close without borrowing a lifetime across an async task.

Protocol-specific payload: DoQ stores the cloned `quinn::Connection`; attempts call `open_bi` on it and never share stream halves. DoH3 stores the H3 sender template and a long-lived driver handle; each query clones the sender (pinned `h3 0.0.8` is cloneable) while the driver task owns and continuously polls the H3 `Connection`. The driver carries a health flag and shutdown command/notification; a terminal error marks the entry dead and does not silently replace it for the request using it.

Connection creation is single-flight per key. The admission section installs a **reserved placeholder** under the owner-map lock before any async connect/build begins, so the map never holds a half-defined entry and every observer sees `Initializing` (reservation exists, key+generation fixed, exactly one initializer, no leases), `Active` (usable transport published, leases allowed), or `Closing`/`Drained`/`Failed` (7.1). A `tokio::sync::OnceCell` or equivalent entry-local state makes concurrent first users join the same initializer rather than open duplicates.

The `Initializing -> Active` publication and the owner/entry close decision share one linearization point: the owner-map/state lock the admission section and `begin_close` use. The initializer may publish `Active` only when, under that lock, the owner is still `Open` **and this exact key+generation is still `Initializing`**; otherwise it observes `Closing` and must never publish `Active`, so the entry is never both `Active` and `Closing`. Initialization outcomes are classified by resource acquisition, not error type (3.1, 3.2).

### 3.1 Initialization versus owner close

Owner close and initialization share one state linearization point:

- Under the owner-map lock, `begin_close` transitions **both `Initializing` and
  `Active` entries to `Closing`**. No entry can be `Initializing` while the owner is
  already closing and still publish `Active`.
- An initializer that observes `Closing` (or a `TeardownRequested` flag set in the same lock section) **must not publish `Active`** and never returns an entry; its fate follows exactly one rule, the 3.2 resource rule — **no transport/H3 resource acquired yet** releases immediately by exact key+generation identity (no supervised task starts), while **a resource was acquired** joins the shared supervised `Closing -> Drained | Failed` teardown (7.2). This crossing rule is referenced by 3.2, 7.1, 7.2, 9, and PRD R6, never restated with a different outcome.
- The reservation carries a generation identity from install time, so a stale initializer can never publish into or tear down a newer replacement; idle expiry and connection-level logical deactivation follow the same lock rule.

### 3.2 Initialization failure and early release

Failure, or close observed during initialization, is classified by resource acquisition, not error class:

- **No resource acquired** — the connect/build failed before any endpoint, QUIC connection, or H3 driver/handle existed, or `TeardownRequested` came first. The reservation never became `Active`, has nothing to drain, and is released immediately by exact key+generation identity, freeing its slot for a later exchange; this is the only removal path outside `Drained`/`Failed`.
- **A resource exists** — an endpoint, QUIC connection, or H3 driver/handle existed before the failure or close request, so the entry must use the shared `Closing -> Drained | Failed` teardown (7.2): the resource belongs to its creating generation, never served a lease, and cannot be released early; one constructed after teardown started is closed by the supervised task.

No async wait occurs while the owner map lock is held; all network, stream, driver, and close awaits happen after the relevant entry or snapshot is acquired.

## 4. Bounds and admission

Task-local non-configurable constants: `MAX_STREAMS_PER_CONNECTION = 32`; `MAX_CONNECTIONS_PER_OWNER = 8`; `QUIC_IDLE_TIMEOUT = 30s` (lazy maintenance, not a background timer); one physical connection per key.

Admission sequence:

1. Register the exchange through the owner's `Arc<Lifecycle>`, rejecting `Closed(NotSent)` before any network operation.
2. Enter the **atomic admission section** (4.1): look up the key, transition idle-expired entries to `Closing` (discoverable until `Drained`/`Failed`, per 4.2 and 7.1), check capacity, and either reuse an `Active` entry or install an `Initializing` reservation.
3. Resolve the lookup by state (4.3): `Active` used directly, `Initializing` joins the single-flight initializer, `Closing` returns the typed pre-send closed result.
4. Take a stream permit with a non-queueing operation; if local permits are exhausted, return a typed `Backpressure` error with `NotSent`, never growing an owner queue or opening a second same-key connection.
5. Race QUIC/H3 stream opening against the exchange's original `ExchangeControl` and absolute deadline; if the peer exhausted its advertised stream credit, the open future may stay pending only under this bounded caller-owned race.
6. On every exit, release the permit exactly once and update idle time only when the entry has no active permits and stays healthy.

### 4.1 Atomic multi-key admission

`MAX_CONNECTIONS_PER_OWNER = 8` is a cross-key invariant enforced by a single
owner-map critical section with **no `await` inside it**: (1) lookup the requested
key; (2) transition dead and idle-expired entries to `Closing` in this section
(never removal — they stay discoverable until `Drained`/`Failed`, per 4.2 and 7.1,
and the transition starts the supervised teardown with no drain awaited on the
admission path); (3) perform the capacity check (a same-key hit consumes no new
capacity) counting `Initializing`, `Closing`, and `Active` alike, because a slot
frees only at `Drained`/`Failed` or via the no-resource early path (3.2); (4) reuse
the existing `Active` entry, join the existing `Initializing` one, return the typed
pre-send result for `Closing` (4.3), or install a new `Initializing` reservation with
a fresh generation identity, so a concurrent admission for another key observes the
reserved or occupied slot.

The owner map guard is `std::sync::Mutex` (or equivalent non-async lock), dropped before any connect, stream, driver, or close await. Explicitly non-conforming: a check-then-insert split across two lock acquisitions; releasing the lock between the capacity check and the placeholder insert; a per-key lock leaving the global count racy; computing the count from a snapshot then inserting; holding the map lock across an `await`; and publishing `Active` without re-checking owner and generation state under the same lock (3.1).

Deterministic test (Slice 0, no socket): spawn many concurrent admissions for distinct keys against a barrier, observing the entry count under the same lock. Assert live entries never exceed `MAX_CONNECTIONS_PER_OWNER`, the admitted-key count reaches the cap exactly, every excess admission gets the typed capacity result, and the same-key reservation is not double-counted. Cover an entry idle-expired or closed concurrently: while `Initializing` or `Closing` it still occupies its slot, so a new key is admitted only at `Drained`/`Failed` or the no-resource early release. Fail if the section splits, the lock drops before the placeholder insert, or a `Closing` slot is reused before its drain completes.

### 4.2 Idle expiry

Idle expiry is lazy: the admission/maintenance scan checks the injected clock under the owner-map lock (no background reaper). An expired entry transitions to `Closing` in that same lock section, not removed, staying discoverable until `Drained` or `Failed` (7.1).

The **admission path never awaits a drain**: a scan that expires an entry starts (or joins) the supervised teardown (7.2) and returns, never blocking admission on any entry's completion. This preserves the 4.3 guarantees — a same-key `Closing` lookup returns `Closed(NotSent)` immediately, and `Initializing` joining is the only same-key wait. The explicit maintenance method may **optionally await** the shared completion of the entries it just expired so a test or shutdown path can deterministically observe drain; that await is confined to maintenance.

### 4.3 Same-key lookup by entry state

A lookup that finds the requested key present resolves by state, under the admission
lock, with no unbounded internal wait:

| Entry state found | Result for this caller |
| --- | --- |
| `Active` | Lease it and continue the normal exchange path. |
| `Initializing` | Join the entry's single-flight initializer, bounded by this caller's original absolute deadline and `ExchangeControl`. If initialization publishes `Active` while the owner is still `Open`, proceed; if close/teardown wins first, return the typed pre-send closed result. |
| `Closing` | Return one existing typed pre-send closed result (below). |
| `Drained` / `Failed` | The entry is removed at that terminal; a concurrent lookup still sees the terminal state (return the typed pre-send closed result) or, after removal, admits a fresh generation. |

The `Closing` result reuses the repository's existing closed vocabulary, `UpstreamError::Closed(SideEffectState::NotSent)` — one already-defined typed result, not a new error kind. It means: no request byte was sent and no stream opened, so `NotSent` is exact; the caller does **not** wait on the closing entry's internal teardown, so a `Closing` lookup cannot block behind an unbounded drain; the caller opens **no** second generation for that key and does **not** reuse the `Closing` slots; and the caller may retry later, when a fresh generation may be admitted after the old entry's terminal removal.

`Initializing` joining is the only same-key wait, always under the caller's own
deadline/control race, never an internal queue; the join returns the same
`Closed(NotSent)` result if close wins (3.1).

Deterministic same-key `Closing` model test (Slice 0, no socket): hold a key in `Closing` behind a teardown barrier and issue several same-key admissions concurrently. Assert each returns exactly `Closed(NotSent)` without awaiting drain, no second generation is created, the `Closing` slot is not leased, and after the barrier releases and the entry reaches `Drained`/`Failed` a later admission can admit a fresh generation. Fail if a lookup opens a duplicate generation, reuses a `Closing` slot, or blocks on the drain.

## 5. DoQ exchange flow

For a leased DoQ entry: (1) check owner/caller/deadline with `SideEffectState::NotSent`; (2) open one fresh bidirectional stream on the shared connection; (3) copy the caller query, zero its ID, and write it with the existing two-byte stream framing helper; (4) finish the request side, stopping only that stream if local cancellation wins; (5) read exactly one framed response through the peer FIN, validate peer wire ID zero, restore the original caller ID, and run the existing DNS response validator; (6) apply `Lifecycle::commit_final_response` with the original caller cancellation/deadline before releasing the permit; (7) return the connection to the entry if healthy. A stream reset, malformed response, or local cancellation is stream-local; a QUIC connection error or closed handle is connection-terminal and logically deactivates the entry `Active -> Closing` by exact key+generation, immediately unleasable, with map removal only at `Drained`/`Failed` (7.4). A failed query is never replayed: a replacement is available only to a later independent exchange, once the old generation reached `Drained`/`Failed` (or the 3.2 early release for an initialization that never acquired a resource) so its slot is free.

## 6. DoH3 exchange and long-lived driver

Connection setup is one-time per entry: (1) build the exact `TlsPolicy`-derived client config offering only `h3`; (2) open the numeric QUIC connection using the endpoint's separate identity; (3) build H3 once, creating control/QPACK streams and returning the driver plus cloneable `SendRequest`; (4) register the driver as an owned child and hand its `JoinHandle` to the supervised teardown task (7.2) before the entry publishes `Active`, keeping no droppable copy; (5) spawn it on the caller's Tokio runtime, looping `poll_close` to normal or terminal completion and publishing health without detaching. If teardown was requested during setup (3.1), the driver is never published `Active` and the supervised teardown closes it.

Per request: (1) clone the H3 sender template and build the existing DoH GET target from `DohEndpoint::get_request_target`; (2) open one H3 request stream, send exactly the existing headers, and finish the request side; (3) read/validate with the existing bounded DoH validator and restore only that request's original ID; (4) race every phase against the original control/deadline; (5) commit through the shared lifecycle gate, then release only that request's permit.

A query cancellation resets/stops only its request stream **as permitted by the R0a
per-phase contract in 0.1**. If cancellation wins while h3-quinn owns an internal
read future, the receive side is in the pinned `None`-option hazard state, and the
implementation must not call `RequestStream::stop_sending`, which unwraps that
option and panics. For such a phase the contract is drop-only teardown: return the
unchanged typed control error and drop the `RequestStream`, letting Quinn's
`RecvStream::Drop` stop unread receive data. Slice 0 proves this at the
decision/state-model level only; the real pinned-stack loopback proof belongs to
Slice 2/A5 (0.1). This is a stream-local requirement, not permission to close the
connection, and R0a must close the decision model before Slice 1.

The H3 driver is not reused from `H2ScopeLease` (which assumes one exchange and can seal/abort at response completion); it must stay alive across requests, and a dedicated QUIC driver handle must make admission, shutdown, and drain idempotent.

## 7. Close, logical deactivation, and final commit

### 7.1 Entry lifecycle

Every `QuicConnectionEntry` exposes an explicit, observable lifecycle:

```text
Initializing ──publish──▶ Active ──close/deactivate──▶ Closing ──drain complete──▶ Drained
    │                                                     └─drain terminal──▶ Failed
    ├──close observed, resource acquired──▶ Closing (supervised teardown)
    └──close observed / fail, no resource──▶ released at once, no Closing state
```

- **Initializing** — a reserved placeholder for one key+generation with one initializer and no leases; entered under the owner-map lock in the admission section (4.1) and described in section 3.
- **Active** — admitted and leasable; reachable only from `Initializing` via the 3.1 publication rule.
- **Closing** — admission stopped, immediately unleasable, and **still in the owner map and discoverable** until `Drained` or `Failed`, so a concurrent close caller or maintenance pass joins the in-progress teardown rather than racing a vanished entry; removal happens only at `Drained`/`Failed` (3.2, 7.2).
- **Drained** — the H3 driver stopped, all request streams ended, the QUIC connection/endpoint handle was released, and the `Lifecycle` liveness registration was dropped.
- **Failed** — the drain could not complete gracefully and the entry was made terminal by the explicit bounded force path (7.2); it releases the liveness registration and never leaves a detached driver.

`Drained` and `Failed` are the only states from which a served entry is removed from
the map; both are idempotent terminals. The one other removal path is the
`Initializing` failure that never acquired a transport/H3 resource (3.2).

### 7.2 Entry-owned supervised teardown

When an entry transitions to `Closing`, an **entry-owned supervised teardown task starts exactly once**. That task — not any close caller — owns the H3 driver task and its `JoinHandle`, the driver's shutdown signal, the entry's `Lifecycle` liveness guard, and the shared completion primitive reporting `Drained`/`Failed`. All close paths — owner close, idle expiry, connection-level logical deactivation, protocol-terminal failure, and initialization that acquired a resource then observed close — converge on this one task and completion:

- **Exactly one task.** The `Closing` transition starts it once by generation identity; a second transition returns the same completion and never starts a second teardown.
- **Callers only await.** Every close caller — concurrent `close()` callers, the owner close path, the maintenance pass — awaits the shared completion; none drives, polls, or performs teardown work itself, so no caller starts a competing teardown or observes a half-finished state.
- **Abort cannot stop progress.** Dropping the first close waiter's future, or **all** close waiter futures, drops only their awaits. The supervised task continues independently because the entry owns it and it holds the driver/`JoinHandle`, shutdown signal, and liveness guard — so teardown makes progress with **no surviving caller** and never depends on a later close/drain pass to be re-driven.
- **Liveness held to terminal.** The task holds the `Lifecycle` registration until `Drained`/`Failed`; an aborted or vanished caller cannot make `Lifecycle` report drained while the driver runs.
- **Terminal-only removal.** The task performs the map removal itself, only at `Drained`/`Failed`; no other code path removes a served entry.
- **Bounded force path.** If the peer never cooperates, the task uses local stream cancellation and an explicit QUIC close code, then reaches `Failed` through the same completion; the owner never waits forever.

Teardown is idempotent: replaying the completion returns the recorded outcome without re-running shutdown, re-closing the QUIC connection, or double-releasing a guard.

### 7.3 Owner close sequence

1. `begin_close` transitions the lifecycle and cancels the owner token, atomically rejecting new exchanges.
2. Under the owner map lock, **every `Initializing` and `Active` entry transitions to `Closing`** but **is not removed**; that one lock/state linearization point both blocks `Initializing -> Active` publication (3.1) and starts each entry's supervised teardown task (7.2), and no exchange can reacquire a `Closing` entry because the lease path rejects non-`Active` entries.
3. The owner close path awaits each `Closing` entry's shared completion — the same one every other caller awaits. At `Drained`/`Failed` the supervised task has already removed the entry and dropped its liveness guard, and all exchange registrations then drain through `Lifecycle` before `finish_close`.

If an entry is already `Drained`/`Failed`, close is idempotent and only waits for
the owner-level registrations.

### 7.4 Logical deactivation and final commit

A served or `Active` entry that hits a connection-level failure is **logically deactivated, not immediately removed**: under the map/state lock it transitions `Active -> Closing` by exact key+generation, making it immediately unleasable, then takes the same supervised teardown as every other close path; physical map removal happens only at `Drained`/`Failed` (7.2). A stale failure callback cannot touch a newer replacement, and stream-local protocol errors do not transition the entry unless connection/driver health also proves terminal, per the R0b table (0.2, section 9).

The final response commit occurs before stream permit release, so a successful
commit remains a success even if close starts immediately after it; close, cancel,
or deadline winning before the commit returns a typed error; and no entry returns to
the pool before the commit decision is complete.

## 8. Resolver composition

Use the existing resolver snapshot read-only: DoQ uses `ResolverComposition::doq_endpoint(PublishedTarget, ServerIdentity)`; DoH3 uses `ResolverComposition::doh_endpoint(PublishedTarget, service_url)` and the existing `DohEndpoint`. The owner builds a key from the resulting endpoint and policy, so a selected A or AAAA address produces a distinct key without copying resolver state or changing the authenticated service identity. Resolver refresh does not proactively close an established connection: the old key stays valid until idle/dead/owner teardown and a new selected target creates or reuses its own key. No resolver algorithm, fallback, cross-family race, or socket policy is added.

## 9. Error and state matrix

One explicit row per behavior; **Entry** comes from the R0b table (0.2), not each call site. `NotSent`/`Sent`/`MaybeSent` is independent of the deactivation class: stream-local can be `Sent` (request written) and connection-terminal can be `NotSent` (no handshake).

| Behavior | Exchange | Entry | Reuse |
| --- | --- | --- | --- |
| invalid key/zero port | pre-I/O error, `NotSent` | none | no entry |
| local slots full | backpressure, `NotSent` | healthy | keep |
| init failure, no resource | connect/TLS, `NotSent` | released now by key+gen | replace later |
| init failure, resource exists | connect/TLS, `NotSent` | `Initializing -> Closing -> Drained/Failed` | after terminal |
| init close, no resource | `Closed(NotSent)` | released now by key+gen; never `Active` | unavailable now |
| init close, resource exists | `Closed(NotSent)` | `Initializing -> Closing`, then teardown | after terminal |
| same-key `Initializing` | joins single-flight under caller deadline; `Closed(NotSent)` if close wins | unchanged | one entry, no 2nd gen |
| same-key `Closing` | `Closed(NotSent)`, no drain wait | `Closing`, slot unlent | retry after terminal removal |
| stream open/send uncertainty | send/connect error | keep unless terminal | no replay |
| peer reset / malformed response | DoQ/DoH3 terminal error, usually `Sent` | keep if healthy | reusable |
| caller cancel / deadline | existing control error | keep if healthy | reusable |
| owner close | `Closed` with side-effect state | `Initializing`/`Active -> Closing`, drain | unavailable |
| connection/driver terminal failure | connection error | logical `Active -> Closing`; unleasable; removal only at `Drained`/`Failed` | after terminal |
| final commit | candidate until commit | keep after permit release | reusable after commit |

Preserve the closed side-effect vocabulary and never turn stream-local failure into an implicit retry or cross-protocol fallback; an unclassified error is a Slice 0 gap blocking Slice 1, not a runtime guess.

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT, session resumption, connection migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks, interface/source binding, UDP retransmission, listeners, config, plugins, sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or general-purpose pool abstraction enters this task. The QUIC/H3 graph stays locked: any dependency change is a Slice 0 blocker requiring revised plan/review, and the same rule covers R0a/R0b/R0c — a pinned API that cannot satisfy the contract stops the task for re-review and is never worked around by a dependency change. Rollback is a task-scoped revert of the new QUIC reuse module, exports, tests, and any narrowly required lifecycle/error additions; existing one-shot QUIC, secure, resolver, and generic TCP reuse paths are unchanged, and unrelated dirty files are not staged or reverted.

## 11. Test architecture

- Slice 0: pure key/limit/entry-state model tests with no socket or QUIC I/O, covering multi-key cap (4.1), same-key `Closing` (4.3), init-vs-owner-close (3.1), supervised teardown (7.2/11.1), and four-phase R0a (0.1). All fail if their contract is removed; R0a here is model evidence only, not real H3 loopback behavior.
- Slice 1: in-process DoQ fixtures plus accept counting, concurrent stream markers, one-stream cancellation, and dead-connection replacement.
- Slice 2: H3 fixture counting one connection and multiple request stream IDs, hold/release responses, driver liveness, and request cancellation without killing another stream (incl. R0a drop-only phases), with the real pinned-stack proof owned by Slice 2/A5.
- Slice 3: peer stream-budget fixtures, bound/backpressure, idle-clock, concurrent close, resolver A/AAAA-to-key, and bounded stress.
- Every slice asserts connection, stream/request, lifecycle counts, final commit, and no late success at the boundary exercised.

### 11.1 Aborted-at-barrier/no-surviving-caller supervised-teardown test

Slice 0 executable form of 7.2 (no socket):

1. Build an entry with a deterministic teardown **barrier** the task must pass before `Drained`.
2. Spawn concurrent `close()` waiters for that owner/entry.
3. Abort the **first** waiter's future while parked at the barrier.
4. At the barrier assert the entry is still **discoverable** as `Closing`, the task still runs and still **owns** driver/`JoinHandle`, shutdown signal, and liveness guard, and no completion is reported.
5. Drop **all** remaining waiters, leaving no surviving caller; assert the task still reaches the terminal state **autonomously**, never re-driven by a later close/drain pass.
6. Release the barrier and assert **exactly one** teardown ran, completion reports `Drained`/`Failed`, removal happened inside the task only at that terminal, liveness reached zero, and no late response committed.

Fails if teardown moves into a caller, a caller owns the driver handle or shutdown signal, teardown stalls once all callers are gone, or a served entry is removed in any state other than `Drained`/`Failed`.
