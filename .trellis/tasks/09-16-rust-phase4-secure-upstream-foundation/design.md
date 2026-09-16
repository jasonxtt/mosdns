# Secure upstream architecture proposal

Status: Slice0 implementation active, 2026-09-16. No planning/root-review PASS
is claimed; `implement.md` defines the remaining slice gates. Slice0 contracts,
TLS policy, and dependency/API evidence are implemented or recorded below.
Requirements live in `prd.md`, source evidence in
`research/secure-upstream-evidence.md`.

## 1. Dependency boundary and API shape

Extend `rust/upstream-core` rather than creating another runtime or copying the
Go transport hierarchy. It remains a sibling of sequence-core; dns-core is its
only MosDNS dependency. TLS/HTTP crates are ordinary library dependencies.

Proposed types (design signatures, not implemented APIs):

```rust
DotEndpoint::new(dial: SocketAddr, identity: ServerIdentity) -> Result<DotEndpoint, SecureError>
DohEndpoint::new(service_url: &str, dial: SocketAddr) -> Result<DohEndpoint, SecureError>
TlsPolicy::verified(roots: RootCertStore) -> Result<TlsPolicy, SecureError>
TlsPolicy::insecure_skip_verify() -> TlsPolicy
DotUpstream::new(endpoint: DotEndpoint, tls: TlsPolicy) -> Result<DotUpstream, SecureError>
DohUpstream::new(endpoint: DohEndpoint, tls: TlsPolicy) -> Result<DohUpstream, SecureError>
// Both owners:
exchange(ExchangeRequest<'_>, ExchangeContext) -> Result<SecureResponse, SecureError> // async
close() -> CloseResult // async
in_flight_exchanges() -> usize
```

`ServerIdentity` distinguishes validated DNS names and IP literals. `SecureResponse`
owns DNS wire and reports original request ID, returned response ID and an explicit
DoT/DoH + HTTP version enum; it must not report TLS as plain TCP. Preserve current
`Endpoint`, `Transport::{Udp,Tcp}`, `ExchangeResponse` and UdpTcpPolicy interfaces.
Keep common lifecycle and framing helpers crate-private; no general plugin API.
Secure errors wrap a typed existing control/transport cause where appropriate
rather than duplicating or stringifying every existing variant.

Verified trust is explicit constructor input, not a hidden OS scan per request.
Synthetic roots make tests deterministic. Native host will load platform roots
and map config flags; absence of that host code is not a silent Mozilla-only
trust-policy change. Insecure mode is an explicit equivalent of the existing
configuration field, not a fallback on TLS failure. It skips chain/name/time
checks but uses the selected crypto provider's TLS1.2/1.3 signature checks.

## 2. Endpoint construction and configuration boundary

- Numeric dial address is mandatory, port nonzero, IPv4 and IPv6 supported.
  Identity-only domain names never invoke system DNS. Future bootstrap passes
  a resolved numeric destination into this same boundary.
- DoT receives service identity separately; config-layer default port 853 and
  `dial_addr` parsing remain host work. DNS name SNI and certificate identity
  match the service; IP identity uses IP SAN verification and no DNS-name SNI.
- DoH parses an HTTPS service URL, deriving certificate identity and HTTP
  authority from its host. Include explicit nondefault service port in Host/
  :authority. Numeric dial override affects neither. Normalize an empty path
  to `/`; preserve escaped path and unrelated query fields, remove all decoded
  `dns` query keys, then append one encoded DNS message.
- Reject empty host, invalid port/identity, userinfo, fragments, unsupported
  scheme, CR/LF and malformed URI before dialing. Bound encoded request target
  at 96 KiB; a DNS query over 65535 bytes fails before encoding/network I/O.
  IDNA/IPv6 URL handling uses the selected parser; construction tests freeze
  normalization so a second hand-written parser is not introduced.
- No generic config object accepts unsupported bootstrap/pipeline/HTTP3/socket
  flags. Those fields remain preserved obligations in their follow-up tasks.
  `insecure_skip_verify` is covered here; loading YAML/JSON is not.

## 3. TLS setup and one-shot DoT flow

1. Validate request/endpoint, register under existing lifecycle admission lock.
2. Race fresh numeric TCP connect with owner/caller/deadline controls.
3. Build per-owner rustls client config with explicit ring provider and roots
   or explicit insecure mode; no client auth, early data or session resumption.
   Handshake races the same controls. No independent three-second reset.
4. Use existing generic `tcp::write_frame` on TLS stream, then explicitly flush
   under control. TLS's internal buffer makes write success insufficient evidence
   of completed send. Read with existing `tcp::read_frame`.
5. Reuse dns-core full response validation and QR/original-ID checks, retaining
   complete TC responses without invoking UdpTcpPolicy or plaintext fallback.
