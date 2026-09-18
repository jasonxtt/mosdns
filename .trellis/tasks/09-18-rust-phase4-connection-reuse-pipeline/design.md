# Design — Rust Phase 4 upstream connection reuse and pipeline foundation

Status: planning only. This document authorizes no implementation until the
final planning summary is explicitly approved and `task.py start` is run.

## 1. Boundary and crate ownership

The deliverable lives entirely in `rust/upstream-core`. It adds a **reuse owner
layer above the existing primitives** and does not replace them:

| Layer | Existing artifact | Role in this design |
|---|---|---|
| Wire/framing | `tcp.rs:145-238` (`encode_frame`, `write_frame`, `flush_bytes`, `read_frame`) | **Reused verbatim.** This remains the single 2-byte-prefix framing implementation. |
| Race/control | `tcp.rs:239-287` (`race_control`, `race_io`) | **Reused verbatim** so deadline/cancel/close race once, not twice. |
| Lifecycle | `lib.rs:589-801` (`Lifecycle`, `register*`, `drain`, `commit_final_response`) | **Reused verbatim.** The pool adds no second gate. |
| Context | `lib.rs:159-248` (`ExchangeContext`, `ExchangeControl`, `SideEffectState`) | **Reused verbatim.** |
| Plain TCP | `tcp.rs:46-144` (`exchange`) | Kept as the fresh-connection path. |
| DoT | `secure/dot.rs:178-303` (`DotUpstream`) | Facade preserved; its prepare/validate helpers are reused. |
| DoH | `secure/doh.rs:368-522` (`DohUpstream`, `H2*`) | Facade preserved; H2 child tracking is reused for owned streams. |
| Identity | `secure/endpoint.rs:23-40,184,227` (`ServerIdentity`, `DotEndpoint`, `DohEndpoint`) | **Never bypassed.** Identity is part of the reuse key. |
| Resolver | `resolver/owner.rs:857-938` (`ResolverComposition`, `ResolvedUpstream`) | **Read-only consumer.** Not modified by this task. |

No Go/cgo/FFI, no C ABI symbols, no `MOSDNS_*_BACKEND` selector, no second
runtime, no YAML/config/API/WebUI, and no new crate dependency are introduced.

## 2. Reuse key

```
ReuseKey {
    dial:        SocketAddr,        // the numeric destination actually dialed
    transport:   Transport,         // Tcp (plain) — UDP is connectionless and not pooled
    secure:      Option<SecureKey>, // None for plain TCP
}

SecureKey {
    kind:        SecureKind,        // Dot | Doh
    identity:    ServerIdentity,    // DoT SNI identity; for DoH the endpoint's authority identity
    authority:   Option<String>,    // DoH URL authority (host[:port]) — must equal identity for DoH
    tls_policy:  TlsPolicyDiscriminant, // e.g. verification-on + roots revision, not the roots themselves
    alpn:        NegotiatedProtocol,    // Http11 | H2  (unknown until handshake)
}
```

Rules, all testable:

1. `dial` is the **numeric** address. A hostname never appears in the key, so a
   resolver refresh changes only future dials; established connections stay valid
   for the target they were opened to (R8).
2. `transport` separates plain TCP from TLS. A plain-TCP connection must never
   satisfy a DoT/DoH request.
3. `secure.identity` is mandatory for a secure transport. A connection
   authenticated as identity X is **never** eligible for identity Y (A2).
4. `secure.alpn` is known only after the handshake, so a secure entry is
   classified into its final key at handshake completion. A DoH entry negotiated
   as HTTP/2 is never reused for an HTTP/1.1 request and vice versa.
5. `tls_policy` discriminates on configuration identity (verification mode and a
   roots revision identifier), not on the root material, so rotating roots
   invalidates entries without leaking trust material into the key.
6. The resolver's `ResolutionSnapshot` is **not** part of the key. It is consumed
   one layer up via `PublishedTarget::dial()`; the owner only sees the resulting
   `SocketAddr`.

## 3. Data flow and ownership

