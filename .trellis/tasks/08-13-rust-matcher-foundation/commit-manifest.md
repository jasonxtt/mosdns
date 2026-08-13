# Ordered exact-scope commit-series manifest

Date: `2026-08-13`

This is a preflight manifest for an ordered, reviewable commit series. It is
not a staging instruction by itself. The matcher task remains `in_progress`:
independent review passed on `2026-08-13`, and the next gates are an
exact-scope commit series followed by the Trellis finish gate. The task
metadata remains `commit: null`. Do not use `git add -A`.

The work series follows the module/contract/core/bridge/build-and-document
split in `docs/ai/rust-rewrite-plan.md`. A–E are the work commits for the
current pre-finish snapshot. F/G are the later Trellis finish commits and
their generated paths are classified below in advance. Every path in the
current tracked diff or untracked enumeration must be classified exactly once
as one of A–E, `REVIEW REQUIRED`, or `EXCLUDE`; every dynamic F/G output is
classified by its exact source/target path below. A path not listed there is
not a commit input.

The current A–E snapshot was checked against 25 tracked diff paths and 188
untracked file paths (213 total). F/G generated outputs are intentionally not
included in that 213-path number.

## Commit A — Trellis/governance, planning, specs, and task history

This commit establishes the project-level Trellis/Codex integration and the
planning/spec/task history. The listed `.codex` files are project files, not a
blanket inclusion of `.codex`.

