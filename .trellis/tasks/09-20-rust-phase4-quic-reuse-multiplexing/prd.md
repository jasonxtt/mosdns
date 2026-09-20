# Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. This task authorizes no implementation until the final
planning summary is explicitly approved, `task.py start` is run, and the
selected external executor receives a bounded slice assignment.

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

- `rust/upstream-core/src/quic.rs:1-66` and the `DoqUpstream`/`Doh3Upstream`
  definitions (`:210-217`, `:725-738`) implement fresh one-shot connections;
  pooling, reuse, and multiplexing are explicitly deferred there.
- `Lifecycle` in `rust/upstream-core/src/lib.rs:585-826` serializes owner
  admission with `Open -> Closing`, provides shared liveness registration for
  children, and exposes the only crate-level final response commit gate.
- `ExchangeContext`/`ExchangeControl` and `SideEffectState` are the shared
  absolute-deadline, caller/owner-cancellation, and send-state vocabulary; the new
  owner consumes them rather than creating a second control race.
- `rust/upstream-core/src/reuse.rs:1-27` limits the existing owner to serial
  TCP/DoT/DoH reuse; its `MAX_PENDING_PER_CONNECTION = 1` and idle pool are not a
  QUIC design, so this task needs a separate owner and key/state model.
- `DoqEndpoint` and the one-shot DoQ path already enforce numeric dial plus separate
  `ServerIdentity`, exact `doq` ALPN, zeroed outbound DNS ID, stream FIN, response
  ID validation, and original-ID restoration.
- The one-shot DoH3 path already reuses `DohEndpoint::get_request_target`, exact
  `h3` ALPN, the bounded DoH response validator, and a tracked short-lived H3
  driver; its `H2ScopeLease`-style teardown is not a long-lived connection owner.
- `ResolverComposition::doq_endpoint` in
  `rust/upstream-core/src/resolver/owner.rs:900-915` consumes only
  `PublishedTarget::dial()` while preserving caller identity; DoH3 uses the
  existing `doh_endpoint` composition with `DohEndpoint`.
- `TlsPolicy` carries a verification mode and opaque `roots_revision` for reuse
  identity, keeping 0-RTT early data and session resumption disabled.
- The locked QUIC/H3 graph is already in `rust/upstream-core/Cargo.toml`; Slice 0 is
  model-only and adds no dependency.

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

Owner teardown is cancellation-safe through an **entry-owned supervised teardown
task**. An entry moves through an explicit
`Initializing -> Active -> Closing -> Drained | Failed` lifecycle and stays
discoverable until it reaches `Drained` or `Failed`. When an entry enters
`Closing`, exactly one entry-owned supervised teardown task starts and owns the
H3 driver/`JoinHandle`, the driver shutdown signal, the entry's `Lifecycle`
liveness guard, and the shared completion. Every close caller (including the
owner close path) merely awaits that one shared completion. Aborting the first
close caller's future, or **all** close caller futures, cannot stop teardown
progress, cannot detach the driver, and cannot lose liveness: the supervised task
reaches `Drained`/`Failed` and removes the entry from the map on its own, with no
surviving caller and without relying on a later close/drain pass.

Initializer execution is equally entry-owned and cancellation-safe: installing an
`Initializing` reservation starts and holds one entry-owned initializer task with a
`JoinHandle` (or equivalent entry-owned shared future plus guard), so no exchange
caller owns, polls, or must drive it. Every same-key caller only awaits the shared
completion, and its cancellation/deadline/`ExchangeControl` ends only its own wait —
dropping the last waiter, or every caller, cannot stop the initializer, which still
yields exactly one completion for the supervised teardown (`design.md`
§3/§3.1/§7.2).

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

`MAX_CONNECTIONS_PER_OWNER = 8` admission is **atomic across keys**. A single
no-await owner-map critical section carries one model-only **`accepting` (Open)
gate** and performs, in order: **check `accepting` first** (if false, install
nothing, start no initializer, return `Closed(NotSent)`); lookup for the key;
transition of dead and idle-expired entries to `Closing` **without removal**;
capacity check against the entry bound while **counting `Initializing`,
`Closing`, and `Active` entries as occupied capacity until they reach the
terminal `Drained`/`Failed`**; and either reuse/join of the existing entry or
installation of a new `Initializing` placeholder with a generation identity. No
entry-teardown path — including initialization failure with or without a
transport/H3 resource — bypasses that terminal removal, so a slot is never freed
early. No `await` may occur inside that section, and no two concurrent admissions
for different keys may both observe a below-capacity map and over-commit. A
separate map lock, a check-then-insert split, a per-key lock, installing without
checking `accepting`, or reusing a `Closing` entry's slot before its drain
completes does not satisfy this requirement. Pure validation failures that
construct no entry at all (for example an invalid key or zero port) are pre-I/O
errors, not entry teardown, and are resolved before this section.

