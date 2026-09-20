# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only; no code before the summary is approved and `task.py start` runs.

## 0. Pre-start gates (R0, blocking)

R0 blocks Slice 1 and is part of Slice 0's exit criteria.

### 0.1 R0a — H3 request cancellation is core

Define the per-phase local cancellation contract for one H3 stream against the pinned `h3 0.0.8` / `h3-quinn 0.0.10` API for four phases: (1) **before
send** — no request byte exists, the stream is simply dropped; (2) **after request
FIN** — the request is complete, only the receive side is torn down; (3) **during
response head** — a read future in the pinned h3-quinn layer may already hold the
internal receive stream; (4) **during body read** — same hazard as (3), on the data
path.

Pinned hazard: `h3_quinn::RecvStream::poll_data` takes the `Option<quinn::RecvStream>` into its in-flight `read_chunk_fut`, restoring it only when that future completes; a local control decision drops it, leaving `None`, and a later `h3::client::RequestStream::stop_sending` reaches `self.stream.as_mut().unwrap()` and panics. R0a therefore decides per phase whether an active stop is safe or the contract is drop-only; where unsafe, return the unchanged typed control error and drop the `RequestStream`, letting Quinn's `RecvStream::Drop` stop unread data (deliberate and tested).

**Slice 0 proves the decision/state model, not the pinned stack.** It has no socket or QUIC/H3 I/O; its four-phase test asserts only that (a) the model never selects the pinned `Option::None` `stop_sending` path, (b) the transition is per-phase and deterministic, and (c) the logical shared-entry state stays healthy. **The real pinned-stack H3 proof belongs to Slice 2/A5**, which owns the `h3`/`h3-quinn` loopback fixture proving canceling one real H3 request leaves the real connection, driver, and another concurrent request healthy; Slice 0 must not present model evidence as real H3 evidence. A dependency change is never the remedy: a phase the pinned API cannot satisfy is drop-only and stops the task for re-review.

### 0.2 R0b — connection-level versus stream-level error classification is core

Produce one authoritative table over the pinned error vocabulary; every later slice consumes it. Each error is exactly one class: **stream-local** (query fails; entry stays healthy and reusable) or **entry-terminal** (exact key+generation logically deactivated by `Active -> Closing`, immediately unleasable, removal only at `Drained`/`Failed`, no query replayed).

Derived from the pinned sources, not a `Result` guess, it covers at least DoQ stream reset/stop, DoQ framing/response-validation failures, DoQ `open_bi`/write/read failures, H3 request stream errors, H3 driver `poll_close` terminal outcomes, and `quinn::Connection` `closed`/`close` outcomes. A `SendRequest`-level failure does not prove the connection dead, nor a stream protocol error that it is healthy. R0b also records `NotSent`/`Sent`/`MaybeSent` for `SideEffectState`, independent of the deactivation class.

### 0.3 R0c — pinned API assumptions are frozen

Every fact in `research/quic-reuse-evidence.md` for `quinn 0.11.7`, `h3 0.0.8`, `h3-quinn 0.0.10` is verified in R0 against the locked local registry source, pinned to an exact file/line range with a holds/does-not-hold result. The graph stays locked; an assumption that does not hold stops for re-review, and dependency add/remove/bump is forbidden.

### 0.4 R0 exit criteria

R0 closes when the four-phase model is documented and exercised (not by real H3 I/O), the classification table is complete and referenced by the DoQ/H3 sections, the pinned assumptions are verified with locked-local-registry citations, and the diff has no dependency change. It does **not** close the real pinned-stack H3 health proof (Slice 2/A5).

## 1. Boundary and ownership

The deliverable stays in `rust/upstream-core`. The one-shot `DoqUpstream`/`Doh3Upstream` remain valid fresh-connection primitives and regression coverage; the new code is a sibling QUIC-specific owner (`src/quic_reuse.rs`) with only the minimum re-exports for future Rust-native composition. It must not turn the serial `ReuseOwner` into a cross-protocol pool: the shared vocabulary is reused, not the old storage/lifecycle implementation:

