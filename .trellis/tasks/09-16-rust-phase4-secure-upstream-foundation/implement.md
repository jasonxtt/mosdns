# Secure upstream implementation plan — Slice4 active

Historical record, 2026-09-16: implementation was first authorized for the
bounded Slice0 dependency/MSRV and Hyper API-inspection scope only. That
authorization superseded the earlier planning-only boundary for Slice0; it did
not authorize Slice1+, CI mutation or deployment, and each later slice still
needed its own review and explicit go-ahead. Slice0's technical checklist was
then completed under its separate review boundary.

2026-09-17: Slice1 was explicitly accepted by `rust0916` at
`25c7c961453e15d7347d65bbc401026f813ff27c` (`PASS / Slice1 CLOSED`). The user
then explicitly authorized Slice2, which was closed at its own review boundary.
Slice3 was subsequently implemented and, after one scoped remediation round,
formally accepted as `PASS / Slice3 CLOSED` at
`d3566bf105008e23c315536d6560b00d55250e55`. The user has now explicitly
authorized Slice4 as the final quality and isolated Linux evidence gate;
Slice4 is the active bounded scope. Production wiring, deployment, and
automatic progression to any later slice remain unauthorized. The executor
routing and prompt-approval contract is recorded in
`.trellis/spec/backend/quality-guidelines.md`.

## Planning package review checklist

- [x] Existing foundation PASS/CLOSED and archival recorded.
- [x] Local Go, config/API/UI and current Rust boundaries inspected.
- [x] Requirements, source anchors, preservation/deviation and explicit deferrals
  consolidated in PRD/design/research; no temporary TBD requirement remains.
- [x] Dependencies researched using official docs; no Cargo file modified.
- [x] Endpoint resolution separated from service identity; future user-facing
  bootstrap/pipeline/HTTP3/idle/socket obligations retained.
- [x] Cancellation, final commit, TLS flush, HTTP2 task ownership and send-state
  rules made testable; per-slice stop boundaries written.
- [ ] Planning package independently/root reviewed if required by the agreed
  execution workflow; record reviewer, exact revision and disposition.
- [x] Subsequent explicit user authorization for activation and Slice0.
  Evidence: the 2026-09-16 user request explicitly authorizes implementation
  according to this plan and the bounded review loop through `rust0916`.

The task's historical `codex.dispatch_mode=inline` setting does not override
the current user-selected Herdr topology: when the quality-spec detection
contract is satisfied, the adjacent Claude Code pane is the bounded executor
and this Codex session remains controller. `rust0916` remains the review
destination. Do not invent another review thread or send work to a different
destination.

## Slice0 — contracts, dependency and lifecycle preflight

Goal: freeze public types, helper reuse and selected libraries before TLS I/O.

- [x] Re-read approved planning and trellis-before-dev; inspect current git state.
  Evidence: parent session re-read the task package/specs and verified branch,
  remote, dirty paths, and preserved unrelated `.DS_Store` files before each
  clean DSH dispatch.
- [x] Resolve candidate dependencies under the actual workspace MSRV (1.85),
  record exact version/features/license graph; do not silently bump MSRV.
  Evidence: `research/secure-upstream-evidence.md` → "Slice0 dependency/MSRV
  resolution record" (resolver 3; hyper 1.11.0 / hyper-util 0.1.20 /
  http-body-util 0.1.3 / tokio-rustls 0.26.4; no resolved package above 1.85.0).
- [x] Inspect selected Hyper HTTP1/2/Hyper-util APIs and sources for retries,
  executor spawns, header bounds and drop semantics; document every owned child.
  Prove feasibility of sealed tracked executor and in-flight accounting.
  Evidence: same research record → "Hyper 1.11.0 HTTP/2 ownership source
  locations": the connection driver and per-request futures all flow through the
  caller-supplied `Executor`. Slice0 API/source feasibility is established; the
  executor itself is selected and proven with task counters/barriers in Slice3,
  so no working executor or in-flight accounting is claimed here.
- [x] Review rustls provider/root/config and insecure verifier interfaces;
  ensure handshake signature checks stay enabled and no 0-RTT/resumption.
  Evidence: `rust/upstream-core/src/secure/tls.rs` and `tests/slice0_secure.rs`;
  `TlsPolicy::verified` requires a non-empty caller root store, while
  `insecure_skip_verify` is explicit. No ClientConfig, verifier override, I/O,
  0-RTT or resumption is constructed in Slice0.
- [x] RED/GREEN constructor and pure request-building contracts: identity/dial
  separation, IPv4/IPv6, invalid roots/identity/port/URL, query/path normalization,
  maximum DNS/URL size and explicit insecure option.
  Evidence: `src/secure/{endpoint,error,tls}.rs` plus 24 focused secure tests;
  endpoint construction is pre-I/O and preserves dial/identity separation,
  URL authority/path/query and explicit TLS mode. `DohEndpoint::get_request_target`
  adds the pure ID-zeroed, unpadded URL-safe GET encoder, decoded `dns` removal,
  and 65535-byte DNS/96 KiB target limits.
- [x] Introduce only reviewed secure types/helper access and dependencies;
  preserve existing UDP/TCP public types and default behavior.
  Evidence: secure module re-exports are additive; locked dependency and Hyper
  API evidence is in `research/secure-upstream-evidence.md`; parent reran 125
  upstream-core tests and workspace checks without changing UDP/TCP modules.

Allowed: upstream-core secure contract skeleton/tests, minimal lib.rs/tcp.rs
private sharing, upstream manifest/lockfile, task evidence. No TLS/HTTP socket
implementation yet. Dependency or ownership infeasibility stops for re-planning.
STOP for scoped review; Slice1 requires new authorization.

## Slice1 — TLS and DoT one-exchange primitive

- [x] One RED -> GREEN behavior at a time; synthetic local CA/server fixtures
  with documented provenance (valid, wrong name, expired and unknown issuer).
- [x] Numeric connect, authenticated handshake, shared context/deadline, explicit
  insecure path; no query before successful handshake and no plaintext fallback.
- [x] Reuse exact framing helpers; explicitly test TLS flush, partial writes,
  partial prefix/body, EOF, wrong ID, malformed/full-TC response and size limits.
- [x] Assert unchanged query/original ID, fresh connections per exchange,
  concurrent isolation and typed TLS/send/receive/control error state.
- [x] Test owner close, caller cancellation, deadline and dropped/aborted future
  during connect, handshake, frame write/flush/read and final commit; drain all.

Allowed: secure TLS/DoT modules, tests/fixtures and minimal private helper changes.
STOP for scoped review; no DoH implementation or connection reuse in this slice.

### Slice1 execution and evidence record — 2026-09-16

