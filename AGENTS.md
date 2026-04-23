# AGENTS.md

Start here.

If you are a successor agent working on this repository, read files in this order:

1. `AGENTS.md`
2. `docs/ai/project-context.md`
3. `docs/ai/config-notes.md`
4. `docs/ai/handover.md`

This repository is a maintained fork of `yyysuo/mosdns`. The main work here is not generic upstream sync. It is a fork with custom routing, custom WebUI behavior, and operator-specific workflow decisions already made.

## High-signal facts

- The default UI at `/` is the maintained Vue UI.
- The legacy UI is kept at `/log` for compatibility and comparison.
- `webui-log/` is the current main Vue frontend workspace even though the directory name still says `log`.
- `webui-blog/` is an experimental Bento-style UI workspace. It is paused and not the active production UI.
- The canonical backend feature name is `special_groups`. Do not reintroduce the old placeholder name `route_group`.
- This fork intentionally does not follow the `nft` or `eBPF` direction from upstream.
- Upstream changes on or before `2026-04-18` were already reviewed in prior work. Skip re-reviewing them unless the user explicitly asks.

## Working rules for this fork

- Preserve behavior parity first when changing the main UI. Do not redesign core flows on `/` unless the user asks.
- Treat WebUI changes as configuration workflow changes, not just frontend styling. Saving in the UI is expected to affect generated config and runtime behavior.
- Prefer the fork's established terminology:
  - `special_groups`
  - dedicated upstream groups
  - online rules
  - local/manual lists
- Do not store passwords, tokens, or private credentials in repo docs.

## Operational expectations

- Typical validation flow is: local build -> test host -> production host.
- The user often wants real deployment verification, not only local compilation.
- When discussing sync with upstream, use exact dates and keep the already-reviewed cutoff in mind.

## Current known design decisions

- The query log and routing UI are expected to show the effective final routing label, not every intermediate match that happened during evaluation.
- For the custom dedicated-routing workflow, manual lists and URL-based lists both bind to a specific upstream group.
- The newer custom routing path was intentionally kept out of global cache for now.
- For `HTTPS` (`Type65`) handling, the fork can block the whole record today, but selective stripping of `ipv4hint`, `ipv6hint`, or `ECH` is not implemented yet. That feature is feasible as a response-rewrite plugin if requested.

## When you need deeper context

- Project shape and code map: `docs/ai/project-context.md`
- Config generation and runtime behavior: `docs/ai/config-notes.md`
- Current state, pending work, and pitfalls: `docs/ai/handover.md`
