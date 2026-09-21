# Research — Current Routing System Audit (baseline 1489ebf)

Evidence gathered 2026-09-21 by direct code inspection of branch `rust`
(baseline HEAD `1489ebf41975bf0ab836b2592060aa9d16905b36`, Trellis 0.6.14).

## Routing state

- Live legacy state exists at `.trellis/.runtime/routing/codex_*.json`
  (9 conversation files observed; `.trellis/.gitignore` line 8 ignores
  `.runtime/`). State version 2 with `surface` / `executor` / `reviewer`
  slots; v1 files are migrated in-memory by `_migrate_v1`.

## Import graph (who depends on routing)

- `.codex/hooks/inject-workflow-state.py` imports `detect_surface`,
  `load_state`, `resolve_codex_provider`, `routing_missing_slots`,
  `target_summary` from `common.codex_routing` (per-turn).
- `.trellis/scripts/common/git_context.py:32` imports `detect_surface`,
  `load_state`, `resolve_codex_provider` (used by `get_context.py
  --mode phase --platform codex`).
- `.trellis/scripts/common/task_store.py:27,154-162` uses
  `get_codex_dispatch_mode(...) == "auto"` to decide jsonl manifest seeding.
- `.trellis/scripts/common/config.py:255-325` implements
  `get_codex_dispatch_mode` / `get_codex_host_routes`
  (defaults `auto`, `cli→herdr`, `desktop→ask`, `unknown→ask`).
- `.trellis/scripts/common/workflow_phase.py:144-222`
  (`resolve_effective_platform`) maps codex to `codex-sub-agent /
  codex-inline / codex-herdr / codex-dsh-web / codex-auto` using
  `dispatch_mode`, `host_routes`, and surface.
- `.trellis/scripts/codex_routing.py` is the CLI front-end
  (`show/discover/validate/prompt/set-*/clear-*/invalidate`).
- Tests: `.trellis/tests/test_codex_routing.py` (318 lines),
  `.trellis/tests/test_codex_hook.py` (93 lines).

## workflow.md provider-specific surface

- Breadcrumb tags: `planning`, `planning-inline`, `planning-auto`,
  `planning-dsh-web`, `planning-herdr`, `in_progress`, `in_progress-inline`,
  `in_progress-auto`, `in_progress-dsh-web`, `in_progress-herdr`
  (plus `no_task`, `completed`).
- Platform-marker sections: `[codex-sub-agent]`, `[codex-inline, ...]`,
  `[codex-dsh-web, codex-auto]`, `[codex-herdr]` appear in steps 1.2, 1.3,
  2.1, 2.2 and in "Active Task Routing".
- The WORKFLOW-STATE BREADCRUMB CONTRACT comment (lines ~99-150) documents
  the variant tags and must be updated with them.

## Hooks

- `session-start.py` (551 lines): no subprocess discovery of Herdr/DSH Web;
  it emits the workflow Phase Index and task status. Only coupling is via
  workflow.md text and `resolve_context_key`/`resolve_active_task`.
- `inject-workflow-state.py`: per-turn surface detection
  (`CODEX_SURFACE`, `CODEX_HOST_SURFACE`, `CODEX_APP_TOOLS_PIPE_PATH`,
  `CODEX_CLI_SURFACE`, `CODEX_CLI`), `_codex_mode_banner`,
  `_codex_routing_banner`, provider-aware `resolve_breadcrumb_key`.
- `inject-subagent-context.py`: mentions "codex" only for the sub-agent
  platform name; no routing-state dependency (confirmed by grep).

## Discovery mechanisms to demote

- `discover_herdr()` runs `herdr agent list` (JSON), identifies the current
  pane via `HERDR_PANE_ID` or focused+codex.
- `discover_dsh_web()` scans `ps -axo pid=,command=` for `dsh web` and parses
  `--port` / `--trusted-host`.

## Spec

- `.trellis/spec/backend/quality-guidelines.md` (~lines 170-230) contains
  the "Host-aware Codex routing and explicit self-selection" contract that
  documents surface detection markers, `resolve_codex_provider`,
  `host_routes` mapping, and the routing state shape. This section is the
  spec-side target for replacement.

## Config

- `.trellis/config.yaml` lines 110-132: `codex.dispatch_mode: auto` +
  `host_routes` block with extensive policy comments. All other config
  sections (session, channel worker guard, context injection,
  prompt_injection) are unrelated and stay.

## Notes for implementers

- `resolve_context_key(platform="codex")` in `common/active_task.py` is the
  conversation-identity source to reuse for the new automation context path.
- `task.py start/finish/archive` are the only writers of
  `task.json.status`; automation state must stay separate
  (`.trellis/.runtime/automation/`, gitignored via the existing `.runtime/`
  rule).
- Test runner used by this repo: `python3 -m unittest` style
  (`test_codex_routing.py` uses `unittest`); confirm discover command in
  Slice 0 before writing new tests.
