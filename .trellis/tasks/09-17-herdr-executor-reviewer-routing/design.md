# Design — Herdr executor and reviewer routing mode

## Architecture

The feature extends the existing Codex project integration with a third
execution topology and a small conversation-scoped routing state.

1. `.trellis/config.yaml` selects `codex.dispatch_mode: herdr`.
2. Codex config/workflow helpers normalize the value as a first-class mode and
   map it to a `codex-herdr` virtual workflow platform.
3. The per-prompt Codex hook resolves the stable Codex context key already used
   by active-task state, reads the routing state for that key, and injects one
   of three facts: selection missing, selection valid, or replacement needed.
4. The main Codex session uses Herdr CLI discovery/control and ChatGPT browser
   capabilities. The Python hook does not scrape terminal UI or ChatGPT.
5. A project-local helper owns schema validation and atomic persistence so
   prompts and future commands do not hand-edit runtime JSON.

## Runtime state

Use a separate project-ignored runtime file keyed by the Trellis/Codex context
key, adjacent to but not embedded in the active-task session record:

```text
.trellis/.runtime/routing/codex_<thread-id>.json
```

Proposed schema:

```json
{
  "version": 1,
  "platform": "codex",
  "context_key": "codex_<thread-id>",
  "dispatch": {
    "mode": "herdr",
    "workspace_id": "w6",
    "executor_pane_id": "w6:p2",
    "selected_by": "user"
  },
  "reviewer": {
    "provider": "chatgpt",
    "project_title": "mosdns",
    "project_id": "g-p-...",
    "conversation_title": "...",
    "conversation_id": "...",
    "url": "https://chatgpt.com/...",
    "selected_by": "user"
  },
  "updated_at": "..."
}
```

`dispatch.mode` may also be `inline` when the user chooses inline after Herdr
preflight. Native `auto` remains a repository configuration mode and is not a
Herdr fallback.

Runtime state is ephemeral and conversation-scoped. Durable task evidence
records which reviewer accepted a specific commit, but it does not become the
source of future routing decisions.

## Selection state machine

### Entry

Routing selection is required only before an action that would dispatch
implementation/check work or submit external review. Planning, repository
inspection and local read-only checks do not trigger the chooser.

### Discovery

1. Run `herdr agent list`.
2. Resolve the current pane from the active Codex/Herdr evidence.
3. Filter other panes to the same Herdr workspace; retain every candidate.
4. Read `herdr agent explain` for the current pane and candidates when needed
   to present authoritative labels/evidence.
5. Resolve reviewer candidates through user-supplied references or the visible
   ChatGPT project UI. Do not use undocumented endpoints.

### One combined question

Build a single concise question containing only unresolved selections:

- no executor candidates: `inline` or wait;
- one/many candidates: list all and request an executor pane;
- reviewer unresolved: existing referenced conversation, listed project
  conversation, create in project, or create outside a project.

The answer is parsed as one decision packet. Follow-up occurs only for a real
ambiguity, such as two conversations with the same title and no URL/ID.

### Reuse and invalidation

- The selected pane is identified by workspace ID + pane ID. Position, agent
  label, cwd, state and title are refreshed metadata.
- A missing pane or workspace mismatch invalidates the executor selection.
- An unreachable/ambiguous reviewer invalidates the reviewer selection.
- Valid selections remain untouched when the other selection fails.
- Explicit user instructions replace the named selection immediately.

## Workflow integration

Add `codex-herdr` blocks to `.trellis/workflow.md` for planning and execution.
The execution block must tell Codex to load task/spec context as controller,
ensure the conversation routing selection exists, dispatch the bounded slice
with `herdr agent prompt`, use bounded `herdr agent wait/read` monitoring,
inspect/approve only commands within the safety contract, independently
validate the executor's handoff, and send the verified commit to the selected
reviewer.

The per-turn `<codex-mode>` banner must distinguish Herdr external execution
from native sub-agents and inline work. It must not claim that the current main
session implements directly.

## Reviewer integration

Reviewer selection is expressed as a ChatGPT conversation identity, not only a
title. Existing project conversations can be resolved from a supplied
conversation reference/URL or the project UI. Creating a reviewer conversation
uses the chosen project's composer and the first review/setup message; creating
or messaging a conversation occurs only after the user selects it.

The existing external-review request/response contract remains applicable.
Project-wide guidance becomes destination-neutral. Historical `rust0916`
mentions inside completed or active task evidence remain factual records.

## Security and safety boundaries

- A discovered terminal is only a candidate. Discovery never authorizes
  sending it repository content or approving its commands.
- User selection authorizes the pane as executor for the current Codex
  conversation, subject to per-command safety inspection.
- Repository path is always explicit in executor prompts. A different cwd is
  reported and corrected before edits.
- Broad deletion, reset/rebase/force push, `git add -A`, secret access,
  out-of-repository writes and ambiguous commands remain prohibited.
- ChatGPT conversation creation and review submission must follow the existing
  UI confirmation/communication policy and the user's selected destination.

## Compatibility and migration

- Extend, do not reinterpret, `auto`, `sub-agent` and `inline`.
- `herdr` gets its own workflow namespace rather than overloading
  `codex-inline`.
- Existing `.trellis/.runtime/sessions/*.json` schema remains unchanged.
- Missing/corrupt routing files behave as unselected; they never imply inline.
- Remove hard-coded global assumptions about Claude/right pane/`rust0916`, but
  preserve historical task records and review evidence.

## Rollback

Reverting `.trellis/config.yaml` to `inline` restores current main-session
behavior. The routing runtime files are ignored by other modes and can remain
without affecting active-task resolution. Code changes are confined to local
Trellis/Codex integration and project specs.
