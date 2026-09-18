# Design — Rust Phase 4 QUIC/HTTP3/DoQ foundation

Status: planning only. This document authorizes no implementation until the
final planning summary is explicitly approved and `task.py start` is run.

## 1. Boundary and crate ownership

The deliverable lives entirely in `rust/upstream-core`. It adds a **QUIC
transport layer beside the existing primitives** and does not replace them:

| Layer | Existing artifact | Role in this design |
|---|---|---|
| Wire/model | `lib.rs` (`Endpoint`, `ExchangeRequest`, `ExchangeContext`, `ExchangeControl`, `SideEffectState`, `Lifecycle`, `Upstream`, `ExchangeResponse`) | **Reused verbatim.** QUIC exchanges use the same request/context/lifecycle/commit vocabulary. |
| Identity | `secure/endpoint.rs` (`ServerIdentity`, `DotEndpoint`, `DohEndpoint`, `get_request_target`) | **Reused verbatim.** DoQ endpoint mirrors `DotEndpoint` shape; DoH3 reuses `DohEndpoint` unchanged. |
| TLS policy | `secure/tls.rs` (`TlsPolicy`, `client_config_with_alpn`) | **Reused verbatim.** QUIC builds its TLS config from the frozen policy with exact ALPN. |
| TCP framing | `tcp.rs:145-238` + `dns-core` `frame_response(_, FrameMode::Stream)` | **Reused, not duplicated.** The DoQ 2-byte-prefix encode reuses the same frozen `dns-core` Stream framing helper that `tcp.rs::encode_frame` uses. Only the stream lifecycle around it (FIN signaling, per-stream error-code mapping) is new driver code — never a second prefix codec. No change to `tcp.rs` or `dns-core`. |
| Resolver | `resolver/owner.rs:857-938` (`ResolverComposition`, `PublishedTarget::dial()`) | **Read-only consumer.** One same-shaped composition entry is added; resolver internals untouched. |
| Reuse owner | `reuse.rs` (`ReuseKey`, serial policy) | **Untouched.** This task is one-shot only; QUIC pooling is deferred. |

New code: one `quic` module (endpoint construction, DoQ exchange, DoH3
driver, typed errors, ALPN constants) plus exact manifest/lock additions for
the audited QUIC dependencies. No Go/cgo/FFI, no C ABI symbols, no
`MOSDNS_*_BACKEND` selector, no second runtime, no YAML/config/API/WebUI, no
host wiring.

## 2. Dependency audit gate (hard gate before any QUIC code)

Slice 0 admits a QUIC stack only if **all** of the following hold, recorded
with exact versions in the lock:

1. License compatible with `GPL-3.0-only` (e.g. MIT/Apache-2.0/ISC tri-license
   families as with the existing `rustls`/`hyper` graph).
2. Declared and resolved MSRV `<= 1.85`; `cargo metadata` audit shows no
   resolved package above 1.85 (same method as the secure task).
3. Client-only feature selection: no server/listener features, no extra async
   runtimes, no platform trust-store auto-loading (roots stay caller-supplied
   via `TlsPolicy`).
4. Expected shape: a QUIC connection crate (quinn family) for DoQ streams,
   plus an HTTP/3 client layer for DoH3. If one crate cannot cover both, two
   minimal crates are acceptable; each is audited independently.
5. TLS-stack alignment: the QUIC stack's rustls version must equal the
   workspace `rustls 0.23` that `TlsPolicy::client_config_with_alpn` builds
   from. A mismatch (e.g. QUIC pins rustls 0.22/0.24) fails the audit — the
   frozen-policy config cannot be handed to a different rustls major without
   re-reviewing the verification semantics.
6. `cargo tree -e normal` for `mosdns-upstream-core` shows the new crates and
   nothing else new; `#![forbid(unsafe_code)]` still applies to this crate's
   own code.

If the audit fails, the task **stops at Slice 0**: no QUIC code is written,
and the failure is reported to the reviewer as scoped evidence rather than
worked around by weakening any criterion above.

## 3. Endpoint construction

```rust
DoqEndpoint { dial: SocketAddr, identity: ServerIdentity }
// construction mirrors DotEndpoint::new: reject zero port, validate identity,
// never resolve, never open a socket.
```

- `DoqEndpoint::new(dial, identity) -> Result<_, SecureError>`: same error
  vocabulary as `DotEndpoint::new` (`ZeroDialPort`, `InvalidIdentity`).
- DoH3 needs **no new endpoint type**: it reuses `DohEndpoint` unchanged
  (service URL + numeric dial + identity accessors). The transport selects H3
  instead of H1/H2.
- ALPN constants: `DOQ_ALPN = b"doq"`, `H3_ALPN = b"h3"`. Offered lists are
  exact singletons per transport: DoQ offers only `doq`; DoH3 offers only
  `h3`. No `h2,http/1.1` mixing on QUIC paths and no `doq` on TCP/TLS paths.

