# Design — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only; no code before approval and `task.py start`.

## 0. Pre-start gates (R0, blocking)

R0 blocks Slice 1 and is part of Slice 0's exit criteria.

### 0.1 R0a — H3 request cancellation is core

Per-phase cancellation contract for one H3 stream on pinned `h3 0.0.8`/`h3-quinn 0.0.10`, four phases: (1) **before send** — drop the stream; (2) **after request FIN** — receive side only; (3) **during response head** — a h3-quinn read future may already hold the internal receive stream; (4) **during body read** — same hazard.

Pinned hazard: `h3_quinn::RecvStream::poll_data` takes the `Option<quinn::RecvStream>` into its in-flight `read_chunk_fut`, restoring it only on completion; a local control decision drops it, leaving `None`, and a later `stop_sending` unwraps and panics. R0a marks each phase active-stop or drop-only; where unsafe, return the unchanged typed control error and drop the `RequestStream`, letting Quinn's `RecvStream::Drop` stop unread data.

**Slice 0 proves the decision/state model, not the pinned stack**: no socket or QUIC/H3 I/O; its four-phase test asserts only that the model never selects the pinned `Option::None` `stop_sending` path, that transitions are per-phase and deterministic, and that the logical shared-entry state stays healthy. **The real pinned-stack H3 proof is Slice 2/A5**, whose loopback fixture shows one canceled real H3 request leaves connection, driver, and another request healthy; Slice 0 must not present model evidence as real H3 evidence. A dependency change is never the remedy: an unsatisfiable phase is drop-only and stops for re-review.

### 0.2 R0b — connection-level versus stream-level error classification is core

One authoritative table over the pinned error vocabulary, consumed by later slices. Each error is exactly one class: **stream-local** (query fails; entry stays healthy/reusable) or **entry-terminal** (exact key+generation logically deactivated `Active -> Closing`, immediately unleasable, removal only at terminal, no replay). Derived from pinned sources, not a `Result` guess, it covers at least DoQ stream reset/stop, framing/response-validation, `open_bi`/write/read failures, H3 request-stream errors, H3 driver `poll_close` terminal outcomes, and `quinn::Connection` `closed`/`close`. A `SendRequest`-level failure proves neither dead nor healthy. R0b also records `NotSent`/`Sent`/`MaybeSent` for `SideEffectState`, independent of deactivation class.

### 0.3 R0c — pinned API assumptions are frozen

Every fact in `research/quic-reuse-evidence.md` for `quinn 0.11.7`, `h3 0.0.8`, `h3-quinn 0.0.10` is verified in R0 against the locked local registry source and pinned to an exact file/line range with a holds/does-not-hold result. The graph stays locked; a non-holding assumption stops for re-review, and dependency add/remove/bump is forbidden.

### 0.4 R0 exit criteria

R0 closes when 0.1-0.3 hold at model level with no dependency change; it does **not** close the pinned-stack H3 proof (Slice 2/A5).

## 1. Boundary and ownership

Deliverable stays in `rust/upstream-core`: one-shot `DoqUpstream`/`Doh3Upstream` remain valid primitives and regression coverage; the new code is a sibling QUIC-specific owner (`src/quic_reuse.rs`) with minimal re-exports for a future Rust-native host, and must not turn `ReuseOwner` into a cross-protocol pool. `Lifecycle` stays the only owner admission/close/in-flight/final-commit gate; `ExchangeContext`/`ExchangeControl` the only deadline and caller/owner cancellation race; `ServerIdentity`, `TlsPolicy`, `PublishedTarget`, `DohEndpoint::get_request_target` authoritative; `tcp::write_frame` and the existing DNS validators reused, with no second framing or DoH target/response contract.

## 2. Validated key

`QuicProtocol::{Doq, Doh3}` is closed, fixing ALPN to `doq`/`h3`; no arbitrary ALPN. `QuicReuseKey` holds numeric `SocketAddr`, protocol/ALPN discriminator, canonical identity text, TLS mode, `TlsPolicy::roots_revision`, and optional DoH3 authority; constructors `from_doq(&DoqEndpoint, &TlsPolicy)` / `from_doh3(&DohEndpoint, &TlsPolicy)` accept only validated endpoints/policies. DoQ has no HTTP authority; DoH3 authority is keyed because `DohEndpoint` separates origin authority from the numeric dial, so a connection must not silently serve a different origin. No TLS key material enters the key: mode plus opaque `roots_revision` identifies trust; cloning preserves the revision, and a new verified policy intentionally prevents reuse with old roots. A resolver refresh or A/AAAA change alters only the numeric dial — never identity, authority, or an established connection.

