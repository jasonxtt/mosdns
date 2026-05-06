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
- Be conservative with mobile WebUI table layout changes. In this fork, some users access `/` through mobile browsers or embedded WebViews with inconsistent CSS table behavior.
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
- For releases or deployment binaries that embed the Vue UI, build order matters: rebuild `webui-log/` first, then run `go build`. Do not run frontend build and Go build in parallel, or the binary can embed mismatched `app.js` / `app.css` assets.

## Current known design decisions

- The query log and routing UI are expected to show the effective final routing label, not every intermediate match that happened during evaluation.
- For the custom dedicated-routing workflow, manual lists and URL-based lists both bind to a specific upstream group.
- The newer custom routing path was intentionally kept out of global cache for now.
- The maintained `/` UI now exposes both `IPv4优先` and `IPV6屏蔽`, and they are intentionally treated as mutually exclusive operator modes in the UI and generated runtime flow.
- The maintained `/` UI now persists more operator appearance state server-side, including panel background, text color, and button color settings.
- For `HTTPS` (`Type65`) handling, the fork can block the whole record today, but selective stripping of `ipv4hint`, `ipv6hint`, or `ECH` is not implemented yet. That feature is feasible as a response-rewrite plugin if requested.

## Frontend compatibility notes

- Mobile browser compatibility matters for the maintained Vue UI. Do not assume Chrome desktop behavior matches Android browsers, iOS Safari, or embedded WebViews.
- The overview page has accumulated several custom layout behaviors, including adaptive visible-row counts, anchored trend-detail popovers, and narrow-screen truncation rules. Treat overview-card CSS as behavior-sensitive, not decorative-only styling.
- A confirmed pitfall in this fork is the combination of `table-layout: fixed` with `calc(...)` column widths inside narrow mobile cards. This caused the `/` overview `最慢查询` card to render differently across users:
  - some browsers looked normal
  - some mobile browsers collapsed the left domain column almost to zero width
  - the visible symptom was that only the right `耗时` column remained visible
- A related confirmed pitfall is mixing aggressive emergency wrapping rules such as `overflow-wrap: anywhere` with narrow fixed-layout metric tables. That combination can make domains or client identifiers break character-by-character on some screens.
- For narrow-screen metric tables, prefer stable sizing strategies:
  - keep the rigid width on the short numeric/status column if needed
  - let the primary text column use automatic remaining space
  - prefer single-line truncation with ellipsis for long text fields
  - avoid relying on `calc(100% - Npx)` for table columns in mobile cards unless it has been verified on real devices
- If a table needs emergency wrapping for long values, scope that behavior carefully. Global `overflow-wrap: anywhere` or aggressive `word-break` rules can interact badly with fixed-width mobile layouts.
- When touching overview-card or mobile table CSS, validate with the project's normal flow:
  - local build
  - test host
  - production host only after confirmation
- If users report that only some phones reproduce a layout issue, treat that as a browser compatibility bug first, not a data bug.

## When you need deeper context

- Project shape and code map: `docs/ai/project-context.md`
- Config generation and runtime behavior: `docs/ai/config-notes.md`
- Current state, pending work, and pitfalls: `docs/ai/handover.md`