## 4. DoQ exchange data flow

```
caller
  └─ DoqUpstream::exchange(ExchangeRequest, ExchangeContext)
       ├─ validate request/endpoint                      (existing pure checks)
       ├─ lifecycle.ensure_open()                        (existing gate)
       ├─ QUIC connect to numeric dial                   (caller runtime, no hidden runtime)
       ├─ TLS handshake with TlsPolicy-derived config    (ALPN exactly ["doq"], verifies identity)
       ├─ open one bidirectional stream                  (one query ↔ one stream)
       ├─ write length-prefixed query, wire ID zeroed    (owned outbound copy; caller bytes untouched)
       ├─ STREAM FIN (no more request bytes)             (RFC 9250 §4.2)
       ├─ read length-prefixed response on same stream
       ├─ validate peer wire FIRST: dns-core header check + wire ID MUST be 0
       │   (RFC 9250 §4.2.1; a nonzero peer ID is a terminal typed error, never committed)
       ├─ restore original ID into owned response copy   (caller-visible wire keeps its ID)
       ├─ dns-core full response validation on the restored copy
       ├─ commit_final_response(...)                     (existing linearization)
       └─ close connection (one-shot; no pooling)
```

Framing notes:

- The 2-byte big-endian length prefix reuses the frozen `dns-core`
  `frame_response(_, FrameMode::Stream)` helper — the same one
  `tcp.rs::encode_frame` uses — so DoQ and TCP cannot disagree on the prefix
  shape or its edge handling. Only the surrounding stream lifecycle (FIN
  signaling, per-stream error codes) is new driver code.
