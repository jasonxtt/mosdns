# Design — Rust Phase 4 endpoint resolution foundation

## 1. Boundary and crate ownership

The task stays inside the pure Rust workspace:

- `mosdns-dns-core` owns pure bootstrap DNS query encoding and response
  decoding/validation. It gains no I/O, timers, cache, runtime, or config.
- `mosdns-upstream-core` owns the numeric bootstrap UDP exchange, resolution
  publication, lifecycle, and composition with existing transport endpoints.
- The future host will map YAML values into these typed inputs. No Go adapter,
  C ABI, selector, fallback, YAML loader, WebUI/API, or production path is added.

Likely files are new focused modules such as `dns-core/src/resolver.rs` and
`upstream-core/src/resolver.rs`, plus public re-exports and contract tests. Reuse
existing `ExchangeContext`, `TransportCancellation`, `Lifecycle`, numeric
`Endpoint`, and `ServerIdentity`; do not duplicate their state machines.

## 2. Public model

- `AddressFamily::{Ipv4, Ipv6}` is explicit. Host mapping preserves config:
  `bootstrap_version` 0/4 -> IPv4, 6 -> IPv6. The enum is intentionally not a
  boolean so a later dual-stack policy can be additive.
- `ResolutionTarget` holds a validated normalized DNS name, nonzero port, and
  family. IP literals bypass the resolver and construct the numeric endpoint
  directly.
- `BootstrapEndpoint` wraps a validated numeric UDP `SocketAddr`; hostname
  bootstrap values are rejected before I/O.
- `ResolutionPolicy` explicitly carries minimum/maximum positive TTL and the
  one-second retransmit interval. Defaults are 5 minutes and 7 days. Invalid
  bounds fail construction.
- `ResolvedDestination` owns the numeric `SocketAddr`, selected family, observed
  TTL, and expiry metadata. The type is compatible with returning multiple
  candidates later without changing service identity.
- `BootstrapResolver` is configured for one target/bootstrap tuple. Per-owner
  state avoids a global cache and makes the tuple itself the cache key.

`ServerIdentity` remains the secure identity source. A DNS target may reuse its
existing normalized `dns_name()` result; a resolved address never overwrites or
reconstructs the identity or DoH URL authority.

## 3. Pure DNS wire contract

Query builder:

1. Validate/encode one normalized FQDN question, class IN, selected A/AAAA type.
2. Obtain a two-byte unpredictable ID through an injected ID source; production
   uses exact `getrandom 0.4.3`, tests use deterministic IDs.
3. Set RD and append one root-owned OPT record advertising 1200-byte UDP payload;
   no ECS, DO bit, padding, or user query mutation.

Response parsing performs one bounded walk and returns a typed result only when:

- QR/opcode, peer, ID, one exact question name/type/class, declared counts, and
  all record bounds are valid;
- RCODE is NOERROR and TC is clear;
- an address RR of the requested family belongs to the QNAME or a bounded,
  loop-free in-message CNAME chain rooted at it;
- address size is exact and every used TTL is available.

The maximum accepted CNAME chain is eight links. The first matching address in
wire order wins. Effective TTL is the minimum over the used CNAME links and the
chosen address RR, then clamped to policy. Only-CNAME, wrong-family, empty,
NXDOMAIN/NODATA, SERVFAIL, REFUSED, malformed, wrong-question, and truncated
responses are typed failures. Negative results are not cached. Trailing bytes
and unrelated answers do not create success.

## 4. UDP exchange and control flow

One leader binds a fresh ephemeral UDP socket in the bootstrap endpoint's IP
family and connects it to the numeric bootstrap peer. It sends the same encoded
query immediately and then retransmits at the policy interval until one valid
reply, caller cancellation, owner shutdown, or the caller's original absolute
deadline wins. It never creates a private five-second budget or resets the
deadline before target dialing.

