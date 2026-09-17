# Herdr executor and reviewer routing mode

## Goal

Add a Codex-specific Herdr routing mode that prevents execution ownership and
ChatGPT review destinations from drifting between turns. For each new Codex
conversation, the user makes one combined choice of execution mode/executor
pane and reviewer conversation. That choice remains in force for the lifetime
of the Codex conversation unless the user changes it or the selected resource
becomes invalid.

## Background and confirmed facts

- `.trellis/config.yaml` currently sets `codex.dispatch_mode: inline`.
- `.codex/hooks/inject-workflow-state.py` injects the selected Codex mode on
  every prompt. Its current `inline` wording requires the main session to
  implement directly and caused a later slice to bypass an already selected
  Herdr executor.
- Trellis already resolves a stable Codex conversation identity from
  `CODEX_THREAD_ID` and stores session-scoped runtime state under
  `.trellis/.runtime/sessions/`.
- `herdr agent list` exposes pane ID, workspace, detected agent, state, cwd and
  title; `herdr agent explain <pane>` supplies authoritative detection
  evidence.
- Herdr panes are not Codex native Trellis sub-agents. `auto`, `inline`, and
  the new Herdr mode must remain distinct execution topologies.
- The ChatGPT `mosdns` project exposes its existing conversations and a
  project-scoped new-chat composer. The current project guideline incorrectly
  hard-codes the historical `rust0916` conversation and a right-side Claude
  pane as globally required.

## Requirements

### R1 — Explicit Herdr dispatch mode

- Add a supported Codex dispatch mode named `herdr`.
- `auto` continues to mean native Codex Trellis sub-agent dispatch.
- `inline` continues to mean implementation/checking in the main Codex
  session.
- `herdr` means the main Codex session is controller and a user-selected Herdr
  pane is executor. It must not silently launch a native implement/check
  sub-agent or implement directly.
- Invalid configured values continue to fail safely instead of enabling an
  arbitrary executor.

### R2 — Candidate discovery without position or agent restrictions

- At the first execution-relevant turn in a new Codex conversation, inspect
  the live Herdr inventory and identify the current Codex pane.
- Every other live pane in the same Herdr workspace is an executor candidate,
  regardless of its position, detected agent label, title, or current cwd.
- Present enough evidence to distinguish candidates: pane ID, detected agent,
  state, cwd and title. Do not assume that the candidate is Claude or to the
  right of Codex.
- Selection is always made by the user; automatic discovery must not become
  automatic selection.

### R3 — Single combined user decision

- When no valid conversation-scoped selection exists, ask one combined
  question covering all missing decisions: execution mode/executor pane and
  ChatGPT reviewer destination.
- If the current Codex pane is the only pane in its Herdr workspace, explain
  that Herdr dispatch cannot start and offer `inline` or wait for another
  Herdr pane.
- If one or more candidate panes exist, list all of them and ask which one to
  use. Do not auto-select even when there is exactly one candidate.
- Reviewer choices must support an explicitly supplied conversation, an
  existing conversation selected from a ChatGPT project, a new conversation
  inside a selected ChatGPT project, or a new non-project conversation.
- Ask a follow-up only when the single answer is genuinely ambiguous or the
  environment changes before the choice can be applied.

### R4 — Conversation-scoped persistence and override

- Persist the resolved mode/executor and reviewer selection under the current
  Codex conversation identity, not as a repository-wide default and not as a
  per-task selection.
- Reuse the selection across tasks, slices and review rounds in the same Codex
  conversation without asking again.
- A new Codex conversation must begin unselected and must obtain its own user
  choice before execution/review routing.
- An explicit user instruction may replace either or both selections. Store
  the replacement and use it for later turns in that conversation.
- Planning and read-only investigation may proceed without a routing choice;
  implementation dispatch and external review submission may not.

### R5 — Selection validity and fail-closed behavior

- Before each dispatch, verify that the selected executor pane still exists in
  the expected Herdr workspace. Agent label, pane position, title, state and
  cwd may change and are refreshed evidence, not selection identity.