- `Lifecycle` remains the only owner admission, close, in-flight registration, and final commit gate.
- `ExchangeContext`/`ExchangeControl` remain the only absolute deadline and caller/owner cancellation race.
- `ServerIdentity`, `TlsPolicy`, `PublishedTarget`, `DohEndpoint::get_request_target` remain authoritative.
- `tcp::write_frame` and the existing DNS validators are reused; no second framing or DoH target/response contract.

## 2. Validated key

`QuicProtocol::{Doq, Doh3}` is a closed enum fixing ALPN to `doq` or `h3`; callers cannot pass arbitrary ALPN bytes. `QuicReuseKey` holds the numeric `SocketAddr`, protocol/ALPN discriminator, canonical identity text, TLS verification mode, TLS roots revision, and optional DoH3 authority. Constructors such as `from_doq(&DoqEndpoint, &TlsPolicy)` and `from_doh3(&DohEndpoint, &TlsPolicy)` accept only validated endpoints/policies and add no semantics. DoQ has no HTTP authority; DoH3 authority is keyed because `DohEndpoint` separates origin authority from the numeric dial and a connection must not silently serve a different origin. TLS key material is never copied into the key: mode plus the opaque `TlsPolicy::roots_revision` identifies trust; cloning preserves the revision and a new verified policy intentionally prevents reuse with old roots. A resolver refresh or A/AAAA change only changes the numeric dial field, never identity, authority, or an established connection.

Key tests are pure and deterministic, covering every isolation dimension including DoQ versus DoH3 on one numeric address and two equal verified policies with distinct roots revisions.

## 3. Owner and connection-entry state

`QuicReuseOwner`: `Arc<Lifecycle>` + owner `TransportCancellation`; a short-lock map `QuicReuseKey -> Arc<QuicConnectionEntry>`; task-local limits + injected clock/maintenance seam; no generic TCP pool or hidden config selector.

Each `QuicConnectionEntry` owns one physical QUIC connection per key, a stream-slot semaphore, a last-used timestamp, a generation/health state, the endpoint handle keeping Quinn's UDP driver alive, and a `Lifecycle::register_owned` liveness guard from insertion to terminal removal. Payload: DoQ stores the cloned `quinn::Connection` and calls `open_bi` per attempt without sharing stream halves; DoH3 stores the H3 sender template plus a long-lived driver handle, each query cloning the sender (pinned `h3 0.0.8` is cloneable) while the driver task continuously polls the H3 `Connection`. A driver terminal error marks the entry dead and does not silently replace it for the request using it.

Connection creation is single-flight per key: the admission section installs a **reserved placeholder** under the owner-map lock before any async connect/build, so the map never holds a half-defined entry and observers see `Initializing` (key+generation fixed, one initializer, no leases), `Active` (transport published, leasable), or `Closing`/`Drained`/`Failed` (7.1). A `OnceCell`-equivalent entry-local state makes concurrent first users join the same initializer.

The `Initializing -> Active` publication and the owner/entry close decision share one linearization point: the owner-map/state lock the admission section and `begin_close` use. The initializer may publish `Active` only when, under that lock, the owner is still `Open` **and this exact key+generation is still `Initializing`**; otherwise it observes `Closing`/`TeardownRequested` and must never publish `Active`, so the entry is never both `Active` and `Closing`. Initialization outcomes are classified by resource acquisition, not error type. Sections 3.1, 3.2, 7.1, 7.2, 9, and PRD R6 all describe this one protocol; no second rule exists.

### 3.1 The single initialization crossing protocol

Owner close, idle expiry, and initialization share one state linearization point:

- Under the owner-map lock, `begin_close` (or idle expiry / connection-level logical deactivation) marks **every `Initializing` and `Active` entry `Closing`/`TeardownRequested`**; the `Initializing` reservation/generation is **not** removed, and `Closing` stays discoverable but unleasable. Entering `Closing` starts the entry-owned supervised teardown task exactly once (7.2), which supervises the initializer handoff and owns driver/`JoinHandle`, shutdown signal, liveness, and shared completion — no close-waiter abort can stop it.
- The initializer builds resources **outside** the lock, then hands the result to the entry-owned teardown/owner state under the **same** lock/handoff protocol: owner `Open` and generation `Initializing` → publish `Active` (the only publication point); already `Closing`/`TeardownRequested` → **never** publish `Active` nor leave a resource in a removed generation, handing the whole result — including a late-acquired resource — to the supervised teardown, with a failed initializer still delivering its completion so teardown finishes promptly on one terminal outcome.
- Only the terminal `Drained`/`Failed` performs exact key+generation removal and releases slot+liveness: no late-resource race, early drain, stranded `Closing`, second generation, or reliance on a later close/drain pass.
- Same-key `Closing` keeps its semantics: `UpstreamError::Closed(SideEffectState::NotSent)`, no drain wait, no second generation (4.3).

