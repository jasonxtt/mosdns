# Phase 4 upstream transport implementation plan

> The Phase4 planning gate passed at `16192ea`; the user authorized
> `task.py start` on 2026-09-16. The task is `in_progress`; Slice0 passed root
> review and the user explicitly authorized the Slice2 fresh plain-TCP framing
> round after the Slice1 root-review PASS. Do not start Slice3/TC fallback or
> add production wiring without another explicit root-review authorization.

## Execution rules

- Preserve the current clean implementation baseline and unrelated worktree
  ownership.
- Work only in the future Rust-native upstream boundary. Do not reopen Phase3B,
  extend the transitional runtime FFI, or add a Go adapter, selector, fallback,
  C handle, or Go pool ownership.
- Every slice follows the same loop:
  1. write or extend the failing/red contract tests and evidence fixtures;
  2. implement the minimum Rust-native behavior needed by those tests;
  3. run focused verification plus the relevant existing gates;
  4. stop and request root review before starting the next slice.
- A failed gate stops the slice. Do not paper over a failure with a broader
  retry, a hidden runtime, a second implementation, or a Go fallback.
- The implementation must use the decisions in design.md and the classifications
  in its compatibility matrix. Any new behavior or scope needs a new review.
- Keep the task status `in_progress` after the authorized task start. Each
  later slice still requires its own acceptance decision.

## Slice 0 — contract skeleton and dependency boundary

Goal: establish the smallest reviewable Rust API and test vocabulary without a
complete network implementation.

Red tests and fixtures first:

- query validation rejects empty, malformed, and unframeable input without
  touching a socket;
- the request API borrows query bytes read-only and records the original ID;
- typed errors expose InvalidRequest, InvalidEndpoint, Cancelled,
  DeadlineExceeded, Closed, and side-effect state;
- an absolute deadline and cancellation token have deterministic precedence;
- a prepared exchange keeps caller cancellation distinct from owner close and
  reports `Closed` versus `Cancelled` without polling-only ownership;
- Open -> Closing -> Closed is idempotent, rejects new work, and describes
  in-flight cleanup;
- close completion cannot skip `Closing` and turn an open owner directly into
  `Closed`;
- the DNS header inspection contract distinguishes QR, TXID, TC, and
  undersized headers without duplicating RR parsing;
- the future crate dependency graph has no sequence-core/upstream-core cycle;
  the host composes the two sibling crates and upstream-core does not import
  sequence-core or rust/runtime FFI;
- SideEffectState is a closed NotSent/MaybeSent/Sent enum, TCP connect failure
  is NotSent, and Runtime/Internal preserves the last tracked state without an
  Unknown variant;

Minimum implementation after the red tests:

- after task start is authorized, add future rust/upstream-core to the workspace;
- add only reviewed Rust-native dependencies, expected to be Tokio,
  tokio-util, and bytes if the API needs them;
- define Endpoint, ExchangeRequest, ExchangeContext, owned response,
  side-effect marker, typed error, cancellation boundary, and close state;
- add the smallest dns-core header-inspection extension if the existing public
  API cannot inspect TC safely;
- provide no complete UDP/TCP path and no production host wiring in Slice 0.

Focused verification:

- cargo fmt --check for the affected Rust files;
- cargo test -p mosdns-dns-core and the new contract crate tests;
- cargo clippy for the affected packages with warnings denied;
- inspect cargo tree to confirm the dependency direction and absence of
  rust/runtime or sequence-core from the low-level transport dependency;
- task.py validate, while keeping task.json status as authorized by the
  workflow and never archiving from this slice.

STOP: report the contract and dependency evidence to the root reviewer. Do not
start Slice 1 without explicit approval.

### Slice 0 completion record — 2026-09-16

- `task.py start rust-phase4-upstream-foundation` completed successfully;
  `task.json.status = in_progress`.
- RED evidence: the new contract test target initially failed to compile for
  the missing upstream types and missing `dns-core` header helper. The narrow
  remediation test target then failed to compile for the missing
  `CloseCompletion`, owner-cancellation access, and prepared-control checks.
