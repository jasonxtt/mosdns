# Rust-native special groups and upstream binding workflow

## Current closure status — 2026-10-04

S1–S7 and cumulative exact-source C2C review PASS on ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc.
Parent checked all 527 local/remote/reviewed source inputs and 115 remediation
artifacts without mismatches; independently reran 37 focused integration tests.
User authorized conditional archival after inspection. Original planning baseline
and authorization-boundary text below remain historical, superseded by the genuine
implementation authorization and final cumulative records. Source stays uncommitted
on real rust HEAD79d93ae1, preserving the original no-ordinary-commit/no-push boundary.
No migration/cutover/deployment authorization is implied. See owner-acceptance.md
in the public validation record for closure evidence.


## Goal and authorization

One cohesive delivery connects the existing special_groups/upstream UI to native
configuration, persisted state, real DNS routing and final audit facts. User has
authorized task creation and planning only. No product code, task start, builds,
implementation authorization snapshot or dedicated code-reviewer binding has
been initiated.
Baseline rust branch 79d93ae1; public contracts live in docs/rust/contracts.

## Confirmed baseline

- Native GET /api/v1/special-groups returns actual empty state, mutation405.
- Native config currently permits exactly one UDP or TCP listener. Group custom
  ports require an owned multi-listener lifecycle, not an API-only addition.
- Existing UI group schema: slot/name/listen_port/custom_port_only; slot starts50,
  port53 reserved, main routing disabled only when custom-only with a nonzero port.
  Group view exposes stable derived upstream/diversion/manual tags and paths.
- Existing Go generated YAML uses sd_set_light, domain_set_light, aliapi,
  domain_mapper, mark, switch4, cname_remover and per-group cache/sequence. Most
  of those plugin types are absent in native compilation. Existing template
  cannot be written and claimed runnable without resolving those dependencies.
- Upstream API is /api/v1/upstream/{tags,config,runtime/{tag}}, with POST config
  body plugin_tag/upstreams; DNS protocols and signed AliDNS are distinct modes.
- Current native forward supports UDP/TCP/TLS/HTTPS, bootstrap, multi-entry and
  bounded reuse. HTTP3/DoQ/proxy/socket policy are not complete native host modes.

## Frozen delivery profile

- R1 Preserve special group CRUD payload/view, stable slot/path identity,
  normalization/conflict errors and deterministic slot-based routing order.
- R2 Integrate group upstream inventory/config/runtime endpoints and existing
  Vue group selection; runtime facts must reflect actual published config.
- R3 Give each group real owned rule/upstream/cache/sequence state, generated
  native-compatible config and honest final group/sequence/supplier audit facts.
  No synthetic tag may replace actual supplier entry/transport.
- R4 Include manual/local plain-text domain rules, independent group caches and
  supported standard DNS upstream configuration in one working chain. Advanced
  rule formats/downloaders and signed AliDNS are deferred, never silently
  downgraded. Unsupported data must preserve original persistent state on refusal.
- R5 Include primary-entry routing and custom UDP/TCP group ports, including
  custom_port_only semantics; own all listeners under host shutdown and drain.
- R6 Freeze persistence/apply ordering, failure matrix, generation publication,
  rollback/restart recovery and shared resource retirement before implementation.
  Never return successful save while presenting stale runtime as applied.
- R7 Prevent cache reuse across changed upstream/rule policies and pending old
  refresh/miss publication. Define cache identity/dump retirement on edit/delete
  without destroying unrelated caches or files. Existing durable flush contract
  and ECS placement gate remain mandatory.
- R8 Preserve Go UI behavior for supported profiles; unavailable fields/protocols
  must be visibly unavailable and refused server-side. Do not alter Go behavior.

## Acceptance

- A1 Existing Vue creates/renames/deletes groups, edits their standard upstreams
  and local rules through real native HTTP and persistent state.
- A2 Same domain across two groups yields different controlled answers; overlapping
  rules honor frozen order; unmatched requests reach the default path; audit
  retains effective group and real supplier. No public DNS/port53 in proofs.
- A3 Independent UDP/TCP custom ports select the correct group, custom-only does
  not affect main routing, occupied-port/bind failure leaves old state operational.
- A4 Valid save/apply/restart agrees on runtime config, generated YAML, JSON and
  final DNS; invalid input/write/compile/publication failures have no partial
  externally accepted generation. Interrupted transactions recover explicitly.
- A5 Group edits/deletion cannot revive old cached answers/refreshes, close work
  belonging to an unrelated group, or leak owned sockets/tasks.
- A6 Unknown/unsupported fields and protocols produce explicit safe failures;
  existing query/cache/ECS/API/UI regressions pass. Real browser, DNS, HTTP,
  disk, process restart and exact-source reviewer evidence required.

## Scope selection and planning gate

User selected the recommended profile: standard UDP/TCP/DoT/DoH, local plain-text
rules and complete main/custom-port group lifecycle. Defer signed AliDNS,
SRS/geodata/download/auto-update, H3/DoQ, proxy/socket policy, positive upstream
idle_timeout and pipeline=true. This is a scope approval, not task start.

Design freezes explicit native management opt-in and router placement, transactional
hot publication rather than delayed self-restart, capability schema1, bounded local
rule APIs, cache invalidation through upstream/routing dependency closure and
crash recovery. These intentional native differences are included in final plan
approval. Existing unmanaged config keeps its behavior. No production cutover.

C2C planning review: PLAN READY, no actionable blockers. Final user approval
must precede implementation/start; no code-review PASS is claimed.
