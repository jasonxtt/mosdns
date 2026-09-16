# Phase 4 upstream transport architecture

> Planning artifact plus Slice0/Slice1 contract. The architecture and
> compatibility questions were root-reviewed before implementation. Slice0
> passed root review and the user explicitly authorized Slice1 UDP; this
> document does not authorize Slice2+, TCP fallback, or production wiring.

## 1. Boundary and dependency direction

Phase 4 adds a pure Rust library crate named rust/upstream-core. Slice0
registered it as a workspace member; Slice1 is the currently authorized UDP
implementation scope after the Slice0 root-review PASS.

The intended crate relationship is:

    rust/dns-core
       | +       |  rust/sequence-core     (execution and policy)
       |
       +-- rust/upstream-core     (UDP/TCP exchange and composite policy)
                         /
                future Rust host/server
                (one process-owned async runtime)

The host composes sequence-core and upstream-core; upstream-core must not
depend on sequence-core:
sequence-core currently depends only on dns-core, and making the low-level
transport depend upward would create a cycle and would make the transport
unusable by the future server layer. The host or a thin future orchestration
layer will translate sequence cancellation and policy into the
upstream-core API.

The transport crate owns no Go pointer, C handle, cgo callback, pool buffer,
capability bit, selector, or FFI record. It exposes Rust types and futures only
and must use #![forbid(unsafe_code)] in the future crate. Socket-option support
that needs unsafe platform APIs is a separate reviewed scope.
rust/runtime is the existing transitional staticlib/FFI assembly point; it is
not the async owner for Phase 4 and must not become a second transport runtime.

Direct numeric SocketAddr endpoints, plain UDP, plain TCP, DNS wire validation,
absolute deadlines, cancellation, and deterministic close are the first
bounded transport surface. Configuration parsing, hostname resolution,
bootstrap, proxying, socket policy knobs, secure protocols, listeners, and
production wiring remain separate decisions.

## 2. Shared async runtime and lifecycle ownership

The final Rust host will create one Tokio runtime for server listeners,
sequence execution, upstream exchanges, timers, and shutdown. A runtime may be
multi-threaded, but the choice of worker count belongs to the host. Tokio is
the selected model because the current Rust workspace and the audited KixDNS
reference both use Tokio-style asynchronous I/O, and it supplies the required
net, time, task, and cancellation composition without a runtime per upstream.

upstream-core will run on the caller's runtime. It will never call
Runtime::new, block_on, or create a hidden executor. It may use tokio::spawn
for exchange-scoped tasks only when the ownership and join handle are held by
the upstream object. Timers use one absolute deadline per logical exchange.
Socket creation, reads, writes, and task joins are owned by the upstream
object; the host owns the process runtime and top-level shutdown signal.

The boundary is:

- the host creates the runtime and supplies the root shutdown signal;
- the upstream object owns its Open/Closing/Closed state and exchange children;
- an exchange owns its socket or TCP stream, deadline timer, response bytes,
  and cancellation join/cleanup;
- DNS validation is a synchronous pure operation over an owned wire buffer;
- close cancels children, prevents new work, closes owned I/O, waits for
  children, and then returns Closed state;
- no per-upstream runtime, blocking thread, detached task, or timer survives
  the owning exchange or close operation.

### Cancellation primitive

The current sequence-core CancellationToken is a reviewed execution primitive
owned by sequence-core. upstream-core must not import sequence-core merely to
reuse that type. Phase 4 therefore owns a transport-level cloneable token
implemented over a Rust-native cancellation primitive, with a small API for
cancel, is_cancelled, and child/observation semantics. tokio-util's
CancellationToken is the planned implementation dependency, subject to the
Slice 0 dependency review.

The future host will fan out its sequence cancellation into an
upstream-core token at the orchestration boundary. A later task may extract a
shared cancellation abstraction into a lower-level crate only if that crate
has a non-cyclic dependency graph and receives a separate review. Until then,
there is one semantic rule but two deliberately owned layers:

- sequence cancellation stops execution policy and prevents new sequence work;
- transport cancellation terminates socket, timer, and exchange task work.

