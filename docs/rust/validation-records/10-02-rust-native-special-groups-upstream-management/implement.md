# Implementation plan (not started)

One task, seven slices; all share snapshot/transaction ownership. No subagents.
Final human plan approval required before task.py start, code edits or builds.
Use task.py lifecycle operations, no hand-edit of status/authorization records.
Before execution read AGENTS, docs/ai context/config/handover/rewrite, public contracts,
all task artifacts, trellis-before-dev and active workflow phase. Local tooling
is ignored, public planning/contract/evidence summaries must survive a fresh clone.

## Slice 1 (S1) — Managed schema, generator and compiled router

Ownership: native-host config plus focused managed-control-plane model/generator,
plugins/execution/router if necessary; sequence-core only narrowly required API.
Freeze serializable groups/overrides/rule catalog, stable generated namespace and
profile capability shape. Generate native YAML from complete snapshot and compile
exact staged artifacts, explicit opt-in/router hook and collision checks. Generalize
listener inventory while preserving single-listener helper compatibility. Include
native CNAME policy and group flow/domain_set provenance before cache dispatch.

First tests: empty opt-in sample, missing/unreachable hook rejection, no-opt-in existing fixture,
name/slot/port conflicts,
path resolution from another CWD, source precedence, custom-only and no-match,
unsupported enabled import vs preserved disabled data, aliapi wrapper distinction,
all-disabled refusal, duplicate generated include/tags. Wire-level compressed
CNAME+A/AAAA, pure CNAME/DNAME/other qtypes, negative answer, ECS+supplier invariants.
Mock file/compile/upstream boundaries, do not mirror serialization implementation.

## Slice 2 (S2) — Snapshot admission and supervisor-owned listeners

Ownership: assembly/udp/tcp/execution/api handle wiring and shared owner close.
Build snapshot-local IDs/owner mapping, per-datagram/frame capture, prepared socket
set, stable API/observer, multi-listener bind/drain and bounded retired generation.
Expose prepare/install/drain operations with infallible preallocated install.

Tests: UDP/TCP same port, occupied one of pair leaves neither candidate serving;
old in-flight completes with old audit/supplier, later frame on same TCP connection
uses new config; unchanged listener/cache owner continues after another removal;
API/DNS failure cancels whole host; every owner closes exactly once. Controlled
fixture pause points expose races; transport mocks alone are insufficient.

## Slice 3 (S3) — Durable transaction/recovery

Ownership: new focused persistence/transaction module and cache persistence gates.
Implement scoped undo/redo journal, OS writer lock, fsync ordering/digest checks,
short admission barrier, commit marker, rollback/recovery_required and shutdown.
Prepare graph/resources before any canonical replacement; integrate candidate
generated YAML, JSON, owned text files and cache dumps in one journal.

Tests first: injected write/rename/fsync failure at each stage and rollback failure;
SIGKILL/restart at prepared/replaced/marker/swap/retirement/cleanup boundaries;
concurrent external edits409, two-process writer refusal, corrupt/out-of-root journal
fail startup, disconnected management client after commit. Fake FileStore failures
plus real filesystem/subprocess proofs, no misleading blanket 'atomic save' assertion.

## Slice 4 (S4) — Cache dependency invalidation and retirement

Ownership: cache catalog/owner generation gate and compiled policy dependency map.
Invalidate route/upstream closure, allocate candidate empty owners, transactional
empty dump replacement, stop old periodic writer/final snapshot while preserving
unrelated owners. Distinguish slot incarnations and snapshot-local executable IDs.
Integrate flush/import/save lock ordering; preserve ECS placement validation.

Tests: cache_all → router and sibling disjoint cache; upstream/rule/main membership
edits then warm queries cannot return old policy; pending miss/lazy cannot publish
or old final dump overwrite after commit/restart; failed precommit keeps original
cache/generation/dump and permits; rename/port-only retains correct safe caches;
delete/recreate slot, quick-cache dependencies, canceled join and shutdown final dump.
Run existing cache/ECS/dump/refresh regressions in addition to new focused tests.

## Slice 5 (S5) — HTTP management and capability contracts

Ownership: api plus model/coordinator adapters. Implement exact methods/body/status
matrix from design; inventory/config/runtime reflect committed actual state.
Managed manual post and diversion catalog PUT join same transaction; capability
flags derive enabled runtime, not hardcoded future capabilities. Bound request
sizes with existing native limits, reject unknown mutation keys except documented
harmless UI defaults; retain stored disabled unsupported fields read-only.

Tests: CRUD/reset/restart, method/status/error schema, invalid tag/profile/options,
percent-decoded traversal, resource conflict and missing enabled files; POST200
proves effective generation and real DNS, not just saved JSON. Concurrency between
upstream save/rules/flush/import/host close must follow one serialization order.

## Slice 6 (S6) — Maintained Vue workflow

Ownership: webui-log/src capability client, RulesManager/UpstreamManager and focused
panels; no compatibility UI removal or broad CSS refactor. Native profile: file-only
validation, supported-protocol controls, truthful inventory/errors, one-transaction
rename, unavailable features visibly disabled. Go404 discovery preserves old flow;
network/server failure stays error. Build required maintained+compatibility bundles
through existing repository scripts, never frontend and Go build in parallel.

Validate actual browser create/edit/delete, group and local-file rules, upstream
save, enable/disable, custom port/custom-only and invalid save. Confirm Go form
behavior and audit/cache/detail pages. Screenshot and HTTP/request traces plus
DNS/audit/disk facts; no browser-only success claim without runtime verification.

## Slice 7 (S7) — Whole-chain proof and public handover

Ownership: integration tests/proof runner/docs/rust plan/contracts/validation summary.
One isolated SSH mosdns-rust directory, controlled UDP/TCP/DoT/DoH peers on private
high ports. No local builds, public DNS/53, deployment or production replacement.
Prove two groups same qname on custom ports return distinct answers, overlapping
main-entry rules choose lower slot, no-match default, custom-only exclusion;
cache hits and post-save changed answers, true group/source/sequence/supplier/
entry/transport/attempts in native HTTP and Vue; failures/recovery/close and unrelated
cache/provider preservation. Audit-off retains existing allocation/metrics contract.
Record exact source manifest+SHA, initial failures and repaired reruns honestly.

Required final remote checks: fmt, clippy all-targets -D warnings, native-host tests,
workspace libs and full workspace integration, native build, prescribed UI builds.
Any environment failure is preserved and blocks its gate until repaired; no inherited
or narrowed PASS presented as full workspace proof. No measurement/cutover claim.

## Review and handoff

Each slice: meaningful tests → exact-source dedicated C2C review → fix open findings
→ rerun affected validation → same finding IDs re-review until PASS. Final cumulative
BASE79d93ae1 full SHA through exact tested source HEAD, full code+tests+proof scope,
separate whole-task C2C PASS. Do not reuse archived task PASS or planning review as
code review. Dedicated reviewer target/transport must be verified before execution
according to workflow; this planning chat is not the implementation reviewer.

No push, deployment, unrelated cleanup or automatic archive. Update public contracts,
feature coverage/handover and plan summary with bounded evidence; preserve inherited
dirty docs. Stop with reviewable changed paths, tests/limitations and final status.