- GREEN evidence: the focused upstream contract suite has 12 passing tests;
  the dns-core header suite and affected clippy/format checks pass.
- The workspace member is `rust/upstream-core`; it depends on
  `mosdns-dns-core` and `tokio-util` only. `tokio-util` supplies the
  wakeable Rust cancellation token; Slice0 creates no Tokio runtime. No
  sequence-core, FFI, or Go dependency was added.
- The Slice0 stop condition is active: no UDP/TCP I/O, fallback, retry,
  listener, host wiring, or Slice1 work was started.

### Slice 0 narrow remediation record — 2026-09-16

- Root review found two scoped blockers: prepared exchanges did not observe
  owner cancellation separately from caller cancellation, and public
  `finish_close` could skip `Open -> Closing`.
- The minimum repair adds a `tokio-util` cancellation token with an async wake
  method, an `ExchangeControl` containing distinct caller/owner tokens, and a
  compare-exchange guarded `finish_close` that returns `NotClosing` from
  `Open`. It still performs no network I/O and does not add Slice1 scope.

## Slice 1 — UDP exchange primitive

Goal: implement the chosen one-exchange/one-socket UDP architecture.

Red tests and fixtures first:

- basic numeric IPv4 and IPv6 loopback exchange;
- caller query bytes and original DNS ID remain unchanged;
- accepted response requires QR, matching ID, expected source, and valid
  non-TC wire;
- wrong-peer and wrong-ID datagrams do not get accepted by the exchange;
- malformed and undersized expected-peer datagrams produce the documented
  typed error;
- concurrent exchanges cannot consume each other's responses;
- cancellation before bind/send, after send, and during receive releases the
  socket and returns the correct side-effect marker;
- deadline before send and after send are distinct from cancellation;
- a legal large UDP wire is not silently truncated by a 4095-byte buffer;
- late traffic after return/cancellation cannot mutate or complete a later
  exchange;
- no automatic duplicate UDP send occurs unless a separately approved policy
  exists.

Minimum implementation:

- bind an ephemeral address in the endpoint family;
- send the unchanged query once;
- receive with a full legal DNS datagram capacity;
- enforce expected source and response-ID checks;
- validate complete responses through dns-core;
- compose deadline/cancellation with socket I/O and cleanly drop all resources;
- return structured errors and diagnostic mismatch context.

Focused verification:

- focused UDP unit/integration tests with deterministic local test servers;
- IPv4/IPv6 matrix where the host supports both;
- cargo fmt, package tests, clippy, and a short repeated concurrent run;
- run the repository's Rust quality commands required by the task workflow;
- record any OS-specific behavior instead of weakening the contract.

STOP: root review of UDP ownership, validation, error states, and concurrency
evidence.

### Slice 1 RED contract record — 2026-09-16

- RED-test-only step: no UDP/TCP production code, TC fallback, pooling,
  pipeline, retry, listener, TLS, Go/cgo/ABI/selector/fallback, or runtime
  wiring was added.
- Added `rust/upstream-core/tests/slice1_udp.rs` (19 tests) pinning the reviewed
  one-exchange/one-socket UDP contract through the `Upstream::exchange` entry
  point sketched in `design.md` section 3: numeric IPv4/IPv6 loopback exchange,
  unchanged borrowed query/original ID, QR/TXID/source validation, wrong-peer
  and wrong-ID handling, undersized/malformed expected-peer behavior,
  concurrent isolation, cancellation/deadline/owner-close side-effect states,
  legal full-datagram capacity (the full 65507-byte IPv4 payload on Linux; the
  host UDP maximum elsewhere, because macOS caps `net.inet.udp.maxdgram` at
  9216), no duplicate send, and late-datagram release isolation.
