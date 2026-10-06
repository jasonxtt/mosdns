# Rust migration handover

Last verified: `2026-10-04`

Concise cross-session handover for the Rust migration on branch `rust`.

## Current planning — native switch state (2026-10-05)

[Next task plan](../rust/plans/native-switch-state-management.md) covers all
switch1–17 query/admission semantics, durable configured state owners, actual-tag
API/capabilities and truthful generic controls in both Vue shells, in six slices.
Scope discussion and round2 written-plan review in the same C2C chat returned
PLAN READY; valid filesystem-conflict/I/O-discovery/header-parser findings closed.
Full Lazy recipe retention remains unchanged; no generic cache invalidation or
requery, FakeIP/AdGuard, appearance/media, aliases or runtime logging implementation.
Local task10-05-rust-native-switch-state-management stays planning, awaiting
subsequent explicit human implementation approval and a fresh dedicated code-review
binding.265 product inputs match accepted dirty c98dc source; HEAD/index unchanged.
No product code/build/test/deploy/push in this planning session.

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

See [final cumulative verdict](../rust/validation-records/10-04-rust-native-webui-runtime-capabilities/cumulative-review-result-final.md) and [S6 evidence](../rust/validation-records/10-04-rust-native-webui-runtime-capabilities/s6-status.md)
and [hosting contract](../rust/contracts/native-webui-hosting.md). Inherited accepted
source ac629018 and unrelated dirty changes are preserved; real HEAD/index stay
unchanged. Only demonstrated C11/C12 subitems are registered; full5D/Phase6,
scaffold retirement and production/default approval remain required.

Parent acceptance and archive completed after current-source checks and VM rerun391 native/26 frontend tests. [Acceptance record](../rust/validation-records/10-04-rust-native-webui-runtime-capabilities/owner-acceptance.md). Real source HEAD/index remain unchanged; no commit/push/deploy.

## Native special-groups task (2026-10-04)

The authorized S1–S7 stages and complete `79d93ae1...` baseline-to-final cumulative
review have explicit `FINAL: PASS` on tested source `ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc`.
Final isolated workspace 1,184/0/3, native-host 381/0/3, libraries 410/0/3,
fmt/strict Clippy and native build pass. Same-ID P1-1 repairs retain actual supplier
entry/peer/transport through cache hits, native save and process restart without
fabricating attempts or ECS echo; legacy wire-only imports remain unknown.
Three real restart hits add zero peer requests; the existing Go reader accepts
all three extended v2 dumps. Unchanged Vue/Go source retains S6/S7 build/browser
proof. All failures and earlier rejected audit objects remain public.

See the [final cumulative result](../rust/validation-records/10-02-rust-native-special-groups-upstream-management/cumulative-review-remediation-round-2-result.md),
[public plan](../rust/plans/special-groups-upstream-management.md) and
[contract](../rust/contracts/native-special-groups.md). Real branch HEAD/index and
original authorization remain unchanged. No push, deployment, production switch,
ordinary commit or archive. Task PASS is separate from the full migration gate.

## Native client/ECS task (2026-10-02)

Task `.trellis/tasks/archive/2026-10/10-02-rust-native-client-context-ecs/` owns trusted UDP/TCP
client context/client_ip, scoped ecs_handler/legacy ecs, supplier-only echo,
opt-in full ECS cache keys, conservative unsafe-placement rejection, and canonical
v2 Go/native dump interoperability. S1–S6 and cumulative whole-task C2C review PASS on source f9c523bb; S6 proves
real first-refresh/disconnect, DNS/API/Vue, shutdown/restart and workspace proof.
Final whole-task review and closure evidence are recorded in that task's research,
which takes precedence over this concise handover. Default/quick ECS cache bypass
remains; no scope-covering lookup or byte-identical noncanonical Go key promise.
All validation uses the isolated SSH mosdns-rust environment. No push/deployment
or production/default cutover is authorized by this task.
User requested conditional archive after verification. Parent verification found
no actionable issue, matched all129 source/manifests locally/remotely and reran
28 focused tests successfully; see research/owner-acceptance.md in the archive.

## Source of truth (precedence)

1. **Live execution** — `python3 ./.trellis/scripts/task.py current` and
   `python3 ./.trellis/scripts/task.py list`, plus the selected task's `prd.md`,
   `design.md`, and `implement.md`. An empty current pointer does not mean no
   task is in progress.