```text
.gitattributes
.gitignore
AGENTS.md

.agents/skills/trellis-before-dev/SKILL.md
.agents/skills/trellis-brainstorm/SKILL.md
.agents/skills/trellis-break-loop/SKILL.md
.agents/skills/trellis-channel/SKILL.md
.agents/skills/trellis-channel/references/command-reference.md
.agents/skills/trellis-channel/references/forum.md
.agents/skills/trellis-channel/references/progress-debugging.md
.agents/skills/trellis-channel/references/workers.md
.agents/skills/trellis-channel/references/workflows.md
.agents/skills/trellis-check/SKILL.md
.agents/skills/trellis-continue/SKILL.md
.agents/skills/trellis-finish-work/SKILL.md
.agents/skills/trellis-meta/SKILL.md
.agents/skills/trellis-meta/references/customize-local/add-project-local-conventions.md
.agents/skills/trellis-meta/references/customize-local/change-agents.md
.agents/skills/trellis-meta/references/customize-local/change-context-loading.md
.agents/skills/trellis-meta/references/customize-local/change-hooks.md
.agents/skills/trellis-meta/references/customize-local/change-skills-or-commands.md
.agents/skills/trellis-meta/references/customize-local/change-spec-structure.md
.agents/skills/trellis-meta/references/customize-local/change-task-lifecycle.md
.agents/skills/trellis-meta/references/customize-local/change-workflow.md
.agents/skills/trellis-meta/references/customize-local/overview.md
.agents/skills/trellis-meta/references/local-architecture/bundled-skills.md
.agents/skills/trellis-meta/references/local-architecture/context-injection.md
.agents/skills/trellis-meta/references/local-architecture/generated-files.md
.agents/skills/trellis-meta/references/local-architecture/multi-agent-channel.md
.agents/skills/trellis-meta/references/local-architecture/overview.md
.agents/skills/trellis-meta/references/local-architecture/spec-system.md
.agents/skills/trellis-meta/references/local-architecture/task-system.md
.agents/skills/trellis-meta/references/local-architecture/workflow.md
.agents/skills/trellis-meta/references/local-architecture/workspace-memory.md
.agents/skills/trellis-meta/references/platform-files/agents.md
.agents/skills/trellis-meta/references/platform-files/hooks-and-settings.md
.agents/skills/trellis-meta/references/platform-files/overview.md
.agents/skills/trellis-meta/references/platform-files/platform-map.md
.agents/skills/trellis-meta/references/platform-files/skills-and-commands.md
.agents/skills/trellis-session-insight/SKILL.md
.agents/skills/trellis-session-insight/references/cli-quick-reference.md
.agents/skills/trellis-session-insight/references/triggering-patterns.md
.agents/skills/trellis-spec-bootstrap/SKILL.md
.agents/skills/trellis-spec-bootstrap/references/mcp-setup.md
.agents/skills/trellis-spec-bootstrap/references/repository-analysis.md
.agents/skills/trellis-spec-bootstrap/references/spec-task-planning.md
.agents/skills/trellis-spec-bootstrap/references/spec-writing.md
.agents/skills/trellis-start/SKILL.md
.agents/skills/trellis-update-spec/SKILL.md

.codex/agents/trellis-check.toml
.codex/agents/trellis-implement.toml
.codex/agents/trellis-research.toml
.codex/config.toml
.codex/hooks.json
.codex/hooks/inject-subagent-context.py
.codex/hooks/inject-workflow-state.py
.codex/hooks/session-start.py

.trellis/.gitignore
.trellis/.template-hashes.json
.trellis/.version
.trellis/agents/check.md
.trellis/agents/implement.md
.trellis/config.yaml
.trellis/scripts/__init__.py
.trellis/scripts/add_session.py
.trellis/scripts/common/__init__.py
.trellis/scripts/common/active_task.py
.trellis/scripts/common/cli_adapter.py
.trellis/scripts/common/config.py
.trellis/scripts/common/developer.py
.trellis/scripts/common/git.py
.trellis/scripts/common/git_context.py
.trellis/scripts/common/io.py
.trellis/scripts/common/log.py
.trellis/scripts/common/packages_context.py
.trellis/scripts/common/paths.py
.trellis/scripts/common/safe_commit.py
.trellis/scripts/common/session_context.py
.trellis/scripts/common/task_context.py
.trellis/scripts/common/task_queue.py
.trellis/scripts/common/task_store.py
.trellis/scripts/common/task_utils.py
.trellis/scripts/common/tasks.py
.trellis/scripts/common/trellis_config.py
.trellis/scripts/common/types.py
.trellis/scripts/common/workflow_phase.py
.trellis/scripts/get_context.py
.trellis/scripts/get_developer.py
.trellis/scripts/hooks/linear_sync.py
.trellis/scripts/init_developer.py
.trellis/scripts/task.py
.trellis/spec/backend/config-compatibility.md
.trellis/spec/backend/directory-structure.md
.trellis/spec/backend/error-handling.md
.trellis/spec/backend/index.md
.trellis/spec/backend/logging-guidelines.md
.trellis/spec/backend/quality-guidelines.md
.trellis/spec/backend/rust-migration.md
.trellis/spec/guides/code-reuse-thinking-guide.md
.trellis/spec/guides/cross-layer-thinking-guide.md
.trellis/spec/guides/index.md
.trellis/tasks/archive/2026-08/00-bootstrap-guidelines/prd.md
.trellis/tasks/archive/2026-08/00-bootstrap-guidelines/task.json
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/check.jsonl
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/design.md
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/implement.jsonl
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/implement.md
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/prd.md
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/task.json
.trellis/tasks/08-13-rust-matcher-foundation/check.jsonl
.trellis/tasks/08-13-rust-matcher-foundation/commit-manifest.md
.trellis/tasks/08-13-rust-matcher-foundation/design.md
.trellis/tasks/08-13-rust-matcher-foundation/implement.jsonl
.trellis/tasks/08-13-rust-matcher-foundation/implement.md
.trellis/tasks/08-13-rust-matcher-foundation/prd.md
.trellis/tasks/08-13-rust-matcher-foundation/task.json
.trellis/workflow.md
.trellis/workspace/index.md
.trellis/workspace/tom/index.md
.trellis/workspace/tom/journal-1.md

docs/ai/rust-rewrite-plan.md
```

