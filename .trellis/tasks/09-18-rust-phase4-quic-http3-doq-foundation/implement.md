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
