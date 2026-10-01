# Rust-native query fallback and address preference

Planning approved for implementation by the user on 2026-10-01; the complete
bundled scope is frozen here. Baseline 601b65a45e10b8b534fda5c5052ff310815dec66,
branch rust. Requirements are owned here; design/implement own technical detail.

## Goal

Deliver fallback and prefer_ipv4/prefer_ipv6 together through shared, isolated
query branches and causal result commit. Compose existing routing/cache and
UDP/TCP/DoT/DoH with truthful final-answer observation and actual DNS/HTTP proof.

## Confirmed facts

- native-host/src/config.rs:555 accepts neither fallback nor preference.
  execution.rs:430 drives one machine/cache successor/live exchange checkpoint.
  sequence-core/src/engine.rs:289 has no successor-fork API.
- plugin/executable/sequence/fallback/fallback.go:51 defines primary, secondary,
  threshold and always_standby. First eligible successful branch with a response
  wins; forward IP-answer priority does not apply. Lines 84–86 use 0ms for omitted
  threshold while the option comment says 500ms; R1 records the approved native
  deviation.
- plugin/executable/dual_selector/dual_selector.go:77 wraps successor execution,
  changes probe QTYPE and suppresses nonpreferred responses on positive preferred
  evidence. Lines 38–44 fix reference wait 500ms, cache 65536 entries and 1 hour.
- Reviewed forwarding is archived at ../archive/2026-10/09-30-rust-native-upstream-forwarding/.
  Its schema 1 has no branch-role/QTYPE fields, important for internal probes.

## Requirements

### R1 — Config and control flow

Named fallback accepts primary/secondary supported executable references,
integer millisecond threshold and boolean always_standby. Quick prefer_ipv4
and prefer_ipv6 take no arguments. Resolve sequence/forward/cache/fast_mark/
flow_setter/fallback targets, rejecting missing/non-executable references and
wrong types/options with source locations before I/O. Preserve include and
call/jump/goto/try/return/accept/reject/exit. Nested/recursive work consumes one
shared finite fuel budget, never fresh fuel per fork.
User-approved on 2026-10-01: omitted threshold 500ms, explicit 0 immediate.
Negative threshold retains the existing 500ms compatibility default; duration
conversion/absolute-time arithmetic overflow fails load. Preserve
omission versus supplied zero in parsing. This is an intentional native
deviation from Go omission=0, not accidental parity or a bootstrap setting.

### R2 — Primary/secondary selection

Primary starts immediately on isolated entry state. Without standby, secondary
starts only at threshold or primary failure/no response; fast-primary success
starts no secondary. With standby, secondary starts immediately but buffers
success until threshold or primary failure/no response. Primary success before
release wins; afterwards first eligible successful response wins. Valid DNS
RCODE including REFUSED/SERVFAIL is not a transport failure; malformed or
wrong-correlation response is. Exit carrying response normalizes as success.
Both branches unsuccessful yields typed failure. Commit winner response, marks,
flags and routing together, resume caller exactly once, and prevent loser facts
from overwriting final state. Preserve a valid inherited entry response: a branch
that succeeds without replacing it can return that response, retaining its
original causal supplier rather than inventing a new branch supplier. Control
winner and factual network supplier are separate; failed branch stays ineligible
even with inherited response. Existing caller/try semantics decide how propagated
failure affects a previously held response. See the normative decision matrix.

### R3 — Address preference

Non-A/AAAA passes through once without reference. Preferred-family queries run
successor once and remember valid preferred Answer records. Nonpreferred query
with positive remembered evidence returns empty NOERROR without target I/O.
Otherwise run original successor and preferred-QTYPE reference successor on
independent state/raw query copies, bypassing this preference wrapper itself.
Valid preferred A/AAAA Answer suppresses original with empty NOERROR; failed or
negative reference permits original result. After original completes, reference
wait is at most 500ms capped by root deadline; elapsed wait permits original.
Preserve original error if preference does not suppress it. CNAME-only and
malformed/address-invalid answers are not positive evidence. Keep current
single-question admission; no multi-question expansion or implicit preference.
Positive evidence cache is bounded per compiled callsite, canonical-name keyed,
65536 entries, 1-hour absolute expiry, positive-only/nonpersistent. No unbounded
cleaner. Original/reference DNS cache keys differ by QTYPE. Endpoint/bootstrap
address family policy remains independent.

### R4 — Ownership, budget and cache publication