Implementation is complete and stops at the scoped review boundary. No DoH,
HTTP, connection pooling/reuse, resolver/bootstrap, socket policy, listener,
host composition, YAML/API/WebUI, or Go/cgo/selector/fallback work is included.

Produced behavior and contracts:

- `rust/upstream-core/src/secure/dot.rs`: `DotUpstream` (one fresh numeric
  connection, authenticated handshake before any DNS byte, one framed
  query/response exchange), `SecureResponse`, and `SecureTransport::Dot`.
  Reuses the existing `tcp::{write_frame, read_frame, encode_frame}` framing and
  the shared `tcp::race_control` control race; it adds only `tcp::flush_bytes`
  for the TLS write-buffer flush that plain TCP does not need. No second framing
  implementation and no duplicated TCP state machine.
- `rust/upstream-core/src/secure/tls.rs`: per-exchange rustls client
  configuration with an explicit `ring` provider and safe default protocol
  versions; `server_name_for` derives the SNI/service name from the service
  identity only; `classify_handshake_error` maps structured rustls outcomes to
  typed reasons. The insecure mode skips chain/name/time checks but still runs
  the provider's TLS1.2/1.3 handshake signature verification.
- `rust/upstream-core/src/secure/error.rs`: `SecureError::{Tls, Transport}`,
  `TlsHandshakeFailure`, `CertificateRejection`, and `TlsConfigError::Provider`.
  A handshake failure is always `NotSent`; a transport cause keeps its own
  tracked `SideEffectState`.
- `rust/upstream-core/src/lib.rs` / `src/tcp.rs`: only visibility and generic
  signature changes so the secure path shares the reviewed lifecycle gate,
  final-commit linearization, and control race.

Fixtures (`rust/upstream-core/tests/fixtures/`): synthetic EC P-256 roots A/B
and four leaves. **Superseded by the second remediation:** the original version
of this paragraph described DER constants, including PKCS#8 private keys,
generated once with OpenSSL 3.6.4 and checked in. All of that material has been
removed; the fixtures are now generated in memory at test runtime and no
certificate or key bytes are committed. See the P1-3 record below.

RED/GREEN evidence (focused, macOS arm64, cargo/rustc 1.95.0):

- `a_verified_policy_never_retries_after_a_rejection` and the four certificate
  cases were proven to fail for the right reason by mutation A, which routed the
  verified policy through the insecure verifier: `certificate_for_a_different_name`,
  `expired_certificate`, `certificate_from_an_untrusted_issuer`, and
  `a_verified_policy_never_retries_after_a_rejection` all FAILED, then passed
  after restoring the source. `the_same_untrusted_certificate_verifies_under_its_own_root`
  and `insecure_policy_...` stayed green, confirming the failures were caused by
  verification and not by the fixture set.
- Mutation C removed the response/request ID equality check:
  `a_response_with_a_different_transaction_id_is_a_mismatch` FAILED, then
  passed after restore.
- Mutation D replaced the insecure verifier's delegated signature checks with an
  unconditional success: `a_bad_handshake_signature_fails_even_under_the_insecure_policy`
  FAILED, then passed after restore. This is the focused proof that the explicit
  insecure mode skips only chain/name/time checks and still enforces the
  provider's handshake signature verification. The paired test
  `a_bad_handshake_signature_also_fails_under_the_verified_policy` asserts the
  same typed `Certificate(BadSignature)` outcome under verified TLS; both
  present the genuine trusted `dns.example` leaf with a foreign private key, so
  the chain, name, and validity window all pass and only the handshake signature
  is wrong.
- Both mutations used a repo-local `mktemp target/slice1-*-backup.XXXXXX` copy
  with a `trap`-guard that restored the source on every exit path; the temporary
  copies were removed afterwards and `git diff --check` is clean.

Verification commands and results in this environment:

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS (clean) |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 155 tests (slowest target `slice1_dot`: 27 passed) |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, all targets ok |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS (no warnings) |
| `cargo build --manifest-path rust/Cargo.toml --workspace --release --locked` | PASS |
| `python3 .trellis/scripts/task.py validate rust-phase4-secure-upstream-foundation` | PASS (4 + 4 entries) |
| `git diff --check` | PASS (clean) |

Toolchain limitation: the required `cargo +1.85.0 check` MSRV command could NOT
run here because the 1.85.0 toolchain is not installed (`rustup toolchain list`
shows only `stable-aarch64-apple-darwin` and `nightly-aarch64-apple-darwin`).
Actual toolchain used was cargo/rustc 1.95.0. MSRV compatibility for this slice
is therefore not re-proven by a 1.85 build; no new dependency or lockfile change
was made in Slice1, so the Slice0 resolver-3 MSRV record still governs.

Scope limitations: results are macOS arm64 loopback evidence only and are not
Linux, production, throughput, or long-running deployment evidence. Covering
stored (`root_count`) and `TlsPolicy` accessor behavior is unchanged Slice0
contract. DoH, ALPN, HTTP, pooling, resolver, and host wiring remain Slice2+.

## Slice2 — bounded DoH over HTTP/1.1

- [x] Pure GET encoding tests precede I/O: zero outbound ID without mutation,
  unpadded base64url, path and unrelated query fields, duplicate dns replacement,
  service Host identity independent of numeric dial address.
- [x] Low-level Hyper HTTP1 driver with no pooling/retry/redirect/proxy, served
  by local TLS fixture; missing ALPN and negotiated HTTP1 are both covered.
- [x] Incremental body/header limits and full DNS response validation: 200,
  MIME parameters/case, missing/wrong MIME, 3xx/4xx/5xx, Content-Encoding,
  Content-Length mismatch, chunked body, 65535 vs 65536 bytes and early EOF.
- [x] Restore caller ID regardless of remote DNS ID; assert owned response
  metadata agrees with wire. No implicit cache/TTL adjustment.
- [x] One absolute deadline and drop/close coverage across header/body/commit;
  request counter proves at most one GET on failure, never a second connection.

Allowed: secure DoH module/request builder and HTTP1 tests/fixtures.
STOP for scoped review; no HTTP2 executor/pool in this slice.

### Slice2 execution and evidence record — 2026-09-17

Implementation is complete and stops at the scoped review boundary. No HTTP/2,
pooling/reuse, proxy, resolver/bootstrap, socket policy, listener, host
composition, YAML/API/WebUI, or Go/cgo/selector/fallback work is included.

Produced behavior and contracts:

- `rust/upstream-core/src/secure/doh.rs`: `DohUpstream` (one fresh numeric
  connection, authenticated handshake against the service URL identity, exactly
  one HTTPS `GET`), plus the in-crate deterministic `DohPhase` seam.
- `rust/upstream-core/src/secure/error.rs`: `DohProtocolError` and
  `SecureError::DohProtocol`. The variant is `Sent`, never `NotSent`, because it
  can only be observed after the request was transmitted.
