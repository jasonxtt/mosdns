# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only; no code before approval and `task.py start`.

## 0. Pre-start gates (R0, blocking)

R0 blocks Slice 1 and is part of Slice 0's exit criteria.

### 0.1 R0a — H3 request cancellation is core

Per-phase cancellation contract for one H3 stream on pinned `h3 0.0.8`/`h3-quinn 0.0.10`, four phases: (1) **before send** — drop the stream; (2) **after request FIN** — tear down the receive side only; (3) **during response head** — a h3-quinn read future may already hold the internal receive stream; (4) **during body read** — same hazard on the data path.

Pinned hazard: `h3_quinn::RecvStream::poll_data` takes the `Option<quinn::RecvStream>` into its in-flight `read_chunk_fut`, restoring it only on completion; a local control decision drops it, leaving `None`, and a later `stop_sending` unwraps and panics. R0a therefore marks each phase active-stop or drop-only; where unsafe, return the unchanged typed control error and drop the `RequestStream`, letting Quinn's `RecvStream::Drop` stop unread data.

**Slice 0 proves the decision/state model, not the pinned stack**: no socket or QUIC/H3 I/O; the four-phase test asserts only that the model never selects the pinned `Option::None` `stop_sending` path, that transitions are per-phase and deterministic, and that the logical shared-entry state stays healthy. **The real pinned-stack H3 proof is Slice 2/A5**, whose loopback fixture shows one canceled real H3 request leaves connection, driver, and another request healthy; Slice 0 must not present model evidence as real H3 evidence. A dependency change is never the remedy: an unsatisfiable phase is drop-only and stops for re-review.

### 0.2 R0b — connection-level versus stream-level error classification is core

One authoritative table over the pinned error vocabulary, consumed by later slices. Each error is exactly one class: **stream-local** (query fails; entry stays healthy/reusable) or **entry-terminal** (exact key+generation logically deactivated `Active -> Closing`, immediately unleasable, removal only at `Drained`/`Failed`, no replay). Derived from pinned sources, not a `Result` guess, it covers at least DoQ stream reset/stop, framing/response-validation, `open_bi`/write/read, H3 request-stream errors, H3 driver `poll_close` terminal outcomes, and `quinn::Connection` `closed`/`close`. A `SendRequest`-level failure proves neither dead nor healthy. R0b also records `NotSent`/`Sent`/`MaybeSent` for `SideEffectState`, independent of deactivation class.

### 0.3 R0c — pinned API assumptions are frozen

Every fact in `research/quic-reuse-evidence.md` for `quinn 0.11.7`, `h3 0.0.8`, `h3-quinn 0.0.10` is verified in R0 against the locked local registry source and pinned to an exact file/line range with a holds/does-not-hold result. The graph stays locked; a non-holding assumption stops for re-review, and dependency add/remove/bump is forbidden.

### 0.4 R0 exit criteria

R0 closes when 0.1-0.3 hold at the model level with no dependency change; it does **not** close the pinned-stack H3 proof (Slice 2/A5).

## 1. Boundary and ownership

Deliverable stays in `rust/upstream-core`: one-shot `DoqUpstream`/`Doh3Upstream` remain valid primitives and regression coverage; new code is a sibling QUIC-specific owner (`src/quic_reuse.rs`) with minimal re-exports for a future Rust-native host. It must not turn serial `ReuseOwner` into a cross-protocol pool — shared vocabulary reused, not its storage/lifecycle. `Lifecycle` stays the only owner admission, close, in-flight registration, and final commit gate; `ExchangeContext`/`ExchangeControl` the only absolute deadline and caller/owner cancellation race; `ServerIdentity`, `TlsPolicy`, `PublishedTarget`, `DohEndpoint::get_request_target` authoritative; `tcp::write_frame` and the existing DNS validators reused, with no second framing or DoH target/response contract.

## 2. Validated key