All branches belong to one admitted query. Root deadline bounds branch work,
lookup/dial/TC fallback and waits; no fresh five-second deadline. Winner/cancel
paths cancel and await losers; forced drop records started slots exactly once,
with async supervisor/catalog close draining owned transport work. Internal
branches do not admit new client queries or create extra audit rows.
Raw question/state/cache access guard/store token are branch-local. Never clone
an outer pending token into children. Completed valid successor may publish
its own cache result even if parent selects its sibling; partial/canceled
successor cannot publish. Synthetic suppression is not a reference upstream
cache response. Preserve one configured cache plugin and one dynamic cache
access per branch path; sequential duplicate access on that path fails closed.

### R5 — Observation and current detail UI

One client query yields one terminal record/duration/rcode sample. Upstream
metrics count actual entry work including probe/losers. One root live collector
orders attempts by actual registration, not completion or branch grouping;
terminal outcomes are exactly once. Final wire owns answers/flags/rcode;
selected entry/peer/transport exists only for final network response, not local
suppression/cache output. Reference supplier/error and loser flow_setter labels
cannot become final provenance. Preserve label precedence, old envelopes and
audit-off small-ID allocation rules.
User-approved on 2026-10-01: optional schema 2 with branch/QTYPE details and
existing detail display, retaining schema-1 reader support. Plain existing
forwarding retains schema 1. This approval fixes observation scope; it does
not start implementation.

### R6 — Real composition

YAML -> native listeners -> controlled peers -> HTTP records covers routing/
flow_setter, cache before/inside wrappers, multi-entry UDP and secure DoT/DoH,
busy fresh paths and sibling cancellation. Approved schema-2 branch details
require actual existing Vue/browser proof, not a new panel or trace service.

## Acceptance criteria

| ID | Observable proof | Requirements |
| --- | --- | --- |
| A1 | Includes/targets/quick syntax, invalid references/types/options/args, bounded recursion and source-position pre-I/O errors. | R1 |
| A2 | Barrier peers prove early-primary skip, failure/threshold start, standby hold/release, valid low-priority RCODE eligibility, first eligible winner, both fail and one caller continuation. | R2 |
| A3 | Both family directions, positive/negative/error/expiry, original/reference completion order, capped wait, TXT pass-through and CNAME/malformed wire match peer counts. | R3 |
| A4 | Winner state/routing/flow_setter, separate QTYPE keys, cache before/inside-wrapper hit/miss and canceled-store rollback; no probe/loser provenance leak. | R2–R4 |
| A5 | Root deadline, caller cancel, future drop/nesting and close terminalize all started work once; async drain leaves no owned pending work and permits rebind. | R4 |
| A6 | One client record, actual metrics/ordered attempts, accurate selected omission and HTTP final facts agree with wire under approved public policy. | R5 |
| A7 | Isolated mosdns-rust UDP/TCP DNS/HTTP routing/cache/secure composition and browser proof of approved details. | R6 |
| A8 | Relevant checks, fmt/clippy, full workspace integration/doctest regression/native build and changed UI builds pass; whole delivery dedicated C2C PASS. Environment failures are unresolved, not false PASS. | All |

## Out of scope

QUIC/H3/pipeline, OS resolver/Happy Eyeballs, ECS/lazy cache/dump, multiple cache
plugins, upstream editing, switch/special_groups management, Send/multicore,
capacity/performance claims, Go/cgo extension, deploy/default cutover and Phase 6.
P31/P45 gain only evidenced subitems, not complete 5B/5C/5D. All builds/tests via
isolated mosdns-rust SSH VM; preserve unrelated local/remote files/processes.

## Planning status and handoff

Product decisions are resolved. Inherited response, predecessor cache-watch
completion and schema omission contracts are frozen in research/branch-contracts.md.
PRD convergence preserves R1–R6, A1–A8 and all source anchors. Three planning
artifacts and the TDD interface/mock-boundary matrix are complete.

Primary risks are successor capture/cache-watch semantics and live accounting
for nested branches. They require the designated red tests before implementation,
not a new product choice. No implementation or test execution occurred here.
The revised task received advisory C2C Planning READY after two discussion
rounds; dispositions are in research/c2c-discussion.md. This is not a code review
PASS. The final-summary approval is now present in the user message; the
dedicated reviewer binding/transport and genuine scope snapshot are verified
before start. See execution-prompt.md for a transferable conditional handoff.
