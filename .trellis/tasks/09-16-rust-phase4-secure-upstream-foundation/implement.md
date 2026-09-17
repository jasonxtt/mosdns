# Secure upstream implementation plan — Slice2 active

Historical record, 2026-09-16: implementation was first authorized for the
bounded Slice0 dependency/MSRV and Hyper API-inspection scope only. That
authorization superseded the earlier planning-only boundary for Slice0; it did
not authorize Slice1+, CI mutation or deployment, and each later slice still
needed its own review and explicit go-ahead. Slice0's technical checklist was
then completed under its separate review boundary.

2026-09-17: Slice1 was explicitly accepted by `rust0916` at
`25c7c961453e15d7347d65bbc401026f813ff27c` (`PASS / Slice1 CLOSED`). The user
then explicitly authorized Slice2. Slice2 is now the active bounded scope;
Slice3+, production wiring, deployment, and automatic progression remain
unauthorized. The executor routing and prompt-approval contract is recorded in
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

- [ ] Pure GET encoding tests precede I/O: zero outbound ID without mutation,
  unpadded base64url, path and unrelated query fields, duplicate dns replacement,
  service Host identity independent of numeric dial address.
- [ ] Low-level Hyper HTTP1 driver with no pooling/retry/redirect/proxy, served
  by local TLS fixture; missing ALPN and negotiated HTTP1 are both covered.
- [ ] Incremental body/header limits and full DNS response validation: 200,
  MIME parameters/case, missing/wrong MIME, 3xx/4xx/5xx, Content-Encoding,
  Content-Length mismatch, chunked body, 65535 vs 65536 bytes and early EOF.
- [ ] Restore caller ID regardless of remote DNS ID; assert owned response
  metadata agrees with wire. No implicit cache/TTL adjustment.
- [ ] One absolute deadline and drop/close coverage across header/body/commit;
  request counter proves at most one GET on failure, never a second connection.

Allowed: secure DoH module/request builder and HTTP1 tests/fixtures.
STOP for scoped review; no HTTP2 executor/pool in this slice.

## Slice3 — HTTP/2 and complete structured shutdown

- [ ] Implement only reviewed ALPN dispatch and scoped HTTP2 executor/driver.
  No independent queries share a connection; no h3 or failed-request replay.
- [ ] Test h2 success, service authority/path, concurrent independent owners,
  unexpected ALPN, reset/GOAWAY/refused stream/EOF and send-state classification.
- [ ] Track all executor futures before spawn; freeze spawn admission at teardown;
  prove child liveness retains owner registration when caller future is aborted.
- [ ] Explicit barriers cover cancellation and close while handshake, request,
  headers, data, final validation/commit or driver teardown are parked.
- [ ] Record zero sockets/registrations/driver/executor tasks after close on
  success, all errors, dropped requests, and repeated/concurrent close.
- [ ] Server-side counters prove no hidden library retry or protocol fallback.

Allowed: scoped executor/DoH HTTP2 implementation and lifecycle tests. If API
behavior violates the ownership design, STOP and revise; do not weaken drain.
STOP for scoped review before final evidence slice.

## Slice4 — final quality and isolated Linux evidence

- [ ] Re-inspect every PRD AC against tests and every matrix/deferred item.
- [ ] Run full required checks once at final boundary; additional runs only for
  changes/failures. Use existing loopback CI structure; ordinary rust-foundation
  must actually run secure tests and clippy, not only dns/upstream legacy targets.
- [ ] Record exact source revision, toolchain, OS/architecture, commands/results,
  test counts, dependency features/MSRV/licenses and resource-drain evidence.
- [ ] No public upstream or production host is needed. Linux Actions/isolated
  test evidence is distinct from macOS results; no claim that a library test is
  full native host E2E, production throughput or long-running deployment proof.
- [ ] Inspect exact changed paths; retain no Go/cgo/ABI/selector/host/listener/
  config/API/WebUI changes or copied KixDNS source.
- [ ] Final root acceptance, then STOP. Archive only after a later wrap-up
  instruction; no implied next protocol/task activation.

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
