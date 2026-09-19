# Surface and routing evidence

## Repository evidence

- `.trellis/config.yaml` currently sets `codex.dispatch_mode: herdr`.
- `.trellis/scripts/common/codex_routing.py` validates only `herdr` and
  `inline` dispatch modes and requires a ChatGPT URL for reviewers.
- `.codex/hooks/inject-workflow-state.py` detects the broad `codex` platform,
  then emits a Herdr-specific banner when that repository mode is configured.
- `.trellis/spec/backend/quality-guidelines.md` already contains separate
  safety contracts for MCP DSH and Herdr, including bounded waits, exact diff
  inspection, parent-owned verification, and explicit reviewer PASS.
- Existing routing state is keyed by the Codex conversation and is therefore
  suitable for user-selected targets; the schema, not the storage boundary,
  needs to become provider-neutral.

## Current host evidence

The current Desktop/App process exposes the environment marker
`CODEX_APP_TOOLS_PIPE_PATH`. The value is intentionally not recorded. The
current `herdr agent list` response contains no agents, so this host cannot
be treated as a Herdr pane merely because the `herdr` executable exists.

## Available execution providers

The current tool inventory exposes MCP DSH operations for read-only
investigation, execute/start/wait, diff, apply, continue, cancel, and end.
Those tools are provider capabilities, not a concrete user selection. The
controller must still honor the user's explicit executor choice and the DSH
clean/snapshot safety rules.

## Design consequence

The host default may select a provider class, but it must not select a pane,
worker session, ChatGPT conversation, or model. Missing or ambiguous surface
evidence must be represented as `unknown` and resolved through an explicit
conversation choice. Explicit user selections, including current Codex as
executor or reviewer, take precedence over all defaults.