2. **Evidence** — archived task artifacts under `.trellis/tasks/archive/`.
3. **Architecture** — `docs/ai/rust-rewrite-plan.md`.
4. This file is a **concise handover only**. It summarizes current state and
   constraints; it does not restate slice narratives, test transcripts, or file
   inventories. Where it disagrees with the sources above, they win.

## Resume protocol

1. Read `AGENTS.md`, `docs/ai/project-context.md`, `docs/ai/config-notes.md`.
2. Read this file and `docs/ai/rust-rewrite-plan.md`.
3. Check state: `git branch --show-current`, `git status --short --branch`,
   `python3 ./.trellis/scripts/task.py list`.
4. Read all three artifacts of the task being resumed, if any.
5. Run the `trellis-before-dev` skill before changing implementation code.
6. **Preserve unrelated dirty files.** Keep Trellis auto-commit disabled, never
   use `git add -A`, and stage exact paths only.

## Branch and worktree

- Dedicated worktree `/Users/tom/github/mosdns-rust` on branch `rust`.
- Base commit `3896a4a7e0ce4311b40a7e4c80c93f2c8b3b4f1d`, base release `v0.7.1`.
- Do not switch to `main` merely from the folder name. `main` stays Go-only; the
  `rust` branch is a future pure Rust-native replacement, not an intermediate
  production runtime.

## Status as of 2026-09-24

The Rust-native host has reviewed, bounded W1 UDP/TCP forwarding, W2 simple
cache, and W3 domain/IP routing evidence; those tasks are archived. The first
Go/Rust native-process comparison task is also reviewed and archived. Its
[report](../rust/phase5a-native-comparison.md) supports only the stated
W1/W2/W3 cases: 12/21 scenario-stage groups formed three valid pairs, with
no objective overload point, no resolved service-recovery result, and no
multi-core capacity claim. None of these gates enables a production/default
cutover.

The [stage plan](../rust/next-stage-plan.md) created
`rust-phase5a-native-query-observability`, completed and archived on 2026-09-26. Its
[PRD](../rust/validation-records/09-24-rust-phase5a-native-query-observability/prd.md),
design and implementation plan record the authorized Slices0–3 and bounded
validation. Basic host-owned audit/metrics are implemented and Linux workspace
regression passes. M8 W1 TCP100QPS and M9 W2 warm100QPS screens pass; W2 cold
has correctness-only evidence. M9 wrong-fixture W3 remains invalid. The separately
authorized M10 nine-session W3 batch completed 27000 correct queries and 45000
ordered routing events; unchanged paired median gates pass. Original driver
FAIL (shared-clock route oracle and one exited-process cleanup race) is retained.
A separate unique-ID offline proof and complete process-exit receipts pass;
M10-FINAL-001 explicitly approved the repair and bounded A5/A6 acceptance.
See the task's research/m10-w3-assessment.md. User authorized lifecycle closure and GitHub push; production remains gated.
Inspect live Trellis task state before resuming, because archive moves and task pointers
may change independently of this concise handover.

Latest measurement task: [Phase 5A measurement reliability](../rust/validation-records/09-27-rust-phase5a-measurement-reliability/prd.md) is archived as **closed — incomplete matrix**, not full acceptance. W1 official-r3 supplied two valid Go/Rust pairs; Go r3 crossed only the frozen terminal health p95 band, so its paired Rust, W2 and independent profiling were not started. C2C accepted stop-and-report after the retry budget was exhausted. Capacity, objective overload/service recovery and hotspot conclusions remain unavailable; see the [closeout report](../rust/phase5a-measurement-reliability.md). The archive lifecycle field `completed` does not turn A4/A5 into PASS. Next frontier is bounded 5B configuration/sequence composition on the same native host; the measurement closeout itself did not authorize new implementation. No evidence currently justifies a runtime/Send refactor; production remains gated.

User authorized revising the existing [5B representative query-chain task](../rust/validation-records/09-27-rust-phase5b-config-sequence-composition/prd.md) on 2026-09-27 to avoid overdesign. Revision 2 targets one real configuration-derived routing/direct-child/cache/upstream chain, including top-level include, provider files/multiple rules, qtype/has_resp and common reject. Old W2 remains compatible, but its fixed placement is no longer the design boundary. Six slices/four gates and per-batch microsecond screens are superseded; planning v1 digests are historical only.

