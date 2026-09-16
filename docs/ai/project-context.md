# Project Context

## Stable model

This repository is an enhanced fork of `yyysuo/mosdns`. Its main value is the
operator workflow around:

- dedicated routing groups (`special_groups`)
- upstream-group binding
- online rule management
- query and audit visibility
- the maintained Vue WebUI

The Rust branch must preserve these **product contracts** while moving toward a
pure Rust-native host. The opt-in/versioned Go↔Rust boundaries already present
in cache, matcher, and query foundation are transitional migration scaffolding,
not the final architecture and not a pattern to extend automatically into new
Phase 3B+ modules.

## Repository map

- `coremain/`: HTTP/API server, runtime state, audit endpoints, and embedded assets under `coremain/www/`
- `plugin/`: executable plugins and sequence/routing behavior
- `pkg/`: shared DNS and query-context utilities
- `webui-log/`: active Vue frontend workspace; `src/` builds the maintained UI and `src-log1/` builds the compatibility UI
- `rust/`: experimental Rust cores, runtime, ABI, and adapters
- `.trellis/`: migration tasks, specs, and task runtime; use only within the active Trellis workflow
- `docs/`: fork notes, release documentation, and Rust migration evidence

## UI topology

The normal route contract is:

- `/` → maintained Vue UI
- `/log` → compatibility UI

Do not infer the route from a directory name. Verify `coremain/mosdns.go`, the
embedded HTML files, and the Vite configuration before changing UI behavior.

## Compatibility contracts

`special_groups`, online rules, local/manual lists, upstream-group binding,
generated routing order, final DNS/routing behavior, query audit fields,
metrics, runtime JSON state, persistent formats, and the existing WebUI/API are
product-contract surfaces for the Rust migration. Current Go data structures,
interfaces, fallback mechanisms, and incidental implementation quirks are not
compatibility surfaces unless a reviewed contract explicitly elevates them.

Diagnostics should prefer the effective final routing label, final upstream
group, and final upstream path. Intermediate matcher tags must not be shown as
if they were all effective.

The `/` UI contains real operator workflows, including overview diagnostics,
appearance persistence, and system settings. Changes to their save flows or CSS
can change runtime behavior.

## Source of truth

- Configuration generation, runtime JSON, package boundaries, and switch-bit rules: `docs/ai/config-notes.md`
- Rust migration state and worktree ownership: `docs/ai/rust-handover.md`
- Rust architecture and acceptance gates: `docs/ai/rust-rewrite-plan.md`
- Upstream cutoff and excluded upstream direction: `UPSTREAM_SYNC.md`