### 3.2 Initializer completion and handoff outcomes

The initializer reports exactly one completion under the same lock/handoff protocol:

- **Published** — owner `Open`, generation `Initializing`: transport became `Active`, serving leases.
- **No resource** (failed, or close won before any handle existed) — teardown records terminal `Failed` and removes the entry exactly once, nothing to drain.
- **Close won, resource acquired** — a handle existed, even one built after teardown started: handed to the supervised teardown and `Drained` after cleanup; it belongs to its creating generation and is never released early or orphaned.

No async wait occurs while the owner map lock is held; all network, stream, driver, and close awaits happen after the relevant entry or snapshot is acquired.

## 4. Bounds and admission

Task-local non-configurable constants: `MAX_STREAMS_PER_CONNECTION = 32`; `MAX_CONNECTIONS_PER_OWNER = 8`; `QUIC_IDLE_TIMEOUT = 30s` (lazy maintenance, not a background timer); one physical connection per key.

Admission sequence:

1. Register the exchange through the owner's `Arc<Lifecycle>`, rejecting `Closed(NotSent)` before any network operation.
2. Enter the **atomic admission section** (4.1): look up the key, transition idle-expired entries to `Closing` (discoverable until `Drained`/`Failed`, per 4.2 and 7.1), check capacity, and either reuse an `Active` entry or install an `Initializing` reservation.
3. Resolve the lookup by state (4.3): `Active` used directly, `Initializing` joined via single-flight, `Closing` returns the typed pre-send closed result.
4. Take a stream permit with a non-queueing operation; if local permits are exhausted, return a typed `Backpressure` error with `NotSent`, never growing an owner queue or opening a second same-key connection.
5. Race QUIC/H3 stream opening against the exchange's original `ExchangeControl` and absolute deadline; if the peer exhausted its advertised stream credit, the open future may stay pending only under this bounded caller-owned race.
6. On every exit, release the permit exactly once and update idle time only when the entry has no active permits and stays healthy.

### 4.1 Atomic multi-key admission

`MAX_CONNECTIONS_PER_OWNER = 8` is a cross-key invariant enforced by one owner-map critical section with **no `await` inside it**: (1) lookup the key; (2) mark dead/idle-expired entries `Closing` here (no removal — discoverable until `Drained`/`Failed`, per 4.2/7.1 — and the transition starts supervised teardown with no drain awaited on the admission path); (3) capacity-check (a same-key hit consumes no capacity) counting `Initializing`/`Closing`/`Active` alike, since a slot frees only at terminal `Drained`/`Failed` (3.1, 3.2); (4) reuse `Active`, join `Initializing`, return the typed pre-send result for `Closing` (4.3), or install a fresh-generation `Initializing` reservation, so a concurrent admission for another key sees the reserved/occupied slot.

The owner map guard is `std::sync::Mutex` (or equivalent non-async lock), dropped before any connect/stream/driver/close await. Non-conforming: a check-then-insert split across two lock acquisitions; releasing the lock between capacity check and placeholder insert; a per-key lock leaving the count racy; counting from a snapshot then inserting; holding the lock across an `await`; publishing `Active` without re-checking owner and generation state under the same lock (3.1); or removing a `Closing` reservation before terminal (3.1).

