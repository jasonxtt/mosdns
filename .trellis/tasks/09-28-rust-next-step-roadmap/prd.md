# Rust migration next-step roadmap

## Goal

Produce a repository-grounded plan for the next Rust migration work, have the
same new C2C ChatGPT conversation review and approve the persisted plan, then
execute its bounded work packages in order and return each committed result to
that same conversation until every planned review passes.

## Background

- The current checkout is the dedicated `rust` branch. The pure Rust-native
  host remains the migration target; production/default cutover is still gated
  by the documented Rust-native E2E and hybrid-retirement gates.
- Current migration evidence, task history, and constraints are recorded in
  `docs/ai/rust-handover.md`, `docs/ai/rust-rewrite-plan.md`, and
  `docs/rust/`.
- One separate Trellis task, `09-28-rust-mos-test-native-sidecar-canary`, is
  already in planning and has its own fixed candidate and review history. Keep
  it unchanged as the first, separately gated work package; do not duplicate,
  retarget, or execute it before its pending user decisions are resolved.
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
  tasks only after that plan review passes. Execute tasks in the planned order,
  record each result, and return it to the same conversation for review.
- The plan review does not resolve the four execution inputs recorded by the
  existing canary task: use a read-only config snapshot (never live
  `/cus/mosdns` state), make any Go comparator best-effort/non-gating without a
  Go build, use controlled peers as the hard DNS oracle, and use identity-checked
  TERM plus verified complete resource release without claiming graceful
  shutdown. Obtain the user's explicit choice to execute or defer the canary;
  if deferred, ask whether 5B may proceed without its remote result. Do not
  connect to `mos-test`, build, or launch anything before that gate is resolved.
- Resolve review findings locally and repeat the same-conversation review loop
  until every planned task receives an explicit pass. Preserve original
  failure evidence and do not claim completion while any planned task or review
  remains open.
- Preserve all MosDNS product contracts and Rust migration guardrails in
  `AGENTS.md` and `docs/ai/`; do not relax production cutover gates or extend
  transitional Go/cgo bridge patterns into new Rust phases.

## Out of Scope

- Production/default cutover before the documented Rust-native E2E and
  hybrid-scaffolding retirement gates pass.
- Changes to product code before the C2C plan is approved and the planned task
  artifacts are ready for execution.
- Replacing, deleting, or silently merging unrelated dirty work or the existing
  active mos-test canary task.

## Acceptance Criteria

- [x] A new ChatGPT Chat in the existing mosdns-rust Project confirms the exact
      `Codex with ChatGPT · mosdns-rust` workspace through `workspace_info`.
- [x] The same conversation returns detailed, source-grounded next-step
      recommendations with rationale, ordered work, dependencies, concrete
      files/artifacts, validation, acceptance criteria, and deferred items.
- [ ] The roadmap document and Trellis task map match those recommendations,
      preserve project guardrails, avoid duplicate active work, and receive an
      explicit C2C planning-review pass before execution begins.
- [ ] The four canary execution inputs and the execute/defer decision are
      resolved with the user after the plan-review pass; no remote action occurs
      before then.
- [ ] Every planned task is executed by Codex in dependency order; execution
      records are available to the reviewer without pasting logs or diffs into
      ChatGPT.
- [ ] The same C2C conversation explicitly passes every task review, including
      any corrective iterations; the final report lists the reviewed artifacts
      and any work explicitly deferred by the approved plan.

## Current State

The new C2C chat was created inside the saved `mosdns-rust` Project and bound
through the exact `Codex with ChatGPT · mosdns-rust` connector. Its workspace
check confirmed branch `rust` and HEAD `9fd0bc0c`. The conversation returned
the detailed plan. The durable roadmap and Trellis artifacts are being prepared
for an exact committed-range review in that same chat; no downstream task has
started and no remote canary action has occurred.
