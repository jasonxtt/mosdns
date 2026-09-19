# Implementation plan — host-aware Codex automation routing

## Preconditions and gates

- User explicitly approved self-execution and self-review in this conversation;
  the task is active (`in_progress`) and the main Codex session owns execution.
- Run `trellis-before-dev` for the `.trellis`/`.codex` integration package
  before editing implementation files.
- Preserve the existing unrelated dirty files, especially the active Rust
  tasks and `.DS_Store` files; stage exact paths only.
- Do not touch MosDNS product/runtime code or start QUIC Slice 1.
- Each implementation slice follows RED test -> minimum GREEN change ->
  focused checks; the parent controller owns diff inspection and final checks.

## Slice 1 — Generic targets and v1 migration ✅

- Add failing tests for provider/reference targets, current-Codex self targets,
  arbitrary user-selected references, independent slot invalidation, and v1
  migration.
- Replace the closed `dispatch`/ChatGPT-only reviewer validation with a
  versioned generic target schema while retaining v1 reads.
- Add provider-neutral set/clear/show command arguments and safe identity
  validation.
- Focused routing tests pass; provider-neutral CLI operations are covered by
  the script boundary smoke checks below.

## Slice 2 — Surface detection and host policy ✅

- Add failing fixtures for explicit surface, Desktop App marker, explicit CLI
  marker, conflicting evidence, and unknown evidence.
- Implement injectable evidence resolution and safe marker-name reporting.
- Extend configuration parsing with `auto`, `dsh`, and `host_routes`, while
  preserving explicit legacy modes and invalid-value fail-closed behavior.
- Change the repository default to the host-aware policy only after all
  parsers/tests agree on the new modes.
- Config, surface, and workflow-platform focused tests pass.

## Slice 3 — Dynamic hook/workflow and selection prompt ✅

- Add failing tests for dynamic mode banners, `codex-cli`/`codex-desktop`
  policy output, unknown-surface prompts, and one combined missing-slot prompt.
- Update hook state injection and workflow platform filtering to use the
  resolved policy/provider rather than assuming Herdr.
- Show the current surface/evidence and user-overridable executor/reviewer
  forms without auto-selecting concrete resources.
- Update `.trellis/workflow.md` and the project quality guidance with the
  controller/provider contracts.
- Hook/workflow tests and a manual hook fixture inspection pass.

## Slice 4 — Integration, compatibility, and review readiness (in progress)

- Exercise v1 state fixtures, explicit self-routing, Herdr candidate
  invalidation, DSH routing selection, and reviewer replacement end to end at
  the script boundary.
- Verify no fixed reviewer title, URL, pane position, or agent brand remains
  in active routing code/guidance.
- Curate task context manifests with the final specs/research entries.
- Run the full targeted Python test suite, formatting, `git diff --check`,
  `task.py validate`, and exact changed-path review.
- No external dispatch or reviewer handoff is needed: the user explicitly
  selected the current Codex session for both roles. Complete the parent-owned
  full-scope check and report the self-review result.

## Validation commands

```bash
python3 -m unittest discover -s .trellis/tests -p 'test_*.py'
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-19-codex-host-automation-routing
python3 ./.trellis/scripts/codex_routing.py show
git diff --check
```

If the repository's test runner requires a narrower invocation, record the
actual command and result in `implement.md` before the final review.

## Execution record

- `python3 -m unittest discover -s .trellis/tests -p 'test_*.py'` — 26 tests
  passed.
- Python compile checks for the changed routing, workflow, and hook modules —
  passed.
- `python3 ./.trellis/scripts/task.py validate
  .trellis/tasks/09-19-codex-host-automation-routing` — passed.
- `git diff --check` — passed.
- Manual hook fixture — Desktop evidence resolved to `dsh` by default, while
  this conversation's explicit `codex/current` targets produced the inline
  self-execution banner and preserved the same reviewer target.

## Risk and rollback points

- Surface marker availability is the main technical risk. Unknown evidence
  must remain a safe, user-actionable state rather than being guessed.
- v1 state migration must not invalidate existing explicit Herdr selections.
- Hook/workflow changes can alter the injected per-turn contract; test both
  current Desktop evidence and synthetic CLI fixtures.
- Revert only the exact routing/config/docs/test paths if a slice fails.
