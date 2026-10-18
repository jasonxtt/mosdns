# Public repository and local development files

The public repository contains MosDNS code, tests, build/release workflows,
product contracts and selected validation summaries. Trellis, Codex and shared
agent tooling are optional local development resources, retained locally but
excluded from Git tracking. A fresh clone does not require them to build MosDNS.

- Product contracts: [index](contracts/index.md).
- Validation and explicit limitations: [summary](validation-summary.md).
- Architecture and migration gates: [rewrite plan](../ai/rust-rewrite-plan.md).
- `.trellis/`, `.agents/`, `.codex/`: local only; install/configure separately if needed.
- Old `.superpowers/` and `docs/superpowers/` design drafts: backed up outside the repository and removed from the current tree.
- Root `proxy.o`: obsolete eBPF object removed; nft/eBPF remains excluded by [upstream policy](../../UPSTREAM_SYNC.md).

## WebUI

`/` serves `rust/native-host/ui/index.html`, built from `webui-log/src` into
`rust/native-host/ui/assets/vue-log` (checked in, embedded by
`rust/native-host/build.rs`). Since Phase 6a the `rust` branch has no Go host and no
`/log` compatibility UI (`webui-log/src-log1` was removed; it lives in the Go
repository). The leftover Go-era `webui-blog/` source was removed on 2026-10-18 (user
decision); it remains in git history and in the Go repository.

## Pure Rust branch (Phase 6a)

Removed from this branch: all Go source and module files (`coremain/`, `plugin/`,
`pkg/`, `mlog/`, `main.go`, `go.mod`, `go.sum`, `tools/*.go`), the Go build/release files
(`Dockerfile*`, `release.py`, `minify_safe.py`, `postcss.config.js`, the Go workflows),
the cgo bridge (`rust/runtime`, `scripts/build-rust-cache.sh` and the other hybrid
scripts) and the `/log` sources. The Go MosDNS stays on `main` and in
`/Users/tom/github/mosdns`; `tools/go-reference/build.sh` builds the Go reference
binary and the Go-side comparison tools from a pinned commit of it. The Phase 5a load
generator `tests/phase5a-baseline/cmd/phase5a-baseline` remains as a standalone Go
module (its configs and workloads are fixtures of Rust tests). Details and the full
list: `docs/rust/validation-records/10-18-p6a/README.md`.

## Git history

This cleanup changes the current tracked tree only. Existing Git history is not
rewritten; old tracked workflow files remain retrievable from earlier commits.