`QuicProtocol::{Doq, Doh3}` is closed, fixing ALPN to `doq`/`h3`; no arbitrary ALPN. `QuicReuseKey` holds numeric `SocketAddr`, protocol/ALPN discriminator, canonical identity text, TLS verification mode, TLS roots revision, and optional DoH3 authority; constructors `from_doq(&DoqEndpoint, &TlsPolicy)` / `from_doh3(&DohEndpoint, &TlsPolicy)` accept only validated endpoints/policies. DoQ has no HTTP authority; DoH3 authority is keyed because `DohEndpoint` separates origin authority from the numeric dial, so a connection must not silently serve a different origin. TLS key material is never copied into the key: mode plus opaque `TlsPolicy::roots_revision` identifies trust; cloning preserves the revision, a new verified policy intentionally prevents reuse with old roots. A resolver refresh or A/AAAA change alters only the numeric dial — never identity, authority, or an established connection.

Key tests are pure, deterministic, and cover every isolation dimension, including DoQ vs DoH3 on one numeric address and two equal verified policies with distinct roots revisions.

## 3. Owner and connection-entry state

`QuicReuseOwner`: `Arc<Lifecycle>` + owner `TransportCancellation`; short-lock `QuicReuseKey -> Arc<QuicConnectionEntry>` map; task-local limits + injected clock/maintenance seam; no generic TCP pool or hidden config selector.

Each `QuicConnectionEntry` owns one physical QUIC connection per key, a stream-slot semaphore, a last-used timestamp, a generation/health state, the endpoint handle keeping Quinn's UDP driver alive, and a `Lifecycle::register_owned` liveness guard from insertion to terminal removal. DoQ payload stores the cloned `quinn::Connection` and calls `open_bi` per attempt without sharing stream halves. DoH3 stores the sender template plus a long-lived driver handle; each query clones the sender (`h3 0.0.8` is cloneable) while the driver polls the H3 `Connection`. A driver terminal error marks the entry dead and never silently replaces it for the in-flight request.

Connection creation is single-flight per key: the admission section installs a **reserved placeholder** under the owner-map lock before any async connect/build, so the map never holds a half-defined entry and observers see `Initializing` (key+generation fixed, one initializer, no leases), `Active` (transport published, leasable), or `Closing`/`Drained`/`Failed` (7.1). Installing it also **starts and holds one entry-owned initializer task with a `JoinHandle`** (or an equivalent entry-owned shared future plus guard) under that same lock — never a caller-owned future; concurrent first users join that initializer.

Initializer execution ownership is frozen and cancellation-safe. No exchange caller owns, polls, or must drive it: every same-key caller only awaits the shared completion, and its `ExchangeControl`/deadline/cancellation releases only that caller's wait, never the initializer task. Dropping the last waiter, or every caller, cannot stop it — the entry owns the task/`JoinHandle`/guard, never spawn-and-forget, released only at terminal `Drained`/`Failed`. It produces exactly one completion, the latch the supervised teardown (7.2) awaits, so an aborted leader caller can never strand an `Initializing`/`Closing` entry.

Publication and close share one linearization point (3.1): `Active` is published only while owner `Open` **and this exact key+generation still `Initializing`**; otherwise the initializer observes `Closing`/`TeardownRequested`, never publishes `Active`, and the entry is never both `Active` and `Closing`. Outcomes are classified by resource acquisition, not error type (3.2).

### 3.1 The single initialization crossing protocol

- Under the owner-map lock, `begin_close` (or idle expiry / connection-level logical deactivation) marks **every `Initializing` and `Active` entry `Closing`/`TeardownRequested`**; the reservation/generation is **not** removed and `Closing` stays discoverable but unleasable. Entering `Closing` starts the supervised teardown exactly once (7.2), which supervises the handoff and owns driver/`JoinHandle`, shutdown signal, liveness, and completion — no close-waiter abort stops it, and the entry-owned initializer (section 3) runs independently of every exchange waiter.
- The initializer builds resources **outside** the lock, then hands one result to the entry-owned teardown/owner state under the **same** lock/handoff protocol: owner `Open` + generation `Initializing` → publish `Active` (the only publication point); already `Closing`/`TeardownRequested` → never publish `Active` or leave a resource in a removed generation, hand the whole result (including a late-acquired resource) to the supervised teardown, which always receives one completion (3.2).
- Only terminal `Drained`/`Failed` performs exact key+generation removal and releases slot+liveness: no late-resource race, early drain, stranded `Closing`, second generation, or reliance on a later close/drain pass.
- Same-key `Closing` -> `UpstreamError::Closed(SideEffectState::NotSent)`, no drain wait, no second generation (4.3).

