# S6 isolated native build and whole-chain proof

S1–S6 exact-source and separate complete cumulative C2C reviews are **FINAL: PASS**.

The initial S6 evidence below is historical: final cumulative repairs supersede its source/binary/bundle identity. [Final verdict](cumulative-review-result-final.md), [repair evidence and final validation](cumulative-remediation-round1.md), [final manifest](cumulative-build-manifest.json) and [source/artifact binding](cumulative-source-artifact-match.json). Current native391/0, Node26/0, HTTP14/fmt/strict Clippy and rebuilt all-tabs/source-unavailable/Go proofs PASS; workspace1194 is pre-repair evidence. Human acceptance pending.

## Final source and artifact

Opt-in `scripts/build-rust-native.sh`: locked npm ci → maintained Vue → compatibility Vue → required-root/assets/version preflight → locked native release. Source/asset-hash-keyed Cargo directories prevent stale mtime cache reuse. Invalid CR/LF product versions stop before npm (actual Bash script RED/GREEN). Product version `10-04-native-ui+proof`, URL stamp `10-04-native-ui%2Bproof`; CLI and health agree. Source ID null. [Actual build manifest](s6-build-manifest.json) hashes219 inputs and embedded assets plus the executable. [Local/VM/physical-runtime match](s6-source-artifact-match.json) binds the exact final artifact; the script-only guard correction rebuilt to the identical executable SHA256, so the runtime evidence remains applicable. Fresh VM source has no .git/local Trellis/Codex tooling; building requires standard Rust/Node dependencies, runtime does not.

## Validation

- [Summary](s6-test-summary.json): workspace1194 passed/0 failed; native-host391 passed/0 failed (included in workspace); 3 parent-invoked subprocess probes intentionally ignored. fmt and all-target workspace strict Clippy PASS. Debug information and incremental caching disabled to fit owned disk; debug assertions remain enabled.
- Node26/0. [Limited CGO0 Go regression](s6-go-regression.log) and repository Go build PASS after both UI bundles; no blanket full/cgo-suite claim.
- [Current release all-tabs](s6-all-tabs/result.json): `/`9 and `/log`14 views, global refresh, pending/error/invalid/retry/old-native schema and three read-only reasons; unsupported requests0, page errors0, owned SIGINT exit0.
- [Source-unavailable clean-runtime proof](s6-native-source-unavailable/result.json): executable/config/data only, child PATH has no Go/Node/tools; actual source checkout temporarily unavailable and restored. Both same-origin shells use embedded roots/assets, health identity/null schemas. Actual UI group rename, upstream replacement and local-rule saves change controlled UDP DNS answers and final suppliers. Audit detail, named non-prefix cache inventory/list/flush, cache hit, durable files, refresh and process restart pass. Restart group-cache hit adds no supplier request. Real rule-directory persistence failure preserves drafts through refresh; recovery saves them. Eligible **unmanaged** file-backed rule edits also work in both shells and survive restart; unmanaged groups.manage is false independently.
- Actual external paths301/200, escape404, oversize413, live reread, no DNS/cache metric changes from static requests. Four16MiB held downloads cause503, health and DNS remain responsive, ten-second timeout permits recover, SIGINT drains held owners. Partial headers close at the five-second budget while health/DNS work. All owned PIDs exit0; API/DNS/group listeners can be rebound afterwards. No port53/public DNS.
- [Actual Go legacy proof](s6-go-final/result.json): real Go discovery404, both shells keep local-rule write/file/DNS behavior and legacy appearance requests. Served JS/CSS SHA256 matches final native assets. SIGINT exit0.
- Basename process regression: meaningful [HTTP404 RED](s6-basename-red3.log) → [real external200 GREEN](s6-basename-green3.log). File-backed startup now supplies `.` for a bare filename; explicit no-base in-memory semantics stay unchanged.
- [Build preflight](s6-build-preflight.json): valid/restore PASS; malformed version, absent referenced asset, forbidden key, symlink and mismatched stamp rejected.

## Failed attempts and cleanup

All failed logs remain: initial missing test-runner expect, browser modal selector/query/wait mismatches, invalid sequence/enable_audit test fixture and stale Cargo mtimes. One early selector timeout left an owned native process after an unhandled response-wait rejection; exact PID272376 was stopped with SIGINT and confirmed absent. Harness response waits now have rejection handlers and normal finally cleanup. These attempts are not PASS. Initial filesystem-basename failure is the actual product RED; corrected fixture reproduces404 separately. Owned debug/incremental build artifacts alone were cleared for disk space, then final tests rebuilt without debug info; unrelated services/files were untouched.

This closes only the delivered C11/C12 subitems. Full C11/C12, full5D/Phase6, hybrid retirement, performance/stability and production cutover remain gated. No push, deployment, default switch, ordinary automatic commit or archive.
