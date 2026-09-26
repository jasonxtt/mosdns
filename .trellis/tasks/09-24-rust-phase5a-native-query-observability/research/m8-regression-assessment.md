# M8: simplified100QPS W1 screen passed

M8-UNIT1-001 received prospective002reviewer PASS09:56:21UTC on2026-09-26
for a4c2fec719f8e23cc228928d0e3e97cf89855bfd. The single authorized batch
then ran exactly9sessions,100QPS30s,old/new-off/new-on3each,balancedorder.
No retries, exclusions, parameter changes, extra queries or replacement runs.

## Fixed result

All9runner exits0.27000planned/scheduled/sent/received/correct on time;
zero sender shortfall,late,wrong,protocol,transport or timeout. All27response/
sender/fixture-session oracle invocations passed. Both source/binary/config/
helper identity checks match. No startup, evidence or cleanup failures.

**M8 PASSED the predeclared simplified100QPS W1 TCP screen.** All four
within-round paired median latency ratios are<=1.10. This is limited practical
regression evidence, not statistical equivalence, capacity or fullA5 acceptance.

| Round | Actual version | Correct | Shortfall | p95 ms | p99 ms |
|---|---|---:|---:|---:|---:|
|1|Old audit off|3000|0|1.060|2.018|
|1|New audit off|3000|0|1.096|1.724|
|1|New audit on|3000|0|1.207|2.192|
|2|New audit off|3000|0|1.054|1.973|
|2|New audit on|3000|0|1.116|1.896|
|2|Old audit off|3000|0|1.119|2.011|
|3|New audit on|3000|0|1.166|2.010|
|3|Old audit off|3000|0|1.121|2.094|
|3|New audit off|3000|0|1.141|1.900|

| Paired median ratio | p95 | p99 |
|---|---:|---:|
|New off / old off|1.0178|0.9074|
|New on / new off|1.0588|1.0579|

Individual variability remains visible: first-round audit-on/off p95ratio
1.1013 and p99ratio1.2715 exceed1.10. The frozen rule uses the median of3
rounds, not an every-round ceiling; this batch passes that declared rule
without proving each run is stable. No post-hoc threshold/exclusion was used.
All ratios and rows remain in assessment.json.

Auxiliary median sampled peak RSS: old2996KiB,new-off3128KiB,new-on4616KiB.
Median CPU seconds over server resource brackets:0.58,0.61,0.63 respectively.
Those samples do not establish normalized CPU overhead or sustainable capacity.

M7 had3missed sender slots and diagnostic audit-on p99median1.1101; M8 has0
missed slots and1.0579. This is consistent with the revised path performing
better in this batch. It does not isolate the effects of qname allocation,
helper buffering, host scheduling or network variability. No causal claim
that either change permanently fixes jitter or that Rust is globally faster.

## Exact evidence and boundary

Source18d71c8c, rebuilt native/helper pinned in m8-build-identity.json;
staged107Rust files matched committed source. All variants use the same new
helper on both hosts. Baseline/config/corpus unchanged from M7. Dedicated
M8roots preserve all earlier artifacts. Client clock remains unsynchronized;
latency/schedule intervals are client-local monotonic, resources server-local,
no cross-host wall-clock subtraction or clock changes.

Independent reconstruction matches all merged stage/resource originals;
owned PID/start sampling identities match in all9sessions. All18remote source
manifests rehash exactly and owned test processes have stopped.
Complete durable raw:
/Users/tom/.codex/artifacts/mosdns-phase5a-m8-w1-a4c2fec-20260926.
All251fullmanifest entries verified; adjacent m8-w1-results has244files:
251entries minus9requestledgers plus manifest.json/SHA sidecar. Full ledgers
remain durable and on client. ManifestSHA
5c558b884114c19922ebc5409fa71e440c4e17e4fa676dc6d53cf16880bda9d2.
See evidence-verification.json for independent local/remote checks.

Submit one consolidated result review. This batch is consumed; no new runs
or expanded workload authorized. M7 and higher-loadV12FAIL/M2–M6unqualified
remain unchanged; fullA5,W2/W3,capacity,production and lifecycle closure stay
outside scope. This screen supports only the stated100QPS W1TCP result.
