# Implementation plan — Herdr executor and reviewer routing mode

## Gate

- [ ] User reviews and explicitly approves `prd.md`, `design.md` and this plan.
- [ ] Run `task.py start` only after that approval.
- [ ] Load `trellis-before-dev` before editing implementation files.

## Slice 1 — mode parsing and workflow namespace

- [ ] Add RED tests for `herdr` config normalization and the
      `codex-herdr` effective platform/workflow key.
- [ ] Extend `.trellis/scripts/common/config.py`,
      `.trellis/scripts/common/workflow_phase.py` and
      `.codex/hooks/inject-workflow-state.py` consistently.
- [ ] Add `codex-herdr` planning/execution workflow blocks without changing
      `auto`, legacy `sub-agent` or `inline` behavior.
- [ ] Update `.trellis/config.yaml` documentation and select `herdr` only after
      the new mode is fully recognized.

## Slice 2 — conversation-scoped routing state

- [ ] Add a project-local routing-state helper with versioned schema, context
      key resolution, atomic write, validation and corruption-safe reads.
- [ ] Keep routing files separate from active-task session JSON.
- [ ] Add tests proving same-conversation reuse, different-conversation
      isolation, explicit replacement and partial invalidation.
- [ ] Ensure runtime files remain untracked and do not change historical task
      artifacts automatically.

## Slice 3 — Herdr discovery and selection contract

- [ ] Add a read-only discovery helper or documented structured command path
      around `herdr agent list/explain`.
- [ ] Cover zero, one and multiple non-current candidates.
- [ ] Preserve candidates regardless of agent label, position, title or cwd.
- [ ] Generate one combined decision prompt covering all unresolved executor
      and reviewer choices.
- [ ] Add tests/fixtures for candidate summaries, inline-or-wait behavior and
      no automatic candidate selection.

## Slice 4 — reviewer resolution and project guidance

- [ ] Implement/instruct canonical reviewer selection by conversation
      reference, URL or unambiguous project title.
- [ ] Document supported creation paths: existing project, new project chat and
      new non-project chat. Do not introduce undocumented ChatGPT endpoints.
- [ ] Update `.trellis/spec/backend/quality-guidelines.md` so future routing is
      destination-neutral and executor-neutral.
- [ ] Preserve historical task-local mentions of `rust0916`.
- [ ] Replace fixed `我是Claude` identity with a selected-pane handoff identity
      while retaining complete changed-path/validation/commit reporting.

## Slice 5 — integration and fail-closed verification

- [ ] Exercise the per-turn hook with synthetic Codex thread IDs and routing
      states for missing, selected, corrupt and invalidated cases.
- [ ] Verify a selected executor is rechecked before dispatch and cwd drift is
      corrected explicitly rather than causing silent reselection.
- [ ] Verify reviewer failure invalidates only reviewer state and executor
      failure invalidates only executor state.
- [ ] Verify planning/read-only work does not force early selection while
      dispatch/review submission does.
- [ ] Run task validation and exact diff checks.

## Validation

Use the repository's existing Python test mechanism if present; otherwise add
focused `unittest` coverage next to the local Trellis integration tests and run
it directly. Required checks include:

```text
python3 -m py_compile .codex/hooks/inject-workflow-state.py
python3 ./.trellis/scripts/get_context.py --mode phase --platform codex
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-17-herdr-executor-reviewer-routing
git diff --check
```

Also run every focused test added for config parsing, workflow filtering,
routing-state isolation, discovery summaries and invalidation.

## Risk and rollback points

- Do not set `dispatch_mode: herdr` until every parser recognizes it; otherwise
  an intermediate commit can safely but incorrectly collapse to inline.
- Keep the active-task session schema unchanged so task selection cannot be
  corrupted by routing-state work.
- Do not edit historical review evidence to remove `rust0916` references.
- If ChatGPT UI automation is unavailable, leave reviewer state unresolved and
  ask the user; never guess or silently reuse a different conversation.
- Rollback is the exact scoped revert plus restoring `dispatch_mode: inline`.
