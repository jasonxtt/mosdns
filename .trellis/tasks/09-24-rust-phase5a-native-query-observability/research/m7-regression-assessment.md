# M7 simplified W1 result: not passed

Prospective review M7-UNIT1-002 passed08:28:41UTC on2026-09-26 for
ba0f4a890e96adf2210d70576bb1c4c9b82a809d. Its single frozen batch ran
after that PASS:100QPS30s, actual old/new-off/new-on three times each,
balanced order. No tuning, exclusions, replacement runs or product changes.

## Outcome

Nine sessions started and finished; seven runner exits0, two exits1.
27000 planned queries;26997 scheduled/sent/received/correct on time.
Zero late, wrong, protocol, transport or timeout responses. Third-round
new-on missed1 planned slot(max sender lag24813us); third-round old missed2
(max32303us). Their stage/sender oracles fail, while all nine fixture
session-counter oracles pass. This is sender schedule shortfall, not server
response loss; its cause is not isolated.

**M7 is NOT PASSED.** The predeclared zero-shortfall rule fails. The full
three-round latency matrix therefore cannot establish an accepted regression
PASS. Mechanical ratios below retain all observations and are diagnostic.
Audit-on p99 additionally exceeds the declared1.10 median screen; both fully
valid first/second round ratios are1.1101 and1.1535. This flags a latency
concern, without isolating observer cost from client/network/host variation.

| Round | Actual version | Correct | Shortfall | p95 ms | p99 ms |
|---|---|---:|---:|---:|---:|
|1|Old audit off|3000|0|1.049|1.762|
|1|New audit off|3000|0|1.144|1.898|
|1|New audit on|3000|0|1.159|2.107|
|2|New audit off|3000|0|1.181|2.137|
|2|New audit on|3000|0|1.250|2.465|
|2|Old audit off|3000|0|1.143|2.033|
|3|New audit on|2999|1|1.110|2.002|
|3|Old audit off|2998|2|1.325|2.158|
|3|New audit off|3000|0|1.435|2.515|

| Diagnostic paired median | p95 | p99 |
|---|---:|---:|
|New off / old off|1.0830|1.0772|
|New on / new off|1.0131|1.1101|

Auxiliary median sampled peak RSS: old3132KiB,new-off3208KiB,new-on4708KiB.
Median CPU seconds over sampler brackets:0.57,0.65,0.64 respectively. These
are process samples, not capacity or normalized CPU-overhead acceptance.

## Evidence and limits

Before/after tool/binary/config/corpus identities match. Actual binary SHA,
audit flag, owned PID/start and affinity are recorded per session. Independent
reconstruction matches all merged stage/resource files; both original host
manifests per session rehash exactly,18remote trees. Owned SUT/fixture
processes have stopped. See m7-w1-results/evidence-verification.json.

Complete durable raw:
/Users/tom/.codex/artifacts/mosdns-phase5a-m7-w1-ba0f4a8-20260926.
All251 full-manifest entries verified against that copy. Adjacent
m7-w1-results contains244files:251 entries minus9 request ledgers plus
manifest.json and its SHA sidecar. Ledgers remain in durable/client raw.
ManifestSHA c9bdecb8fdd961517e8a2af9e092214c2cd8f7fb267945ff216cea0131b62976.

Client wall clock is about14minutes behind local/server clock (read-only
check08:36:32local,08:36:33server,08:22:27client). Client started_at fields
therefore describe that host's clock, not synchronized global chronology.
Latency and schedule durations are measured within the client process;
resource bracket duration is server-local. No cross-host wall-clock
subtraction or clock setting change was used.

This consumed batch is limited100QPS W1 TCP real-host evidence. It neither
qualifies prior M2–M6 nor erases higher-load V12FAIL; fullA5/W2/W3/capacity
and production acceptance stay closed. Submit this whole outcome once for
result review; no automatic rerun or threshold change. Further fixes should
start from these retained sender/latency records, with bounded scope.

M7-REPORT-001 returned FINAL: PASS08:40:43UTC for report/evidence and stop
only. It confirms the retained outcome, not regression/fullA5 acceptance.