Deterministic test (Slice 0, no socket): spawn many concurrent admissions for distinct keys against a barrier, observing the entry count under the same lock. Assert live entries never exceed `MAX_CONNECTIONS_PER_OWNER`, the admitted-key count reaches the cap exactly, every excess admission gets the typed capacity error, and the same-key reservation is not double-counted. Cover an entry idle-expired/closed concurrently: while `Initializing` or `Closing` it still occupies its slot, so a new key is admitted only at `Drained`/`Failed`. Fail if the section splits, the lock drops before the placeholder insert, or a `Closing` slot is reused before terminal removal.

### 4.2 Idle expiry

Idle expiry is lazy: the admission/maintenance scan checks the injected clock under the owner-map lock (no reaper). An expired entry is marked `Closing` in that same lock section, not removed, staying discoverable until `Drained`/`Failed` (7.1).

The **admission path never awaits a drain**: a scan that expires an entry starts (or joins) the supervised teardown (7.2) and returns, never blocking admission on any entry's completion. This preserves the 4.3 guarantees — a same-key `Closing` lookup returns `Closed(NotSent)` immediately, and `Initializing` joining is the only same-key wait. The explicit maintenance method may **optionally await** the shared completion of the entries it just expired so a test or shutdown path can deterministically observe the terminal outcome; that await is confined to maintenance.

### 4.3 Same-key lookup by entry state

A lookup that finds the key present resolves by state under the admission lock, with no unbounded internal wait:

| Entry state found | Result for this caller |
| --- | --- |
| `Active` | Lease it and continue the normal exchange path. |
| `Initializing` | Join the single-flight initializer under this caller's absolute deadline/`ExchangeControl`; proceed if it publishes `Active` while the owner is `Open`, else return the typed pre-send closed result. |
| `Closing` | Return one existing typed pre-send closed result (below). |
| `Drained` / `Failed` | Removed at that terminal; a concurrent lookup still sees the terminal state (typed pre-send closed result) or, after removal, admits a fresh generation. |

The `Closing` result reuses the repository's existing closed vocabulary, `UpstreamError::Closed(SideEffectState::NotSent)` — one already-defined typed result, not a new error kind: no request byte was sent and no stream opened, so `NotSent` is exact; the caller does **not** wait on the closing entry's internal teardown (no unbounded drain); it opens **no** second generation for that key and does **not** reuse the `Closing` slots; and it may retry later, when a fresh generation may be admitted after the old entry's terminal removal.

`Initializing` joining is the only same-key wait, always under the caller's deadline/control race, never an internal queue, returning the same `Closed(NotSent)` if close wins (3.1).

Deterministic same-key `Closing` model test (Slice 0, no socket): hold a key in `Closing` behind a teardown barrier and issue several same-key admissions concurrently. Assert each returns `Closed(NotSent)` without awaiting drain, no second generation is created, the `Closing` slot is not leased, and after the barrier releases and the entry reaches `Drained`/`Failed` a later admission can admit a fresh generation. Fail if a lookup opens a duplicate generation, reuses a `Closing` slot, or blocks on the drain.

## 5. DoQ exchange flow

For a leased DoQ entry: (1) check owner/caller/deadline as `NotSent`; (2) open one fresh bidirectional stream on the shared connection; (3) copy the caller query, zero its ID, and write it with the existing two-byte framing helper; (4) finish the request side, stopping only that stream on local cancellation; (5) read exactly one framed response through the peer FIN, validate wire ID zero, restore the caller ID, and run the existing DNS validator; (6) apply `Lifecycle::commit_final_response` with the original cancellation/deadline before releasing the permit; (7) return the connection if healthy. A stream reset, malformed response, or local cancellation is stream-local; a QUIC connection error or closed handle is connection-terminal and logically deactivates the entry `Active -> Closing` by exact key+generation, immediately unleasable, with map removal only at `Drained`/`Failed` (7.4). A failed query is never replayed: a replacement is available only to a later independent exchange, once the old generation reached terminal `Drained`/`Failed` so its slot is free.

## 6. DoH3 exchange and long-lived driver

One-time setup per entry: (1) build the `TlsPolicy`-derived config offering only `h3`; (2) open the numeric QUIC connection using the endpoint's separate identity; (3) build H3 once (control/QPACK streams, driver, cloneable `SendRequest`); (4) hand the driver `JoinHandle` to the supervised teardown task (7.2) with no droppable copy; (5) spawn it on the caller's runtime, looping `poll_close` to normal/terminal completion. If teardown was requested during setup (3.1), the driver is never published `Active` and the supervised task closes it.