This is the **sole map-side admission-vs-close linearization** (first of the
two-stage owner close, R6): an exchange that completed `Lifecycle::register` but
reaches this section after close set `accepting=false` must release that
registration and any local liveness guard before returning `Closed(NotSent)` — no
reservation or initializer, no second generation, zero liveness/slot residue —
while an admission installed inside the lock is ordered against close and captured
as `Closing`/`TeardownRequested`. Publication into `Active` likewise requires
`Lifecycle == Open`, `accepting == true`, and this exact generation still
`Initializing` under that same lock; the real `Lifecycle` state and the map
admission gate are independent and neither substitutes.

A same-key lookup resolves by the found entry state: an `Active` entry is leased
normally; an `Initializing` entry is joined through the single-flight initializer
under the caller's own deadline/control race; a `Closing` entry returns the
repository's existing typed pre-send closed result
`UpstreamError::Closed(SideEffectState::NotSent)` without waiting on drain,
without opening a second generation for that key, and without leasing the
`Closing` entry's slots. The caller may retry the same key in a later independent
admission, which may admit a fresh generation after the old entry reaches
terminal removal.

### R6. Failure, replacement, and teardown

- A reservation is installed as an explicit `Initializing` placeholder with a
  generation identity under the owner-map/state lock; a `Closing` entry stays
  discoverable but unleasable.
- Owner close is **two-stage**. **Stage one:** `Lifecycle::begin_close` turns the
  real `Lifecycle` `Open -> Closing` and rejects further `register` calls, before
  any map lock is taken (`begin_close` is never executed inside it). **Stage two:**
  the owner-map critical section sets `accepting=false` (the sole map-side
  admission-vs-close gate), marks every `Initializing`/`Active` entry
  `Closing`/`TeardownRequested` without removing its reservation/generation, and
  starts its exactly-once supervised teardown. Since `accepting=false` is set in
  the same lock, an exchange still between registration and map admission is
  rejected rather than stranded.
- Installing the reservation starts and holds one **entry-owned initializer task
  with a `JoinHandle`** (or equivalent entry-owned shared future plus guard); no
  exchange caller owns, polls, or drives it. All same-key callers only await the
  shared completion, and a caller's cancellation/deadline/`ExchangeControl` ends
  only its own wait; dropping the last waiter, or every caller, cannot stop it
  (entry-owned, never spawn-and-forget, released only at terminal), so it delivers
  exactly one completion and an aborted leader caller can never strand an
  `Initializing`/`Closing` entry.
- The initializer builds outside the lock and hands its single result to the
  entry-owned teardown/owner state under the same lock/handoff protocol: it may
  publish `Active` (the only publication point) only when `Lifecycle == Open`,
  `accepting == true`, and this exact generation is still `Initializing` all hold.
  Because stage one alone already makes `Lifecycle != Open`, an initializer that
  wins the lock after `begin_close` but before `accepting=false` still must not
  publish `Active`; in either order it never publishes `Active` and never leaves a
  resource in a removed generation — the whole result, including a late-acquired
  resource, goes to the supervised teardown, and a failed initializer still
  delivers its completion so teardown finishes promptly on one explicit terminal
  outcome.
- Entering `Closing` starts exactly one entry-owned supervised teardown task owning
  the initializer completion/handoff, H3 driver/`JoinHandle`, shutdown signal,
  liveness guard, and shared completion. Concurrent `close()` calls are idempotent,
  converge through the existing lifecycle machine, and merely await that one
  completion.
- Aborting the first close caller, or **all** close callers, must not detach the
  driver/`JoinHandle`, drop the entry's liveness registration before drain, or
  strand the entry in `Closing`; teardown progress is owned by the supervised task,
  never a caller future, so it still reaches `Drained`/`Failed` and removes the map
  entry with no surviving caller and without a later close/drain pass.
- An entry exposes `Initializing -> Active -> Closing -> Drained | Failed`.
- A stream-local reset, malformed response, or local query cancellation does not
  deactivate a healthy connection merely because the query failed.
- A QUIC connection close, H3 driver terminal failure, endpoint failure, or
  unusable shared transport **logically deactivates** the exact key/generation:
  `Active -> Closing` under the map/state lock, immediately unleasable; physical
  removal happens only at `Drained`/`Failed` through the supervised teardown, and no
  query is replayed after bytes may have been sent.
