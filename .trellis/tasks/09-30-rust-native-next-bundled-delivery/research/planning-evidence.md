# Planning evidence and review state

## Current state checked on 2026-09-30

- Branch `rust`, initial planning HEAD `96fcd0b9320e677acf42d3688b564a1c2a6af2b9`.
- Prior bounded `domain_set` local-rule workflow archived as completed at
  `.trellis/tasks/archive/2026-09/09-28-rust-native-domain-set-management/`;
  the user supplied the exact-range `FINAL: PASS` for `390a6d97..3e1e2183`.
  This does not complete 5C or authorize deployment.
- Parent `09-28-rust-next-step-roadmap` remains planning. Its existing dirty
  text and `docs/rust/next-stage-plan.md` refer to the prior 5C child as
  planning; archive state and current source take precedence. Their owner
  should reconcile those status paragraphs in a separate inspected scope.
- The connected `Codex with ChatGPT · mosdns-rust` workspace reported branch
  `rust`, commit `96fcd0b9` and a dirty worktree. Existing dirty files include
  canary archive movement, parent roadmap documents, workflow/spec/test edits
  and `docs/rust/next-stage-plan.md`. This task's eight new planning artifacts
  and the precise child-link change in parent `task.json` form one planning
  range; all other dirty paths stay out.

## C2C discussion in `核查任务问题`

Conversation ID: `6abbc3e5-dc34-83e8-9d07-eb45af7783da`.

1. First recommendation: prefer native audit control and DNS dashboard over
   cache management/dump as the next larger PRD. The existing observer,
   host-owned HTTP lifecycle and Vue consumer form a shorter path to a real
   management workflow. Keep `special_groups`/upstream config for later due to
   immutable native graph and much larger config-generation requirements.
2. Follow-up challenge: distinguish lifetime host metrics from the retained
   audit ring and its v2 stats; use one terminal-time capture decision rather
   than copying Go's async race. Preserve canonical `webinfo/audit_settings.json`
   and legacy read precedence. Propose capacity 0, explicit range error and
   persist-before-publish failure atomicity as product decisions. Include
   `/stats/windows` to cover the DNS card's popover. Keep full OverviewManager
   ranks, QueryManager filters and unrelated System endpoints deferred.
3. Codex cross-checked source anchors in `research/observability-option.md`
   and prepared `prd.md`, `design.md`, and `implement.md`. Strict C2C planning
   review found three P1, seven P2 and two P3 issues. The revised drafts
   address scope language, parent-link staging, window-clock proof, direct
   log limit/body deviations, projection fields, bounded read progress,
   VM/browser topology, failure layers, v1 logs exclusion and per-response
   snapshot semantics. Rereview closed all twelve original findings, then
   found two new P1, one P2 and two P3 gaps: C10 coverage scope, settings-file
   load semantics, explicit user-decision gate, planning dirty-tree gate and
   empty-window coverage omission. Those have been patched in the latest
   drafts. Final C2C planning check marked all five closed and found no new
   actionable P1/P2/P3 findings. This is a planning PASS only; product-choice
   confirmation and separate implementation authorization remain pending.

The user consented to create a new planning task. A direction-choice question
was sent; this draft follows the recommended audit package while that answer
is pending. Task creation and discussion do not authorize `task.py start`.