- `rust/upstream-core/src/secure/dot.rs`: `SecureTransport::Doh` and a DoH
  `SecureResponse` constructor that restores the caller ID and reports response
  metadata that agrees with the returned wire.
- `rust/upstream-core/src/secure/tls.rs`: `client_config_with_alpn`, which
  builds the same reviewed policy with an explicit ALPN list.
- `rust/upstream-core/tests/fixtures/mod.rs`: `root_chain()` for a server that
  must present its issuer.

Design points that the review should confirm:

- **Low-level Hyper HTTP/1.1 only.** `hyper::client::conn::http1` is used; no
  `client-legacy`, `client-pool`, or `hyper-util` legacy client is involved. The
  `http1::Connection` is itself a `Future` that the exchange polls inline, so no
  background task or executor owns any part of the exchange and there is no
  detached driver to reap.
- **ALPN.** Exactly `http/1.1` is offered. `h2` is deliberately **not**
  advertised, because a negotiated protocol this client cannot drive would be a
  silent fallback; Slice3 adds `h2` with its scoped HTTP/2 driver. Absent ALPN
  is accepted (HTTP/1.1 on the established stream) and any other negotiated
  protocol is a typed terminal `UnexpectedAlpn`.
- **One GET.** A terminal failure never retries, follows a redirect, or opens a
  second connection. This is proven by a server-side request counter.
- **Request shape.** No body (`EmptyBody` is a closed type with a zero
  exact-size hint), no `User-Agent`, no request `Content-Encoding`; `Host` is
  the service authority from the URL, never the numeric dial address.
- **Response validation.** Status must be 200; the media type must be
  `application/dns-message` case-insensitively with parameters allowed; a
  `Content-Encoding` other than `identity` is rejected; the body is bounded
  incrementally at 65535 bytes and must reach a complete end. The caller's
  original ID is restored into the owned wire, and the upstream's own DNS ID is
  not used as a routing key.

RED/GREEN evidence (focused, macOS arm64; every mutation used a repo-local
`mktemp target/*` copy with a `trap` restore, and no backup remains):

| Mutation | Result |
| --- | --- |
| Remove the media-type check | `a_missing_or_wrong_media_type_is_rejected` FAILED |
| Remove the incremental body-size bound | `a_body_larger_than_the_dns_maximum_is_rejected` FAILED |
| Remove the status check | `a_non_200_status_is_a_typed_protocol_error` FAILED |
| Accept 3xx as success | `a_redirect_is_not_followed` FAILED |
| Ignore `Content-Encoding` | `a_non_identity_content_encoding_is_rejected` FAILED |
| Remove caller-ID restoration | `the_outbound_query_id_is_zeroed_while_the_caller_id_is_restored` FAILED |
| Treat early EOF as end-of-body | both incomplete-body tests FAILED |
| Add a `User-Agent` header | `a_successful_exchange_sends_a_get_with_no_body_and_no_user_agent` FAILED |
| Send the dial address as `Host` | `the_authority_is_the_service_host_not_the_numeric_dial_address` FAILED |
| Retry once on failure | `a_terminal_protocol_failure_never_sends_a_second_request` FAILED (request count 2 vs 1) |

Two of these mutations initially passed, and the tests were strengthened rather
than the mutations relaxed: the status/redirect cases now carry a *valid* DNS
body so only the status can cause rejection, and the no-retry case now uses a
fast peer-driven failure (503) because a deadline failure cannot reconnect and
therefore could never detect a retry.

Deterministic control matrix (no sleeps):

- Five pre-result phases (`BeforeConnect`, `BeforeHandshake`, `BeforeRequest`,
  `BeforeBody`, `BeforeCommit`) crossed with four controls (owner close, caller
  cancellation, the one shared absolute deadline, and dropped/aborted future) =
  20 deterministic cells sharing one helper. Each cell first observes the seam
  and the live registration, so the control is proven to land at the intended
  phase.
- Side-effect layering is asserted per phase and must agree across all four
  controls, mirroring the reviewed table: `NotSent` before connect, handshake,
  and before the request is handed to the driver; `Sent` once response headers
  have been observed (body and commit phases).
- A dedicated test asserts the phase array contains exactly the five distinct
  pre-result phases, so a phase cannot be silently dropped.
- No test uses a sleep to establish ordering; `BeforeBody`/`BeforeCommit` tests
  are also bounded by a short deadline so a missing response cannot stall CI.

