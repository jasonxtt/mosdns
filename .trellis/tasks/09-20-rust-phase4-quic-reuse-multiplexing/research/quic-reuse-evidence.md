# QUIC reuse planning evidence

Recorded: 2026-09-20. This file is planning evidence, not a replacement for
the implementation contract in `prd.md` or the design in `design.md`.

## Existing MosDNS evidence

- `rust/upstream-core/src/quic.rs:1-66` documents the existing QUIC foundation:
  DoQ and DoH3 are fresh one-shot exchanges, H3 driver ownership is scoped to
  one exchange, and pooling/reuse is deferred.
- `rust/upstream-core/src/quic.rs:210-217` makes the one-shot DoQ
  connection lifetime explicit; `:725-738` does the same for DoH3.
- `rust/upstream-core/src/quic.rs:718-723` fixes the current H3 type
  relationship: a client `Connection`, cloneable `SendRequest`, and
  per-request `RequestStream`.
- `rust/upstream-core/src/quic.rs:969-1043` shows the current H3
  sequence: build H3, spawn a tracked driver, run one request, seal/drain the
  driver, commit, then close the QUIC endpoint. Reuse needs the same ownership
  discipline with the driver lifetime moved to a connection entry.
- `rust/upstream-core/src/reuse.rs:1-27` explicitly limits the existing
  reuse implementation to serial TCP/DoT/DoH and says QUIC/HTTP3 are separate.
- `rust/upstream-core/src/reuse.rs:61-79` confirms the old pool constants
  are serial-TCP decisions and must not be copied as a QUIC concurrency contract.
- `rust/upstream-core/src/lib.rs:585-826` is the authoritative
  lifecycle model: admission and close share one mutex, shared liveness guards
  can outlive a caller future, and `commit_final_response` is the final
  control-aware response linearization point.
- `rust/upstream-core/src/secure/endpoint.rs` separates numeric dial
  from `ServerIdentity` and DoH service authority/path.
- `rust/upstream-core/src/secure/tls.rs` gives each verified policy an
  opaque roots revision and keeps early data/resumption disabled.
- `rust/upstream-core/src/resolver/owner.rs:858-915` already composes a
  `PublishedTarget` into DoQ, DoT, and DoH endpoints without rewriting
  identity. DoH3 can consume the existing DoH composition.

## Archived task evidence

- Archived task
  [09-18-rust-phase4-quic-http3-doq-foundation](../../../tasks/archive/2026-09/09-18-rust-phase4-quic-http3-doq-foundation/)
  (`.trellis/tasks/archive/2026-09/09-18-rust-phase4-quic-http3-doq-foundation/`)
  establishes the protocol foundation and explicitly defers QUIC connection
  reuse/pooling, stream-depth/backpressure, and H3 reuse. Its dependency audit
  records exact locked versions `quinn 0.11.7`, `h3 0.0.8`, and
  `h3-quinn 0.0.10`, all already present in the current manifest. This
  task therefore does not need a dependency admission slice unless the
  implementation unexpectedly requires a new crate.
- Archived task
  [09-18-rust-phase4-connection-reuse-pipeline](../../../tasks/archive/2026-09/09-18-rust-phase4-connection-reuse-pipeline/)
  (`.trellis/tasks/archive/2026-09/09-18-rust-phase4-connection-reuse-pipeline/`)
  is useful only for contrast: its owner is serial per connection with one
  outstanding query. Its key dimensions and Lifecycle integration are evidence
  for what must remain safe, not a template for multiplexing.

## Local locked-crate API evidence

The local Cargo registry was inspected at the locked versions. Each item below
carries the exact locked local registry source location that R0c must
re-verify (paths are relative to `~/.cargo/registry/src/<registry>/<crate>-<version>/`):

- `h3 0.0.8` `client::Builder::build` returns a driver and
  cloneable `SendRequest`; `SendRequest::send_request` takes
  `&mut self`, and `SendRequest` implements `Clone`
  (`h3-0.0.8/src/client/connection.rs:109`, `:225-245`).
- `h3 0.0.8` `client::Connection::poll_close` must be polled
  continuously; `shutdown` initiates graceful shutdown and
  `wait_idle` waits for closure
  (`h3-0.0.8/src/client/connection.rs:384`, `:391`, `:397`).
