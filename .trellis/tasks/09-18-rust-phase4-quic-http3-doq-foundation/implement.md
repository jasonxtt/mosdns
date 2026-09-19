# Implementation plan — Rust Phase 4 QUIC/HTTP3/DoQ foundation

Planning status: ready for review; implementation is not authorized until the
final planning summary is explicitly approved and `task.py start` is run.

## Routing and worktree rules

- Executor after activation: user-selected Claude, Herdr pane (to be confirmed
  at dispatch; prior tasks used `w6:p2` — re-confirm, do not assume).
- Reviewer after the verified commit: the user-selected ChatGPT web project
  conversation for MosDNS Phase 4 upstream review (to be confirmed; never
  substitute another conversation silently).
- Current branch: `rust`; base branch: `rust`. Preserve the unrelated dirty
  documents (handover/rewrite-plan/workflow/quality-guidelines edits) and
  `.DS_Store` files already present in the worktree.
- Stage exact task/code paths only. Do not use `git add -A`, reset/rebase,
  force-push, or switch branches. Trellis auto-commit stays disabled.
- No Go/cgo/FFI, YAML/config loader, API/WebUI, host/plugin/sequence wiring,
  production/default selection, installed-service mutation, port 53, QUIC
  pooling/multiplexing/migration/0-RTT/resumption, socket policy, server
  listeners, Go mirror/fallback, `MOSDNS_*_BACKEND` selectors, or deployment.
- Every slice is RED first: write the failing public contract test, record the
  real failure, then the minimum GREEN change, then a bounded refactor.
- Planning-only: no implementation code, no dependency changes, no
  `task.py start` until the reviewer approves this plan.

## Slice 0 — dependency audit and RED endpoint/wire contracts (hard gate)

- Audit the QUIC candidate stack: license compatibility with `GPL-3.0-only`,
  declared + resolved MSRV `<= 1.85` (`cargo metadata` audit, same method as
  the secure task), client-only feature set (no server, no extra runtime, no
  platform trust-store auto-load), TLS-stack alignment (QUIC's rustls version
  must equal workspace `rustls 0.23` per `design.md` §2 item 5), and
  `cargo tree -e normal` review.
- Record exact versions, licenses, MSRVs, and the resolved-graph audit in
  `research/` alongside `quic-doq-doh3-evidence.md`.
- **Gate**: if the audit fails any criterion in `design.md` §2, the task stops
  here with a scoped report; no Slice 1+ work begins.
- If the audit passes, add RED public tests for `DoqEndpoint` construction
  (numeric dial + identity separation, zero-port rejection, no resolution),
  DoH3's reuse of `DohEndpoint`, ALPN singleton constants, and the DoQ
  length-prefix/ID-zero byte shape as pure helpers. STREAM FIN is
  stream-transport behavior, not a byte-helper property: Slice 0 asserts only
  the byte shape, and FIN observability is proven by the Slice 1 loopback
  fixture, not by any pure test.
- Add the minimal endpoint/ALPN/byte-shape code to make only those pure tests
  green (prefix encode via the reused `dns-core` Stream framing helper, not a
  new codec). No socket I/O in this slice.
- Checks: focused upstream-core tests, manifest/tree/metadata inspection, fmt,
  clippy, diff.
- Allowed: `rust/upstream-core/src/quic/**` (new module skeleton only),
  exact Cargo manifests/lock, endpoint/wire tests, this task's evidence.
  Forbidden: socket I/O, handshake code, resolver/reuse/secure modifications.

## Slice 1 — DoQ one-shot exchange over loopback

- RED loopback tests: one fresh-connection DoQ exchange succeeds — service
  identity authenticated, peer wire ID 0 asserted before restore, original ID
  restored at the public boundary, 2-byte prefix framing, request STREAM FIN
  and peer response FIN both observed by the fixture, exactly one response
  (returned as `SecureResponse` with `transport == Doq`, `http_version == None`),
  exactly one accepted connection; handshake failure is `NotSent`; post-write
  failure is terminal with no retry; no fallback to any other transport.
- RED negative tests: missing peer response FIN and a trailing second response
  are both terminal `PROTOCOL_ERROR`, never committed; cancellation actively
  issues receive-side `DOQ_REQUEST_CANCELLED` on the stream.
- Implement `DoqUpstream::exchange` per `design.md` §4 on the caller's
  runtime: numeric QUIC connect → `TlsPolicy`-derived TLS config with ALPN
  exactly `["doq"]` → one bidirectional stream → length-prefixed write with
  zeroed wire ID → FIN → length-prefixed read → assert peer wire ID is 0
  (nonzero is terminal, never committed) → ID restore → dns-core validation
  on the restored copy → `commit_final_response` → close.
- Loopback fixture: audited QUIC stack server side on ephemeral loopback
  ports with explicit handshakes and bounded waits (no wall-sleep ordering,
  no port 53, no installed service).
- Allowed: DoQ exchange module/tests/fixture and necessary exact Tokio test
  features. Forbidden: DoH3 driver, pooling, 0-RTT/resumption, config changes.

## Slice 2 — DoH3 one-shot GET over loopback

- RED loopback tests: one fresh-connection DoH3 GET succeeds — `:authority`/
  path equal `DohEndpoint` accessors, request target byte-equal to
  `get_request_target` output for the same input, request send-side FIN
  observed, response meets the full DoH contract (200 +
  `application/dns-message` + identity encoding + complete bounded body
  ≤ 65535), original ID restored (returned as `SecureResponse` with
  `transport == Doh3`, `http_version == Some(Http3)`), exactly one connection;
  handshake/ALPN failure is `NotSent`; no fallback to DoH/H2/H1.
- RED negative tests: non-200, wrong media type, compressed encoding,
  oversized/incomplete body are all typed protocol errors; driver leaves no
  residue after close/cancel (registration count reaches zero).
- Implement the DoH3 driver per `design.md` §5–§5.1 reusing the existing
  encoder verbatim and boundedly extracting/reusing the existing crate-private
  DoH semantic helpers where the H3 response shape permits; ALPN exactly
  `["h3"]`; one GET shape only; H3 driver follows the H2 child-tracking
  ownership (tracked child, sealed teardown + drain, commit after drain).
- Allowed: DoH3 driver module/tests/fixture. Forbidden: generic H3 client
  surface, H3 server code, encoder changes, pooling.

## Slice 3 — deadline, cancellation, close, and error-code mapping

- RED tests: reuse-hit-free absolute deadline respected across connect/
  handshake/stream phases (no private timeout); caller cancellation and owner
  close keep precedence and typed errors; `close()` drains in-flight
  exchanges and is idempotent; guard/registration accounting reaches zero;
  no late success after terminal control.
- RED tests for the explicit stream-error-code → typed-error mapping
  (NO_ERROR / INTERNAL_ERROR / PROTOCOL_ERROR / REQUEST_CANCELLED at minimum,
  with `0x2` covering nonzero peer ID, missing response FIN, and trailing
  extra responses) and the `NotSent`-vs-`Sent` classification table in
  `design.md` §7.
- Implement the control/error wiring with the existing `Lifecycle`/
  `ExchangeControl` vocabulary; no second gate, no hidden timer.
- Allowed: control/error-mapping code and tests. Forbidden: behavior changes
  to existing transports, new retry/fallback paths.

## Slice 4 — resolver composition entry, quality, Linux evidence, review gate

- Add the read-only resolver composition entry for QUIC (same shape as
  `dot/doh_endpoint`, consuming `PublishedTarget::dial()`); deterministic
  tests prove numeric-dial/identity separation for both A and AAAA selections.
- Run the focused resolver/upstream tests repeatedly enough to expose missed
  wakeups or late publication, using explicit handshakes and bounded waits.