They are not interchangeable hidden globals. Transport cancellation is checked
before starting a TC fallback or any future retry. If cancellation and a
deadline become ready together, cancellation wins at the API boundary unless a
fully validated response has already been committed.

## 3. Pure Rust API and ownership sketch

The following is a contract sketch, not code to add during planning:

    Endpoint {
        address: SocketAddr,
        transport: Udp | Tcp,
    }

    ExchangeRequest<'q> {
        query: &'q [u8],
    }

    ExchangeContext {
        deadline: Instant,
        cancellation: TransportCancellation,
    }

    ExchangeControl {
        context: ExchangeContext,
        owner_cancellation: TransportCancellation,
    }

    ExchangeResponse {
        wire: Bytes,                 // fully owned response wire
        request_id: u16,
        response_id: u16,
        transport: Udp | Tcp,
        truncated: bool,
    }

    Upstream::exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<ExchangeResponse, UpstreamError>

    Upstream::close(&self) -> impl Future<Output = CloseResult>

The final spelling may use Vec<u8> instead of Bytes, but the returned wire
must be owned by the Rust response and must outlive the exchange call without a
Go pool release. The request is borrowed and read-only. The transport copies
the query into its own outbound frame or datagram before any asynchronous
boundary; it never mutates the caller's bytes, DNS ID, EDNS fields, or
question section.

The public request API does not expose an internal transaction ID because the
first UDP architecture does not rewrite IDs. request_id is read from the
original query and response_id is read from the accepted response. The
response is accepted only when they match. A future shared multiplexer could
hide internal demultiplexing, but it would need a separate contract review and
could not change the caller-visible ID.

ExchangeContext carries one absolute deadline, not separate timeout values for
UDP and TCP. A composite UDP/TCP policy passes the same deadline into the TCP
fallback. The context also carries cancellation; it does not carry Go
context.Context, a pointer, or a C handle.

A prepared exchange retains caller cancellation and owner shutdown as separate
tokens in `ExchangeControl`. Owner shutdown is checked before caller
cancellation and reports `Closed`; caller cancellation reports `Cancelled`.
Both token types expose an async cancellation wake primitive for I/O selection.
The crate does not create a Tokio runtime. Slice1's exchange runs on the
caller's runtime and registers an in-flight guard before any socket ownership.

The API must make close observable. New exchange calls after Closing begins
return Closed. An in-flight call terminated by owner close returns Closed with
its side-effect state; an explicit caller cancellation returns Cancelled. The
error contract below defines the distinction.

The owner close operation is awaitable once real I/O exists. It changes
`Open -> Closing`, rejects new registrations, cancels the owner scope, awaits
the in-flight registration count reaching zero, and only then performs the
guarded `Closing -> Closed` transition. The registration gate serializes
admission with close, and the guard releases on success, every error, or a
dropped exchange future. A direct `finish_close` while registrations remain
returns `InFlight` and cannot expose `Closed` early. No lock is held across the
drain await, and no exchange task is spawned by upstream-core.

## 4. DNS validation boundary

upstream-core reuses mosdns-dns-core for DNS message and response wire
semantics. It must not add a second RR parser, OPT parser, TTL walker, or DNS
message parser. The relevant existing boundary is parse_query for request
validation and validate_response for complete response validation.

dns-core also provides the existing outbound frame_response(Stream) helper for
writing a two-byte DNS-over-TCP length prefix. It does not currently provide
an inbound TCP prefix/body reader. upstream-core owns that transport-level I/O:
read exactly two bytes, decode the big-endian u16, then read exactly the
declared body length. This is stream framing and lifecycle, not a second
DNS/RR/OPT parser.

Before full response validation, the transport performs a minimal header
inspection that is safe for both complete and TC responses:

1. the wire has at least the twelve-byte DNS header;
2. QR is set;
3. the response ID equals the original request ID;
4. the response is associated with the expected transport/peer;
5. the TC bit is observable before attempting a full RR walk.

A non-truncated accepted response is then passed to dns-core's complete
validation. A TC response is not forced through a complete-RR validator that
would reject the intentionally truncated answer; its header is checked, it is
returned as a transport result with truncated=true, and the composite policy
decides whether to perform TCP fallback. The fallback response is fully
validated.