- Added a test-only `tokio` dev-dependency (`rt`, `sync`, `time`). Mock servers
  are blocking `std::net::UdpSocket` tasks on Tokio's blocking pool, so no
  `net`/`macros` feature and no new package entered `Cargo.lock`; the only
  lockfile change is `tokio` in the `mosdns-upstream-core` dependency list.
- RED evidence: `cargo test -p mosdns-upstream-core --test slice1_udp` exits
  101 with exactly six `E0599: no method named exchange` errors and no other
  error kind. The existing `slice0_contract` target still passes 12/12, the
  library still builds, and `cargo fmt -p mosdns-upstream-core -- --check` is
  clean.
- API/design blocker for review: `design.md` section 3 sketches
  `Upstream::exchange(&self, ExchangeRequest<'_>, ExchangeContext)`, while the
  Slice0 record materialized `prepare_exchange`/`PreparedExchange` as the
  pre-I/O boundary. The RED tests pin the high-level `Upstream::exchange` shape;
  the reviewer should confirm whether Slice 1 attaches the UDP I/O to
  `Upstream` or to `PreparedExchange` before implementation.
- Stop condition active: no attempt was made to make the tests pass, and all
  changes remain uncommitted.

### Slice 1 implementation record — 2026-09-16

- The user explicitly authorized Slice1 after the Slice0 root-review PASS.
- RED-first evidence is preserved in local commit `3a04a2c`: the new
  `slice1_udp` target failed with exactly six missing-`Upstream::exchange`
  compiler errors before production implementation.
- DSH implemented the bounded one-exchange/one-ephemeral-socket UDP primitive
  in `rust/upstream-core/src/udp.rs` and the smallest public API wiring in
  `src/lib.rs`. Production Tokio uses only `macros`, `net`, `rt`, and `time`;
  the crate still creates no runtime.
- Main-worktree verification reproduced GREEN: Slice1 19/19, Slice0 12/12,
  `cargo check --locked`, and `cargo fmt --all -- --check` pass. The DSH
  implementation also passed three repeated Slice1 runs and the focused
  clippy check. Full workspace and root review remain before completion.
- Scope remains UDP only: no TCP, TC-to-TCP fallback, retransmission, generic
  retry, pool, reuse, pipeline, listener, Go/cgo/ABI/selector/fallback, or
  production wiring. Task status remains `in_progress`; stop here for root
  review before Slice2.

### Slice 1 narrow root-review remediation record — 2026-09-16

- Formal review of `276d847` returned a scoped FAIL with exactly three
  blockers: synchronous close could expose `Closed` before a real UDP future
  exited; local UDP bind/setup was classified as `Runtime(NotSent)` instead of
  `Connect(NotSent)`; and ignored wrong-peer/wrong-ID observations were not
  retained in terminal diagnostics.
- DSH first attempted the complete scoped remediation in an isolated clean
  worktree; that run timed out after 600 seconds with no applicable diff and
  was safely discarded. The remediation was then split into two isolated DSH
  executions. The blocker-A diff was inspected and applied only from the
  explicit four-file whitelist, then committed locally as `45c7030`.
- Blocker-A RED evidence: before production changes, focused Slice1 tests
  exited 101 with 27 compile errors: nine `CloseResult is not a future`, 16
  missing `in_flight_exchanges`, one missing `poll`, and one missing
  `CloseCompletion::InFlight`. GREEN added nine tests and passed Slice1 28/28,
  Slice0 12/12, focused clippy, and fmt.
- Blocker-A implementation uses a mutex-serialized lifecycle/registration
  gate, RAII in-flight guards, `Notify` drain wakeups, and caller-runtime
  `Upstream::close().await`. It does not spawn exchange tasks or create a
  runtime; `Closed` is exposed only after registrations reach zero.
- Blockers-B/C RED evidence was then produced in the second isolated DSH
  worktree: the bind helper import failed to compile, and diagnostic imports /
  accessors were missing. GREEN adds the minimal `Connect` mapping seam and
  `Diagnosed { cause, ignored }` typed wrapper with two boolean flags. Wrong
  peer/ID remain ignored; deadline/cancellation/owner-close/receive retain the
  primary cause and `Sent` state, while later valid responses succeed normally.