- Idle expiry and explicit close mark the entry `Closing` under the owner-map lock;
  only an `Initializing -> Active` published entry is leasable, so a `Closing` entry
  rejects new leases without removal, stays discoverable, and is removed only at
  `Drained`/`Failed` — no new lease or concurrent close caller races a missing entry.
- Only terminal `Drained`/`Failed` performs exact key+generation physical removal
  and releases slot/liveness: no late-resource race, early drain, stranded
  `Closing`, second generation, or reliance on a later close/drain pass.
- No response may commit after close wins.

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

### R0. Pre-start contract gates (blocking, Slice 0 exit)

R0 is a **blocking predecessor of Slice 1**, so it is listed last and numbered
separately from the R1-R8 requirements rather than implying it runs after them.
R0 produces the pinned-API evidence and the two classification contracts that
every later slice consumes. It is in scope for this task and is not optional
follow-up.

R0a. **H3 request cancellation is core.** Define the safe per-phase local
cancellation of one H3 request stream across the pinned
`h3 0.0.8` / `h3-quinn 0.0.10` API: before send, after request FIN, during
response head, and during body read. Cancellation must reset or stop only the
offending request stream where the pinned API permits it, must not invoke the
known panic-prone path (an aborted `poll_data` leaves the h3-quinn receive
stream as `None`, and a subsequent `RequestStream::stop_sending` unwraps it),
and must never close a healthy shared H3 connection. If the pinned API cannot
express a safe per-phase cancellation for a phase, that phase's contract is an
explicit documented drop-only teardown, and the task stops for re-review rather
than changing dependencies.

The R0a evidence boundary is precise: **Slice 0 proves the four-phase
decision/state model only** — no socket, no QUIC/H3 I/O — by showing the model
never selects the pinned `Option::None` `stop_sending` path and keeps the logical
shared-entry state healthy. The **real pinned-stack H3 loopback proof** that
canceling one actual request leaves the shared connection, the driver, and
another concurrent request healthy belongs to Slice 2/A5 and must not be claimed
as Slice 0 or R0 evidence.

R0b. **Connection-level versus stream-level error classification is core.**
Produce the authoritative mapping from the pinned `h3`/`h3-quinn`/`quinn` error
vocabulary to exactly one of: stream-local (keep a healthy entry), or
connection/entry-terminal (logically deactivate the exact key+generation by
`Active -> Closing`, with physical map removal only at `Drained`/`Failed`). Every
DoQ and DoH3 path in later slices must use this mapping; no path may infer
deactivation from an arbitrary error or from a bare `Result` shape, and no path
may replay a query whose bytes may have been sent.

R0c. **Pinned API assumptions are frozen.** The API facts recorded in
`research/quic-reuse-evidence.md` for `quinn 0.11.7`, `h3 0.0.8`, and
`h3-quinn 0.0.10` are the assumptions R0 verifies against the locked
local registry source. R0 must pin each assumption to an exact source location
and record whether it holds. **No dependency may be added, removed, or
version-bumped to satisfy this contract.** If a pinned API demonstrably cannot
satisfy R0a or R0b, the task stops and returns to planning review with the
evidence.

R0d. **Atomic admission, initialization, and teardown contracts are fixed here,
implemented in Slice 0:** the one-lock `accepting`-gated admission rule (R5), the
single initialization crossing/handoff protocol including `Initializing -> Active`
publication (R6), and the entry-owned supervised teardown
`Initializing/Active -> Closing -> Drained/Failed` that alone performs terminal
removal (R3/R6) — specified in `design.md` §3/§4/§7 and not deferred.

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
      failure instead logically deactivates the exact key+generation by
      `Active -> Closing` (immediately unleasable; physical map removal only at
      `Drained`/`Failed`), and the next independent query establishes a
      replacement without replaying the failed query. A same-key admission that
      lands while the old entry is still `Closing` returns `Closed(NotSent)`
      with no drain wait and opens no second generation; the replacement becomes
      admissible only after the old entry reaches terminal removal.
- [ ] A4. DoH3 loopback accepts one QUIC/H3 connection for multiple concurrent
      GET request streams, preserves `:authority`/path and the existing DoH
      response contract, restores each original ID, and proves the driver stays
      alive between requests.