If dns-core does not expose a small header-only inspection helper, Slice 0
must add the smallest reviewed extension to dns-core. It must not duplicate
wire parsing inside upstream-core. The extension may report QR, TXID, TC,
header-length, and basic malformed-header conditions only; RR and OPT
semantics remain in dns-core.

The minimum validation failures are typed as malformed or mismatch errors.
TCP framing is validated before the resulting DNS wire is passed to dns-core:
a zero-length inbound frame is rejected, an outbound query over u16::MAX is
rejected before send, a partial prefix/body is a truncated frame, and stream
chunks are never treated as separate DNS messages.

## 5. UDP architecture: one exchange, one socket

The first UDP implementation deliberately chooses a per-exchange socket rather
than a shared socket plus transaction map. This is the bounded choice for the
initial Rust-native foundation:

- each exchange binds one ephemeral local socket in the endpoint's address
  family, sends one datagram, and owns all receives for that datagram;
- the query ID is not rewritten and no internal QID allocator or demultiplexer
  is needed;
- concurrent exchanges are isolated by their sockets and response ownership;
- no pool, receiver task, DashMap, or background demux loop is introduced in
  the first slice;
- the design leaves a future shared multiplexer possible, but that is not a
  silent optimization and would require a new lifecycle, ID, peer, and close
  review.

The socket lifecycle is:

1. validate query and numeric endpoint before binding;
2. bind an ephemeral local address in the endpoint family;
3. allocate a legal full-datagram receive buffer, up to the DNS wire maximum,
   rather than inheriting Go's 4095-byte implementation buffer;
4. send the unchanged-ID query to the expected endpoint;
5. receive datagrams until a valid matching response, cancellation, or the
   absolute deadline;
6. validate peer and response ID before accepting bytes;
7. close/drop the socket on every terminal path.

The receive source must equal the configured endpoint address, including the
port. A datagram from another peer is ignored while the exchange remains
within its deadline. A datagram from the expected peer with the wrong ID is
also ignored, with a diagnostic mismatch retained for the terminal error if no
valid response arrives. This avoids accepting stale or spoofed data without
creating a second request. A malformed datagram from the expected peer is a
terminal MalformedResponse once its header can be attributed to this exchange;
an undersized header is handled the same way. A valid TC header is a
successful UDP observation, not a malformed response.

A non-cancellation recv_from failure is terminal Receive/Read with Sent or
MaybeSent state, closes the socket, and does not trigger a duplicate send.

When an ignored wrong-peer or wrong-ID datagram has been observed, a terminal
deadline, caller cancellation, owner close, or receive failure retains a small
structured diagnostic alongside the truthful primary cause. It does not turn a
timeout into `ResponseMismatch`, and a later valid response succeeds without
carrying the diagnostic.

A late or duplicate datagram cannot be delivered after the exchange returns:
the socket owner is dropped and the kernel discards subsequent traffic for
that ephemeral socket. Cancellation drops the receive future and socket,
cancels the deadline timer, and joins any exchange task before the API returns.
There is no background receiver to leak.

The first implementation has no automatic UDP retransmit. A later retry policy
may add protocol-reviewed retransmission, but it must carry the same absolute
deadline, distinguish a new datagram send from a sequence rerun, and be
classified against the product contract first. The current Go one-second
retransmit and ten-second internal ceiling are not copied merely for
implementation parity.

The first receive buffer is sized for a complete legal DNS UDP wire. An
oversized or OS-truncated datagram must not be silently accepted as a complete
response. If a platform exposes a truncation indicator, the transport reports
a malformed/truncated receive according to the reviewed error contract; if the
datagram fits the maximum buffer, dns-core decides DNS-level validity. EDNS
advertised size is part of the query wire and is not rewritten by transport.

## 6. TC to TCP composite policy

UDP exchange is a primitive. TC fallback belongs to a higher-level composite
policy, called UdpTcpPolicy in this plan, so a caller that wants UDP-only can
use the primitive without hidden protocol work.

For an enabled UdpTcpPolicy:

