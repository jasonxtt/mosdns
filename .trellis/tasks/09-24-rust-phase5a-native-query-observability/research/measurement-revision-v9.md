# M9: remaining bounded acceptance

User authorized unify remaining criteria, one latest-source Linux regression,
W2/W3 supplement and one final A1–A6 review. Executor=current inline;
reviewer=002reviewer01a0d43d-d0aa-7401-af0f-2ca3a45ba519. Preparation is
prospectively reviewed; after PASS exactly18 sessions execute once and all
evidence is consolidated for final review. No substitutions, resampling,
tuning, repeated W1, new pressure tests, deployment, archive or task closure.

M8 W1 PASS remains bounded100QPS. Earlier higher-load failures retain their
verdicts. Final R7 interpretation in PRD limits cold-cache measurement to
correctness/cache publication; two cold observations cannot establish tail
performance. W2 fresh SUT/fixture receives exactly two one-pass cold queries
(warm-prefill), then the same processes/ledger receive100QPS25s warm-reference
(2500queries). Existing fixture TTL30s, safety500ms; existing ledger TTL oracle
must pass for both keys. W3 normal-reference is100QPS30s (3000queries).

Each scenario has three balanced old/off/on,off/on/old,on/old/off repetitions.
Every primary scheduled/sent/received/correct count equals2500 or3000;
zero shortfall, late, wrong, protocol/transport errors or timeout. Cold also
requires2/2correct and zero errors. Existing response/sender/session oracles
must pass. W2 final fixture counts exactly one upstream query per key across
cold+warm. W3 existing journal oracle reconciles every request and ordered
A,B→A,B→C legs. Paired median p95/p99 new-off/old and new-on/new-off <=1.10
per scenario. Disclose every individual ratio; p50/throughput/CPU/RSS reported.
Invalid load blocks performance PASS. CPU/RSS auxiliary, no capacity claim.

Unchanged hardware: Debian10.0.0.50 1CPU/2GiB client; mosdns-rust10.0.0.92
2CPU/4GiB server. SUTCPU0, clientCPU0, fixtures/samplerCPU1. HelperGo roles
GOMAXPROCS1/GOGCoff/GODEBUGgctrace1/GOMEMLIMITabsent; deadline500ms,
late drain100ms. Client monotonic latency, server-local resource bracket;
unsynchronized wall clocks never subtracted. No hardware/allocation changes.
W2 LAN UDP15355→loopback15455; W3 LAN UDP15356→loopback15456/15457/15458.
Config changes only listener LAN bind and audit toggle. Readiness checks owned
ss UDP PID/address, never sends a DNS query. All fixture affinities verified.
Unchanged M5 sampler covers SUT and first fixture only (W3 B/C resources not
sampled); all three fixture process ownership/counters/journal are retained.
Server resource window covers primary warm only; cold client samples retained
as diagnostics. No claim of all-fixture resource totals.

Reuses reviewed M8 Rust/helper source18d71c8c06d98a405d889b1bee54021d899616b8,
Rust tree029d171b54348236ef285ef02f9621b89499db21. Rust SHA
13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce;
helper-v11 SHA1fceab7d2f26dbd40dab8b06e56026482fd42168ac7b8d13f792c3e6076ee2f3
(CLI interface v10); old SHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa.
All variants use the same helper. New measurement-v9/results-m9-server and
/root/mosdns-phase5a-m9-client roots; IDs m9-w2/w3-rN-variant. Freeze inputs
before/after; source manifests verified for every host tree, merged stages
reconstructed locally, full ledgers retained externally with SHA manifest.
Attempted-start cleanup stops only PID/start-identity owned processes even
when SSH launch reply is lost. Retain failed slots without replacement.

Linux regression uses unchanged pinned Rust sources plus repository YAML
fixtures and current pinned Go helper sources. Initial package omitted YAML
include_str fixtures and failed compilation before tests; preserve that log,
restore same-commit fixtures and run the one actual regression. No runtime
code change/rebuild or W1 repetition is required by this packaging correction.
Go helper tests initially lacked their runner-shell fixture; restore the
same-commit scripts/run-phase5a-baseline.sh and preserve that packaging log.
Actual full Linux regression:869 Rust tests pass, strict Clippy/rustfmt pass;
helper race/vet pass.120 staged source/input hashes match pinned HEAD.
Final review maps A1–A6 and updates only bounded coverage/handover; full5A,
5C management/API/GUI and release/cutover remain deferred.
