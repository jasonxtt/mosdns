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

---

## Addendum — Planning review remediation (2026-09-21)

Root reviewer returned FINAL: FAIL on planning commit e632a41 (P0:0, P1:7,
P2:1). All findings were planning-artifact gaps; remediation stayed inside
the task directory. How each was addressed:

- **P1-1 (planning gate not wired)**: fixed activation order added —
  artifacts → planning PASS → reviewer resolved AND transport-verified →
  unit authorization snapshot → `task.py start` once → confirm
  `in_progress` → create run (prd §8.1, design §2.3 entry gate, implement
  Slice 2). A `planning` task can never enter implementation via a run.
- **P1-2 (no reviewer transport)**: narrow ChatGPT reviewer transport
  contract added (prd §5.5, design §2.4a, implement Slice 3): platform-native
  send/read/wait, no unofficial APIs / no browser automation, transport
  verified before `task.py start`, failure ⇒ ask user, never silent
  self-review; fake-adapter unit tests.
- **P1-3 (adapters promised but not planned)**: Herdr/DSH Web discovery and
  dispatch helpers are MOVED into adapter modules
  (`automation_herdr.py` / `automation_dsh_web.py`), not deleted; explicit
  overrides stay operational (design §2.2, implement Slice 1/4).
  Speculative `codex-thread` dropped.
- **P1-4 (run state not resumable)**: durable per-unit review state added —
  `phase`, `submission{parent_sha, head_sha, review_round, request_kind,
  submitted_to}`, run-level `reviewer_bootstrap_sent` (design §2.1(B),
  implement Slice 2).
- **P1-5 (corrupt run treated as absent)**: split corruption policy —
  conversation context → safe defaults; active run → fail-CLOSED `BLOCKED`
  (prd §8.2, design §4).
- **P1-6 (5-round off-by-one)**: counter renamed `failed_remediation_rounds`;
  initial discovery = 0; +1 only after executed + resubmitted remediation
  still fails; boundary test pins BLOCKED at round 5 (prd §6.3, implement
  Slice 3).
- **P1-7 (migration provenance + rollback)**: auto-migrate only
  `selected_by="user"`; ambiguous provenance (incl. legacy v1-migrated
  reviewers stamped `migration`) → confirm-or-null, never silent; legacy
  file left in place byte-for-byte with `migrated_from: {path, sha256}`
  fingerprint in the new context (prd §10, design §2.5, implement Slice 0).
- **P2 (jsonl `_example` seed lines)**: removed from both manifests.

## Addendum 2 — Remediation round 2 (2026-09-21)

Re-review of 7cc275d: P0:0, P1:1, P2:0. The single remaining finding:

- **P1 (ChatGPT transport assumed, not proven)**: there is no verified
  evidence the Codex host exposes send/read/wait into a plain ChatGPT
  conversation; a fake adapter proves interface logic only, not the
  end-to-end loop. Fixed by adding a pre-implementation feasibility exit
  gate — implement.md "Slice G" (real host-level probe of send + read +
  stable identity against a real user-selected conversation, no UI
  automation / no unofficial API, evidence recorded in
  `research/chatgpt-transport-probe.md`, sequenced strictly before Slice 0
  and before any de-routing mutation) — plus PRD §5.6 and design §2.4a /
  §5. On probe failure the task stops pre-mutation and the reviewer
  transport choice (browser-use driver / Codex detached reviewer / other
  transport / manual relay) is escalated to the user; Trellis never picks
  silently. Slice 0 and Slice 1 both carry the Slice G precondition.
