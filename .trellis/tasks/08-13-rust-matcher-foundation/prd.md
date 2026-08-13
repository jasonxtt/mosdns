# Rust matcher and rule compiler foundation

## Goal

Move the next bounded data-plane slice—domain and IP rule compilation plus
read-only matching—into Rust. This advances the native Rust architecture
instead of optimizing the temporary per-query cache cgo boundary, while
preserving MosDNS configuration, reload, routing, API, and audit behavior.

## Background and confirmed decisions

- The cache foundation has a safe, versioned ABI and real Linux+cgo parity
  coverage. It remains experimental because full p99/CPU/RSS soak and the 10%
  default-performance gate are not complete.
- The user explicitly prefers continuing the full Rust migration over further
  cache micro-optimization. Temporary hybrid-boundary overhead is acceptable
  only in experimental builds; it must not become the production default.
- The approved migration order in `docs/ai/rust-rewrite-plan.md` selects
  domain/IP matchers before the Rust query/sequence core.
- Current domain matching supports `full`, `domain`, `regexp`, and `keyword`
  rules with case-insensitive/trailing-dot normalization and fixed precedence.
  Current IP matching masks, sorts, deduplicates, and binary-searches IPv4/IPv6
  prefixes.
- Go currently owns provider downloads, text/SRS parsing entrypoints, HTTP
  APIs, subscriptions, reload scheduling, `RuleExporter`, sequence integration,
  and query-context/audit updates. Online providers atomically replace compiled
  snapshots after reload.
- CodeGraph shows the reusable matcher interfaces feed `qname`, `cname`,
  `client_ip`, `resp_ip`, `ptr_ip`, `domain_set`, `ip_set`, `sd_set`, `si_set`,
  and `domain_mapper`; changing these interfaces all at once would be an unsafe
  migration boundary.
- KixDNS commit `2da3a2d` remains the audited Rust reference. Reuse is selective
  and attributed; its JSON pipeline does not define MosDNS semantics.

## Requirements

### R1 — Freeze matcher contracts before replacement

Add executable fixtures for normalization, rule syntax, precedence, duplicate
rules, invalid rules, IPv4/IPv6 masking, nested prefixes, SRS/text inputs,
provider composition, atomic reload, and match-result metadata. Record every
consumer and classify it as migrated in this task or deferred.

### R2 — Establish one Rust runtime boundary

At the second Rust module, extract the cache ABI into one Rust runtime
`staticlib` that can host cache and matcher capabilities without linking two
independent Rust runtimes. Preserve existing cache ABI entrypoints and add
capabilities rather than silently changing their contracts. Generalize the
experimental build tag/path while retaining a documented compatibility route
for the cache-only tag during transition.

### R3 — Implement safe domain and IP matcher cores

Implement immutable Rust matcher snapshots for the exact Go contracts:

- domain `full`, `domain`, `regexp`, and `keyword` rules;
- lowercase/trailing-dot normalization and current precedence;
- IPv4/IPv6 address/prefix normalization, containment, sorting, and
  deduplication;
- deterministic build errors with no panic crossing FFI;
- O(1)-style exact lookup, reverse-label suffix lookup, compiled regex, and
  logarithmic or trie-based prefix matching without per-rule cgo calls.

### R4 — Make build-and-swap transactional

Build a complete matcher handle off-path, validate all accepted rules, then
atomically publish it. A failed initial build selects the Go matcher; a failed
reload retains the previous Rust snapshot. Concurrent match/reload/close must
be memory-safe and race-free.

### R5 — Integrate through existing Go interfaces

Keep YAML fields, plugin types, provider download/config/API behavior,
`RuleExporter`, subscriptions, `special_groups`, `domain_set` audit metadata,
and sequence semantics in Go. The first integration covers the reusable
domain/IP engines and the direct `domain_set`/`ip_set` plus base matcher paths.
Online-source provider fan-out and `domain_mapper` result compilation are
prepared by fixtures and interfaces but migrate only after the base engines
pass parity.

### R6 — Preserve default and fallback behavior

Normal builds require neither Rust nor cgo and use the current Go matchers.
Rust matchers are explicit and experimental. Startup ABI/capability/build
failure falls back to Go; runtime corruption/internal failure disables only the
affected Rust matcher snapshot and leaves the established Go path available.

### R7 — Verify reuse, safety, parity, and migration progress

Maintain a KixDNS reuse ledger with commit, origin, license, and local changes.
Run Rust unit/property/malformed tests, Go golden parity, race tests, real
Linux+cgo integration, full Go tests, default builds, and experimental builds.
Benchmark load time, snapshot memory, and match throughput, but do not spend
this task micro-optimizing the transitional cgo call or use performance as a
reason to stop the wider migration.

## Acceptance Criteria

- [ ] A checked-in consumer/compatibility matrix maps domain/IP behavior and
  every affected provider/matcher to fixtures and migration status.
- [ ] The default build and configuration remain Go-only and pass without a
  Rust toolchain or cgo.
- [ ] One experimental Rust `staticlib` exports the unchanged cache ABI plus
  versioned matcher capabilities; it does not link separate Rust runtimes.
- [ ] Rust domain results match Go for normalization, all four rule types,
  precedence, duplicates, invalid input, and composed sets.
- [ ] Rust IP results match Go for IPv4, IPv4-mapped representation, IPv6,
  host addresses, CIDRs, overlapping prefixes, masking, and deduplication.
- [ ] Reload is transactional: invalid replacement leaves the active snapshot
  and its observable results unchanged.
- [ ] Concurrent match/reload/close passes Go race tests and Rust concurrency
  tests; no panic, ambiguous buffer ownership, or freed handle crosses FFI.
- [ ] Existing YAML, HTTP API bodies/statuses, rule files, SRS behavior,
  subscriptions, metrics, and audit/query-context fields are unchanged.
- [ ] Direct `domain_set`/`ip_set` and base qname/IP matchers can exercise Rust
  on Linux through an explicit switch and deterministically fall back to Go.
- [ ] KixDNS-derived code is pinned and attributed; MosDNS fixtures remain the
  authority where semantics differ.
- [ ] Full Go/Rust checks, default Vue-aware build, experimental Linux build,
  and isolated `mos-test` parity smoke pass before considering provider fan-out.

## Out of scope

- Making Rust matchers, Rust cache, or the hybrid binary the production default.
- Further cache hot-path micro-optimization or hiding cgo cost behind a Go L1.
- Migrating sequence control flow, query-context ownership, upstreams, servers,
  YAML/coremain, HTTP control plane, or Vue UI.
- Replacing `domain_mapper`, `sd_set`, `sd_set_light`, `si_set`, or AdGuard
  runtime ownership in the first integration slice; their parity fixtures and
  compatible interfaces are in scope so they can fan out next.
- Adopting KixDNS JSON pipeline/rule ordering when it differs from MosDNS.

## Risks and deferred items

- Matcher calls still cross cgo while sequence/query execution remains in Go.
  This is transitional and disappears when the Rust query/sequence core owns
  the request hot path; until then the matcher backend stays experimental.
- Go `regexp` and Rust regex syntax/behavior are not automatically identical.
  Unsupported differences must be detected by parity fixtures and use the Go
  path, not silently reinterpret rules.
- SRS decoders and online-provider lifecycle are duplicated across several Go
  plugins. This task must preserve their inputs and avoid a broad provider
  cleanup while establishing the Rust snapshot boundary.
