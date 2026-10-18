# AGENTS.md

Start here.

Read these files for normal work:

1. `AGENTS.md`
2. `docs/ai/project-context.md`
3. `docs/ai/config-notes.md`

When working on the Rust branch or Rust migration, also read:

4. `docs/ai/rust-handover.md`
5. `docs/ai/rust-rewrite-plan.md`
6. `docs/rust/contracts/index.md`
7. `.trellis/workflow.md` when optional local Trellis tooling is installed and a task is active

This repository is a maintained fork of `yyysuo/mosdns`. The current worktree
is the dedicated `rust` branch at `/Users/tom/github/mosdns-rust`; do not infer
`main` from the repository name or switch branches merely because the folder
name is similar.

## Scope and stable facts

- **The `rust` branch is a pure Rust project** (Phase 6a, 2026-10-18): it has no Go source, no `go.mod`, no cgo bridge (`rust/runtime`, `MOSDNS_*_BACKEND`) and needs no Go toolchain to build. The Go MosDNS lives on `main` and in `/Users/tom/github/mosdns` (`jasonxtt/mosdns`); the Go-side comparison tools and the reference binary are built from a pinned commit of that repository with `tools/go-reference/build.sh` (see `tools/go-reference/README.md`). Paths like `coremain/*.go` in Rust comments, contracts and validation records name files of that Go repository.
- The final Rust migration target is a pure Rust-native MosDNS binary/runtime; the Rust-native host still has to pass its final cutover gate before it replaces the production Go program.
- `/` is the maintained Vue UI, built from `webui-log/src` into `rust/native-host/ui/` (checked in, embedded by `rust/native-host/build.rs`). The Rust-native host serves `/` (built in) and any external UI package placed in `<config root>/ui/<name>/` at `/<name>/` (`main` is only an example; see `docs/rust/external-ui.md`); it has no `/log` (that compatibility UI exists only in the Go repository). `webui-log/` is the active frontend workspace despite its historical name.
- The canonical backend feature name is `special_groups`; do not reintroduce `route_group`.
- Do not import the upstream `nft` / `eBPF` direction. The detailed cutoff and decisions are in `UPSTREAM_SYNC.md`.

## Working rules

- Use `rg` for normal code, config, YAML, documentation, and UI-copy searches. Verify cross-module assumptions against current source.
- Product contracts and public validation summaries live in `docs/rust/contracts/` and `docs/rust/validation-summary.md`. Optional Trellis/AI tooling (`.trellis/`, `.agents/`, `.codex/`) is local-only and is not required to build a fresh clone. When a local Trellis task is active, use its `prd.md`, `design.md`, and `implement.md` as implementation gates. Keep Trellis auto-commit disabled and preserve unrelated dirty-worktree changes.
- Preserve the MosDNS product contract: YAML/config and sequence/plugin semantics, final DNS/routing/audit behavior, WebUI/API workflows, metrics/persistent formats, and other explicitly frozen user-visible behavior. Current Go code is a discovery reference, not the normative Rust implementation. Do not reproduce Go internals or accidental quirks unless they are explicitly classified as product contract.
- The Phase 1/2/3A hybrid scaffolding (cgo adapters, `MOSDNS_*_BACKEND` selectors, Go mirrors/fallback, paired generations) was removed with the Go source in Phase 6a; do not reintroduce it. Historical documents about it (`cache-compatibility.md`, `matcher-compatibility.md`, `test-host-*.md`, validation records) stay as evidence.
- Treat WebUI changes as configuration workflow changes, not just frontend styling.
- Treat overview-card and narrow mobile-table CSS as behavior-sensitive. Avoid reintroducing the combination of `table-layout: fixed`, `calc(...)` column widths, and broad `overflow-wrap: anywhere` rules.
- Do not store passwords, tokens, or private credentials in repository docs.

## Build and validation

- For binaries that embed the Vue UI, use `scripts/build-rust-native.sh` (it builds the UI bundle into `rust/native-host/ui/` before `cargo build`; release artifacts: `scripts/build-rust-release.sh`, external UI package: `npm run build:external` in `webui-log`). Do not run frontend and Rust builds in parallel.
- Do not make the incomplete `rust` branch a production/default release. Phase 3B+ foundations should target the final Rust-native host directly; production replacement is allowed only after the Rust-native E2E gate and the later hybrid-scaffolding retirement gate.
- Config compatibility and switch-bit reservations are documented in `docs/ai/config-notes.md`; do not change them casually.
- When real deployment verification is requested, build locally, validate on `mos-test`, and promote to `mosdns` only after confirmation. Prefer the SSH aliases in `~/.ssh/config`.

## When you need deeper context

- Project shape and runtime behavior: `docs/ai/project-context.md`
- Config generation, package boundaries, and switch bits: `docs/ai/config-notes.md`
- Rust migration state and worktree ownership: `docs/ai/rust-handover.md`
- Rust architecture and phase gates: `docs/ai/rust-rewrite-plan.md`
- Upstream sync baseline and exclusions: `UPSTREAM_SYNC.md`
