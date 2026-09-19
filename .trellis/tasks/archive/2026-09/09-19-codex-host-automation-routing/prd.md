# Separate Codex CLI and Desktop automation routing

## Goal

Make the repository's Codex automation policy distinguish the Codex CLI from
the Codex Desktop/App surface, while keeping executor and reviewer selection
conversation-scoped, user-controlled, and extensible.

The default policy is:

- Codex CLI -> a user-selected Herdr executor;
- Codex Desktop/App -> MCP DSH;
- reviewer -> a user-selected reviewer conversation or explicitly selected
  Codex self-review.

The policy must never hard-code a particular Herdr pane, DSH worker, ChatGPT
conversation, model, agent brand, pane position, or project title.

## Background and confirmed facts

- The current hook detects only the broad `codex` platform. It does not
  distinguish CLI from Desktop/App.
- `.trellis/config.yaml` currently sets `codex.dispatch_mode: herdr`, which
  makes the per-turn banner claim Herdr even in the Desktop/App environment.
- `.trellis/scripts/common/codex_routing.py` currently accepts only `herdr`
  and `inline` dispatch state, and its reviewer schema requires a ChatGPT URL.
- The current environment exposes an App-tools pipe marker, while
  `herdr agent list` has no visible agent inventory. This is evidence that
  host detection must be explicit and fail closed; terminal titles and the
  presence of the `herdr` executable are not sufficient.
- The existing quality guidelines already define separate safety contracts for
  Herdr and MCP DSH. This task connects those contracts through one routing
  policy; it does not change Herdr or DSH themselves.
- The current conversation-scoped state is the correct persistence boundary.
  Existing v1 state and historical task/reviewer evidence must remain readable.
- The user may explicitly name an executor or reviewer in a conversation,
  including the current Codex session itself. An explicit user choice takes
  precedence over host defaults and remains in force until replaced or
  invalidated.

## Requirements

### R1 — Evidence-based host surface detection

1. Represent the Codex surface as `cli`, `desktop`, or `unknown`, together
   with non-secret detection evidence.
2. Prefer a host-provided surface value or an App-only marker. Do not infer
   CLI solely from the absence of an App marker, a terminal title, a cwd, a
   pane position, or the installed `codex` executable.
3. Support an explicit conversation-scoped surface override for environments
   where the host cannot expose a stable marker.
4. An unresolved surface must produce an actionable selection prompt and must
   not silently choose Herdr, DSH, a native sub-agent, or inline execution.

### R2 — Host policy with backward-compatible explicit modes

1. Add an `auto` policy that maps `cli` to Herdr and `desktop` to MCP DSH.
2. Keep explicit `inline` and `herdr` behavior compatible for users who
   intentionally configure those modes; add an explicit DSH mode where the
   adapter is available.
3. Move the repository default from the current unconditional Herdr setting
   to the host-aware policy.
4. Keep the host-to-provider mapping configurable/documented rather than
   embedding a particular worker identity in Python code.

### R3 — Generic conversation-scoped executor identity

1. Replace the closed `dispatch` shape with a versioned executor target that
   records a provider/kind, an opaque user-selected reference, display
   metadata, selection source, and the surface/policy evidence that led to it.
2. Provide built-in adapters for the current Codex session, Herdr pane
   selection, and MCP DSH, but validate targets through provider adapters so
   new providers can be added without changing the state model.
3. Allow an explicit user instruction to select Codex itself as executor. This
   means the controller performs the implementation under the explicit
   self-execution contract; it is not an implicit fallback from a missing
   external executor.
4. Never auto-select a Herdr pane or DSH session. Host policy may choose the
   provider class, but a provider requiring a resource must still obtain the
   user's resource selection or a valid provider-managed target.

### R4 — Generic conversation-scoped reviewer identity

1. Store reviewer identity using the same provider/reference model rather than
   requiring a ChatGPT URL or a fixed title.