Verification commands and results in this environment:

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice2_doh --locked` | PASS, 26 tests |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 201 tests |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, 23 targets |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS, no warnings |

Toolchain limitation (unchanged): `cargo +1.85.0 check` still cannot run because
Rust 1.85.0 is not installed; actual toolchain is cargo/rustc 1.95.0. Slice2 adds
no dependency, so the MSRV position does not change.

Deferred beyond this slice (explicit, not silently dropped): HTTP/2 (Slice3),
pooling/reuse, proxies, redirect handling, HTTP cache/TTL adjustment, h3, and
host wiring. Results are macOS loopback evidence only, not production,
throughput, or long-running deployment evidence.

### Slice2 root-review remediation — 2026-09-17

The root review of `3702998` returned `BLOCKED / FAIL` with P0=0 and three P1
findings. Only those three were addressed; no later slice, production wiring,
or scope widening is included.

**P1-1 — side-effect semantics.**
`SecureError::DohProtocol` previously reported `Sent` for every variant and for
the whole error, which was wrong in two places:

* `UnexpectedAlpn` is decided during the TLS handshake, before any HTTP request
  exists, so it is now `NotSent`.
* A connection that ended before any response head was observed was mapped to
  `IncompleteBody`, i.e. `Sent`. Whether the request reached the peer is
  unknowable at that point, so it now has its own variant,
  `DohProtocolError::ResponseHeadNotReceived`, classified `MaybeSent`.

The second review round also required the handoff itself to be a real
linearization point. Previously the `AfterRequestSent` seam ran *before*
`SendRequest::send_request`, so the phase still described a `NotSent` window.
The exchange now performs the last provably-`NotSent` control check, then calls
`send_request` synchronously (which dispatches the request into the connection
driver), and only then reaches the `AfterRequestSent` seam; the post-handoff
select races the connection, the request and all controls with a `MaybeSent`
baseline. The seam therefore observes a state in which the query may already be
on the wire, which is what the phase is named for. The matrix remains 6 x 4 = 24
cells with the same four controls, drain assertions and at-most-one-request
counter. `an_absent_response_head_is_never_reported_as_sent` is tightened from
"not `Sent`" to exactly `MaybeSent`.

Classification is now per-variant via `DohProtocolError::side_effect`, and every
defect that can only be observed after a complete head (status, media type,
encoding, head size, body size, incomplete body) remains `Sent`. The
deterministic matrix grew from five to six pre-result phases — adding
`AfterRequestSent`, the window where the request is with the driver but no head
exists — so it is now 6 x 4 = 24 cells, each asserting the layering, the
registration drain and at most one request.

**P1-2 — response preservation.**
The exchange used `dns_core::patch_response_id_ra`, which is the frozen
server-response oracle and also sets RA. A DoH client must return the upstream's
response with only the transaction ID rewritten, so it now uses a local
`restore_request_id` that rewrites bytes 0-1 and preserves every other byte.
Tests assert a real HTTP/1.1 response with RA clear stays RA clear, that
`wire[2..]` matches the upstream byte for byte, that an RA-set control stays set,
and that the reported metadata agrees with the returned wire.

**P1-3 — response bounds.**
Header bounding was a count (`max_headers(64)`) plus a transport buffer size,
not the 16 KiB byte bound the design requires.

The second review round narrowed this further: `response_head_bytes` reconstructs
the head from *parsed* fields, which Hyper has already reduced to a status code
and a header map, so it cannot see bytes the parser discarded — most obviously a
long non-canonical reason phrase, which is not retained unless the `ffi` feature
is enabled. A post-parse check alone therefore cannot be the raw bound.

The raw bound is now enforced during parsing: the Hyper HTTP/1.1 client builder
sets `max_buf_size` to `MAX_RESPONSE_HEADER_BYTES` (16 KiB), so the parser aborts
the connection once the head bytes it has buffered reach the limit, before the
exchange ever sees a head. `response_head_bytes` is retained as
defense-in-depth, and the documented comment now states explicitly that it
cannot see discarded bytes and is therefore secondary.

`Content-Length` above 65535 is still rejected at the head stage, and the
incremental body bound is still applied to the bytes actually received, so a
response that omits or understates `Content-Length` is still stopped.

Tests: a head one byte under the bound is accepted and a head well over it is
rejected; the raw bound is proven by a real HTTP/1.1 loopback exchange whose head
is a 20 KiB non-canonical reason phrase, which the post-parse reconstruction
would underestimate but the parser now refuses. A genuinely valid 65535-byte DNS
response is accepted against 65536 rejected (the fixture is built to an exact
length from real A records, not filler), and both a chunked and a
close-delimited 65536-byte body — neither carrying `Content-Length`, so the
head-stage check cannot fire — are rejected by the incremental gate.

RED evidence (each mutation applied with a repo-local `mktemp target/*` copy and
a `trap` restore; no backup remains):

| Mutation | Result |
| --- | --- |
| Re-add the RA forcing | `a_ra_clear_response_keeps_ra_clear_and_only_rewrites_the_id` FAILED |
| Restore `IncompleteBody` for an absent head | `an_absent_response_head_is_never_reported_as_sent` FAILED |
| Force `UnexpectedAlpn` to `Sent` | `the_side_effect_classification_of_each_doh_defect_is_explicit` FAILED |
| Remove the head byte bound | `a_response_head_over_the_byte_bound_is_rejected` FAILED |
| Remove the head-stage `Content-Length` check | `a_declared_content_length_over_the_dns_maximum_fails_at_the_head` FAILED (`IncompleteBody` instead of `BodyTooLarge`) |
| Widen the raw parser buffer back to 32 KiB | both raw-wire head tests FAILED (the over-long head parses and the exchange *succeeds*) |
| Remove the incremental body bound | both chunked and close-delimited 65536-byte tests FAILED |

Verification after remediation (macOS Darwin 25.5.0 arm64; cargo/rustc 1.95.0):

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice2_doh --locked` | PASS, 39 tests |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 215 tests |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, 23 targets |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS, no warnings |
| `python3 .trellis/scripts/task.py validate rust-phase4-secure-upstream-foundation` | PASS |
| `git diff --check` | PASS |

The toolchain limitation is unchanged: `cargo +1.85.0 check` cannot run because
Rust 1.85.0 is not installed. This remediation adds no dependency.

## Slice3 — HTTP/2 and complete structured shutdown

- [x] Implement only reviewed ALPN dispatch and scoped HTTP2 executor/driver.
  No independent queries share a connection; no h3 or failed-request replay.
- [x] Test h2 success, service authority/path, concurrent independent owners,
  unexpected ALPN, reset/GOAWAY/refused stream/EOF and send-state classification.
- [x] Track all executor futures before spawn; freeze spawn admission at teardown;
  prove child liveness retains owner registration when caller future is aborted.
- [x] Explicit barriers cover cancellation and close while handshake, request,
  headers, data, final validation/commit or driver teardown are parked.
- [x] Record zero sockets/registrations/driver/executor tasks after close on
  success, all errors, dropped requests, and repeated/concurrent close.
- [x] Server-side counters prove no hidden library retry or protocol fallback.

Allowed: scoped executor/DoH HTTP2 implementation and lifecycle tests. If API
behavior violates the ownership design, STOP and revise; do not weaken drain.
STOP for scoped review before final evidence slice.

### Slice3 execution and evidence record — 2026-09-17

Implementation is complete and stops at the scoped review boundary. The change
adds only DoH ALPN dispatch, low-level Hyper HTTP/2 ownership, lifecycle
liveness retention, response HTTP-version metadata, and loopback contract tests.
It does not add pooling/reuse, resolver/bootstrap, socket policy, HTTP/3,
listeners, host composition, YAML/API/WebUI, Go/cgo/selector/fallback, or
production wiring.

Produced behavior and contracts:

- `rust/upstream-core/src/secure/doh.rs` offers `h2,http/1.1` in order;
  negotiated `h2` uses `hyper::client::conn::http2`, negotiated or absent
  HTTP/1.1 keeps the existing inline driver, and an unrecognized ALPN is a
  terminal typed pre-request error. Each exchange remains one fresh numeric
  connection and one GET; no independent caller queries share a connection.
- HTTP/2 response validation now produces an uncommitted candidate. The tracked
  executor scope seals and drains before `BeforeCommit` and the single final
  lifecycle commit, so owner close/caller cancellation/deadline during teardown
  cannot turn into a late success; HTTP/1.1 keeps its inline commit path.
- `TrackedH2Executor` accounts for every Hyper future submitted through the
  caller-supplied executor. Registration happens before `tokio::spawn`, a
  sealed scope drops later submissions, and abort handles plus a Notify drain
  every child. The shared `Lifecycle` hold is retained by the executor state
  and its child tasks, so dropping the caller future cannot release owner
  liveness before those tasks are dropped.
- `SecureHttpVersion` reports HTTP/1.1 or HTTP/2 on DoH responses while DoT
  reports no HTTP version. HTTP status/MIME/body validation and DNS ID restore
  are shared by both protocols; h2 reset, GOAWAY and EOF failures remain
  terminal `MaybeSent` observations with no retry or protocol fallback.

RED/GREEN and drain evidence:

- `negotiated_h2_serves_one_doh_get_with_service_authority_and_path` first
  failed at the TLS ALPN boundary when the client offered only HTTP/1.1, then
  passed after dispatch was added. It verifies the service authority/path,
  original caller ID, and `SecureHttpVersion::Http2` on the owned response.
- `independent_h2_owners_use_independent_fresh_connections` runs two owners
  concurrently against separate TLS+h2 loopback peers. The reset/GOAWAY/EOF
  matrix is terminal and `h2_reset_goaway_and_eof_are_terminal_maybe_sent_failures`
  asserts `MaybeSent`; its listener counts both accepted TCP connections and
  accepted h2 application streams, requiring exactly one of each after
  REFUSED_STREAM and a non-REFUSED RST_STREAM, proving no hidden retry/fallback
  on the same stream or a second connection.
- `h2_executor_seals_admission_and_drains_registered_children` parks a tracked
  child, proves active accounting, seals admission, aborts/drains it, rejects a
  post-seal submission, and proves the shared lifecycle registration reaches
  zero. The two real h2 hang tests cancel or abort immediately after request
  handoff; `close().await` returns `Closed` only after `in_flight_exchanges()`
  is zero. `h2_teardown_barrier_preserves_liveness_until_scope_release` proves
  owner drain stays pending while the shared h2 liveness hold is retained, and
  `h2_validated_response_cannot_commit_until_teardown_releases` proves the
  validated candidate cannot commit before the parked teardown; owner close
  then wins. The real h2 handoff tests cover caller cancellation, dropped
  future, and owner close after handoff.
- The existing deterministic DoH phase matrix remains green across its six
  pre-result phases and four controls; the h2 handoff tests additionally cover
  caller cancellation, owner close/drop, driver teardown, and final resource
  drain without sleeps.

Focused verification (Darwin arm64, cargo/rustc 1.95.0; Rust 1.85.0 is not
installed here):

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice3_doh --locked` | PASS, 6 tests |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core 'secure::doh::tests::h2_' --locked` | PASS, 3 tests |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 225 tests across 9 targets |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, 431 tests across 24 targets |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS |

### Slice3 root-review remediation — 2026-09-17

The first Slice3 root review of `b4aff4de1aef917d0ff69f18d1bf793b43523b8b`
returned `BLOCKED / FAIL`, P0=0 and two scoped P1 findings. The review confirmed
ALPN dispatch, Hyper child interception, numeric/service identity separation and
the basic ownership direction; it blocked only on final-commit ordering and
structured-shutdown evidence.

- P1-1 is addressed by separating `ValidatedDohResponse` from
  `commit_doh_response`. HTTP/2 calls `finalize_h2_response`, which seals and
  drains the tracked scope before `BeforeCommit` and the lifecycle gate. A
  deterministic teardown pause proves owner close wins while the candidate is
  parked; no second commit or post-hoc repair is used.
- P1-2 is addressed with a real h2 owner-close-after-handoff test, h2 failure
  servers that count both TCP connections and application streams, and a
  non-REFUSED `RST_STREAM(CANCEL)` case. The executor tests now include a
  parked teardown/liveness barrier and a candidate-before-commit ordering test.

The scoped remediation was independently root-reviewed against GitHub commit
`d3566bf105008e23c315536d6560b00d55250e55`. The final gate returned
`PASS / Slice3 CLOSED` with P0=0 and P1=0. The review confirmed that the h2
teardown barrier precedes the only final commit, owner/caller/deadline controls
cannot become late success during teardown, and the structured h2 failure
evidence proves exactly one application stream and one TCP connection. The
remediation stays within DoH HTTP/2 ownership/tests and the corresponding
quality/task evidence only.

This closes Slice3 only; Slice4 is not automatically authorized. The final
full-workspace/release and Linux evidence checks remain later Slice4 work.

This record is not a production, Linux, host E2E, throughput, or deployment
claim.

## Slice4 — final quality and isolated Linux evidence

- [x] Re-inspect every PRD AC against tests and every matrix/deferred item.
  Evidence: the AC1-AC8 re-inspection in the Slice4 local record below.
- [x] Run full required checks once at final boundary; additional runs only for
  changes/failures. Use existing loopback CI structure; ordinary rust-foundation
  must actually run secure tests and clippy, not only dns/upstream legacy targets.
  Evidence: rows 1-13 of the local record; the CI audit found the job already
  ran the secure suites and needed only `--all-features` added.
- [x] Record exact source revision, toolchain, OS/architecture, commands/results,
  test counts, dependency features/MSRV/licenses and resource-drain evidence.
  Evidence: the Slice4 local record below.
- [x] Inspect exact changed paths; retain no Go/cgo/ABI/selector/host/listener/
  config/API/WebUI changes or copied KixDNS source.
  Evidence: the scope check in the Slice4 local record.
- [x] No public upstream or production host is needed. Linux Actions/isolated
  test evidence is distinct from macOS results; no claim that a library test is
  full native host E2E, production throughput or long-running deployment proof.
  Evidence: the Linux Actions run `35180813379` for
  `c84d268c66b20ea6e339b4379674ccfb262211bf` succeeded; see "Linux Actions
  evidence" below. Still a library/loopback gate, not host E2E or deployment
  proof.
- [ ] Final root acceptance, then STOP. Archive only after a later wrap-up
  instruction; no implied next protocol/task activation.
  NOT done: the Linux evidence above is complete, but the final root acceptance
  from `rust0916` has not been returned. Awaiting that explicit review result.

Expected checks after implementation is separately authorized:

```sh
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo build --manifest-path rust/Cargo.toml --workspace --release --locked
cargo tree --manifest-path rust/Cargo.toml -p mosdns-upstream-core -e features --locked
cargo +1.85.0 check --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
go test ./...
go vet ./...
go build ./...
python3 .trellis/scripts/task.py validate rust-phase4-secure-upstream-foundation
git diff --check
```

The MSRV toolchain command is a future requirement, not evidence that 1.85.0 is
installed here. Record and resolve graph/toolchain failures before accepting
Slice0; changes to promised MSRV or library family need review. When testing a
binary with embedded UI, use repository UI-first build scripts and never build
frontend and Go concurrently.

### Slice4 local execution and evidence record — 2026-09-17

Scope: this record covers the macOS local half of the Slice4 gate; the Linux
GitHub Actions half is recorded separately under "Linux Actions evidence" below.
Both halves are now complete; only the final root acceptance remains.

Revision and environment actually used:

- branch `rust`; reviewed source revision
  `c84d268c66b20ea6e339b4379674ccfb262211bf`, which contains the Slice3 CLOSED
  record `d3566bf105008e23c315536d6560b00d55250e55` as an ancestor.
- The macOS local checks below were run on the working tree that became
  `c84d268`; the Linux checks were run by GitHub Actions on that exact commit.
- OS/arch: Darwin 25.5.0 arm64 (Apple silicon).
- Toolchain: cargo 1.95.0 / rustc 1.95.0 (Homebrew), the only installed
  toolchains are `stable-aarch64-apple-darwin` and `nightly-aarch64-apple-darwin`.
- **Rust 1.85.0 is NOT installed**, so `cargo +1.85.0 check` could not be run.
  This is recorded as unavailable rather than reported as a pass. MSRV evidence
  is therefore indirect: a `cargo metadata --locked` audit of the full resolved
  graph reports **no package whose `rust-version` exceeds 1.85** (142 packages;
  resolver 3 selected the MSRV-compatible graph recorded in
  `research/secure-upstream-evidence.md`).

Commands and results (all run once at this boundary, in this order):

| # | Command | Result |
| --- | --- | --- |
| 1 | `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| 2 | `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 225 tests across 9 targets |
| 3 | `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, 431 tests across 24 targets |
| 4 | `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS, no warnings |
| 5 | `cargo build --manifest-path rust/Cargo.toml --workspace --release --locked` | PASS |
| 6 | `cargo tree --manifest-path rust/Cargo.toml -p mosdns-upstream-core -e features --locked` | PASS (inspected; no new dependency) |
| 7 | `cargo metadata --manifest-path rust/Cargo.toml --locked` MSRV audit | PASS, no package above 1.85 |
| 8 | `cargo +1.85.0 check ...` | **NOT RUN — toolchain 1.85.0 not installed** |
| 9 | `go build ./...` | PASS |
| 10 | `go vet ./...` | PASS |
| 11 | `go test ./...` | PASS, 32 packages ok, 0 failures |
| 12 | `python3 .trellis/scripts/task.py validate rust-phase4-secure-upstream-foundation` | PASS |
| 13 | `git diff --check` | PASS |

upstream-core per-target counts behind row 2 (9 targets): 55 lib, 12
`slice0_contract`, 27 `slice0_secure`, 27 `slice1_dot`, 35 `slice1_udp`, 39
`slice2_doh`, 14 `slice2_tcp`, 6 `slice3_doh`, 10 `slice3_policy`.

No Rust feature is declared by any workspace member, so `--all-features` selects
the same graph as the default set today; it is still specified explicitly so the
gate cannot silently stop covering a feature that is added later.

Dependency review (unchanged by Slice4): `mosdns-upstream-core` depends only on
`mosdns-dns-core`, `tokio`, `tokio-util`, `url`, `base64`, `rustls`,
`tokio-rustls`, `hyper`, `hyper-util` and `http-body-util`, plus the test-only
`rcgen` dev-dependency, and keeps `#![forbid(unsafe_code)]`. The exact resolved
versions, features, licenses and MSRV ledger, including the correction that
`rcgen`'s default-feature trimming does **not** remove its optional
`x509-parser` dependency (lockfile-only, not activated), is recorded in
`research/secure-upstream-evidence.md`.

Resource-drain evidence: the secure suites assert `in_flight_exchanges() == 0`
after every terminal path. Counted as zero-value assertions: 18 in
`slice1_dot`, 10 in `slice2_doh`, 7 in `slice3_doh`, 9 in `slice3_policy`, 13 in
`slice1_udp` and 5 in `slice2_tcp`, plus in-crate assertions in `secure/doh.rs`
and `secure/dot.rs`; the remaining occurrences in those files assert the
non-zero in-flight state that proves a registration is actually held while the
exchange is parked. `close().await` is asserted to return `Closed` only after
that count reaches zero. Slice3 additionally proves the HTTP/2 tracked executor
seals admission and drains every child, that a dropped caller future cannot
release owner liveness before its children are dropped, and that a validated
response cannot commit while teardown is parked.

CI audit and the one Slice4 change: the ordinary `rust-foundation` job already
ran the secure suites, clippy and fmt on Linux via
`cargo test -p mosdns-dns-core -p mosdns-upstream-core --all-targets --locked`
(13 test executables, verified locally), so it was not rewritten. The only
change is adding `--all-features` to that job's `cargo test` and `cargo clippy`
commands, closing the one gap against the Slice4 requirement. Both edited
commands were re-run locally and pass (278 tests; clippy clean). No other
workflow content was touched, and the `rust-runtime-experimental` job remains
`workflow_dispatch`-only.

AC coverage re-inspection (PRD AC1–AC8):

- AC1 / R1: `DotEndpoint`/`DohEndpoint` numeric dial versus service identity —
  `slice0_secure.rs` construction cases, `slice1_dot.rs` and `slice2_doh.rs`
  authority/identity separation tests, `slice3_doh.rs` service authority/path.
  Covered.
- AC2 / R2: trusted success, wrong name, expired, unknown issuer, bad handshake
  signature, explicit insecure mode, and handshake stalls under
  deadline/cancel/close — `slice1_dot.rs` (synthetic in-memory fixtures).
  Covered.
- AC3 / R3: DoT framing, partial reads/writes, flush, zero/oversize length, EOF,
  wrong ID, malformed and full-TC response, unchanged query bytes —
  `slice1_dot.rs` plus the in-crate DoT phase matrix. Covered.
- AC4 / R4: HTTP/1.1 and h2 success against isolated loopback servers, server
  observed GET encoding/authority and zero outbound ID, original ID restored,
  status/MIME/oversize/truncated body/URL query/ALPN cases —
  `slice2_doh.rs` and `slice3_doh.rs`. Covered.
- AC5 / R5: cancellation and close at each I/O phase and immediately before
  final commit; registrations, sockets and driver/executor tasks drain on
  normal/error/drop/abort paths — the 6-phase x 4-control deterministic DoH
  matrix, the 6-phase x 4-control DoT matrix, and the Slice3 teardown barrier,
  liveness-retention and candidate-before-commit tests. Covered.
- AC6 / R6: fault injection proves side-effect classification and exactly one
  application request; redirects, disconnects and h2 stream errors never trigger
  hidden retry, plaintext fallback or a second connection — request counters in
  `slice2_doh.rs`/`slice3_doh.rs` and the per-variant
  `DohProtocolError::side_effect` tests. Covered.
- AC7 / R7: Rust fmt/test/clippy/release and dependency review pass; existing
  transport regressions and the Go gates pass (rows 1–13 above). The Linux half
  is covered by Actions run `35180813379` on
  `c84d268c66b20ea6e339b4379674ccfb262211bf` (`rust-foundation` and Go `build`
  jobs both SUCCESS; `rust-runtime-experimental` skipped by design). Covered.
- AC8 / R1,R7: no Go/cgo/ABI/selector, host/listener, YAML/API/WebUI,
  deployment or metrics-schema change; the only non-Rust edits are this task's
  evidence documents and the two CI flags. Covered by the scope check below.

Scope check: `git status` shows only `.github/workflows/test.yml`, this task's
`task.json` and `implement.md`, and `docs/ai/rust-handover.md`. No Rust runtime,
Go, config, API/UI, dependency or later-scope file changed. The three untracked
`.DS_Store` files are local tool state and are excluded from any commit.

Explicit limitations of this record:

- The table above is **macOS arm64 local evidence only**; it is not Linux
  evidence. The separate Linux evidence is recorded in the next section.
- All of it is library-level testing against loopback peers. It is **not** native
  host end-to-end evidence, production throughput, or long-running deployment
  proof, on either platform.
- `cargo +1.85.0 check` did not run on macOS (toolchain absent); MSRV is
  evidenced by resolved metadata only, plus the Linux CI build on stable.

### Linux Actions evidence — 2026-09-17

Slice4's Linux half was obtained from GitHub Actions on the reviewed revision.

- Revision: `c84d268c66b20ea6e339b4379674ccfb262211bf`
- Run: `35180813379` — <https://github.com/jasonxtt/mosdns/actions/runs/35180813379>
- Overall workflow "Test mosdns": **SUCCESS**.
- `rust-foundation` job (`105072346672`): **SUCCESS**, 57s, on `ubuntu-latest`.
  It ran `cargo fmt --all --check`, the secure-upstream
  `cargo test -p mosdns-dns-core -p mosdns-upstream-core --all-targets
  --all-features --locked` (all 13 test executables, including every slice0-3
  secure target) and `cargo clippy -p mosdns-dns-core -p mosdns-upstream-core
  --all-targets --all-features --locked -- -D warnings`. This is the Linux
  evidence for the Rust half of AC7.
- Go `build` job (`105072346784`): **SUCCESS** (`go build`, `go vet`,
  `go test ./...` and the focused matcher normal/race suites).
- `rust-runtime-experimental`: **skipped by design**, because that job is
  `workflow_dispatch`-only and this run was push-triggered. Its absence is
  expected and is not a failure; the transitional cgo/ABI/selector path is out
  of Slice4 scope.

Boundaries kept: this run proves the pure Rust foundation builds, tests and lints
on Linux, and that the Go default gates pass. It does **not** claim native host
end-to-end behavior, production throughput, deployment, or any host/production
wiring, and it does not exercise the `workflow_dispatch`-only experimental path.

Remaining: final root acceptance

Both halves of the Slice4 gate (macOS local and Linux Actions) are now complete
for `c84d268c66b20ea6e339b4379674ccfb262211bf`. The only outstanding item is the
final root acceptance result from `rust0916`. Until that explicit `PASS` is
returned, Slice4 is not closed, the task stays `in_progress`, and no later slice
or production step is authorized.

## Slice handoff and rollback

Each handoff includes revision/diff allowlist, RED/GREEN result, parent checks,
AC/matrix coverage, cancellation/side-effect evidence and deferred scope. An
active/pending review is not PASS. Each accepted slice stops; later slice work
requires explicit authorization, preserving the predecessor task's review model.
Rollback only the selected secure slice and its dependencies, preserve archived
UDP/TCP evidence and unrelated work, never add Go fallback as rollback machinery.

## Planning-only validation record — 2026-09-16

- `task.py validate rust-phase4-secure-upstream-foundation`: PASS; both
  context manifests contain four real source/spec/research entries.
- `task.py validate .trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation`:
  PASS; archival preserves the predecessor's context manifests.
- PRD convergence pass complete: R1–R7 map to AC1–AC8, every deferred feature
  has an owner, and requirements/design/execution checklists are separated.
- Scope inspection: only task/spec/journal and migration documentation changes;
  no Rust/Go/Cargo/CI/config/WebUI file changed. `git diff --check`: PASS.
- Runtime tests above have NOT run for a secure implementation, because none
  exists. Predecessor PASS/CI evidence remains historical and was not rerun.
- The preceding bullets are a historical planning-only snapshot. They predate
  the later user authorization and must not be read as the current task state.
- Current Slice0 implementation evidence is recorded in the dependency/MSRV
  and request-target records below and in the secure contract tests. The
  technical checklist is complete; the task stops at its scoped root review.

## Implementation authorization — 2026-09-16

The user subsequently authorized activation, bounded Slice0 implementation,
commit/push, and formal review through `rust0916`. This supersedes the original
planning-only boundary for Slice0. Slice1+ implementation, production wiring,
deployment, and automatic progression after review remain unauthorized.

### Slice1 root-review remediation — 2026-09-16

The first formal review of remote `d781a5e` returned `BLOCKED / FAIL — Slice1
remains OPEN` with P0=0 and three P1 findings. Only those three were addressed;
no later slice, production wiring, or scope widening is included.

**P1-1 (implementation blocker) — post-commit `Open` assertion removed.**
`exchange_inner` asserted `debug_assert_eq!(prepared.lifecycle.state(),
LifecycleState::Open)` after `commit_final_response` returned `Ok`. A commit
that wins the lifecycle mutex may legally be followed immediately by another
thread's `begin_close()`, which moves the owner to `Closing`; the assertion then
panicked and violated "commit wins first, a later close cannot reverse the
committed response". The assertion is deleted and replaced by a comment stating
why nothing may be asserted about owner state after the commit. TLS/DoT I/O
structure is unchanged.
Deterministic regression: `owner_close_immediately_after_a_winning_commit_still_returns_the_response`
parks the exchange on a new `DotPhase::AfterCommit` seam, calls `begin_close()`
in that window, and asserts the committed response is still returned intact, the
registration is released, `close().await` reaches `Closed`, and nothing panics.
RED evidence: re-adding the assertion makes that test fail with exactly the
review's symptom (`left: Closing, right: Open`).

**P1-2 (test/contract blocker) — the acceptance matrix is now actually
closed, with no sleeps.** The first remediation added a `DotPhase` seam but left
the *claimed* matrix open: the abort/drop cases omitted `BeforeCommit`, the
deadline cases omitted `BeforeConnect` and `BeforeCommit`, `BeforeCommit` had
only an owner-close case, and write/flush were largely exercised through caller
cancellation alone. The claim in this file therefore described coverage the
tests did not have.

The coverage is now a table-driven matrix rather than a set of ad-hoc tests:

- Phases: `BeforeConnect`, `BeforeHandshake`, `BeforeWrite`, `BeforeFlush`,
  `BeforeRead`, `BeforeCommit` — the six pre-result phases.
- Controls at **every** phase: owner close, caller cancellation, the one shared
  absolute deadline, and dropped/aborted future.
- That is 6 x 4 = 24 deterministic cells, driven by the shared
  `run_phase_control_matrix` / `run_phase_control_case` helpers, so no phase and
  no control can be silently dropped: `MATRIX_PHASES` is asserted by
  `the_control_matrix_covers_every_pre_result_phase_exactly_once` to contain
  exactly the six distinct pre-result phases.
- Side-effect layering is asserted per phase and must agree across all four
  controls: `NotSent` before/at connect and handshake, `MaybeSent` at write and
  flush, `Sent` at read and the final commit.
- Each cell first observes the deterministic seam arrival and asserts the
  in-flight registration is live, so the control is provably applied at the
  intended phase and not guessed from elapsed time.
- `AfterCommit` is deliberately **not** in this matrix; it exists only for the
  commit-wins regression (`owner_close_immediately_after_a_winning_commit_still_returns_the_response`)
  plus its pre-commit complement, because a post-commit control is a different
  contract (the response is already committed) rather than a pre-result phase.

Also in the module: no plaintext query may precede the handshake (first-byte
TLS-record assertion) and the handshake authenticates the service identity
rather than the dial address. The two `tokio::time::sleep(150ms)` ordering tests
were replaced by a listener that signals the accepted TCP connection through a
`oneshot` the test awaits, so connect-then-control ordering is proven by the
signal; `git grep sleep` over the Slice1 tests matches only comments explaining
their absence. Server scripts return on client EOF, so no test can hang on
`join()`.

RED evidence for the matrix: deleting the `BeforeRead` marker makes
`control_matrix_at_before_read` fail at `Elapsed`, and weakening the flush
phase's side-effect state to `NotSent` makes `control_matrix_at_before_flush`
fail with `left: Transport(Closed(NotSent)) / right: Transport(Closed(MaybeSent))`.
Both show the matrix observes the real phase and the real state rather than
passing vacuously.

Scope note: the phase seam is `cfg(test)` only in its active form; in a
non-test build the seam is absent and every phase marker is a no-op, so the
production exchange path is unchanged.

**P1-3 (data-constraint blocker) — no private key material in the repository.**
All committed DER constants, including the PKCS#8 private keys (`KEY_GOOD_A`,
`KEY_WRONG_NAME_A`, `KEY_EXPIRED_A`, `KEY_UNKNOWN_ISSUER_B`) and the certificate
constants, were removed. `tests/fixtures/mod.rs` now generates every root, leaf,
and key in memory at test runtime with the test-only `rcgen` dev-dependency, and
`tests/slice1_dot.rs` generates its own identity per test. Coverage is
unchanged: valid, wrong-name, expired, unknown-issuer, untrusted-root positive
control, and bad handshake signature are all still exercised; provenance is
recorded in the module documentation and in
`research/secure-upstream-evidence.md` instead of as committed bytes.
Dependency record (test-only, corrected in the third review):
`rcgen = { version = "=0.14.7", default-features = false, features = ["ring"] }`,
`MIT OR Apache-2.0`, MSRV 1.71. This record has now been corrected twice, and
the distinction that matters is between three layers:

1. **Manifest**: in the published rcgen 0.14.7 manifest, `x509-parser` is
   declared `optional = true`, so Cargo creates an implicit same-named feature
   `x509-parser = ["dep:x509-parser"]` that would activate it — but that feature
   is **not selected** here (only `ring` is). Every *declared* reference to it in
   rcgen's own feature table is a weak reference (`x509-parser?/verify` under
   the `ring` feature, `x509-parser?/verify-aws` under the aws-lc-rs features),
   and a weak reference never activates the dependency by itself. In other
   words: no **enabled** rcgen feature activates `dep:x509-parser`.
2. **Lockfile resolution superset**: `rust/Cargo.lock` lists `x509-parser`
   under rcgen's `dependencies`, which is why a lock-only audit sees it. The
   lock records candidate packages for optional dependencies and is not
   evidence of compilation.
3. **Activated build graph**: with only `ring` enabled, rcgen's real edges are
   `ring`, `rustls-pki-types`, `time` and `yasna`.
   `cargo tree -p rcgen -e features --locked` lists exactly those, and a
   workspace-wide `cargo tree --workspace -e features --locked` contains no
   `x509-parser` line at all.

So the second review's claim that `x509-parser` is a *mandatory* rcgen
dependency and an active member of the dev build graph was **wrong**;
`x509-parser`, `asn1-rs`, `der-parser`, `oid-registry`, `nom`,
`rusticata-macros`, `data-encoding`, `lazy_static`, `displaydoc`,
`num-bigint`, `num-traits` and `thiserror` are **not compiled** under the
selected features. They are retained in `research/secure-upstream-evidence.md`
only as a lockfile-only conservative license/MSRV audit, in case a future
feature change activates them.

The activated graph's exact ledger (versions, licenses, rust-version) is also
in `research/secure-upstream-evidence.md`. Its binding MSRV constraints sit
exactly at the ceiling: `deranged 0.5.8` and `zeroize 1.9.0` declare 1.85,
`time`/`time-core` declare 1.83.0; a `cargo metadata --locked` audit of the
full resolved graph reports no package above the workspace MSRV of 1.85.
Re-audited isolation (wording corrected in the fourth pass): the two packages
are absent from the normal tree for **different** reasons and must not be
described together. `cargo tree -e normal` shows **zero** `rcgen` and **zero**
`x509-parser` entries for both `mosdns-upstream-core` and the whole workspace.
`rcgen` appears only on the **dev edges**, where it is a genuine
dev-dependency; `x509-parser` and its transitives are **lockfile-only** and
absent from the active dev/feature tree too, because they are not activated by
the selected features. No aws-lc-rs, OpenSSL, network, or external `openssl`
dependency is involved. `rust/Cargo.lock` was updated for this test-only
graph.
Residue proof: `git ls-files rust/upstream-core/tests/fixtures/` lists only
`mod.rs`; no `.der`/`.pem`/`.key`/`.crt` file is tracked anywhere; and a
`git grep` for private-key constants finds only prose in documentation comments.

Verification after remediation (macOS Darwin 25.5.0 arm64):

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice1_dot --locked` | PASS, 27 tests |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked` | PASS, 167 tests (incl. 42 lib tests, of which 24 are matrix cells across 6 phase tests) |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked` | PASS, 22 targets ok |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | PASS, no warnings |
| `python3 .trellis/scripts/task.py validate rust-phase4-secure-upstream-foundation` | PASS |
| `git diff --check` | PASS |

Toolchain limitation (unchanged): `cargo +1.85.0 check` still cannot run because
Rust 1.85.0 is not installed (`rustup toolchain list` shows only
`stable-aarch64-apple-darwin` and `nightly-aarch64-apple-darwin`). Actual
toolchain is cargo/rustc 1.95.0; MSRV compatibility for the new test-only
dependency is evidenced by resolver-3 selection plus a `cargo metadata` audit
showing no resolved package declares `rust-version` above 1.85, not by a 1.85
build.
