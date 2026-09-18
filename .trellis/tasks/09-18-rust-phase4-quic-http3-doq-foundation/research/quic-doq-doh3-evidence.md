# QUIC/DoQ/DoH3 evidence (source-backed, planning only)

All line numbers below were read from the working tree on 2026-09-18 (branch
`rust`). They are discovery pointers, not normative contracts; the reviewed
`prd.md`/`design.md` are authoritative for the task.

## 1. Existing Rust contracts this task builds on

### 1.1 Crate boundary and safety

- `rust/Cargo.toml` workspace: `resolver = "3"`, `edition = "2024"`,
  `license = "GPL-3.0-only"`, `rust-version = "1.85"`.
- `rust/upstream-core/Cargo.toml`: crate `mosdns-upstream-core`, `rlib` only;
  normal deps `mosdns-dns-core`, `getrandom 0.4.3` (no default features,
  `std` only), `tokio 1` (`macros,net,rt,sync,time`), `tokio-util 0.7.16`
  (`rt`), `url 2`, `base64 =0.22.1` (no default features, `alloc` only),
  `rustls 0.23` (no defaults; `ring,std,tls12`), `hyper =1.11.0`
  (`client,http1,http2`, no defaults), `hyper-util =0.1.20` (`tokio`, no
  defaults), `http-body-util 0.1.3`, `tokio-rustls =0.26.4`
  (`ring,tls12`, no defaults). Dev-deps: `rcgen =0.14.7` (no defaults,
  `ring` only), `h2 =0.4.19`.
- `rust/upstream-core/src/lib.rs:1` — `#![forbid(unsafe_code)]` for this
  crate's own code.

### 1.2 Exchange model (lib.rs)

- `Transport::{Udp,Tcp}` + `Endpoint` (numeric `SocketAddr`, zero port
  rejected at construction).
- `ExchangeRequest::new` validates via `mosdns-dns-core::parse_query` and
  records the original ID; docs: borrowed bytes "never copied, rewritten, or
  revalidated".
- `ExchangeContext` (absolute deadline + `TransportCancellation`);
  `ExchangeControl` (caller vs owner cancellation tokens, separately wakeable).
- `check_at` tie-break: owner close > caller cancellation > deadline.
- `SideEffectState::{NotSent,MaybeSent,Sent}` closed enum; connect/setup
  failures are `NotSent`.
- `Lifecycle` (`Open/Closing/Closed`, `register*`, `drain`,
  `commit_final_response`): the single commit linearization point and drain
  gate; `close().await` waits for in-flight guards; repeated close converges.
- `ExchangeResponse` owns the complete returned wire with
  `request_id`/`response_id`/`transport`/`truncated`.
- `Upstream::prepare_exchange` / `exchange` / `close`; each exchange carries
  its own owner lifecycle.
- `UdpTcpPolicy` (`composite.rs`): one TCP attempt after a valid UDP TC
  observation, same borrowed query/ID/deadline; UDP errors never trigger
  fallback.

### 1.3 Secure foundation (secure/)

- `secure/endpoint.rs`: `ServerIdentity` (DNS name or IP literal, validated,
  never resolved); `DotEndpoint::new(dial, identity)`;
  `DohEndpoint::new(service_url, dial)` with scheme/userinfo/fragment checks,
  empty path normalized to `/`, `host()`/`authority()`/`path()`/`query()`
  accessors, `get_request_target()` (origin-form, removes existing `dns`
  pairs structurally, appends one generated `dns` pair; out-of-band copy has
  ID bytes zeroed; 65535 query cap, 96 KiB target cap).
- `secure/tls.rs`: `TlsPolicy::verified(roots)` (empty store rejected) /
  `insecure_skip_verify()` (explicit opt-in, never auto-selected);
  `client_config()` uses `ring` provider + safe default versions; ALPN
  cleared for DoT; **`enable_early_data = false`,
  `resumption = disabled`** (`tls.rs:224-227`); `client_config_with_alpn()`
  for per-transport ALPN; `roots_revision()` opaque ordinal for reuse keys.
- `secure/dot.rs`: `DotUpstream` — one fresh numeric connection per exchange,
  TLS handshake before any DNS byte, one framed query/response; ALPN none
  (RFC 7858). Pooled session type exists crate-internally for reuse.
- `secure/doh.rs`: `DohUpstream` — one fresh numeric connection + one HTTPS
  GET over HTTP/1.1 or HTTP/2 (ALPN `h2,http/1.1` order, no fallback/replay);
  `H2Children`/`TrackedH2Executor`/`H2ScopeLease`/`H2ChildGuard` child-task
  ownership; `restore_request_id`; `negotiated_protocol`/`classify_alpn` are
  the ALPN authority. Pooled session type exists crate-internally.
- `secure/mod.rs` docs: pooling/reuse, resolver/bootstrap, **HTTP/3**, and
  listener/host composition are outside the secure slice. Pooling and
  resolver have since been completed by later tasks; **HTTP/3 remains open**.

### 1.4 Resolver consumer boundary (resolver/)

- `resolver/owner.rs:857-903` `ResolverComposition::{endpoint,dot_endpoint,
  doh_endpoint}` each take one `PublishedTarget`; `ResolvedUpstream`
  (`:905-938`) derives numeric `SocketAddr` via `published.dial()`.