Steps 1–2 of that task were implemented inline on 2026-09-27 (uncommitted working tree on `11bd56c4`). `rust/sequence-core` gained a direct named call (`ExecutableSpec::Call`, its own child scope where natural completion/`accept`/`reject` return to the caller and `exit` propagates until a `try` catches it), stable per-scope identity, a `watch_enclosing_scope`/`MachineStep::ScopeComplete` boundary, and a `last_origin` that reports the real named executing sequence without letting a synthetic inline scope or a fixed entry tag stand in for it. `rust/native-host/src/config.rs` was rewritten as a collect-then-resolve composition compiler: ordered top-level `include` resolved against the declaring file's directory (nested include rejected), `domain_set` `exps`/`files`, `qname`/`qtype`/`has_resp`/`resp_ip`/`_true`/`_false` with `!` negation, direct `$sequence` → named child call, exec scalar/list, configurable positive cache `size`, load-time `reject` validation (`0..=15` accepted, `>15` unsupported rather than truncated), and bind-time rejection of duplicate tags, cross-type references and malformed rule files. The request driver now publishes a cache result only at the cache's own successor completion, so a parent's later rewrite cannot pollute it, an entry cache stores its own successor, and a second dynamic cache access fails closed instead of replacing the first token. The forward response policy and the later-leg deadline gate no longer branch on forward count.

Evidence for those steps passes locally, including over real listeners. `rust/native-host/tests/slice3_composition.rs` covers the loader (include/rule-path/negative/reorder) and runs the representative chain against live loopback peers over both a UDP listener and a TCP listener: blocked and qtype-65 queries answer locally with zero peer calls, a local-suffix query reaches the local peer on a cache miss, an unmatched query falls through the parent to the default peer, two repeats of the local name are child-cache hits that leave both peers at one request, and the TCP variant with audit disabled keeps identical wire and peer counts with no retained record. The audit-enabled run records the real executing sequence rather than a fixed entry tag. `rust/native-host/src/execution.rs` additionally covers the child cache surviving a parent overwrite, entry-cache wrapping, a repeated dynamic cache access failing closed, and cancellation leaving no cached result. The initial full workspace run reported 891 passed / 0 failed; after review fixes, the full workspace reported 896 passed / 0 failed. `cargo fmt --check` and `cargo clippy --all-targets -D warnings` also passed on the review candidate.

**Step 3's selected Linux E2E evidence and the latest user-selected C2C evidence review passed; the accepted task scope was archived with dedicated remote fault/cancel/close E2E explicitly deferred.** On 2026-09-28, `ssh mosdns-rust` was available. The review-fix commit `abeeb3e3bfb4458588430b83bfbd9280b359d37d` was built in an isolated remote Rust workspace; `slice2_config` passed 12/12 and `slice3_composition` passed 11/11, including UDP/audit-on and TCP/audit-off representative chains. C2C reviewed `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767` and returned `FINAL: PASS`; the reviewer confirmed the remotely tested Rust code/spec are byte-identical to reviewed HEAD (which adds documentation only). A later `mosdns-rust` full Rust workspace run passed 896/896 across 60 targets, including W1/W2/W3 and cancellation/close/shutdown regressions; a source-preserved file-backed `full:local.only.test` retest passed 12/12 across real UDP/audit-on and TCP/audit-off listeners. A dedicated remote fault/cancel/close E2E remains unrun/deferred; workspace regressions do not replace that separate scope. Codex `002reviewer` passed the prior A6 evidence range, but the user-selected C2C review of `bcac20374312d5bf875164f87673224b9da2a796..4fc737aa0dffc8e92ed878577b9c8a4131568077` returned `FINAL: FAIL` with P2-1 (remaining-scope summary omitted the dedicated remote fault gap) and P2-2 (temporary exact-rule source was not retained). The 5B artifacts and scope summary are corrected. The follow-up C2C range `4fc737aa0dffc8e92ed878577b9c8a4131568077..098b4c5e2bc3427f591456d6725a04a8cb8bcc23` closed P2-1/P2-2 but found P2-3: the exact rerun command list omitted the remote `mktemp`/`mkdir -p` setup. Those actual commands and the observed generated root were added from the local execution transcript. C2C then reviewed `098b4c5e2bc3427f591456d6725a04a8cb8bcc23..146849c042bfa90a5d61cc6fbe9712b78d562e94` and returned `FINAL: PASS`, closing P2-1/P2-2/P2-3 with no new finding. The user authorized archiving the accepted task scope on 2026-09-28; the dedicated remote fault/cancel/close E2E remains deferred. Earlier `dnsperf` figures include SSH-forwarding overhead and are not a performance gate or PASS. No production or deployment change was made.

