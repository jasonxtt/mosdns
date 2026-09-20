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

The archived task
__.trellis/tasks/archive/2026-09/09-18-rust-phase4-quic-http3-doq-foundation/`
establishes the protocol foundation and explicitly defers QUIC connection
reuse/pooling, stream-depth/backpressure, and H3 reuse. Its dependency audit
records exact locked versions `quinn 0.11.7`, `h3 0.0.8`, and
`h3-quinn 0.0.10`, all already present in the current manifest. This
task therefore does not need a dependency admission slice unless the
implementation unexpectedly requires a new crate.

The archived
__.trellis/tasks/archive/2026-09/09-18-rust-phase4-connection-reuse-pipeline/`
is useful only for contrast: its owner is serial per connection with one
outstanding query. Its key dimensions and Lifecycle integration are evidence
for what must remain safe, not a template for multiplexing.

## Local locked-crate API evidence

The local Cargo registry was inspected at the locked versions:

- `h3 0.0.8` `client::Builder::build` returns a driver and
  cloneable `SendRequest`; `SendRequest::send_request` takes
  `&mut self`, and `SendRequest` implements `Clone`.
- `h3 0.0.8` `client::Connection::poll_close` must be polled
  continuously; `shutdown` initiates graceful shutdown and
  `wait_idle` waits for closure.
- `h3-quinn 0.0.10` makes `OpenStreams__ cloneable and maps each
  H3 request to a fresh Quinn bidirectional stream.
- `h3 0.0.8` exposes `RequestStream::stop_stream` for the send
  direction and `stop_sending` for the receive direction. The local
  implementation must account for the existing h3-quinn behavior where an
  interrupted read can temporarily move the underlying receive stream into an
  internal `Option::None__.
- `quinn 0.11.7` `Endpoint__ is cloneable and its endpoint driver is
  spawned by the crate's endpoint construction; the owner must keep the endpoint
  handle alive and use `close` / `wait_idle` during teardown.
  `quinn::Connection__ is cloneable, `open_bi` is per-stream, and
  `closed` / `close` provide connection-level health/teardown.
- Quinn's local `RecvStream::Drop` stops unread receive data with code
  zero. This is evidence that H3 cancellation needs an explicit phase-aware
  stream teardown test; it is not permission to close the shared connection.

## Planning decisions derived from the evidence

1. Use a new QUIC-specific key/owner and do not extend the serial TCP pool.
2. Keep one physical connection per key in this task. Use per-stream permits
   for bounded concurrency and use generation identity for dead-entry removal.
3. Make H3 driver lifetime a connection-entry responsibility. A request stream
   may fail without killing a healthy H3 connection; driver/connection failure
   evicts the entry.
4. Keep the existing final commit gate after response validation and before
   releasing the request permit.
5. Use resolver-selected numeric `PublishedTarget::dial()` only in the
   key; identity, authority, and TLS policy remain independent fields.
6. Keep the dependency graph locked and defer all socket policy, retransmission,
   listener, host, config, and production wiring.

## Deferred implementation checks

- Confirm the exact safe h3 request-stream cancellation sequence for each phase
  (before send, after request FIN, during response head, and during body read)
  without invoking the known missing-stream panic path.
- Confirm which Quinn/H3 errors prove the physical connection is dead versus only
  terminating one request stream.
- Confirm the single-flight entry removal race and the owner close race under
  concurrent first-use and dead-generation replacement.
- Confirm the chosen finite bounds with loopback peer stream-limit fixtures. The
  numeric values are implementation-only and may be tuned while preserving the
  bounded/no-queue contract.