- `ResolutionSnapshot`: per-family candidate/diagnostics/`generation()`;
  `selected_target(now)` returns only fresh candidates.
- DoT SNI and DoH URL authority/path stay the configured identity for both
  A and AAAA selections (dual-stack task Slice 3).
- This task adds a same-shaped read-only consumer (e.g. `doq_endpoint`) and
  does not modify resolver internals.

### 1.5 Reuse owner (reuse.rs, ~3980 lines)

- `ReuseKey { dial, transport, secure: Option<SecureKey> }`;
  `SecureKey { kind, identity, authority, tls_policy discriminant,
  negotiated ALPN }`. Hostnames never in keys; resolver snapshots not inputs.
- Serial per connection (`MAX_PENDING_PER_CONNECTION = 1`); task-local
  non-configurable bounds `MAX_IDLE_PER_KEY = 1`, `MAX_IDLE_TOTAL = 8`,
  `IDLE_TIMEOUT = 10s`; no wait queue; retry-once only at `NotSent`.
- QUIC is explicitly out of scope there; the QUIC multiplexing question is
  deferred here (see §4 below for why DoQ differs structurally).

## 2. Go characterization (pkg/, behavior discovery only)

- `pkg/upstream/transport/conn_quic.go`:
  - `QuicDnsConn` wraps `*quic.Conn`; `ReserveNewQuery` opens one stream per
    query (`OpenStream`; error treated as peer stream limit → not closed).
  - `ExchangeReserved`: `copyMsgWithLenHdr` (2-byte big-endian prefix, same
    shape as TCP), **wire ID zeroed** (`binary.BigEndian.PutUint16(..., 0)`),
    `stream.SetDeadline(now + 6s)` (`quicQueryTimeout`), write, `pool`
    release, `stream.Close()` for STREAM FIN; response read via
    `dnsutils.ReadRawMsgFromTCP`; **original QID restored** on success;
    `CancelRead(_DOQ_NO_ERROR)` after success; ctx-done → cancel both
    directions with `_DOQ_REQUEST_CANCELLED` and return `context.Cause(ctx)`.
  - `WithdrawReserved`: cancel both directions with `_DOQ_REQUEST_CANCELLED`.
  - Error codes: `_DOQ_NO_ERROR = 0x0`, `_DOQ_INTERNAL_ERROR = 0x1`,
    `_DOQ_REQUEST_CANCELLED = 0x3`.
- `pkg/upstream/upstream.go`:
  - `IdleTimeout` default comment: "TCP, DoT: 10s, DoH, DoH3, Quic: 30s".
  - `EnableHTTP3` → `http3.Transport` + `DialEarly` (DoH3); comment: "There
    is no fallback. Make sure the server supports it."
  - Protocols `quic`/`doq`: `tlsConfig.NextProtos = ["doq"]`, QUIC dial via
    `quic.Transport`, `newDefaultClientQuicConfig()`.
  - `newDefaultClientQuicConfig()`: `TokenStore LRU(4,8)`, stream windows
    4 KiB, conn windows 8/64 KiB, `MaxIdleTimeout 30s`,
    `KeepAlivePeriod 25s`, `HandshakeIdleTimeout = tlsHandshakeTimeout`.
  - `EventObserver`: "Not implemented for quic based protocol (DoH3, DoQ)."
- `pkg/utils/quic.go`: stateless-reset key from first non-zero MAC +
  SHA-256 salt (server-side concern; listed for completeness, not for client
  reuse).
- `go.mod`: `github.com/quic-go/quic-go v0.59.0`, indirect
  `github.com/quic-go/qpack v0.6.0`.
- Disposition: all of the above are **implementation-only discovery**. The
  6s/30s/25s/window values, the Go stream-timeout mechanics, and the internal
  pool/buffer layouts are not product contracts and must not be asserted as
  Rust parity.

## 3. Protocol facts (RFC 9250 DoQ, RFC 9114 H3)

- RFC 9250 §3: DoQ ALPN token is `doq`.
- RFC 9250 §4.2: client sends the query on the selected bidirectional stream
  and MUST signal STREAM FIN for end-of-request; each query uses its own
  stream, so correlation is by stream, not by ID.
- RFC 9250 §4.2.1: DNS Message ID MUST be 0 on the wire.
- RFC 9250 §4.3: error codes incl. `DOQ_NO_ERROR (0x0)`,
  `DOQ_INTERNAL_ERROR (0x1)`, `DOQ_REQUEST_CANCELLED (0x3)`.
- DoH3: standard DoH semantics (RFC 8484 GET with `dns` parameter) over an
  HTTP/3 (RFC 9114) connection; ALPN `h3`. Request-target encoding is the
  same origin-form contract as H1/H2; only the transport changes.

## 4. Why DoQ differs from the reuse-task deferral

The reuse task deferred "same-connection multi-outstanding + demux" because
correlating concurrent TCP queries without rewriting caller IDs needs an
unambiguous policy first. DoQ **structurally** avoids that problem: one query
per bidirectional stream, correlation by stream identity, wire ID fixed at 0.
The deferred question therefore does not transfer: this task's one-shot DoQ
(one connection, one stream, one query) needs no demux policy, and QUIC
multiplexing depth/backpressure/pooling remain a separate future task with
their own design.
