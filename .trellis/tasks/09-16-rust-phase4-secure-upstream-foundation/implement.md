# Secure upstream implementation plan — Slice0 active

2026-09-16: implementation is now authorized and Slice0 is active for the
bounded dependency/MSRV and Hyper API-inspection scope only. This supersedes
the earlier planning-only boundary for Slice0; it does not authorize Slice1+,
CI mutation or deployment, and each later slice still needs its own review and
explicit go-ahead. Slice0's technical checklist is now complete; its scoped
external/root review is still pending and no PASS is claimed by this document.

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

`codex.dispatch_mode=inline` remains the task setting; the user explicitly
selected MCP DSH as the bounded executor and `rust0916` as the review
destination for this implementation turn. Follow the quality spec and do not
invent another review thread or send work to a different destination.

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

- [ ] One RED -> GREEN behavior at a time; synthetic local CA/server fixtures
  with documented provenance (valid, wrong name, expired and unknown issuer).
- [ ] Numeric connect, authenticated handshake, shared context/deadline, explicit
  insecure path; no query before successful handshake and no plaintext fallback.
- [ ] Reuse exact framing helpers; explicitly test TLS flush, partial writes,
  partial prefix/body, EOF, wrong ID, malformed/full-TC response and size limits.
- [ ] Assert unchanged query/original ID, fresh connections per exchange,
  concurrent isolation and typed TLS/send/receive/control error state.
- [ ] Test owner close, caller cancellation, deadline and dropped/aborted future
  during connect, handshake, frame write/flush/read and final commit; drain all.

Allowed: secure TLS/DoT modules, tests/fixtures and minimal private helper changes.
STOP for scoped review; no DoH implementation or connection reuse in this slice.

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
