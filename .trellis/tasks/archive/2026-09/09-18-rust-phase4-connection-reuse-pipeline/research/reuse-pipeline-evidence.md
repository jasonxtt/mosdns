# Research evidence — connection reuse and pipeline foundation

Date: 2026-09-18. This is source-backed evidence, not an implementation contract.
`prd.md` and `design.md` are normative for this task. Every reference below was
verified against the working tree at `cccadcd`.

## 1. Existing contracts the reuse layer must not weaken

### Lifecycle and registration (the gate that must cover pooled resources)

- `rust/upstream-core/src/lib.rs:563-580` — `LifecycleState::{Open,Closing,Closed}`
  and `LifecycleInner { state, in_flight }`. The doc comment records the crucial
  invariant: "A single `std::sync::Mutex` protects both the `Open -> Closing`
  admission transition and exchange registration, so close can never observe a
  zero registration count while a new exchange is registering… no await happens
  while it is held." A pooled lease therefore **must** be registered here, or
  close can return while a connection is still on loan.
- `lib.rs:594-801` — `Lifecycle::{new, state, begin_close, commit_response,
  commit_final_response, finish_close, ensure_open, register, register_shared,
  register_owned, drain}` plus `SharedInFlightGuard` (`:803`) whose `Drop`
  (`:807`) releases the registration.
- `lib.rs:630-673` — `commit_response` and `commit_final_response`. The latter is
  documented as "the single final control-aware response-commit operation" with a
  fixed priority under one lock: owner state/close first, caller cancellation
  second, the original absolute deadline third, commit last. A reuse design must
  not introduce a second linearization point.
- `lib.rs:815-826` — `CloseTransition` / `CloseCompletion`, the typed
  close-result vocabulary that a pooled `close()` must map into.

### Deadline, cancellation, side effects

- `lib.rs:159-198` — `ExchangeContext` with `new`, `deadline()`, `check_at(...)`.
- `lib.rs:204-248` — `ExchangeControl` with `caller_cancellation()`,
  `owner_cancellation()`, `check_at(...)`; the module notes cancellation is
  checked before the deadline so it wins a tie.
- `lib.rs:250` — `SideEffectState::{NotSent, MaybeSent, Sent}`. The lib doc
  (`lib.rs:486-489` region) fixes the classification: "Connect/setup, invalid
  request/endpoint, and outbound frame-too-large are `NotSent`; runtime errors
  retain the last tracked state." **This is the only sound basis for deciding
  whether a failed attempt may be retried on a fresh connection.**

### Request identity (why ID rewriting is not available)

- `lib.rs:87-115` — `ExchangeRequest<'q>` stores the borrowed query and the
  recorded `request_id`. Its doc: "`Copy` lets one validated request be handed to
  two transport primitive calls unchanged. It only copies the borrowed reference
  and the recorded original ID; the caller's query bytes are never copied,
  rewritten, or revalidated." A pipelining design that needed per-connection ID
  rewriting would contradict this.
- `lib.rs:98-104` — validation via `mosdns_dns_core::parse_query`, mapping failure
  to `UpstreamError::InvalidRequest`.

### Framing (single implementation, to be reused)

- `rust/upstream-core/src/tcp.rs:1-18` — module doc: `exchange` "composes those
  helpers with exactly one fresh `TcpStream` per call. It is deliberately
  independent of policy, fallback, pooling, reuse, pipelining, and retry: there is
  no second framing implementation, **no connection is kept after the call**."
- `tcp.rs:19-20` — `const PREFIX_BYTES: usize = 2`.
- `tcp.rs:145-238` — `encode_frame`, `write_frame`, `flush_bytes`, `read_frame`.
- `tcp.rs:239-287` — `race_control` and `race_io`, the deadline/cancel/close race
  helpers; `tcp.rs:289-317` — `write_all_bytes`, `read_exact_bytes`.
- `tcp.rs:46-144` — the fresh plain-TCP `exchange` that remains the non-pooled
  path and defines the request/response ordering contract.

### Secure identity separation (the highest-risk surface for reuse)

- `rust/upstream-core/src/secure/endpoint.rs:23-40` — `ServerIdentity` doc:
  "Construction only syntax-checks the identity; it never resolves a DNS name.
  The numeric destination a socket connects to is supplied separately so an
  override such as `dial_addr` cannot silently change the authenticated identity."
- `secure/endpoint.rs:47` — `ServerIdentity::new`; `:172-184` `DotEndpoint::new(dial, identity)`;
  `:210-227` `DohEndpoint::new(service_url, dial)`; `:287` `host()`; `:302` `path()`.
- `rust/upstream-core/src/secure/dot.rs:1-31` — the ordering contract: numeric
  connect, then authenticated handshake, then (and only then) write the query. The
  doc states a handshake failure is a typed TLS error with
  `SideEffectState::NotSent`, and "no path exists that sends the query in
  plaintext or retries with verification disabled."
- `secure/dot.rs:48-58` — `SecureTransport` and `SecureHttpVersion`; `:71-80`
  `SecureResponse`; `:178` `DotUpstream`; `:196` `DotPhase`; `:230` `DotPause`.
- `rust/upstream-core/src/secure/doh.rs:1-29` — same ordering for DoH, and the
  explicit statement that no path "retries, **pools the connection**, or falls
  back to another protocol." This task is the reviewed contract that changes that
  last clause in a bounded way.