- Main-worktree focused verification after both applies: Slice1 33/33 on the
  first run and two additional repeated runs; Slice0 12/12; upstream-core lib
  bind test passed; full workspace tests, dns-core tests, warnings-denied
  clippy, fmt, cargo tree, task validate, and diff checks all pass. The final
  remediation commit is `fda0455`; task status remains `in_progress` and the
  result is awaiting root review.
- Scope remains strictly Slice1 UDP remediation. No TCP, TC-to-TCP fallback,
  retransmission, retry, pool, reuse, pipeline, listener, Go/cgo/ABI/selector/
  fallback, or production wiring was added. STOP for root review before Slice2.

### Slice 1 response-commit linearization remediation record — 2026-09-16

- Formal review of baseline `9e30bd2` left exactly one Slice1 blocker: after a
  datagram passed expected-peer, header/ID, and (for non-TC) dns-core
  `validate_response`, both UDP success paths returned the owned response
  without an atomic response-commit decision against the owner lifecycle
  `Open -> Closing`. A concurrent owner close could therefore race a successful
  response return. The frozen contract is that a completed response commit
  beats a later close, while a close that enters `Closing` first wins and the
  exchange returns `Closed(Sent)`.
- RED evidence: the focused tests were added before production changes. The
  deterministic seam and lifecycle-gate tests referenced a still-missing
  `crate::CommitPause`, `Upstream::install_commit_pause`, and
  `Lifecycle::commit_response`. `cargo test -p mosdns-upstream-core --lib
  --locked` exited 101 with exactly six errors and no other error kind: one
  `E0432` unresolved `crate::CommitPause` import, one `E0599` missing
  `install_commit_pause`, and four `E0599` missing `Lifecycle::commit_response`.
- Minimum implementation: `Lifecycle::commit_response` is one synchronous
  operation that takes the same short-lived mutex as `register` and
  `begin_close`. It returns `Ok(())` only while the owner is `Open`, so a later
  `begin_close` cannot reverse a committed success, and returns
  `Closed(Sent)` from `Closing`/`Closed`. A `ResponseCommit` handle carries only
  the lifecycle borrow (and, under `cfg(test)`, an optional pause) into
  `udp::exchange`; it owns no response bytes, socket, or parser state. Both the
  valid TC path and the validated non-TC path call the gate immediately after
  their required header/ID/validation steps and immediately before constructing
  the owned response. The commit error is mapped through the existing
  `diagnosed` helper so retained wrong-peer/wrong-ID diagnostics survive a
  close-wins commit, while a committed success still carries none.
- Deterministic ordering proof: a private `#[cfg(test)]` `CommitPause` seam
  parks the transport immediately before the gate and is released by the test;
  it is not a public API, and no sleep or second `check_at` poll is the proof.
  The in-flight registration guard is untouched and remains held through the
  commit and response return, so async close still drains it.
- Coverage: two lifecycle ordered-outcome unit tests (commit-before-close and
  close-before-commit), three in-crate transport seam tests (non-TC
  close-before-commit, TC close-before-commit, and commit-before-close), and two
  public-API integration tests (a truncated response is a committed observation;
  a committed response survives a later `close().await`). Slice1 is 35/35,
  Slice0 12/12, the upstream-core lib target 6/6, the commit/close race tests
  passed 20 consecutive runs, and `cargo fmt --check`, warnings-denied clippy,
  `cargo check --all-targets`, and `--locked` all pass.
- Scope remains strictly the Slice1 UDP response-commit gate. No TCP,
  TC-to-TCP fallback, retransmission, retry, pool, reuse, pipeline, listener,
  generalized transport trait/factory, new dependency, hidden runtime,
  `block_on`, `spawn`, Go/cgo/ABI/selector/fallback, or production wiring was
  added. `#![forbid(unsafe_code)]` remains. Task status stays `in_progress`;
  STOP for root review before Slice2.