### 3.2 Initializer completion and handoff outcomes

One completion under the same lock/handoff protocol:

- **Published** — owner `Open`, generation `Initializing`: transport became `Active`, serving leases.
- **No resource** (failed, or close won before any handle) — teardown records terminal `Failed`, removes exactly once, nothing to drain.
- **Close won, resource acquired** — a handle existed (even built after teardown started): handed to the supervised teardown, `Drained` after cleanup, never released early or orphaned.

No async wait occurs while the owner map lock is held; all network, stream, driver, and close awaits happen after the relevant entry or snapshot is acquired.

## 4. Bounds and admission

Task-local non-configurable constants: `MAX_STREAMS_PER_CONNECTION = 32`; `MAX_CONNECTIONS_PER_OWNER = 8`; `QUIC_IDLE_TIMEOUT = 30s` (lazy maintenance, no background timer); one physical connection per key.

Admission sequence:

1. Register through the owner's `Arc<Lifecycle>`, rejecting `Closed(NotSent)` before any network operation; if the later map-side admission loses the `accepting` race (4.1), that registration is released again with no residue.
2. Enter the **atomic admission section** (4.1): check `accepting`, look up the key, mark idle-expired entries `Closing` (discoverable until `Drained`/`Failed`, 4.2/7.1), check capacity, and reuse `Active` or install an `Initializing` reservation.
3. Resolve by state (4.3): `Active` used directly; `Initializing` joined via the entry-owned initializer; `Closing` returns the typed pre-send closed result.
4. Take a stream permit without queueing; if exhausted, return typed `Backpressure` with `NotSent`, never growing an owner queue or opening a second same-key connection.
5. Race QUIC/H3 stream opening against the original `ExchangeControl` and absolute deadline; a peer stream-credit wait may stay pending only under that bounded caller-owned race.
6. On every exit release the permit once and update idle time only when the entry has no active permits and stays healthy.

### 4.1 Atomic multi-key admission

`MAX_CONNECTIONS_PER_OWNER = 8` is a cross-key invariant enforced by one owner-map critical section with **no `await` inside it**. The map carries one model-only **`accepting` (Open) gate** (not config/API): (1) **check `accepting` first** — if false, install nothing, start no initializer, return `Closed(NotSent)`; (2) look up the key; (3) mark dead/idle-expired entries `Closing` (no removal — discoverable until `Drained`/`Failed`, 4.2/7.1 — no drain awaited); (4) capacity-check (a same-key hit consumes none) counting `Initializing`/`Closing`/`Active` alike, since a slot frees only at terminal `Drained`/`Failed` (3.1, 3.2); (5) reuse `Active`, join `Initializing`, return the typed pre-send result for `Closing` (4.3), or install a fresh-generation `Initializing` reservation.

This is the **sole map-side admission-vs-close linearization** (7.3). An exchange that completed `Lifecycle::register` (4 step 1) but reaches this section after close set `accepting=false` must **release that registration and any local liveness guard** before returning `Closed(NotSent)` — no reservation or initializer, no second generation, zero `Lifecycle` liveness or slot residue. An admission completing its install inside the lock is ordered against close by it, so close's scan captures it; one still waiting after `accepting=false` can only be rejected.

The owner map guard is `std::sync::Mutex` (or equivalent non-async lock), dropped before any connect/stream/driver/close await. Non-conforming: installing without checking `accepting` in the same lock; a check-then-insert split across two acquisitions; dropping the lock between capacity check and insert; a per-key lock leaving the count racy; counting from a snapshot then inserting; holding the lock across an `await`; publishing `Active` without re-checking owner/generation state (3.1); or removing a `Closing` reservation before terminal (3.1).

Cap test (Slice 0, no socket): many concurrent admissions for distinct keys against a barrier, count observed under the same lock. Assert live entries never exceed the cap, the admitted-key count reaches it exactly, every excess admission gets the typed capacity error, and no same-key reservation is double-counted. Cover concurrent idle-expiry/close: while `Initializing` or `Closing` an entry still occupies its slot, so a new key is admitted only at `Drained`/`Failed`. Fail if the section splits, the lock drops before the insert, a `Closing` slot is reused before terminal, or any admission installs after the gate closes.

### 4.2 Idle expiry