- `secure/doh.rs:110-345` — `H2Children`, `TrackedH2Executor`, `H2ScopeLease`,
  `H2ChildGuard`, `H2TeardownPause`: existing ownership/drain machinery for
  HTTP/2 child futures, which a pooled H2 connection must build on.
- `secure/doh.rs:946-958` — `restore_request_id`; `:960-977` —
  `negotiated_protocol` / `classify_alpn`, the authority for HTTP/1.1 vs HTTP/2
  classification and therefore for the ALPN component of the reuse key.

### Composite policy and resolver consumer boundary

- `rust/upstream-core/src/composite.rs:1-31` — `UdpTcpPolicy` doc: it "never
  reimplements framing, validation, socket ownership, lifecycle, or
  cancellation", performs at most one reviewed TCP fallback on a validated TC=1
  observation, and has "no retransmission, retry, pooling, reuse, pipelining, Go
  re-entry, or hidden runtime."
- `composite.rs:39-63` — `UdpTcpPolicy::new` derives the TCP endpoint from the same
  `SocketAddr`, and `in_flight_exchanges()` is the sum of the two legs.
- `rust/upstream-core/src/resolver/owner.rs:857-903` — `ResolverComposition`
  (`endpoint`, `dot_endpoint`, `doh_endpoint`) takes one `PublishedTarget`;
  `:905-938` — `ResolvedUpstream` and `dial()`. Reuse consumes `dial()` only.
- `rust/upstream-core/src/resolver/mod.rs:44-47` — the injected `Clock` trait, the
  existing pattern for deterministic time; a pool's idle expiry should use it
  rather than a timer task.
- `resolver/mod.rs` — `ResolutionSnapshot` exposes `generation()` and
  `selected_target(now)`; a refresh changes future dials only.

## 2. Go characterization — evidence, explicitly not contract

The archived upstream foundation already classified the Go reuse/pipeline
machinery.
`.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/design.md:520-521`:

| Contract | Go source | Classification |
|---|---|---|
| TCP connection reuse | `pkg/upstream/transport/reuse.go` | Implementation-only / defer — "Go resource optimization until audited" |
| TCP pipelining and pending demux | `pkg/upstream/transport/pipeline.go` | Implementation-only / defer — "Go implementation, not yet product contract" |

Concrete Go values confirmed in the current tree, recorded only so nobody
mistakes them for requirements:

- `pkg/upstream/upstream.go:52-55` — `pipelineConcurrentLimit = 64`, with the
  comment "Maximum number of concurrent queries in one pipeline connection. See
  RFC 7766 7. Response Reordering." and a `TODO: Make this configurable?`.
- `pkg/upstream/upstream.go:321-323` — plain TCP `idleTimeout` defaults to
  `time.Second * 10` when unset.
- `pkg/upstream/upstream.go:405-407` — DoH `idleConnTimeout` defaults to
  `time.Second * 30`.
- `pkg/upstream/upstream.go:352` — `transport.NewReuseConnTransport(...)` for TCP.
- `pkg/upstream/transport/` contains `reuse.go`, `pipeline.go`,
  `conn_lazy_dial.go`, `conn_traditional.go`, `conn_quic.go`.

**Consequence for this task:** the Rust contract confirms its own explicit
task-local bounds (design.md §6: `MAX_IDLE_PER_KEY = 1`, `MAX_IDLE_TOTAL = 8`,
`IDLE_TIMEOUT = 10s`, `MAX_PENDING_PER_CONNECTION = 1`) and does not claim parity
with the Go values 64 / 10s / 30s. Those Go values remain evidence for a future
performance comparison; they are not product contract, and the Rust constants are
not exposed as configuration.

## 3. Protocol constraints that shape the design

- **RFC 7766 §7 (Response Reordering)** is the reason Go allows 64 concurrent
  queries per connection: TCP DNS responses may arrive out of order, so a
  multiplexed connection needs demultiplexing. Demultiplexing without rewriting
  the caller's ID requires that concurrently outstanding queries on one
  connection carry distinct IDs. The user therefore settled this task to
  **serial per connection** (design.md §7) and moved same-connection
  multi-outstanding requests, reordering, and ID demux to a separate task, rather
  than silently rewriting IDs to make demux easy.
- **The 2-byte length prefix is the only framing** (`tcp.rs:19-20,145-238`), shared
  by plain TCP and both secure transports. Reuse must not add a second framing
  path (e.g. a newline- or block-based codec).
- **TLS connection reuse implies reusing the authenticated session.** Since
  authentication is bound to `ServerIdentity` (`secure/endpoint.rs:23-40`) and DoH
  additionally to the URL authority, those values must be part of the key, or
  reuse would transfer trust across identities.

## 4. Why the ordering in this task is correctness-first

Three observables make a pooled design unsafe if rushed:

1. A connection can be closed by the peer while idle; the first write then fails
   after zero bytes left the buffer. Only `NotSent` may be retried (design.md §5).
2. An owner close can race a guard returning a connection. The guard must discard
   rather than insert once `Closing` has begun (design.md §4).
3. A `Sent`-state failure (write succeeded, read failed) must never be retried,
   because the peer may have received and acted on the query.

Each of these is directly testable with loopback fixtures and a fixture-side
accept counter, without wall-clock sleeps: connection reuse is proven by counting
accepted connections, not by measuring time.