```
caller
  └─ ReuseOwner::exchange(ExchangeRequest, ExchangeContext)
       ├─ validate request/endpoint                      (existing pure checks)
       ├─ lifecycle.ensure_open()                        (existing gate)
       ├─ key = ReuseKey::from(endpoint)                 (pure)
       ├─ checkout(key)  ── hit ──▶ borrowed connection guard
       │                    └─ miss ─▶ dial + (secure) handshake ─▶ insert
       ├─ write one framed request                       (tcp.rs helpers)
       ├─ read one framed response                       (tcp.rs helpers)
       ├─ validate_response + original-ID check          (dns-core)
       ├─ commit_final_response(...)                     (existing linearization)
       └─ guard.release()                                (return-to-idle or discard)
```

Ownership rules:

- Every connection is owned by the `ReuseOwner`. A caller never holds a
  `TcpStream`/`TlsStream` directly; it holds a **guard** whose drop returns or
  discards the connection.
- A connection is in exactly one of: `Idle` (in the map), `Leased` (one guard),
  `Closing`, `Closed`. The state is guarded by one short synchronous mutex, and no
  `await` occurs while it is held (`Lifecycle` already establishes this pattern at
  `lib.rs:569-580`).
- The `Lifecycle` in-flight count registers **leased** connections *and* the
  exchange itself, so `drain()` cannot observe zero while a connection is on
  loan. Idle entries are not in-flight work; they are released by close.
- No detached task, no background reaper runtime. Idle expiry is evaluated
  lazily at checkout/insert time from the injected `Clock` (the resolver's
  `Clock` trait, `resolver/mod.rs:44-47`, is the existing injected-time pattern),
  which keeps tests deterministic and avoids a timer task.

## 4. Lifecycle, deadline, cancellation, close

- **One absolute deadline, never reset.** Checkout must not create a timer. All
  I/O still races `context.deadline()` through `race_control`/`race_io`, so a
  reuse hit cannot extend a caller's budget (R4, A4). Reuse removes connect cost;
  it does not grant time.
- **Cancellation and close precedence is unchanged** (`lib.rs:185`,
  `commit_final_response`): owner close first, then caller cancellation, then the
  deadline, then commit.
- **Close drains and discards.** `close()` (a) transitions the owner to
  `Closing` through the existing `Lifecycle`, (b) prevents any further checkout,
  (c) waits for every leased guard to return, and (d) drops every idle connection.
  A guard returning after `Closing` must **discard** rather than insert, so a
  closed pool can never be repopulated (A4). Repeated close converges.
- **No post-terminal publication.** A connection whose exchange already committed
  may still be returned to idle, but a connection whose owner began closing is
  discarded.

## 5. Failure classification and rebuild

The only safe retry trigger is "the query was never written on this connection".

| Observation | `SideEffectState` | Action |
|---|---|---|
| Idle connection found half-closed (read EOF / RST on first write) | `NotSent` | Discard, open **one** fresh connection, retry once |
| Dial/connect/setup error | `NotSent` | Typed error to caller; no retry loop |
| TLS handshake failure | `NotSent` | Typed TLS error (existing `TlsHandshakeFailure`) |
| Write completed, read failed/EOF/partial | `Sent` | Terminal typed error; **discard** connection; no retry |
| Response ID/QR mismatch or malformed wire | `Sent` | Terminal typed error; discard connection |
| Deadline/cancel/close observed | per phase | Typed control error with current state; no retry |

"Retry once on `NotSent`" is **not** protocol fallback: it stays on the same
transport and same identity, uses a fresh connection, and is bounded to a single
attempt so a hostile peer cannot induce an unbounded dial loop. Writes to a
reused connection are the one place where a local send can fail after the bytes
left the buffer; that case keeps `MaybeSent`/`Sent` semantics and is not retried.

## 6. Bounds and backpressure

All bounds are **implementation-layer task-local bounded constants**, and their
values are **confirmed for this task** (user-confirmed 2026-09-18). They are
deliberately *not* configurable: no YAML key, environment variable, or API surface
exposes them, so they are **not a product configuration contract**. This task also
makes **no parity claim** against the Go values (64 / 10s / 30s), which the
archived matrix classifies as implementation-only.

