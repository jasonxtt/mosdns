# Current capability evidence

## Trellis/Codex

- `.trellis/config.yaml` currently supports `auto`, legacy `sub-agent`, and
  `inline`; it is explicitly set to `inline`.
- `.codex/hooks/inject-workflow-state.py` emits a per-turn `<codex-mode>` banner
  and selects the workflow breadcrumb namespace.
- `.trellis/scripts/common/active_task.py` verifies that Codex CLI provides
  `CODEX_THREAD_ID` to shell children and uses it to build a session-scoped
  context key.
- `.trellis/.runtime/sessions/codex_<thread-id>.json` demonstrates that separate
  Codex conversations already have isolated runtime records.

## Herdr

- `herdr agent list` returns all visible agents/panes with pane ID, workspace,
  detected agent, state, cwd and terminal title.
- `herdr agent explain <pane>` returns the detection rule and evidence.
- The current workspace demonstrated multiple Codex panes and a Claude pane,
  proving that position and agent labels should be presented as evidence rather
  than encoded as the selection rule.

## ChatGPT reviewer conversations

- The ChatGPT `mosdns` project page lists existing project conversations,
  including `rust0916` and other review conversations.
- The project page exposes a `mosdns中的新聊天` composer, so a new reviewer
  conversation can be created inside the project.
- The global ChatGPT sidebar exposes `新聊天`, so a non-project conversation is
  also available.
- Official OpenAI documentation states that projects contain multiple chats
  and that starting a chat from a project uses the project's shared context.
- Codex CLI itself does not expose the ChatGPT Projects view, so project chat
  listing/creation remains a browser/app interaction rather than a Codex CLI
  configuration feature.
