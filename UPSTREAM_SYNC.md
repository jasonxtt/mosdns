# Upstream Sync Baseline

- Upstream repo: `yyysuo/mosdns`
- Baseline date: `2026-04-18`

## Decisions already made

- All upstream updates on or before `2026-04-13` have already been manually reviewed and handled.
- Upstream commit `a5d1a1d` on `2026-04-15` is intentionally not followed.
- Reason: it only touches `plugin/executable/nftadd/proxy.o`, and this fork has intentionally removed the `nftadd`/related nft functionality.

## Rule for future sync work

- When asked to review or merge upstream updates, only inspect upstream commits after `2026-04-18` unless explicitly asked to re-check older history.
- Treat commits on or before `2026-04-18` as already triaged according to the decisions above.