The backend error/logging specs are included because `backend/index.md`
references them. The guides are included because `workflow.md`,
`trellis-start`, and `trellis-before-dev` read them directly. The bootstrap
task archive is included as the source history for the generated specs. The
workspace index and journal are included as Trellis cross-session memory; they
are not disposable local state. These current files were inspected as
project-generated text and the credential scan found no real credentials or
local database files. `.trellis/config.yaml` is included with
`session_auto_commit: false`; that setting is a deliberate guard for the
finish phase and must remain false.

## Commit B — Rust cache foundation and Go compatibility/FFI tests

This commit owns the cache core and its Go-side compatibility, lifecycle, raw
wire, and FFI-facing tests. The query-context, server-handler, and audit raw
changes belong here because they provide the cache foundation's raw ownership
and ABI coverage. The Go bridge paths are reviewed here even though their
tagged link gate uses the unified runtime supplied by commit C.

```text
plugin/executable/cache/cache.go
plugin/executable/cache/cache_test.go
plugin/executable/cache/cache_contract_test.go
plugin/executable/cache/cache_dump_transaction_test.go
plugin/executable/cache/rust_backend.go
plugin/executable/cache/rust_backend_benchmark_linux_test.go
plugin/executable/cache/rust_backend_linux.go
plugin/executable/cache/rust_backend_linux_test.go
plugin/executable/cache/rust_backend_soak_test.go
plugin/executable/cache/rust_backend_stub.go
plugin/executable/cache/rust_backend_test.go
plugin/executable/cache/testdata/rust-smoke.yaml

rust/cache-core/Cargo.toml
rust/cache-core/src/lib.rs
rust/cache-core/src/wire.rs

pkg/query_context/context.go
pkg/query_context/context_raw_test.go
pkg/server_handler/entry_handler.go
pkg/server_handler/entry_handler_raw_test.go
coremain/audit_raw_test.go
```

## Commit C — Unified Rust runtime, matcher core, and ABI/header tests

This commit supplies the one Rust staticlib workspace, preserves cache
exports, adds matcher-core, and owns the matcher ABI/header contract tests.
After C lands, the tagged B+C integration link/build gate can be run before
reviewing adapters in D.

```text
rust/Cargo.lock
rust/Cargo.toml
rust/matcher-core/Cargo.toml
rust/matcher-core/src/full.rs
rust/matcher-core/src/ipnet.rs
rust/matcher-core/src/keyword.rs
rust/matcher-core/src/lib.rs
rust/matcher-core/src/mix.rs
rust/matcher-core/src/normalize.rs
rust/matcher-core/src/regex.rs
rust/matcher-core/src/trie.rs
rust/runtime/Cargo.toml
rust/runtime/include/mosdns_cache_core.h
rust/runtime/src/lib.rs
rust/runtime/src/matcher.rs
rust/runtime/tests/abi_contract.rs
```

## Commit D — Go domain/IP adapters and matcher fixtures/tests

This commit integrates the opt-in Rust backend while retaining Go fallback and
existing matcher order/semantics. It owns the direct domain/IP providers,
base-domain/base-IP adapters, golden fixtures, Linux+cgo integration, and
benchmark coverage.

```text
pkg/matcher/domain/golden_test.go
pkg/matcher/netlist/golden_test.go

plugin/data_provider/domain_set/domain_set.go
plugin/data_provider/domain_set/golden_test.go
plugin/data_provider/domain_set/rust_backend.go
plugin/data_provider/domain_set/rust_benchmark_linux_test.go
plugin/data_provider/domain_set/rust_integration_linux_test.go
plugin/data_provider/domain_set/rust_stub.go
plugin/data_provider/domain_set/testdata/real-rule-set.txt
plugin/data_provider/ip_set/ip_set.go
plugin/data_provider/ip_set/golden_test.go
plugin/data_provider/ip_set/rust_backend.go
plugin/data_provider/ip_set/rust_benchmark_linux_test.go
plugin/data_provider/ip_set/rust_integration_linux_test.go
plugin/data_provider/ip_set/rust_stub.go
plugin/matcher/base_domain/domain_matcher.go
plugin/matcher/base_domain/rust_integration_linux_test.go
plugin/matcher/base_ip/ip_matcher.go
plugin/matcher/base_ip/rust_integration_linux_test.go
```

