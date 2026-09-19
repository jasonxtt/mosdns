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
