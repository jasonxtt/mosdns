# Secure upstream implementation plan — NOT STARTED

2026-09-16 authorization is planning only. All checkboxes below are prospective.
Do not run activation, dependency edits, RED tests, implementation, CI mutation,
commit/push or deployment until the user separately authorizes the reviewed
planning package. No external/root-review PASS is claimed by this document.

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
- [ ] Subsequent explicit user authorization for activation and Slice0.

`codex.dispatch_mode=inline` currently applies. Curated JSONL context is included
for future reviews but does not authorize dispatch. If the user later selects a
DSH/external-review workflow, follow the relevant quality spec and confirmed
review destination; do not invent a review thread or send messages now.

## Slice0 — contracts, dependency and lifecycle preflight

Goal: freeze public types, helper reuse and selected libraries before TLS I/O.

- [ ] Re-read approved planning and trellis-before-dev; inspect current git state.
- [ ] Resolve candidate dependencies under the actual workspace MSRV (1.85),
  record exact version/features/license graph; do not silently bump MSRV.
- [ ] Inspect selected Hyper HTTP1/2/Hyper-util APIs and sources for retries,
  executor spawns, header bounds and drop semantics; document every owned child.
  Prove feasibility of sealed tracked executor and in-flight accounting.
- [ ] Review rustls provider/root/config and insecure verifier interfaces;
  ensure handshake signature checks stay enabled and no 0-RTT/resumption.
- [ ] RED/GREEN constructor and pure request-building contracts: identity/dial
  separation, IPv4/IPv6, invalid roots/identity/port/URL, query/path normalization,
  maximum DNS/URL size and explicit insecure option.
- [ ] Introduce only reviewed secure types/helper access and dependencies;
  preserve existing UDP/TCP public types and default behavior.

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
- No task activation, dependency installation, implementation, commit or push.
  The new task is planning, with final proposal delivered for later review.

## Publication authorization — 2026-09-16

The user subsequently authorized committing and pushing the wrap-up and planning
artifacts to GitHub. This supersedes the original no-commit/push boundary only;
task status remains planning and every implementation gate remains closed.
