# Implementation plan: Phase 2 matcher correctness remediation

## Start gate

- Keep the task in `planning` until root reviews this PRD, design, and plan.
- After explicit approval, run `task.py start` and load `trellis-before-dev`
  for the Rust matcher and affected provider packages.
- Do not stage, commit, archive, start Phase 3B, or touch cache/Phase 0/ABI
  cleanup in this task.
- Execute one slice at a time. Stop after each slice and report the red test,
  minimum implementation, focused verification, and remaining caveats for
  root review before starting the next slice.

## Slice 0 — regexp compatibility and normalization gate

### Red tests

- Add Rust matcher-core tests showing that Go-compatible ASCII patterns remain
  accepted and that `\\w`, `[\\w]`, `\\d`, `\\s`, `(?:foo)`, `(?i:foo)`, and
  `\\p{Han}` are rejected even when Rust can compile some of them. Exercise
  `\\\\w`, `\\.`, `[.]`, `[a-z]`, and `[^a-z]` to prove escape/class state is
  parsed correctly.
- Add Rust core tests proving non-ASCII `full`, `domain`, `keyword`, and
  regexp rules are rejected before a matcher can be built.
- Add runtime ABI tests proving a direct non-ASCII domain query returns the
  existing invalid-argument status rather than a match result.
- Add a valued-domain construction test proving the shared validator is used
  instead of a private direct `regex::Regex::new` path.
- Add Go adapter/provider tests for non-ASCII rule batches and non-ASCII query
  input; the Rust candidate must not be selected, while Go matching remains
  available. Include `base_domain` anonymous Rust wrapper coverage: the
  non-ASCII query must reach the paired Go matcher, leave the Rust handle
  healthy, and allow the next ASCII query to use Rust.
- Preserve a Go oracle fixture that demonstrates the compileable-but-divergent
  `^\\w+$` result rather than testing compiler failure only.

### Minimum implementation

- Implement the shared Rust regexp validator and route `RegexMatcher`,
  `MixMatcher`, and `ValuedDomainMatcher` through it.
- Make matcher-core the final ASCII rule boundary for every domain rule kind
  and add the runtime query guard; keep the Linux adapter's ASCII rule
  preflight and shared query safety helper as defense-in-depth. Guard
  `domain_set`, `sd_set`, `base_domain`, and `domain_mapper` Rust calls.
- Keep ABI symbols, status numbers, handle namespaces, and default Go-only
  selection unchanged.

### Verification and stop

- `cargo fmt --manifest-path rust/Cargo.toml --all --check`
- focused matcher-core tests and `cargo clippy -p mosdns-matcher-core --all-targets --locked -- -D warnings`
- focused Go matcher-adapter/domain_set/sd_set/domain_mapper/base_domain tests,
  race tests, and `go vet` for those packages
- `git diff --check`
- Stop for root review; do not begin Slice 1 until approved.

### Root review status — Slice 0 closed (2026-08-17)

- Root review approved Slice 0 after the final class-leading `]` state-machine
  fix. The validator now conservatively rejects `[]&&]`, `[^]&&]`, `[]--]`,
  and `[]~~]` while retaining ordinary/negated classes and escaped items.
- Go normalization goldens now freeze `例.EXAMPLE.` → `例.example`,
  `Ä.EXAMPLE.` → `ä.example`, and `İ.EXAMPLE.` → `i.example`.
- Focused Rust/Go normal, race, vet, workspace tests, and `git diff --check`
  passed. Linux+cgo real adapter evidence remains a later integration gate.
- No Slice 1 code has started. The next gate is transactional
  `domain_set`/`ip_set` fallback only; do not modify `sd_set`, `si_set`, or
  `domain_mapper` lifecycle in that slice.

## Slice 1 — transactional `domain_set` and `ip_set` fallback

### Red tests

- Change the existing domain_set POST build-failure expectation from HTTP 500
  to the required success/new-Go-generation contract, retaining assertions
  that the old generation is not mixed into the new one.