Per request: (1) clone the H3 sender template and build the existing DoH GET target from `DohEndpoint::get_request_target`; (2) open one H3 request stream, send exactly the existing headers, and finish the request side; (3) read/validate with the existing bounded DoH validator and restore only that request's original ID; (4) race every phase against the original control/deadline; (5) commit through the shared lifecycle gate, then release only that request's permit.

A query cancellation resets/stops only its request stream **as permitted by the R0a per-phase contract in 0.1**. If cancellation wins while h3-quinn owns an internal read future, the receive side is in the pinned `None`-option hazard state and the implementation must not call `RequestStream::stop_sending`, which unwraps that option and panics; for such a phase the contract is drop-only teardown — return the unchanged typed control error and drop the `RequestStream`, letting Quinn's `RecvStream::Drop` stop unread data. Slice 0 proves this at the decision/state-model level only; the real pinned-stack loopback proof belongs to Slice 2/A5 (0.1). This is stream-local, not permission to close the connection, and R0a must close the decision model before Slice 1.

The H3 driver is not reused from `H2ScopeLease` (which assumes one exchange and can seal/abort at response completion); it must stay alive across requests, and a dedicated QUIC driver handle must make admission, shutdown, and drain idempotent.

## 7. Close, logical deactivation, and final commit

### 7.1 Entry lifecycle

Every `QuicConnectionEntry` exposes an explicit, observable lifecycle:

```text
Initializing --publish (owner Open, gen Initializing)--> Active
     |                                                     |
     +-- close/expiry: mark Closing/TeardownRequested -----+--> Closing
                                                                 | supervised teardown awaits
                                                                 | initializer handoff
                                                                 v
                                          Drained (cleanup) / Failed (nothing to drain)
                                                                 | terminal only
                                                                 v
                                       exact key+gen removal, slot+liveness release
```

- **Initializing** — reserved placeholder for one key+generation, one initializer, no leases; entered under the owner-map lock in the admission section (4.1), described in section 3.
- **Active** — admitted and leasable; reachable only from `Initializing` via the 3.1 publication rule.
- **Closing** — admission stopped, unleasable, and **still in the owner map and discoverable** until `Drained`/`Failed`, so a close caller or maintenance pass joins the in-progress teardown instead of racing a vanished entry; a `Closing` reservation is never deleted early (3.1, 3.2, 7.2).
- **Drained** — the supervised teardown observed the initializer handoff and the H3 driver stopped, all request streams ended, the QUIC connection/endpoint handle was released, and the `Lifecycle` liveness registration was dropped.
- **Failed** — teardown could not complete gracefully, or there was nothing to drain; the entry became terminal via the explicit bounded force path (7.2), releasing liveness and never leaving a detached driver.

`Drained`/`Failed` are the only terminal outcomes and the only removal point for the exact key+generation, releasing slot+liveness; no other removal path exists (3.1, 3.2).

### 7.2 Entry-owned supervised teardown

When an entry transitions to `Closing`, an **entry-owned supervised teardown task starts exactly once**. That task — not any close caller — owns the initializer completion/handoff, the H3 driver task and its `JoinHandle`, the driver's shutdown signal, the entry's `Lifecycle` liveness guard, and the shared completion primitive reporting `Drained`/`Failed`. All close paths — owner close, idle expiry, connection-level logical deactivation, protocol-terminal failure, and initialization that acquired a resource then observed close — converge on this one task and completion:

