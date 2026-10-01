# Source-backed branch contracts (planning)

Baseline 601b65a4, inspected 2026-10-01. No runtime tests/builds ran in planning.

## Inherited response and scope watch

pkg/query_context/context.go:251–278 deep-copies query, decoded/raw response,
marks and fast flags. fallback.go:116–132 copies entry context and accepts a
successful branch with any existing response. Freeze that product result:
branch no-op success may select inherited response; preserve its causal
supplier/generation. Do not require every fallback branch to create a fresh
response or call an upstream. Both unsuccessful branches retain the typed
failure rather than relabeling inherited wire as a fresh success.

sequence-core engine.rs:331–345 allows one watch, keyed to current enclosing
scope. At lines 574–587 the machine reports the boundary before outer continuation;
exit propagation is preserved in pending_completion. Freeze capture/replay:
a parent pending cache token/watch remains parent-owned, children inherit no
parent store/watch. Once policy commits its selected state, consuming successor
must report the parent watch once at its original boundary. A child cache access
may independently arm/watch its own successor. No extra watch conflict and no
outer continuation runs before parent publication. Root fuel sharing needs a
new narrow primitive because ExecutionControl currently owns remaining_fuel;
cloning it would multiply the allowance. Engine APIs must preserve old sync users.

Preference probe rewrites raw QTYPE and typed question together, validates
responses against that new question and discards an inherited response whose
question is incompatible; never qualify it against the original question.
Original child preserves original inherited state. Test this case explicitly.

## Schema 2 exact field/omission policy

User approved branch/QTYPE diagnostics and detail display on 2026-10-01.
Existing envelopes/routes and schema-1 reader behavior remain. Schema 2 is
emitted for records where a fallback/preference descriptor actually executes;
merely configuring unused policy does not upgrade a plain-forward record.
All common v2 projections (logs, logs/domain, rank/slowest) share one projection.

- branches: ordered query-local descriptors, root id 0 plus parent_id for every
  child; role root/primary/secondary/original/reference, numeric qtype and terminal
  decision selected/suppressed/completed/skipped/failed/canceled/interrupted.
  No public pending terminal state; completed means valid nonselected branch,
  skipped means declared branch never executed. Do not rewrite a response
  attempt to canceled when a valid nonwinner completes.
- Parent ID establishes nesting. Add policy fallback/prefer_ipv4/prefer_ipv6 on
  child descriptors when they are created by that policy; root has no policy.
  Every actually started entry attempt has branch_id and qtype, keeping existing
  ordinal/entry/peer/transport/outcome fields. Skipped branch gets no fake attempt.
- selected has branch_id only when a network response causally supplies final
  wire; suppression/cache/local/no-response omit selected entirely. Inherited
  network response keeps its original supplier slot/branch, not selecting child ID.
- Peer/transport retain existing omission rules before target I/O. Bootstrap
  traffic is not an entry attempt or supplier. All branch fields/attempts share
  root audit capture; no independent row/ranking admission for probe work.
- Unknown versions: explicit unsupported diagnostics state, ordinary answers and
  route remain usable. Schema 1 keeps its previous presentation. Empty attempts
  are valid when cached preferred evidence suppresses a query without I/O.

## User-approved threshold omission (2026-10-01)

fallback.go Args.Threshold is int with no initializer. Init computes the
millisecond duration and substitutes 500 only for negative values. Thus
omitted and explicit 0 currently act immediately, despite the documented
500ms default comment. User approved native semantics: omitted 500, explicit 0
immediate; negative 500 preserves existing compatibility. Retain explicit
presence during parsing. This is an intentional compatibility choice; do not
claim Go omission parity or reinterpret it during implementation.


## Inherited-response decision matrix (C2C feedback, source-checked)

Control winner and factual final network supplier are separate concepts.
A branch decision=selected names control selection; diagnostics.selected names
only the network supplier of final wire and may be absent for that same branch.
A noop returning inherited network wire preserves the ancestor supplier slot.