Datagrams with the wrong ID/question are ignored as diagnostics while budget
remains; malformed/terminal matching responses fail deterministically. Because
the socket is connected, only the configured peer is accepted. A send moves the
bootstrap operation's side effect to `Sent`; it does not claim the final target
query was sent.

The resolver runs entirely on the caller's Tokio runtime. No detached refresh,
`spawn_blocking`, second runtime, system resolver, or external DNS is used.

## 5. Cache, single-flight, and lifecycle

State under one short synchronous mutex contains an optional immutable positive
entry plus expiry, at most one refresh generation and a `Notify` for waiters,
and the last non-secret typed diagnostic for the current generation. Existing
`Lifecycle` owns admission and drain.

Algorithm:

1. Reject new calls atomically after close begins.
2. Return a fresh positive entry immediately.
3. If absent/expired and no refresh exists, the caller becomes leader.
4. Other callers wait on that generation while independently observing their
   own cancellation/deadline and owner shutdown.
5. Leader validates a complete result, then commits it at one linearization
   point only if owner/caller/deadline still permit success.
6. Failure clears the generation and wakes waiters but never overwrites the old
   entry. Expired old entries are retained only as evidence and never served.
7. Dropping/aborting the leader clears or transfers generation ownership through
   an RAII guard, so waiters cannot deadlock. A later caller can retry.
8. Close cancels owned work, prevents publication, wakes waiters, drains all
   registered work, and is idempotent.

There is no background refresh or negative cache. This keeps task ownership
bounded and makes every network action attributable to a caller.

## 6. Composition

Provide the minimum explicit composition helpers needed to turn a
`ResolvedDestination` into plain UDP/TCP `Endpoint`, `DotEndpoint` with the
original `ServerIdentity`, or `DohEndpoint` with the original service URL and
authority. Resolution plus exchange receives one original `ExchangeContext`;
helper code passes its unchanged absolute deadline onward.

## 7. Error and behavior matrix

| Event | Result / publication |
| --- | --- |
| Numeric target | immediate numeric destination; no DNS socket |
| Valid A/AAAA | publish requested family, clamped TTL |
| Valid CNAME chain + address | publish address; TTL min across used chain |
| CNAME loop, >8 links, only CNAME | typed invalid/no-data; publish nothing |
| NXDOMAIN/NODATA/SERVFAIL/REFUSED | typed failure; no negative cache |
| TC=1 | typed truncated failure; no hidden TCP fallback |
| Wrong peer/ID/question/family | ignore or typed mismatch; never publish |
| Caller deadline/cancel | caller-specific terminal result; no late publish |
| Owner close | `Closed`, wake/drain all work, no later admission/publication |
| Concurrent same target | one UDP resolution leader, bounded waiters |
| Refresh failure | old entry preserved internally; expired value not served |

## 8. Compatibility and explicit deviations

Preserved: config meanings for `dial_addr`, numeric bootstrap and default port,
single-family version 0/4/6, five-minute positive TTL floor, service identity
separation, and positive last-result publication.

Intentional Rust deviations: caller absolute deadline replaces a detached
five-second query; exact question/CNAME/peer validation replaces accepting the
first unrelated address RR; no unbounded stale serving; no negative cache; and
TC fails explicitly rather than consuming a partial response.

Deferred but mandatory: native dual-stack resolution/address racing. Also
deferred: TCP bootstrap fallback, pooling/reuse/pipeline, socket policy/proxy,
QUIC/HTTP3, listeners, host/config/API/WebUI wiring, metrics/audit, deployment,
and hybrid retirement.

## 9. Rollback and evidence

All new behavior is unreachable from the Go/default runtime. Rollback removes
only the new Rust modules/re-exports/dependency and their tests; existing numeric
transports remain intact. Each slice stops for controller verification and the
selected ChatGPT web reviewer conversation **建立评审上下文** in project
**mosdns**. Final closure requires Linux loopback evidence and an explicit
scoped PASS.