Key tests are pure, deterministic, and cover every isolation dimension, including DoQ vs DoH3 on one numeric address and two equal verified policies with distinct roots revisions.

## 3. Owner and connection-entry state

`QuicReuseOwner`: `Arc<Lifecycle>` + owner `TransportCancellation`; short-lock `QuicReuseKey -> Arc<QuicConnectionEntry>` map; task-local limits + injected clock/maintenance seam; no generic TCP pool or hidden config selector.

Each `QuicConnectionEntry` owns one physical QUIC connection per key, a stream-slot semaphore, a last-used timestamp, a generation/health state, the endpoint handle keeping Quinn's UDP driver alive, and a `Lifecycle::register_owned` liveness guard from insertion to terminal removal. DoQ payload stores the cloned `quinn::Connection` and calls `open_bi` per attempt without sharing stream halves. DoH3 stores the sender template plus a long-lived driver handle; each query clones the sender (`h3 0.0.8` is cloneable) while the driver polls the H3 `Connection`. A driver terminal error marks the entry dead and never silently replaces it for the in-flight request.

Connection creation is single-flight per key: the admission section installs a **reserved placeholder** under the map lock before any async connect/build, so observers see `Initializing` (key+generation fixed, one initializer, no leases), `Active` (transport published, leasable), or `Closing`/terminal (7.1). Installing it also **starts and holds one entry-owned initializer task with a `JoinHandle`** (or equivalent entry-owned shared future plus guard) under that same lock — never a caller-owned future; concurrent first users join that initializer.

Initializer execution ownership is frozen and cancellation-safe: no exchange caller owns, polls, or must drive it. Every same-key caller only awaits the shared completion, and its `ExchangeControl`/deadline/cancellation releases only that caller's wait, never the initializer task. Dropping the last waiter, or every caller, cannot stop it — the entry owns the task/`JoinHandle`/guard, never spawn-and-forget, released only at terminal. It produces exactly one completion, the latch the supervised teardown (7.2) awaits, so an aborted leader caller can never strand an `Initializing`/`Closing` entry.

Publication and close follow one two-stage protocol (3.1, 7.3): `Active` requires `Lifecycle == Open`, `accepting == true`, and this exact key+generation `Initializing` under the map/state lock. `Lifecycle::Open` is the real `Lifecycle` state and `accepting` the separate map admission gate — both are checked and neither substitutes for the other; any miss takes the (3.2) supervised-teardown path.

### 3.1 The single initialization crossing protocol

- Owner close is **two-stage** (7.3): `Lifecycle::begin_close` turns the real `Lifecycle` `Open -> Closing` and rejects `register`; only then does the owner-map/state critical section set `accepting=false` (the sole map-side admission-vs-close linearization) before marking every `Initializing`/`Active` entry `Closing`/`TeardownRequested` and starting its exactly-once supervised teardown. `begin_close` is never executed inside the map lock; idle expiry and connection-level logical deactivation take only that map-side path, keeping reservation/generation discoverable but unleasable (7.2, 7.3).
- The initializer builds resources **outside** the lock, then hands one result back under that **same** map/state lock — the only publication point — publishing `Active` only when `Lifecycle == Open`, `accepting == true`, and this exact key+generation is `Initializing` all hold. Because `begin_close` alone already makes `Lifecycle != Open`, an initializer winning the lock after `begin_close` but before `accepting=false` still fails publication and hands the whole result (including a late-acquired resource) to the supervised teardown (3.2); if owner close wins the lock first, `accepting=false` plus the `Closing` mark fail it the same way. Neither order reaches `Active`.
- Only terminal performs exact key+generation removal and releases slot+liveness: no late-resource race, early drain, stranded `Closing`, second generation, or reliance on a later close/drain pass.
- Same-key `Closing` -> `UpstreamError::Closed(SideEffectState::NotSent)`, no drain wait, no second generation (4.3).

### 3.2 Initializer completion and handoff outcomes

