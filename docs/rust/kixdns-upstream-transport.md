# KixDNS upstream transport audit

Audited upstream: https://github.com/olicesx/kixdns

Pinned commit: 2da3a2d59466e996a0f846c3e7e504970b878b06
(commit date 2026-08-12, fix(geoip): preserve overlapping dat tags #37).

This ledger is for the Phase 4 planning gate. The pinned checkout was audited
read-only. It is not a workspace dependency, is not copied into the repository,
and does not authorize implementation. The existing
docs/rust/kixdns-reuse.md ledger covers cache and matcher work; its one-line
transport deferral cannot substitute for this transport-specific audit.

## Audit method and license boundary

The audit inspected the pinned Cargo manifest and these source areas:

- Cargo.toml for runtime, I/O, byte, map, and socket dependencies;
- src/engine/transport.rs for UDP state, demultiplexing, TCP framing,
  multiplexing, cancellation, and connection lifecycle;
- src/engine/upstream.rs for UDP timeout/retry and TC-to-TCP policy;
- src/config.rs for timeout, pool, TCP fallback, and endpoint options;
- src/socket_utils.rs for socket-option and unsafe libc abstraction;
- LICENSE for redistribution and attribution constraints.

KixDNS is GPL-3.0, as is this repository. No KixDNS source is extracted in
Phase 4. If a later task proposes source extraction or a direct source-derived
file, it must record the upstream URL, exact pinned commit, original copyright
and GPL notice, local modifications, and the project's license review. A
behavioral or architectural adaptation is still credited to this pinned audit,
but it must be reimplemented against the MosDNS contract rather than copied.

The KixDNS manifest uses Tokio (including runtime, net, time, sync, signal),
tokio-util, hickory-proto, bytes, dashmap, socket2, libc, and related
dependencies. Phase 4 has no direct KixDNS dependency. Planned Rust
dependencies are independently reviewed against the MosDNS workspace and the
forbid-unsafe-code boundary.

## Findings by transport concern

| Concern | Pinned evidence | Observed KixDNS design | Classification | Phase 4 decision |
| --- | --- | --- | --- | --- |
| Async runtime and task spawn | Cargo.toml; src/engine/transport.rs imports Tokio net/time/sync and tokio-util | One Tokio-based application with spawned UDP receiver and TCP reader tasks | adapted design | Use one host-owned Tokio runtime; upstream-core never creates a runtime. Keep task ownership with each Rust upstream/exchange |
| UDP socket ownership | transport.rs UDP state/pool around lines 62-115 | A bound UDP socket has a long-lived receiver task and shared state | adapted design | Phase 4 first choice is one ephemeral socket per exchange, no background receiver or pool |
| UDP multiplexing | transport.rs UdpSocketState/UdpPool around lines 62-115 | Shared socket receives many exchanges | rejected for first slice | Avoid a shared demux lifecycle until product need and close semantics are separately reviewed |
| UDP demultiplexing | transport.rs UdpInflightMap around lines 44-60 and receiver around 130-179 | DashMap keyed by internal u16 ID maps to original ID, expected source, and oneshot sender | rejected for first slice | No internal QID allocator/map; per-exchange socket isolates responses |
| UDP QID rewriting | transport.rs receiver 140-145 and send path 197-274 | Internal ID is written into the wire and restored before delivery | rejected | MosDNS contract keeps caller query ID; Rust sends unchanged bytes and matches original ID |
| UDP source validation | transport.rs receiver around 130-179 | Source address is checked against the in-flight expected address before delivery | adapted design | Preserve expected-peer validation; wrong peers are ignored until deadline/valid response |
| UDP wire ownership | transport.rs Bytes/BytesMut imports and send/receive paths | Bytes-backed owned packet data crosses task/oneshot boundaries | adapted design | Use Bytes or Vec for fully owned returned wire; never Go pool buffers or FFI pointers |
| UDP timeout | engine/upstream.rs forward_udp_smart around 463-518 | Timeout is split across UDP attempts and fallback policy | adapted design | Use one absolute deadline across UDP and any TC fallback; no fresh timeout for TCP |
| UDP retry/retransmit | engine/upstream.rs forward_udp_smart around 463-518 | UDP timeout/retry is part of smart forwarding | rejected for first slice | No generic post-send retry initially; any retransmission needs MosDNS evidence and a new review |
| TCP framing | transport.rs TCP client/read/write paths around 853-900 and reader around 492+ | Two-byte big-endian length prefix and exact payload reads | adapted design | Preserve protocol framing; test partial prefix/body, zero, oversize, EOF, and short writes |
| TCP stream reader | transport.rs TcpMuxClient pending/read loop around 492+ | A reader task reads prefix/body and dispatches completed response | adapted design | First Rust transport uses one fresh stream per request, with synchronous ownership inside the exchange; no shared reader |
| TCP pool/multiplexer | transport.rs TcpMultiplexer around 277+ | Pool of multiplexed clients and pending request map | rejected for first slice | Defer pooling, pipelining, pending demux, and idle policy; do not call them implementation decides |
| TCP reuse and recovery | transport.rs exchange/retry around 801-847 and connection setup around 853+ | Reused client can retry after a connection failure | rejected | Close every first-slice connection on failure; no post-send reuse retry without contract evidence |
| TCP concurrency | transport.rs pending map and multiplexed client | Multiple requests can share one TCP client | rejected for first slice | One request per fresh connection gives explicit isolation; pooling needs a separate review |
| Cancellation primitive | transport.rs TcpMuxClient pending state and tokio_util CancellationToken | CancellationToken participates in read/write/select cleanup | adapted design | Own a transport cancellation token in upstream-core; map sequence cancellation at the upper boundary, no sequence-core dependency |
| Cancellation cleanup | transport.rs reader and pending-request removal around 492+ | Select paths remove pending state and terminate reader/client work | adapted design | Every UDP socket, TCP stream, timer, and pending exchange is owned and reaped on cancel/close |
| Connect/read/write deadline | transport.rs timeout/select paths; upstream.rs policy timeout | I/O operations are bounded by timeout and cancellation | adapted design | One absolute deadline is checked at every phase; cancellation wins ties and blocks new fallback |
| TC detection | engine/upstream.rs forward_udp_smart around 463-518 | UDP response can trigger TCP fallback | adapted design | Keep composite UdpTcpPolicy; inspect TC header before complete RR validation and reuse the same query/deadline |
| TC fallback retry meaning | engine/upstream.rs around 463-518 | Fallback is coupled to KixDNS upstream policy and may include retries | adapted design | Retain only protocol-approved TC-to-TCP transition; do not copy KixDNS retry policy or sequence semantics |
| Response validation | hickory-proto use in Cargo.toml and engine parsing paths | KixDNS uses hickory-proto/engine helpers in its application model | rejected as direct code | Reuse mosdns-dns-core as the authoritative Rust DNS parser/validator; add only a reviewed header-only helper if needed |
| Close state | transport.rs pool/client close paths and pending cleanup | Clients/pools close readers and pending work | adapted design | Formal Open -> Closing -> Closed state; idempotent close rejects new work and reaps all exchange resources |
| Idle timeout | src/config.rs timeout/pool fields; transport.rs pool/client lifecycle | Configurable idle/pool behavior exists for reused TCP clients | rejected for first slice | No pool or idle timer in first implementation; classify as future product/config audit |
| Socket abstraction | src/socket_utils.rs | socket2/libc helpers set options and use unsafe libc paths | rejected | No source extraction and no unsafe socket-options layer in the bounded direct-endpoint slice; options require separate review |
| Socket options | src/socket_utils.rs; src/config.rs endpoint fields | Mark/device/bind-style knobs are handled outside the basic exchange path | research unresolved | Audit MosDNS SoMark, BindToDevice, local bind, and bootstrap semantics before promising support |
| Attribution and license | LICENSE and existing docs/rust/kixdns-reuse.md | GPL-3.0 source and project-level attribution obligations | direct dependency: none | No direct dependency or extracted source; preserve this ledger and require attribution if scope changes |

Classification meanings in this table are deliberate:

- direct dependency means a pinned KixDNS crate or source is linked directly;
  none is approved for Phase 4;
- adapted design means the behavior or shape is useful evidence but is
  independently reimplemented against MosDNS contracts;
- extracted code means source is copied with license/attribution and review;
  none is planned or authorized;
- rejected means the KixDNS behavior or mechanism is unsuitable for the first
  Rust-native boundary, not that it was missed.

## MosDNS compatibility conclusions

The useful KixDNS evidence is the separation of async I/O from policy, the need
for explicit source/ID validation, exact TCP framing, owned bytes across task
boundaries, cancellation-aware cleanup, and a single close lifecycle.

The following KixDNS mechanisms are specifically not MosDNS contracts:

- internal UDP ID rewriting and a shared DashMap demultiplexer;
- a long-lived UDP receiver task and transport pool;
- multiplexed/reused TCP clients and their pending-request retry behavior;
- KixDNS timeout splitting and generic recovery/retry policy;
- hickory-proto/application helpers as a replacement for mosdns-dns-core;
- unsafe socket-option helpers and KixDNS configuration ownership;
- KixDNS JSON pipeline, resolver, or server/control-plane semantics.

MosDNS product behavior remains authoritative. The Phase 4 design therefore
chooses per-exchange UDP sockets and fresh TCP connections, preserves the
original DNS ID and query bytes, uses one absolute deadline, permits only
reviewed TC-to-TCP fallback, and makes every post-send error explicitly
non-retryable unless a later characterization proves a protocol contract.

## Open research before any deferred scope

The following questions remain planning blockers for the named features, but do
not block the bounded direct numeric UDP/TCP architecture once the root review
accepts it:

1. Which MosDNS configuration fields make SoMark, BindToDevice, local bind,
   SOCKS5, hostname resolution, or bootstrap externally relied upon?
2. Is Go TCP reuse/pipeline observable through latency, connection limits,
   upstream behavior, or only an internal optimization?
3. Is Go UDP retransmission a configured/user-visible guarantee or an internal
   availability strategy?
4. What maximum accepted DNS wire size and EDNS/OS truncation behavior should
   the Rust product contract freeze on each supported platform?
5. If a future pooled TCP layer is needed, what are its checkout, idle, pending,
   eviction, and shutdown guarantees?

Each answer requires a focused source/config characterization and a matrix
classification before the related implementation is authorized. No KixDNS
default or implementation detail answers these MosDNS questions automatically.

## Audit conclusion

KixDNS supplies useful transport research but no Phase 4 source extraction or
direct dependency. The auditable reuse is classified as adapted design for
Tokio ownership, exact framing, source validation, cancellation cleanup, owned
bytes, and TC policy shape; shared UDP/TCP multiplexing, QID rewriting,
generic retry/recovery, unsafe socket options, and application semantics are
rejected or deferred. This conclusion is the evidence package for the root
planning gate, not an implementation authorization.