## Commit E — CI/build, smoke, evidence, and handover

This commit records the operational gates and evidence after A–D. Build-local
and handover files have mixed ownership notes below and require hunk-level
review.

```text
.github/workflows/test.yml
scripts/build-local.sh
scripts/build-rust-cache.sh
scripts/build-rust-experimental.sh
scripts/benchmark-rust-matchers.sh
scripts/smoke-rust-matcher-mos-test.sh

docs/ai/handover.md
docs/ai/rust-handover.md
docs/rust/benchmarks/cache-foundation.md
docs/rust/benchmarks/matcher-foundation.md
docs/rust/cache-compatibility.md
docs/rust/kixdns-reuse.md
docs/rust/matcher-compatibility.md
docs/rust/overview.html
docs/rust/test-host-cache-foundation.md
```

The series is intentionally not one cache+matcher mega-commit. B's cache
consumer/compatibility paths are kept separate from C's unified runtime and
matcher-core paths; the integrated tagged build is verified after C. D then
adds only direct domain/IP adapters, and E records the CI/build/smoke/evidence
and handover changes. Provider fan-out and `domain_mapper` remain outside the
five work commits.

## Commit F — `chore(task): archive rust-matcher-foundation`

F is the first finish-phase commit and runs only after A–E have been committed
and their exact-scope diffs are reviewed. `session_auto_commit: false` is
required, and the explicit `--no-commit` is retained as a second guard:

```bash
python3 ./.trellis/scripts/task.py archive \
  .trellis/tasks/08-13-rust-matcher-foundation \
  --no-commit
```

The exact task path transition is:

```text
source moved away:
  .trellis/tasks/08-13-rust-matcher-foundation/
target created:
  .trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation/
```

`task.py archive` updates only the archived `task.json` lifecycle fields to
`status: completed` and the current `completedAt`; it does not write a
`commit` field. The archived `task.json` therefore retains `commit: null`.
It also clears matching `.trellis/.runtime/sessions/*.json` files. Those
runtime files are local state and remain excluded, not staged.

After inspecting the move, stage only the exact archive additions/updates and
source-side deletions:

```bash
git add -- .trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation
git add -u -- .trellis/tasks/08-13-rust-matcher-foundation
git diff --cached --check -- \
  .trellis/tasks/08-13-rust-matcher-foundation \
  .trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation
git diff --cached --name-status -- \
  .trellis/tasks/08-13-rust-matcher-foundation \
  .trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation
git diff --cached -- \
  .trellis/tasks/08-13-rust-matcher-foundation \
  .trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation
git commit -m 'chore(task): archive rust-matcher-foundation'
```

The `git add -u` source path is intentional: after the move it stages only
the tracked source deletions. Do not use `git add -A`, and do not stage the
workspace or any unrelated dirty path in F.

## Commit G — `chore: record journal`

G records the finish session after F. The current workspace is below the
2,000-line rotation threshold, so the exact files are:

```text
.trellis/workspace/tom/index.md
.trellis/workspace/tom/journal-1.md
```

Use full hashes for A–E only. Do not put F or G in the journal's `--commit`
value:

```bash
A_HASH='<full hash of commit A>'
B_HASH='<full hash of commit B>'
C_HASH='<full hash of commit C>'
D_HASH='<full hash of commit D>'
E_HASH='<full hash of commit E>'
WORK_HASHES="${A_HASH},${B_HASH},${C_HASH},${D_HASH},${E_HASH}"
python3 ./.trellis/scripts/add_session.py \
  --title 'Rust matcher foundation commit series' \
  --branch rust \
  --commit "${WORK_HASHES}" \
  --summary 'A-E Rust cache and matcher foundation work committed; F archived the task; G records this handoff.' \
  --next-step 'Await independent follow-up after the Trellis finish gate.' \
  --no-commit
```