| Branch completion | Fallback eligibility | Final result if chosen | Factual supplier |
| --- | --- | --- | --- |
| success, valid inherited response, no replacement | eligible | inherited wire; child is control winner | original causal slot, or omitted for inherited local/cache |
| success, new valid network response | eligible | child wire/state | actual child's supplying slot |
| success, new local/synthetic/cache response | eligible | child's local/cache wire/state | omitted |
| success, no response | unsuccessful | wait/use eligible sibling; both unsuccessful => typed failure | no new supplier |
| failure even with inherited response | ineligible | eligible sibling wins; no promotion of failed branch | no new supplier |
| both branches unsuccessful with inherited response present | no successful winner | propagate typed fallback failure through existing caller/try semantics | do not claim inherited answer is new fallback success |
| root cancellation/deadline before result commit | no buffered winner | existing cancellation/deadline terminal policy | no newly committed supplier |

Fallback typed failure propagation is distinct from final caller behavior: an
outer try/sequence can retain prior response according to existing engine rules.
Do not force final SERVFAIL if existing caller semantics retain valid prior wire;
do not pretend failed fallback selected that response either. Tests assert both
plugin result and final parent state/provenance. Preference suppression produces
new local empty NOERROR (original client question), not original/preexisting wire.
A completed reference remains completed, never a final network selected supplier.

## Root budget and cancellation fork API

engine.rs:60–65 derives Clone on ExecutionControl holding a plain remaining_fuel;
consume_dispatch at 953–960 decrements that local value. Native branching MUST
NOT derive child controls by ExecutionControl::clone. Preserve standalone legacy
control construction/clone API for old synchronous callers; introduce a narrow
shared-mode constructor and explicit fork_child API for native branching.

Proposed typed seams: RootFuelHandle (shared remaining counter),
ExecutionControl::with_shared_budget(root, cancellation), and
ExecutionControl::fork_child(child_cancellation). Both factories share the same
counter handle; cloning the handle never copies/reset its numeric allowance.
Native root and every replay/named-target child use shared mode. Legacy writable
remaining_fuel remains authoritative only in standalone mode; native shared-mode
code reads a remaining_budget accessor and never mutates the legacy field to
replenish execution. No snapshot/merge-on-join budget accounting.

RootFuelHandle uses Rc<Cell<u64>> on current-thread runtime: every matcher/exec
consumes immediately through the canonical consume_dispatch, with no await/borrow
inside debit. No atomic or Send refactor needed. Tests assert root total debits
and eventual bounded termination, not arbitrary per-sibling counts under races;
controlled polling/barriers establish deterministic cases.

Root cancellation must reach all descendants; loser cancellation reaches only
that child and its descendants, not siblings/root. Plain cloning the same
mutable cancel bit is NOT child cancellation. Use explicit child-token ancestry
in core or synchronize existing transport child cancellation into the child's
core token before each driver step; the driver also checks root cancellation.
Select one of these equivalent private mechanisms without changing that frozen
observable contract. Tests cancel primary alone while secondary progresses,
then cancel root and prove every descendant stops. Root deadline never renews.

## Canonical diagnostic ownership and keys

ExecutionFacts/branch driver assembles one typed terminal UpstreamDiagnostics.
Its root policy-executed bit is the sole schema selector: no policy executed =>1,
any policy executed =>2. ExecutionCheckpoint snapshot on abnormal drop carries
that same bit/collector. Observer admission/finish retains the typed object in
one AuditRecord; slowest shares its Arc (observer.rs:819–820,1137–1138). Existing
api.rs:1129 project_upstream_diagnostics is the only JSON mapping used by common
rich-log projections. Endpoints/UI do not infer version, branch role or winner.
Logs, logs/domain and rank/slowest must agree on all diagnostic fields for the
same native query ID. No new persistent log format/schema service is introduced.

Preference evidence storage is separate from NativeCacheAdapter's DNS wire
store. It is owned per compiled preference callsite with a fixed preferred
family, so its logical namespace is (callsite ID, preferred QTYPE, canonical name).
An instance map may physically key name alone because ID/family are fixed by
its owner. Do not add a duplicate family field or share evidence across callsites
without need. DNS response key stays its existing wire-derived qname/qtype/qclass
contract (native-host/cache.rs key_for_query); do not put evidence in that store.

Fallback timing: sample monotonic invocation_start when native driver enters
the fallback descriptor, before polling primary/secondary child. eligibility_at
is checked invocation_start + normalized threshold, capped by root deadline;
overflow is a config error. Root deadline/cancellation check precedes eligible
winner commit. First network send, resolver completion and child scheduling do
not restart timer. Tests use injected clock or paused Tokio time/barriers;
never rely on wall-clock sleeps as an ordering oracle.