- Run the local gates:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo tree --manifest-path rust/Cargo.toml --workspace -e features --locked
python3 ./.trellis/scripts/task.py validate rust-phase4-quic-http3-doq-foundation
git diff --check
```

- Use the isolated Debian VM through `ssh mosdns-rust` for focused Linux/Rust
  1.85.x loopback evidence. Do not use Mac Docker/Colima and do not mutate the
  installed `mosdns`/`mos-test` services or bind port 53.
- If Rust 1.85 cannot actually be installed/run, record MSRV as indirect
  evidence exactly as the secure task did — do not claim a toolchain run that
  never happened.
- Controller independently verifies the complete task diff, reruns the gates,
  stages exact paths, commits, pushes branch `rust`, and sends one compact
  review request (full commit, diff scope, evidence, status, forbidden
  follow-on scope) to the confirmed reviewer conversation. Then wait/read at
  ~one-minute intervals until explicit PASS/FAIL; a scoped FAIL restarts only
  the bounded remediation loop; a scope-changing FAIL stops for user input.
  PASS closes this task's authorized scope only — never another task, host
  wiring, or production selection.

## Slice-to-acceptance traceability

| Slice | Covers |
|---|---|
| 0 | A1 (audit), A2 (endpoint/ALPN/wire-shape) |
| 1 | A3 (DoQ loopback incl. `SecureResponse` result type), A5 (DoQ errors, no fallback), A6 (DoQ control) |
| 2 | A4 (DoH3 loopback incl. `SecureResponse` result type), A5 (DoH3 errors, no fallback), A6 (DoH3 control) |
| 3 | A5 (code mapping), A6 (deadline/cancel/close) |
| 4 | A2 (resolver composition), A7 (no regression), A8 (gates/Linux/review) |

## Planning review record — 2026-09-18

- Reviewer: user-selected ChatGPT web project conversation for MosDNS Phase 4
  upstream review (new conversation for this QUIC task).
- Round 1: `FINAL: FAIL` on `af8a4cb` — 3 planning P1s (DoQ completion/
  cancellation contract; DoH3 HTTP semantics + H3 driver ownership; public
  result vocabulary unresolved). Remediated in commit `6ca5c4a`.
- Round 2: `FINAL: PASS` on
  `6ca5c4a9d439140bf8255a0c7da7adf9970818ff` (base `af8a4cb`; exactly 1 commit,
  exactly 4 planning files; `rust` HEAD = `6ca5c4a`). P0=0, P1=0. All three
  prior P1s closed; no new P0/P1 introduced.
- Boundary: this PASS closes the planning remediation review only. It does not
  authorize `task.py start`, dependency changes, implementation, Slice 0
  execution, production wiring, or any later task. Next phase needs separate
  explicit user authorization.

## Slice 0 pre-I/O review record — 2026-09-19

- Reviewer: user-selected ChatGPT web project conversation for MosDNS Phase 4
  upstream review (same conversation as the planning rounds).
- Round 1: `FINAL: FAIL` on
  `4a7dfc0aacc88d81b60aa4ec18d0dd12a836ec2d` (base `b75bd7f`; 6 files,
  +288/-0) — P0-1 (QUIC admitted into the plain-TCP reuse owner and executed
  as TCP), P1-1 (DoQ/DoH3 `SecureResponse` shapes not constructible from the
  QUIC module), P2-1 (short-buffer no-op untested), P2-2 (audit upstream SHAs
  unanchored, ruled non-blocking). Remediated in commit `0d2b64e`
  (TCP-only pool gate; crate-internal `doq`/`doh3` seams; 0/1-byte pin tests).
- Round 2: `FINAL: PASS` on
  `0d2b64ed4592597c117f16407accb4c4dfff4825` (base `4a7dfc0`; exactly 1 commit,
  exactly 4 files; `rust` HEAD = `0d2b64e`). P0=0, P1=0, P2=0. Both prior
  blockers closed; short-buffer P2 closed; no new findings introduced.
- Boundary: this PASS closes Slice 0 pre-I/O contracts only, including the
  remediation. It does not authorize `Cargo.toml`/`Cargo.lock` pinning, locked
  dependency-audit completion, Slice 1 socket/handshake code, production
  wiring, or any other task. The task stays `in_progress`; the Slice 0 exit
  gate (locked manifest/tree/MSRV audit) and every later slice each need
  separate explicit user authorization.

## Slice 0 exit-gate review record — 2026-09-19

- Reviewer: user-selected ChatGPT web project conversation for MosDNS Phase 4
  upstream review (same conversation as the pre-I/O rounds).
- Round 1: `FINAL: FAIL` on
  `111687b8050fb7abbecf005a3e5dca15cc885252` (base `d65f497`; 3 files,
  +294/-1) — P1-1 (`futures-executor 0.3.34` in the lock undispositioned,
  violates design §2 "no extra async runtimes" as written), P2-1 (false
  "stay at default features" / `futures-io`-as-adapter wording), P2-2
  ("pure-additive" wording vs one edited pre-existing lock line).
  Remediated in commit `a9b0cc7` (source-backed `futures-executor`
  disposition: single path via h3-quinn's un-narrowed `futures` dep,
  `std`-only executor surface, `thread-pool` absent with zero feature-tree
  lines, zero executor refs in h3-quinn/h3 sources, quinn `block_on`
  tokio test-only; corrected feature wording; socket2 0.6.5
  disambiguation note; `Cargo.lock` untouched).
- Round 2: `FINAL: FAIL` on
  `a9b0cc7bc943a3179693afa73947cee1581df94e` (base `111687b`; exactly 1
  commit, exactly 2 files, +57/-13). P0=0, P1=0 — P1-1 substantively
  remediated, P2-2 closed (sole removed lock line is `- "socket2",`,
  no version line removed). Two P2 residuals: (a) item 4 "activated quinn
  features are exactly runtime-tokio + rustls-ring" still false — h3-quinn
  0.0.10 requests quinn with `features = ["futures-io"]`, so unification
  also activates quinn's `futures-io`; (b) "The single `runtime.block_on`
  hit" understates — exact quinn 0.11.7 has five
  (`src/tests.rs:55,155,177,563,596`), all tokio test-only.
  Remediated in commit `75dcb74` (resolved set =
  runtime-tokio + rustls-ring + futures-io, with `futures-io` characterized
  as h3-quinn-requested poll adapters on quinn stream types
  `recv_stream.rs:476-477` / `send_stream.rs:248-249`; "five hits" with
  exact lines; `Cargo.toml` comment distinguishes manifest request from
  resolved set; `Cargo.lock` untouched).
- Round 3: `FINAL: PASS` on
  `75dcb7415a68dc006cef21e61d11d2236d134bed` (base `a9b0cc7`; exactly 1
  commit, exactly 2 files, +20/-9; `Cargo.lock` blob SHA identical before
  and after). P0=0, P1=0, P2=0. Reviewer verified against exact upstream
  tags (h3-quinn `2dc3412`, quinn `d8302df`): `futures-io` gates only the
  two adapter impls; all five `block_on`s are tokio test-only under
  `#[cfg(test)] mod tests`; scope check passes.
- Boundary: this PASS closes the Slice 0 exit-gate remediation only. It
  does not authorize Slice 1 socket/handshake I/O, production wiring, or
  any other task. The task stays `in_progress`; Slice 1 and every later
  slice each need separate explicit user authorization.

## Slice 1 implementation record — 2026-09-19

- Executor: DSH Web in the Chrome workspace session, with one whole-phase
  remediation task. MCP DSH was not used for this remediation; the controller
  reviewed the resulting allowlisted diff in the shared `rust` worktree.
- Implementation commits:
  - `af017b2` — DoQ one-shot loopback exchange with numeric dial/identity
    separation, exact `doq` ALPN, one bidirectional stream, two-byte
    big-endian framing, wire ID zeroing/restoration, response validation,
    commit gate, and `SecureResponse::doq`.
  - `93f8d72` — reject trailing bytes/second response without commit.
  - `a86d04d` — reject response streams that do not complete with normal FIN.
  - `e57e640` — caller/local control cancellation actively sends
    `STOP_SENDING(DOQ_REQUEST_CANCELLED=0x3)` while preserving the existing
    typed error and commit semantics.
