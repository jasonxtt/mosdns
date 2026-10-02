# S7 completed evidence — 2026-10-02

Review scope is HEAD 368daef0d25cf92121ee51a27671a326860c5032 to the task-owned working tree. No commit, push, deployment or local compilation. All builds/tests run through SSH alias mosdns-rust in /root/mosdns-rust-cache-lifecycle-20261001, offline Cargo, one job, no incremental/debug info. Preserve unrelated inherited dirty files. Same C2C conversation: https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abefd59-a3c0-83e8-ad5f-e599acf02c06 . Iterations 6/7/8/9/10 passed S1-S2/S3/S4/S5/S6 respectively; iteration 11 passed S7 and final task closure.

## Implementation and automated checks

Existing Vue DataManagementManager/DataCachePanel now read validated schema-1 named inventory. Only 404 or explicit unsupported falls back to Go legacy discovery; empty native catalog stays empty; timeout/500/unknown schema is an error with a persistent cache-local alert. Metrics absent/invalid are unavailable (dash), genuine zero remains zero; tag escaping is exact. Batch clear reports settled success count and failed tags. Details and flush encode tags. New cacheInventory Node tests cover these decisions without introducing a framework.

The real CLI SIGTERM proof initially failed: process was killed without cache drain/save. Native serve_host now captures TERM/INT, cancels and awaits the owning host scope, and preserves aggregated save-error exit code 2. An actual child-binary test checks two final-save failures are both diagnosed. Final focused test also exercises unusual inventory tags through percent-encoded plugin URLs and escaped Prometheus labels; malformed encodings return 404 and valid actions retain 405 semantics.

- Full Rust workspace: 1031 passed, zero failed; workspace clippy all-targets -D warnings, fmt and native binary build passed. Raw output /tmp/cache-s7-final-rust.log. This full run precedes the final narrow encoded-tag API change.
- After encoded-tag change: workspace clippy/fmt/native build and cache_http (3), slice6_management_http (10), slice7_management_publication (12): 25 passed, zero failed. Raw /tmp/cache-s7-final-rust2.log.
- Final frontend: four Node tests and both existing npm run build / build:log1 bundles passed. /tmp/cache-s7-ui-final.log. Existing chunk-size warning only. Bundles built remotely; generated assets not copied over inherited local changes.
- Genuine Go v2 generator and Go reader interoperability remains covered by S5 evidence/fixture. No claim of ECS support, whole Rust migration cutover, static assets or full metrics parity.

## Real DNS/API/browser proof

Sources and raw oracle outputs are in research/browser-proof. Actual existing Vue served against native API and a controlled UDP upstream; two real named caches alpha/beta, beta intentionally has an unwritable missing-directory dump path. Disposable loopback ports, no production services or data.

Observed initial old Vue showed seven nonexistent legacy caches with fabricated zeros despite alpha/beta inventory. Fixed UI shows only alpha/beta. Real DNS miss returned TTL2 address .1; fresh hit reused it; two lazy followers returned TTL5 and shared one refresh; released refresh returned .2 with TTL300. Upstream count was two for five client requests. Foreground audit/counters excluded background refresh. Alpha save, native shutdown/restart, then hit reused .2 without another upstream query and restored domain_set/original timestamps (lifecycle.json/restart.json).

Browser clicked real size/details: Case.example [group], original timestamps, DNS message sections and .2 answer. Answer search found .2; nonexistent search yielded empty page. Repopulated both caches with .3 and clicked Clear All plus its confirmation: notice reported one success and failed beta; alpha size became zero while beta retained size one. Backend raw before-partial-clear.json/after-partial-clear.json corroborates durable-first partial failure. Separate owned native stop caused inventory HTTP500; persistent cache-local alert and error row were visible with no legacy fallback. This test discovered sibling loading overwrote global notice; cache-local error prop fixes it. Screenshots of details and partial clear were displayed inline during execution.

Initial proof harness attempts failed on unsupported fixture ttl plugin, audit GET instead of POST, and matched sequence falling through twice; corrected only disposable harness config/oracle. Initial cargo offline signal dependency lookup lacked errno; transferred cached crate, retried successfully. These failures are recorded, not represented as passing product proofs. Signal red/green logs are /tmp/cache-s7-signal-red.log and /tmp/cache-s7-signal2.log. Final source/validation output is available via C2C execution_output.

## Completion boundary

S7 and full-task closure independently reviewed in the same C2C conversation: iteration 11 FINAL: PASS / DONE on 2026-10-02; no stable open finding. No unresolved current test failures. The approved ECS deferral and production cutover gates remain outside this task. Native cache API/frontend integration and shutdown lifecycle are included; no new cache syntax or hybrid backend pattern was added.

Final binary restarted after successful alpha durable-empty flush: real Vue refresh recovered its catalog with alpha/beta both zero and cleared the cache-local loading alert. Alpha's cleared entry did not resurrect; beta starts empty because its intentionally missing dump is not persisted.

Final review: S7 inventory/fallback, metrics, partial failure, detail workflow and CLI signal drain/save all PASS; full S1–S7 closure PASS / DONE. Task status completed; kept unarchived to retain existing review/evidence paths while source remains uncommitted.

Cleanup stopped only owned native/Vite/upstream processes after PID plus start-time verification and closed the proof tunnel. Initial Vite cleanup refused because whitespace in Linux comm shifted naive stat splitting; parsing after the closing parenthesis confirmed the original start time and cleanup then succeeded. Proof start helper corrected; no product source changed after final review. Original C2C tab remains available.
