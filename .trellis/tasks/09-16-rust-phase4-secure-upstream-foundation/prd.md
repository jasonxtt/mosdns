# Rust Phase 4 TLS/HTTPS upstream foundation

## Authorization and status

2026-09-16: the user authorized closing the accepted UDP/TCP foundation and
planning its successor, explicitly saying **do not begin execution**. This task
is `planning`; the package is a proposal for review, not an accepted planning
gate or implementation authorization. Do not run `task.py start`, add crate
code/tests/dependencies, dispatch implementation, deploy, commit or push as part
of this planning request.

Subsequent authorization on 2026-09-16 permits committing and pushing all
wrap-up and planning artifacts to GitHub. It does not authorize activation or
implementation.

## Goal and value

Provide the next pure Rust data-plane building block: bounded DNS-over-TLS
(DoT) and DNS-over-HTTPS (DoH) exchanges that a future native host can compose
without Go/cgo. Preserve service identity, query ownership, DNS results and
configuration meaning while making cancellation and resource lifetime explicit.
This task is a transport foundation, not a complete forward plugin or host.

## Background and evidence

- UDP/TCP and TC→TCP foundation is accepted and archived at
  `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/`;
  final review `9d43e9f`, closure record `cb15361`, Actions `35090514316`.
- `rust/upstream-core/src/lib.rs:26` supports only numeric UDP/TCP endpoints.
  New secure types must not reinterpret those already-reviewed endpoints.
- `pkg/upstream/upstream.go:67` separates DialAddr from TLS/HTTP identity;
  DoT is constructed at line 353 and DoH at line 402.
- `plugin/executable/forward/forward.go:67` and
  `coremain/api_upstream.go:42` expose dial address, insecure verification,
  pipeline, HTTP3, idle timeout and bootstrap. They are user-facing settings,
  even when this bounded library does not implement their host/config mapping.
- `pkg/upstream/doh/upstream.go:77` copies query bytes, zeroes outbound ID and
  restores the caller ID. Its detached timeout is implementation behavior,
  not the Rust cancellation contract. Full classification and source anchors
  are in `research/secure-upstream-evidence.md`.

## Requirements

### R1 — Pure Rust boundary and bounded endpoint scope

Extend upstream-core with separately typed DoT/DoH owners on the existing
host-owned Tokio runtime. Each exchange uses one fresh numeric TCP destination
supplied by its caller and a separate service DNS name or IP identity. For
DoH, the HTTPS authority and path remain those of the service URL even when the
dial destination differs. No implicit DNS lookup, Go callback or hidden runtime.
A hostname used for TLS identity is supported; resolving a hostname is deferred.

### R2 — Explicit TLS authentication

Verified TLS is the default, using caller-supplied trust roots and service
identity. Reject an empty verified root store or invalid identity before dial.
Support the explicit opt-in equivalent of existing `insecure_skip_verify`,
without ever turning verification off after an authentication failure. Certificate
chain/name/time checks are skipped only in that explicit mode; TLS handshake
signature verification remains enforced. No mTLS, 0-RTT or session resumption
in this first foundation. TLS1.2/1.3 support is subject to the selected library's
safe defaults, not custom cipher negotiation. No certificates or keys from a
real service enter the repo; tests use synthetic fixtures.

### R3 — Correct bounded DoT exchange

Preserve the borrowed query and original ID; send one two-byte-length-prefixed
DNS message over a newly authenticated TLS stream. Flush TLS output, read one
complete DNS frame, validate QR/ID and complete wire using dns-core, and return
owned bytes. Retain existing TCP frame/size semantics and expose typed failures.
TC on a complete DoT response is returned; it never causes plaintext fallback.

### R4 — DoH over HTTP/2 and HTTP/1.1

Use HTTPS GET with unpadded base64url `dns` query encoding to preserve the
existing client method. Zero the ID only in an owned outbound copy; return a
validated owned DNS response with the original caller ID. HTTP request/stream
association owns response association; a remote DNS ID is not a routing key.
Negotiate h2/http1.1 with ALPN; absent ALPN permits HTTP/1.1 on the same TLS
connection, but no failed request is replayed using another protocol.
Require HTTP 200, DNS media type and a complete DNS body no larger than 65535
bytes. No redirects, decompression, HTTP cache, proxy, retry or h3 fallback.
Preserve URL path and non-dns query parameters; replace all existing `dns`
parameters with one generated value. Reject userinfo/fragments and non-HTTPS
URLs. These deliberate differences from current Go edge behavior are listed
in the compatibility matrix; no Go/UI behavior changes in this task.

