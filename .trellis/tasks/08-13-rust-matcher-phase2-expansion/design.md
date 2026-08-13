# Rust matcher Phase 2 expansion — design

## 1. Boundary

Go remains the control plane for provider configuration, downloads, APIs,
timers, subscriptions, file/SRS parsing, query context, sequence execution,
and the mutable `domain_mapper.QuickAdd` map. Rust owns immutable compiled
domain/IP and valued-domain snapshots behind the existing experimental runtime.

```text
Go provider reload / RuleExporter
            |
     accepted rule batch
            |
  shared internal adapter
            |
  single mosdns_runtime ABI
            |
 Rust matcher-core snapshots
```

No provider receives a private Rust runtime. Cache and every matcher remain in
the existing single `rust/runtime` static library.

## 2. Shared adapter

Introduce an internal package under the data-provider tree with build-tagged
real/stub implementations. It owns:

- environment selection and ABI/capability negotiation;
- domain, IP, and valued-domain snapshot interfaces;
- typed integer handles and idempotent close;
- caller-owned input/output buffers;
- per-snapshot circuit breaking and error classification;
- default-build stubs that import neither cgo nor Rust artifacts.

Refactor the existing `domain_set` and `ip_set` bridge files to delegate to this
package before adding `sd_set`, `si_set`, and `domain_mapper`. This is a bounded
reuse refactor: public provider interfaces and matcher behavior do not change.

## 3. Provider generations

`sd_set` and `si_set` publish a generation containing the current Go matcher
and an optional Rust snapshot built from the same parsed input. Match selects
Rust only while that generation's Rust handle is healthy; otherwise it uses
that generation's Go matcher.

Candidate construction happens off-path. Publication is one atomic swap. The
retired Rust handle is closed only after it can no longer be selected; the
runtime registry's read/write locking completes any in-flight match before
removal. A Rust build failure is not a provider reload failure: publish the new
Go generation with Rust absent. This prevents a stale Rust snapshot from
shadowing freshly downloaded rules.

The existing provider-specific behavior for missing or invalid sources remains
authoritative. The migration does not reinterpret partial reload semantics.

## 4. Light providers

`domain_set_light` and `sd_set_light` keep no compiled matcher and their
`Match` method remains constant-false. They continue exporting temporary rule
batches during `domain_mapper` rebuild. The batch crosses the Rust boundary
only once per rebuild; it is not retained as a second Go matcher in either
light provider.

## 5. Valued domain snapshot

Add a valued matcher to `matcher-core` that accepts rule records containing the
normalized rule and a compact result payload. The build input is a versioned,
length-safe off-path batch (JSON is acceptable at build time; no line-delimited
escaping contract). The Rust builder performs the same ancestor inheritance,
pooling/deduplication, rule compilation, and overlap semantics as the frozen Go
implementation.

Lookup returns one merged result through a caller-owned versioned byte buffer:

- a 64-bit fast-mark mask;
- sorted/deduplicated `u32` context marks;
- joined/deduplicated output tags;
- joined/deduplicated source names.

The ABI reports required output length when the buffer is too small. No Rust
allocation crosses ownership boundaries. The Go adapter validates the result
format before applying it. Encoding details and status values are added to the
checked-in header and ABI tests.

Go builds an equivalent `compiledMatcher` candidate from the same aggregated
records. This is the deterministic fallback and parity oracle during the
experimental phase. Unsupported Rust regex or ABI construction errors publish
that Go candidate instead of a partial Rust candidate.

## 6. Dynamic hot-map merge

`QuickAdd` remains in Go because it is mutable request-adjacent state owned by
`domain_output`, which belongs to the later query/sequence migration. Lookup:

1. reads the current static generation;
2. obtains either its Rust result or its Go fallback result;
3. reads the Go hot-map entry;
4. merges both with existing deduplication/order behavior.

Rebuild clears and repopulates full-rule hot entries exactly as today. The
generation swap and hot-map replacement are serialized so lookup cannot join
static data from one rebuild with precomputed hot entries from another.

## 7. Compatibility and observability

The following remain byte/field compatible: YAML fields, plugin names, API
bodies/status codes, source JSON, SRS/text parsing, rule counts, subscriptions,
`KeyDomainSet`, `KeyMatchedRuleSource`, fast/context marks, and default tags.

Rust selection remains `MOSDNS_MATCHER_BACKEND=rust`. Build/ABI/construction
failure is logged once per generation with provider identity. Runtime failure
trips only that generation's Rust path and exposes the existing Go behavior.
No production default or config migration is added.

## 8. Verification and rollback

Pure tests freeze aggregation and encoded-result behavior before integration.
Go golden tests compare identical records and queries through Go, pure Rust,
and Linux+cgo. Race tests cover lookup/rebuild/close and `QuickAdd`. Fixed
benchmarks report build time, lookup throughput, allocations, result size, and
cgo calls without treating the temporary bridge as a production gate.

`mos-test` uses an isolated binary, temporary config/rules, and random high
loopback ports. It must not touch port 53, the installed service, production,
or unrelated host files. Rollback is removal of the env selection: the same
generation's Go matcher remains available.