- [ ] A5. DoH3 owner close drains the long-lived driver and every request
      stream, rejects new admissions, converges under concurrent/repeated close,
      and leaves zero registered child/stream residue. A canceled request does
      not close a healthy connection used by another request. A deterministic
      aborted-at-barrier/no-surviving-caller test proves the strong contract:
      entering `Closing` starts exactly one entry-owned supervised teardown task
      that owns the initializer completion/handoff plus guard, the
      driver/`JoinHandle`, the shutdown signal, the liveness guard, and the shared
      completion. Initializer execution itself is entry-owned: aborting the first
      initializer caller and dropping **every exchange waiter** cannot stop it, and
      aborting the first close waiter and then dropping **all** close waiter
      futures cannot stop teardown; the task reaches `Drained`/`Failed`, removing
      the entry and releasing its slot/liveness only at that terminal, with no
      surviving caller and without a later close/drain pass. The real pinned-stack H3 loopback proof
      (one canceled request leaves the shared connection, the driver, and another
      concurrent request healthy) is part of this criterion.
- [ ] A6. Local stream-slot and owner-entry bounds are finite and observable;
      exhaustion returns a typed pre-send backpressure result without an
      unbounded queue. Peer stream-limit exhaustion is controlled by the caller's
      original deadline/cancellation race. A deterministic multi-key concurrency
      test proves `MAX_CONNECTIONS_PER_OWNER` cannot be exceeded: many
      simultaneous admissions for distinct keys never leave more than the cap of
      live entries, and the admitted-key count equals the cap exactly when the
      cap is reached. `Initializing` and `Closing` entries still occupy their
      slots until the terminal `Drained`/`Failed` removal, so a slot is not
      reusable early. A deterministic same-key test
      proves that a lookup finding `Closing` returns `Closed(NotSent)` with no
      wait on drain, creates no second generation, and does not lease the
      `Closing` slot. A post-close admission test proves that an
      exchange registered but not yet admitted when `accepting=false` is
      linearized returns `Closed(NotSent)` with no reservation, initializer,
      second generation, or residue.
- [ ] A7. Idle expiry transitions an unused connection to `Closing` under the
      owner-map lock according to the injected clock/maintenance path, so it is
      no longer leasable while it stays discoverable until `Drained`/`Failed`;
      a connection used before expiry remains reusable. Expired/dead entries
      cannot be returned after owner close.
- [ ] A8. Existing lifecycle precedence and final commit behavior remain
      observable for DoQ and DoH3: owner close, caller cancellation, deadline,
      and a successful final response cannot be reordered into a late success.
      A deterministic initialization-versus-owner-close barrier test proves the
      crossing protocol: the first initializer caller is aborted and every exchange
      waiter dropped first, yet the entry-owned initializer still produces exactly
      one completion; close then wins the shared lock first, the `Initializing`
      reservation is not removed, and the initializer completes and acquires its
      resource; the entry never publishes `Active`, the generation does not
      disappear, the resource is taken over and closed by the supervised teardown
      (no orphan, no late-resource race, no second generation), `Lifecycle` does
      not drain early, and removal plus slot/liveness release happen only at the
      terminal `Drained`/`Failed`.
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
- [ ] A12. R0 pre-start gates are closed before any Slice 1 network work:
      the per-phase H3 request-stream cancellation contract (R0a) is documented
      and exercised by the Slice 0 **decision/state model only** (no socket or
      QUIC/H3 I/O), which proves the model never selects the pinned `Option::None`
      `stop_sending` path and keeps the logical shared-entry state healthy; the
      real pinned-stack H3 health proof is deferred to Slice 2/A5 and is not
      claimed here. The connection-level versus stream-level error classification
      is enumerated against the pinned `h3`/`h3-quinn`/`quinn` vocabulary (R0b),
      and every pinned API assumption is verified against the locked local
      registry source (R0c). The dependency graph is unchanged; a pinned API that
      cannot satisfy the contract is reported for re-review rather than worked
      around.
- [ ] A13. Owner admission and entry teardown are covered by deterministic
      Slice 0 model tests: the multi-key cap test (A6), the same-key `Closing`
      lookup test (A6), the post-close admission race test (A6), the
      init-vs-owner-close barrier test (A8, including initializer-caller abort
      with zero surviving waiters, close-wins-then-late-resource acquisition, and
      the begin_close-to-accepting=false publication race), and the
      aborted-at-barrier/no-surviving-caller supervised-teardown test (A5) all
      fail if the atomic no-await admission section with its `accepting` gate,
      the single initialization crossing/handoff protocol, the entry-owned
      cancellation-safe initializer execution, or the supervised teardown
      contract is removed.

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

- The owner is QUIC-specific and key-driven, not a specialization of serial TCP
  `ReuseOwner`; one key has one physical connection, concurrency is stream-level
  and bounded, and multiple connections per key would need a separate policy review.
