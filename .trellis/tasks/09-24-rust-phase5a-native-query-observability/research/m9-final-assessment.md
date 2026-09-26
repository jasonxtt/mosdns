# M9 final bounded acceptance: W2 passes; W3 invalid, A5 blocked

User authorized one Linux regression, remaining W2/W3 supplement and one final
A1–A6 review. Preparation M9-UNIT1-001 received002reviewer PASS12:52:56UTC
on2026-09-26 for parent6650a1f4..headceea0ebb. The frozen18-session batch
was started once; no retry, replacement or hidden threshold change occurred.

**Final acceptance is NOT PASSED.** W2 passes the declared bounded screen;
W3 is invalid because the executor's controller used unsupported fixture IDs
`route_a/b/c` instead of the helper's `route-a/b/c`. This is a measurement
implementation defect, not evidence that Rust query performance is inadequate.
It affects both old and new binaries. No W3 tail-performance inference is valid.
The batch was stopped on this major issue:9 complete W2,4 complete failed W3,
1 interrupted W3,4 W3 slots never started. No new traffic is authorized.

## Linux and source gate

Unchanged Rust/helper source18d71c8c, Rusttree029d171b; latest Linux workspace
test log totals869 passed/zero failed including doc tests (native-host88).
`cargo test --workspace --locked`, strict workspace/all-targets Clippy and
rustfmt check pass. Latest helper `go test -race -count=1` and `go vet` pass.
120 staged source/input files match pinned source. Two initial packaging
failures (Rust include_str YAMLs, Go runner shell script) were corrected by
restoring same-commit dependencies, with both logs retained separately.
No product/runtime source change or binary rebuild occurred in M9.

Candidate SHA13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce;
helper-v11 SHA1fceab7d2f26dbd40dab8b06e56026482fd42168ac7b8d13f792c3e6076ee2f3
(CLI v10); old SHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa.
Same helper used in all variants. Preflight/after-stop executed remote inputs
match the frozen identity; historical local tool hashes match reviewed commit.
Offline repair below has NOT been staged to either host or used for queries.

## W2 cache result

All9 sessions have2500 scheduled/sent/received/correct warm queries plus2
correct cold queries:22500 warm +18 cold, zero shortfall/error/late/timeout.
All54 W2 response/sender/session-counter/TTL oracle invocations pass. Final
fixture counts are exactly one query per cache key across cold+warm, proving
warm queries add no upstream traffic. TTL30s/safety500ms oracle passes using
the same client ledger/processes. Cold observations are correctness-only;
two queries per session do not establish a cold p95/p99 performance claim.

| Paired median ratio | p95 | p99 |
|---|---:|---:|
| New off / old off |0.8651|0.9697|
| New on / new off |0.9249|0.9421|

All four medians<=1.10 under the frozen rule. Individual audit-on/off p95
round1=1.1625 and p99 round2=1.1200 cross1.10; this variability is retained.
Passing the predeclared median screen does not establish every-round stability
or a general speedup. Full rows and all individual comparisons remain available.

| Variant | Median p50 ms | Median p95 ms | Median p99 ms | Median peak RSS KiB | Median bracket CPU s |
|---|---:|---:|---:|---:|---:|
| Old off |0.351|0.912|1.958|3204|0.23|
| New off |0.341|0.772|1.758|3344|0.23|
| New on |0.347|0.725|1.821|3980|0.24|

Correct offered throughput100QPS for each25s primary. CPU/RSS are auxiliary
server-local brackets; no normalized CPU-overhead or capacity conclusion.
M8 W1 remains27000 correct/zero errors and its reviewed100QPS screenPASS;
its four paired medians and disclosed round1audit-on p99=1.2715 are unchanged.
Existing M8 raw-stage median p50:old0.653ms,new-off0.661ms,new-on0.674ms;
no W1 query was repeated to obtain these values.

## W3 defect, stopping and offline correction

