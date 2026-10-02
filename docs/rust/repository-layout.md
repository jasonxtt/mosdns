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

## WebUI retained

`/` serves `coremain/www/log.html`, built from `webui-log/src` into
`coremain/www/assets/vue-log`. `/log` serves `coremain/www/log1.html`, built from
`webui-log/src-log1` into `coremain/www/assets/vue-log1`. The Go host embeds
`coremain/www/*`; existing build/release workflows build both bundles. None of
these sources/assets/routes/workflows were removed or modified by this cleanup.
`webui-blog/` also remains pending a separate removal decision. No current build
or route reference to it was found; its output path is assets/vue-blog.

## Git history

This cleanup changes the current tracked tree only. Existing Git history is not
rewritten; old tracked workflow files remain retrievable from earlier commits.