- `h3-quinn 0.0.10` makes `OpenStreams` cloneable and maps each
  H3 request to a fresh Quinn bidirectional stream
  (`h3-quinn-0.0.10/src/lib.rs:189-252`).
- `h3 0.0.8` exposes `RequestStream::stop_stream` for the send
  direction and `stop_sending` for the receive direction
  (`h3-0.0.8/src/client/stream.rs:225`, `:251`). The local
  implementation must account for the existing h3-quinn behavior where an
  interrupted read can temporarily move the underlying receive stream into an
  internal `Option::None` (`h3-quinn-0.0.10/src/lib.rs:375-387`), after which
  `stop_sending` unwraps that option
  (`h3-quinn-0.0.10/src/lib.rs:391-397`).
- `quinn 0.11.7` `Endpoint` is cloneable (`#[derive(Debug, Clone)]`, and a
  refcounted `EndpointRef` whose `clone` bumps the count) and its endpoint driver
  is spawned by the crate's endpoint construction; the owner must keep the
  endpoint handle alive and use `close` / `wait_idle` during teardown
  (`quinn-0.11.7/src/endpoint.rs:47`, `:292`, `:316`, `:669-708`).
- `quinn 0.11.7` `quinn::Connection` is cloneable
  (`#[derive(Debug, Clone)] pub struct Connection(ConnectionRef)`), `open_bi` is
  per-stream, and `closed` / `close` provide connection-level health/teardown
  (`quinn-0.11.7/src/connection.rs:291`, `:316`, `:361`, `:420`).
- Quinn's local `RecvStream::Drop` stops unread receive data with code zero
  (`conn.inner.recv_stream(self.stream).stop(0u32.into())`) when the stream was
  not fully read (`quinn-0.11.7/src/recv_stream.rs:500-515`). This is evidence
  that H3 cancellation needs an explicit phase-aware stream teardown test; it is
  not permission to close the shared connection.

All Quinn/H3 citations above are paths inside the **locked local registry
source** (`~/.cargo/registry/src/<registry>/`), not files copied into this
repository.

Locked package locations, symmetric across all three pins: `quinn 0.11.7` /
`h3 0.0.8` / `h3-quinn 0.0.10` are pinned with `default-features = false` in
`rust/upstream-core/Cargo.toml:80-82`, and `rust/Cargo.lock` locks them at
`h3 0.0.8` (`:377-380`), `h3-quinn 0.0.10` (`:391-394`), and `quinn 0.11.7`
(`:792-795`). These are the frozen assumptions R0c verifies; a mismatch is a
re-review stop and never a dependency change.

## Planning decisions derived from the evidence

1. Use a new QUIC-specific key/owner and do not extend the serial TCP pool.
2. Keep one physical connection per key in this task. Use per-stream permits
   for bounded concurrency and use generation identity to select the exact entry
   for logical deactivation/`Closing`. Any installed entry or `Initializing`
   reservation is removed from the map, and has its slot/liveness released, only
   at the terminal `Drained`/`Failed`: every initialization failure — whether or
   not it acquired a transport/H3 resource — delivers its completion to the same
   entry-owned supervised teardown, which finishes promptly with an explicit
   terminal outcome when there is nothing to drain. There is no entry-teardown
   exception. Only a pre-entry validation failure (for example an invalid key or
   zero port) creates no entry at all, is not teardown, and may be returned
   directly before admission.
3. Make H3 driver lifetime a connection-entry responsibility. A request stream
   may fail without killing a healthy H3 connection; a driver/connection
   terminal failure logically deactivates the exact key+generation
   (`Active -> Closing`, immediately unleasable) and the supervised teardown
   removes it only at `Drained`/`Failed`.
4. Keep the existing final commit gate after response validation and before
   releasing the request permit.
5. Use resolver-selected numeric `PublishedTarget::dial()` only in the
   key; identity, authority, and TLS policy remain independent fields.
6. Keep the dependency graph locked and defer all socket policy, retransmission,
   listener, host, config, and production wiring.
