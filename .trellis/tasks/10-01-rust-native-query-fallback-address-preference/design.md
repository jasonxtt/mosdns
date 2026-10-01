# Design — owned query branches

Final planning at 601b65a4; R1 threshold and R5 schema-2 decisions and the
complete R1–R6/A1–A8 scope were approved on 2026-10-01. PRD owns requirements
and acceptance.

## Interpreter seam

ExternalDispatch stays ID-only. Host immutable descriptors map IDs to fallback
resolved targets and preference callsites. Lower supported executable targets
to dedicated child roots using the same interpreter/driver, not duplicate logic.
Sequence-core needs a typed successor capture/replay API at preference dispatch:
capture remaining execution to its enclosing chain boundary, excluding caller
parent continuation. Replay on owned state copies, return typed state/completion,
and consume parent successor once. Preserve source origins, cache watches and
inline/named scope call/jump/goto/try/return/accept/reject/exit. A preexisting cache watch stays
with the parent: after selected child state is committed, the parent reports
its watched boundary once, before running its outer continuation. Children
do not inherit that parent watch; cache acquired in a child owns a child watch. Network/Tokio stay outside core;
existing synchronous callers keep current APIs.

## Branch driver and causal state

Factor one native machine driver returning BranchResult; root alone admits and
terminalizes clients. Owned child futures on LocalSet avoid mandatory spawn,
static lifetimes/Send/new runtime. Box recursive async dispatch if necessary.
Children share one root fuel/deadline and hierarchical cancellation, not cloned
fresh DEFAULT_FUEL or one shared loser-cancel bit. Use the explicit shared-budget
constructor/fork API in research/branch-contracts.md; legacy control cloning
stays safe for standalone users but is forbidden for branch creation.
Two siblings per wrapper is a local bound, not a global traffic limit.
Branch control selection is distinct from factual network supplier; the
inherited-response decision matrix in research/branch-contracts.md is normative.
BranchResult owns state, final response/source/routing snapshot, selected slot
and completion/failure. Attempt slots exist only in a root shared checkpoint
collector, registered before polls/I/O with branch ID/QTYPE, invocation ID,
original EntryId and global ordinal. Copyable branch descriptor has parent/role.
Winner commit is atomic; sibling attempts remain independently accounted.
Audit capture alone materializes display strings; metrics borrow static labels.
Guards cancel on drop, sealing/late writes are idempotent; normal return joins
children and async parent close owns abnormal-drop drain. Reuse existing stable
pooled/fresh secure transports; do not create new lifetime owners per branch.

## Policies and cache

Fallback distinguishes secondary execution from eligibility. Absolute threshold
starts at driver entry before first child poll, not first I/O, and is capped
by root deadline. Root cancel/deadline is checked before buffered winner commit. Primary-first poll breaks
simultaneously-ready ties; controlled peer barriers prove actual ordering.
Commit stays immutable during cleanup. Valid REFUSED/SERVFAIL remains response.
Preference copies raw query and rewrites only validated QTYPE via dns-core,
then reparses question/correlation. Never change typed QuestionInfo while sending
unchanged wire. Client ID/question remain immutable; probe bypasses its wrapper.
Original completion starts capped 500ms wait. Commit suppression or original
result, cancel/join reference. Positive bounded cache uses canonical names and monotonic expiry/injected
clock. Internal storage selection may reuse cache-core or a small bounded
container; it must preserve R3 constants and adds no new product behavior.
One cache adapter is shared but access/store tokens are driver-local. Capture
metadata, never pending tokens. Outer token stays outer. Child successor may
commit a valid result even if it loses parent selection; partial children drop
store. A/AAAA keys differ, fallback siblings may race same key under current
store-admission semantics. Test placement before/inside wrappers thoroughly.

## User-approved public diagnostics — schema 2

Root execution/checkpoint owns the policy-executed bit and builds one typed
terminal diagnostic object retained by observer AuditRecord; all common HTTP
rich-log routes use the existing single projection. UI never infers branches.
Policy-executing rich records use optional upstream_diagnostics schema 2;
ordinary forwarding stays schema 1. Preserve selected/attempt fields/envelopes.
Add branches [{id,parent_id?,role,qtype,decision?}] and per-attempt branch_id/qtype;
selected adds branch_id only for final network response. IDs ordered query-local
integers, QTYPE numeric; roles root/primary/secondary/original/reference.
Decisions selected/suppressed/completed/skipped/failed/canceled/interrupted where meaningful.
A valid nonwinner remains completed, not failed or canceled.
Never-started branch may show skipped but must not invent entry attempts.
Parent ancestry disambiguates nesting; the omission matrix is in
research/branch-contracts.md. No trace buffer, read job or version negotiation.
Shared Vue detail renderer reads schemas 1/2 and explains local suppression;
unknown schema shows unsupported diagnostics while ordinary facts remain.
Probe metrics create no extra client counters/ranking admissions/audit rows.
Do not hide actual probe attempts or claim a probe supplied the final answer.

## Risks

Successor/cache boundaries and root fuel/drop ledger are main risks. Existing
transport identity/client budget stay stable. Final reviewer must inspect whole
committed delivery, not merely last fix. Preflight VM disk/inodes; retain failures
and clean only task-owned artifacts. No rollout/production in this task.