One completion under the same lock/handoff protocol: **published** (owner `Open`, generation `Initializing`) → transport became `Active`, serving leases; **no resource** (failed, or close won before any handle) → teardown records terminal `Failed`, removes exactly once, nothing to drain; **close won, resource acquired** (even one built after teardown started) → handed to the supervised teardown, drained after cleanup, never released early or orphaned.

No async wait occurs while the owner map lock is held; all network, stream, driver, and close awaits happen after the relevant entry or snapshot is acquired.

## 4. Bounds and admission

Task-local non-configurable constants: `MAX_STREAMS_PER_CONNECTION = 32`; `MAX_CONNECTIONS_PER_OWNER = 8`; `QUIC_IDLE_TIMEOUT = 30s` (lazy maintenance, no background timer); one physical connection per key.

Admission sequence:

1. Register through the owner's `Arc<Lifecycle>`, rejecting `Closed(NotSent)` before any network operation; if the later map-side admission loses the `accepting` race (4.1), that registration is released again with no residue.
2. Enter the **atomic admission section** (4.1): check `accepting`, look up the key, mark idle-expired entries `Closing` (discoverable until terminal, 4.2/7.1), check capacity, and reuse `Active` or install an `Initializing` reservation.
3. Resolve by state (4.3): `Active` used directly; `Initializing` joined via the entry-owned initializer; `Closing` returns the typed pre-send closed result.
4. Take a stream permit without queueing; if exhausted, return typed `Backpressure` with `NotSent`, never growing an owner queue or opening a second same-key connection.
5. Race QUIC/H3 stream opening against the original `ExchangeControl` and absolute deadline; a peer stream-credit wait may stay pending only under that bounded caller-owned race.
6. On every exit release the permit once and update idle time only when the entry has no active permits and stays healthy.

### 4.1 Atomic multi-key admission

`MAX_CONNECTIONS_PER_OWNER = 8` is a cross-key invariant enforced by one owner-map critical section with **no `await` inside it**. The map carries one model-only **`accepting` (Open) gate** (not config/API): (1) **check `accepting` first** — if false, install nothing, start no initializer, return `Closed(NotSent)`; (2) look up the key; (3) mark dead/idle-expired entries `Closing` (no removal — discoverable until terminal, 4.2/7.1 — no drain awaited); (4) capacity-check (a same-key hit consumes none) counting `Initializing`/`Closing`/`Active` alike, since a slot frees only at terminal; (5) reuse `Active`, join `Initializing`, return the typed pre-send result for `Closing` (4.3), or install a fresh-generation reservation.

This is the **sole map-side admission-vs-close linearization** (7.3): an exchange registered (4 step 1) but reaching here after `accepting=false` must **release that registration and any local liveness guard** and return `Closed(NotSent)` with no reservation, initializer, second generation, or liveness/slot residue; one installing inside the lock is instead captured by close's scan.

The owner map guard is `std::sync::Mutex` (or equivalent non-async lock), dropped before any connect/stream/driver/close await. Non-conforming: installing without checking `accepting` in the same lock; a check-then-insert split across two acquisitions; dropping the lock between capacity check and insert; a per-key lock leaving the count racy; counting from a snapshot then inserting; holding the lock across an `await`; publishing `Active` without re-checking `Lifecycle == Open`, `accepting`, and generation state in that same lock (3.1); or removing a `Closing` reservation before terminal.

Cap test (Slice 0, no socket): many concurrent admissions for distinct keys against a barrier, count observed under the same lock. Assert live entries never exceed the cap, the admitted-key count reaches it exactly, every excess admission gets the typed capacity error, and no same-key reservation is double-counted. Cover concurrent idle-expiry/close: while `Initializing`/`Closing` an entry still occupies its slot, so a new key is admitted only at terminal. Fail if the section splits, the lock drops before the insert, a `Closing` slot is reused before terminal, or any admission installs after the gate closes.

### 4.2 Idle expiry

Idle expiry is lazy: the admission/maintenance scan checks the injected clock under the map lock (no reaper); an expired entry is marked `Closing` in that same lock section, not removed, staying discoverable until terminal (7.1). The **admission path never awaits a drain**: a scan that expires an entry starts (or joins) the supervised teardown (7.2) and returns, never blocking admission on any completion. This preserves 4.3: a same-key `Closing` lookup returns `Closed(NotSent)` immediately, and `Initializing` joining is the only same-key wait. The explicit maintenance method may **optionally await** expired entries' completion for deterministic tests; that await stays confined to maintenance.

