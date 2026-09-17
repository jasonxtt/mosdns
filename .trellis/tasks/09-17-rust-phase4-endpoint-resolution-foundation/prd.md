# Rust Phase 4 endpoint resolution foundation

## Goal

Build the pure-Rust endpoint-resolution foundation needed to turn a configured
upstream hostname into validated numeric dial destinations without changing the
independent TLS/HTTP service identity. The result must compose directly with the
reviewed UDP/TCP/DoT/DoH primitives and the future Rust-native host; it must not
add a Go bridge, selector, fallback, or production wiring.

User value: hostname-based upstreams and explicit bootstrap DNS remain usable
after the Rust-native host replaces the current Go control/data plane, while
resolution latency, cancellation, expiry, refresh failure, and shutdown have
deterministic behavior.

## Background and confirmed facts

- The archived UDP/TCP foundation accepts only validated numeric
  `SocketAddr` endpoints. The archived secure-upstream foundation deliberately
  keeps that numeric dial destination separate from `ServerIdentity` and the
  DoH URL authority (`rust/upstream-core/src/lib.rs`,
  `rust/upstream-core/src/secure/endpoint.rs`).
- Current MosDNS configuration exposes global and per-upstream `bootstrap` and
  `bootstrap_version`, plus `dial_addr`. Global values are defaults for the
  per-upstream values (`plugin/executable/forward/forward.go:54-85,131-156`).
- `dial_addr` changes only the network destination. It must not change TLS SNI
  or the HTTP Host/authority (`pkg/upstream/upstream.go:69-77`).
- A bootstrap server must be a numeric IP with optional port; the default port
  is 53 (`pkg/upstream/upstream.go:103-110`, `pkg/upstream/utils.go:77-90`).
- The current Go bootstrap path sends an EDNS(0) UDP query to the numeric
  bootstrap server, retransmits once per second, uses a five-second resolution
  budget, returns the first usable A/AAAA answer, refreshes no sooner than five
  minutes, retries a failed refresh after two seconds, and retains the last
  successfully published address (`pkg/upstream/bootstrap/bootstrap.go:37-41,
  88-152,155-237`). These are characterization inputs, not automatically
  normative Rust internals.
- Current `bootstrap_version` accepts `0`, `4`, or `6`; `0` currently maps to A
  like `4`, while dual-stack resolution is explicitly not implemented
  (`pkg/upstream/bootstrap/bootstrap.go:239-247`,
  `pkg/upstream/upstream.go:103-110`).
- The user approved preserving that single-family contract for this foundation
  on 2026-09-17. Native dual-stack resolution and address racing remain a known
  gap that must be implemented in a dedicated later task; the Rust API must not
  make that extension needlessly breaking.
- Existing Rust transports consume one caller-owned absolute deadline and
  distinguish caller cancellation, owner shutdown, and side-effect state. A
  resolver must not reset or extend that budget before dialing.
- The secure-upstream task explicitly deferred resolver/bootstrap, connection
  pooling/reuse, socket policy, QUIC/HTTP3, listeners, and host composition.

## Requirements

### R1 — Pure Rust resolution boundary

- Introduce a resolver abstraction and owned resolution state in the pure Rust
  upstream layer, with no Go/cgo/FFI edge and no internally created runtime.
- Numeric dial addresses bypass DNS resolution and remain immediately usable.
- Hostname parsing/validation and resolution errors are typed and observable;
  malformed names, zero ports, empty answers, wrong record families, malformed
  DNS replies, and terminal DNS rcodes must not silently become dial attempts.

### R2 — Bootstrap configuration compatibility

- Accept the existing product-level meanings of `dial_addr`, `bootstrap`, and
  `bootstrap_version` without changing TLS SNI or HTTP authority.
- Bootstrap transport is a numeric UDP endpoint with default port 53. No
  bootstrap hostname recursion is allowed.
- Preserve the current single-family semantics: `0` and `4` select A/IPv4;
  `6` selects AAAA/IPv6. No additional user-visible config key is introduced in
  this foundation.

### R3 — Deadline, cancellation, and shutdown

- Resolution, any bootstrap retransmission, refresh publication, and the later
  dial share the caller's original absolute deadline. Resolution must not grant
  the transport a fresh timeout.
- Owner shutdown and caller cancellation interrupt in-flight resolution and
  waiting callers promptly and deterministically. Close prevents new work,
  drains owned work, and is idempotent.
- A query must never be sent to the target upstream before a valid numeric dial
  destination has been published. Bootstrap traffic is the only allowed network
  side effect during resolution.

### R4 — Cache, refresh, and publication

- Publish only complete, validated resolution results. Concurrent callers for
  the same unresolved/expired name share bounded resolution work rather than
  creating an unbounded query fan-out.
