# Rust migration next-step roadmap — design

## Purpose and boundary

This parent task turns the same-chat C2C plan into durable repository planning
artifacts and coordinates three downstream work packages. It changes planning
documents only. It does not implement product code, execute the existing
`mos-test` canary, change feature-coverage acceptance states, or relax the Rust
production/cutover gates.

The final target remains a pure Rust-native MosDNS binary/runtime. Existing Go,
cgo, selectors, mirrors, and fallback are migration scaffolding scheduled for
retirement only after the established Rust-native E2E and retirement gates.

## Evidence and current technical position

- The verified C2C planning conversation was opened from the saved
  `mosdns-rust` Project. `workspace_info` confirmed workspace `mosdns-rust`,
  branch `rust`, HEAD `9fd0bc0c`.
- `docs/rust/next-stage-plan.md` says native config currently covers one
  forward, one cache, sequence, `domain_set`, and a UDP/TCP listener; unsupported
  shapes fail at load. The representative 5B composition has already passed
  its bounded reviews, while full package/config compatibility, management,
  persistence, advanced upstream behavior, capacity/recovery, and cutover remain
  unproven.
- `docs/rust/feature-coverage.md` intentionally keeps broad rows pending even
  when some subitems have evidence. Preserve that distinction.
- `rust/sequence-core/src/state.rs` already has a `u64` `fast_flags` field and
  routing fields for `matched_group`, `final_sequence`, and `final_upstream`.
  Native host configuration/execution/observation does not yet expose the full
  `fast_mark`/`flow_setter` contract.
- Go `fast_mark` supplies sequence quick setup plus matcher OR / executable
  set semantics for IDs 0–63. The shared reservation at bit 48 and
  `switch17`/bit 49 must stay unchanged.
- Go `flow_setter` supplies quick keys `group`, `sequence`, and `upstream`, plus
  the corresponding routing fields. The native host currently derives some
  terminal metadata itself; precedence must be characterized and frozen before
  implementation.
- Native `domain_set` already supports load-time matching. The first 5C slice
  is only its `/plugins/{tag}/show`, `/save`, and `/post` control-plane path,
  persistent update, generation publication, and next-query effect.
- The separate active canary task targets fixed candidate
  `016103f3c21ed2d659694ce10e64aaf24b5c2767`. Its own scope and planning review
  remain untouched; no remote execution has begun.

## Work package map and ordering

| Order | Existing/new task | Deliverable | Start condition |
| --- | --- | --- | --- |
| 1 | Existing `09-28-rust-mos-test-native-sidecar-canary` | Bounded UDP/audit-on and TCP/audit-off operational/functional check of the fixed native candidate on `mos-test`; no performance or full compatibility claim | This roadmap receives same-chat C2C `FINAL: PASS`; then the user resolves all four execution inputs and explicitly chooses execute. |
| 2 | New child `09-28-rust-native-fast-mark-flow-setter` | Native YAML/sequence integration for `fast_mark` and `flow_setter`, with fresh-query flags, branching, routing metadata, and native observer evidence | Canary passes, or the user explicitly defers it and permits 5B to proceed without the remote canary result. The roadmap review must pass. |
| 3 | New child `09-28-rust-native-domain-set-management` | First bounded `domain_set` management loop: show/save/post, durable update, atomic new-generation visibility in subsequent DNS queries, restart and shutdown behavior | 5B task has a same-chat C2C `FINAL: PASS`; roadmap review must pass. This is the planned order, not an architectural assertion that the API requires `fast_mark`. |

Parent/child links are used only for the two new tasks. The existing canary
remains a separate active task and is referenced here without retargeting or
changing its metadata. Each child repeats its dependency and start gate because
Trellis parent links do not enforce execution order.

## Frozen compatibility boundaries

### `fast_mark` and `flow_setter`

- Preserve `fast_mark` IDs 0 through 63, matcher OR semantics, executable set
  semantics that do not clear unrelated bits, per-query isolation, and the
  existing bit reservations.
- Accept the documented quick-setup and normal configuration forms that the
  native sequence compiler owns. Unknown keys, malformed IDs, unresolved or
  cross-type references, and unsupported shapes fail during compile/load with
  useful location context; they do not silently become no-ops.