- **Exactly one task.** The `Closing` transition starts it once by generation identity; a second transition returns the same completion and never starts another.
- **Initializer handoff is supervised.** The task awaits the initializer's single completion latch and closes any resource it built — including one built *after* teardown was requested — under the state lock, so no resource is left in a removed generation and no `Active` is published once close won.
- **Callers only await.** Concurrent `close()` callers, the owner close path, and the maintenance pass all await the shared completion; none drives, polls, or performs teardown work, so none starts a competing teardown or observes a half-finished state.
- **Abort cannot stop progress.** Dropping the first close waiter, or **all** waiters, drops only their awaits; the task continues because the entry owns it and it holds the initializer handoff, driver/`JoinHandle`, shutdown signal, and liveness guard. Teardown progresses with **no surviving caller** and never depends on a later close/drain pass.
- **Liveness held to terminal.** The task holds the `Lifecycle` registration until `Drained`/`Failed`, so a vanished caller cannot make `Lifecycle` drain while the driver runs or the handoff is outstanding.
- **Terminal-only removal.** The task alone performs map removal and slot/liveness release, only at `Drained`/`Failed`; no other path removes a served or `Closing` entry.
- **Bounded force path.** If the peer never cooperates, the task uses local stream cancellation and an explicit QUIC close code, then reaches `Failed` through the same completion.

Teardown is idempotent: replaying the completion returns the recorded outcome without re-running shutdown, re-closing the QUIC connection, or double-releasing a guard.

### 7.3 Owner close sequence

1. `begin_close` transitions the lifecycle and cancels the owner token, atomically rejecting new exchanges.
2. Under the owner map lock, **every `Initializing` and `Active` entry is marked `Closing`/`TeardownRequested`** but **is not removed**: that one lock/state linearization point blocks `Initializing -> Active` publication (3.1) and starts each entry's supervised teardown task (7.2), and no exchange can lease a `Closing` entry. A `Closing` reservation stays discoverable.
3. The supervised task awaits the initializer handoff (closing any late-acquired resource), then the owner close path awaits each `Closing` entry's shared completion, as every other caller does. Only at `Drained`/`Failed` does the task remove the entry, release its slot, and drop its liveness guard; only then do all exchanges drain through `Lifecycle` before `finish_close`.

If an entry is already `Drained`/`Failed`, close is idempotent and only waits for
the owner-level registrations.

### 7.4 Logical deactivation and final commit

A served or `Active` entry hitting a connection-level failure is **logically deactivated, not immediately removed**: under the map/state lock it transitions `Active -> Closing` by exact key+generation, immediately unleasable, then takes the same supervised teardown as every other close path; physical removal happens only at `Drained`/`Failed` (7.2). A stale failure callback cannot touch a newer replacement, and stream-local protocol errors do not transition the entry unless connection/driver health also proves terminal, per the R0b table (0.2, section 9).

The final response commit occurs before stream permit release: a successful commit stays a success even if close starts immediately after; close/cancel/deadline winning before commit returns a typed error; and no entry returns to the pool before the commit decision completes.

## 8. Resolver composition

Use the existing resolver snapshot read-only: DoQ uses `ResolverComposition::doq_endpoint(PublishedTarget, ServerIdentity)`; DoH3 uses `ResolverComposition::doh_endpoint(PublishedTarget, service_url)` and the existing `DohEndpoint`. The owner builds a key from the resulting endpoint and policy, so a selected A or AAAA address produces a distinct key without copying resolver state or changing the authenticated service identity. Resolver refresh does not proactively close an established connection: the old key stays valid until idle/dead/owner teardown and a new selected target creates or reuses its own key. No resolver algorithm, fallback, cross-family race, or socket policy is added.

## 9. Error and state matrix

One row per behavior; **Entry** comes from the R0b table (0.2), not each call site. `NotSent`/`Sent`/`MaybeSent` is independent of the deactivation class: stream-local can be `Sent`, connection-terminal can be `NotSent`.

| Behavior | Exchange | Entry | Reuse |
| --- | --- | --- | --- |
| invalid key/zero port | pre-I/O error, `NotSent` | none | no entry |
| local slots full | backpressure, `NotSent` | healthy | keep |
| init failure, no resource | connect/TLS, `NotSent` | `Closing`; task terminal, nothing to drain | after terminal |
| init failure, resource exists | connect/TLS, `NotSent` | `Initializing -> Closing`; handoff closed by task | after terminal |
| init close, no resource | `Closed(NotSent)` | `Closing`; never `Active`; task terminal | unavailable now |
| init close, resource exists | `Closed(NotSent)` | `Closing`; late resource handed to task | after terminal |
| same-key `Initializing` | single-flight join under caller deadline; `Closed(NotSent)` if close wins | unchanged | one entry, no 2nd gen |
| same-key `Closing` | `Closed(NotSent)`, no drain wait | `Closing`, slot unlent | retry after terminal removal |
| stream open/send uncertainty | send/connect error | keep unless terminal | no replay |
| peer reset / malformed response | DoQ/DoH3 terminal error, usually `Sent` | keep if healthy | reusable |
| caller cancel / deadline | existing control error | keep if healthy | reusable |
| owner close | `Closed` with side-effect state | `Initializing`/`Active -> Closing`, drain | unavailable |
| connection/driver terminal failure | connection error | logical `Active -> Closing`; unleasable; removal at terminal only | after terminal |
| final commit | candidate until commit | keep after permit release | reusable after commit |

