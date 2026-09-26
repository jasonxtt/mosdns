# M8: fixed simplified comparison after reviewed cost remediation

User said “好 继续” to rebuilding and repeating the same simple nine-run
real-host comparison. Executor inline; reviewer002reviewer native task
01a0d43d-d0aa-7401-af0f-2ca3a45ba519. This is a new explicitly authorized
batch after source changes, not repetition of M7 until favorable.

Unit1: pin rebuilt Rust/helper, fresh M8 roots/scripts, no-query preflight and
prospective review. Unit2 only after PASS: exactly nine runs once, consolidated
evidence review. No tuning, exclusions, replacements, added calibration,
new workload, production or task closure. Preserve M7 and earlier failures.

All M7 measurement rules stay unchanged:100QPS30s,old/off/on;off/on/old;
on/old/off,3000queries each,27000planned. Every scheduled/sent/received/
correct count must equal3000,zero shortfall/late/wrong/protocol/transport/
timeout, response/sender/fixture session oracles pass. Paired median p95/p99
new-off/old and new-on/new-off <=1.10. CPU/RSS auxiliary. Invalid load blocks
performance PASS; mechanical latency ratios are diagnostic then. No capacity,
fullA5,W2/W3 or retroactive V12/M2–M7 qualification.

Fixed clientDebian10.0.0.50 1CPU/2GiB; servermosdns-rust10.0.0.92 2CPU/4GiB.
SUTCPU0,fixture/samplerCPU1,clientCPU0; same500msdeadline/100msdrain,
GoGOMAXPROCS1/GOGCoff/GODEBUGgctrace1/GOMEMLIMITabsent. TCPtestlistener
10.0.0.92:15354,loopbackfixture127.0.0.1:15454. Corpus and false/true audit
configs are byte-identical to M7. Client clock remains unsynchronized; no
cross-host wall-clock subtraction, hardware/allocation/OS clock changes.

Rust/helper source18d71c8c06d98a405d889b1bee54021d899616b8 contains reviewed
POST-M7-CODE-001: exact-sized qname rendering and64KiB helper ledger buffering.
New native SHA13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce;
new helper SHA1fceab7d2f26dbd40dab8b06e56026482fd42168ac7b8d13f792c3e6076ee2f3.
Old baselineSHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa
is unchanged. Both hosts use the rebuilt helper for every actual variant, so
measurement I/O behavior is contemporaneously controlled. Artifact filename
helper-v11 identifies the build; unchanged CLI interface version reportsv10.

New candidate-m8,measurement-v8/results-m8-server under existing benchmarkBASE;
client /root/mosdns-phase5a-m8-client/{tools,results},run IDs m8-rN-variant.
M8 scripts copy frozen M7 with generation/binary/helper identities replaced,
and inventory explicitly checks rebuilt-helper filename and interface version.
Unchanged run-m6-w1 transport/hash helpers and m5-remote-tools sampling/merge
are reused. Attempted-start cleanup guard retained. Exact input hashes checked
before/after; all source trees, failed attempts, merged files and ledgers retained.
Source-level optimizations do not establish a speedup before this fixed batch.