Idle expiry is lazy: the admission/maintenance scan checks the injected clock under the owner-map lock (no reaper); an expired entry is marked `Closing` in that same lock section, not removed, staying discoverable until `Drained`/`Failed` (7.1).

The **admission path never awaits a drain**: a scan that expires an entry starts (or joins) the supervised teardown (7.2) and returns, never blocking admission on any completion. This preserves 4.3: a same-key `Closing` lookup returns `Closed(NotSent)` immediately, and `Initializing` joining is the only same-key wait. The explicit maintenance method may **optionally await** expired entries' completion for deterministic tests/shutdown; that await stays confined to maintenance.

### 4.3 Same-key lookup by entry state

A lookup that finds the key present resolves by state under the admission lock, with no unbounded internal wait:

| Entry state found | Result for this caller |
| --- | --- |
| `Active` | Lease it and continue the normal exchange path. |
| `Initializing` | Await the entry-owned initializer's completion, bounded by this caller's deadline/`ExchangeControl`; proceed if it publishes `Active` while the owner is `Open`, else return the typed pre-send closed result. Cancelling ends only this caller's wait, never the initializer. |
| `Closing` | Return one existing typed pre-send closed result (below). |
| `Drained` / `Failed` | Removed at that terminal; a concurrent lookup still sees the terminal state (typed pre-send closed result) or, after removal, admits a fresh generation. |

The `Closing` result reuses the existing closed vocabulary, `UpstreamError::Closed(SideEffectState::NotSent)` — not a new kind: no request byte and no stream were sent, so `NotSent` is exact; the caller does **not** wait on the closing entry's teardown (no unbounded drain); it opens **no** second generation, does **not** reuse the `Closing` slots, and may retry once a fresh generation is admitted after terminal removal.

`Initializing` joining is the only same-key wait, under the caller's deadline/control race, never an internal queue, returning `Closed(NotSent)` if close wins (3.1). Only that wait is cancellable; the entry-owned initializer survives the caller's abort and every waiter's drop, so the completion latch is eventually written (3.1, 7.2).

Same-key `Closing` test (Slice 0, no socket): hold a key in `Closing` behind a teardown barrier and issue concurrent same-key admissions. Assert each returns `Closed(NotSent)` without awaiting drain, no second generation is created, the `Closing` slot is not leased, and after release and terminal `Drained`/`Failed` a later admission admits a fresh generation. Fail if a lookup opens a duplicate generation, reuses a `Closing` slot, or blocks on the drain.

## 5. DoQ exchange flow

For a leased DoQ entry: (1) check owner/caller/deadline as `NotSent`; (2) open one fresh bidirectional stream on the shared connection; (3) copy the caller query, zero its ID, write it with the existing two-byte framing helper; (4) finish the request side, stopping only that stream on local cancellation; (5) read one framed response through the peer FIN, validate wire ID zero, restore the caller ID, run the existing DNS validator; (6) apply `Lifecycle::commit_final_response` with the original cancellation/deadline before releasing the permit; (7) return the connection if healthy. A stream reset, malformed response, or local cancellation is stream-local; a QUIC connection error or closed handle is connection-terminal and deactivates the entry `Active -> Closing` by exact key+generation, immediately unleasable, with map removal only at `Drained`/`Failed` (7.4). A failed query is never replayed; a replacement is available only to a later independent exchange, once the old generation reached terminal `Drained`/`Failed`.

## 6. DoH3 exchange and long-lived driver

One-time setup per entry: (1) build the `TlsPolicy`-derived config offering only `h3`; (2) open the numeric QUIC connection using the endpoint's separate identity; (3) build H3 once (control/QPACK streams, driver, cloneable `SendRequest`); (4) hand the driver `JoinHandle` to the supervised teardown (7.2) with no droppable copy; (5) spawn it **from the entry-owned initializer (section 3)**, looping `poll_close` to completion, never from a caller future. If teardown was requested during setup (3.1), the driver is never published `Active` and the supervised task closes it.

Per request: (1) clone the sender template and build the DoH GET target from `DohEndpoint::get_request_target`; (2) open one H3 request stream, send the existing headers, finish the request side; (3) read/validate with the bounded DoH validator, restoring only that ID; (4) race every phase against the original control/deadline; (5) commit through the shared lifecycle gate, then release only that permit.