Completed milestones (all archived; each archive holds its own evidence):

| Milestone | Outcome | Archive |
| --- | --- | --- |
| Phase 1 cache | `rust/cache-core`: Moka/`Bytes` cache, raw-wire TTL, versioned ABI, opt-in cgo bridge. Soak/Miri/host gates closed; hybrid bridge missed the 10% QPS gate, so Rust stays experimental. | `.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/` |
| Phase 2 matcher (+ expansion) | `rust/matcher-core`: pure Rust domain/IP matchers and compiled rule indexes, plus `sd_set`/`si_set`/valued `domain_mapper` opt-in adapters. | `.trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation/`, `.trellis/tasks/archive/2026-08/08-13-rust-matcher-phase2-expansion/`, `.trellis/tasks/archive/2026-08/08-17-rust-phase2-matcher-correctness-remediation/` |
| Phase 3A query/wire | `rust/dns-core`: DNS wire parsing, TTL/EDNS/ECS helpers, opt-in query ABI/Go adapter. | `.trellis/tasks/archive/2026-08/08-15-rust-phase3-query-execution-core/` |
| Phase 3B sequence | `rust/sequence-core`: owned execution state, validated program model, explicit continuation stack, fuel/cancellation. No Go adapter/ABI/selector. | `.trellis/tasks/archive/2026-08/08-17-rust-phase3b-sequence-execution-foundation/` |
| Phase 4 UDP/TCP | `rust/upstream-core`: numeric UDP, fresh plain TCP, and UDP TC→TCP policy with cancellation/deadline/close contracts. | `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/` |
| Phase 4 secure DoT/DoH | `rust/upstream-core` secure module: one-shot DoT and DoH over HTTP/1.1 and HTTP/2, explicit numeric dial separate from service identity, verified TLS by default. | `.trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/` |
| Phase 4 resolver/bootstrap | `rust/upstream-core` resolver module: single-family numeric-address resolution through a numeric bootstrap peer, TTL/cache/refresh, single-flight publication, close/drain. | `.trellis/tasks/archive/2026-09/09-17-rust-phase4-endpoint-resolution-foundation/` |
| Phase 4 dual-stack selection | Resolver extension: explicit `bootstrap_version=0` A+AAAA candidate collection, A-preferred selection, per-family TTL/state. No connection fallback or Happy Eyeballs. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-dual-stack-endpoint-selection/` |
| Phase 4 connection reuse | `rust/upstream-core` reuse owner: explicit reuse key over numeric dial + transport + secure identity/authority/ALPN, serial-per-connection minimum, bounded idle/pending limits, typed errors. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-connection-reuse-pipeline/` |
| Phase 4 QUIC/HTTP3/DoQ foundation | `rust/upstream-core` fresh one-shot DoQ and DoH3 clients with exact ALPN, identity separation, bounded response validation, lifecycle/commit semantics, and no fallback. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-quic-http3-doq-foundation/` |
| Phase 5A W1/W2/W3 host | Strict native YAML subset, W1 UDP/TCP forwarding, W2 simple cache, and W3 bounded routing, each with scoped Linux correctness gates. | `.trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-forwarding/`, `.trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-cache/`, `.trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-routing/` |
| Phase 5A first process comparison | Frozen paired W1/W2/W3 Go/Rust run with retained invalid attempts and explicit uncertainty. | `.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/` |

The workspace also holds `rust/runtime`, the transitional single `staticlib`
from the earlier hybrid work. New Phase 3B+ modules compose as plain Rust
libraries for the future host and must not add ABI symbols to extend the hybrid
shape or create a second runtime.

### Secure upstream status and caveats

Secure upstream Slices 0–4 were root-reviewed and closed (Slice4 `PASS / CLOSED`,
P0=0/P1=0), then archived at
`.trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/`.
Its exact commands, results, and limitations are in that archive's
`implement.md`.

Two evidence caveats carried from that review:

- **Rust 1.85 was never actually installed or run.** MSRV is evidenced only
  indirectly, via resolver-3 selection plus a `cargo metadata` audit showing no
  resolved package above 1.85, and the stable-toolchain Linux CI build.
- The task's `research/secure-upstream-evidence.md` is **evidence-only**, not
  normative; the reviewed code and `implement.md` are authoritative.

Scope of what exists: the DoT/DoH and QUIC/HTTP3 foundations are **bounded
one-shot, fresh-connection** transport work, while the Phase 4 connection reuse
and QUIC foundation tasks are separately archived. None of these foundations,
nor the W1 native host, is a production/default runtime.

## Product priorities and next frontier

The user confirmed Linux amd64 as the primary platform. Full backend feature
compatibility and correctness are prerequisites; query latency (especially
p95/p99), sustainable useful throughput, overload recovery, and long-running
stability are the primary improvements. Memory is secondary with no required
reduction percentage; bounded, reclaimable extra memory is acceptable when
measurements justify the performance benefit.

The strict W1 UDP/TCP and W2/W3 native host now accepts its sole listener's
`enable_audit` flag and exposes read-only basic metrics/terminal audit snapshots.
Linux functional tests cover bounded provenance, retention and lifecycle;
M10 W3 evidence and offline validation repair passed scoped final review. Full C08 audit/API parity remains Phase5C. Later
measurement/profiling must separately examine higher-load offered-load validity
and multi-core scaling; current100QPS screens do not establish capacity.
Full cache behavior, remaining
plugins/transports and representative production query combinations remain
5B/Phase 4 work; management and persistence remain 5C work.

Remaining Phase 4 foundations compose with **5B full query features**; **5C full
control plane** covers APIs, existing UI, persistent state and updates; **5D**
validates full-system performance and stability; **Phase 6** retires hybrid
scaffolding before production replacement. Current single-thread local-set
runtime and serial transport reuse are not evidence of final concurrency or
multi-core performance.

- Architecture, dependencies, and gates: `docs/ai/rust-rewrite-plan.md`.
- Feature ownership and native acceptance inventory:
  `docs/rust/feature-coverage.md`.
- Reproducible performance/stability workloads and threshold-freeze rules:
  `docs/rust/performance-validation.md`.

The basic-observability task completed and archived after explicit user
authorization following M10-FINAL-001 PASS. Preserve M9's invalid verdict and
M10's raw/derived evidence. No further traffic, deployment or next task is
authorized by this closure. Read the stage plan before choosing subsequent work.

## Non-negotiable constraints

- Preserve the MosDNS **product contract**: YAML/config and sequence/plugin
  semantics, final DNS/routing/audit behavior, API/WebUI workflows, metrics and
  persistent formats. Go internals and accidental quirks are not contract.
- The final target is a **pure Rust-native host**. Existing Phase 1/2/3A hybrid
  adapters, selectors, Go mirrors/fallback, and cgo scaffolding stay safe but
  frozen: do not extend that pattern into Phase 3B+.
- Do not make the incomplete `rust` branch a production or default release until
  the Rust-native E2E gate **and** the later hybrid-retirement gate pass.
- KixDNS `2da3a2d` is a selective GPL-3.0 design/source reference, not a subtree
  merge. `/Users/tom/github/mosdns-rust-cache` is a read-only prototype
  reference, not a drop-in.
- Validate locally, then on isolated `mos-test`, and promote to production only
  with explicit user approval.

## 2026-10-02 cache lifecycle continuation

The completed 10-01-rust-native-cache-lifecycle-management task implements its native non-ECS named/quick cache, owner refresh, v2 persistence, cache HTTP/metrics and existing Vue workflow in the reviewed task delivery. S1-S7 passed the same C2C review; iteration 11 returned full-task FINAL: PASS / DONE. This supersedes earlier single-cache/lazy=0 descriptions only for that frozen scope. SIGTERM/SIGINT now run supervisor drain/final save; all owner failures aggregate through the existing exit2 entrypoint. See task research evidence and .trellis/spec/backend/native-cache-lifecycle.md. This is not a production/default release or the full native cutover gate.

## 2026-10-02 response-policy/IP increment

The [response-policy task](../rust/validation-records/10-02-rust-native-response-policy-ip-rules/prd.md) has S1–S5 independently reviewed PASS and S6 real UDP/TCP/API/Vue/SIGTERM-restart proof plus Linux workspace 1,070 tests/fmt/clippy PASS; final cumulative review is FINAL: PASS (2c059b0e..13d60d3e, iteration 7); task archived. Native hosts/redirect/TTL and Answer-IP IPv4/IPv6/CIDR composition are implemented, with immutable inline/plain-text loaders. Binary/SRS/compression/provider sets and management reload remain deferred. Existing cache v2 retained dumps are not policy-versioned: quiesce query producers, Flush while cache owner/API remains open, then stop/drain/restart to avoid old-policy resurrection. No source branch switch, production cutover, push, or default release.