| Bound | Meaning | Confirmed value | Rationale | On exceed |
|---|---|---|---|---|
| `MAX_IDLE_PER_KEY` | retained idle connections for one key | `1` | A single spare connection absorbs the next sequential query, which is the whole point of this foundation; more than one per key buys nothing while work is serial per connection. | discard the existing idle entry, keep the newer one |
| `MAX_IDLE_TOTAL` | retained idle connections across all keys | `8` | Bounds file descriptors on a host with many configured upstreams without needing a global budget policy. | discard the least-recently-returned entry |
| `IDLE_TIMEOUT` | maximum age of an idle entry | `10s` | Matches the common peer idle-close interval so a stale entry is evicted before its first use would fail; evaluated lazily from the injected `Clock`, so no timer task exists. | discard on checkout/insert |
| `MAX_PENDING_PER_CONNECTION` | concurrent outstanding queries on one connection | `1` | See §7: serial per connection is the settled decision, not a default. | typed `PoolBusy`-class error |

Backpressure semantics are **fixed as no wait queue**: a caller that cannot get a
slot receives a typed error or opens its own fresh connection. Queue-based
admission is deferred (§10) because a queue converts backpressure into latency and
needs its own fairness/latency contract — a separate reviewed decision.

## 7. Key decision — serial per connection (settled)

**Decision (user-confirmed 2026-09-18):** a reused connection serves **exactly one
outstanding query at a time** (`MAX_PENDING_PER_CONNECTION = 1`). Concurrency
across callers comes from having several connections, not from multiplexing one
connection.

Rationale:

- RFC 7766 §7 permits out-of-order responses, so N>1 outstanding queries on one
  connection requires demultiplexing replies back to the right caller. Doing that
  by the caller's original ID requires the IDs to be pairwise distinct; doing it
  otherwise would require rewriting IDs.
- Rewriting IDs is **not available**: `lib.rs:87-115` records the original
  `request_id` and states the caller's bytes are "never copied, rewritten, or
  revalidated". Serial per connection preserves that contract exactly and is
  therefore provably unambiguous.
- Correctness precedes throughput here. The performance-oriented demux is a
  larger design surface (reordering buffers, per-connection pending maps, failure
  and timeout semantics for individual outstanding queries) and deserves its own
  task and review.

Consequences for this task:

- No pending map, no reordering buffer, and no ID bookkeeping exist on the reuse
  path. A second concurrent request for a busy connection is rejected typed or
  served by a separate connection; it is never queued behind the first and never
  shares the wire.
- The `ExchangeRequest` contract is untouched: the bytes written are the caller's
  bytes, with the caller's ID.

**Deferred to a separate task:** same-connection multi-outstanding requests,
response reordering, and original-ID demux. That task must first establish an
unambiguous correlation policy under the no-ID-rewrite constraint before any
concurrency depth is chosen. See §10.

## 8. Service identity and protocol contract preservation

- DoT keeps `DotEndpoint { dial, identity }` (`secure/endpoint.rs:184`). The
  handshake still authenticates `identity`; the socket still connects to `dial`.
  Reuse adds no path that connects by name.
- DoH keeps `DohEndpoint { service_url, dial }` (`secure/endpoint.rs:227`) and the
  existing authority/host/path accessors (`:287`,`:302`). The `GET` target and
  `:authority` are derived exactly as today; reuse does not change request
  encoding, headers, or the `application/dns-message` requirement.
- ALPN is observed per connection and recorded in the key, so an HTTP/2
  connection is only used for HTTP/2 and an HTTP/1.1 connection only for HTTP/1.1
  (`secure/doh.rs:960-977` `negotiated_protocol`/`classify_alpn` remain the
  authority for that decision).
- `H2Children`/`H2ChildGuard`/`H2TeardownPause` (`secure/doh.rs:110-345`) already
  own and drain HTTP/2 child futures. A pooled H2 connection must be built on that
  same tracking so no child future outlives its connection.

## 9. Compatibility