### Slice 2 authorization record — 2026-09-16

- The Slice1 gate is formally `PASS / CLOSED` at root review commit
  `7c65a1741601a22d61375c38c30932b34033eca3`. The user explicitly authorized
  starting Slice2 in this task; `task.json` remains `in_progress` and no second
  `task.py start` is required.
- The authorized scope is only one fresh plain TCP connection per exchange,
  exact two-byte DNS framing, complete writes, exact prefix/body reads, typed
  malformed/truncated/oversize handling, deadline/cancellation, validation,
  connection close, and concurrent stream isolation.
- Slice3 TC-to-TCP composite policy, pooling, reuse, pipeline, retry,
  retransmission, production wiring, Go/cgo/ABI/selector/fallback, and later
  phases remain unauthorized. Slice2 must stop for root review when its
  focused and full checks pass.

## Slice 2 — TCP framing primitive

Goal: implement one fresh plain TCP connection per exchange with exact DNS
framing, without pooling or pipelining.

Red tests and fixtures first:

- two-byte big-endian length encoding;
- partial prefix reads, partial body reads, zero length, and an outbound query
  larger than u16::MAX;
- full write semantics when the test stream accepts short writes;
- EOF before prefix/body completion maps to TruncatedFrame;
- response ID, QR, and DNS validation are enforced after a complete frame;
- connect, write, read, and validation respect the same absolute deadline;
- cancellation during connect, write, prefix read, and body read closes the
  stream and prevents a retry;
- a failed or malformed connection is never reused;
- concurrent TCP exchanges use separate streams and owned responses.

Minimum implementation:

- connect to the numeric TCP endpoint;
- encode one non-zero u16 big-endian prefix and the query;
- perform complete writes and exact prefix/body reads;
- reject a zero inbound prefix, partial/mismatched frames, and an outbound
  oversize query with typed errors; a non-zero two-byte inbound prefix is
  inherently at most u16::MAX;
- close the connection on every failure or cancellation;
- pass only a complete response wire to dns-core.

Focused verification:

- deterministic in-process TCP test server with deliberate short reads/writes;
- malformed-frame and EOF matrix;
- timeout/cancellation tests under repeated runs;
- cargo fmt, package tests, clippy, and sanitizer/Miri coverage where
  applicable to owned state (not network timing assumptions).

STOP: root review of TCP framing, side-effect classification, and fresh
connection lifecycle.

### Slice 2 implementation record — 2026-09-16 (root review PASS / CLOSED)

- The implementation was executed through bounded, isolated DSH jobs. The
  parent inspected each complete patch before applying it; DSH did not commit,
  push, or modify files outside the allowlist.
- `rust/upstream-core/src/tcp.rs` now owns the fresh plain-TCP exchange,
  two-byte big-endian framing, complete writes, exact prefix/body reads, one
  absolute deadline, owner/caller cancellation precedence, typed side-effect
  errors, response validation, and the existing response-commit gate. The
  stream is local to one exchange and is dropped on every return path.
- `rust/upstream-core/tests/slice2_tcp.rs` covers fragmented framing, valid
  sequential/concurrent exchanges, cancellation/deadline/owner close while
  reading, zero/partial/EOF frames, QR/DNS/ID validation failures, oversize
  pre-connect rejection, refused connect, registration release, unchanged
  borrowed query bytes, and bounded no-retry observations. Non-EOF read
  failure remains covered by the deterministic in-crate framing unit test.
- The same-thread root review found one narrow blocker: the synchronous
  validation-to-commit boundary did not observe caller cancellation and the
  original absolute deadline. Commit `95a7aa1` adds one final control-aware
  decision under the existing lifecycle mutex, after complete frame read and
  DNS validation, with owner close > caller cancellation > deadline > success
  precedence. Deterministic real-TCP gate tests cover cancellation,
  deadline, owner close, and commit-before-close success.
