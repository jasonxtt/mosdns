# Rust migration next-step roadmap

## Goal

Produce a repository-grounded plan for the next Rust migration work, have the
same new C2C ChatGPT conversation review and approve the persisted plan, then
execute each non-deferred, user-authorized work package in order and return each
committed result to that same conversation until every executed package passes
review. Explicitly deferred packages remain recorded as deferred and are not
executed merely to close this parent task.

## Background

- The current checkout is the dedicated `rust` branch. The pure Rust-native
  host remains the migration target; production/default cutover is still gated
  by the documented Rust-native E2E and hybrid-retirement gates.
- Current migration evidence, task history, and constraints are recorded in
  `docs/ai/rust-handover.md`, `docs/ai/rust-rewrite-plan.md`, and
  `docs/rust/`.
- The separately scoped canary task used fixed candidate
  `016103f3c21ed2d659694ce10e64aaf24b5c2767`. Its execution inputs were frozen
  and the corrected `mosdns-rust` run passed; the original task is retained as
  terminal `superseded`, and the replacement review task is archived after an
  exact committed-range C2C `FINAL: PASS`. Do not rerun or retarget that
  canary from this parent task.
- The same C2C conversation returned a `PLAN_STATUS:
  READY_WITH_EXPLICIT_EXECUTION_GATE` plan for the verified `mosdns-rust`
  workspace at `rust` HEAD `9fd0bc0c`. The plan recommends the existing canary,
  followed by bounded 5B `fast_mark`/`flow_setter` integration and a first 5C
  `domain_set` management closure. Its precise evidence, dependencies,
  boundaries, and deferred work are recorded in `design.md` and
  `implement.md`.
- Update `docs/rust/next-stage-plan.md` in place so it records this ordered
  work, keeps the existing final 5D/Phase 6/cutover gates intact, and does not
  overstate coverage.

## Requirements

- Start a new ChatGPT conversation in the existing mosdns-rust C2C Project and
  verify it is bound to this exact workspace before asking for recommendations.
- Ask ChatGPT to inspect the connected workspace and provide detailed next-step
  recommendations grounded in current code, evidence, archived tasks, active
  task artifacts, and migration constraints. The response must include
  rationale, ordered actions, dependencies, scope boundaries, concrete files
  or artifacts, validation, acceptance criteria, and risks or deferred work.
- Codex owns all repository edits and execution. ChatGPT is the planning and
  independent review partner. Do not paste source files, diffs, or logs into
  ChatGPT; use the connected workspace MCP and C2C execution records.
- Turn the reviewed recommendations into a durable plan document and
  independently verifiable Trellis tasks, preserving existing gates and
  documenting ordering and review boundaries.
- Have the same C2C conversation review the completed plan. Begin approved
  tasks only after that plan review passes. Execute each non-deferred package
  in the agreed order, record each result, and return each executed package to
  the same conversation for review. Keep any explicitly deferred package
  unexecuted with its planning and review history recorded.
- The canary execution inputs were resolved and must remain frozen as evidence:
  use a read-only config snapshot (never live `/cus/mosdns` state), keep any Go
  comparator best-effort/non-gating without a Go build, use controlled peers as
  the hard DNS oracle, and use identity-checked TERM plus verified complete
  resource release without claiming graceful shutdown. The run used only the
  `mosdns-rust` SSH alias, passed its bounded UDP/audit-on and TCP/audit-off
  checks, and did not change service or production state. No canary rerun is
  authorized or required by this roadmap.
- Resolve review findings locally and repeat the same-conversation review loop
  until every executed package receives an explicit pass. Preserve original
  failure evidence; explicitly deferred packages are reported with their
  existing status and do not need an execution review. Do not claim completion
  while an authorized, non-deferred package or its review remains open.
- Preserve all MosDNS product contracts and Rust migration guardrails in
  `AGENTS.md` and `docs/ai/`; do not relax production cutover gates or extend
  transitional Go/cgo bridge patterns into new Rust phases.

## Out of Scope

- Production/default cutover before the documented Rust-native E2E and
  hybrid-scaffolding retirement gates pass.
- Changes to product code before the C2C plan is approved and the relevant
  child task is explicitly activated.
- Replacing, deleting, or silently merging unrelated dirty work or the
  historical canary evidence/task; its terminal supersession and replacement
  review record must remain intact.

## Acceptance Criteria

- [x] A new ChatGPT Chat in the existing mosdns-rust Project confirms the exact
      `Codex with ChatGPT · mosdns-rust` workspace through `workspace_info`.
- [x] The same conversation returns detailed, source-grounded next-step
      recommendations with rationale, ordered work, dependencies, concrete
      files/artifacts, validation, acceptance criteria, and deferred items.
- [x] The roadmap document and Trellis task map match those recommendations,
      preserve project guardrails, avoid duplicate active work, and receive an
      explicit C2C planning-review pass before execution begins.
- [x] The four canary execution inputs were resolved, the bounded run completed
      on `mosdns-rust`, and the replacement exact-range review returned
      `FINAL: PASS`; no further canary action is pending.
- [ ] Every non-deferred, user-authorized task is executed by Codex in the
      agreed order; execution records are available to the reviewer without
      pasting logs or diffs into ChatGPT. Any deferred package remains
      unexecuted and is explicitly recorded with its existing status.
- [ ] The same C2C conversation explicitly passes every executed task review,
      including corrective iterations; the final report lists the reviewed
      artifacts and any explicitly deferred work.

## Current State

The new C2C chat was created inside the saved `mosdns-rust` Project and bound
through the exact `Codex with ChatGPT · mosdns-rust` connector. Its workspace
check confirmed branch `rust` and HEAD `9fd0bc0c`; the same conversation returned
the detailed plan and passed the cumulative planning range
`9fd0bc0c061fb440c88781949f5088652aca70b9..3887af32d6ef17624f8b1d198e0dcb0bed4a28d7`
on review iteration 3. The canary then used the frozen read-only snapshot,
controlled peers, non-gating no-Go-comparator rule, and identity-checked
cleanup on `mosdns-rust`; its corrected attempt passed all bounded UDP/TCP
cases. The original canary task remains terminal `superseded`, while the
replacement review task
`09-28-rust-mosdns-rust-canary-review-restart` is archived as completed after
the same C2C conversation returned `FINAL: PASS` for the exact correction
range `242cbcbbc2d02c9ae77a81291a07c5c143ee6b57..79ddded8edea9f53b07d051ce20b3daf6b56e868`.
No product code or production state changed. The next unstarted deliverable is
the 5B `fast_mark`/`flow_setter` child; the 5C child remains ordered after the
5B child receives its own same-chat C2C `FINAL: PASS`. If 5B is explicitly
deferred or blocked, reordering 5C still requires the user's explicit approval.