The explicit `--no-commit` keeps `add_session.py` from staging or committing.
Review and stage only the two workspace files, then create G:

```bash
git add -- \
  .trellis/workspace/tom/index.md \
  .trellis/workspace/tom/journal-1.md
git diff --cached --check -- \
  .trellis/workspace/tom/index.md \
  .trellis/workspace/tom/journal-1.md
git diff --cached --name-status -- \
  .trellis/workspace/tom/index.md \
  .trellis/workspace/tom/journal-1.md
git diff --cached -- \
  .trellis/workspace/tom/index.md \
  .trellis/workspace/tom/journal-1.md
git commit -m 'chore: record journal'
```

If `add_session.py` would rotate beyond `journal-1.md`, stop and extend this
manifest with the exact new journal path before staging. Under the current
7-line journal and `max_journal_lines: 2000`, no rotation is expected. Do not
stage the deleted runtime session JSON files, `.trellis/.developer`, or any
other workspace path in G.

## A–G verification matrix and gates

Each row is a post-commit gate for that boundary; a failure blocks the next
commit. These commands are recorded for the future finish sequence and are
not being rerun in this documentation-only review.

| Commit | Required verification and threshold |
| --- | --- |
| A | Exact staged governance paths only; `session_auto_commit: false`; JSON/task/spec references parse; `task.py validate` passes; no credential/local-database content; staged diff has no unrelated hunks. |
| B | Default Go/cache/raw gate only: `go vet ./plugin/executable/cache ./pkg/query_context ./pkg/server_handler ./coremain` and `go test ./plugin/executable/cache ./pkg/query_context ./pkg/server_handler ./coremain`; no Rust build, `mosdns_rust` tag, or Linux cgo link is claimed at B alone. |
| C | On any host, cargo fmt/locked all-target tests/locked clippy with `-D warnings`/locked release build must pass; macOS cargo success is cargo-only evidence. After B+C, **on a Linux runner only** (CI or the isolated `mos-test` environment), run `scripts/build-rust-cache.sh` and both tagged bridges: `CGO_ENABLED=1 go test -tags mosdns_rust ./plugin/executable/cache ./pkg/matcher/... ./plugin/matcher/... -count=1`, plus the real provider selector `CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -tags mosdns_rust ./plugin/data_provider/domain_set/... ./plugin/data_provider/ip_set/... ./plugin/matcher/... -count=1`. A macOS `go test -tags mosdns_rust` is not a cgo pass because `rust_backend_linux.go` is excluded and the stub can compile. |
| D | Matcher golden/default tests, focused race tests, and real Linux+cgo domain/IP adapter tests pass with positive/negative Rust evidence and Go fallback semantics; use the focused package set from `.github/workflows/test.yml`, including `CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -race -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/matcher/base_domain ./plugin/matcher/base_ip -count=1`. |
| E | Full/CI-equivalent `go build ./...`, `go vet ./...`, `go test ./...`, Rust/ABI/header checks, experimental binary build, benchmark/evidence consistency, and isolated smoke all pass; docs match actual command outputs and keep Rust experimental/default Go-only. |
| F | `task.py list` no longer shows the active matcher task; `task.py list-archive 2026-08` shows `08-13-rust-matcher-foundation`; archived `task.json` has `status: completed`, non-null `completedAt`, and unchanged `commit: null`; staged diff contains only the exact source deletion/target archive paths. |
| G | `index.md` and `journal-1.md` contain the session record with A–E hashes only; staged diff contains only those two exact workspace paths; `git diff --cached --check` passes; after G, verify the exact-scope commit series and no staged paths remain. |

F/G are finish artifacts generated after the A–E snapshot. They were
pre-classified above but are deliberately not counted in the current 213-path
coverage number.

## REVIEW REQUIRED — tracked files with mixed ownership

Review each listed file hunk by hunk. Use `git add -p` or an equivalent
temporary patch selection; never stage the whole file when it contains user
changes. The comments identify the only intended Rust/Trellis hunks.