6. Final commit uses existing lifecycle lock ordering; drop stream and release
   registration. No peer-dependent close_notify wait may hold shutdown open.

Refactor only the private helper boundary required to share framing/control;
retain existing plain TCP tests. Do not duplicate the full TCP state machine
or weaken its error distinctions to accommodate TLS.

## 4. One-shot DoH flow

1. Admit exchange, copy borrowed DNS query, change only outbound copy's ID to 0.
2. Encode unpadded base64url into GET target; set Accept application/dns-message;
   no request body, Content-Encoding or default User-Agent.
3. Connect numeric destination and authenticate using service URL identity.
   Offer ALPN h2,http/1.1. Select negotiated h2 or http/1.1; absent ALPN means
   http/1.1 on this already-established TLS stream. Unexpected ALPN is terminal.
4. Use Hyper's low-level client connection APIs. Exactly one DNS GET per new
   connection; do not use reqwest or hyper-util's pooled/legacy client. HTTP2
   is supported without multiplexing independent caller queries on one stream.
5. Drive connection and request/body together under the same control scope.
   Status must be 200; no redirect/location following or retry on GOAWAY,
   refused stream, EOF, 429 or 5xx. Those are typed terminal errors.
6. Require media type application/dns-message, case-insensitive and allowing
   parameters. Reject missing/wrong MIME and non-identity Content-Encoding.
   Bound response header storage/list size at 16 KiB (protocol-specific builder
   settings verified in Slice0). Inspect Content-Length if supplied, but enforce
   size on actual incremental data too. Read at most 65536 bytes: the extra byte
   detects overflow. Require complete end-of-body, not a capped prefix.
7. Require 12..65535 bytes, QR and full dns-core validity. HTTP stream provides
   association, so do not demand the upstream DNS ID be 0; restore original ID
   in owned response. Any response metadata returned to caller must agree with
   that restored wire. No second DNS parser or query mutation.
8. Reap driver/executor work before releasing registration and perform final
   controlled commit immediately before returning success; owner/cancel/deadline
   checks during teardown prevent late success. No implicit HTTP cache or TTL
   rewriting; HTTP cache support needs a separate reviewed policy.

The GET method and caller-visible DNS behavior preserve source contracts.
Strict MIME/body validation and preserving non-dns URL parameters are explicit
proposed deviations, to be accepted by planning review before implementation.

## 5. Task ownership, cancellation and drop

Reuse existing Open -> Closing -> Closed admission/drain contract. Every
exchange's scope owns its socket, TLS stream, request/response buffers,
connection driver and any executor-spawned Hyper HTTP2 futures. Closing the
owner cancels all scopes and waits for their registrations and child work.
No lock may be held over network I/O or a task join.

Preferred driver structure is a pinned connection future polled alongside the
request/body in the exchange, rather than a detached spawned driver. Slice0
source inspection of the selected Hyper 1.11.0 low-level client shows this is
not wholly caller-driven: `hyper::client::conn::http2::handshake(exec, io)`
(`src/client/conn/http2.rs:77`) submits its connection driver through the
supplied `Executor` and returns a `Connection` that is only a dispatcher.
Internally `src/proto/h2/client.rs:192` calls
`exec.execute_h2_future(H2ClientFuture::Task { .. })` for the connection task,
`:556` and `:566` submit the body-pipe and send/response futures the same way,
and `Connection::send_request` (`src/client/conn/http2.rs:150`) merely
dispatches into a channel. `Executor::execute` (`src/rt/mod.rs:45`) therefore
observes every child. No detached TokioExecutor is allowed.
Slice3 must choose and prove, before implementation, either a re-entrancy-safe
queued executor (execute() only enqueues; the exchange's own driver polls the
queue and never recurses under a held borrow) or a tracked/abortable
child-task executor. Either way every spawn must hold the exchange scope's
liveness registration and observe its cancellation token. The scope seals
spawning during termination; after seal, execute() drops incoming work. Track a
child before spawning so there is no admission gap. Never create a second
runtime or use spawn_blocking for resolver/certificate work.

On normal completion/error: seal, cancel internal scope work, abort if needed,
join/drain child tasks, drop connection and sender, then release registration.
On caller future drop/abort: a non-async drop guard seals and cancels the scope,
requests aborts; children retain liveness registrations until their futures
are actually dropped. Owner close therefore cannot report Closed early. No
untracked async cleanup is spawned by Drop. Runtime shutdown is owned by the
future host and must follow upstream close/drain.

Slice0 established from the Hyper 1.11.0 API/source that the supplied Executor
sees every child (connection driver plus per-request send/pipe futures), so a
tracked executor can account for them. Slice3 tests must still prove that with
task counters/barriers after selecting the executor above; if the selected
version cannot meet this design, stop and revise the plan; do not quietly
detach tasks or widen pooling/runtime scope.

