# Rust migration validation summary

Updated 2026-10-06. This is a public summary of existing evidence, not a new test run.
The Rust branch remains experimental: full native E2E, scaffold retirement and
explicit production approval are still required. Completed task lifecycle does
not imply complete feature coverage or performance acceptance.

## Native switch state task (2026-10-06; cumulative C2C review PASS)

The current S4–S6 implementation adds configured `switch1`–`switch17` tags to
the native HTTP API, a value-free capability inventory with config generation,
durable exact-value control, and generic native controls in both Vue shells.
Focused HTTP coverage is 14/14, the candidate-rebind and post-drain value
regressions are green, Node coverage is 29/29, both Vue bundles build, and the
canonical-temp-root native-host library run is 181/181 (four subprocess probes
ignored). One unrelated integration test cannot bind `127.0.0.2:0` in this
environment. The ordered release manifest now includes 225 per-path source
hashes and the controlled `/` + `/log` browser/DNS evidence is complete. C2C
found P1-4/P2-10/P2-11; the complete remediation snapshot passed, and a fresh
9-path delta review passed after the post-drain regression fix. This entry
does not claim production, default release, deployment, push, or cutover. See the
[switch contract](contracts/native-switch-state.md) and
[S4](validation-records/10-05-rust-native-switch-state-management/s4-status.md),
[S5](validation-records/10-05-rust-native-switch-state-management/s5-status.md),
and [S6](validation-records/10-05-rust-native-switch-state-management/s6-status.md)
records.

## Native WebUI runtime increment (2026-10-04; accepted and archived)

The approved S1–S6 task adds embedded `/`/`/log`/assets on native api.http,
pinned safe external ui/name mounts, truthful lifecycle health/shared version,
30-operation capability admission in both existing shells, and a separate opt-in
pure native build. S1–S6 exact-source reviews and separate cumulative review PASS
at c98dc5d1012ce73aed318504c1053b04ab15de08. Cumulative P2-1/P3-1 were repaired
with actual RED/GREEN proof. Final native-host391/0, Node26/0, HTTP14, fmt/strict
Clippy and rebuilt native/Go artifacts pass; workspace1194/0 is pre-repair evidence,
with other workspace packages unchanged. Three subprocess probes are intentionally
parent-invoked. Limited Go regression passed before repair; final real Go proof
serves the rebuilt assets. Both shells complete actual managed/unmanaged rule,
audit/details/cache/group/upstream DNS/files/restart workflows; current release
all-tabs has zero unsupported requests/page errors. Source-unavailable runtime,
external saturation/timeout/drain and actual Go404 proof passed.

See [final cumulative verdict](validation-records/10-04-rust-native-webui-runtime-capabilities/cumulative-review-result-final.md) and [S6 evidence](validation-records/10-04-rust-native-webui-runtime-capabilities/s6-status.md)
and [hosting contract](contracts/native-webui-hosting.md). Inherited accepted
source ac629018 and unrelated dirty changes are preserved; real HEAD/index stay
unchanged. Only demonstrated C11/C12 subitems are registered; full5D/Phase6,
scaffold retirement and production/default approval remain required.

Parent acceptance and archive completed after current-source checks and VM rerun391 native/26 frontend tests. [Acceptance record](validation-records/10-04-rust-native-webui-runtime-capabilities/owner-acceptance.md). Real source HEAD/index remain unchanged; no commit/push/deploy.

## Latest accepted source

### Accepted native special-groups task (2026-10-04)

S1–S7 and the separate original-baseline cumulative C2C review have explicit
`FINAL: PASS` on exact tested source `ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc`
(tree `1575272a271e02ca9b7bd5051b651e9d63f471cf`). Cumulative P1-1 was corrected
through memory and persisted supplier provenance; both failed review objects
and all failed validation attempts remain visible. Final isolated verification:
workspace 1,184 passed/0 failed/3 parent-invoked subprocess entrypoints,
native-host 381 passed, libraries 410 passed; strict fmt/Clippy and native build.
Real DNS/HTTP save/shutdown/new-process restart preserves three group-cache hits'
entry/peer/transport without new attempts or peer requests. Existing Go reader
accepts all three native dumps. Prior Vue builds and controlled browser proof
remain applicable to unchanged frontend inputs.