- If the pane disappeared, moved to another workspace, or cannot be resolved,
  invalidate only the executor selection and ask one combined replacement
  question for any now-missing decisions.
- If the pane cwd differs from the repository, report it in the dispatch
  context and instruct the executor to enter the exact repository before
  acting; cwd mismatch alone does not authorize selecting another pane.
- Before each review request, verify that the stored reviewer conversation is
  addressable. If unavailable or ambiguous, invalidate only the reviewer
  selection and ask once for a replacement.
- Never fall back from `herdr` to `inline`, a native sub-agent, or another pane
  without the user's explicit choice.

### R6 — Reviewer identity and creation

- Store reviewer provider, project title/ID when applicable, conversation
  title/ID and canonical URL after resolving or creating the conversation.
- When creating a project conversation, use the selected project's new-chat
  entry so the conversation inherits project context.
- Do not create a conversation or send a review request until the user has
  selected that destination.
- Existing task artifacts that record historical review by `rust0916` remain
  immutable evidence. Project-wide guidance must stop treating `rust0916` as
  the permanent reviewer for future conversations.

### R7 — Executor and review contracts

- The controller retains scope definition, safety decisions, exact diff
  inspection, verification, commit/push checking and the review loop.
- The selected executor receives the active task, exact authorized slice,
  allowed paths, required checks, prohibited actions and a recognizable
  handoff/reporting contract. The contract must not depend on the executor
  being Claude or on a fixed phrase such as `我是Claude`; reports identify the
  selected pane and executor evidence instead.
- A reviewer PASS/FAIL remains a gate. PASS does not automatically authorize a
  later slice or new task.

### R8 — Compatibility and scope

- Preserve existing `auto`, legacy `sub-agent`, and `inline` behavior.
- Preserve session-scoped active-task resolution and unrelated runtime files.
- Keep Trellis auto-commit disabled and preserve unrelated dirty files.
- Make the project-local change in `.trellis/`, `.codex/` and the applicable
  project spec; do not modify global npm packages.

## Acceptance Criteria

- [ ] `codex.dispatch_mode: herdr` is parsed consistently by config helpers,
      workflow filtering and per-turn hook injection.
- [ ] A new Codex conversation with only its own Herdr pane receives one
      combined prompt offering inline or wait plus all reviewer choices.
- [ ] A new Codex conversation with one candidate receives one combined prompt
      and does not auto-select it.
- [ ] A new Codex conversation with multiple candidates lists each candidate's
      pane ID, agent, state, cwd and title in the same combined prompt.
- [ ] A non-Claude pane and a pane in any position can be selected and used.
- [ ] Executor and reviewer choices are reused across later turns, tasks and
      slices in the same Codex conversation without another question.
- [ ] A different Codex conversation does not inherit those choices.
- [ ] Explicit user replacement updates later routing in the same conversation.
- [ ] Executor disappearance or reviewer unavailability invalidates only the
      affected choice and never triggers a silent fallback.
- [ ] An existing ChatGPT project conversation can be selected by reference,
      title or URL and is stored canonically.
- [ ] A new reviewer conversation can be created inside the `mosdns` project,
      and a non-project new conversation remains an available choice.
- [ ] Historical task evidence naming `rust0916` remains intact while the
      project-wide fixed-reviewer rule is removed.
- [ ] Tests cover mode parsing, workflow-key selection, session isolation,
      one-question state transitions, invalidation and safe fallback behavior.
- [ ] Planning validation and targeted Python tests pass; the implementation
      does not modify MosDNS product/runtime code.

## Out of Scope

- Changing Herdr itself or its agent-detection manifests.
- Automatically choosing an executor based on agent brand, pane position,
  title, cwd or recency.
- Building a general ChatGPT API client or depending on undocumented private
  web endpoints.
- Migrating or rewriting historical task evidence.
- Automatically authorizing later slices after reviewer PASS.