- Focused evidence before remediation was 4 passed. The review found that the
  uncommitted `connection.close`/`endpoint.close`/`wait_idle` experiment could
  drop `STOP_SENDING`, so the remediation first reproduced RED with both
  cancellation tests observing no stop code. The web DSH then replaced that
  pseudo-flush with a shared post-`open_bi` `RecvStream::stop(0x3)` path and a
  debug-only `DoqStopPause` observation seam; it also mapped
  `ReadError::ConnectionLost(_)` to the existing missing-response-FIN error.
- Remediation evidence: `cargo test --manifest-path rust/Cargo.toml -p
  mosdns-upstream-core --test slice1_doq --locked` — 7 passed; the two
  cancellation tests passed 16/16 in an 8-round stress run. The added
  full-frame-payload and partial-frame-payload connection-loss fixtures both
  verify missing-FIN, `Sent`, no commit, and zero in-flight exchanges; they
  intentionally do not claim that the payload reached the client before
  connection close. Release validation compiled the seam out (0 release
  symbols, 16 debug symbols) and the release Slice 1 target passed 5 tests
  with zero warnings.
- Parent verification also passed: focused Slice 1, upstream-core all-target
  tests (423 tests), workspace all-target tests, format check, upstream-core
  and workspace warnings-denied clippy, task validation, and `git diff --check`.
- Scope boundary: DoH3, production host wiring, pooling/retry/fallback,
  Linux/VM deployment evidence, and the later Slice 2–4 work remain
  unauthorized and untouched. The remediation is committed as `62e8f1f`,
  pushed to `origin/rust`, and passed the same-conversation GPT web root review
  with `FINAL: PASS`; the task remains `in_progress` at the Slice 1 boundary.

## Slice 2 implementation record — 2026-09-19

- Executor: DSH Web in the Chrome workspace session, single executor with no
  sub-task split, no second session, and no MCP. The parent controller retains
  diff inspection, the quality gates, and the external review round.
- Scope: Slice 2 only — the one-shot DoH3 driver, its loopback integration
  fixture/tests, and the crate-internal module exports it needs. Slice 3
  (deadline/cancel/close precedence and the full QUIC/H3 stream-error-code
  mapping) is **not started**.

### RED baseline (before any Slice 2 implementation)

`cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test
slice2_doh3 --locked` failed to compile with

```text
error[E0432]: unresolved import `mosdns_upstream_core::quic::Doh3Upstream`
  --> upstream-core/tests/slice2_doh3.rs:48:34
```

which is the required proof that the DoH3 one-shot driver and its public
capability were absent before the slice.

### Implementation

- `rust/upstream-core/src/quic.rs` — new Slice 2 section with `Doh3Upstream`:
  - fresh numeric QUIC connect on the caller's runtime using
    `TlsPolicy::client_config_with_alpn(&[H3_ALPN])`; the service identity, not
    the dial address, selects the TLS name;
  - the request target is the reused `DohEndpoint::get_request_target` output
    and `:authority` is `DohEndpoint::authority()` (no second `dns` encoder and
    no `Host` header beside the pseudo-header);
  - exactly one `GET` with `Accept: application/dns-message`, no body, no
    `User-Agent`, no `Content-Encoding`, followed by a request send-side FIN;
  - the h3 connection driver is spawned as a tracked child of the exchange
    scope *before* the first request byte; teardown seals admission, aborts the
    child, and drains to guard drop, and `commit_final_response` runs only after
    that drain;
  - response validation reuses the DoH contract (status 200, head ≤ 16 KiB,
    declared length ≤ 65535, identity/absent encoding, case-insensitive
    `application/dns-message` with parameters), adds the 64-header bound the h3
    layer does not impose itself, and reads a complete bounded body (≤ 65535)
    rejecting early EOF / length mismatch as `IncompleteBody`;
  - ID restore plus `dns-core` validation, then `SecureResponse::doh3`
    (`Doh3`/`Some(Http3)`); the one-shot `connection.close` /
    `endpoint.close` / `wait_idle` runs only on the success path after the
    commit, so it is never a cancellation barrier.
- `rust/upstream-core/src/secure/doh.rs` — behavior-preserving bounded
  extraction so H3 cannot drift from H1/H2: `parsed_head_bytes`,
  `validate_doh_head_parts` (now returning the already range-checked declared
  length), crate-visible `MAX_*` bounds and `DNS_MEDIA_TYPE`,
  `restore_request_id`, and an `H2ScopeLease::spawn` tracked-child seam with
  `pub(crate)` `new`/`finish`.
- `rust/upstream-core/src/secure/mod.rs` — crate-internal re-exports of that
  surface; nothing is re-exported from the crate root.
- `rust/upstream-core/src/secure/dot.rs` — drops the now-stale
  `#[allow(dead_code)]` and Slice-0 note from the `SecureResponse::doh3` seam
  that Slice 2 now calls.
- `rust/upstream-core/tests/slice2_doh3.rs` — new loopback integration test
  with a real in-process h3 server on an ephemeral IPv4 loopback port (server
  code confined to the integration test), a trusted synthetic leaf, exact `h3`
  ALPN, and a TCP listener sharing the QUIC port as no-fallback evidence.

No manifest or `Cargo.lock` change, no new dependency, and no debug-only test
seam, so nothing in the new code is release-gated.

### Evidence

- Focused: `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core
  --test slice2_doh3 --locked` → 22 passed / 0 failed; the same target with
  `--release` → 22 passed / 0 failed.
- Upstream-core: `cargo test ... -p mosdns-upstream-core --all-targets
  --all-features --locked` → 443 passed / 0 failed.
- Workspace: `cargo test ... --workspace --all-targets --all-features
  --locked` → all passed.
- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` clean;
  `cargo clippy ... -p mosdns-upstream-core --all-targets --all-features
  --locked -- -D warnings` clean; workspace clippy with `-D warnings` clean;
  `git diff --check` clean.
- Covered behaviors: fresh-connection successful GET with exact
  authority/path/headers and observed request FIN; one request per connection
  and two connections for two exchanges; ALPN, untrusted-certificate, and
  identity-mismatch handshake failures as `NotSent` with no TCP fallback;
  non-200; wrong and missing media type; non-identity content encoding;
  declared-over-maximum and actual-over-maximum bodies; incomplete body;
  response trailers after a complete body, both finished and withheld without a
  stream FIN; too-many-headers; close-before-head as `ResponseHeadNotReceived`
  (`MaybeSent`); outbound ID zeroing and caller ID restoration; and zero
  in-flight registrations after owner close, caller cancellation, a dropped
  exchange future, and an exchange after close.

### Deliberate boundary and known limits

- Slice 3 is unimplemented: no deadline-precedence test, no active h3 stream
  cancellation, and no explicit QUIC/H3 stream-error-code → typed-error matrix.
  A non-`HeaderTooBig` h3 head-phase error is conservatively
  `ResponseHeadNotReceived` (`MaybeSent`) until that mapping lands.
- The > 16 KiB response-head rejection is enforced on the receive path by the
  `max_field_section_size` the client advertises plus h3's QPACK decode bound,
  and again by the shared post-parse reconstruction. The fixture's h3 server
  refuses to emit a head above the client's advertised limit, so the directly
  exercised head-bound negative test is the 64-header case; the byte bound has
  no dedicated over-limit emitted fixture.
- Task status was not changed. Slice 2 is committed as `c5ef3a5` and pushed to
  `origin/rust`. The GPT web root review of `69ec2f6..c5ef3a5` returned
  `FINAL: FAIL` on the single P1-1 blocker: `read_h3_body` treated the first
  `Ok(None)` from `recv_data` as response completion, although in h3 0.0.8 that
  `None` also means "a trailing HEADERS frame was buffered as response
  trailers". The remediation calls `recv_trailers` once on the same
  `race_control` path and the same absolute deadline, accepts only `Ok(None)` as
  completion, maps `Ok(Some(_))` to `DohProtocolError::IncompleteBody` (the
  existing HTTP/1.1 and HTTP/2 semantics), and keeps `Err(_)` on
  `classify_h3_body_error`; the declared `Content-Length`, `MAX_DNS_BODY`,
  empty-body, and commit-gate semantics are unchanged. The remediation is
  committed as `3aec651` and pushed to `origin/rust`; the follow-up GPT web
  root review of `c5ef3a5..3aec651` returned `FINAL: PASS` with P0/P1 both zero.

## Slice 3 implementation record — 2026-09-19

- Executor: DSH Web, single executor with no sub-task split, no second session,
  and no MCP. The parent controller retains diff inspection, the quality gates,
  and the external review round.
- Scope: Slice 3 only — the QUIC deadline/control/close contract, the
  H3/DoQ stream-error-code → typed-error mapping, the `NotSent`/`Sent`
  classification calibration, and their real loopback tests. Slice 4 (resolver
  composition, Linux/MSRV evidence, release/review gate) is **not started**; no
  pooling, retry, fallback, host wiring, or dependency change is included.

### RED baseline (before any Slice 3 implementation)

`cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test
slice3_quic --locked` failed to compile with four errors, proving the Slice 3
typed vocabulary and mapping were absent:

```text
error[E0432]: unresolved import `mosdns_upstream_core::secure::PeerStreamError`
error[E0599]: no variant or associated item named `DoqProtocolNonzeroResponseId`
              found for enum `SecureError`
