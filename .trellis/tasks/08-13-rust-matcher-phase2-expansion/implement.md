# Rust matcher Phase 2 expansion — implementation plan

Each slice starts with a failing behavior test and keeps the default Go build
green. Do not stage or modify the unrelated dirty WebUI/coremain worktree.

## External implementer handoff

The implementation owner is the separately opened Luna session. The current
root session owns planning and acceptance review only; it does not dispatch the
implementer from this inline Trellis session.

Before changing code, Luna must:

1. read `AGENTS.md`, `docs/ai/project-context.md`, `docs/ai/config-notes.md`,
   `docs/ai/handover.md`, `docs/ai/rust-handover.md`, and
   `docs/ai/rust-rewrite-plan.md` in that order;
2. read this task's `prd.md`, `design.md`, and `implement.md` completely;
3. verify branch `rust`, inspect the dirty worktree, and preserve every path
   identified as user-owned in `docs/ai/rust-handover.md`;
4. load `trellis-before-dev` before implementation and activate this exact task
   only after the user approves the final planning summary;
5. implement one slice at a time with red test → minimal green change → focused
   validation, without staging, committing, archiving, or starting Phase 3.

After each slice, Luna must report to the root reviewer through Herdr with:

- slice number and completed checklist items;
- exact changed-path list and any intentionally untouched risky paths;
- red-test evidence and final focused/full validation commands with results;
- fallback, concurrency, lifecycle, ABI, and compatibility decisions made;
- known limitations or unresolved review questions;
- confirmation that no unrelated file was staged, committed, or overwritten.

Luna must stop after each slice and wait for root review. A failed review returns
the same slice to Luna with concrete findings; later slices do not begin until
the current slice passes. Only the root reviewer may declare Phase 2 complete
or authorize the Trellis finish/archive sequence.

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

- [ ] Add the internal real/stub Rust matcher adapter and move ABI negotiation,
  domain/IP handles, result buffers, close, and circuit-breaker behavior into it.
- [ ] Convert `domain_set` and `ip_set` to the shared adapter without public or
  observable behavior changes.
- [ ] Run existing golden, integration, ABI, default-build, and race gates.

Exit: the foundation uses one reusable Go/Rust bridge before provider fan-out.

Review gate: root verifies no provider-to-provider import, duplicate cgo block,
default-build Rust dependency, ABI drift, or lifecycle regression was added.

## Slice 2 — `sd_set` and `si_set`

- [ ] Capture the exact accepted domain/IP rule stream during existing SRS
  parsing without introducing a second parse or changing rule counts.
- [ ] Build Go and optional Rust candidates off-path and atomically publish one
  generation per reload.
- [ ] Add Close and runtime-failure handling that cannot retire a replacement.
- [ ] Add real Linux+cgo, fallback, reload, empty-set, and race tests.

Exit: both real online providers can use the Rust core explicitly and fall back
to a current, never-stale Go generation.

Review gate: root stress-checks generation identity, concurrent swap/close, SRS
count parity, runtime circuit breaking, and empty/partial reload behavior.

## Slice 3 — Pure Rust valued mapper

- [ ] Add result payload types, inheritance, pooling, overlap merging, and
  deterministic ordering to `matcher-core`.
- [ ] Add property/golden/malformed tests for all rule/result combinations,
  invalid regex, large result sets, empty snapshots, and concurrent reads.
- [ ] Add versioned create/match/len/close ABI functions, typed handle namespace,
  caller-owned result encoding, header declarations, and misuse tests.

Exit: pure Rust and ABI results match the frozen Go mapper matrix.

Review gate: root audits result ordering, deduplication, buffer ownership,
malformed lengths, typed handle namespace, panic containment, and regex parity.

## Slice 4 — `domain_mapper` integration and light-provider fan-out

- [ ] Refactor rebuild into explicit aggregation plus Go/Rust candidate builders
  without changing provider iteration, source metadata, invalid-rule, or
  notification behavior.
- [ ] Publish one static generation and merge it with the Go `QuickAdd` hot map.
- [ ] Keep `domain_set_light`/`sd_set_light` constant-false and prove their rules
  participate through the mapper without resident handles.
- [ ] Verify `FastMatch`, `Exec`, `GetFastExec`, defaults, query-context fields,
  dynamic updates, rebuild failure, and concurrent rebuild/lookup/close.

Exit: every Phase 2 provider path reaches the Rust matcher runtime when opted in.

Review gate: root compares Go and Rust results field-by-field and verifies the
hot map cannot be combined with static data from a different rebuild.

## Slice 5 — CI, evidence, test-host smoke, and handover

- [ ] Extend CI with Rust fmt/test/clippy/header, default Go full/focused/race,
  Linux+cgo provider/mapper normal+race, benchmark, and experimental build gates.
- [ ] Record fixed-fixture build/lookup/allocation/result-size/cgo evidence and
  known transitional limitations.
- [ ] Run isolated `mos-test` provider reload, mapper overlap/source metadata,
  concurrent query/reload, Rust failure→Go fallback, and restart smoke.
- [ ] Update compatibility, benchmark, test-host, rewrite-plan, and handover docs;
  mark Phase 2 complete only after independent full-scope review.

Exit: Phase 2 is archived and Phase 3 may be planned separately; Rust remains
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