Preserve the closed side-effect vocabulary and never turn stream-local failure into an implicit retry or cross-protocol fallback; an unclassified error is a Slice 0 gap blocking Slice 1, not a runtime guess.

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT, session resumption, connection migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks, interface/source binding, UDP retransmission, listeners, config, plugins, sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or general-purpose pool abstraction enters this task. The QUIC/H3 graph stays locked: a dependency change is a Slice 0 blocker requiring revised plan/review, and R0a/R0b/R0c follow the same rule — a pinned API that cannot satisfy the contract stops the task for re-review, never a dependency change. Rollback is a task-scoped revert of the new module, exports, tests, and any narrowly required lifecycle/error additions; existing one-shot QUIC, secure, resolver, and generic TCP reuse paths are unchanged, and unrelated dirty files are not staged or reverted.

## 11. Test architecture

- Slice 0: pure key/limit/entry-state model tests, no socket or QUIC I/O, covering multi-key cap (4.1), same-key `Closing` (4.3), init-vs-owner-close with late resource acquisition (3.1/11.1), supervised teardown (7.2), and four-phase R0a (0.1); all fail if their contract is removed, and R0a is model evidence only.
- Slice 1: in-process DoQ fixtures plus accept counting, concurrent stream markers, one-stream cancellation, dead-connection replacement.
- Slice 2: H3 fixture counting one connection and multiple request stream IDs, hold/release responses, driver liveness, and cancellation without killing another stream (incl. R0a drop-only phases); the real pinned-stack proof is Slice 2/A5.
- Slice 3: peer stream-budget fixtures, bound/backpressure, idle-clock, concurrent close, resolver A/AAAA-to-key, bounded stress.
- Every slice asserts connection, stream/request, and lifecycle counts, final commit, and no late success at the boundary exercised.

### 11.1 Aborted-at-barrier/no-surviving-caller supervised-teardown test

Slice 0 executable form of 3.1/7.2 (no socket); assertions are ordered — held-barrier
liveness first, post-release terminal second.

1. Build an entry with a deterministic teardown **barrier** the supervised task must pass before `Drained`, plus an initializer that acquires its resource only when released.
2. Spawn concurrent `close()` waiters and abort the **first** while it is parked at the barrier.
3. **Close wins the linearization point**: the entry is marked `Closing`/`TeardownRequested`, its reservation is **not removed**, and no `Active` may be published.
4. **Held barrier — liveness/ownership only**: `Closing` stays **discoverable**; the task is alive and still **owns** the initializer handoff, driver/`JoinHandle`, shutdown signal, and liveness guard; `Lifecycle` has **not** drained; no completion reported. No terminal outcome is asserted yet.
5. Drop **all** waiters (no surviving caller), still held: progress does not depend on any waiter.
6. **Then complete the initializer and let it acquire its resource**: it never publishes `Active`, the generation does not disappear, and the resource is handed to and closed by the task (no orphan, no late-resource race, no second generation).
7. **Release the barrier, terminal assertions only now**: exactly one teardown ran; it reaches `Drained`/`Failed` **autonomously**; removal and slot/liveness release happen inside the task and only at that terminal; no late response committed.

Fails if a `Closing` reservation is removed early, `Active` is published after close
won, a late resource is orphaned or excluded from teardown, teardown moves into a caller
or stalls once all callers are gone, `Lifecycle` drains early, or removal happens in any
state other than `Drained`/`Failed`.