1. validate the query once and record its original wire and ID;
2. start one absolute deadline and one transport cancellation scope;
3. perform the UDP exchange;
4. if the response is complete, return it;
5. if the response has a valid TC=1 header, check cancellation and remaining
   deadline;
6. connect to the same upstream over TCP and send the same query bytes with
   the same original ID;
7. fully validate the TCP response and return it;
8. report the TCP error if fallback fails, retaining the UDP/TC observation in
   structured error context.

The fallback is a protocol-approved second transport attempt, not a second
sequence execution. There is no Go fallback and no hidden re-entry into a
resolver. Cancellation observed after UDP and before TCP prevents TCP work.
The TCP operation receives the same absolute deadline; it does not receive a
fresh timeout. A TC response is not itself returned as the final answer when
the configured policy requires TCP.

## 7. TCP framing and connection lifecycle

The first TCP primitive opens one new TCP connection per exchange. It does not
pool, pipeline, or reuse connections. This is an intentional bounded
implementation choice: it makes ownership, cancellation, close, response
matching, and recovery unambiguous while the product contract for Go
connection reuse is still being separated from its internal optimization.
The Go reuse/pipeline behavior is recorded as implementation-only/defer in the
matrix unless a later product audit proves a user-visible contract.

Every DNS-over-TCP message is encoded as exactly:

    two-byte unsigned big-endian payload length
    payload bytes

The payload length must be non-zero and fit in u16. An outbound query larger
than u16::MAX is FrameTooLarge before socket send; the two-byte inbound prefix
can never encode a value larger than u16::MAX. The request payload is
validated before framing. The implementation must use full write semantics:
partial writes are progress, not message boundaries. A write failure closes
the connection and returns Send/Write with side-effect state; it never leaves
a possibly poisoned stream for reuse.

The reader first reads exactly two prefix bytes. EOF before both bytes is
TruncatedFrame. A zero length is a malformed frame. There is no smaller
configured inbound maximum in the first slice: every non-zero two-byte prefix
is at most u16::MAX. The reader then reads exactly the declared payload
length; EOF after only part of the body is TruncatedFrame.
Only the complete body is handed to the DNS validation boundary. Stream
chunks, read calls, and prefix/body boundaries are never treated as separate
responses.

Connect, prefix write, body write, prefix read, body read, validation, and
close all observe the same absolute deadline and transport cancellation. If
the response ID does not match the request ID, the response is rejected as
ResponseMismatch and the connection is closed. A valid peer is inherent in
the connected TCP stream, but a connect result must still retain the resolved
peer for diagnostics and future endpoint-policy checks.

A later pooling design would need a separate reviewed contract covering
checkout, return, in-flight ownership, one-response-at-a-time versus
pipelining, idle timers, poisoned-connection eviction, cancellation of pending
requests, response-ID demultiplexing, and Close. No phrase such as
implementation decides is sufficient for that future scope.

## 8. Timeout, cancellation, and error taxonomy

Timeout and cancellation are distinct:

- DeadlineExceeded means the absolute deadline won while the operation was
  still otherwise active. It carries whether a query may have been sent.
- Cancelled means the caller or parent transport token requested termination.
  It wins ties with the deadline and prevents new fallback/retry work.
- Closed means the owner entered Closing/Closed and terminated the exchange.
- A network or validation failure reports its own category and side-effect
  state; it is not relabeled as a timeout.

The planned typed error taxonomy is:

| Error | Meaning | Side-effect information |
| --- | --- | --- |
| InvalidRequest | Query is empty, malformed, or cannot be framed | NotSent |
| InvalidEndpoint | Endpoint is unsupported or not numeric in Phase 4 | NotSent |
| Cancelled | Caller transport cancellation won | NotSent, MaybeSent, or Sent |
| DeadlineExceeded | Absolute deadline won | NotSent, MaybeSent, or Sent |
| Connect | TCP connect or local socket setup failure; no DNS payload has been sent | NotSent |
| Send/Write | UDP send or TCP write failure | MaybeSent or Sent |
| Receive/Read | Non-framing receive/read failure | Sent or MaybeSent |
| MalformedResponse | Header/RR/wire validation failed | Sent |
| UnexpectedPeer | A terminal peer-policy failure was selected | Sent |
| ResponseMismatch | Expected response ID or request association failed | Sent |
| TruncatedFrame | TCP prefix/body ended before completion | Sent |
| FrameTooLarge | Outbound query exceeds u16::MAX before send | NotSent |
| Closed | Owner close terminated or rejected work | NotSent, MaybeSent, or Sent |
| Runtime/Internal | Invariant, join, or runtime failure | Last tracked state: NotSent, MaybeSent, or Sent |
| Diagnosed | Deadline/cancellation/close/receive terminal with ignored UDP context | Primary cause state: NotSent, MaybeSent, or Sent |