7. Enforce `MAX_CONNECTIONS_PER_OWNER` in one no-await owner-map critical section
   carrying a model-only `accepting` (Open) gate: check it first (reject with
   `Closed(NotSent)` and start no initializer), then lookup, mark dead/idle-expired
   entries `Closing` (without removal), capacity-check counting
   `Initializing`/`Closing`/`Active` as occupied until terminal `Drained`/`Failed`,
   and reservation/join/reuse together. This is the sole map-side
   admission-vs-close linearization: owner close sets `accepting=false` in the same
   lock before marking entries, so an exchange registered but not yet admitted is
   rejected and must release its registration plus any local liveness guard with no
   residue. A check-then-insert split, installing without checking `accepting`, or
   reusing a `Closing` slot before terminal, is a concurrency bug.
8. Make `Initializing` an explicit state and freeze one crossing protocol with
   cancellation-safe execution ownership: installing the reservation starts and
   holds one **entry-owned initializer task with a `JoinHandle`** (or an
   equivalent entry-owned shared future plus guard), never a caller-owned future
   and never spawn-and-forget. Same-key callers only await the shared completion,
   and a caller's cancellation/deadline/`ExchangeControl` ends only its own wait,
   so dropping the last waiter or every caller cannot stop the initializer. Owner
   close marks every `Initializing`/`Active` entry `Closing`/`TeardownRequested`
   at the shared map/state linearization point without removing the reservation,
   and entering `Closing` starts the supervised teardown exactly once. The
   initializer builds outside the lock and hands one result back under that same
   lock/handoff protocol: it publishes `Active` only while the owner is `Open`
   and the generation is still `Initializing`; otherwise it never publishes and
   hands the whole result, including a late-acquired resource, to the supervised
   teardown. No resource yields a terminal `Failed` with nothing to drain, and
   only terminal `Drained`/`Failed` removes the exact key+generation and releases
   slot+liveness, so an aborted leader caller can never strand an
   `Initializing`/`Closing` entry.
9. Give each entry an explicit
   `Initializing -> Active -> Closing -> Drained | Failed` lifecycle. Entering
   `Closing` starts exactly one entry-owned supervised teardown task that owns the
   initializer completion/handoff, driver/`JoinHandle`, shutdown signal, liveness
   guard, shared completion, and terminal-only map removal plus slot/liveness
   release. Close callers only await the shared completion, so aborting one or all
   of them cannot stop teardown or detach the driver, and teardown never relies on
   a later close/drain pass.
10. Resolve a same-key lookup by state: `Active` leases, `Initializing` joins the
    single-flight initializer under the caller deadline, and `Closing` returns
    the existing `Closed(NotSent)` vocabulary with no drain wait, no second
    generation, and no lease of the `Closing` slot.

## R0 gate items (blocking, resolved in Slice 0)

These are **not** deferred follow-up work. They are R0 pre-start gates and must
be closed before any Slice 1 network work:

- **R0a.** Confirm the safe h3 request-stream cancellation decision for each
  phase (before send, after request FIN, during response head, and during body
  read) without invoking the pinned `Option::None` unwrap panic. Record which
  phases are active-stop and which are drop-only, and add the Slice 0
  **decision/state-model** four-phase test. Slice 0 has no socket or QUIC/H3 I/O,
  so this is model evidence only: it proves the model never selects the panic
  path and keeps the logical shared-entry state healthy. The real pinned-stack
  H3 loopback proof belongs to Slice 2/A5 and must not be claimed here. A phase
  with no safe active stop is documented as drop-only; it is not a
  dependency-change trigger.
- **R0b.** Enumerate which Quinn/H3 errors prove the physical connection is dead
  versus only terminating one request stream, as the single classification table
  later slices consume. Entry-terminal errors logically deactivate by exact
  key+generation `Active -> Closing`; physical map removal stays a terminal
  `Drained`/`Failed` concern. Derive the table from the pinned sources, not from
  a `Result` shape.
- **R0c.** Re-verify every pinned API assumption above against the locked
  local registry source with the cited file/line, and record
  holds/does-not-hold. A mismatch stops the task for re-review; the dependency
  graph is never changed to work around it.

## Later implementation checks (non-blocking after R0)

- Confirm the single-flight entry lifecycle race
  (`Initializing -> Active -> Closing -> Drained/Failed`), the atomic multi-key
  admission section, the same-key `Closing` lookup, and the owner close race
  under concurrent first-use and terminal replacement, including the
  aborted-at-barrier and no-surviving-caller close cases.
- Confirm the chosen finite bounds with loopback peer stream-limit fixtures. The
  numeric values are implementation-only and may be tuned while preserving the
  bounded/no-queue contract.
