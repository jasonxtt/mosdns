# Implement — Simplify Trellis Execution and Review Automation

Ordered, independently verifiable slices. Each slice: implement → focused
validation → exact diff → exact commit/push → independent reviewer →
auto-remediate scoped FAILs → PASS → auto-advance. Final slice PASS stops
without archive (per PRD §6.5).

Common rules for every slice:

- Branch: `rust`. Commit style: `refactor(trellis): ...` / `test(trellis): ...`
  matching recent history.
- Run `.trellis/tests` (python unittest) before each commit:
  `python3 -m unittest discover -s .trellis/tests -v` (adjust if the suite
  uses a different runner — confirm in Slice 0).
- Never touch MosDNS runtime code, `Cargo.*`, WebUI, `tests/phase5a-baseline`
  semantics, or deployment files.
- Stage exact paths only; inspect `git status --porcelain` before staging.

---

## Slice 0 — Automation context foundation

**Goal**: new state model + migration, with CLI, replacing
`common/codex_routing.py` as the source of truth (shim kept).

**Changes**:

- New `.trellis/scripts/common/automation.py`:
  - `AutomationContext` load/save at `.trellis/.runtime/automation/<ctx>.json`
    (`version: 1`, `executor_override`, `reviewer`, `updated_at`);
  - `resolve_executor(ctx)` → `current` or the override target;
  - generic target validation `{provider, reference, label?}`;
  - `migrate_legacy_routing(root, context_key)` per design §2.5
    (preserve explicit-user reviewer/supported executor; drop surface,
    host-route results, MCP dsh, policy-derived executors; rename legacy file
    to `.migrated`; corrupt → empty).
- New `.trellis/scripts/automation.py` CLI: `show`, `set-executor`,
  `clear-executor`, `set-reviewer`, `clear-reviewer`, `migrate`
  (conversation-scoped via `resolve_context_key`, same as today).
- Rewrite `.trellis/tests/test_codex_routing.py` → `test_automation.py`
  covering: fresh-conversation defaults (override null ⇒ current; reviewer
  missing), precedence rules, persistence, migration preserve/discard matrix,
  corrupt-state safety.

**Acceptance**: new tests pass; legacy routing tests deleted/migrated with
them; no other module imports `common.codex_routing` yet (shim added in
Slice 4 wiring; callers switch in Slice 1).

---

## Slice 1 — Workflow/hook de-routing

**Goal**: remove surface/host routing from config, hooks, workflow text.

**Changes**:

- `.trellis/config.yaml`: `codex` section reduced to
  `dispatch_mode: inline` with a short comment; delete `host_routes`.
- `.trellis/scripts/common/config.py`: `dispatch_mode` accepts
  `inline | sub-agent(auto)` (legacy `sub-agent` alias kept); `host_routes`
  parser removed or deprecation-warn-and-ignore; defaults updated.
- `.codex/hooks/inject-workflow-state.py`: delete surface detection,
  `_codex_mode_banner` provider semantics, `resolve_breadcrumb_key`
  provider/host-route branches (return plain status); replace routing banner
  with the compact automation banner (executor/reviewer/run summary;
  missing reviewer informational only).
- `.codex/hooks/session-start.py`: remove any routing/discovery coupling;
  add one-line automation summary if context exists.
- `.trellis/scripts/common/workflow_phase.py`: `resolve_effective_platform`
  drops herdr/dsh-web/auto virtual platforms.
- `.trellis/scripts/common/git_context.py`: drop
  `_resolve_codex_routing`/surface; codex phase filtering uses simplified
  mode.
- `.trellis/scripts/common/task_store.py`: jsonl seeding keyed off simplified
  dispatch mode only.
- `.trellis/workflow.md`: delete the 8 provider-variant `[workflow-state:*]`
  blocks and `[codex-herdr]` / `[codex-dsh-web, codex-auto]` marker sections;
  rewrite generic `in_progress` body to the PRD §17 target semantics; update
  the breadcrumb-contract comment's tag table.
- `.trellis/tests/test_codex_hook.py` rewritten: plain-status key resolution,
  banner content, **assert no subprocess discovery is invoked** by
  session-start/per-turn hooks (mock `subprocess.run`, assert not called with
  `herdr`/`ps`).
- Switch remaining `common.codex_routing` imports to `common.automation`.