- The outbound copy (not the caller's borrowed bytes) has its ID bytes
  zeroed before send; the committed `ExchangeResponse` wire has the original
  ID restored. `request_id == response_id == caller ID` holds at the public
  boundary exactly as for TCP/DoT/DoH.

## 5. DoH3 driver data flow

```
caller
  └─ Doh3Upstream::exchange(&DohEndpoint, ExchangeRequest, ExchangeContext)
       ├─ validate request/endpoint                      (DohEndpoint + ExchangeRequest checks)
       ├─ target = endpoint.get_request_target(request)  (existing encoder, reused verbatim)
       ├─ QUIC connect to endpoint.dial()                (caller runtime)
       ├─ TLS handshake, ALPN exactly ["h3"], verify endpoint.identity()
       ├─ one H3 GET with :authority = endpoint.authority(), path = target
       ├─ read application/dns-message body
       ├─ restore original ID, dns-core validation, commit_final_response
       └─ close connection (one-shot)
```

- Request-target encoding is **not** reimplemented: `get_request_target`
  stays the single encoder. Any future encoder fix applies to H1/H2/H3 alike.
- No H3 generic client surface is exposed: the driver speaks exactly one GET
  shape. Server/listener H3 code is forbidden in this task.

## 6. Lifecycle, deadline, cancellation, close

Identical to the secure foundation, restated so the executor has no ambiguity:

- **One absolute deadline, never reset.** QUIC connect, handshake, stream
  open/write/read all race `context.deadline()` through the existing
  `race_control` helper verbatim. The new QUIC error type implements
  `From<UpstreamError>` (same pattern as the existing
  `From<UpstreamError> for SecureError` at `secure/error.rs:387`, which lets
  the secure path receive typed control failures through the generic bound),
  so QUIC I/O futures satisfy `race_control`'s `E: From<UpstreamError>` bound
  with no ad-hoc error reshape. No private timeout such as Go's 6s stream
  timeout is introduced.
- **Precedence unchanged**: owner close → caller cancellation → deadline →
  commit, via `ExchangeControl::check_at` and `commit_final_response`.
- **Close drains.** `close()` transitions through the existing `Lifecycle`,
  refuses new exchanges after `Closing`, waits for in-flight exchanges, drops
  the (nonexistent) idle set trivially, and converges on repeat.
- **No late success**: a response validated after close/cancel/deadline is
  discarded; the terminal control error wins, same as the H2 final-commit rule.

## 7. Failure classification

| Observation | `SideEffectState` | Action |
|---|---|---|
| QUIC connect/setup error | `NotSent` | Typed error; no retry loop |
| TLS handshake failure (incl. ALPN mismatch) | `NotSent` | Typed TLS error (`TlsHandshakeFailure` family) |
| Stream open failure before write | `NotSent` | Typed error; no retry loop |
| Write completed, read failed/EOF/partial | `Sent` | Terminal typed error; no retry |
| Stream error code received | mapped | `NO_ERROR` after completion is benign; `REQUEST_CANCELLED` on caller-cancel paths; `INTERNAL_ERROR`/peer errors are terminal typed errors |
| Response ID/QR mismatch or malformed wire | `Sent` | Terminal typed error |
| Deadline/cancel/close observed | per phase | Typed control error with current state; no retry |
| Valid response committed | — | `commit_final_response`; connection closed |

Cross-protocol fallback is **forbidden**: DoQ failure never falls back to
DoT/TCP, DoH3 failure never falls back to DoH/H2. This matches Go's
`EnableHTTP3` "no fallback" note and is an explicit contract here, with
negation tests.

## 8. Service identity and protocol contract preservation

- DoQ authenticates `identity` (SNI + certificate verification via the
  `TlsPolicy`-derived config); the socket connects to `dial`. Same split as
  DoT, enforced by construction plus a direct test.
- DoH3 keeps `DohEndpoint` authority/path semantics; `:authority` equals
  `endpoint.authority()` and the GET target equals `get_request_target`
  output byte-for-byte (test asserts this against the same input run through
  the DoH path's encoder).
- `insecure_skip_verify` remains explicit opt-in only; no handshake failure
  path selects it.
- 0-RTT and session resumption stay **off**; the QUIC config constructor must
  disable them explicitly rather than relying on crate defaults.

## 9. Compatibility

| Contract | Source | Disposition | Evidence |
|---|---|---|---|
| Original DNS ID at public boundary | `lib.rs`; reuse PRD | **Preserve** | ID equality tests on both DoQ and DoH3 paths |
| Numeric dial separate from service identity | `secure/endpoint.rs` | **Preserve** | Cross-identity construction/dial tests |
| One absolute deadline; cancel/close precedence | `lib.rs`; secure tasks | **Preserve** | Deadline/cancel/close tests |
| `Open -> Closing -> Closed`, close drains | `lib.rs` Lifecycle | **Preserve** | Close/drain/idempotence tests |
| DoQ wire ID zero + STREAM FIN | RFC 9250 §4.2/§4.2.1 | **New protocol contract** | Loopback asserts zeroed wire ID + FIN |
| DoQ ALPN `doq`, H3 ALPN `h3` | RFC 9250 §3; RFC 9114 | **New protocol contract** | Handshake ALPN assertions |
| GET target encoding | `DohEndpoint::get_request_target` | **Preserve (reuse)** | Byte-equality with encoder output |
| No cross-protocol fallback | Go note + PRD R8 | **New explicit contract** | Negation tests |
| Fresh connection per exchange | secure foundation | **Preserve for this task** | Fixture counts accepts/connections |
| QUIC pooling/multiplex tuning | deferred | **Defer** | §10 |
| Go timeouts/windows/keepalive values | `upstream.go`, `conn_quic.go` | **Implementation-only, not contract** | Not asserted |

## 10. Explicitly deferred to separate tasks

1. QUIC connection reuse/pooling and H3 multiplexing (depth, backpressure,
   reuse key with `doq`/`h3` ALPN discrimination).
2. 0-RTT early data, session resumption, connection migration — each needs its
   own security/correctness review.
3. mTLS / custom cipher suites / custom TLS versions.
4. Socket policy (SOCKS, local bind, marks), UDP retransmission changes.
5. Server listeners (DoQ/DoH3 inbound) and any inbound handling.
6. Cross-address retry / Happy Eyeballs (user-excluded).
7. Metrics/audit/logging wiring, benchmarks, soak/long-run evidence.
8. Making QUIC tuning parameters configurable (YAML/config/API).

## 11. Rollback

Rollback is a single Rust commit revert of the new `quic` module, its
re-exports, its tests, and the exact manifest/lock additions. `lib.rs`,
`tcp.rs`, `secure/*`, `resolver/*`, `composite.rs`, `reuse.rs`, and
`Upstream::exchange` keep their current behavior, so all pre-existing paths
remain operational with no Go/config/production involvement. If Slice 0 audit
fails, there is nothing to roll back (no code was written).

## 12. Risks

- **Dependency admission** is the top risk: QUIC stacks are large with deep
  transitive graphs. Mitigated by the Slice 0 hard gate (§2) and by stopping
  the task there on failure.
- **ALPN confusion** (wrong protocol on a connection) is mitigated by exact
  singleton ALPN offers plus handshake assertions in tests.
- **Missing ID restore** would break downstream correlation; mitigated by
  asserting public-boundary ID equality on every success test.
- **Hidden runtimes/timers** inside QUIC crates: the executor must verify the
  chosen crates run on the caller's runtime (as `tokio`/`hyper` do today) and
  introduce no background tasks; tests assert no detached work via the
  existing lifecycle drain semantics.
- **Scope creep** toward pooling/migration/0-RTT/listeners; the PRD Out of
  scope list and §10 are the guard.
