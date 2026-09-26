# M6: correct distributed W1 startup, then fresh fixed controls

The user's “继续下一步” authorizes this correction after M5-REPORT-001.
Executor=current inline, reviewer=002reviewer native task
01a0d43d-d0aa-7401-af0f-2ca3a45ba519. No hardware or allocation changes.

## Frozen units

Unit1: fix/review startup availability and failure evidence; freeze new
generation roots, tools and protocol. No measured traffic before explicit
prospective PASS. Unit2 after PASS: exactly one fresh two-batch W1 run,
18 attempts,36 primary windows,270000 queries. Report every outcome. Failure
stops before W2/W3/candidate; no reruns, exclusions or margin changes. Qualified
W1 permits only preparation of a separately reviewed W2/W3 extension.

## Correction and retained behavior

Probe sets SO_REUSEADDR before bind, never SO_REUSEPORT/listen. This matches
ordinary listeners' reuse semantics for closed TIME_WAIT sessions while an
active listener still rejects. It changes no OS setting or service. Real
Linux loopback active-close regression demonstrated errno98 without reuse
and three successful probes after correction; active-listener test still fails.
The tests use dummy TCP only, not measured SUT/DNS traffic.

Server creates fresh session root and empty owned.json before preflight;
early failure leaves startup-error.txt. Existing owned-start/pidfd cleanup
rules remain. Client creates session.txt before server startup. Captured remote
stdout/stderr is retained with controller/cleanup/evidence errors. Even an
early failed attempt has nonempty raw trees for source manifests. None of
these records contains credentials or an entire process environment.

M5 sources/results remain frozen; M6 driver/controller/qualifier are separate
experiment artifacts copied from reviewed M5 with this bounded correction.
Shared m5-remote-tools.py is unchanged and reused with exact hashes; original
summarize-slice3-pilot-v2.py analyzer is unchanged. Shared qualification only
adds m6 to existing25-second profiles; M2–M5 numerical rules are unchanged.
Roots: server measurement-v6/results-m6-server under existing benchmarkBASE;
client /root/mosdns-phase5a-m6-client/{tools,results}; run IDs m6-batchN-... .
No M5 roots/IDs are reused or overwritten; old verdict remains INVALID MATRIX.

## Unchanged fixed measurement and gate

Client10.0.0.50 Debian1vCPU/2GiB; server10.0.0.92 mosdns-rust2vCPU/4GiB.
All slots identical archived Rust-before audit-off on same server. Balanced
three repetitions per batch: before_off/after_off/after_on; after_off/after_on/
before_off; after_on/before_off/after_off. Each session200 then400QPS25s each,
500ms deadline/100ms drain; same SUT/fixture across both windows, fresh per
attempt. No recovery/capacity claim. No source/Rust/runtime/audit changes.

SUTCPU0, loopbackTCPfixtureCPU1, server1HzsamplerCPU1; clienthelperCPU0.
Go roles GOMAXPROCS1/GOGCoff/GODEBUGgctrace1/GOMEMLIMITabsent; client768MiB and
server2GiB available, sampledGoRSS<=256MiB, zeroGCtraces. Bind overlay only
127.0.0.1→10.0.0.92:15354; upstream remains127.0.0.1:15454. Original corpus,
helperv10 SHA28d5faf5f5129aa990aac51efd8752eba655b852f0bcb27619b216e572c0450e,
baselineSHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa
and config hashes from M5 stay fixed.

Owned PID/start must match before ready and every sample; both windows compare
to owned.json. Two host namespaces remain separate; sut_pid0 in client,
actualserverPID nested. 25–35.5s bracket/atleast25samples per role,100Hz clock;
CPU includes SSH gaps, cross-host clocks never subtracted. Client latency
includes LAN roundtrip. Postshutdown host-local source manifests verified
against exact copies, raw merge reconstructed, all seven oracles zero.

All18 runner exits0,36valid primaries,270000fullycorrect-on-time requests,
zero late/wrong/protocol/transport/timeout/shortfall. Both batches zero original
individual p95/p99guard crossings. All8 six-pair90%t5log-ratio CIs strictly
inside ±log1.10 (t2.01504837333302,SD/sqrt6); no widened budget. Freeze source
HEAD, tool/source/input identities, host inventories and order before traffic.
Actual run requires reviewed exact HEAD; perattempt identities before/after.
Failed/incomplete matrix is unqualified; W1 alone cannot pass A5. M2–M4,
V12 and acceptance gate stay unchanged. No deployment/archive/taskfinish.