A query cancellation resets/stops only its request stream **as permitted by R0a (0.1)**; while h3-quinn owns an internal read future the receive side is in the pinned `None`-option hazard state and `RequestStream::stop_sending` must not be called (it unwraps and panics) — drop-only via the unchanged typed control error and a `RequestStream` drop, letting Quinn's `RecvStream::Drop` stop unread data. Slice 0 proves the decision model only; the real loopback proof is Slice 2/A5. Stream-local, not permission to close the connection; R0a closes the decision model before Slice 1.

The H3 driver is not `H2ScopeLease` (one exchange, seals/aborts at response completion): it stays alive across requests; a dedicated QUIC handle must make admission, shutdown, and drain idempotent.

## 7. Close, logical deactivation, and final commit

### 7.1 Entry lifecycle

Every `QuicConnectionEntry` exposes an explicit, observable lifecycle:

```text
Initializing --publish--> Active; close/expiry -> Closing -> {Drained|Failed}; terminal only -> exact key+gen removal + slot/liveness release
```

- **Initializing** — reserved placeholder, one key+generation, one initializer, no leases (4.1, section 3).
- **Active** — admitted and leasable, reachable only from `Initializing` via 3.1.
- **Closing** — unleasable, **still in the map and discoverable** until `Drained`/`Failed`, so a caller/maintenance pass joins the in-progress teardown instead of racing a vanished entry; a `Closing` reservation is never deleted early (3.1, 3.2, 7.2).
- **Drained** — the initializer handoff was observed, the H3 driver stopped, request streams ended, the QUIC connection/endpoint handle was released, and the `Lifecycle` liveness registration was dropped.
- **Failed** — teardown could not complete gracefully, or nothing was left to drain; terminal via the explicit bounded force path (7.2), releasing liveness and never leaving a detached driver.

`Drained`/`Failed` are the only terminal outcomes and the only removal point for the exact key+generation, releasing slot+liveness; no other removal path exists (3.1, 3.2).

### 7.2 Entry-owned supervised teardown

On entering `Closing`, an **entry-owned supervised teardown task starts exactly once**; it — not any close caller — owns the initializer completion/handoff plus guard, the H3 driver task/`JoinHandle`, shutdown signal, `Lifecycle` liveness guard, and the shared `Drained`/`Failed` completion. Every close path — owner close, idle expiry, connection-level logical deactivation, protocol-terminal failure, and init that acquired a resource then observed close — converges on it:

- **Exactly one task** per generation: a second `Closing` transition returns the same completion, never a second teardown.
- **Handoff supervised.** It holds the initializer task/`JoinHandle`/guard and awaits its single completion latch, closing any resource built (even after teardown was requested) under the state lock, so nothing stays in a removed generation and no `Active` is published once close won; the guard drops only at terminal, while aborting an exchange caller drops only its own await.
- **Callers only await.** Every close caller (concurrent `close()`, owner close, maintenance) awaits the shared completion; none drives, polls, or performs teardown work itself.
- **Abort cannot stop progress.** Dropping the first, **all**, or every exchange waiter drops only their awaits; the entry-owned task keeps its initializer guard, driver/`JoinHandle`, shutdown signal, and liveness guard, so it progresses with **no surviving caller** and never needs a later close/drain pass.
- **Liveness held to terminal.** It keeps the `Lifecycle` registration until `Drained`/`Failed`, so no vanished caller drains it while the driver runs or the handoff is outstanding.
- **Terminal-only removal.** It alone removes the map entry and releases slot/liveness at `Drained`/`Failed`; no other path removes a served or `Closing` entry.
- **Bounded force path.** If the peer never cooperates, it cancels locally, closes with an explicit QUIC code, and reaches `Failed` via the same completion.

Teardown is idempotent: replaying the completion returns the recorded outcome without re-running shutdown, re-closing, or double-releasing a guard.

### 7.3 Owner close sequence