Each of4 complete W3 sessions has3000 scheduled/sent/received but0 correct,
3000 wrong responses. Sender oracle passes; response, routing and counters
fail (12 failed oracle invocations). Helper `fixtureAnswer` uses exact hyphen
IDs; unsupported underscore IDs fall through to NXDOMAIN. Bound sockets and
affinity checks could not detect this contract mismatch. The controller lacked
an independent fixture-answer ID contract check; prospective review missed it.

The fifth W3 was interrupted after299 scheduled/sent,154 wrong responses,
34 transport errors,111 timeouts and2701 skipped planned slots. Its raw stage,
299 ledger records, fixture journal/counters and sampler are retained separately;
it is not a complete primary or replacement. Four remaining IDs are explicitly
unstarted in interruption.json. Driver exit130 and original13 completed rows
are preserved. Full-matrix assessment is FAIL; zero latency in failed W3 rows
cannot serve as performance data.

Offline correction changes fixture IDs and matching counter filenames to
route-a/b/c. Regression extracts the helper's actual fixtureAnswer case IDs
and checks controller specs: RED on underscore IDs, GREEN on hyphen IDs.
Five focused tests now pass, including owned cleanup/UDP occupied-bind tests.
Inline trellis-check passes52 measurement regressions (one Linux-only skip)
and exact-path whitespace checks. Durable inputs and repaired current scripts
are deliberately distinguished; no report or script claims final A5 PASS.
No Rust source or helper behavior changed; repaired scripts are not measured.
Only a separately authorized, pinned W3 batch can close this evidence gap.

## A1–A6 consolidated mapping

| Criterion | Evidence/status |
|---|---|
| A1 | Linux slice2_config acceptance/negative grammar and observability tests; sole listener/audit switch and disabled sensitive capture pass. |
| A2 | Linux observer/execution/UDP/TCP tests: exclusive terminals, actual cache/routes, upstream/local failure provenance and interrupted checkpoints pass. |
| A3 | Linux observer tests: cumulative inclusive histogram, partition/reconciliation, bounded labels, exact ring eviction and100000 default capacity pass. |
| A4 | Linux W1 UDP/TCP,W2,W3 integration: mixed identity/route, deadlines, malformed input, canceled cold publication, shutdown/drain/rebind pass. |
| A5 | Linux functional gate and bounded M8 W1/M9 W2 pass; valid W3 comparison is missing. **FAIL/BLOCKED**, no full task acceptance. |
| A6 | Coverage/handover updated only for proven subset, exact-path audit and consolidated review submitted. Final reviewer verdict required; cannot close while A5 is blocked. |

Implementation discovery/refinement range for semantic review is
c0e905612960ff6e5b3102397e500f1be50cd83c..18d71c8c06d98a405d889b1bee54021d899616b8,
restricted to native-host observability/config/execution/listeners and tests;
reviewed source units/evidence are in implement.md. Current final submitted
range contains only task evidence, offline harness repair, quality guideline
and bounded handover/coverage updates. No unsolicited full suite or traffic.

## Durable evidence and boundary

Full durable tree:
/Users/tom/.codex/artifacts/mosdns-phase5a-m9-remaining-ceea0ebb-20260926.
480 manifest entries verified; SHA
ebd8797c749d6d7be596b314080c505b3fa08c2b8af53ce9a8f5cf6b503bd318.
Selected repo copy463 files =480 minus14 request ledgers and5 routing journals
plus manifest andSHA sidecar. All omitted raw files are present in durable
copy and on their source hosts.34817 total ledger records include all failures
and the interrupted299.28 source trees rehash exactly;13 completed primary
stage/resource merges reconstruct exactly.58 oracles pass/12 fail. All owned
server processes independently verified stopped; interrupted client already
exited. Hardware,CPU masks,TTLs and unsynchronized clocks unchanged.

M8/M9 screens are limited100QPS; V12/M2–M7 failures/unqualified results stay
unchanged. Full5A, C08 audit/API/Prometheus/WebUI/persistence, higher-load and
multicore capacity, production/cutover and task lifecycle closure remain open.