- Preserve `flow_setter` values through async sequence suspension/resume and
  expose the final native-observer fields required by the existing contract.
  Before code changes, compare Go externally observable behavior with native
  host-derived terminal values, choose precedence, and lock it in tests and the
  child design. Do not expand this into a full audit API migration.
- A real native YAML-to-listener integration test must prove the composed
  behavior; isolated parser fixtures alone are insufficient.

### `domain_set` management

- Keep the already-supported load-time `domain_set` query behavior.
- Match the external Go contract for the selected endpoints: `/show` returns
  the live rules as plain text, `/save` reports persistence success/failure,
  and `/post` accepts `{ "values": [...] }`, validates and compiles a candidate,
  persists before publication, then exposes the complete new rule generation
  to subsequent queries. Invalid input or a persistence failure leaves the
  currently published generation unchanged.
- Concurrent DNS queries observe one complete generation, never a partially
  updated rule list. A restart loads the successful persisted rules.
- Preserve the `/plugins/{tag}` namespace while using safe Rust-native file
  replacement and ownership. Do not reproduce unsafe Go file-write mechanics.
- This slice does not imply completion of all plugin APIs, configuration
  management, Prometheus, Vue workflows, all providers, or full 5C.

## TDD behavior slices and seams

Tests should drive each child from a failing externally observable behavior.
Keep production loaders, routers, sequence runners, and generation publication
real in the integration tests. Replace only external I/O boundaries where a
deterministic fault is needed.

| Child | Public surface under test | Observable behavior | Mock/fake boundary |
| --- | --- | --- | --- |
| 5B | Native YAML configuration compiler and sequence quick setup | Supported forms compile; malformed/unknown forms fail with path context | No parser/compiler mocks; controlled upstream only at network boundary |
| 5B | Real native listener and sequence execution state | Matcher OR and executable set alter a branch; next DNS query starts with fresh flags; unrelated flags survive | No sequence engine/state mocks; use controlled loopback peers for routing outcomes |
| 5B | `flow_setter` configuration plus native observer record | Group/sequence/upstream metadata survives asynchronous forward and appears with frozen precedence | No host metadata mocks; controlled peer only for actual upstream selection |
| 5C | Native HTTP router and plugin handler at `/plugins/{tag}` | GET show/save and POST post preserve status/body/route contracts | No HTTP router mocks; real router with temporary file-backed provider |
| 5C | Rule compiler + generation publisher + real DNS query path | Successful update changes next-query matching; malformed or failed write preserves old generation | Inject only persistence failure through the narrow persistence seam; use real matcher, publish path, HTTP stack, and DNS request path |
| 5C | Concurrent query/update and process lifecycle | Queries see whole old or new snapshot; restart sees persisted snapshot; shutdown frees API and DNS listeners | Real temp files and loopback listeners; barrier/scheduler coordination may be test-controlled |

## Explicitly deferred

Do not fold these into either child: `switch1..17`, `special_groups`, `aliapi`,
upstream groups/secure policy/bootstrap retries, extra cache plugins or cache
dump/persistence, CNAME/ECS/response rewrite families, complete config/package
compatibility and presets, full audit/Prometheus/Vue/system/update/service
workflows, remote upstream fault/cancel/close E2E, multi-core/`Send` redesign,
capacity/recovery/soak/stability acceptance, full 5D, Phase 6 hybrid-scaffolding
retirement, or production cutover.

The feature-coverage table remains the status source. Update only specific
subitems supported by completed evidence; never mark an entire broad row or
stage complete from these slices.

## Review and release boundary

1. Commit only the roadmap/task planning files in this task's exact review
   range. Keep auto-commit disabled, do not push, and do not stage unrelated
   paths.
2. Ask the same C2C conversation for `MODE: REVIEW_ONLY`, `STATE: REVIEW`,
   `CONTROLLER: TRELLIS` against the exact base/HEAD and path list. Do not paste
   source, diffs, or command logs into ChatGPT.
3. Address findings in a new narrow commit/range and repeat review until the
   same conversation returns explicit `FINAL: PASS`.
4. Only after plan PASS resolve the canary execution inputs with the user. The
   plan review alone does not authorize remote operations.
5. Each downstream task ends with its own complete same-chat C2C review of the
   committed task range. A PASS is limited to that task's acceptance criteria;
   it does not close deferred rows or authorize production cutover.
