# AGENTS.md

Start here.

Read these files for normal work:

1. `AGENTS.md`
2. `docs/ai/project-context.md`
3. `docs/ai/config-notes.md`

When working on the Rust branch or Rust migration, also read:

4. `docs/ai/rust-handover.md`
5. `docs/ai/rust-rewrite-plan.md`
6. `.trellis/workflow.md` when a Trellis task is active

This repository is a maintained fork of `yyysuo/mosdns`. The current worktree
is the dedicated `rust` branch at `/Users/tom/github/mosdns-rust`; do not infer
`main` from the repository name or switch branches merely because the folder
name is similar.

## Scope and stable facts

- The final Rust migration target is a pure Rust-native MosDNS binary/runtime. The current Go default and existing opt-in Rust bridges remain only as transitional validation scaffolding until the Rust-native host passes its final cutover gate.
- `/` is the maintained Vue UI. `/log` is the compatibility UI. `webui-log/` is the active frontend workspace despite its historical name.
- The canonical backend feature name is `special_groups`; do not reintroduce `route_group`.
- Do not import the upstream `nft` / `eBPF` direction. The detailed cutoff and decisions are in `UPSTREAM_SYNC.md`.

## Working rules

- Use `rg` for normal code, config, YAML, documentation, and UI-copy searches. Verify cross-module assumptions against current source.
- Rust migration work is governed by `.trellis/tasks/`: use each task's `prd.md`, `design.md`, and `implement.md` as its implementation gate. Keep Trellis auto-commit disabled and preserve unrelated dirty-worktree changes.
- Preserve the MosDNS product contract: YAML/config and sequence/plugin semantics, final DNS/routing/audit behavior, WebUI/API workflows, metrics/persistent formats, and other explicitly frozen user-visible behavior. Current Go code is a discovery reference, not the normative Rust implementation. Do not reproduce Go internals or accidental quirks unless they are explicitly classified as product contract.
- Phase 1/2/3A cgo adapters, `MOSDNS_*_BACKEND` selectors, Go mirrors/fallback, and paired-generation logic are temporary migration scaffolding. Keep existing paths safe until retirement, but do not extend this hybrid pattern into Phase 3B+ without explicit approval.
- Treat WebUI changes as configuration workflow changes, not just frontend styling.
- Treat overview-card and narrow mobile-table CSS as behavior-sensitive. Avoid reintroducing the combination of `table-layout: fixed`, `calc(...)` column widths, and broad `overflow-wrap: anywhere` rules.
- Do not store passwords, tokens, or private credentials in repository docs.

## Build and validation

- For binaries that embed the Vue UI, use the repository build scripts/workflows and build the required UI bundles before Go compilation. Do not run frontend and Go builds in parallel.
- Do not make the incomplete `rust` branch a production/default release. Phase 3B+ foundations should target the final Rust-native host directly; production replacement is allowed only after the Rust-native E2E gate and the later hybrid-scaffolding retirement gate.
- Config compatibility and switch-bit reservations are documented in `docs/ai/config-notes.md`; do not change them casually.
- When real deployment verification is requested, build locally, validate on `mos-test`, and promote to `mosdns` only after confirmation. Prefer the SSH aliases in `~/.ssh/config`.

## When you need deeper context

- Project shape and runtime behavior: `docs/ai/project-context.md`
- Config generation, package boundaries, and switch bits: `docs/ai/config-notes.md`
- Rust migration state and worktree ownership: `docs/ai/rust-handover.md`
- Rust architecture and phase gates: `docs/ai/rust-rewrite-plan.md`
- Upstream sync baseline and exclusions: `UPSTREAM_SYNC.md`