- Add domain_set assertions for new Go matching, Rust handle absence, old Rust
  close count, and publication-before-close ordering.
- Add ip_set POST and `/flush` build-failure tests with the same assertions;
  include persistence failures to prove the old generation is retained when
  Go file operations fail.

### Minimum implementation

- Keep the existing relative order: build the Go candidate, attempt the Rust
  candidate, persist the accepted rules/prefixes, then publish the paired
  generation. A Rust-only build error is non-fatal; a persistence error closes
  the new Rust candidate and preserves the old generation.
- Convert Rust build errors into warnings/Go-only generation for domain_set and
  ip_set POST/flush; close only retired handles after replacement publication.
- Preserve existing response bodies/statuses for valid Go updates and existing
  errors for invalid JSON/Go parsing/file persistence.

### Verification and stop

- focused normal and `-race` tests for `domain_set` and `ip_set`
- `go vet ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set`
- lifecycle/close-count and concurrent reader tests
- `git diff --check`
- Stop for root review; do not begin Slice 2 until approved.

### Slice 1 implementation status — CLOSED / APPROVED (2026-08-17)

- Red tests first reproduced HTTP 500 for Rust-only build failures in
  `domain_set` POST and `ip_set` POST/flush.
- The minimum implementation now logs a warning, discards any Rust candidate
  on build error, persists the accepted Go candidate, publishes the Go-only
  generation, and closes the retired Rust handle after publication.
- Persistence-failure and invalid-JSON tests retain the old generation and
  close a built-but-unpublished candidate exactly once.
- `TestDomainSetPostBuildDoesNotBlockMatch` and
  `TestIPSetFlushBuildDoesNotBlockMatch` prove readers continue using the old
  generation while candidate construction is blocked.
- Focused normal, `CGO_ENABLED=0`, race, vet, full `go test ./...`, full
  `go vet ./...`, task validation, and `git diff --check` passed.
- Linux+cgo real-staticlib evidence remains a platform gate; no Slice 2,
  spec update, staging, commit, or archive was performed. External root review
  explicitly approved Slice 1; the task remains in_progress for the next gate.

## Slice 2 — cross-provider parity and integration evidence

### Red tests

- Add sd_set regressions proving unsafe regexp rules publish Go-only while the
  accepted source generation remains usable.
- Add valued domain_mapper regressions proving the same policy and that a
  non-ASCII query falls back to Go without disabling the next ASCII Rust query.
- Review si_set's existing reload/update generation build-failure coverage;
  extend it only if same-generation fallback and close lifecycle are not
  already asserted. Do not add or assume `/post` or `/flush` for si_set.
- On Linux+cgo, add real adapter evidence for unsafe regexp rejection and
  ASCII-compatible acceptance; retain stub/default and `CGO_ENABLED=0` cases.

### Minimum implementation

- Fill only any remaining provider guard or test seam required by the frozen
  Slice 0/1 contracts. Do not broaden the compatibility policy or redesign
  matcher generations.

### Verification and stop

- focused provider normal/race tests and Linux+cgo tagged tests when available
- Rust workspace fmt, all-targets locked tests, clippy, and release build
- Go query-context/server-handler focused gates, full default Go-only tests,
  vet, build, and `CGO_ENABLED=0` tests
- Confirm no new ABI symbols, no default Rust selector, and no unrelated files
- Stop for final root review; the task is not archived automatically.

### Slice 2 implementation status — CLOSED / APPROVED (2026-08-17)

- Added an `sd_set` regression through `reloadAllRules` proving an unsafe
  regexp Rust candidate is closed once, the new generation is Go-only, and
  the accepted full rule plus Go's `^\\w+$` regexp remain usable.
- Added a valued `domain_mapper` regression through the real mapper rebuild
  proving the same unsafe-regexp policy, candidate cleanup, and complete Go
  source-generation retention. The existing Slice 0 valued compatibility
  regression continues to prove non-ASCII query-level Go fallback without
  disabling the next ASCII Rust query.