### R5 — One deadline, cancellation and deterministic close

Connect, handshake, protocol I/O and final commit share the caller's original
absolute deadline. Owner close wins over caller cancellation, which wins over
deadline, which wins over response commit. Every socket, TLS stream, HTTP driver
and HTTP library executor task is tracked and drained; aborting/dropping an
exchange cannot detach work. `close().await` rejects new admissions and returns
only after all owned work is reaped. No sleeps used as concurrency ordering.

### R6 — Side-effect and error evidence

Typed errors identify construction/connect/TLS/HTTP/DNS/control phase and retain
NotSent/MaybeSent/Sent for the application query. TLS handshake traffic alone
is not a DNS send. Entering a possibly partial application send is MaybeSent;
DoT full write plus flush, or evidence of HTTP request transmission/response,
can advance to Sent. Never claim a generic HTTP send error proves NotSent.
No automatic retransmission or retry at any state in this task.

### R7 — Preserve earlier gates; prove the new boundary

Keep UDP/TCP/TC behavior and sibling crate boundaries intact. Use deterministic
local TLS/HTTP test servers, synthetic trust and loopback sockets, plus isolated
Linux CI coverage, not public DNS services. Review dependency features, licenses,
MSRV and hidden executor behavior before runtime implementation. This is not a
production performance or full protocol-matrix completion claim.

## Acceptance criteria

- [ ] AC1 / R1: numeric dial override leaves TLS identity and DoH authority/path
  unchanged; invalid endpoint cases fail before network I/O; no resolver called.
- [ ] AC2 / R2: trusted-name success; wrong name, expiry and unknown CA fail;
  explicit insecure mode succeeds only as requested; bad handshake signatures
  still fail; handshake stalls obey caller deadline/cancel/close.
- [ ] AC3 / R3: DoT framing covers partial reads/writes/flush, zero/oversize
  lengths, EOF and malformed/wrong-ID responses; query bytes stay unchanged.
- [ ] AC4 / R4: h2 and HTTP/1.1 succeed against isolated servers; server sees
  correct GET encoding/authority and zero outbound ID; caller receives original
  ID. Status, MIME, oversize/truncated body, URL query and ALPN cases are explicit.
- [ ] AC5 / R5: cancellation and close at each I/O phase and immediately before
  final commit cannot return a late success; all registrations, sockets and
  driver/executor tasks drain on normal/error/drop/abort paths.
- [ ] AC6 / R6: fault injection proves side-effect classification and exactly
  one application request; redirects, disconnects and h2 stream errors never
  trigger hidden retry, plaintext fallback or another connection.
- [ ] AC7 / R7: Rust fmt/test/clippy/release, dependency review, existing transport
  regressions and Linux secure loopback tests pass; existing Go gates remain green.
- [ ] AC8 / R1,R7: no Go/cgo/ABI/selector, host/listener, YAML/API/WebUI, deployment,
  metrics schema or hybrid retirement change; final evidence states limitations.

## Out of scope and follow-up ownership

- Resolver/bootstrap and its TTL/cache/refresh/dual-stack behavior: separate
  Phase4 endpoint-resolution task. Supply numeric addresses here; never silently
  accept and ignore bootstrap configuration. Preserve future config support.
- TCP/DoT/HTTP pooling, reuse, multiplexing across caller queries, pipeline and
  idle settings: separate connection-lifecycle task. One request per fresh
  connection is a bounded foundation limitation, not final performance policy.
- SOCKS5, SO_MARK, BindToDevice/local bind: separate socket-policy task.
- QUIC/HTTP3 and listeners: subsequent Phase4 protocol/server tasks.
- YAML/plugin registration, root-store OS loading, audit/metrics mapping and
  all forward/config options: Phase5 host composition with explicit coverage.
- POST selection, HTTP caching/TTL age adjustment, mTLS and URL templates beyond
  the existing endpoint shape: separate reviewed extension if required.
- Go bridge expansion, production/default switch and hybrid retirement: excluded.

## Planning convergence

The bounded scope follows the accepted next-step direction and existing product
contracts. The proposed technical choices and intentional deviations are explicit
in `design.md`; there is no unanswered product question needed to write this
planning package. Dependency lock/MSRV and executor-lifecycle preflight are
specified Slice0 gates, not claims of completed experiments. The final package
still requires review and a subsequent explicit implementation authorization;
this request grants neither.
