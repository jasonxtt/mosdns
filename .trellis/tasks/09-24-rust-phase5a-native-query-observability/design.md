# Design: native query observability, bounded Phase 5A subset

Read `prd.md` and `research/source-audit.md` first. This design is a planning gate, not an implementation authorization.

## Boundary and data flow

The existing native host remains the sole owner of parsing, sequence execution, cache, upstreams, and listener I/O. Add one host-owned observer that receives a compact terminal result from each listener. The execution driver supplies *facts it alone knows* (cache lookup result, ordered external attempts, accepted final upstream, response provenance/error); the UDP/TCP listener supplies admission time, transport/client identity, framing/send outcome, and cancellation. A terminal guard finalizes once for every admitted request, including early return and shutdown paths. Observing a result must not alter response generation, forwarding, cache publication, or cancellation.

Suggested flow:

```
listener admit/parse -> request context + observer guard
  -> existing sequence/exchange/cache driver -> wire response + execution facts
  -> existing frame/send -> terminal outcome
  -> observer counters + optional bounded audit append
```

Use an explicit terminal outcome enum rather than inferring delivery from a nonempty wire buffer. Count malformed/partial input at the listener before admission; a parse failure does not create a fake query audit entry. Cancellation, failed send, and internal no-response have separate outcomes. Account for TCP framing and UDP send at the actual existing I/O boundary. Audit `duration` covers admitted request to terminal outcome; query execution latency can be a separate field if needed. Test deterministic terminalization with injected send failure/cancellation, not timing sleeps.

## Stable semantic contracts

The Phase 5A audit record is a typed Rust internal record, not an early promise of the 5C JSON/API wire schema. Its fields are: timestamp, client IP/address, transport, qname/qtype/qclass, elapsed time, final DNS rcode when one exists, terminal outcome, final sequence tag, cache status, final upstream identity, ordered attempted upstream identities, and failure provenance. The final upstream is set only after a qualifying upstream response has become the final DNS response; a W2 hit has no upstream. W3's B leg remains diagnostic when the final answer came from A or C. A locally synthesized SERVFAIL is not an upstream SERVFAIL. Where a field cannot be established, record an explicit absent/unknown value and never guess from configuration.

The Phase 5A metrics snapshot uses fixed counters and a fixed-bucket duration histogram. It includes the R4 dimensions without qname/client/trace labels. Upstream dimensions are bounded to the strictly compiled catalog. All counters reconcile at a snapshot boundary: `admitted = in_flight + terminal_total` for a quiescent single host, and `terminal_total = sent + send_failed + canceled + no_response`. Forward attempt totals match mock upstream events where a network attempt was actually made; cache hits add no forward attempt. Implement the snapshot through a host-owned interface suitable for `Send + Sync` use later, even though today's listener tasks run on a current-thread local set. Do not introduce a fresh `Rc` coupling in the observer API.

Detailed audit is enabled by the existing listener `enable_audit` flag. For this isolated 5A process, true begins in-memory capture at host startup and false retains none; 5C will add the complete management capture lifecycle. Use a bounded ring with a default 100,000-record limit and an explicit eviction count, matching the current Go default retention size without reproducing Go's silent queue loss. A test-only host option can lower the limit to exercise eviction. No per-query disk/network I/O, blocking async writer, or unbounded channel is introduced. Store no credentials. Any operational error in capture must be visible in the snapshot rather than silently losing records.

## Compatibility and ownership

- Keep strict YAML grammar and plugin-count checks. This task changes only the known `enable_audit` value acceptance; unsupported syntax must still fail before assembly and socket bind.
- Preserve the current single host runtime and all existing W1/W2/W3 DNS semantics. The observer's read-only snapshots can later feed 5C `/metrics` and audit APIs, but this task does not expose an HTTP endpoint or commit to a JSON schema.
- Keep the Rust-native path independent of Go/cgo adapters. Go source is a field/behavior discovery reference; the Rust observer must not copy silent-loss behavior or Go internals.
- During shutdown, stop admission first, finalize or cancel/join admitted requests, then close upstream owners as today. After drain, in-flight is zero and snapshots are stable. Rebind creates fresh owner state.
- Full audit fields such as response answers/flags, domain-set/group/source metadata, API pagination/ranking/window stats, persistence, capture control, and UI behavior are reserved for 5C. Record enough accurate execution provenance now so future fields do not require a second router.

## Trade-offs and evidence gate

An in-memory snapshot avoids a premature management API and keeps the query path free of disk/network I/O. It is not a user-facing audit UI; Phase 5C owns that contract. A ring retains the most recent events rather than promising unbounded history, with visible eviction accounting. Its lock or synchronization cost must be measured with audit on and off; if the current-thread collector becomes a bottleneck, optimize from profiling evidence without replacing the behavioral contract.

Before implementation, freeze the old Rust source/binary, W1/W2/W3 fixtures, runner hash, offered rates, duration/repetition, affinity/VM topology, validity gates, and latency/throughput/CPU/RSS regression budgets in `research/performance-manifest.md`. Pin the new commit/binary before official Linux candidate runs. Compare old versus new with audit off; compare new audit on versus new audit off, since the old host rejects audit on. The prior report's sender shortfalls and absent overload trigger prohibit using an invalid high-rate stage to excuse or prove overhead. Use only valid low/moderate offered load for this task; a later dedicated profiling/measurement task owns overload discovery and multi-core analysis. Preserve rejected attempts and limitations in the result report. Never test on production `mos`.