error[E0599]: no variant named `PeerStreamTerminated` found for enum
              `DohProtocolError` (two call sites)
```

### Implementation

- `rust/upstream-core/src/secure/error.rs`:
  - new `PeerStreamError` closed category (`NoError`, `InternalError`,
    `ProtocolError`, `RequestCancelled`, `Other`). The raw peer code and any
    reason text are never retained, so `Display`/`Debug` cannot leak peer
    material, and `NoError` is documented as *not* benign by itself: a
    termination observed while the response is incomplete can never commit;
  - `DohProtocolError::PeerStreamTerminated { code }` (`Sent`): a peer h3
    stream termination is an explicit per-stream signal, and the request stream
    was already written and finished before the response was awaited;
  - `SecureError::DoqProtocolNonzeroResponseId` (`Sent`): the RFC 9250 §4.2.1
    `PROTOCOL_ERROR` (`0x2`) case of a nonzero peer DoQ wire ID. This replaces
    the previous plain `UpstreamError::ResponseMismatch` mapping, which did not
    carry the DoQ protocol-error semantics required by `design.md` §7 / PRD R9;
    the other two DoQ protocol variants are unchanged, so the Slice 1 contract
    tests still hold.
- `rust/upstream-core/src/secure/mod.rs`: public re-export of `PeerStreamError`.
- `rust/upstream-core/src/quic.rs`:
  - DoQ nonzero peer wire ID now returns `DoqProtocolNonzeroResponseId` instead
    of a generic DNS mismatch; a peer reset with any RFC 9250 §4.3 code
    (`0x0`-`0x3`, including `NO_ERROR`) remains the terminal missing-FIN
    protocol error, because completion requires the response-side FIN and a
    reset never carries one;
  - `classify_h3_head_error` / `classify_h3_body_error` match
    `h3::error::StreamError::RemoteTerminate { code, .. }` **structurally** and
    map the numeric `h3::error::Code` value through `classify_peer_stream_code`
    to the closed category: `0x100`/`0x0` → `NoError`, `0x101`/`0x2` →
    `ProtocolError`, `0x102`/`0x1` → `InternalError`, `0x10c`/`0x3` →
    `RequestCancelled`, anything else → `Other`. No reason string is parsed.
    A peer `STOP_SENDING` on the request *send* side stays a typed
    `Send(MaybeSent)` failure, because the request write is the side still in
    doubt;
  - the DoH3 response-head race was calibrated from `MaybeSent` to `Sent`: the
    request head was written and its send side finished, so a deadline/cancel
    while waiting for the response head is a post-write read failure per
    `design.md` §7. The `ResponseHeadNotReceived` *variant* keeps its
    conservative `MaybeSent` state, so the Slice 2 close-before-head contract is
    unchanged.
- No manifest or `Cargo.lock` change, no new dependency, no production sleep,
  timer, or private timeout; every phase still runs through the existing
  `race_control` / `ExchangeControl::check_at` / `commit_final_response`
  vocabulary.

### Evidence

- RED: the four compile errors above on the new `tests/slice3_quic.rs`.
- Focused: `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core
  --test slice3_quic --locked` → 21 passed / 0 failed; the same target with
  `--release` → 21 passed / 0 failed.
- Regression (focused): `--test slice1_doq --test slice2_doh3 --test slice0_quic
  --test slice0_contract` all passed (DoQ 7, DoH3 22, quic contracts 10, secure
  contracts 12).
- Upstream-core: `cargo test ... -p mosdns-upstream-core --all-targets
  --all-features --locked` → 466 passed / 0 failed across all targets.
- Workspace: `cargo test ... --workspace --all-targets --all-features --locked`
  → all targets passed, 0 failed.
- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` clean;
  `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets
  --all-features --locked -- -D warnings` clean; `git diff --check` clean.
- Covered behaviors:
  - DoQ: deadline at connect/TLS handshake (silent UDP sink) and at stream open
    (zero advertised bidirectional budget) are `NotSent`; deadline while waiting
    for the response and after a complete frame without peer FIN are `Sent` and
    never commit; ALPN mismatch is a typed TLS failure with `NotSent`; a
    nonzero peer wire ID is the typed nonzero-response-ID protocol error; peer
    resets with `0x0`/`0x1`/`0x2`/`0x3` are all terminal missing-FIN errors with
    `Sent`; `close()` drains the in-flight exchange, refuses a new exchange with
    `Closed(NotSent)`, converges on repeat close, and leaves zero registrations;
    owner close beats a simultaneous caller cancellation; owner close → caller
    cancellation → deadline precedence is asserted in all three deterministic
    pre-flight orders.
  - DoH3: deadline at connect/handshake (silent UDP sink) is `NotSent`; a
    blocked h3 request-stream open is `MaybeSent` (h3 0.0.8 fuses stream open and
    request write into one `send_request`); the response wait and the
    complete-body-without-FIN wait are `Sent` and never commit; a trailing
    HEADERS field section after a complete body is `IncompleteBody` (`Sent`);
    peer `RemoteTerminate` codes `0x100`/`0x101`/`0x102`/`0x10c` are
    `PeerStreamTerminated` with the matching category in the head phase and
    `0x10c` again in the body phase, always `Sent` and never committed; a local
    caller cancellation after a complete body returns the typed `Cancelled`
    control error rather than a peer error and commits nothing; `close()` drains,
    refuses, and converges; owner close beats caller cancellation; the three
    precedence orders are asserted.

### Deliberate boundary and known limits

- **Active h3 `H3_REQUEST_CANCELLED` is not sent, and this is an API blocker,
  not an omission.** `h3::client::RequestStream::stop_sending` delegates to
  `h3_quinn::RecvStream::stop_sending`, which does
  `self.stream.as_mut().unwrap().stop(..)`
  (`h3-quinn-0.0.10/src/lib.rs:390-397`); `h3_quinn::RecvStream::poll_data`
  *takes* that `Option` into the in-flight read future and only restores it after
  that future completes (`h3-quinn-0.0.10/src/lib.rs:375-387`); a local control
  decision wins by dropping that read future, leaving the inner stream `None`,
  and an immediate `stop_sending` panics with
  `called Option::unwrap() on a None value` (observed with `RUST_BACKTRACE=1`
  before the call was removed). The local cancellation is therefore expressed by
  the unchanged typed `Closed`/`Cancelled`/`DeadlineExceeded` error (with its own
  `SideEffectState`) plus the connection/endpoint teardown, and a local decision
  is never disguised as a peer error. The boundary and the exact source lines
  are documented on `run_h3_request` in `quic.rs`. DoQ keeps its Slice 1 active
  `STOP_SENDING(DOQ_REQUEST_CANCELLED)`, which is reachable through quinn's own
  `RecvStream::stop`.