| Contract | Source | Disposition | Evidence |
|---|---|---|---|
| Original DNS ID never rewritten | `lib.rs:87-115`; Go swap in `pkg/upstream/transport/*` | **Preserve** | ID equality + serial-per-connection isolation test |
| Numeric dial separate from service identity | `secure/endpoint.rs:23-40` | **Preserve** | Cross-identity reuse refusal test (A2) |
| One absolute deadline; no reset on reuse | `lib.rs:185`; `tcp.rs:239-287` | **Preserve** | Reuse-hit deadline test |
| `Open -> Closing -> Closed`, close drains | `lib.rs:589-801` | **Preserve** | Pool close/drain/idempotence tests |
| Framing = one 2-byte-prefix implementation | `tcp.rs:145-238` | **Preserve** | Reuse path exercises the same helpers |
| Fresh connection per exchange | `tcp.rs:1-18`; `doh.rs:1-29` | **Intentional change, bounded** | Reuse is opt-in via the new owner; the existing `Upstream::exchange` fresh path stays available and its tests stay green |
| Serial per connection (one outstanding query) | settled decision §7 | **New contract for this owner** | Second concurrent request is typed-rejected or served by another connection |
| Go idle 10s/30s, pipeline limit 64 | `pkg/upstream/upstream.go:52-55,321-323,405-407` | **Implementation-only, not contract** | Per archive matrix `08-17-.../design.md:520-521` |
| Pool/pipeline in general | same archive matrix | **Defer, then implement** | This task is the reviewed contract that entry asked for |
| Same-connection multi-outstanding / demux | RFC 7766 §7; settled decision §7 | **Defer to a separate task** | Requires an unambiguous no-ID-rewrite correlation policy first |
| UDP retransmission, QUIC, socket policy | PRD Out of scope | **Defer** | — |

## 10. Explicitly deferred to separate tasks

1. **Same-connection multi-outstanding requests, response reordering, and
   original-ID demux** (§7) — settled to a separate task; it must first establish
   an unambiguous correlation policy under the no-ID-rewrite constraint.
2. **Queue-based admission backpressure** — needs a latency/fairness contract.
3. **QUIC/HTTP3/DoQ consumer contract** — the `PublishedTarget` shape is already
   numeric and protocol-agnostic, so a future QUIC task consumes the same result.
4. **Connection-failure cross-family/cross-address retry and Happy Eyeballs** —
   user-excluded; belongs to a dedicated address-racing task if ever wanted.
5. **UDP retransmission policy**, **SOCKS/local bind/socket marks**, **server
   listeners**, **metrics/audit wiring**, **host/config/API/WebUI and production
   selection**, **Go/cgo/FFI and hybrid retirement**.
6. **Pool performance benchmarks and soak/long-run evidence** — Phase 4 asks for
   foundation correctness; performance claims come later.
7. **Making the task-local bound constants configurable** (YAML/config/API) — the
   constants stay fixed and non-configurable in this task.

## 11. Rollback

Rollback is a single Rust commit revert of new modules/re-exports and their tests.
`tcp.rs`, `secure/*`, `resolver/*`, `composite.rs`, and `Upstream::exchange` keep
their current behavior, so the pre-existing fresh-connection path remains the
operational behavior with no Go/config/production involvement.

## 12. Risks

- **Cross-identity reuse** would silently transfer trust; mitigated by keying on
  identity plus a direct refusal test.
- **Unsafe retry after send** would double-send a query; mitigated by gating retry
  on `NotSent` only.
- **Leaked/unnormalized idle entries** on close or error; mitigated by the guard
  invariant "return or discard, never insert after `Closing`" and by draining via
  the existing `Lifecycle`.
- **Serial-per-connection throughput** is an accepted, user-confirmed
  correctness-first position (§7), not an unnoticed gap. It may measure as a
  regression against Go's 64-way pipeline; the deferred task (§10.1) is where
  throughput is pursued, and only with an unambiguous ID correlation policy.
- **Scope creep** toward QUIC/fallback/listeners; the PRD Out of scope list and
  this section are the guard.
- **Bound constants being mistaken for contract**: the confirmed values in §6 are
  implementation constants for this task. Recording them as task-local constants
  (not config) keeps them out of the product configuration contract surface, so a
  later value change is not a compatibility event.
