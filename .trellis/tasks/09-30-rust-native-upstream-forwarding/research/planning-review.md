# Supplied planning feedback — source verification and disposition

2026-09-30; source baseline 860c6253. The user pasted an external assessment
with P1-1/P1-2/P1-3/P2-1/P2-2 and requested adoption where justified. This is
feedback content, not a verified dedicated reviewer binding or final verdict.
This session inspected actual source/specs and adopted all five points.

| Finding | Verified evidence | Planning correction |
| --- | --- | --- |
| P1-1 | sequence-core/src/program.rs:499 External carries only ExecutableId; native-host/src/assembly.rs:553 catalog dispatch maps that ID to an adapter; config.rs:652 primary_forward discovers externals. | Host call-site external IDs map to immutable parent-definition/ordered-original-entry descriptors. Quick calls share that model; owners remain definition-owned. Generic sequence-core payload is unchanged. Design fixes primary/introspection/checkpoint/public identity boundaries; A1 and Slice1 verify them. |
| P1-2 | native-host/src/api.rs:812 AuditLogResponse has old supplier strings but no serialized attempts or final target transport. flow_setter may override final_upstream; same peer does not identify entry. config-compatibility.md:12 requires explicit versioned migration. | User selected expansion in response to the explicit scope question. Optional upstream_diagnostics/schema_version1 freezes factual selected entry/peer/transport and ordered terminal attempts across v2 logs/domain/slowest and actual QueryManager/Overview nested details. Old fields/routes/Go behavior remain. Same-peer/override/missing/unknown-version assertions in A7/Slice3/4. |
| P1-3 | execution.rs:101 in_flight_executable is one pending plugin ID; Drop at :203 captures partial observation; attempts are currently formed after await at :491. observer.rs:722 fallback adds one pending upstream. | Before I/O register checkpoint-visible copyable entry slots; each guard terminalizes once. Normal cancel/winner awaits legs, abnormal drop cancels/seals unfinished slots; shared ledger replaces plugin fallback. Parent async drain separately proves resource release. A8 and Slice1/3 cover active multi-leg drop, exact metrics and rebind. |
| P2-1 | rust-migration.md Phase5A audit-off contract requires audit-only string omission, metric-required ordered attempts and copyable pending IDs; current capture sampling is an admission eligibility bit plus terminal runtime gate. | Selection/checkpoint facts use invocation/entry IDs, SocketAddr and enums; compiled static registry supplies basic metric labels without per-query display allocation. Started slots retain start order despite reverse completion; winner ordering is independent. Preserve inline zero/one case and explicit sampling distinction. |
| P2-2 | rust-migration.md:931 incorrectly described explicit0 as A-only; resolver/mod.rs ConfigVersion::mode uses Zero -> PreferIpv4Dual; from_optional(None) is still Ipv4. | Correct current spec signatures while preserving archived history/core None/Default. Record future user-approved host omitted/0 mapping separately from already implemented core dual mode. Execution checklist requires explicit conformance check, not generic 'necessary specs'. |

All five are closed as planning gaps in the revised artifacts. No product
implementation or test PASS is claimed. New code must prove the frozen contracts
and later receive an exact whole-task independent review. Latest full-summary
approval is still required before activation; prior family/schema approvals
are decisions within the plan, not task-start authorization.

The P1-3 correction explicitly extends the host ExchangeExecutor signature with
an AttemptSink passed from the already checkpoint-attached ledger before await.
This makes visibility possible on failure/drop; merely returning a richer result
would not close that gap. sequence-core's External payload stays unchanged.

Retained choices: distinct maximum-three entry fanout, explicit bootstrap/dial,
no OS lookup or cross-family connect racing, same-target exactly-once TC fallback,
typed-busy-only fresh exchange, immutable secure service identity and scoped
loser cleanup. No supplied point required overturning the bundled scope.
