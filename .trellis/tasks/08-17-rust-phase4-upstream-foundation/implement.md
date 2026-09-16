# Phase 4 upstream transport implementation plan

> The Phase4 planning gate passed at `16192ea`; the user authorized
> `task.py start` on 2026-09-16. The task is `in_progress` and this round is
> limited to Slice0. Do not start Slice1, add network code, or add production
> wiring without another explicit root-review authorization.

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
- Keep the task status planning until the root reviewer explicitly authorizes
  task start. After authorization, each later slice still requires its own
  acceptance decision.

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
- Open -> Closing -> Closed is idempotent, rejects new work, and describes
  in-flight cleanup;
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
  the missing upstream types and missing `dns-core` header helper.
- GREEN evidence: the focused upstream contract suite has 10 passing tests;
  the dns-core header suite and affected clippy/format checks pass.
- The workspace member is `rust/upstream-core`; its only normal dependency is
  `mosdns-dns-core`. No Tokio/runtime, sequence-core, FFI, or Go dependency
  was added.
- The Slice0 stop condition is active: no UDP/TCP I/O, fallback, retry,
  listener, host wiring, or Slice1 work was started.

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
