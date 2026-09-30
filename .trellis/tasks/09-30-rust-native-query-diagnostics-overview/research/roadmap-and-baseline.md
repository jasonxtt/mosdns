# Bundling rationale and live baseline

User approved planning, C2C new project conversation and stop/handoff.
One coupled output is owned by this task: terminal DNS facts -> searchable
read projection -> existing query/detail/ranking workflows. These six pieces
share provenance, answer projection and read ownership; shipping just the
backend or just a UI panel does not meet the operator outcome. Keep them in
one PRD with behavior slices, not six child tasks and six review cycles.

## Verified completed predecessors

- Local-rule API/file/generation/DNS/Vue closure:
  `.trellis/tasks/archive/2026-09/09-28-rust-native-domain-set-management/`.
- Audit control/settings/DNS card/System panel:
  `.trellis/tasks/archive/2026-09/09-30-rust-native-next-bundled-delivery/`;
  completion in implement.md, final product d7fc11ec and subsequent journal/
  archive commits through research baseline 9f6dfdb2.
- Native query/compiler/cache/fast_mark/flow_setter already has bounded evidence.
  Native forward is still one numeric UDP/TCP endpoint per configured instance.
- docs/rust/next-stage-plan.md and the old parent describe completed work as
  pending. They have pre-existing dirty changes; this task does not overwrite
  their owner's content. This note records current state for the executor.

## Ordered next bundles (recommendation, no new tasks/authorization)

1. This query diagnostics/query-and-ranking workflow, using the existing store.
2. Shared provider/matcher/query-response family from actual config: provider
   IP sets and client/response IP CIDR/IPv6, basic qclass/response predicates,
   then hosts/TTL/drop-response together with real query-chain tests as justified.
   Freeze exact parameters when planning that batch; it is not this task.
3. Forward transport/selection family: plug existing secure/bootstrap/reuse
   foundations into native YAML forward, then strategy/fallback/cancel and
   corresponding factual upstream diagnostics as one runnable chain. Avoid
   bundling every protocol/listener if unrelated to that selected chain.
4. Switch-family management and routing/settings workflow, preserving bits
   48/49, then special_groups/online rules/config generation as shared control
   mechanisms. Final feature table completeness remains the phase criterion.
5. Full 5D with complete config, profiling-backed optimization and retirement.

Each batch has one concise planning review and one whole-delivery final review;
related tests during work, full VM regression at the end. Material changes or
real high risk can add review. No per-edit digest or full formal benchmark
matrix for ordinary feature implementation. Keep final performance/production
gates and Linux-only project validation convention unchanged.

## Dirty baseline preservation

Before task creation get_context reported 19 unrelated changes. Status after
creation includes modified quality-guidelines/workflow/Trellis automation test,
old parent PRD/design/implement/task metadata, archived canary-review artifacts,
canary directory moves and next-stage-plan. Treat ALL paths outside this new
task as pre-existing/other-owner work; don't stage or revert them. No task
parent link is created because that would mutate already-dirty parent metadata.
No product files were changed in this planning turn.