SideEffectState is a closed three-state enum: NotSent, MaybeSent, or Sent.
Runtime/Internal preserves the last tracked state rather than introducing an
Unknown state. Before any send/write attempt the state is NotSent; once a
send/write begins it is at least MaybeSent; after a confirmed complete send it
is Sent. A failed write cannot always prove whether the kernel accepted bytes,
so it must not be treated as safely retryable. The concrete Rust error should
include transport, endpoint, phase, and this side-effect marker. Error
equality and user-facing text are not compatibility contracts; structured
classification is.

## 9. Side-effect and retry matrix

No automatic retry is allowed solely because an operation failed after a
request could have crossed the network. Only TC-to-TCP is approved in the
initial policy. The matrix is normative for the first implementation:

| Event | Side effect | Required result/action | Automatic retry |
| --- | --- | --- | --- |
| Invalid query or unsupported endpoint | NotSent | InvalidRequest or InvalidEndpoint | No |
| Local bind/setup failure | NotSent | Connect | No |
| UDP send fails before completion | MaybeSent | Send/Write with uncertainty | No |
| UDP send completes | Sent | Continue receive | No duplicate send |
| UDP deadline after send | Sent | DeadlineExceeded with Sent | No |
| UDP cancellation before send | NotSent | Cancelled | No |
| UDP cancellation after send | Sent | Cancelled with Sent | No |
| Wrong UDP peer | Sent | Ignore until valid response/deadline; retain diagnostic | No |
| Wrong UDP ID | Sent | Ignore until valid response/deadline; retain mismatch | No |
| Malformed expected-peer UDP response | Sent | MalformedResponse | No |
| Valid UDP TC response | Sent | Return TC observation to composite policy | TCP fallback only |
| TCP connect fails | NotSent for TCP frame | Connect, preserving prior TC context | No |
| TCP prefix/body write is partial then fails | MaybeSent or Sent | Send/Write, close connection | No |
| TCP write completes | Sent | Read one complete framed response | No |
| TCP read timeout after full write | Sent | DeadlineExceeded with Sent | No |
| TCP EOF before full prefix/body | Sent | TruncatedFrame, close connection | No |
| TCP inbound frame has zero length | Sent | MalformedResponse, close connection | No |
| TCP outbound query exceeds u16::MAX | NotSent | FrameTooLarge before connect/send | No |
| TCP response ID mismatches | Sent | ResponseMismatch, close connection | No |
| Cancellation during TCP connect/write/read | NotSent, MaybeSent, or Sent | Cancelled, close connection | No |
| Owner Close during any phase | Recorded state | Closed, cancel and await registration drain | No |

A future UDP retransmission policy would be an explicit additional row and
would require evidence that the product contract needs it. It cannot be
smuggled in as a generic retry loop.

## 10. Close state machine

Each upstream object has the states Open, Closing, and Closed.

    Open -> Closing -> Closed

Open accepts new exchanges. Close atomically changes Open to Closing, rejects
new registrations, cancels the upstream cancellation scope, and asks every
in-flight exchange to stop. Slice1 exchanges run in their caller tasks, so the
owner awaits registration guards reaching zero rather than joining detached
exchange tasks. The guard covers UDP sockets, timers, response ownership, and
the TCP placeholder until the exchange future returns or is dropped. Once the
in-flight count is empty, it enters Closed. Repeated close calls are
idempotent and await the same completion state.