**Acceptance**: full test suite green; grep shows no `host_routes`,
`detect_surface`, `resolve_codex_provider`, `codex-herdr`, `codex-dsh-web`
references outside the legacy shim file slated for Slice 4.

---

## Slice 2 — Authorized execution loop state

**Goal**: run-state model + snapshot semantics + auto-advance bookkeeping.

**Changes**:

- `common/automation.py` (or `common/automation_run.py`):
  - `AutomationRun` load/save per design §2.1(B);
  - `authorize(task_dir, units | "all")` → parse `implement.md` Slice
    headings, snapshot list, persist;
  - `record_unit_result`, `advance()` (PASS → next pre-authorized unit or
    `authorized_scope_complete`), guard against advancing past the snapshot;
  - status transitions: `running | blocked | authorized_scope_complete`;
    `auto_finish` is always false in v1.
- CLI: `automation.py authorize <task> --units "Slice 0-3"`,
  `run-status`, `record-pass`, `record-fail`, `complete`.
- Tests: snapshot exactness (0–3 ⇒ exactly 4 units; later implement.md edits
  don't extend), no auto-entry into unauthorized units, final PASS ⇒
  `authorized_scope_complete` and **no archive/finish invoked** (assert
  task.json status unchanged), compaction-resume (reload run mid-stream).

**Acceptance**: unit tests green; no hook behavior change in this slice
(state layer only).

---

## Slice 3 — Reviewer bootstrap + remediation loop contract

**Goal**: codify the review protocol so the controller (LLM) and tests share
one machine-readable contract.

**Changes**:

- `common/automation.py`: bootstrap template builder
  `build_review_request(run, unit, evidence)` producing the full first-request
  payload (PRD §5.4 field list incl. automation-contract education) and the
  compact re-review variant; pure function of run state + git facts
  (base/head SHA, changed paths) → fully unit-testable.
- Finding ledger: `record_review_result(run, findings[])` with stable IDs,
  closed markers, consecutive-failure counters, semantic root-cause field;
  `is_blocked()` at 5 consecutive on the same root cause.
- Review-result parser: extract `FINAL: PASS|FAIL` + findings from reviewer
  text; pending/idle/silence/partial ⇒ `pending`, never PASS.
- Runbook section in `workflow.md` (generic `in_progress` body) referencing
  these helpers as the controller's per-unit checklist.
- Tests: bootstrap contains all required fields for a new task; compact
  re-review omits history but keeps findings/parent-head/diff scope; new task
  on same reviewer ⇒ full bootstrap again; counter matrix (1–4 continue, 5 ⇒
  blocked, new finding independent, closed stops counting, renamed-ID same
  root cause still counts); pending ≠ PASS; out-of-scope finding ⇒ blocked
  immediately.

**Acceptance**: tests green; template output snapshot-reviewed.

---

## Slice 4 — Cleanup and compatibility

**Goal**: remove legacy routing semantics; leave at most a thin shim.

**Changes**:

- `common/codex_routing.py` + `codex_routing.py`: reduce to deprecated
  forwarding shims (load/save → automation context with migration;
  surface/dispatch/discovery APIs raise or no-op with guidance), or delete
  outright if no imports remain.
- `.trellis/spec/backend/quality-guidelines.md`: replace "Host-aware Codex
  routing" section with the new automation contract.
- Negative regression tests (PRD §24): CLI marker ⇏ herdr; Desktop ⇏
  dsh-web; missing providers don't block; defaults ≯ explicit target;
  provider ≠ granularity; final PASS ≠ archive.
- Full Trellis regression: entire `.trellis/tests` suite + hook smoke runs.
- Verify zero MosDNS runtime diff: `git diff` restricted to `.trellis/`,
  `.codex/`.

**Acceptance**: suite green; `grep -r "codex_routing\|host_routes\|
detect_surface\|resolve_codex_provider"` returns only the shim (if kept) and
its deprecation test; final report per PRD §6.5; task left `in_progress`.

---

## Validation commands

- `python3 -m unittest discover -s .trellis/tests -v`
- `python3 .trellis/scripts/task.py validate .trellis/tasks/09-21-trellis-automation-simplification`
- Hook smoke: `echo '{"cwd":"."}' | python3 .codex/hooks/inject-workflow-state.py`
  and `... session-start.py` (assert valid JSON, no discovery calls).

## Rollback points

Each slice is one reviewable commit set; revert by exact SHA. Migration
renames rather than deletes legacy state, so Slice 0/1 rollback loses no
routing data.