- Reviewed existing `si_set` Slice 2 generation tests; they already cover
  accepted prefix builds, Rust build failure publication, same-generation
  runtime fallback, in-flight reader/close ordering, empty generations, and
  idempotent close. No `si_set` API or lifecycle changes were needed.
- Added Linux+cgo tagged real-adapter evidence for unsafe regexp rejection and
  ASCII-safe regexp acceptance; the existing real valued mapper integration
  covers safe valued-regexp acceptance.
- No Slice 2 production implementation, ABI, selector, spec, staging, commit,
  archive, or later-phase changes were made. The frozen Slice 0/1 production
  contracts were sufficient; this slice adds only cross-provider evidence.
- Focused normal/race/vet, full Go tests/vet/build, `CGO_ENABLED=0` tests,
  query-context/server-handler gates, Rust fmt/workspace locked tests/clippy/
  release build, task validation, and `git diff --check` passed.
- Linux+cgo real-staticlib execution remains a Darwin arm64 platform caveat;
  the tagged tests are present for the Linux gate and local stub/default and
  no-cgo evidence passed.
- External root review explicitly returned `Slice 2 APPROVED` with no
  required changes. Stop here; do not start Slice 3 or any later phase.

## Final gate (after all slice reviews)

- Review the complete diff against the three-item scope and the non-goals.
- Refresh the task evidence with exact commands and platform caveats.
- Run `trellis-update-spec` to replace the stale Rust-build-failure rule in
  `.trellis/spec/backend/rust-migration.md` with the verified Go-only
  publication contract.
- Only after root approval may the normal Trellis finish/commit/archive flow be
  considered; this plan itself does not authorize those actions.

## Final Gate evidence — 2026-08-18

- Reviewed the complete task diff against the Slice 0/1/2 scope and non-goals.
  The task-owned changes remain limited to matcher compatibility, transactional
  `domain_set`/`ip_set` fallback, cross-provider evidence, task evidence, and
  the required backend contract update. No Phase 3B work, ABI change, default
  Rust selector, cache/sequence/upstream/WebUI/coremain/OpenWrt change, stage,
  commit, or archive was introduced; unrelated dirty files remain untouched.
- Applied `trellis-update-spec` to
  `.trellis/spec/backend/rust-migration.md`: Rust-only build failure now
  explicitly permits warning plus Go-only generation publication after
  persistence, while Go parse/persistence failure retains the old complete
  generation and closes an unpublished Rust candidate.
- All final verification commands passed:
  - `go test -count=1 ./...`
  - `go test -race -count=1 ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter ./plugin/matcher/base_domain`
  - `go vet ./...`
  - `go build ./...`
  - `CGO_ENABLED=0 go test -count=1 ./...`
  - `CGO_ENABLED=0 go build ./...`
  - `go test -count=1 ./pkg/query_context ./pkg/query_context/rust_bridge ./pkg/server_handler`
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked`
  - `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings`
  - `cargo build --manifest-path rust/Cargo.toml --workspace --release --locked`
  - `python3 ./.trellis/scripts/task.py validate .trellis/tasks/08-17-rust-phase2-matcher-correctness-remediation`
  - `git diff --check`
  - `gofmt -d` on the three new Slice 2 Go test files (no output).
- Linux+cgo real-staticlib tagged adapter execution remains a Darwin arm64
  platform caveat. The tagged tests are present for the Linux gate; local
  default/stub and `CGO_ENABLED=0` evidence passed.
- Webpage GPT final review: `FINAL TASK APPROVED` (2026-08-18); REQUEST
  CHANGES: none. The review confirmed the Rust-build-failure → Go-only
  publication contract, generation pairing and close ordering, rollback and
  candidate cleanup, regexp/Unicode boundaries, default Go-only selection,
  ABI/non-goals, and the Darwin arm64 Linux+cgo caveat. No file or line needs
  modification. The task intentionally remains `in_progress`; no stage,
  commit, archive, or Phase 3B start was performed.
