# Project Context

## Stable model

This repository is an enhanced fork of `yyysuo/mosdns`. Its main value is the
operator workflow around:

- dedicated routing groups (`special_groups`)
- upstream-group binding
- online rule management
- query and audit visibility
- the maintained Vue WebUI

The `rust` branch is a **pure Rust project** (since Phase 6a it has no Go source and
needs no Go toolchain). It must preserve these **product contracts** while the
Rust-native host moves toward replacing the production Go program. The Go MosDNS
lives on `main` and in `/Users/tom/github/mosdns`; it is used only as the
behavioural reference (see `tools/go-reference/README.md`).

## Repository map

- `rust/`: the Cargo workspace. `native-host` is the product (`mosdns` binary, built-in `/` UI, management API, update guard); `dns-core`, `cache-core`, `matcher-core`, `sequence-core`, `upstream-core` are its libraries
- `rust/native-host/ui/`: the built `/` UI (checked in; produced from `webui-log/src` by `npm run build`, embedded by `rust/native-host/build.rs`)
- `webui-log/`: active Vue frontend workspace (historical name); `src/` builds the maintained UI (built in at `/`, or as an external package for any `ui/<name>/` with `npm run build:external`)
- `scripts/`: Rust build and release scripts (`build-rust-native.sh`, `build-rust-release.sh`, `generate-rust-update-manifest.sh`) and measurement helpers
- `tools/`: differential and evidence tools (Python, plus thin Go wrappers built from the Go repository by `tools/go-reference/build.sh`)
- `tests/phase5a-baseline/`: workload/config fixtures read by Rust tests, and the standalone Go load-generator helper (own `go.mod`, not needed by the Rust build)
- `docs/rust/contracts/`: public product and migration contracts
- `docs/rust/validation-summary.md`: accepted scope, validation evidence and limitations
- `.trellis/`, `.agents/`, `.codex/`: optional local-only development tools, not tracked or required for builds
- `docs/`: fork notes, Go release documentation (history), and Rust migration evidence

## UI topology

The normal route contract is:

- `/` → maintained Vue UI (built in)
- `/log` → does not exist on the Rust-native host (the Go repository's compatibility UI)
- `/<name>/` → optional external UI package from `<config root>/ui/<name>/`, any directory name (Rust-native; see `docs/rust/external-ui.md`)

Do not infer the route from a directory name. Verify `rust/native-host/build.rs`,
`rust/native-host/src/static_ui.rs`, `rust/native-host/ui/index.html` and the Vite
configuration before changing UI behavior.

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
