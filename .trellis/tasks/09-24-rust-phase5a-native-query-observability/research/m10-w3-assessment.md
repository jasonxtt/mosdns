# M10 W3 evidence remediation: numeric/route gates met, review pending

User explicitly authorized corrected W3 nine sessions once and scoped A5/A6
re-review. M10 preparation002reviewer PASS13:36:08UTC on2026-09-26 pinned
daeff167f16b4b4e816329de3e12c706cd03ccf5. Exactly9 W3 sessions completed
without replacement, Linux/W1/W2 repetition, load/config/clock/hardware changes
or runtime/helper source changes. All27000 scheduled/sent/received/correct
on time;zero late/wrong/protocol/transport/timeout/shortfall. Actual old/new-off/
new-on configs and binary SHA identities match. No new queries after the batch.

**Original driver verdict remains FAIL.** The existing Go route validator
received fixture barriers0/0 from the client and failed in all9 sessions; it
also requires fixture timestamps to lie within client request intervals. That
validator assumes a shared host clock. Our already documented14-minute clock
offset makes it incompatible with this distributed native-only comparison.
Response/sender/route-counter validators all pass (27 invocations); the9
legacy route-validator failures are retained. One session also logged an
ESRCH during post-stage owned cleanup. Neither diagnostic has been erased.

The separate offline-route-review result meets the unchanged DNS, route and
numeric latency contracts. It does not overwrite original rows, timestamps,
latencies, or the raw FAIL verdict. Final acceptance requires explicit reviewer
approval of this validation repair as well as the measured evidence.

## Complete routing proof without shared time

Fresh single-stage sessions have exactly3000 client requests with unique DNS
IDs and5000 contiguous fixture events. This Rust-before/Rust-after subset
preserves query DNS IDs on upstream legs; the oracle verifies this property
for every event rather than assuming clock alignment. Join by unique DNS ID
and exact qname/qtype/qclass, then require each request's ordered fixture
sequence to be A, B→A or B→C according to its frozen workload case.

The offline oracle rejects ID reuse, unknown/mismatched questions, mixed
run/stage identities, request/fixture sequence gaps or duplicates, missing or
extra events, wrong path/order, incorrect client outcomes, or counter/latency
disagreement with immutable raw stages. Client intervals are checked only on
their own clock, preserving nanosecond precision. Server timestamps are
format-validated without shifting or comparing them to client time. Every
event must match one request and every request its complete expected path.
No inferred barriers, time-offset correction, exclusions or extra queries.

All9 proofs pass:27000 unique client IDs and45000 matched events,1000 queries
per frozen case per session. Existing route-counter oracles independently
pass. `m10-route-oracle.py` also requires the original9-slot plan, reviewed Git
tool/preflight hashes, postbatch identity proof, source-host manifests, actual
variant/config identity, all three other original oracle exits0 and the exact
known0/0 legacy failure. Derived rows keep original_runner_exit=1, and their
runner_exit=0 denotes the separate complete offline validation result.

Unit tests are RED missing oracle then GREEN; tampered path/order, missing/
extra events, duplicate IDs, question/run/count mismatch and sequence gaps
fail. A nanosecond parsing regression was RED then GREEN. No general Go
comparison or DNS-ID-reuse workload is supported by this native-only join.

## Cleanup diagnostic and correction

m10-w3-r1-after_off cleanup stderr contains `[Errno 3] No such process`.
The controller's initial /proc read and post-SIGTERM poll caught ENOENT but
not ESRCH when a process disappears during reading. Both boundaries now
handle ProcessLookupError as already exited; PID/start ownership and pidfd
signaling checks are unchanged. Tests for both sites are RED→GREEN. This
controller repair occurred after measurements and was not staged or used for
traffic; the executed controller remains pinned to daeff167 in raw identity.

An independent server receipt verifies all36 recorded PID/start identities
have no active owned process. Subsequent sessions also successfully bind the
same exclusive UDP ports and verify their own PID ownership. The offline
derivation accepts only this exact exited-process diagnostic with a complete
matching cleanup receipt; any other setup/evidence/cleanup failure still blocks.
It does not claim the original cleanup commands all returned zero.

## Fixed latency screen

All actual DNS load is valid100QPS30s. Numeric thresholds are unchanged:
within-round paired median p95/p99 off/old and on/off<=1.10. Offline validation
changes no raw latency/count/RSS/CPU values or pairing. Four medians pass.

| Paired median ratio | p95 | p99 |
|---|---:|---:|
| New off / old off |0.9575|1.0637|
| New on / new off |1.0220|0.9599|

Individual p99 ratios remain disclosed: new-on/off round1=1.1852 and
new-off/old round3=1.1283 exceed1.10. The frozen median rule is not an
every-round bound or statistical equivalence. No capacity/performance-win claim.

| Variant | Median p50 ms | Median p95 ms | Median p99 ms | Median peak RSS KiB | Median bracket CPU s |
|---|---:|---:|---:|---:|---:|
| Old off |0.648|1.060|1.629|3204|0.48|
| New off |0.651|1.047|1.721|3328|0.49|
| New on |0.653|1.053|1.652|4820|0.50|

CPU/RSS are auxiliary server-local SUT brackets; sampler covers first fixture
only, all3 fixture ownership/affinity verified. Client monotonic latency and
unsynchronized clocks retained. VM/hardware/allocations unchanged.

## Evidence and consolidated boundary

Full durable tree:
/Users/tom/.codex/artifacts/mosdns-phase5a-m10-w3-daeff167-20260926.
Original341-entry raw manifest rehashes exactly; all18 source trees and9
primary stage/resource merges reconstruct exactly. Sampled PID/start identities
match owned records. Separate offline proofs and postbatch cleanup/identity
receipts are included. Complete349-entry bundle manifest SHA
0ff0e93aa84173af6e498c8ed994078775b40e1af81721d6cd366e49e6919adc.
Selected repo copy333 files=349 minus9 complete request ledgers and9 routing
journals plus bundle manifest/SHA sidecar. All omitted raw files are durable
and on source hosts. Raw FAIL and cleanup error remain visible alongside the
separate derived assessment and provenance; no evidence is discarded.

Source18d71c8c/Rusttree029d171b/native13785b38/helper1fceab7d/old370573c8
unchanged, full hashes in identity. Latest Linux869/native-host88 tests,
Clippy/rustfmt/helper race/vet and M8 W1/M9 W2 bounded results are reused.
Original M9 wrong-fixture W3 is still invalid; M10 is a separately authorized
new batch, not retroactive qualification. All older failed/unqualified higher
loads retain their verdicts.70 measurement regressions pass(oneLinux-onlyskip),
source/doc whitespace checks pass, immutable raw inventory spaces retained.

Original M9 final review supports A1–A4. This evidence supplies the missing
A5 W3 DNS/routes and unchanged numeric screen, subject to review of the offline
repair. A6 coverage/handover now describes only this bounded basic-observability
subset with final verdict pending. Full5A, C08 audit/API/Prometheus/WebUI/
persistence, high-load/multicore capacity, deployment/cutover and lifecycle
closure remain outside scope. Submit one scoped A5/A6 remediation re-review;
no additional traffic, task archive, lifecycle completion or next task.
