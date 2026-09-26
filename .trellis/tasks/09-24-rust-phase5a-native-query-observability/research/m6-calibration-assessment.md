# M6 W1: startup fixed, offered-load validity still unqualified

Source `b664bade4e0b78b11305b2b70c9b86c76ea216ff` received prospective
002reviewer M6-UNIT1-001 PASS at07:37:41UTC before traffic. Its one fixed run
started07:38:03.910297UTC and finished07:54:50.443290UTC on2026-09-26.
Client10.0.0.50/server10.0.0.92, unchanged resource allocations, same archived
Rust-before/audit-off in every slot. No Rust/helper binary changes.

## Fixed result

| Measure | Result |
|---|---:|
| Attempt slots executed once |18|
| Runner exit0 / exit1 |8 /10|
| Observed primary windows |36|
| Valid primary windows |16|
| Planned queries |270000|
| Scheduled / sent / received / correct on time |269960 each|
| Sender shortfall |40|
| Late / wrong / protocol / transport / timeout |0 each|
| Go GC traces (54 logs) |0|
| Peak sampled generator / fixture RSS |48584 /23436KiB|

Both batches completed9 attempts in frozen balanced order. Batch1 has3
fully valid attempts; batch2 has5. All18 sessions started and completed both
query windows: the M5 TCP TIME_WAIT startup defect did not recur. Each raw
tree and every failed sender/stage oracle remain retained. No exclusion,
replacement, resampling, parameter or threshold change.

**M6 is UNQUALIFIED.** Ten attempts fail the unchanged zero-shortfall offered
load gate. Shortfall occurs in12 windows:3 affected windows at200QPS miss
8 planned requests;9 affected windows at400QPS miss32 planned requests.
There are40 missed planned requests in total, not40 affected windows.
Observed maximum sender lag is39376us. The offered-load gap is a measured
sender-schedule fact; host interference, scheduling, logging, helper work or
network causes are not isolated. Correct receipt of every actual sent query
does not establish valid planned load or performance equivalence.

The unchanged qualifier reports false with88 failures. It mechanically
calculates8 latency intervals from all numeric rows;0/8 fit the equivalence
margin. Those rows include invalid offered-load attempts, so these intervals
are diagnostic only, not eight accepted six-valid-pair estimates. The valid
matrix is incomplete: no candidate performance or stable-control conclusion.
No W2/W3 extension or candidate data; V12 remains FAIL and A5 remains unmet.

## Independent evidence checks

For all18 attempts, independently reconstructed merged stage/resource files
match immutable originals. Host source manifests, PID/start ownership in both
windows, affinity/profile/coverage and sampled Go RSS/GC checks pass before
the final oracle check. Full check passes8 attempts;10 fail specifically
because sender/stage oracles are nonzero. DNS fixture session counters and
resource-sample oracles are retained for each attempt, without hiding failures.

Complete raw run: /tmp/mosdns-phase5a-m6-w1-b664bad-20260926.
Durable complete copy:
/Users/tom/.codex/artifacts/mosdns-phase5a-m6-w1-b664bad-20260926.
All764 manifest entries were verified against that copy. Adjacent
m6-w1-results contains748 files: all764 manifested files except18 request
ledgers, plus manifest.json and its SHA256 sidecar. Omitted ledgers remain
in the full durable tree and on client. ManifestSHA:
`fd1a8df3581cce2ad93cbc7bb886ea836ed35ed405b875c1abb81b0a1e84d5ea`.

Independent remote rehash verified all36 session source manifests (18 server
trees of21 files and18 client trees of10 files). See
m6-evidence-verification.json. Raw server root is existing benchmarkBASE/
results-m6-server; client root /root/mosdns-phase5a-m6-client/results. M5 roots
and verdict stay untouched. No credentials are stored.

## Stop and next boundary

Keep acceptance closed. This fixed M6 run is consumed; do not rerun until
favorable or infer that different server hardware is needed. The next step
would need a separately frozen/reviewed diagnostic of sender lag and its
contribution to latency variance using these same machines. No change to
zero-shortfall/latency budgets, capacity claims, OS settings or production is
authorized by this report. CPU remains a bracket including SSH handoff gaps,
with100Hz quantization; cross-host timestamps were never subtracted.