### 4.3 Same-key lookup by entry state

A lookup that finds the key present resolves by state under the admission lock, with no unbounded internal wait: `Active` leases and continues; `Initializing` awaits the entry-owned initializer under this caller's deadline/`ExchangeControl`, proceeding only on a successful publication (3.1) else returning the typed pre-send closed result (cancelling ends only this caller's wait); `Closing` returns the typed pre-send closed result below; `Drained`/`Failed` is removed at that terminal, so a concurrent lookup sees it (same result) or, after removal, admits a fresh generation.

The `Closing` result reuses `UpstreamError::Closed(SideEffectState::NotSent)` — not a new kind: no request byte and no stream were sent, so `NotSent` is exact; the caller does **not** wait on that entry's teardown (no unbounded drain), opens **no** second generation, does **not** reuse the `Closing` slots, and may retry once a fresh generation is admitted after terminal removal.

`Initializing` joining is the only same-key wait, under the caller's deadline/control race, never an internal queue, returning `Closed(NotSent)` if close wins (3.1); only that wait is cancellable, so the initializer survives any caller abort or waiter drop and the completion latch is eventually written (3.1, 7.2).

Same-key `Closing` test (Slice 0, no socket): hold a key in `Closing` behind a teardown barrier and issue concurrent same-key admissions. Assert each returns `Closed(NotSent)` without awaiting drain, no second generation or slot lease occurs, and after release and terminal a later admission admits a fresh generation. Fail if a lookup opens a duplicate generation, reuses a `Closing` slot, or blocks on the drain.

## 5. DoQ exchange flow

For a leased DoQ entry: (1) check owner/caller/deadline as `NotSent`; (2) open one fresh bidirectional stream; (3) copy the caller query, zero its ID, write it with the existing two-byte framing helper; (4) finish the request side, stopping only that stream on local cancellation; (5) read one framed response through the peer FIN, validate wire ID zero, restore the caller ID, run the existing DNS validator; (6) apply `Lifecycle::commit_final_response` with the original cancellation/deadline before releasing the permit; (7) return the connection if healthy. A stream reset, malformed response, or local cancellation is stream-local; a QUIC connection error or closed handle is connection-terminal and deactivates the entry `Active -> Closing` by key+generation, immediately unleasable, with map removal only at terminal (7.4). A failed query is never replayed: a replacement is available only to a later independent exchange, once the old generation reached terminal.

## 6. DoH3 exchange and long-lived driver

One-time setup per entry: (1) build the `TlsPolicy`-derived config offering only `h3`; (2) open the numeric QUIC connection using the endpoint's separate identity; (3) build H3 once (control/QPACK streams, driver, cloneable `SendRequest`); (4) hand the driver `JoinHandle` to the supervised teardown (7.2) with no droppable copy; (5) spawn it **from the entry-owned initializer (section 3)**, looping `poll_close` to completion, never from a caller future. If teardown was requested during setup (3.1), the driver is never published `Active` and the supervised task closes it.

Per request: (1) clone the sender template and build the DoH GET target from `DohEndpoint::get_request_target`; (2) open one H3 request stream, send the existing headers, finish the request side; (3) read/validate with the bounded DoH validator, restoring only that ID; (4) race every phase against the original control/deadline; (5) commit through the lifecycle gate, releasing only that permit.

A query cancellation resets/stops only its request stream **as permitted by R0a (0.1)**; while h3-quinn owns an internal read future the receive side is in the pinned `None`-option hazard state and `RequestStream::stop_sending` must not be called — drop-only via the unchanged typed control error and a `RequestStream` drop, letting Quinn's `RecvStream::Drop` stop unread data. Slice 0 proves the decision model only; the real loopback proof is Slice 2/A5. Stream-local, not permission to close the connection.

The H3 driver is not `H2ScopeLease` (one exchange, seals/aborts at response completion): it stays alive across requests; a dedicated QUIC handle must make admission, shutdown, and drain idempotent.

## 7. Close, logical deactivation, and final commit

### 7.1 Entry lifecycle

