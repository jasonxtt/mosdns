# Rust matcher Phase 2 expansion — implementation plan

Each slice starts with a failing behavior test and keeps the default Go build
green. Do not stage or modify the unrelated dirty WebUI/coremain worktree.

## Execution handoff

The current root session is executing and reviewing this task inline. No
implementer sub-agent is required for this continuation; the root still stops
at each slice review gate and does not stage, commit, archive, or start Phase 3
until the root explicitly authorizes the finish sequence.

Before changing code, the implementation owner must:

1. read `AGENTS.md`, `docs/ai/project-context.md`, `docs/ai/config-notes.md`,
   `docs/ai/rust-handover.md`, and
   `docs/ai/rust-rewrite-plan.md` in that order;
2. read this task's `prd.md`, `design.md`, and `implement.md` completely;
3. verify branch `rust`, inspect the dirty worktree, and preserve every path
   identified as user-owned in `docs/ai/rust-handover.md`;
4. load `trellis-before-dev` before implementation and activate this exact task
   only after the user approves the final planning summary;
5. implement one slice at a time with red test → minimal green change → focused
   validation, without staging, committing, archiving, or starting Phase 3.

After each slice, the implementation owner must report the following evidence
to the root reviewer:

- slice number and completed checklist items;
- exact changed-path list and any intentionally untouched risky paths;
- red-test evidence and final focused/full validation commands with results;
- fallback, concurrency, lifecycle, ABI, and compatibility decisions made;
- known limitations or unresolved review questions;
- confirmation that no unrelated file was staged, committed, or overwritten.

The implementation owner must stop after each slice and wait for root review.
A failed review returns the same slice with concrete findings; later slices do
not begin until the current slice passes. Only the root reviewer may declare
Phase 2 complete or authorize the Trellis finish/archive sequence.

## TDD interfaces and mock boundaries

| Slice | Public interface under test | Observable behavior | Mock boundary |
| --- | --- | --- | --- |
| 0 | provider `Match`, `GetRules`, `GetRuleEntries`, `Subscribe`, reload helpers; mapper `FastMatch`, `QuickAdd`, `Exec`, `GetFastExec` | frozen Go outputs, metadata, notifications, defaults, reload and concurrency | use temporary files and in-process fake exporters; do not mock matcher semantics |
| 1 | shared adapter `DomainSnapshot`, `IPSnapshot`, create/match/len/close; existing `domain_set`/`ip_set` paths | identical foundation behavior in default stub and Linux+cgo builds | C ABI is real on Linux; default tests use build-tag stub only |
| 2 | `sd_set.GetDomainMatcher`/`Match`, `si_set.GetIPMatcher`/`Match`, reload and `Close` | same-generation Rust selection, current Go fallback, empty/failed/concurrent reload | HTTP download may use `httptest`; SRS parsing and generation publication are real |
| 3 | pure Rust valued matcher plus exported create/match/len/close ABI | exact merged payload, invalid input/status, buffer sizing, handle lifecycle | no mocked Rust matcher or registry; ABI tests call real symbols |
| 4 | `domain_mapper` rebuild/lookup and light-provider exporters | Rust/Go parity, light-provider fan-out, hot/static merge, query-context output | fake `RuleExporter` only for deterministic provider events; matching and FFI are real |
| 5 | build scripts, CI gates, benchmark and smoke scripts | reproducible default/Rust builds, race, reload/fallback/restart and cleanup | isolated `mos-test` process only; never mock production or port 53 |

## Slice 0 — Contract freeze

- [x] Add provider fixtures for `sd_set`, `sd_set_light`,
  `domain_set_light`, and `si_set`: parsing, composition, rule counts,
  subscriptions, reload, empty/invalid sources, and close for providers that
  expose a lifecycle (`sd_set`, `sd_set_light`, and `si_set`).
- [x] Expand `domain_mapper` fixtures for inheritance, overlap, ordering,
  marks/tags/sources, detailed exporters, defaults, `QuickAdd`, and concurrency.
- [x] Update `docs/rust/matcher-compatibility.md` with exact extension states
  and rule-source ownership.

Exit evidence (2026-08-13): the Go baseline is executable through
`plugin/data_provider/{sd_set,sd_set_light,domain_set_light,si_set,domain_mapper}`
Slice 0 contract tests. The shared SRS builder is
`plugin/data_provider/testutil/srs.go`; no Rust adapter or provider fan-out was
started.

Validation: `gofmt` on all Slice 0 Go files; focused `go test` and `go vet` for
the five affected packages; focused `go test -race` for the same packages;
`go test -count=20` for the new Slice 0 fixtures. All passed on the Go-only
default build.

Review gate: root verifies that each new test fails for a missing Rust behavior
or freezes a pre-existing Go contract without becoming tautological.

## Slice 1 — Shared adapter extraction

- [x] Add the internal real/stub Rust matcher adapter and move ABI negotiation,
  domain/IP handles, result buffers, close, and circuit-breaker behavior into it.