The pre-I/O Slice0 contract still exposes `finish_close` for direct lifecycle
inspection, but it can transition only `Closing -> Closed` after the count is
zero; calling it while `Open` returns `NotClosing`, and calling it while work
is registered returns `InFlight`. Slice1's `close().await` is the operation
that performs cancellation and drain without blocking or creating a runtime.

An exchange observes close before starting bind/connect/send, while awaiting
receive/read, and before committing a response. A response that has already
passed DNS validation before close wins and is returned; otherwise owner close
returns Closed with the recorded side-effect state. No detached receiver,
timer, stream, pool entry, pending-map entry, or task may survive Closed.

Although the first implementation has no pool or shared receiver, this state
machine intentionally names those resources because a future pooled transport
must evict every pending connection and request during Closing rather than
invent a second shutdown path.

## 11. Compatibility and deviation matrix

The following is the required pre-implementation matrix. Evidence names are
repo-relative and refer to the current Go behavior-discovery source, existing
Rust contracts, or the pinned KixDNS audit. Tests are planned evidence; they
are not yet implementation.

| Behavior | Evidence | Classification | Rust contract | Test | Characterization |
| --- | --- | --- | --- | --- | --- |
| Query bytes are not mutated | pkg/upstream/transport/transport.go ReservedExchanger contract; upstream tests | Preserve | Borrowed read-only query; owned outbound copy | UDP/TCP input snapshot | Product-visible caller ownership |
| Original DNS ID is preserved | pkg/upstream/transport/transport.go and reuse.go | Preserve | No internal ID rewrite; accepted response ID equals request ID | ID preservation and mismatch tests | Product/protocol |
| QR and complete response validation | rust/dns-core validate_response; Go upstream tests | Preserve | QR required; complete non-TC wire goes through dns-core | QR/RR/malformed tests | Product/protocol |
| Concurrent response isolation | pipeline.go, reuse.go, pipeline_test.go | Preserve | Per-exchange UDP sockets and per-connection TCP ownership isolate responses | Concurrent UDP/TCP exchanges | Product/protocol |
| Expected UDP peer/source | Go net transport behavior; KixDNS source check | Preserve | Only configured address/port may be accepted | Wrong-peer injection test | Protocol safety |
| Internal UDP QID rewrite/demux | KixDNS transport.rs; Go pipeline internals | Intentional Rust deviation | No rewrite or shared demux in first architecture | Original-ID and concurrent isolation tests | Implementation replacement |
| UDP retransmit timing/count | Go transport timeout/retry code and tests | Implementation-only/defer | No automatic retransmit in first slice; later policy needs review | No accidental duplicate-send test | Not yet proven product contract |
| Timeout/deadline semantics | Go upstream/context tests | Preserve | One absolute deadline; typed DeadlineExceeded with side effect | Pre/post-send deadline tests | Product-visible failure semantics |
| Cancellation | Go upstream/context tests; sequence-core token | Preserve | Transport token stops I/O and prevents fallback/retry | Cancel before/after send and during TCP | Product-visible lifecycle |
| UDP receive size and EDNS | Go 4095-byte pool buffer; DNS wire/EDNS behavior | Intentional Rust deviation | Receive legal full wire; never silently truncate; do not rewrite EDNS | Large EDNS/TC/truncation tests | Go buffer is implementation detail |
| UDP TC to TCP | pkg/upstream/upstream.go forward path and tests | Preserve | Composite policy uses same query and absolute deadline | TC fallback and cancellation-before-fallback | Product/protocol |
| TCP two-byte framing | rust/dns-core outbound framing helper; Go upstream tests | Preserve | Big-endian non-zero u16 prefix; upstream-core owns exact inbound read/write | Prefix, partial I/O, zero inbound, outbound oversize | Protocol |
| TCP connection reuse | pkg/upstream/transport/reuse.go | Implementation-only/defer | New connection per exchange in first slice | Recovery/close tests for non-reused streams | Go resource optimization until audited |
| TCP pipelining and pending demux | pkg/upstream/transport/pipeline.go | Implementation-only/defer | No pipeline in first slice; one request per connection | Concurrency without shared stream | Go implementation, not yet product contract |
| Idle timeout and recovery | reuse.go idle close/retry paths | Implementation-only/defer | No idle pool; connection closes at exchange end | Close-after-exchange and error cleanup | Depends on future pool scope |
| Post-send retry/re-execution | reuse.go/pipeline.go retry paths | Intentional Rust deviation | No arbitrary retry; only reviewed TC fallback | Side-effect matrix tests | Safety policy |
| Malformed/undersized response policy | Go tests plus dns-core validation differences | Intentional Rust deviation | Typed deterministic malformed policy; TC header inspected separately | Malformed UDP/TCP cases | Rust contract needs explicit coverage |
| Deterministic Close | transport.go Close contract; reuse.go close | Preserve | Open->Closing->Closed, idempotent, reaps all children | New/in-flight/close races | Product-visible lifecycle |
| Rust byte/pool ownership | Go pkg/pool; current Rust owned types | Intentional Rust deviation | Bytes/Vec owned response, no Go pool release | Ownership/drop and repeated exchange tests | Required native boundary |
| SoMark and BindToDevice | pkg/upstream/upstream.go options | Research unresolved | Not in first direct-endpoint API; product/config audit required | Characterization before scope | Configuration meaning not frozen |
| Local bind address/interface | pkg/upstream/upstream.go DialAddr/options | Research unresolved | No local-bind promise in first slice; must audit user contract | Characterization before implementation | May require socket abstraction |
| SOCKS5 proxy | pkg/upstream/upstream.go options | Research unresolved | Out of first scope; no silent support claim | Configuration/use audit | Separate transport policy |
| Hostname/bootstrap resolution | pkg/upstream/upstream.go Bootstrap options | Research unresolved | Numeric endpoint only; resolution/bootstrap separate task | Config characterization | Control-plane ownership not moved |
| TLS/HTTPS/DoH/DoT | migration plan and current Go schemes | Implementation-only/defer | Explicitly outside Phase 4 UDP/TCP foundation | None in this task | Later protocol phase |
| QUIC/HTTP3/DoQ | migration plan and current Go schemes | Implementation-only/defer | Explicitly outside Phase 4 | None in this task | Later protocol phase |
| Server listeners/config/WebUI/API/metrics | rust-rewrite-plan and AGENTS scope | Implementation-only/defer | No production/control-plane wiring | No production wiring diff | Later Rust-native host work |