Every entry exposes an explicit, observable lifecycle (**terminal** below means `Drained` or `Failed`):

```text
Initializing --publish--> Active; close/expiry -> Closing -> {Drained|Failed}; terminal only -> exact key+gen removal + slot/liveness release
```

- **Initializing** — reserved placeholder, one key+generation, one initializer, no leases (4.1, section 3).
- **Active** — admitted and leasable, reachable only from `Initializing` via 3.1.
- **Closing** — unleasable, **still in the map and discoverable** until terminal, so a caller/maintenance pass joins the in-progress teardown instead of racing a vanished entry; never deleted early (3.1, 3.2, 7.2).
- **Drained** — handoff observed, H3 driver stopped, request streams ended, QUIC connection/endpoint handle released, `Lifecycle` registration dropped.
- **Failed** — teardown could not complete gracefully, or nothing was left to drain; terminal via the explicit bounded force path (7.2), releasing liveness and never leaving a detached driver.

Terminal is the only outcome and the only removal point for the key+generation, releasing slot+liveness; no other removal path exists (3.1, 3.2).

### 7.2 Entry-owned supervised teardown

On entering `Closing`, a **supervised teardown task starts exactly once**; it — not any close caller — owns the initializer handoff plus guard, H3 driver/`JoinHandle`, shutdown signal, `Lifecycle` registration, and shared terminal completion. Every close path (owner close, idle expiry, connection-level logical deactivation, protocol-terminal failure, init after resource then close) converges on it:

- **Exactly one task** per generation: a second `Closing` transition returns the same completion, never another teardown.
- **Handoff supervised.** It holds the initializer task/`JoinHandle`/guard and awaits its single completion latch, closing any resource built (even after teardown was requested) under the state lock, so nothing stays in a removed generation and no `Active` is published once close won; the guard drops only at terminal, while aborting a caller drops only its await.
- **Callers only await.** Every close caller (concurrent `close()`, owner close, maintenance) awaits the shared completion; none drives, polls, or performs teardown work.
- **Abort cannot stop progress.** Dropping the first, **all**, or every exchange waiter drops only their awaits; the task keeps its initializer guard, driver/`JoinHandle`, shutdown signal, and liveness guard, so it progresses with **no surviving caller** and never needs a later close/drain pass.
- **Liveness held to terminal.** It keeps the `Lifecycle` registration until terminal, so no vanished caller drains it while the driver runs or the handoff is outstanding.
- **Terminal-only removal.** It alone removes the map entry and releases slot/liveness at terminal; no other path removes a served or `Closing` entry.
- **Bounded force path.** Peer never cooperates → cancel locally, close with an explicit QUIC code, reach `Failed` via the same completion.

Teardown is idempotent: replaying the completion returns the recorded outcome without re-running shutdown, re-closing, or double-releasing a guard.

### 7.3 Owner close sequence

1. **Stage one — real `Lifecycle` state.** `Lifecycle::begin_close` atomically turns the `Lifecycle` from `Open` to `Closing` and cancels the owner token, rejecting further `register` calls. This happens **before** any map lock is taken, so it is never "inside the map lock".
2. **Stage two — map-side linearization.** The owner-map critical section then, in order: (a) sets **`accepting=false`**; (b) marks **every `Initializing` and `Active` entry `Closing`/`TeardownRequested`** without removing its reservation/generation; (c) starts each entry's exactly-once supervised teardown (7.2). This one no-await lock section is the sole map-side admission-vs-close linearization: it fails an `Initializing -> Active` publication under the same lock (3.1) and prevents leasing a `Closing` entry. As `accepting=false` is set inside it, an exchange still between `Lifecycle::register` and map admission is rejected (4.1) rather than installing a stranded reservation, while an entry installed before (a) is captured by (b); a `Closing` reservation stays discoverable.
3. Teardown awaits the initializer handoff (closing any late-acquired resource); the owner close path then awaits each `Closing` completion like every other caller. Only at terminal does the task remove the entry, release its slot, and drop its liveness guard; only then do exchanges drain through `Lifecycle` to `finish_close`.

If an entry is already terminal, close is idempotent and only waits for the owner-level registrations.

### 7.4 Logical deactivation and final commit