- [x] Convert `domain_set` and `ip_set` to the shared adapter without public or
  observable behavior changes.
- [x] Run existing golden, ABI, default-build, and host-available race gates.
- [ ] Run Linux+cgo provider/adapter integration and race gates.

Implementation/review evidence (2026-08-14): Luna added the shared
`plugin/data_provider/matcher_adapter` real/stub bridge and reduced both
provider-specific Rust backends to delegating wrappers. Root review found no
scope, ABI, build-tag, ownership, lifecycle, fallback, or default-build
regression. Focused and full Go tests, `go vet`, `go build`, race tests, both
Rust-tag stub builds, Rust fmt/test/clippy/release, and the existing Rust ABI
contract suite passed. Linux+cgo provider integration/race and the experimental
Linux binary were not runnable on this macOS arm64 host; rerun those gates on
Linux before authorizing Slice 2. No Slice 2 work is authorized yet.

Exit: the foundation uses one reusable Go/Rust bridge before provider fan-out.

Review gate: root verifies no provider-to-provider import, duplicate cgo block,
default-build Rust dependency, ABI drift, or lifecycle regression was added.

## Slice 2 — `sd_set` and `si_set`

- [x] Capture the exact accepted domain/IP rule stream during existing SRS
  parsing without introducing a second parse or changing rule counts.
- [x] Build Go and optional Rust candidates off-path and atomically publish one
  generation per reload.
- [x] Add Close and runtime-failure handling that cannot retire a replacement.
- [x] Add real Linux+cgo, fallback, reload, empty-set, and race tests.

Implementation/review evidence (2026-08-14): Luna captured the parser-accepted
domain and IP-prefix streams and publishes paired Go/Rust generations for
`sd_set` and `si_set`. Root review found no scope, parser/count, fallback,
generation identity, concurrent Match/reload/Close, empty-ruleset, default
build, or lifecycle regression. Focused tests, focused race tests, repeated
focused runs, provider/full Go tests, provider race/vet, default build/vet,
tagged stub tests, `CGO_ENABLED=0`, `gofmt`, and `git diff --check` passed on
macOS arm64. Linux+cgo real provider/adapter integration and race tests plus
the Linux experimental binary remain pending on a Linux host; this is the
Slice 1 platform caveat and is not claimed as passed here. No Slice 3+ work,
stage, commit, archive, or unrelated-path change was performed.

Exit: both real online providers can use the Rust core explicitly and fall back
to a current, never-stale Go generation.

Review gate: root stress-checks generation identity, concurrent swap/close, SRS
count parity, runtime circuit breaking, and empty/partial reload behavior.

## Slice 3 — Pure Rust valued mapper

- [x] Add result payload types, inheritance, pooling, overlap merging, and
  deterministic ordering to `matcher-core`.
- [x] Add property/golden/malformed tests for all rule/result combinations,
  invalid regex, large result sets, empty snapshots, and concurrent reads.
- [x] Add versioned create/match/len/close ABI functions, typed handle namespace,
  caller-owned result encoding, header declarations, and misuse tests.

Implementation/review evidence (2026-08-14): Root completed Slice 3 inline after
the red ABI/import test failed as expected. `rust/matcher-core/src/valued.rs`
now validates and aggregates the four rule types, recursively inherits domain
payloads in specific-to-broad order, pools immutable result payloads, and emits
sorted/deduplicated marks and first-seen joined tags/sources. Versioned,
length-safe rule/result streams reject invalid versions, UTF-8, truncation,
impossible counts, and trailing bytes before any snapshot is published.
`rust/runtime/src/matcher.rs` exposes the real valued create/match/len/close
symbols behind a separate `0x3000...` handle namespace, catches FFI panics,
handles lock poisoning as `Internal`, reports exact caller-buffer sizes, and
never transfers Rust-owned output memory. The checked-in header and ABI layout
tests cover capability/version constants and fixed-width result layout.

Focused Slice 3 tests (9 matcher valued tests and 5 runtime valued ABI tests)
and the full Rust workspace suite, Rust fmt/check, workspace clippy with
`-D warnings`, and release build passed.
Default `go test ./plugin/data_provider/...`, provider race, `go vet ./...`,
`go build ./...`, `go test ./...`, and `git diff --check` also passed. Linux+cgo
ABI/provider integration, sanitizer/Miri, and the experimental Linux binary
remain unavailable on this macOS arm64 host; the existing Slice 1/2 Linux
caveat is retained and no Linux result is claimed here. No Slice 4+ work,
stage, commit, archive, or unrelated-path cleanup was performed.

Exit: pure Rust and ABI results match the frozen Go mapper matrix.

Review gate: root audits result ordering, deduplication, buffer ownership,
malformed lengths, typed handle namespace, panic containment, and regex parity.

## Slice 4 — `domain_mapper` integration and light-provider fan-out

- [x] Refactor rebuild into explicit aggregation plus Go/Rust candidate builders
  without changing provider iteration, source metadata, invalid-rule, or
  notification behavior.