Any row classified Research unresolved blocks the related implementation scope.
A later root review may move a row to Preserve or Intentional Rust deviation
only with source/config evidence and a focused characterization test.

## 12. Explicit scope

In scope for the first future implementation:

- direct numeric IPv4/IPv6 UDP exchange;
- direct plain TCP exchange with exact DNS framing;
- dns-core-backed query and response validation;
- response-ID and peer safety;
- one absolute deadline and Rust-native cancellation;
- typed side-effect-aware errors;
- optional composite UDP TC to TCP fallback;
- deterministic close and resource cleanup;
- focused tests and isolated network verification.

Implementation-only or research-gated, not silently included:

- SoMark, BindToDevice, local bind address/interface policy;
- SOCKS5 and any proxy abstraction;
- hostname resolution, bootstrap resolver, and configuration migration;
- pooled TCP reuse, pipelining, idle eviction, and connection recovery;
- automatic UDP retransmission or generic post-send retries;
- TLS certificate/config policy and secure transports.

Explicitly out of scope for this task:

- TLS, HTTPS, DoH, DoT;
- QUIC, HTTP/3, DoQ;
- server listeners and inbound protocol handling;
- Go production listeners, EntryHandler, NewUpstream, sequence execution,
  selectors, cgo/ABI, Go fallback, and Go pool ownership;
- config parser migration, WebUI/API changes, metrics format changes,
  deployment, release defaults, and production cutover.

## 13. Planning gate

This architecture is complete only when the root reviewer accepts the decisions,
the compatibility matrix, the KixDNS ledger, and the implementation slices.
The planning gate and Slice0 gate are closed; the user explicitly authorized
Slice1 UDP in the current `in_progress` task. Slice1 must stop for another root
review before Slice2, and nothing in this document authorizes production wiring
or a release.