```text
.gitattributes        # A: retain only the Trellis journal merge rule and corrected spec path
.gitignore            # A: retain only the rust/target/ rule
AGENTS.md             # A: retain only Rust/Trellis instruction additions
docs/ai/handover.md   # E: retain only the Rust migration planning/status section
scripts/build-local.sh # E: retain only GO_TAGS support; preserve other build behavior

plugin/executable/cache/cache.go       # B: inspect against unrelated cache/user hunks
plugin/executable/cache/cache_test.go  # B: inspect against unrelated cache/user hunks
pkg/query_context/context.go           # B: raw-context/FFI hunks only
pkg/server_handler/entry_handler.go    # B: raw-entry/FFI hunks only
plugin/data_provider/domain_set/domain_set.go # D: matcher Slice 4/5 hunks only
plugin/data_provider/ip_set/ip_set.go          # D: matcher Slice 4/5 hunks only
plugin/matcher/base_domain/domain_matcher.go   # D: matcher Slice 4 hunks only
plugin/matcher/base_ip/ip_matcher.go           # D: matcher Slice 4 hunks only
.github/workflows/test.yml              # E: matcher/cache gate hunks only
```

Untracked Rust, adapter, test, evidence, and task files above are still
subject to normal content review; “untracked” does not mean “stage blindly.”

## EXCLUDE — unrelated user work and local/generated state

Never stage these current paths in A–E:

```text
coremain/api_system.go
coremain/config_manager.go
coremain/config_manager_api_test.go
coremain/mosdns.go
coremain/openwrt.go
coremain/openwrt_test.go
coremain/www/assets/vue-log/**
coremain/www/assets/vue-log1/**
coremain/www/log.html
coremain/www/log1.html
webui-log/src/components/SystemControlManager.vue
webui-log/src/components/system/SystemConfigManagePanel.vue

.DS_Store
docs/.DS_Store
.codegraph/**
.superpowers/**
.trellis/.developer
.trellis/.runtime/**
.trellis/scripts/**/__pycache__/**
.trellis/scripts/**/*.pyc
```

The current `.codex` inclusion is limited to the eight exact project files in
commit A; no other `.codex` path is implicitly included. The current
`.trellis/workspace` files are included in A, while developer identity,
runtime state, Python caches, personal credentials, and local databases are
excluded. No production host, installed service, port 53, or remote temporary
artifact is a repository commit input. No provider fan-out (`sd_set`,
`sd_set_light`, `domain_set_light`, `si_set`) or `domain_mapper` files belong
here; they require an independently approved continuation task.

## Pre-staging verification

Before any of A–E is staged:

1. Run `git status --short`, `git diff --name-only`, and
   `git ls-files --others --exclude-standard`.
2. Compare every result with the five path lists, `REVIEW REQUIRED`, and
   `EXCLUDE`; coverage does not imply inclusion. The 213-path number applies
   only to this A–E snapshot.
3. Inspect all mixed tracked diffs and select only intended hunks with
   `git add -p` or a temporary patch. Use explicit per-commit paths for all
   other files; never use `git add -A`.
4. Confirm `git diff --cached --name-only` is empty before beginning staging.

After each work commit A–E, review its staged diff and run its row in the
verification matrix before proceeding. Only after E is committed may the
finish phase begin. Run F's archive command with `--no-commit`, inspect and
stage only its exact source/target task paths, and commit F. Then run G's
`add_session.py` command with `--no-commit`, inspect and stage only the exact
workspace paths, and commit G. Never use `git add -A` at any phase.

This documentation-only review does not execute F/G, change task lifecycle
state, stage files, or create commits. The task remains `in_progress` with
`commit: null` until the future A–G sequence and Trellis finish gate complete.

This manifest, `task.json`, all current matcher task artifacts, and the
project-level Trellis/Codex files are part of the review scope. Do not mark
the task completed or archived before the exact series is committed and the
Trellis finish gate runs.