- The DoH3 missing-FIN case is exercised with a held (never-finished) complete
  body terminated by the deadline/cancellation. A fixture that closes the
  connection immediately after the body races the body delivery and is
  non-deterministic, so it is not used; the held-body tests prove the same
  contract (a complete-looking body without FIN never commits).
- Task status was not changed. Slice 3 is left uncommitted in the Slice 3
  worktree (`8ffd519` base) for the parent to inspect, commit, and push; the
  unrelated dirty `.trellis/spec/...`, `.trellis/workflow.md`, `.DS_Store`, and
  `09-19-ci-rust-foundation-lint-doc-path-filter` files were preserved.

## Slice 3 remediation record — 2026-09-20

- Executor: DSH Web, single executor with no sub-task split, no second session,
  and no MCP. The parent controller retains diff inspection, the quality gates,
  and the external review round.
- Context: the original Slice 3 (`07c2a2f`, "feat(rust): complete Slice 3 QUIC
  control mapping") was committed and pushed by the parent flow on top of
  `8ffd519`. GPT Web's formal review of `8ffd519..07c2a2f` returned
  `FINAL: FAIL`, `P0=0`, `P1=2`, `P2=1`. This record covers only the two P1
  remediations; Slice 4 is not started, and no resolver/pool/retry/fallback/host
  wiring, Cargo manifest/`Cargo.lock`, or other transport is touched.
- Worktree state: this remediation is left **uncommitted and unpushed** in the
  current worktree for the parent to inspect and commit. `task.json` stays
  `in_progress`. Nothing here claims a commit or push.

### P1-1 — a completed-request-FIN DoH3 head failure is `Sent`

- RED (before the fix): the corrected/new loopback tests failed against
  `07c2a2f` production code:
  `slice3_quic::doh3_response_head_connection_loss_after_request_fin_is_sent_and_never_commits`
  and `slice2_doh3::doh3_close_after_the_request_fin_before_any_response_head_is_sent`
  both reported `left: DohProtocol(ResponseHeadNotReceived)`,
  `right: Transport(Receive(Sent))`. Both failures landed in
  `classify_h3_head_error`'s `_` arm, proving the ordinary connection-loss path
  was reaching the `MaybeSent` `ResponseHeadNotReceived` variant after the
  request send side had finished.
- Fix: `classify_h3_head_error`'s catch-all arm now returns
  `SecureError::Transport(UpstreamError::Receive(SideEffectState::Sent))` - the
  existing typed vocabulary, exactly what the DoQ read path already uses for an
  ordinary post-write read failure. `HeaderTooBig` and `RemoteTerminate` keep
  their existing mappings. The change is scoped to the H3 response-head phase:
  `DohProtocolError::ResponseHeadNotReceived` and its `MaybeSent` state are
  untouched and remain the HTTP/1.1/HTTP/2 head-not-received case (their request
  hand-off and head wait are fused, so delivery is genuinely in doubt). The
  variant doc now records that DoH3 does not produce it.
- `Slice2` close-before-head contract: the DoH3 case in `slice2_doh3.rs` is the
  exact post-request-FIN head phase - the fixture reads the request through its
  send-side FIN and only then closes - so its expectation is corrected from
  `ResponseHeadNotReceived`/`MaybeSent` to a `Sent` receive failure and renamed
  to `doh3_close_after_the_request_fin_before_any_response_head_is_sent`. The
  contract that an absent head is a typed terminal failure, never a false
  success, and leaves no residue is preserved.
- New evidence: `slice3_quic`'s `H3Mode::CloseAfterRequest` server reads the
  request FIN and closes the real QUIC connection without any response head; the
  test asserts `Transport(Receive(Sent))`, `Sent`, one accepted connection,
  zero in-flight residue, and an observed request FIN.

### P1-2 — the DoH3 peer code mapping uses only the HTTP/3 code space

- RED (before the fix): the new wire-level reset test failed on the DoQ-shaped
  low codes, e.g. `0x1` → `left: PeerStreamTerminated { code: InternalError }`,
  `right: PeerStreamTerminated { code: NoError }`.
- Fix: `classify_peer_stream_code` no longer aliases the DoQ low codes onto H3
  categories. It maps only the RFC 9114 §8.1 HTTP/3 codes
  `H3_NO_ERROR (0x100)` → `NoError`, `H3_GENERAL_PROTOCOL_ERROR (0x101)` →
  `ProtocolError`, `H3_INTERNAL_ERROR (0x102)` → `InternalError`,
  `H3_REQUEST_CANCELLED (0x10c)` → `RequestCancelled`. Another code *defined* by
  RFC 9114 §8.1 / RFC 9204 (for example `0x103`, `0x200`) is the unclassified
  `Other`. Every other value - the RFC 9000 §20.1 transport code space below
  `0x100`, the reserved `0x1f * N + 0x21` grease space, and any unknown code - is
  treated as equivalent to `H3_NO_ERROR` per RFC 9114 §8 and maps to `NoError`,
  so a DoQ `0x2` reset is never reported as an H3 protocol error.
  `PeerStreamError` docs, `design.md` §7, and PRD R9 were updated to match, and
  state the RFC unknown/unexpected rule explicitly.
- New evidence: `slice3_quic`'s
  `doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized` drives
  real wire-level H3 stream resets for `0x0`/`0x1`/`0x2`/`0x3` and the reserved
  grease code `0x119` (all `NoError`, per RFC 9114 §8) and for `0x103`/`0x200`
  (both `Other`), asserting `Sent`, no commit, and exactly one accepted
  connection per case. DoQ's own `0x0`-`0x3` reset semantics are unchanged and
  still covered by
  `doq_peer_reset_codes_are_terminal_missing_fin_without_commit`.
- Boundary preserved: active h3 `H3_REQUEST_CANCELLED` is still **not**
  attempted (h3-quinn 0.0.10 `stop_sending` panics after a dropped read future);
  local owner close / caller cancel / deadline still return their typed control
  errors unchanged and retain their precedence; DoQ keeps its Slice 1 active
  `STOP_SENDING(DOQ_REQUEST_CANCELLED)`.
- Unrelated dirty files (`.trellis/spec/...`, `.trellis/workflow.md`,
  `.DS_Store`, `09-19-ci-rust-foundation-lint-doc-path-filter`) were preserved;
  no `git reset`/`checkout`/`clean`, no `git add -A`, no commit, no push.

### Gates (final worktree)

- Focused debug `slice0_quic`/`slice0_contract`/`slice1_doq`/`slice2_doh3`/
  `slice3_quic`/`slice2_doh`/`slice3_doh`: all passed, 0 failed (`slice3_quic`
  23/23, `slice2_doh3` 22/22); the same focused set under `--release`: all
  passed, 0 failed.
- `cargo test -p mosdns-upstream-core --all-targets --all-features --locked`:
  0 failed; `cargo test --workspace --all-targets --all-features --locked`:
  **720 passed / 0 failed** across 39 test binaries.
- `cargo fmt --all -- --check`: clean. `cargo clippy --workspace --all-targets
  --all-features --locked -- -D warnings`: clean. `git diff --check`: clean.
- `python3 ./.trellis/scripts/task.py validate
  rust-phase4-quic-http3-doq-foundation`: `All validations passed` (the
  `rust-migration.md` size warning is pre-existing and informational).
- No production `sleep`, private timer, or hidden timeout was added; the only
  waits in the new tests are the existing fixture's bounded `TEST_TIMEOUT` and
  explicit `oneshot`/connection signals.

## Slice 3 remediation record 2 — 2026-09-20 (DoH3 QPACK context codes, review P1)

- Executor: DSH Web, single executor with no sub-task split, no second session,
  and no MCP. The parent controller retains diff inspection, the quality gates,
  and the external review round.
- Context: the committed Slice 3 remediation (`cc11986`, "fix(rust): correct
  Slice 3 QUIC error states") was reviewed by GPT Web over
  `07c2a2f..cc11986` as `FINAL: FAIL`, `P0=0`, `P1=1`, `P2=1`. The single P1 was
  the DoH3 HTTP/3 error-code context mapping in
  `rust/upstream-core/src/quic.rs`: `classify_peer_stream_code()` /
  `is_defined_h3_code()` mapped every defined `0x103..=0x110 | 0x200..=0x202`
  code to `PeerStreamError::Other`, but RFC 9114 §8 requires an error code used
  in an unexpected context to be treated as equivalent to `H3_NO_ERROR`. This
  record covers only that P1. Slice 4 is not started, and no
  resolver/pool/retry/fallback/host wiring, Cargo manifest/`Cargo.lock`, or other
  transport is touched.
- Worktree state: this remediation is left **uncommitted and unpushed** in the
  current worktree (`cc11986` HEAD) for the parent to inspect and commit.
  `task.json` stays `in_progress`. Nothing here claims a commit or push.

### P1 — DoH3 QPACK encoder/decoder stream codes are an unexpected context

- Problem: a `RemoteTerminate` can only arrive on the DoH3 *request/response*
  stream, yet the classifier treated every RFC 9204 code as the unclassified
  `Other`. RFC 9204 §6 defines `QPACK_ENCODER_STREAM_ERROR (0x201)` and
  `QPACK_DECODER_STREAM_ERROR (0x202)` only for the QPACK encoder and decoder
  streams, so on a request/response stream they are an error code in an
  unexpected context. RFC 9114 §8: "use of an error code in an unexpected
  context or receipt of an unknown error code MUST be treated as equivalent to
  H3_NO_ERROR." `QPACK_DECOMPRESSION_FAILED (0x200)` is different: RFC 9204 §6
  defines it precisely for a failed field-section decode on a request stream, so
  it remains a known HTTP/3-family error in this context.
- RED (before the fix): `slice3_quic`'s real-wire loopback reset test was
  extended with `0x201` and `0x202` (expecting `NoError`) and run against the
  unchanged `cc11986` production code:
  `left: DohProtocol(PeerStreamTerminated { code: Other })`,
  `right: DohProtocol(PeerStreamTerminated { code: NoError })` on `0x201`. The
  failure landed in the `is_defined_h3_code` arm, proving the unexpected-context
  codes were still reported as unclassified known errors.
- Fix: `classify_peer_stream_code` keeps its four RFC 9114 §8.1 category arms and
  now consults the renamed, explicitly context-aware
  `is_defined_request_stream_h3_code` predicate, whose range is
  `0x103..=0x110 | 0x200`. `0x201`/`0x202` are deliberately excluded and so fall
  through to `PeerStreamError::NoError` together with every other unknown or
  unexpected code. The mapping is therefore not a blanket "all QPACK codes are
  `NoError`": `0x200` stays `Other`, and the four named H3 codes, the low DoQ
  `0x0`-`0x3` transport values, the `0x119` grease code, and unknown codes keep
  their previous meaning. Every termination still never commits, is `Sent`, and
  opens exactly one connection.
- Docs synced: `quic.rs`'s classifier and predicate docs now state the
  request/response-stream context and the RFC 9114 §8 MUST; `secure/error.rs`'s
  `PeerStreamError` enum docs state that `NoError` includes codes defined only
  for another context and that `Other` means "known in this request/response
  stream context"; `design.md` §7 and PRD R9 record the same rule. No test
  expectation other than the two new wire cases changed.
- New evidence (real QUIC+TLS+h3 loopback reset fixture, `slice3_quic`):
  `doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized` now
  drives wire-level resets for `0x201` and `0x202` and asserts
  `PeerStreamTerminated { code: NoError }`, while still asserting `0x200` and
  `0x103` → `Other`, `0x119` → `NoError`, and the low transport codes
  `0x0`-`0x3` → `NoError`, each with `Sent`, no commit, no in-flight residue, and
  exactly one accepted connection. The four named H3 codes remain covered by
  `doh3_peer_stream_termination_codes_are_typed_and_never_commit`, and DoQ's own
  low-code reset semantics by
  `doq_peer_reset_codes_are_terminal_missing_fin_without_commit`.
- Boundary preserved: active h3 `H3_REQUEST_CANCELLED` is still not attempted
  (h3-quinn 0.0.10 `stop_sending` panics after a dropped read future); local
  owner close / caller cancel / deadline keep their typed control errors and
  precedence; unrelated dirty files (`.trellis/spec/...`, `.trellis/workflow.md`,
  `.DS_Store`, `09-19-...` task files) were preserved; no
  `git reset`/`checkout`/`clean`, no `git add -A`, no commit, no push; no
  production `sleep`, private timer, or hidden timeout was added.

### Gates (final worktree, uncommitted)

- Focused `slice0_quic`/`slice0_contract`/`slice1_doq`/`slice2_doh3`/
  `slice3_quic`/`slice2_doh`/`slice3_doh` under debug and again under
  `--release`: all passed, 0 failed (`slice3_quic` 23/23, including the extended
  loopback reset test, in both profiles).
- `cargo test -p mosdns-upstream-core --all-targets --all-features --locked`:
  exit 0, 0 failed.
- `cargo test --workspace --all-targets --all-features --locked`:
  **720 passed / 0 failed** across 39 test binaries.
- `cargo fmt --all -- --check`: clean. `cargo clippy --workspace --all-targets
  --all-features --locked -- -D warnings`: clean. `git diff --check`: clean.
- `python3 ./.trellis/scripts/task.py validate
  rust-phase4-quic-http3-doq-foundation`: `All validations passed` (the
  `rust-migration.md` size warning is pre-existing and informational).

## Slice 3 remediation record 3 — 2026-09-20 (DoH3 HTTP/3 code context mapping, review P1)

- Executor: DSH Web, single executor with no sub-task split, no second session,
  and no MCP. The parent controller retains diff inspection, the quality gates,
  and the external review round.
- Context: the committed Slice 3 remediation 2 (`a425552`, "fix(rust): classify
  DoH3 QPACK codes by stream context") was reviewed by GPT Web over
  `cc11986..a425552` as `FINAL: FAIL`, `P0=0`, `P1=1`. The single P1 was the
  remaining context-insensitive DoH3 error-code predicate in
  `rust/upstream-core/src/quic.rs`: `is_defined_request_stream_h3_code()` still
  used `matches!(value, 0x103..=0x110 | 0x200)`, so it treated *every* code in
  `0x103..=0x110` as a known error in the DoH3 request/response-stream context
  and mapped it to `PeerStreamError::Other`. RFC 9114 §8 requires a *known* code
  used in an unexpected context to be treated as `H3_NO_ERROR`-equivalent too.
  This record covers only that P1. Slice 4 is not started, and no
  resolver/pool/retry/fallback/host wiring, Cargo manifest/`Cargo.lock`, or other
  transport is touched.
- Worktree state: this remediation is left **uncommitted and unpushed** in the
  current worktree (`a425552` HEAD, `origin/rust` synced to it) for the parent to
  inspect and commit. `task.json` stays `in_progress`. Nothing here claims a
  commit or push.

### P1 — known HTTP/3 codes scoped to another context must be `H3_NO_ERROR`

- Problem: the predicate used a numeric range as a proxy for
  "request/response-stream-valid", but RFC 9114 §8.1 defines several codes in
  that range only for another stream/connection context. RFC 9114 §8: "use of an
  error code in an unexpected context or receipt of an unknown error code MUST
  be treated as equivalent to H3_NO_ERROR." The review's explicit
  counterexamples were `H3_CLOSED_CRITICAL_STREAM` (`0x104`, a control/QPACK
  critical stream, §6.2.1), `H3_SETTINGS_ERROR` (`0x109`) and
  `H3_MISSING_SETTINGS` (`0x10a`, both errors of the control stream's SETTINGS
  frame, §6.2.1/§7.2.4), which on a DoH3 request/response stream must be
  `PeerStreamError::NoError`, not `Other`.
- RED (before the fix): `slice3_quic`'s real QUIC+TLS+h3 loopback reset test was
  extended with `0x104`/`0x108`/`0x109`/`0x10a` (expecting `NoError`) and run
  against the unchanged `a425552` production code. It failed in the
  `is_defined_request_stream_h3_code` arm:
  `left: DohProtocol(PeerStreamTerminated { code: Other })`,
  `right: DohProtocol(PeerStreamTerminated { code: NoError })` at code `0x104`.
  A temporary wire probe over the same fixture recorded every flagged code
  before the fix as `Other`: `0x104 -> Other`, `0x108 -> Other`,
  `0x109 -> Other`, `0x10a -> Other` (and `0x105`/`0x107`/`0x10f`/`0x110` as
  `Other`, which stay `Other`). The probe was removed after use.
- Fix: `classify_peer_stream_code` keeps its four RFC 9114 §8.1 category arms
  (`0x100`/`0x101`/`0x102`/`0x10c`) and now consults the explicit per-code
  `is_request_response_stream_h3_code` allowlist instead of a range. Only codes
  whose §8.1/§6 meaning applies to a request/response exchange stay `Other`:
  `0x103` (retained by the frozen Slice 3 reviewed contract), the
  request/response codes `0x105` `H3_FRAME_UNEXPECTED`, `0x106`
  `H3_FRAME_ERROR`, `0x107` `H3_EXCESSIVE_LOAD`, `0x10b`
  `H3_REQUEST_REJECTED`, `0x10d` `H3_REQUEST_INCOMPLETE`, `0x10e`
  `H3_MESSAGE_ERROR`, `0x10f` `H3_CONNECT_ERROR`, `0x110`
  `H3_VERSION_FALLBACK`, and RFC 9204 §6's `0x200`
  `QPACK_DECOMPRESSION_FAILED` (a failed field-section decode on a request
  stream). Codes defined only for another context now fall through to
  `PeerStreamError::NoError` together with the unknown/unexpected space:
  `0x104` `H3_CLOSED_CRITICAL_STREAM`, `0x108` `H3_ID_ERROR` (connection-level
  stream/push-ID bookkeeping), `0x109` `H3_SETTINGS_ERROR`, `0x10a`
  `H3_MISSING_SETTINGS`, RFC 9204's `0x201`/`0x202`, the RFC 9000 §20.1
  transport space below `0x100`, the `0x1f * N + 0x21` grease space, and any
  unregistered code. The mapping is therefore an RFC-context allowlist, not a
  blanket "all `0x103..=0x110` are `NoError`": `0x200` stays `Other`,
  `0x201`/`0x202` stay `NoError`, and the four named codes keep their own
  categories. Every termination still never commits, is `Sent`, and opens
  exactly one connection.
- Docs synced: `quic.rs`'s module doc, `classify_peer_stream_code` doc, and the
  renamed `is_request_response_stream_h3_code` doc state the explicit
  request/response-stream context, name the control/critical/connection-scoped
  codes excluded from it, and quote the RFC 9114 §8 MUST; `secure/error.rs`'s
  `PeerStreamError` enum and `NoError`/`Other` variant docs record the same
  boundary; `design.md` §7 and PRD R9 record the explicit per-code rule.
- New evidence (real QUIC+TLS+h3 loopback reset fixture, `slice3_quic`):
  `doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized` now
  drives wire-level resets for `0x104`/`0x108`/`0x109`/`0x10a` → `NoError`,
  keeps `0x103`/`0x105`/`0x106`/`0x107`/`0x10b`/`0x10d`/`0x10e`/`0x10f`/`0x110`
  and `0x200` → `Other`, `0x201`/`0x202` → `NoError`, the low transport codes
  `0x0`-`0x3` and the `0x119` grease code → `NoError`, and adds an unregistered
  `0x1234` → `NoError`. Each case asserts `Sent`, no commit, no in-flight
  residue, and exactly one accepted connection. The four named H3 codes remain
  covered by `doh3_peer_stream_termination_codes_are_typed_and_never_commit`,
  the mid-body termination by
  `doh3_peer_stream_termination_after_the_head_is_typed_and_never_commits`, and
  DoQ's own low-code reset semantics by
  `doq_peer_reset_codes_are_terminal_missing_fin_without_commit`.
- Boundary preserved: the already-closed DoH3 response-head side-effect P1
  (`classify_h3_head_error` returning a `Sent` receive failure for an ordinary
  post-request-FIN head loss) is **not** reopened and its HTTP/1.1/HTTP/2
  `ResponseHeadNotReceived`/`MaybeSent` semantics are untouched; active h3
  `H3_REQUEST_CANCELLED` is still not attempted (h3-quinn 0.0.10 `stop_sending`
  panics after a dropped read future); local owner close / caller cancel /
  deadline keep their typed control errors and precedence. Unrelated dirty files
  (`.trellis/spec/...`, `.trellis/workflow.md`, `.DS_Store`, the `09-19-...`
  task files) were preserved; no `git reset`/`checkout`/`clean`, no
  `git add -A`, no commit, no push; no production `sleep`, private timer, or
  hidden timeout was added - the only waits are the existing fixture's bounded
  `TEST_TIMEOUT` and explicit `oneshot`/connection signals.

### Gates (final worktree, uncommitted)

- Focused `slice0_quic`/`slice0_contract`/`slice1_doq`/`slice2_doh3`/
  `slice3_quic`/`slice2_doh`/`slice3_doh`: debug **119 passed / 0 failed** and
  `--release` **117 passed / 0 failed** across the 7 binaries, exit 0 each
  (`slice3_quic` 23/23 in both profiles; the extended reset test runs 22 wire
  cases; the release total is lower only for the debug-assertions-gated DoQ
  observation tests).
- `cargo test -p mosdns-upstream-core --all-targets --all-features --locked`:
  **468 passed / 0 failed** across 23 test binaries, exit 0.
- `cargo test --workspace --all-targets --all-features --locked`:
  **720 passed / 0 failed** across 39 test binaries, exit 0.
- `cargo fmt --all -- --check`: clean (exit 0). `cargo clippy --workspace
  --all-targets --all-features --locked -- -D warnings`: clean (exit 0).
  `git diff --check`: clean (exit 0).
- `python3 ./.trellis/scripts/task.py validate
  rust-phase4-quic-http3-doq-foundation`: `All validations passed` (exit 0; the
  `rust-migration.md` size warning is pre-existing and informational).

## Slice 3 remediation record 4 — 2026-09-20 (DoH3 `0x103`/`0x10f` context mapping, review P1)

- Executor: DSH Web, single executor with no sub-task split, no second session,
  and no MCP. The parent controller retains diff inspection, the quality gates,
  and the external review round.
- Context: the committed Slice 3 remediation 3 (`e2ee8d3`, "fix(rust): make DoH3
  error mapping context aware") was reviewed by GPT Web over
  `a425552..e2ee8d3` as `FINAL: FAIL`, `P0=0`, `P1=1`, `P2=1`. The single blocking
  P1 was the remaining explicit request/response-stream allowlist in
  `rust/upstream-core/src/quic.rs`: `is_request_response_stream_h3_code()` still
  returned `true` for `H3_STREAM_CREATION_ERROR (0x103)` and
  `H3_CONNECT_ERROR (0x10f)`, so a `RemoteTerminate` carrying either was mapped
  to `PeerStreamError::Other`. RFC 9114 §8 requires an error code used in an
  unexpected context to be treated as `H3_NO_ERROR`-equivalent, and the concrete
  context of this path is a single plain HTTPS `GET` request/response stream,
  never a `CONNECT` tunnel. This record covers only that P1. Slice 4 is not
  started, and no resolver/pool/retry/fallback/host wiring, Cargo
  manifest/`Cargo.lock`, or other transport is touched.
- Worktree state: this remediation is left **uncommitted and unpushed** in the
  current worktree (`e2ee8d3` HEAD, `origin/rust` synced to it) for the parent to
  inspect and commit. `task.json` stays `in_progress`. Nothing here claims a
  commit or push.

### P1 — `0x103`/`0x10f` are out of context on the plain `GET` request/response stream

- Whole-allowlist re-review against RFC 9114 §8/§8.1 (and RFC 9204 §6 for QPACK)
  in this exact context — one plain `GET`, one client-initiated bidirectional
  request/response stream, never `CONNECT`, never server push:
  * `H3_STREAM_CREATION_ERROR (0x103)`: RFC 9114 §8.1 defines it as "the endpoint
    detected that its peer created a stream that it will not accept" — a *new*
    stream the endpoint refuses, not the termination of an already-accepted
    request/response stream. Out of context on this stream -> `NoError`.
  * `H3_CONNECT_ERROR (0x10f)`: §8.1 defines it for "the TCP connection
    established in response to a CONNECT request" (RFC 9114 §4.4). This path
    sends a plain `GET`; `CONNECT` never occurs. Out of context -> `NoError`.
  * `0x104` `H3_CLOSED_CRITICAL_STREAM` (control/QPACK critical stream),
    `0x108` `H3_ID_ERROR` (connection-level stream/push-ID bookkeeping),
    `0x109` `H3_SETTINGS_ERROR` and `0x10a` `H3_MISSING_SETTINGS` (control-stream
    SETTINGS), RFC 9204's `0x201`/`0x202` (QPACK encoder/decoder streams), the
    RFC 9000 §20.1 transport space below `0x100`, the `0x1f * N + 0x21` grease
    space, and unregistered codes stay `NoError` — no similar problem found.
  * `0x105` `H3_FRAME_UNEXPECTED` ("not permitted ... on the current stream"),
    `0x106` `H3_FRAME_ERROR` (frame layout/size), `0x107` `H3_EXCESSIVE_LOAD`,
    `0x10b` `H3_REQUEST_REJECTED`, `0x10d` `H3_REQUEST_INCOMPLETE`,
    `0x10e` `H3_MESSAGE_ERROR`, `0x110` `H3_VERSION_FALLBACK`, and RFC 9204 §6's
    `0x200` `QPACK_DECOMPRESSION_FAILED` (a failed field-section decode on a
    request stream) each describe a condition of the current request/response
    exchange, so each stays `Other` — no similar problem found.
  The earlier "retained by the frozen Slice 3 reviewed contract" justification
  for `0x103` is removed rather than reused: PRD R9's original wording already
  scopes the `Other` group to codes "该语境下确实适用", so that phrase never
  supplied a basis for keeping `0x103`/`0x10f` once §8.1 scoping is checked.
- RED (before the fix): `slice3_quic`'s real QUIC+TLS+h3 loopback reset test
  `doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized` was
  changed to expect `NoError` for `0x103`/`0x10f` and run against the unchanged
  `e2ee8d3` production code. It failed in the
  `is_request_response_stream_h3_code` arm:
  `left: DohProtocol(PeerStreamTerminated { code: Other })`,
  `right: DohProtocol(PeerStreamTerminated { code: NoError })` at code `0x103`.
  A second run with `0x10f` ordered first produced the same failure at code
  `0x10f`, proving both codes — not just the first — were still classified as
  known in-context errors. The case order was then restored to `0x103`/`0x10f`.
- Fix: `is_request_response_stream_h3_code` drops `0x103` and `0x10f`. The
  predicate remains an explicit per-code allowlist (not a range) of the codes
  whose §8.1/§6 meaning applies to this request/response stream:
  `0x105`/`0x106`/`0x107`/`0x10b`/`0x10d`/`0x10e`/`0x110`/`0x200`. The four
  named `classify_peer_stream_code` category arms (`0x100`/`0x101`/`0x102`/
  `0x10c`) are unchanged, and everything else — including the newly excluded
  `0x103`/`0x10f` — falls through to `PeerStreamError::NoError` under RFC 9114
  §8. Every termination still never commits, is `Sent`, and opens exactly one
  connection.
- Docs synced: `quic.rs`'s `classify_peer_stream_code` and
  `is_request_response_stream_h3_code` docs now name the concrete `GET`
  request/response-stream context and spell out the §8.1 scoping of `0x103`
  (new-stream refusal) and `0x10f` (CONNECT tunnel); `secure/error.rs`'s
  `PeerStreamError` enum and its `NoError`/`Other` variant docs add
  `0x103`/`0x10f` to the out-of-context group and replace the `0x103` example in
  `Other`; `design.md` §7 and PRD R9 record the same rule and no longer cite a
  "frozen contract" as the reason to keep `0x103`/`0x10f`.
- New evidence (real QUIC+TLS+h3 loopback reset fixture, `slice3_quic`):
  `doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized` now
  asserts `0x103`/`0x10f` -> `NoError` while keeping the existing 22 cases:
  `0x104`/`0x108`/`0x109`/`0x10a`, `0x201`/`0x202`, the low transport codes
  `0x0`-`0x3`, `0x119`, and `0x1234` -> `NoError`; `0x105`/`0x106`/`0x107`/
  `0x10b`/`0x10d`/`0x10e`/`0x110` and `0x200` -> `Other`. Each case asserts
  `Sent`, no commit, no in-flight residue, and exactly one accepted connection.
  The four named H3 codes remain covered by
  `doh3_peer_stream_termination_codes_are_typed_and_never_commit`, the mid-body
  termination by
  `doh3_peer_stream_termination_after_the_head_is_typed_and_never_commits`, and
  DoQ's own RFC 9250 reset-code semantics by
  `doq_peer_reset_codes_are_terminal_missing_fin_without_commit`.
- Boundary preserved: the already-closed DoH3 response-head side-effect P1/H1/H2
  semantics (`classify_h3_head_error` returning a `Sent` receive failure after
  the request FIN; `DohProtocolError::ResponseHeadNotReceived`/`MaybeSent` for
  HTTP/1.1/HTTP/2) are **not** reopened; active h3 `H3_REQUEST_CANCELLED` is
  still not attempted (h3-quinn 0.0.10 `stop_sending` panics after a dropped
  read future); local owner close / caller cancel / deadline keep their typed
  control errors and precedence; DoQ's low-code reset semantics are untouched.
  Slice 4, resolver/pool/retry/fallback/host, the Cargo manifest/`Cargo.lock`,
  and every other transport are untouched. Unrelated dirty files
  (`.trellis/spec/backend/quality-guidelines.md`, `.trellis/workflow.md`,
  `.DS_Store` files, the `09-19-...` task files) were preserved; no
  `git reset`/`checkout`/`clean`, no `git add -A`, no commit, no push; no
  production `sleep`, private timer, or hidden timeout was added — the only
  waits are the existing fixture's bounded `TEST_TIMEOUT` and explicit
  `oneshot`/connection signals.

### Gates (final worktree, uncommitted)

- Focused Slice 0/1/2/3 debug set (`slice0_contract`, `slice0_quic`,
  `slice0_secure`, `slice1_doq`, `slice1_dot`, `slice1_udp`, `slice2_doh`,
  `slice2_doh3`, `slice2_tcp`, `slice3_doh`, `slice3_policy`, `slice3_quic`):
  **232 passed / 0 failed** across 12 binaries, exit 0. `slice3_quic` 23/23.
- Same focused set under `--release`: **230 passed / 0 failed** across 12
  binaries, exit 0. `slice3_quic` 23/23; the only difference is `slice1_doq`
  7 -> 5 because two cases are `debug_assertions`-gated.
- `cargo test -p mosdns-upstream-core --all-targets --all-features --locked`:
  **468 passed / 0 failed** across 23 test binaries, exit 0.
- `cargo test --workspace --all-targets --all-features --locked`:
  **720 passed / 0 failed** across 39 test binaries, exit 0.
- `cargo fmt --all -- --check`: clean (exit 0, no output).
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  clean (exit 0). `git diff --check`: clean (exit 0, no output).
- `python3 ./.trellis/scripts/task.py validate
  rust-phase4-quic-http3-doq-foundation`: `All validations passed` (exit 0; the
  `rust-migration.md` size warning is pre-existing and informational).