- The same-thread final root review of `b83dbb4` formally returned
  **Slice2 PASS / CLOSED**. It accepted the final validation-control gate and
  all previously reviewed framing, lifecycle, ownership, dependency, and
  error-matrix contracts.
- The focused control RED run initially failed with three bounded elapsed
  results before `race_io` was wired; the error-matrix additions were
  tests-only and all passed against the existing implementation, so no
  artificial production defect was introduced. GREEN is 14 Slice2 tests,
  30 library tests, 12 Slice0 tests, and 35 Slice1 tests.
- Verification passed with `cargo fmt --all -- --check`,
  warnings-denied `cargo clippy` for `mosdns-upstream-core`, `cargo tree`
  inspection, and `git diff --check`. No dependency, `sequence-core`,
  `mosdns-runtime`, Go, C ABI, selector, fallback, or production wiring
  change was made. The implementation commits are `8c1ce5b`, `3bf63a2`,
  `d0ca321`, and `95a7aa1`.
- `task.json` remains `status = in_progress`. STOP here after the closed root
  review and wait for the user's decision; Slice3, TC-to-TCP composite
  fallback, and all later phases are not authorized.

## Slice 3 — UDP TC to TCP composite policy

Goal: add the reviewed protocol fallback without rerunning sequence policy or
resetting the deadline.

Red tests and fixtures first:

- valid UDP TC header triggers TCP with the same query bytes and original ID;
- TC does not trigger fallback when cancellation is already requested;
- TCP receives only the remaining portion of the original absolute deadline;
- a complete UDP response does not cause TCP;
- malformed UDP is not reclassified as TC;
- TCP failure preserves structured prior TC context;
- no generic retry occurs after a post-send UDP or TCP failure;
- cancellation during the transition prevents any TCP connect/send;
- close during UDP or fallback leaves no pending work.

Minimum implementation:

- introduce the composite policy above the UDP/TCP primitives;
- inspect TC through the dns-core header boundary;
- carry one ExchangeContext and one side-effect record across the transition;
- use the same owned query wire for TCP;
- implement only the approved TC fallback, not generic retries, pooling,
  retransmission, or Go re-entry.

Focused verification:

- deterministic UDP/TCP pair with counters for sends and connects;
- deadline budget and cancellation race tests;
- malformed/TC/ID/source cases;
- cargo fmt, package tests, clippy, and focused repeated runs.

STOP: root review of protocol fallback, retry safety, and deadline/cancel
precedence.

## Slice 4 — final quality gate and evidence

Goal: prove the bounded transport foundation and document unresolved scope
without wiring it into production.

Required verification plan:

- cargo fmt --check;
- cargo test for the full Rust workspace and focused Phase4 packages;
- cargo clippy --workspace --all-targets --all-features -- -D warnings,
  adjusted only for documented repository command conventions;
- release-profile build for the relevant Rust packages;
- isolated Linux network tests covering loopback UDP/TCP, concurrent exchanges,
  malformed frames, cancellation, deadline, close, and TC fallback;
- existing Go tests and build remain green because no Go production code is
  changed;
- inspect the final dependency graph for no forbidden FFI/selector/pool
  boundary;
- review all compatibility matrix rows, research-unresolved entries, and
  KixDNS evidence;
- confirm no TLS/HTTPS/QUIC/listener/config/WebUI/API/deployment work leaked
  into the diff;
- capture reproducible command output and test-environment constraints in a
  planning/evidence document.

STOP: final root acceptance of the Phase4 foundation. This plan does not
authorize Rust-native host wiring, production/default selection, retirement of
hybrid scaffolding, or release. Those require later tasks and gates.

## Review handoff checklist

Before each root-review stop, report:

- exact slice and commit/worktree diff scope;
- tests added first and the behavior they freeze;
- focused commands and complete pass/fail results;
- compatibility classifications changed, if any;
- resources and cancellation paths inspected;
- unresolved research questions;
- explicit confirmation that no unapproved code path, Go fallback, ABI,
  selector, production wiring, stage, commit, or archive action was taken.
