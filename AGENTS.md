# AGENTS.md

Start here.

If you are a successor agent working on this repository, read files in this order:

1. `AGENTS.md`
2. `docs/ai/project-context.md`
3. `docs/ai/config-notes.md`
4. `docs/ai/handover.md`

This repository is a maintained fork of `yyysuo/mosdns`. The main work here is not generic upstream sync. It is a fork with custom routing, custom WebUI behavior, and operator-specific workflow decisions already made.

## High-signal facts

- In normal operator workflow, infer the intended line from the project folder: `mosdns` means `main`, `mosdns-lite` means `lite`.
- The default UI at `/` is the maintained Vue UI.
- The legacy UI is kept at `/log` for compatibility and comparison.
- `webui-log/` is the current main Vue frontend workspace even though the directory name still says `log`.
- `webui-blog/` is an experimental Bento-style UI workspace. It is paused and not the active production UI.
- The canonical backend feature name is `special_groups`. Do not reintroduce the old placeholder name `route_group`.
- This fork intentionally does not follow the `nft` or `eBPF` direction from upstream.
- Upstream changes on or before `2026-04-18` were already reviewed in prior work. Skip re-reviewing them unless the user explicitly asks.

## Working rules for this fork

- This repo has a local CodeGraph index under `.codegraph/`. For non-trivial code navigation, run `codegraph sync /Users/tom/github/mosdns` first, then use `codegraph query`, `codegraph callers`, `codegraph impact`, or `codegraph context` alongside `rg`. Use CodeGraph for symbols, call chains, and impact analysis; use `rg` for exact text, config fields, YAML, docs, and UI copy searches.
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
- Current known deployment targets:
  - test host: `10.0.0.91` (`mos-test`)
  - production host: `10.0.0.3` (`mosdns`)
  - related hosts often used in debugging:
    - `10.0.0.2` (`sing-box`)
    - `10.0.0.6` (`network-vm`)
- Credentials for deployment hosts must not be stored in repo docs. Keep only non-secret host/path notes here and use a private local credential source outside the repository.
- On this machine, prefer the SSH aliases defined in `~/.ssh/config` over typing raw IPs when they are available.
- The user often wants real deployment verification, not only local compilation.
- When discussing sync with upstream, use exact dates and keep the already-reviewed cutoff in mind.
- For releases or deployment binaries that embed the Vue UI, build order matters: rebuild `webui-log/` first, then run `go build`. Do not run frontend build and Go build in parallel, or the binary can embed mismatched `app.js` / `app.css` assets.
- For normal GitHub releases in this fork, do not require `gh` or a manually created GitHub Release page just to publish. Bumping the version, committing release notes, and pushing the release tag to GitHub is the expected path; the GitHub-side cloud build is triggered naturally by the version tag push.
- Config compatibility is controlled by `requiredConfigSchema` and `requiredConfigPackageID` in `coremain/config_update.go`. Keep both unchanged for binary-only releases. When structural config changes are required, bump the schema and package ID, then rebuild the external `config_up.zip` with its matching manifest. The package remains external at `https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_up.zip`; it is not embedded in the binary.
- When structural config changes are required, maintain the external package source in `/Users/tom/github/file/mosdns/config/config_up` and the builder script `/Users/tom/github/file/mosdns/config/build_config_up.sh`: update the script `SCHEMA` / `PACKAGE_ID` to match the binary, add new maintained YAML files to `managed_files`, add removed `sub_config/*.yaml` files to `deleted_files`, run the script, and commit/push the regenerated `config_up.zip`, synced `config_all/`, and `config_all.zip` in the `jasonxtt/file` repository.
- User-facing config version text is separate from the internal schema. When config structure changes, bump the schema/package and also update the UI display mapping in `webui-log/src/components/SystemControlManager.vue` (current labels: `v1`, `v2`).
- Automatic config updates are transactional: the binary checks `/cus/mosdns/webinfo/config_update_state.json`, applies only manifest-managed structure files, creates a unique backup, validates the complete config tree, and commits the schema only after plugin initialization succeeds. Do not put user rule/state files in `managed_files`.
- Switch plugin bit allocation is shared with `query_context.GlobalSwitchMask` and the config `fast_mark` namespace. Before adding or renumbering a switcher, grep the active config packages for the candidate `fast_mark` value. `fast_mark 48` is already used in `config_custom.yaml` as the "quick-path / unified matcher already checked" sentinel (`!fast_mark 48` gates `$unified_matcher1`), so no switcher may use bit 48. The `switch17` DNS routing-mode plugin uses bit 49 to avoid this collision. A collision here causes every new query context to start with that fast mark already set, skips `unified_matcher1`, and makes white/grey/subscription/manual rules show as `unmatched_rule`, often falling through to FakeIP. Verify fixes with production audit fields such as `domain_set`, `effective_tag`, `final_sequence`, `final_upstream`, and `matched_rule_source`.
- When a task includes deployment verification, prefer this sequence:
  - build locally
  - validate on `mos-test` / `10.0.0.91`
  - promote to `mosdns` / `10.0.0.3` only after confirmation

## Current known design decisions

- The query log and routing UI are expected to show the effective final routing label, not every intermediate match that happened during evaluation.
- For the custom dedicated-routing workflow, manual lists and URL-based lists both bind to a specific upstream group.
- The newer custom routing path was intentionally kept out of global cache for now.
- The maintained `/` UI now exposes both `IPv4优先` and `IPV6屏蔽`, and they are intentionally treated as mutually exclusive operator modes in the UI and generated runtime flow.
- The maintained `/` UI now persists more operator appearance state server-side, including panel background, text color, and button color settings.
- Small runtime JSON state files are stored under `/cus/mosdns/webinfo` and are auto-migrated there from the runtime root or legacy `/cus/mosdns/state` by the binary. When both exist, prefer `webinfo/`.
- For `HTTPS` (`Type65`) handling, the fork can block the whole record today, but selective stripping of `ipv4hint`, `ipv6hint`, or `ECH` is not implemented yet. That feature is feasible as a response-rewrite plugin if requested.

## Frontend compatibility notes

- Mobile browser compatibility matters for the maintained Vue UI. Do not assume Chrome desktop behavior matches Android browsers, iOS Safari, or embedded WebViews.
- Treat overview-card and mobile table CSS as behavior-sensitive, not decorative-only styling.
- Avoid the known bad combination of `table-layout: fixed`, `calc(...)` column widths, and broad `overflow-wrap: anywhere` rules in narrow metric tables.
- Prefer a fixed narrow numeric column plus an automatic main text column with ellipsis, then verify on real devices if the change touches overview cards or narrow tables.
- When touching overview-card or mobile table CSS, validate with the project's normal flow:
  - local build
  - test host
  - production host only after confirmation
- If only some phones reproduce the issue, treat it as a browser compatibility bug first, not a data bug.

## When you need deeper context

- Project shape and code map: `docs/ai/project-context.md`
- Config generation and runtime behavior: `docs/ai/config-notes.md`
- Current state, pending work, and pitfalls: `docs/ai/handover.md`
