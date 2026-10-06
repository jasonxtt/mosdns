# Native WebUI/runtime capability evidence (2026-10-04)

S1–S6 and the separate complete cumulative code review are **FINAL: PASS**.
See [final verdict](cumulative-review-result-final.md) and [cumulative remediation/final checks](cumulative-remediation-round1.md). Parent acceptance completed and task archived; see [acceptance record](owner-acceptance.md).

All product builds, tests, DNS/HTTP and browser runs were executed on the isolated
SSH host `mosdns-rust`, in `/root/mosdns-rust-webui-20261004`. This is controlled
loopback/high-port validation, without port53, public DNS, deployment or a default
release switch. The VM's pre-existing service was not changed.

- S1: shared CLI/HTTP version and truthful all-bind/apply/recovery lifecycle.
- S2: deterministic embedded roots/assets, conditional GET/HEAD, safe redirects,
  methods/path validation and bounded static owners. Review P2-1/P2-2 repaired.
- S3: pinned external mount containment, live file reads, bounded blocking owner
  lifetime and shutdown joining. Real HTTP plus deterministic held-worker proof.
- S4: actual 30-operation managed/unmanaged capability matrix. Review P2-1 repaired
  managed-empty local-write false positive.
- S5: shared discovery and request admission in both shells. Review P2-1 repaired
  adjacent read-only management reasons. Meaningful previous-bundle RED and both
  old-native unmanaged DOM checks are retained.
- S6: [whole-chain status](s6-status.md), independent source-keyed build,
  basename CLI external-mount regression, full workspace checks, actual Go404,
  current-artifact all-tabs and clean-runtime supported workflows.

Each slice has tested-source and review-result records. Failed runs are retained,
including compiler/lint, invalid test fixture, browser selector/query/wait and
stale-Cargo-mtime attempts. They are distinguished from meaningful product REDs.
No failed attempt is treated as acceptance evidence. C2C review objects are made
with a temporary index/commit-tree, without moving the actual branch HEAD/index.
The cumulative result records the final complete reviewed range.

## Reproduction

Fresh-clone native build: `BUILD_VERSION=<version> scripts/build-rust-native.sh`.
It requires Rust/Cargo and Node/npm for building; the executable needs neither Go
nor Node. Locked inputs and actual source/asset/artifact hashes are in
`s6-build-manifest.json`. Both Vue outputs are built in sequence before Cargo;
the limited Go build ran afterwards through the existing repository script.

VM checks: fmt, workspace all-target strict Clippy, native-host and workspace
locked cargo tests, then `node --test webui-log/tests/*.test.mjs`. Final test runs
set `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` to
limit owned build-cache size; debug assertions remain enabled. Three intentionally
ignored subprocess entrypoints are exercised by their parent tests.

Browser harnesses require pinned Playwright1.56.1/Chromium141 in the task-owned
`browser-tools` directory. `s5-browser-check.mjs` accepts `NATIVE_BINARY` for the
current release, visits both full shells and tests pending/error/retry/old schema.
`s6-native-proof.mjs` copies the executable into a fresh runtime-only fixture;
its child uses a PATH with no tools. Controlled UDP suppliers record actual
names/counts. It performs actual UI mutations and observes persisted files,
DNS/cache/final suppliers, real persistence errors and restart. Source-unavailable
proof temporarily moves only this VM checkout aside and restores it in `finally`.
`s6-go-legacy-proof.mjs` uses an actual separately built Go process; no forced
native404 or proxy. It compares served assets to the final native build manifest.

Fixtures and browser test tools are not product dependencies. Result JSON/logs
and screenshots are copied here; bulky caches, runtime binaries and fixture data
remain outside Git. Linux artifact is also retained in the local ignored release
directory for handoff. This record does not close complete C11/C12, 5D, Phase6,
hybrid retirement, performance/stability or production approval.