1. `begin_close` transitions the `Lifecycle` and cancels the owner token, atomically rejecting further `register` calls.
2. The owner-map critical section then performs the map-side close linearization in order: (a) set **`accepting=false`**; (b) mark **every `Initializing` and `Active` entry `Closing`/`TeardownRequested`** without removing its reservation/generation; (c) start each entry's exactly-once supervised teardown (7.2). That one lock section blocks `Initializing -> Active` publication (3.1) and prevents leasing a `Closing` entry. As `accepting=false` is set in the same lock, an exchange still between `Lifecycle::register` and map admission is rejected (4.1) rather than installing a stranded reservation, while an entry installed before (a) is captured by (b); a `Closing` reservation stays discoverable.
3. Teardown awaits the initializer handoff (closing any late-acquired resource); the owner close path then awaits each `Closing` entry's completion like every other caller. Only at `Drained`/`Failed` does the task remove the entry, release its slot, and drop its liveness guard; only then do exchanges drain through `Lifecycle` before `finish_close`.

If an entry is already `Drained`/`Failed`, close is idempotent and only waits for the owner-level registrations.

### 7.4 Logical deactivation and final commit

A served/`Active` entry hitting a connection-level failure is **logically deactivated, not immediately removed**: under the map/state lock it transitions `Active -> Closing` by exact key+generation, immediately unleasable, then takes the same supervised teardown as every other close path; physical removal happens only at `Drained`/`Failed` (7.2). A stale failure callback cannot touch a newer replacement, and stream-local errors do not transition the entry unless connection/driver health also proves terminal (R0b table: 0.2, section 9).

The final commit precedes stream permit release: a successful commit stays a success even if close starts immediately after; close/cancel/deadline winning before commit returns a typed error; no entry returns to the pool before the commit decision completes.

## 8. Resolver composition

Use the existing resolver snapshot read-only: DoQ uses `ResolverComposition::doq_endpoint(PublishedTarget, ServerIdentity)`; DoH3 uses `ResolverComposition::doh_endpoint(PublishedTarget, service_url)` and the existing `DohEndpoint`. The owner builds a key from the resulting endpoint and policy, so a selected A or AAAA address produces a distinct key without copying resolver state or changing the authenticated service identity. Resolver refresh does not proactively close an established connection: the old key stays valid until idle/dead/owner teardown, and a new selected target creates or reuses its own key. No resolver algorithm, fallback, cross-family race, or socket policy is added.

## 9. Error and state matrix

Rows come from the R0b table (0.2), not each call site; `NotSent`/`Sent`/`MaybeSent` is independent of deactivation.

- invalid key/zero port — pre-I/O error, `NotSent`; none; no entry.
- local slots full — backpressure, `NotSent`; healthy; keep.
- init failure, no resource — connect/TLS, `NotSent`; `Closing`, task terminal; after terminal.
- init failure, resource exists — connect/TLS, `NotSent`; `Initializing -> Closing`, handoff closed by task; after terminal.
- init close, no resource — `Closed(NotSent)`; `Closing`, never `Active`, task terminal; unavailable now.
- init close, resource exists — `Closed(NotSent)`; `Closing`, late resource handed to task; after terminal.
- same-key `Initializing` — join initializer under caller deadline, `Closed(NotSent)` if close wins; unchanged; one entry, no 2nd gen.
- same-key `Closing` — `Closed(NotSent)`, no drain wait; `Closing`, slot unlent; retry after terminal removal.
- stream open/send uncertainty — send/connect error; keep unless terminal; no replay.
- peer reset / malformed response — DoQ/DoH3 terminal error, usually `Sent`; keep if healthy; reusable.
- caller cancel / deadline — existing control error; keep if healthy; reusable.
- cancel/deadline during `Initializing` — only that wait ends, initializer unaffected; `Initializing` (or `Closing`), one completion; after terminal or `Active`.
- owner close — `Closed` with side-effect state; `Initializing`/`Active -> Closing`, drain; unavailable.
- connection/driver terminal failure — connection error; logical `Active -> Closing`, removal at terminal only; after terminal.
- final commit — candidate until commit; keep after permit release; reusable after commit.

Preserve the closed side-effect vocabulary; never turn stream-local failure into an implicit retry or cross-protocol fallback — an unclassified error is a Slice 0 gap blocking Slice 1.

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT/resumption, migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks, interface/source binding, UDP retransmission, listeners, config, plugins, sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or general-purpose pool enters this task. The QUIC/H3 graph stays locked: a dependency change is a Slice 0 blocker requiring revised plan/review, and R0a/R0b/R0c follow the same rule — a pinned API that cannot satisfy the contract stops for re-review, never a dependency change. Rollback is a task-scoped revert of the new module/exports/tests and narrowly required lifecycle/error additions; existing one-shot QUIC, secure, resolver, and generic TCP reuse paths are unchanged, and unrelated dirty files are not staged or reverted.