A served/`Active` entry hitting a connection-level failure is **logically deactivated, not immediately removed**: under the map/state lock it transitions `Active -> Closing` by key+generation, immediately unleasable, then takes the same supervised teardown as every other close path, with physical removal only at terminal (7.2). A stale callback cannot touch a newer replacement, and stream-local errors do not transition the entry unless connection/driver health also proves terminal (R0b table: 0.2, section 9).

The final commit precedes stream permit release: a successful commit stays a success even if close starts immediately after; close/cancel/deadline winning before commit returns a typed error; no entry returns to the pool before the commit decision completes.

## 8. Resolver composition

Use the existing resolver snapshot read-only: DoQ uses `ResolverComposition::doq_endpoint(PublishedTarget, ServerIdentity)`; DoH3 uses `ResolverComposition::doh_endpoint(PublishedTarget, service_url)` with the existing `DohEndpoint`. The owner builds a key from the resulting endpoint and policy, so a selected A/AAAA address produces a distinct key without copying resolver state or changing the authenticated identity. Resolver refresh never proactively closes an established connection: the old key stays valid until idle/dead/owner teardown, and a new target creates or reuses its own key. No resolver algorithm, fallback, cross-family race, or socket policy is added.

## 9. Error and state matrix

R0b outcomes, one per behavior (exchange; entry; reuse; **Entry** from 0.2, not call sites; `NotSent`/`Sent`/`MaybeSent` independent): invalid key/zero port → pre-I/O error `NotSent`; none; no entry. Local slots full → backpressure `NotSent`; healthy; keep. Init failure, no resource → connect/TLS `NotSent`; `Closing`, task terminal; terminal. Init failure, resource exists → connect/TLS `NotSent`; `Initializing -> Closing`, handoff closed by task; terminal. Init close, no resource → `Closed(NotSent)`; `Closing`, never `Active`, task terminal; unavailable. Init close, resource exists → `Closed(NotSent)`; `Closing`, late resource handed to task; terminal. Same-key `Initializing` → join initializer under caller deadline, `Closed(NotSent)` if close wins; unchanged; one entry. Same-key `Closing` → `Closed(NotSent)`, no drain wait; slot unlent; retry after terminal removal. Stream open/send uncertainty → send/connect error; keep unless terminal; no replay. Peer reset/malformed response → DoQ/DoH3 terminal error, usually `Sent`; keep if healthy; reusable. Caller cancel/deadline → existing control error; keep if healthy; reusable. Cancel/deadline during `Initializing` → only that wait ends, initializer unaffected; `Initializing`/`Closing`, one completion; terminal or `Active`. Owner close → `Closed` with side-effect state; `Initializing`/`Active -> Closing`, drain; unavailable. Connection/driver terminal failure → connection error; logical `Active -> Closing`, removal at terminal only; terminal. Final commit → candidate until commit; keep after permit release; reusable after commit.

Preserve the closed side-effect vocabulary; never turn stream-local failure into an implicit retry or cross-protocol fallback — an unclassified error is a Slice 0 gap blocking Slice 1.

## 10. Compatibility and forbidden work

Preserve all existing one-shot behavior and tests. No 0-RTT/resumption, migration, Happy Eyeballs, TCP/DoT pipeline, SOCKS5, socket marks, interface/source binding, UDP retransmission, listeners, config, plugins, sequence wiring, API/WebUI, production wiring, Go/cgo/FFI, metrics, or general-purpose pool enters this task. The QUIC/H3 graph stays locked: a dependency change is a Slice 0 blocker requiring revised plan/review, and R0a/R0b/R0c follow — a pinned API that cannot satisfy the contract stops for re-review. Rollback is a task-scoped revert of the new module/exports/tests and narrowly required lifecycle/error additions; existing one-shot QUIC, secure, resolver, and generic TCP paths are unchanged, and unrelated dirty files are not staged or reverted.

## 11. Test architecture

