# AGENTS.md

Start here.

If you are a successor agent working on this repository, read files in this order:

1. `AGENTS.md`
2. `docs/ai/project-context.md`
3. `docs/ai/config-notes.md`
4. `docs/ai/handover.md`

This repository is a maintained fork of `yyysuo/mosdns` with custom routing, custom WebUI behavior, and operator-specific workflows.

## High-signal facts

- In normal operator workflow, infer the intended line from the project folder: `mosdns` means `main`, `mosdns-lite` means `lite`.
- The default UI at `/` is the maintained Vue UI.
- The legacy UI is kept at `/log` for compatibility and comparison.
- `webui-log/` is the current main Vue frontend workspace even though the directory name still says `log`.
- The canonical backend feature name is `special_groups`. Do not reintroduce the old placeholder name `route_group`.
- This fork intentionally does not follow the `nft` or `eBPF` direction from upstream.
- Upstream changes on or before `2026-04-18` were already reviewed in prior work. Skip re-reviewing them unless the user explicitly asks.

## Working rules for this fork

- This repo has a local CodeGraph index under `.codegraph/`. Use CodeGraph for cross-module symbols, call chains, and impact analysis; use `rg` for exact text, config fields, YAML, docs, and UI copy searches. For small UI copy or single-file edits, `rg` alone is enough.
- Preserve behavior parity first when changing the main UI. Do not redesign core flows on `/` unless the user asks.
- Treat WebUI changes as configuration workflow changes, not just frontend styling. Saving in the UI is expected to affect generated config and runtime behavior.
- Be conservative with mobile WebUI table layout changes. In this fork, some users access `/` through mobile browsers or embedded WebViews with inconsistent CSS table behavior.
- Prefer established terminology: `special_groups`, dedicated upstream groups, online rules, local/manual lists.
- Do not store passwords, tokens, or private credentials in repo docs.

## Operational expectations

- Current known deployment targets:
  - test: `10.0.0.91` (`mos-test`)
  - production: `10.0.0.3` (`mosdns`)
  - related debug hosts: `10.0.0.2` (`sing-box`), `10.0.0.6` (`network-vm`)
- Credentials for deployment hosts must not be stored in repo docs. Keep only non-secret host/path notes here and use a private local credential source outside the repository.
- On this machine, prefer the SSH aliases defined in `~/.ssh/config` over typing raw IPs when they are available.
- The user often wants real deployment verification, not only local compilation.
- When discussing sync with upstream, use exact dates and keep the already-reviewed cutoff in mind.
- Release and deployment binaries must embed freshly built Vue assets. Use the repo build scripts/workflows rather than plain `go build`; they build `webui-log/` first. If bypassing them, rebuild both Vue UIs before Go build.
- For UI-only development and browser verification, use Vite instead of rebuilding and replacing the mosdns binary:
  - `/`: `cd webui-log && MOSDNS_DEV_TARGET=http://10.0.0.91 npm run dev -- --host 0.0.0.0`
  - `/log`: `cd webui-log && MOSDNS_DEV_TARGET=http://10.0.0.91 npm run dev:log1 -- --host 0.0.0.0 --port 5174`, then open `/log1.index.html`
  - Vite serves local source with hot reload and proxies `/api`, `/plugins`, and `/metrics` to the test host. This is only for UI-only testing; embedded-asset verification still requires a real UI build and binary.
- For normal GitHub releases in this fork, do not require `gh` or a manually created GitHub Release page just to publish. Bumping the version, committing release notes, and pushing the release tag to GitHub is the expected path; the GitHub-side cloud build is triggered naturally by the version tag push.
- Config compatibility is controlled by `requiredConfigSchema` and `requiredConfigPackageID` in `coremain/config_update.go`. Keep both unchanged for binary-only releases. For structural config releases, follow `docs/ai/config-notes.md`.
- Switch plugin bit allocation shares namespace with config `fast_mark`. Before adding or renumbering a switcher, grep active config packages for the candidate bit. `fast_mark 48` is reserved by the unified matcher sentinel; `switch17` uses bit 49. Details and verification fields are in `docs/ai/config-notes.md`.
- When a task includes deployment verification, prefer this sequence:
  - build locally
  - validate on `mos-test` / `10.0.0.91`
  - promote to `mosdns` / `10.0.0.3` only after confirmation

## Frontend compatibility notes

- Mobile browser compatibility matters for the maintained Vue UI. Do not assume Chrome desktop behavior matches Android browsers, iOS Safari, or embedded WebViews.
- Treat overview-card and mobile table CSS as behavior-sensitive, not decorative-only styling.
- Avoid the known bad combination of `table-layout: fixed`, `calc(...)` column widths, and broad `overflow-wrap: anywhere` rules in narrow metric tables.
- If only some phones reproduce the issue, treat it as a browser compatibility bug first, not a data bug.

## When you need deeper context

- Project shape and code map: `docs/ai/project-context.md`
- Config generation and runtime behavior: `docs/ai/config-notes.md`
- Current state, pending work, and pitfalls: `docs/ai/handover.md`