- DoQ and DoH3 stream association are both structural (no ID demux; the H3
  request-stream handle).
- H3 driver ownership is the highest lifecycle risk: the implementation must prove
  driver admission, shutdown, stream cancellation, and drain with real loopback
  tests, and one-shot H2 scope machinery is evidence, not the new owner. Teardown is
  cancellation-safe through one entry-owned supervised teardown task — a `Closing`
  entry stays discoverable, the task owns the driver/`JoinHandle`, shutdown signal,
  liveness guard, and shared completion, every close caller only awaits it, and
  aborting one or all close futures cannot stop it.
- H3 request cancellation (R0a) and connection-versus-stream error classification
  (R0b) are core blockers, resolved by pinned-API evidence in Slice 0 before Slice 1
  network work; a pinned API that cannot satisfy them stops for re-review rather
  than a dependency change. Slice 0 resolves R0a at the decision/state-model level;
  the real pinned-stack H3 health proof is Slice 2/A5.
- Initialization is an explicit `Initializing` state: publication and the owner/entry
  close decision share one lock linearization point, so an initializer observing
  `Closing`/`TeardownRequested` never publishes `Active` and hands its
  completion/handoff to the same entry-owned supervised teardown. Neither owner close
  nor initialization failure — with or without a transport/H3 resource — bypasses
  that task: one completion always arrives, teardown records a terminal
  `Drained`/`Failed` promptly (nothing to drain with no resource), a late-acquired
  resource is closed by the same task, and only that terminal removes the exact
  key+generation and releases slot/liveness.
- A same-key lookup finding `Closing` returns the existing `Closed(NotSent)` with no
  drain wait, no second generation, and no lease of the `Closing` slot; it may retry
  after terminal removal.
- `MAX_CONNECTIONS_PER_OWNER` is enforced by one no-await map critical section
  carrying the `accepting` gate and performing gate check, lookup, `Closing`
  transition of dead/idle-expired entries (without removal), capacity check counting
  `Initializing`/`Closing`/`Active` until terminal `Drained`/`Failed`, and
  reservation/join/reuse together; a check-then-insert split, an unguarded install,
  or early `Closing`-slot reuse is a concurrency bug.
- A served/`Active` entry hitting a connection-level failure is logically deactivated
  by exact key+generation `Active -> Closing`, immediately unleasable, with physical
  removal and slot/liveness release only at terminal `Drained`/`Failed` — the **only**
  entry removal path, including for an initialization that never acquired a resource;
  pre-entry validation failures (invalid key/zero port) are not teardown.
- The exact task-local bounds are not product behavior: implementation evidence may
  revise the constants without changing the public contract, but they must stay
  finite and the backpressure/close semantics explicit.

## Blocking open questions

No unresolved product/scope question blocks planning: the user decision fixed the
task name, four-slice scope, exclusions, reuse-key dimensions, lifecycle
vocabulary, and external-executor/web-review routing, and the numeric bounds are
implementation-level choices, not unresolved product behavior. Four technical
questions are explicitly blocking Slice 1 under the R0 pre-start gate:

- **BQ1 (R0a).** Which exact per-phase cancellation sequence is safe through the
  pinned `h3 0.0.8` / `h3-quinn 0.0.10` API, given that an aborted `poll_data`
  leaves the internal receive stream as `None` and a subsequent `stop_sending`
  unwraps it? R0 must answer for the four phases with the Slice 0
  decision/state-model test; a phase with no safe cancellation is drop-only.
- **BQ2 (R0b).** Which pinned `h3`/`h3-quinn`/`quinn` error variants prove the
  physical connection terminal versus stream-local? R0 enumerates the mapping.
- **BQ3 (R0c).** Do the recorded API assumptions still hold against the locked
  local registry source? A mismatch is a re-review stop, not a dependency change.
- **BQ4.** Do the proposed numeric bounds survive the loopback peer stream-limit
  fixtures? Values may be tuned while keeping the bounded/no-queue and
  atomic-admission contracts fixed.

None may be answered by adding, removing, or bumping a dependency, and none may be
deferred to a post-Slice-0 follow-up task.

## Planning notes

- Dependencies already exist and were audited by the archived
  `09-18-rust-phase4-quic-http3-doq-foundation` task; no new dependency is
  expected.
- Preserve unrelated dirty files and `.DS_Store` files. Do not use
  `git add -A`. Trellis auto-commit remains disabled.
- The old CI task mentioned in the user's bookkeeping note is already fully
  archived by HEAD `eb40379`; only the handover document needed correction.