See the [final cumulative result](validation-records/10-02-rust-native-special-groups-upstream-management/cumulative-review-remediation-round-2-result.md),
[latest source manifest](validation-records/10-02-rust-native-special-groups-upstream-management/evidence/cumulative-p1-1-persistence-source-manifest.json),
[115-artifact remediation evidence manifest](validation-records/10-02-rust-native-special-groups-upstream-management/evidence/cumulative-p1-1-persistence-evidence-manifest.json),
and [S7 history](validation-records/10-02-rust-native-special-groups-upstream-management/s7-status.md).
The real branch HEAD/index are unchanged; no push/deployment/production switch
or archive occurred. This task PASS does not replace the later full migration,
scaffold retirement and production approval gates.

Client context/ECS source f9c523bb5ae1ce76f8fd698df57abff8b49e792f received
S1-S6 and exact cumulative C2C PASS. Recorded checks: workspace 801 plus native
299 tests (1100 total), strict clippy, fmt, split builds, actual Go dump semantic
interoperability, controlled DNS/API/Vue and shutdown/restart proof. Parent
verification matched129 local/remote source manifests and independently reran28
focused tests. [Closure](validation-records/10-02-rust-native-client-context-ecs/research/closure-review.md)
and [owner verification](validation-records/10-02-rust-native-client-context-ecs/research/owner-acceptance.md)
preserve the exact scope, source and limitations.

Response policy source13d60d3e: final cumulative PASS and1070-test evidence;
plain-text/inline hosts/redirect/TTL/response-IP scope only. Upstream, fallback
and cache lifecycle evidence is retained in validation-records and their native
contracts. Failed resource/fixture attempts are not reclassified as passing.

Phase5A measurement reliability closed with an incomplete matrix; no capacity,
complete recovery or multi-core performance claim. See
[measurement closeout](phase5a-measurement-reliability.md) and
[native comparison](phase5a-native-comparison.md).

## Historical lifecycle inventory

The following statuses are metadata snapshots, not standalone acceptance verdicts.
Read linked product reports and coverage limitations before interpreting them.

| Product task | Recorded lifecycle |
| --- | --- |
| 09-28-rust-next-step-roadmap | planning |
| 08-13-rust-cache-foundation | completed |
| 08-13-rust-matcher-foundation | completed |
| 08-13-rust-matcher-phase2-expansion | completed |
| 08-15-rust-phase3-query-execution-core | completed |
| 08-17-rust-phase2-matcher-correctness-remediation | completed |
| 08-17-rust-phase3b-sequence-execution-foundation | completed |
| 08-17-rust-phase4-upstream-foundation | completed |
| 09-16-rust-phase4-secure-upstream-foundation | completed |
| 09-17-rust-phase4-endpoint-resolution-foundation | completed |
| 09-18-rust-phase4-connection-reuse-pipeline | completed |
| 09-18-rust-phase4-dual-stack-endpoint-selection | completed |
| 09-18-rust-phase4-quic-http3-doq-foundation | completed |
| 09-19-ci-rust-foundation-lint-doc-path-filter | completed |
| 09-20-rust-phase4-quic-reuse-multiplexing | completed |
| 09-21-rust-phase5a-baseline | completed |
| 09-22-rust-phase5a-native-cache | completed |
| 09-22-rust-phase5a-native-forwarding | completed |
| 09-22-rust-phase5a-native-routing | completed |
| 09-23-matcher-adapter-typed-nil | completed |
| 09-23-rust-phase5a-first-native-performance | completed |
| 09-24-rust-phase5a-native-query-observability | completed |
| 09-27-rust-phase5a-measurement-reliability | completed |
| 09-27-rust-phase5b-config-sequence-composition | completed |
| 09-28-rust-mos-test-native-sidecar-canary | superseded |
| 09-28-rust-mosdns-rust-canary-review-restart | completed |
| 09-28-rust-native-domain-set-management | completed |
| 09-28-rust-native-fast-mark-flow-setter | completed |
| 09-30-rust-native-next-bundled-delivery | completed |
| 09-30-rust-native-query-diagnostics-overview | completed |
| 09-30-rust-native-upstream-forwarding | completed |
| 10-01-rust-native-cache-lifecycle-management | completed |
| 10-01-rust-native-query-fallback-address-preference | completed |
| 10-02-rust-native-client-context-ecs | completed |
| 10-02-rust-native-response-policy-ip-rules | completed |

## Local-only evidence

Raw logs, captures, reviewer transport/runtime state, task automation and developer
journals remain in the local-only directories and external backup. Selected
Markdown records preserve linked contracts and validation summaries publicly.
