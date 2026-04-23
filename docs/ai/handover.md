# Handover

## Current state

As of `2026-04-22`, the fork is in a stable maintained state with these major outcomes already landed:

- Vue UI is the default UI at `/`
- legacy UI is retained at `/log`
- dedicated routing groups (`special_groups`) are implemented and actively used
- upstream group binding is implemented in the WebUI workflow
- subscription-rule save flow supports backend download behavior
- major dashboard pages were migrated to the Vue workflow

## Important active realities

- `webui-blog/` exists, but it is an experimental Bento-style branch and is currently paused
- the main ongoing UI target is `/`, not `/blog`
- the legacy UI is kept mainly as fallback, comparison target, and compatibility reference

## Things already decided

- do not follow upstream `nft`-related work for this fork
- do not re-review upstream changes on or before `2026-04-18` unless the user asks
- keep the main UI behavior close to the legacy UI first, then improve selectively
- manual lists and URL-based lists both bind to a specific upstream group in the custom workflow
- the newer custom routing path should not use global cache for now

## Known pitfalls

### 1. Directory name vs route mismatch

`webui-log/` is the main Vue workspace, even though `/log` is now the legacy UI route.

### 2. Naming mismatch

Use `special_groups`, not `route_group`.

### 3. Public docs vs operator notes

This repository can store process notes, but not secrets. Keep passwords and private tokens out of committed files.

## Pending / plausible next work

### Selective Type65 rewriting

The user asked whether the fork can support:

- removing `ipv4hint`
- removing `ipv6hint`
- removing `ECH` while keeping `Type65`

Current conclusion:

- not implemented yet
- feasible
- best implemented as a new response-rewrite plugin over `HTTPS` / `SVCB` answers

### Future UI or config improvements

Most future work is likely to continue in one of these directions:

- refine the maintained Vue UI
- add more explicit routing diagnostics
- add backend plugins that match existing config/UI concepts instead of ad-hoc toggles

## Fast-start advice for a successor agent

If the user opens a fresh task, do this first:

1. Read `AGENTS.md`
2. Read `docs/ai/project-context.md`
3. Read `docs/ai/config-notes.md`
4. Read `docs/ai/handover.md`
5. Confirm actual route mapping in `coremain/www/` before editing frontend behavior
6. Confirm whether the task targets the maintained `/` UI, the legacy `/log` UI, or the paused `webui-blog/` experiment