## 11. Test architecture

- Slice 0: pure model tests, no socket or QUIC/H3 I/O, covering multi-key cap (4.1), same-key `Closing` (4.3), initializer-caller cancellation with zero surviving waiters plus init-vs-owner-close with late resource acquisition (3.1/11.1), the post-close register-but-not-yet-installed admission race (4.1/7.3/11.2), supervised teardown (7.2), and four-phase R0a (0.1); each fails if its contract is removed, and R0a is model evidence only.
- Slice 1: in-process DoQ fixtures plus accept counting, concurrent stream markers, one-stream cancellation, dead-connection replacement.
- Slice 2: H3 one-connection/multi-stream fixture, hold/release responses, driver liveness, cancellation without killing another stream (incl. R0a drop-only phases); Slice 2/A5 owns the real pinned-stack proof.
- Slice 3: peer stream-budget, bound/backpressure, idle-clock, concurrent close, resolver A/AAAA-to-key, bounded stress.
- Every slice asserts connection, stream/request, and lifecycle counts, final commit, and no late success at its boundary.

### 11.1 Aborted-at-barrier/no-surviving-caller supervised-teardown test

Slice 0 form of 3.1/7.2 (no socket); ordered held-barrier liveness first, post-release terminal second; also proves entry-owned initializer execution.

1. Install an `Initializing` reservation behind an **initializer barrier**, plus a teardown barrier before `Drained`; abort/cancel the **first initializer caller** and drop **every exchange waiter**. The entry-owned initializer task/`JoinHandle` **remains alive** and still yields **exactly one** completion once released — no waiter need poll it; completion is asserted only after release (step 3).
2. Close (or expiry) wins the linearization point: the entry is marked `Closing`/`TeardownRequested`, its reservation is **not removed**, no `Active` may be published; drop **all** close waiters too. **Held-barrier assertions — liveness/ownership only**: `Closing` stays **discoverable**; the task still **owns** the initializer handoff, driver/`JoinHandle`, shutdown signal, and liveness guard; `Lifecycle` has **not** drained; no completion reported; progress depends on no waiter.
3. Release the initializer barrier (resource acquired) then the teardown barrier: terminal assertions only now — no `Active`, the generation does not disappear, the resource is handed to and closed by the task (no orphan, no late-resource race, no second generation); exactly one teardown reached `Drained`/`Failed` **autonomously** with removal and slot/liveness release inside the task at that terminal; no stranded `Initializing`/`Closing`, no late response.

Fails if aborting the initializer caller or all waiters stops the initializer, a `Closing` reservation is removed early, `Active` is published after close won, a late resource is orphaned, teardown stalls once all callers are gone, `Lifecycle` drains early, or removal happens outside `Drained`/`Failed`.

### 11.2 Post-close admission (register-but-not-yet-installed) race test

Slice 0 form of 4.1/7.3 (no socket):

1. Exchange A completes `Lifecycle::register` while the owner is `Open`, then parks at a barrier before the owner-map admission section (4.1); no reservation exists for A yet.
2. Owner close runs: `begin_close` transitions the `Lifecycle`, then the map section sets `accepting=false` and captures every then-existing `Initializing`/`Active` entry as `Closing`/`TeardownRequested` with exactly-once supervised teardown (7.3).
3. Release A's barrier: admission returns `Closed(NotSent)` at the `accepting` check, installing no reservation, starting no initializer, creating no second generation, and releasing A's registration plus any local liveness guard — zero `Lifecycle` liveness and slot residue, so `Lifecycle` drains and `finish_close` completes; the map gains no `Initializing` entry or extra initializer task/liveness, while entries captured in step 2 still finish through the existing `Closing -> Drained`/`Failed` protocol (terminal-only removal).

Fails if an admission installs after `accepting=false`, leaks a registration/liveness/slot when rejected, creates a stranded `Initializing`, or lets close-captured entries bypass the existing teardown protocol.
