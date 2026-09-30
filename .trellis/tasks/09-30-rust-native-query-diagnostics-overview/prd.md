# Rust-native query diagnostics and overview workflow

## Goal and value

In one substantial PRD, connect real DNS terminal facts to searchable query
logs, rankings, query details and ranking drill-down in the existing maintained
Vue UI. An operator must be able to answer what response was produced, which
rule actually affected the route and which upstream actually supplied it.
The six related deliverables share one terminal record/read projection and
finish with one integrated acceptance and final implementation review.

## Background and authorization

- Workspace `/Users/tom/github/mosdns-rust`, branch `rust`, research baseline
  `9f6dfdb2c718e65d6969c1485c28df695090ba83`.
- Archived local-rule management and audit-control/DNS-card tasks are already
  delivered. Live coverage and their completion evidence override the stale
  parent/next-stage narrative. Whole C08/C11 and full 5B/5C remain incomplete.
- Existing audit retains terminal records; HTTP currently projects only time,
  name, type, client IP and duration. Vue QueryManager and Overview need the
  additional fields and routes documented in `research/contracts.md`.
- User approved planning on 2026-09-30, requested a new project C2C discussion
  and an execution prompt for another conversation. This turn must stop after
  planning. No product-code edits, `task.py start`, builds/tests, deployment,
  commits or push are part of this planning package. C2C advice does not
  authorize implementation or settle a product deviation for the user.

## Requirements

### R1 — Truthful final query diagnostics

Keep existing five fields and add query class, a real native request identifier,
response code/AA/TC/RA, ordered Answer records (type/TTL/data) and explicit
answer_details_status, domain-set and matched-rule source where established, effective routing label, matched group,
final sequence, final upstream, configured targets and selected upstream.
Only final wire after cache aging/local synthesis/parent response replacement
may supply response details. Intermediate answers and successful earlier
attempts must never become the final answer or selected upstream. Preserve
existing configured flow_setter versus host-derived metadata precedence.
Cache hit and locally formed response must not invent a network attempt.
No-response, upstream error, deadline, cancellation and send failure remain
distinct in native facts; do not infer client delivery from a formed response.
Unknown metadata is absent, not a fabricated label. Categorical rule/effective
ranks omit unavailable provenance while genuinely unmatched routes count as
unmatched_rule; log totals still count both. A request ID is allocated
at real admission, unique within a host process and useful for log search;
it is not a claim that capture tracing has been implemented.

### R2 — Search/filter/page API

Extend `GET /api/v2/audit/logs` with the existing Vue/Go filter names `q`,
`exact`, `domain`, repeated `client_ip`, `answer_ip`, `cname`, `domain_set`,
`effective_tag`. Match before counting/paging; logs are newest first.
Default page=1/limit=50 and existing maximum limit=500 stay intact. Multiple
client IP values form OR; different filter families and q form AND.
Support Go-visible fuzzy/exact and field-specific semantics in the contract.
IPv4-mapped/IPv6/host-port client input normalization must be explicit.
Unsupported parameter names fail 400 rather than silently disappear.
Out-of-range pages return empty arrays and truthful filtered totals.

### R3 — Rankings and slow-query diagnostics

Serve domain/client/domain_set/effective rankings as `[{key,count}]`, plus
slowest queries using the same rich log schema. Default rank limit=20,
slowest=100; Vue's limit=200 client rank must work. Limit handling is bounded
and defined in the contract. Domain/client/rule counts derive from retained
capture; slowest preserves
the separately retained Go top-300 history until clear/capacity change. Stop,
start and normal ring eviction follow the explicit view policy in the contract.
Within one response use one coherent store snapshot; separate endpoints or
separate pages may observe different generations. Tie order is deterministic.
All four rank memberships and native drill-down memberships use the same
field-specific predicate. Domain uses the additive logs/domain exact route;
404-only legacy Go fallback preserves existing behavior and its known limit.
Effective rank and its drill-down must use the same computed label, never
present all intermediate matcher tags as effective routing decisions.

### R4 — Maintained QueryManager closure

Use the existing Vue query list, search, exact search, load-more, quick filters
and detail modal against real native APIs. Show real response flags, answers,
TTL, rule provenance and final upstream data. Empty results, stale pages and
API failure must remain understandable and recoverable. Trace-copy/filter
operates on the real native ID. Existing capture and alias-management APIs
remain out of scope and must fail visibly if used; do not fabricate success
or advertise the entire QueryManager as supported.

### R5 — Overview rankings and drill-down closure