2. Support explicit current-Codex self-review, an existing ChatGPT web
   conversation, a user-selected project conversation, a newly created user-
   selected project/non-project conversation, and future provider adapters.
3. Do not create, message, or switch reviewer destinations without an
   explicit user choice.
4. Validate and invalidate only the reviewer target when it becomes
   unavailable; preserve a valid executor selection.

### R5 — Selection precedence and fail-closed transitions

1. Resolve routing in this order: explicit user override, valid persisted
   conversation selection, explicit repository policy, detected host default,
   then an actionable unresolved state.
2. Persist explicit user overrides under the current Codex conversation key and
   reuse them across tasks, slices, and review rounds.
3. When either target is missing or invalid, ask one combined question for all
   missing decisions. Planning and read-only investigation may continue.
4. A missing Herdr pane, unavailable DSH adapter, or unreachable reviewer must
   never silently fall back to another executor/reviewer.

### R6 — CLI, hook, workflow, and documentation integration

1. Extend `codex_routing.py` with inspect/discover/set/clear operations for
   surface, executor, and reviewer targets using provider/reference values.
2. Make hook banners and workflow filtering describe the resolved surface and
   target contract dynamically instead of assuming Herdr.
3. Document the conversation directive syntax and the exact safety boundary
   for `executor=codex` and `reviewer=codex`.
4. Update the project quality guidance to describe host-aware defaults and
   user-selected overrides without naming a permanent pane or reviewer.

### R7 — Compatibility and scope discipline

1. Migrate readable v1 routing state (`inline`, `herdr`, and ChatGPT reviewer)
   to the versioned target model without changing historical task evidence.
2. Preserve unrelated dirty files and Trellis auto-commit settings.
3. Restrict changes to `.trellis/`, `.codex/`, applicable project specs/tests,
   and task artifacts. Do not modify MosDNS runtime/product code.

## Acceptance Criteria

- [x] A test fixture with an App-only marker resolves to `desktop`; a fixture
      with an explicit CLI marker resolves to `cli`; missing/ambiguous evidence
      resolves to `unknown` with the evidence recorded.
- [x] `auto` selects the Herdr provider for CLI and the MCP DSH provider for
      Desktop, without selecting a concrete pane, worker, or reviewer.
- [x] Explicit repository modes `inline`, `herdr`, and DSH remain available
      and invalid values fail closed.
- [x] A user-selected executor target can represent the current Codex,
      arbitrary Herdr pane identity, or a provider-managed DSH target without
      changing the state schema for each new resource.
- [x] A user-selected reviewer target can represent current Codex, an existing
      ChatGPT conversation, a selected project conversation, or a future
      provider; no fixed reviewer title/URL is required.
- [x] Explicit conversation choices override host defaults, persist across
      turns/tasks, and can be replaced independently for executor or reviewer.
- [x] Invalidating one target leaves the other target intact and produces one
      combined actionable prompt; no silent fallback occurs.
- [x] v1 routing state loads into the new model and remains behaviorally
      compatible for explicit `inline`/`herdr` selections.
- [x] Hook/workflow output no longer claims Herdr for a Desktop/App session
      routed to DSH and clearly reports unknown-surface fail-closed state.
- [x] Targeted routing tests, hook/workflow tests, task validation, formatting,
      and diff checks pass; no MosDNS product/runtime files are changed.

## Out of scope

- Changes to Herdr's agent discovery, pane identity, or command protocol.
- Changes to the MCP DSH implementation or its external service.
- Automatic creation of a reviewer conversation without user selection.
- Natural-language parsing in a shell hook that can reinterpret arbitrary user
  text as authorization; the Codex controller remains responsible for turning
  explicit user instructions into validated routing state.
- Changing Rust migration, QUIC Slice 1, MosDNS configuration, deployment, or
  production/service state.

## Open questions

None blocking. The detector will use the strongest host-provided evidence
available and will require an explicit surface/target selection when evidence
is insufficient.
