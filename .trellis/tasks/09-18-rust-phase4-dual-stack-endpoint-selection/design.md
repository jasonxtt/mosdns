# Design — Rust Phase 4 native dual-stack endpoint selection

Status: planning only. This document authorizes no implementation until the
final planning summary is explicitly approved and `task.py start` is run.

## Boundary and compatibility

The deliverable is a pure Rust resolver foundation in `rust/upstream-core`,
using the existing `mosdns-dns-core` wire codec and upstream lifecycle. It does
not modify the Go resolver, YAML/config loader, API/WebUI, host/plugin wiring,
selectors/fallbacks, production defaults, QUIC/HTTP3 implementations, pools, or
listeners.

The effective configuration mapping is explicit and must distinguish omission
from an integer zero:

| Input | Effective mode | DNS families | Selection |
|---|---|---|---|
| omitted / `None` | IPv4 default | A only | IPv4 |
| explicit `4` | IPv4 | A only | IPv4 |
| explicit `6` | IPv6 | AAAA only | IPv6 |
| explicit `0` | IPv4-preferred dual | A and AAAA | A when usable, otherwise AAAA |

Unknown values remain typed validation errors. Existing single-family
`ResolutionTarget::new(..., AddressFamily)` and numeric-address fast paths stay
available for current callers; the dual mode is an additive plan/constructor
boundary rather than a silent reinterpretation of an existing target.

## Model

1. Keep `AddressFamily` as the wire-level single-family enum. Add a resolver
   mode/plan that can represent `Ipv4`, `Ipv6`, or `PreferIpv4Dual`; do not make
   `AddressFamily` itself pretend to contain a third wire family.
2. Add an effective-version helper such as `ConfigVersion::from_optional` and a
   `Default` implementation that yields IPv4/`4`. `from_u8(0)` represents the
   explicit dual mode, while the existing `4` and `6` paths retain one-family
   semantics. Keep the old single-family constructor as a compatibility seam.
3. Represent one resolution generation as a candidate snapshot containing the
   target identity, at most the selected wire-codec candidate for each family,
   each candidate's TTL/expiry and generation metadata, and per-family typed
   diagnostics. The snapshot is replaced atomically under the resolver state
   lock. Its shape is multi-family even though the current wire codec retains
   one deterministic selected address per query; retaining every same-family RR
   is deferred unless the implementation proves it necessary for this contract.
4. Continue returning one selected `PublishedTarget` to
   `ResolverComposition`, so existing UDP/TCP/DoT/DoH constructors do not need
   a multi-connection API. Expose a read-only candidate snapshot/diagnostic view
   for future transport work; no external caller can mutate resolver state.

## Resolution flow

### Numeric target

Detect a numeric literal before any ID probe or bootstrap exchange. Publish one
timeless candidate using its actual address family. The configured mode does not
cause DNS traffic for a numeric target.

### Single-family mode (`4`, `6`, or omitted)

Reuse the existing one-family `bootstrap::exchange` contract and publication
linearization. A response is converted into the family-specific candidate and
handed to the existing composition boundary.

### Explicit dual mode (`0`)

Create two independent bootstrap lookup futures, one for A and one for AAAA,
under the same caller-owned absolute deadline, caller cancellation, and owner
close scope. The two DNS lookups may be polled concurrently to avoid making one
family wait behind the other, but this is not endpoint racing: no target TCP,
TLS, UDP, DoH, or QUIC connection is opened by the resolver.

Each leg retains the existing wire contract: its own unpredictable transaction
ID, connected ephemeral UDP socket, exact question/ID correlation, typed TC and
rcode errors, no TCP fallback, and no hidden runtime. A successful leg contributes
its family candidate; a failed leg contributes a typed per-family diagnostic.

After both legs reach a terminal outcome or the shared controls terminate the
generation, selection is deterministic:

1. discard expired candidates;
2. select the fresh IPv4 candidate when one exists;
3. otherwise select the fresh IPv6 candidate;
4. if neither family has a candidate, return a typed aggregate/no-usable result
   and preserve the per-family diagnostics.

The resolver never retries AAAA because a selected A connection failed, and it
never opens a second target connection. Connection errors belong to the caller's
transport policy, which is outside this task.

## Publication, refresh, and lifecycle

- One single-flight generation covers both family legs, so waiters observe the
  exact same completed snapshot rather than independently choosing different
  generations.
- A complete snapshot replaces the prior snapshot atomically. A successful
  family refresh replaces only that family's candidate; a failed family refresh
  records its error and preserves a still-fresh prior candidate for that family.
- Expired candidates are retained only as diagnostics and are never selected.
  If one family is fresh and the other has failed or expired, the fresh family
  can still satisfy explicit dual-mode resolution.
- Close, caller cancellation, and the original deadline are checked before
  both query legs and again at the final publication gate. No late leg may
  publish after the generation has been cancelled or superseded.
- The public state handle remains observation-only. Mutation stays inside the
  resolver owner and its internal state-model tests.

## Composition and secure identity

The selected `PublishedTarget` continues to provide only a numeric dial
address plus the original target port. `ResolverComposition` passes that
address to plain UDP/TCP, DoT, and DoH constructors while keeping the original
DoT SNI and DoH URL authority/path. The design records a future QUIC/HTTP3
consumer boundary but adds no QUIC/HTTP3 code, connection reuse, or protocol
fallback.

## Error behavior

- `None`/`4` with no A -> the existing typed no-address/family error.
- `6` with no AAAA -> the existing typed no-address/family error.
- Explicit `0` with A success and AAAA failure -> select A and expose the AAAA
  failure diagnostically.
- Explicit `0` with A failure and AAAA success -> select AAAA and expose the A
  failure diagnostically.
- Explicit `0` with both successful -> select A, regardless of which DNS leg
  was observed first.
- Explicit `0` with neither successful -> return typed aggregate/no-usable
  failure; do not invent an address or call the system resolver.
- Owner close and caller cancellation retain the existing precedence and typed
  errors. A selected-address connection failure is returned by the transport;
  the resolver does not cross-family retry.

## Risks and rollback

- The main API risk is changing the meaning of `ConfigVersion::from_u8(0)` and
  the current single-family `family()` helper. Use additive mode/optional-input
  seams where possible, update contract tests deliberately, and avoid touching
  Go callers.
- A dual snapshot can accidentally serve stale data or publish a late family
  leg. Generation identity, per-family expiry assertions, and a final lifecycle
  gate are required before the result is exposed.
- Concurrent DNS legs can leak a task or keep a waiter alive if ownership is
  split incorrectly. Keep both futures owned by the leader generation and use
  the existing lifecycle guard; do not spawn detached refresh work.
- Rollback is a single Rust commit revert. Because no Go/config/production path
  is changed, the current Go runtime remains the operational fallback outside
  this foundation.