Existing domain/client/effective-rule rankings, slow-query rows, details and
rank-click log filters consume real native data. Preserve DNS card/System
controls and local-rule editing already delivered. Upstream metrics/config,
client aliases and switch17 controls are still unsupported; report their
scope honestly. Do not synthesize an empty upstream catalog or metrics to
claim the full Overview page works. Rank/slowest failures have per-panel
errors and cannot erase other supported
card/rank data; unsupported sections display unavailable state. Preserve
Go-backed behavior.

### R6 — Resource bounds and integrated regression

Preserve single host supervision, terminal capture gating, capacity settings
and persistence, metrics independence, HTTP/DNS shutdown/rebind. No DNS wire
mutation for audit and no raw packet retained redundantly in the ring.
Variable answer payloads, concurrent filtered reads and ranking work must not
cause unbounded memory/job growth or starve DNS. No full-ring deep clone to
return a small page and no encoding/sorting/filtering while holding the
observer mutex. At/near 400000 retained records, verify DNS progress under
repeated real filtered/rank HTTP reads. This is a correctness/resource screen,
not capacity, p95/p99 or 5D acceptance. All builds and product tests run only
through SSH alias `mosdns-rust`, using isolated directories/loopback listeners;
local browser accesses disposable VM Vite via a task-owned SSH tunnel.

## Acceptance criteria

- A1/R1: UDP/TCP real queries cover positive A/AAAA/CNAME/multi-answer and
  uncommon raw/decode-error cases,
  local reject, cache miss/hit and TTL aging, child/parent final replacement,
  repeated matches, negation/later-rule failure, default unmatched branch,
  timeout/cancel/send failure. Wire/peer oracle and audit fields agree;
  overwritten and forbidden legs never leak into final response fields.
- A2/R2: real HTTP tests prove each filter, combinations, repeated IP OR,
  case and exact behavior, mapped IPv6, answer/CNAME semantics, pagination,
  unknown parameters, form/Unicode decoding and malformed encodings, duplicates,
  max limits and empty/out-of-range pages.
- A3/R3: all five rank endpoints have exact membership/count/order proofs,
  label/drill-down agreement and stop/clear/resize/eviction proofs. Preserve
  the approved slowest retention policy and ties after planner discussion.
- A4/R4-R5: isolated maintained Vue browser proof drives actual search,
  exact/quick filters, load-more, details, four rankings plus slow-query rows
  and drill-down. DNS traffic and APIs are real. Existing DNS-card, audit
  control and local-rule workflows still work; unrelated unsupported areas
  remain explicitly recorded.
- A5/R6: full/near-full ring concurrent DNS/HTTP resource/progress screen,
  audit-disabled and capacity-zero cases, close/cancel/rebind regression,
  focused/full Rust, fmt, clippy, disposable Vue build and process E2E pass
  on the specified VM; preserve failures, candidate/config/commands and cleanup.
- A6: update only proven C08/C11 and necessary provenance subitems; one exact
  whole-task final independent review passes. No whole-stage completion or
  production claim. Final plan and proposed decisions are approved in a
  subsequent execution conversation before task activation.

## Out of scope

Capture/diagnostic trace stream, aliases CRUD, v1 full-log migration,
upstream config/groups/Prometheus, special_groups mutation, switch family,
new query plugins or protocols, cache dump/lazy/ECS, static UI serving or /log,
full preset/config-package compatibility, runtime/Send redesign, formal
performance/capacity/recovery/soak, Phase 6 and production cutover. Do not
read or mutate live `/cus/mosdns`. Preserve unrelated dirty files and leave
Trellis auto-commit disabled.

## Proposed decisions for subsequent user approval

The execution prompt explicitly approves the proposed contracts in
`research/contracts.md`: bounded rank limits and read overload, deterministic
ties, native admission ID format, exact-domain route with legacy fallback,
strict encoding errors, missing-provenance rank omission, actual provider/
inline provenance precision, final-answer diagnostics and uncommon raw-RDATA
presentation. Slowest top-300 history and existing control/settings/search
semantics remain preserved. Sending that prompt in a later conversation is
fresh approval of this final plan; this planning turn is not implementation
approval. Any subsequent product change requires a specific new decision.

## Execution resource decision — 2026-09-30

The user explicitly approved retaining the existing 400000 query-diagnostics
record limit and complete answers. No byte-based eviction, answer truncation,
or capacity reduction is allowed. If detailed retention cannot allocate, DNS
terminalization and statistics continue while the detailed record is omitted;
if a diagnostic read cannot allocate or encode, that diagnostic page returns
HTTP 500 without affecting DNS service behavior or statistics.
