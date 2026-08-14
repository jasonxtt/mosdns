# Rust matcher Phase 2 expansion

## Goal

Complete the remaining bounded Phase 2 matcher migration by extending the
experimental Rust runtime to the online domain/IP providers and to
`domain_mapper`. Preserve current MosDNS configuration, provider lifecycle,
special-group routing, result metadata, reload, fallback, and default Go-only
behavior. This task must leave Phase 3 query/sequence ownership untouched.

## Background

- The archived matcher-foundation task completed the single Rust runtime,
  immutable domain/IP indexes, safe versioned ABI, and opt-in adapters for
  `domain_set`, `ip_set`, `base_domain`, and `base_ip`.
- `sd_set` owns a real domain matcher and `si_set` owns a real IP matcher; both
  rebuild snapshots after online/local SRS updates.
- `domain_set_light` and `sd_set_light` deliberately retain only rule sources
  and always return `false` from `Match`. They exist to feed `domain_mapper`
  without maintaining a second in-memory matcher and must remain lightweight.
- `domain_mapper` aggregates every `RuleExporter`, inherits `domain:` results
  into descendants, merges overlapping `full`/`domain`/`keyword`/`regexp`
  results, and publishes fast marks, context marks, joined tags, and joined
  source names. Its mutable `QuickAdd` hot map is also used by `domain_output`.
- Rust selection is experimental through `MOSDNS_MATCHER_BACKEND=rust`; normal
  builds must remain independent of Rust and cgo.

## Requirements

### R1 — Freeze the extension contracts

Add executable Go fixtures for provider text/SRS parsing, source composition,
rule counts, subscriptions, online reload, invalid/partial sources, and close.
Add `domain_mapper` fixtures for ancestor inheritance, overlapping rule types,
deduplicated mark/tag/source merging, detailed source metadata, defaults,
`QuickAdd`, and concurrent rebuild/lookup. Update the compatibility matrix so
every Phase 2 consumer has an explicit final status.

### R2 — Reuse one internal Go/Rust adapter

Create one build-tagged internal matcher adapter used by the existing
`domain_set`/`ip_set` paths and the new providers. It must centralize ABI and
capability negotiation, typed handles, caller-owned buffers, close behavior,
runtime circuit breaking, and non-cgo stubs. Do not create provider-to-provider
imports or duplicate cgo declarations in each provider.

### R3 — Extend real provider matchers

Add transactional opt-in Rust snapshots to `sd_set` and `si_set` from the same
accepted rule stream used by their Go snapshots. Provider download, config,
file/SRS parsing, HTTP API, rule counts, subscription callbacks, and scheduling
stay in Go. Each published generation must represent one rule version; a Rust
build/runtime failure publishes the current Go generation rather than serving
a stale Rust generation.

### R4 — Preserve light-provider semantics

`domain_set_light` and `sd_set_light` remain rule exporters with constant-false
matching and do not acquire resident Rust matcher handles. Their exported rules
must participate in the Rust-backed `domain_mapper` rebuild in bounded batches,
without changing YAML, API, file, SRS, rule-count, or notification behavior.

### R5 — Compile static `domain_mapper` snapshots in Rust

Extend `matcher-core` and the runtime ABI with an immutable valued domain
snapshot that preserves:

- `full`, `domain`, `keyword`, and `regexp` normalization and matching;
- ancestor inheritance for `domain:` rules;
- simultaneous merging of base-domain, keyword, and regexp results;
- deduplicated fast marks, context marks, joined output tags, and source names;
- detailed `RuleEntry` source metadata and provider ordering semantics;
- deterministic handling of invalid or Rust-incompatible expressions.

The Go `QuickAdd` hot map remains dynamic and is merged with the active static
snapshot result. `Exec`, `GetFastExec`, `FastMatch`, default behavior, query
context fields, and the `domain_output` interface remain unchanged.

### R6 — Make publication and fallback generation-safe

Build complete Go and Rust candidates off-path and publish one immutable
generation atomically. A provider parse failure retains the current established
Go behavior. A Rust construction failure publishes the new Go candidate and
logs an observable fallback; it must not combine new Go rules with an old Rust
handle. Concurrent match/rebuild/close must not use freed handles or close a
replacement generation. An empty ruleset is a valid snapshot.

### R7 — Keep rollout reversible

Default builds and runtime selection stay Go-only. ABI/capability mismatch,
unsupported regex, malformed result data, or runtime failure disables only the
affected Rust generation and continues through its matching Go generation.
No YAML, API, rule-file, SRS, metric, audit, or WebUI schema migration is
introduced.

### R8 — Verify Phase 2 completion

Run pure Rust unit/property/malformed/concurrency tests, Go golden tests,
focused race tests, full default Go tests, Linux+cgo integration and race,
fixed-fixture benchmarks, an experimental binary build, and isolated
`mos-test` reload/fallback/restart smoke. Update evidence and handover documents
with measured transitional costs and the Phase 3 entry gate.

## Acceptance Criteria

- [x] `sd_set` and `si_set` use the real Rust matcher when explicitly selected
  on Linux+cgo and deterministically fall back to the matching Go generation.
- [x] `domain_set_light` and `sd_set_light` remain constant-false, low-memory
  exporters and gain no resident matcher snapshot.
- [x] Rules from all four named providers reach `domain_mapper`; providers that
  implement `DetailedRuleExporter` preserve source metadata.
- [x] Rust-backed `domain_mapper` matches Go for normalization, inheritance,
  overlap, deduplication, result ordering, defaults, tags, sources, fast marks,
  and context marks.
- [x] `QuickAdd`, `FastMatch`, `Exec`, and `GetFastExec` merge dynamic hot-map
  results with the current static generation exactly as before.
- [x] Empty, invalid, partial, failed, and concurrent reloads have documented
  parity; no reload serves mismatched/stale Go and Rust generations.
- [x] Concurrent query/rebuild/close passes Go race and Rust concurrency tests;
  handle namespaces and caller-owned result buffers pass ABI misuse tests.
- [x] Existing `domain_set`/`ip_set` behavior remains green after moving their
  bridge plumbing into the shared internal adapter.
- [x] Default `go build ./...`, `go vet ./...`, and `go test ./...` work without
  a Rust toolchain, cgo, build tag, or runtime environment variable.
- [x] Rust fmt/test/clippy/release, Linux tagged cgo normal/race, benchmark,
  experimental binary, and isolated `mos-test` smoke gates pass.
- [x] Compatibility, benchmark, test-host, rewrite-plan, and handover documents
  identify Phase 2 as complete without claiming Rust is production-default.

## Out of Scope

- Rust ownership of DNS wire requests, query context, sequence control flow,
  `jump/goto/return/exit/try`, upstreams, listeners, YAML, API host, or WebUI.
- Making Rust the default backend or deploying it to production/port 53.
- Rewriting provider download/config/API/SRS parsing in Rust.
- Cache micro-optimization or redesign.
- OpenWrt, lite, docker, arm64, or release fan-out.
- Unrelated WebUI/coremain changes already present in the dirty worktree.