- [x] Publish one static generation and merge it with the Go `QuickAdd` hot map.
- [x] Keep `domain_set_light`/`sd_set_light` constant-false and prove their rules
  participate through the mapper without resident handles.
- [x] Verify `FastMatch`, `Exec`, `GetFastExec`, defaults, query-context fields,
  dynamic updates, rebuild failure, and concurrent rebuild/lookup/close.

Implementation/review evidence (2026-08-14): `domain_mapper` now aggregates
each provider's accepted ordered rule stream once, builds separate Go and
valued-Rust candidates off-path, and publishes the static matcher plus rebuilt
`QuickAdd` hot map under one generation read/write lock. The Rust candidate is
optional and remains behind `MOSDNS_MATCHER_BACKEND=rust`; builder failure
closes any partial candidate and publishes the current Go candidate, while a
runtime error disables only the active valued snapshot and falls back to that
same generation's Go matcher. `Close` serializes with rebuilds, is idempotent,
and retires only the generation it selected.

The shared adapter now owns valued rule/result types, versioned length-safe
encoding/decoding, capability negotiation, caller-owned output buffers, and
the valued snapshot circuit breaker. New mapper tests cover static/overlap
metadata, light-provider fan-out, `FastMatch`, `Exec`, `GetFastExec`, defaults,
QuickAdd merge, build failure, runtime fallback, replacement/close identity,
and the blocked lookup generation gate. Linux+cgo real-ABI mapper and adapter
tests now pass on `mos-test`; macOS focused/default/race/full Go tests, tagged
stubs, `CGO_ENABLED=0`, `go vet`, `go build`, `gofmt`, `git diff --check`, and
the full Rust fmt/test/clippy/release gates also pass. No Slice 5 work, stage,
commit, archive, or unrelated-path cleanup was done before this slice.

Exit: every Phase 2 provider path reaches the Rust matcher runtime when opted in.

Review gate: root compares Go and Rust results field-by-field and verifies the
hot map cannot be combined with static data from a different rebuild.

## Slice 5 — CI, evidence, test-host smoke, and handover

- [x] Extend CI with Rust fmt/test/clippy/header, default Go full/focused/race,
  Linux+cgo provider/mapper normal+race, benchmark, and experimental build gates.
- [x] Record fixed-fixture build/lookup/allocation/result-size/cgo evidence and
  known transitional limitations.
- [x] Run isolated `mos-test` provider reload, mapper overlap/source metadata,
  concurrent query/reload, Rust failure→Go fallback, and restart smoke.
- [x] Update compatibility, benchmark, test-host, rewrite-plan, and handover docs;
  root review completed for the current working tree.

Implementation/review evidence (2026-08-14): the first Slice 5 fixture run
failed at compile time with `undefined: newSlice5MapperBenchmarkFixture`; the
minimal fixed 512-rule fixture and valued mapper benchmark then passed. The
task-owned changes are `.github/workflows/test.yml`, the benchmark/smoke
scripts, `plugin/data_provider/domain_mapper/{slice5_benchmark_fixture_test.go,rust_benchmark_linux_test.go}`,
the Linux adapter type/result fixes, and the compatibility/benchmark/test-host/
rewrite-plan/handover documents. Local `go test ./...`, focused provider and
mapper tests plus race, `CGO_ENABLED=0 go test ./...`, tagged stub tests,
`go build ./...`, `go vet ./...`, Rust fmt/test/clippy/release, `gofmt`, and
`git diff --check` passed. On isolated `mos-test`, Linux+cgo provider/mapper
normal and race tests, valued/header ABI tests, three-run fixed-fixture
benchmarks, the full embedded-UI Rust experimental binary, the no-cgo Go-only
fallback binary, and the mapper-aware reload/source-audit/fallback/restart
smoke passed. No Phase 3 work or unrelated-path cleanup was performed.

Exit: implementation and validation evidence is complete; Phase 2 is ready for
the authorized commit/archive sequence. Phase 3 is not started. Rust remains
experimental/default Go-only.

Review gate: root performs the final independent full-scope audit, checks the
exact commit manifest, and separately authorizes commit/archive/journal steps.

## Required validation families

```text
gofmt and git diff --check on task-owned files
go test ./pkg/matcher/... ./plugin/data_provider/...
go test -race <changed provider and mapper packages>
go build ./...
go vet ./...
go test ./...
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo test --manifest-path rust/Cargo.toml --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path rust/Cargo.toml --release --locked
Linux+cgo tagged provider/mapper tests and race tests
scripts/build-rust-experimental.sh
isolated mos-test smoke on random high ports
```

## Risky boundaries and rollback points

- Refactor the shared adapter before adding consumers; existing direct matcher
  parity is the rollback gate.
- Never publish a Rust handle built from a different rule generation than its
  Go fallback.
- Preserve the light providers' zero-matcher contract.
- Rust mapper output is untrusted FFI data on the Go side and must be length-
  checked before use.
- Do not change query-context ownership or optimize the per-query cgo boundary
  in this task.
- No production service replacement or default-backend switch is authorized.