- Derive freshness from accepted DNS TTLs under explicit reviewed lower/upper
  bounds; tests use an injected clock or deterministic time control rather than
  sleeps.
- A failed refresh must never replace a previously valid published result.
  An expired last-known-good result is retained only as diagnostic evidence and
  must not be returned as success. A later caller may retry resolution; bounded
  stale serving would require a separately reviewed policy.
- Cache keys include every input that can change the answer contract, at least
  normalized hostname, port, selected address family, and bootstrap endpoint.

### R5 — DNS response validation

- Bootstrap queries use valid DNS wire messages and EDNS(0) within the reviewed
  UDP payload boundary.
- Accept only a response correlated to the outstanding query and expected
  question. Ignore or reject unexpected peer, transaction ID, question, record
  type, and malformed payload according to an explicit error matrix.
- CNAME handling, answer selection, TTL derivation, truncation, and retry policy
  must be specified before implementation; no implicit system-resolver behavior
  may fill these gaps.

### R6 — Composition boundary

- The result composes with current UDP/TCP/DoT/DoH endpoint constructors while
  keeping numeric dialing separate from secure service identity.
- This task may add the minimum adapter needed for those Rust primitives to
  consume a resolved numeric destination, but it does not add connection
  pooling, pipeline/reuse, QUIC/HTTP3, socket policy, listeners, YAML loading,
  host composition, WebUI/API changes, or production/default selection.

### R7 — Evidence and execution discipline

- Implement one observable behavior at a time using RED -> GREEN -> bounded
  refactor. Tests exercise public resolver/upstream interfaces with deterministic
  loopback bootstrap fixtures and no external DNS dependency.
- Preserve Rust 1.85 workspace compatibility and `#![forbid(unsafe_code)]`.
- Local focused/workspace fmt, test, warnings-denied clippy, task validation,
  dependency inspection, and diff checks must pass. Linux loopback evidence is
  required before final task closure.
- Routing for implementation/review: executor is the user-selected Claude
  Herdr pane `w6:p2`; the reviewer/root gate is the user-selected ChatGPT web
  conversation **建立评审上下文** in project **mosdns**:
  <https://chatgpt.com/g/g-p-6a186dc3cda481918bb08784cae25b26-mosdns/c/6aaba177-ce44-83ee-b52a-68b5d84be272>.
  The controller may send future review context and GitHub revisions there
  without another confirmation. Routing state is validated before dispatch and
  review.

## Acceptance Criteria

- [ ] AC1: Numeric destinations bypass resolution; hostname destinations resolve
  through an explicit resolver/bootstrap boundary without changing TLS/HTTP
  identity.
- [ ] AC2: Existing `dial_addr`, numeric `bootstrap[:port]`, and the approved
  `bootstrap_version` family semantics are represented by typed Rust inputs and
  validated before network I/O.
- [ ] AC3: Initial lookup, concurrent lookup sharing, TTL refresh, refresh
  failure, and last-known-good publication are deterministic and covered without
  timing sleeps.
- [ ] AC4: Caller cancellation, owner shutdown, and one absolute deadline are
  proven at before-send, waiting-for-response, publication, and handoff-to-dial
  boundaries; no post-terminal publish or hidden task survives closure.
- [ ] AC5: Bootstrap response validation covers valid A/AAAA, empty/no-data,
  malformed message, wrong peer/ID/question/family, terminal rcode, CNAME policy,
  truncation policy, and TTL-bound behavior.
- [ ] AC6: Focused composition tests prove the resolved numeric address feeds the
  existing UDP/TCP and DoT/DoH dial boundary while service identity remains
  unchanged.
- [ ] AC7: No Go bridge, backend selector/fallback, connection pool, QUIC/HTTP3,
  listener, host/YAML/API/WebUI wiring, deployment, or hybrid retirement is
  introduced.
- [ ] AC8: Required local and Linux gates pass, the exact commit is independently
  checked by the controller, and the selected web reviewer returns explicit
  scoped PASS before the task is closed.

## Out of Scope

- Connection pooling, TCP/DoT pipeline, reuse, generic exchange retry, proxying,
  socket marks/device binding, and interface/source-address policy.
- QUIC, DoQ, HTTP/3, server listeners, and Rust-native host composition.
- YAML/config loader, WebUI/API, metrics/audit integration, production wiring,
  deployment, default switching, and Phase 6 hybrid retirement.
- Replacing or deleting the existing Go path.
- Native dual-stack resolution, address ordering/racing, and family fallback.
  This is a required follow-up rather than a rejected feature; it needs its own
  policy and acceptance task after the single-family foundation is stable.