## 6. Error and side-effect matrix

`SecureError` must carry a typed phase plus the existing side-effect model.
Error formatting excludes URL credentials/query, response body and key material.

| Boundary | Required classification | State / retry |
| --- | --- | --- |
| Invalid endpoint/roots/query/URL/size | Construction/request error | NotSent; zero sockets |
| TCP connect failure | Connect | NotSent |
| Certificate/authentication/protocol handshake error | TLS with structured reason | NotSent for DNS application; no insecure/plain fallback |
| DoT write/flush pending or failed | Send with preserved cause | MaybeSent; no retry |
| DoT full frame flushed | Progress marker | Sent, not proof of remote processing |
| Before HTTP send future is polled | Progress marker | NotSent |
| HTTP request handed to driver, send/headers pending | HTTP send/receive error | Conservatively MaybeSent unless transmission completion observed |
| HTTP response headers observed | Progress marker | Sent |
| HTTP status/MIME/header/body/ALPN failure | Typed protocol error | Current state, no follow-up connection |
| Invalid DNS response / wrong DoT ID | Typed DNS response error | Sent |
| Owner/caller/deadline at any stage | Existing distinct control cause | Preserve last state, priority owner > caller > deadline |

Handshake has network side effects but cannot transmit DNS with early data
and buffered application writes disabled; document that NotSent is scoped to
DNS query execution, not to absence of any network bytes. Do not expand the
existing enum to Unknown. No error is used to automatically retry in this task.

## 7. Compatibility and deferral matrix

| Concern | Decision | Evidence / acceptance |
| --- | --- | --- |
| Query ownership and original ID | Preserve | Go ExchangeContext contract; AC3/4 |
| Separate dial/service identities | Preserve | Opt.DialAddr; AC1 |
| Existing insecure config intent | Preserve explicit opt-in | forward TLSConfig; AC2 |
| DoT framing and response validation | Preserve + existing Rust validation | TCP helper contracts; AC3 |
| DoH GET, zero outbound ID, original return ID | Preserve | doh/upstream.go; AC4 |
| h2 + HTTP/1.1 | Preserve protocol capability | Go ConfigureTransports; AC4 |
| Caller cancellation vs detached six seconds | Intentional deviation | existing Rust ownership contract; AC5 |
| MIME/full DNS/oversize checks | Intentional stricter validation | protocol evidence + Rust dns-core; AC4 |
| Unrelated URL query fields | Intentional preservation instead of Go overwrite | source RawQuery assignment; AC4 |
| Fresh connection / no session cache | Bounded deferral of optimization | later lifecycle task, not final cutover performance claim |
| Config pipeline/idle/HTTP3/bootstrap | Preserved product obligation, deferred implementation | API/YAML/UI source; reject/keep unavailable until supported |
| Root discovery/platform policy | Deferred host composition | explicit roots now, AC2 with synthetic trust |
| Go fallback, transport ABI and selectors | Prohibited | native architecture; AC8 |

## 8. Dependencies, files and verification boundaries

Propose rustls 0.23/tokio-rustls 0.26, hyper 1/hyper-util 0.1 (Tokio IO only),
http-body-util 0.1, bytes 1, base64 0.22, url 2. Precise compatible versions,
Cargo features/licenses and transitive MSRV must pass Slice0 dependency review;
no version is added or locked by this planning document. Keep unsafe forbidden
in project code; third-party crypto's audited implementation is not a new local
unsafe socket layer. Synthetic certificate fixtures can avoid a test generator
crate in the locked workspace; record generation commands and validity periods.

Expected future files: `rust/upstream-core/src/secure/` (mod, endpoint, tls,
dot, doh, scope modules as necessary), minimal private helper/export changes
in lib.rs/tcp.rs, secure contract tests/fixtures, upstream Cargo.toml/Cargo.lock,
focused CI and task evidence. No dns-core changes expected; unforeseen required
public parsing changes require a separate design review. No sequence/runtime,
Go/plugin/config/WebUI files are in implementation scope.

## 9. Review, rollback and later integration

Every slice stops at its own review boundary. Failure of a new secure slice
must leave accepted UDP/TCP functionality usable; rollback is removal/revert of
that exact slice's secure files/manifest change, not a Go runtime fallback.
Do not delete/rewrite accepted historical evidence. Foundation performance can
record handshake cost and bounded resource counters, but only a later pooled/
composed-host task can make production throughput claims.

No production rollout is part of this task. Endpoint-resolution, connection
lifecycle, socket options, QUIC/HTTP3 and listeners remain Phase4 follow-ups;
Phase5 composes them with YAML/API/WebUI/audit/metrics, and Phase6 retires hybrid
scaffolding after full native E2E. This proposal does not approve those tasks.