- Slice 0: pure model tests, no socket or QUIC/H3 I/O, covering multi-key cap (4.1), same-key `Closing` (4.3), initializer-caller cancellation with zero surviving waiters plus init-vs-owner-close with late resource acquisition (3.1/11.1), the post-close admission race (4.1/7.3/11.2), the begin_close-to-accepting=false race (3.1/7.3/11.3), supervised teardown (7.2), and four-phase R0a (0.1); each fails if its contract is removed, and R0a is model evidence only.
- Slice 1: in-process DoQ fixtures plus accept counting, concurrent stream markers, one-stream cancellation, dead-connection replacement.
- Slice 2: H3 one-connection/multi-stream fixture, hold/release responses, driver liveness, cancellation without killing another stream (incl. R0a drop-only phases); Slice 2/A5 owns the real pinned-stack proof.
- Slice 3: peer stream-budget, bound/backpressure, idle-clock, concurrent close, resolver A/AAAA-to-key, bounded stress.
- Every slice asserts connection, stream/request, and lifecycle counts, final commit, and no late success at its boundary.

### 11.1 Aborted-at-barrier/no-surviving-caller supervised-teardown test

Slice 0 form of 3.1/7.2 (no socket); held-barrier liveness first, post-release terminal second. Setup: an installed `Initializing` entry whose initializer sits behind an **initializer barrier**, plus a **teardown barrier** before terminal.

1. Abort/cancel the **first initializer caller** and drop **every exchange waiter**: the initializer task/`JoinHandle` stays alive and yields **exactly one** completion once released — no waiter need poll it.
2. Close (or expiry) wins: the entry is marked `Closing`/`TeardownRequested`, its reservation is **not removed**, no `Active` may be published; drop **all** close waiters. Held-barrier assertions only: `Closing` stays **discoverable**; the task owns the handoff, driver/`JoinHandle`, shutdown signal, and liveness guard; `Lifecycle` has not drained; no completion reported.
3. Release both barriers: no `Active`, the generation survives, the resource is handed to and closed by the task (no orphan, no late-resource race, no second generation); exactly one teardown reaches terminal **autonomously**, removing the entry and releasing slot/liveness only there; no stranded `Initializing`/`Closing`, no late response.

Fails if any abort path stops the initializer, a `Closing` reservation is removed early, `Active` is published after close won, a late resource is orphaned, teardown stalls with no callers, `Lifecycle` drains early, or removal happens outside terminal.

### 11.2 Post-close admission (register-but-not-yet-installed) race test

Slice 0 form of 4.1/7.3 (no socket):

1. Exchange A completes `Lifecycle::register` while `Open`, then parks before the owner-map admission section (4.1); no reservation exists yet.
2. Owner close: `begin_close` transitions the `Lifecycle`, then the map section sets `accepting=false` and captures every then-existing `Initializing`/`Active` entry as `Closing`/`TeardownRequested` with exactly-once supervised teardown (7.3).
3. Release A: admission returns `Closed(NotSent)` at the `accepting` check, installing no reservation, initializer, or second generation, and releasing A's registration plus any local liveness guard — zero `Lifecycle` liveness and slot residue, so `Lifecycle` drains and `finish_close` completes; the map gains no `Initializing` entry or extra initializer task/liveness, while entries captured in step 2 still finish via the existing `Closing -> terminal` protocol (terminal-only removal).

Fails if an admission installs after `accepting=false`, leaks registration/liveness/slot when rejected, creates a stranded `Initializing`, or lets close-captured entries bypass the existing teardown protocol.

### 11.3 begin_close-to-accepting=false publication race test

Slice 0 form of 3.1/7.3 (no socket): installed `Initializing` entry, owner close advanced only through stage one.

1. Owner close completes `Lifecycle::begin_close` (`Lifecycle` now `Closing`, `register` rejected) while the owner-map critical section is parked at a barrier, so `accepting` is still `true` and no entry is marked.
2. Release the initializer's publication attempt under the map/state lock: it sees `Lifecycle != Open`, fails the three-way condition, and takes the `Closing`/`TeardownRequested` late-resource/supervised-teardown path — no `Active`, nothing leasable.
3. Held-barrier assertions only: the `Closing` reservation stays **discoverable**; teardown is alive and owns the handoff, driver/`JoinHandle`, shutdown signal, and liveness guard; no completion; `Lifecycle` has not drained.
4. Release the map barrier (stage two sets `accepting=false`, captures the entry): same non-`Active` outcome plus exactly one teardown to terminal with removal and slot/liveness release inside the task.
5. Inverse order: stage two first (`accepting=false`, entry captured), then the initializer — publication fails at the gate too.

Fails if either order publishes `Active`, a single-check publication succeeds, a `Closing` reservation is removed early, or removal happens outside terminal.
