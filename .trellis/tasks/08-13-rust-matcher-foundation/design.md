# Rust matcher and rule compiler foundation — technical design

## 1. Migration boundary

```text
YAML / files / SRS / online downloads / APIs / subscriptions
                         |
                  Go provider control plane
                         |
           build complete immutable rule snapshot
                    /                    \
          default Go matcher     experimental Rust runtime
                                         |
                         domain/IP compiled snapshot handles
                                         |
                            one coarse match call per matcher
```

This task changes rule compilation and read-only index ownership, not query or
sequence ownership. The Go interfaces remain stable so each consumer can move
independently. The temporary per-matcher cgo call is accepted only for
experimental validation; Phase 3 folds matcher dispatch into the Rust query
core and removes that boundary from the native hot path.

## 2. Rust workspace and ABI

The second Rust module is the point where the current cache-only static library
becomes a single runtime library:

```text
rust/
  runtime/       # the only staticlib; ABI, status, panic boundary, capabilities
  cache-core/    # safe Rust cache implementation, linked as an rlib
  matcher-core/  # safe immutable domain/IP engines, linked as an rlib
```

The runtime retains the existing cache symbols and ABI version contract. New
capability bits and matcher-specific handle operations are additive. Handles
are typed/namespaced so a cache handle cannot be accepted as a matcher handle.
No Rust-owned pointer is used as an unvalidated C handle.

The general experimental build tag becomes `mosdns_rust`; the existing
`mosdns_rust_cache` tag remains a temporary cache-compatible alias during this
task. The default build continues to compile only Go stubs.

## 3. Domain snapshot

A builder accepts a complete ordered batch of typed rules. It normalizes and
validates every rule before publishing an immutable snapshot containing:

- exact-name map for `full`;
- reverse-label trie for `domain` suffix matches;
- precompiled regex collection compatible with accepted Go fixtures;
- keyword collection preserving MosDNS match semantics;
- fixed lookup precedence: full → domain → regexp → keyword.

The FFI result for the foundation is boolean. Result-valued compilation for
`domain_mapper` is represented in fixtures/design but deferred until boolean
engines pass, preventing metadata ABI complexity from obscuring base parity.

## 4. IP snapshot

The builder accepts canonical 16-byte addresses plus prefix length and an
IPv4/IPv6 family marker. It reproduces the current IPv4-to-16-byte ordering,
masking, overlap collapse, and containment rules. The published representation
may be a radix trie or sorted disjoint prefix vector; measured build time,
memory, and lookup behavior decide between them without changing the ABI.

## 5. Go adapters and lifecycle

The existing Go loaders remain the initial source of rule strings/prefixes.
Adapters build both the Go fallback and Rust candidate from the same validated
batch. Publication uses an atomic holder:

1. parse/load into temporary data;
2. build and validate a new Rust handle;
3. atomically replace the active holder;
4. close the old handle after readers can no longer acquire it;
5. on failure, keep the old holder and report the error through the existing
   API/logging boundary.

Direct adapters are added first to `domain_set`, `ip_set`, `base_domain`, and
`base_ip`; qname/cname/client/response/PTR matchers inherit the existing base
interfaces. Other online providers continue returning their Go matchers until
the follow-up fan-out task.

## 6. Compatibility and failure policy

- Rule text, YAML, files, API response schemas/status codes, and provider
  subscription behavior do not cross Rust.
- Invalid build/reload is transactional and never publishes a partial matcher.
- An explicit Rust request with unavailable ABI/capabilities logs once and uses
  Go. Internal/closed/panic statuses trip a one-way per-snapshot fallback.
- Expected no-match is not an error and does not trigger fallback.
- No raw domain, IP rule set, or downloaded content is written to logs.

## 7. Testing and rollout

Start with Go golden fixtures, then execute the same vectors against pure Rust
and the real Linux+cgo adapter. Add malformed ABI, concurrent match/swap/close,
large real-list load, and default-build isolation tests. Run only isolated
listeners/processes on `mos-test`; do not replace its port-53 binary or service.

Performance evidence records compilation time, steady snapshot RSS, lookup
throughput, and cgo overhead. It guides Phase 3 architecture but does not make
the transitional matcher bridge a production default.

## 8. Rollback

The runtime switch is explicit. Disabling it selects the untouched Go matcher
implementation. A failed Rust snapshot never replaces the Go snapshot or the
previous valid Rust snapshot. No config or rule-file migration is introduced.
