# 5D-1 baseline — Stop2, external review pending

2026-10-09, BASE `747607a4`, file package `0412666`. **User-confirmed external
Claude threshold feedback recorded; F Go baseline completed before native runs.
Native comparison complete; overall budget FAIL, external acceptance pending.**
No commit, push, deployment, runtime/Go/config-package edits or self-review round.

VM alias mosdns-rust:2vCPU KVM,7938MiB RAM, Linux7.0.9-x64v3-xanmod1.
SUT CPU0;sender/mock CPU1. Go1.26.4,GOMAXPROCS1,default GC/memory settings,
CGO0/trimpath/stripped release. Exact BASE offline builds use repository scripts;
both fresh Vue bundles precede native/Go compilation. Native formal and tool
smoke runs use the same binary. See [environment hashes](environment.json).

Same immutable production snapshot and config_all/config_rust, native migrated
settings. E2E-1 private copies preserve rules/protocols/proxies/cache/log capacity;
reserved probes added symmetrically. Loopback-only netns,private /tmp,no public
routes. Audit explicitly on (snapshot persists capacity, not historical capture
state); cache dump empty per fresh process. Mock domestic10ms, foreign/foreignecs
80ms,cnfake/nocnfake1ms. **Original mixed workload only exercises domestic/
nocnfake UDP; F below adds foreign DoH/SOCKS5/80ms coverage.**

10,451 valid unique names from10,486 production top_domains rows; count weights
retain99.9995096% of original mass.98% weighted production+2% block/rewrite/
unmatched probes;A70/AAAA30 is an explicit assumption. Seeds501/502/503.
Go functional oracle20,910 cells checks question/rcode/all DNS sections;
TTL compared separately within0..300, not equal across cached ages. Prior E2E
establishes detailed route/audit contracts; load does not scrape every audit.
Go baseline "correct" means conformity to its accepted legacy reference;
it does not make Go's known case-sensitive quirk normative for native.
One already approved case-insensitive full: AAAA cell has a strict native
empty-response oracle, pinned from accepted prior evidence before any candidate
measurement. Every other oracle cell is unchanged. See
[contract provenance](oracle-contract-case.json) and
[the frozen rule contract](../../contracts/native-rule-providers.md).

UDP open loop, absolute planned timestamps,128 sockets/unique socket-ID pairs,
deadline500ms from planned slot. Slots over5ms late count as client unsent;
no catch-up bursts. Primary latency planned-to-finish; sent-to-finish also kept.
Received quantiles include completed wrong/late replies if any; timeout/unsent
remain in denominators. Whole-run and stable-window counters both retained.
Resources sampled250ms (RSS peak is sampled, not allocator high-water).
CPU tick bracket can be up to250ms shorter than25s (~1%);100Hz resolution and
GC/seed differences matter especially at low rates. Metrics outside timed load.

Initial matrix3 fresh-process runs:100qps10s warmup,20/100/500/1000/2000/4000/
8000/16000qps each30s,first5s settling,immediate100qps60s recovery.
Since16000 had no service overload, add3 fresh extension runs: same warmup,
32000/64000qps each30s and same recovery. Matrices remain separate (different
cache warm history); native must repeat both after freeze. All attempts retained.

## Go results

Table: median of3 per-run stable quantiles/resources, rate is planned qps;
correct rate includes client unsent. RSS is median steady sampled RSS in MiB.

| qps | p50/p95/p99 ms | Correct % | CPU µs/correct | Steady RSS MiB |
|---:|---|---:|---:|---:|
|20|1.354 /12.750 /13.789|100|380.0|182.84|
|100|1.129 /12.593 /13.641|100|312.0|189.02|
|500|0.807 /11.211 /12.605|100|146.4|195.49|
|1000|0.690 /2.050 /12.099|100|114.0|172.29|
|2000|0.617 /1.766 /11.863|99.996|88.6|194.56|
|4000|0.652 /1.648 /11.333|99.987|68.5|215.73|
|8000|0.552 /1.690 /4.702|100|47.6|225.63|
|16000|0.375 /2.411 /5.512|99.99825|32.7|236.04|
|32000 extension|0.179 /10.674 /50.883|99.987|23.5|266.95|
|64000 extension|90.507 /163.990 /217.345|77.4093|19.4|303.85|

Full per-run/all-window/stable counters, sent latency, p99.9 when >=10k replies,
sampling/noise and60 per-second recovery bins:
[initial3](go-main.json),[extension3](go-extension.json).
Do not read low CPU/correct at overload as an efficiency win: that workload has
large loss and failed latency.32000 p99 ranges36.438–55.367ms;64000 p99
205.117–217.927ms. Initial <=500 p99 ranges about0.55–0.79ms across repeats;
CPU/correct varies roughly24–36% min-to-max, so the proposed CPU gate uses
the median of3 rather than an individual noisy tick sample.

Initial stable service wrong/timeout=0 at all rates; client unsent counts
38/34/4 across entire stable ladders including recovery (run2 includes1 recovery
client-unsent request).
Extension32000 client-unsent0/1010/104, service wrong/timeout0.
Extension64000 stable timeouts239804/213588/223626 and client-unsent
121647/139357/139485. Send coverage91.282–92.397% at64000; SUT CPU95.6–95.8%,
sender87.2–87.9% plus mock~2.7%. This is mixed overload with tool shortfall,
not a clean maximum-capacity measurement.32000 also has client jitter in run2.

Under frozen common latency/correctness budget, highest tested qualifying rate
is16000qps; a finer capacity ceiling is **not established**. Capacity is workload
and cache-history specific, not a production QPS promise.
Initial outer-cache hits98.490–98.497%; extension99.274–99.278%.
Starting sampled RSS168.28–184.24MiB in initial runs; initial peak242.22MiB,
extension peak312.05MiB. Go retains post-overload RSS; this alone is not leakage.
Steal fractions0.038–0.367% across all runs/cores.

After actual64000 overload, first recovery-second p99=41.276/95.371/79.933ms;
each run then reaches5 consecutive healthy1s bins by t=6s. Whole recovery p99
2.075–2.963ms; correct rates100/99.95/100%, run2's3 failures are client-unsent.
FD returns to30,6–7 threads (same as each run's pre-overload count). FD/thread
trends do not prove absence of every internal task; native in-flight and5D-2/3
remain necessary evidence.

## Frozen thresholds (2026-10-09, confirmed by 用户)

Confirmed user-mediated external Claude feedback; executable numerical record:
[frozen-thresholds.json](frozen-thresholds.json). Normal20/100/500 p95/p99 ceilings(ms):
14.025/15.168,13.853/15.006,12.333/13.866 respectively.
Formula max(Go median×1.10,Go median+0.2ms),rounded upward.
Normal correct/on-time >=99.9% each repeat, wrong DNS responses0;
client coverage>=99.9%, client shortfall classified separately.
Capacity>=2,000qps using shared p95<=14.025/p99<=15.168ms and>=99.9%
correct/send in each repeat; ratio to Go is report-only. CPU/correct is report-only;
any native non-overload repeat >1.5×Go3-run median requires targeted profiling.
Native steady<=Go and<=128MiB, sampled total peak<=256MiB;
recover within10s to5 consecutive common-load healthy bins plus bounded
FD/in-flight resources. Overload rows are diagnostic, not normal success gates.
Near-noise results are uncertain/add samples, not repeated tool invalidation.
Latency compares medians of3 stable per-run quantiles; correctness and resource
caps checked each repeat; scatter retained. Capacity uses every repeat's budget.

F added by same user approval:100QPS,globally unique reserved names,A70/AAAA30,
foreign DoH via SOCKS5,80ms mock. Symmetric disposable suffix branch after outer
cache/top_domains and before ordinary A/AAAA routing; policy/mapper/cache/audit
retained. It exercises the forced secure-upstream path, not ordinary fake-IP
routing. Fresh cold process,100QPS10s warmup,30s stage(last25s stable),60s
continued unique tail. All10,000 names unique per run. Three Go formal runs,
before any native formal measurement: p95 median84.000269ms(min83.149314,
max84.229682),p99 median84.853301ms(min83.944322,max85.745173); stablecorrect
and coverage100% each. CPU median408µs/correct(range312–436); steadyRSS median
194.96MiB. All foreign:https,29,999 total sent exchanges,zero cache hits;
SOCKS connections10/10/11 allowed. [Go F baseline](go-f.json).
F p95/p99 ceilings **92.401/93.339ms**,same max(×1.10,+0.2ms) formula rounded
up to0.001ms;each-repeat correctness>=99.9%. F CPU profiling trigger612µs.

F exploration exposed a sender drain classification defect: final on-time
replies were labeled late without testing the actual500ms deadline; drain also
omitted TTL validation. Fixed with one shared classifier/regression test before
formal F baseline and all interleaved comparisons. Three exploratory F runs
retained separately; their stable rows were unaffected. Earlier main/extension
Go baseline sender unchanged; the fix applies only to final drain, not timed
ladder/recovery schedule or steady quantiles. New Go controls use exactly the
same corrected sender as native. Raw records remain unchanged.

## Native results and frozen budget decisions

All18 interleaved runs completed: main/extension/F, Go/native ×3; order
Go/native, native/Go, Go/native by seed. [Matrix manifest](matrix-progress.json).
Tables use medians of3 stable per-run quantiles, not pooled requests. Every
repeat's correctness/coverage/resources remains separately visible in
[main](native-main-public.json), [extension](native-ext-public.json),
[F](native-f-public.json). Comparisons retain the original frozen Go baseline.

| Item | Frozen limit | Native observation | Decision |
| --- | --- | --- | --- |
| 20QPS p95/p99 | 14.025/15.168ms | 13.204/14.703ms | PASS |
| 100QPS p95/p99 | 13.853/15.006ms | 12.125/13.124ms | PASS |
| 500QPS p95/p99 | 12.333/13.866ms | 11.123/12.149ms | PASS |
| Normal20/100/500 on-time correct each repeat | >=99.9% | minima100/99.92/99.936% | PASS relative to pinned oracle |
| F p95 | 92.401ms | 91.241ms | PASS |
| F p99 | 93.339ms | 96.644ms (range96.102–97.415) | FAIL |
| F on-time correct / sender coverage | >=99.9% each | stable100% /100% all3 | PASS |
| Capacity latency/completion component | >=2,000QPS | highest qualifying tested point8,000; Go16,000, ratio0.5 report-only | PASS component; semantic qualification below |
| CPU/effective query | report; >1.5×Go any non-overload repeat requires profile | F904µs vs408µs, ratio2.216; range888–936 >612µs | Report; required profiling completed |
| Steady RSS | <=Go and<=128MiB each | 2k145.41MiB; 8k173.67MiB; extension213.20MiB median | FAIL absolute; relative-to-Go PASS |
| Sampled process peak | <=256MiB | maximum222.18MiB across native runs | PASS sampled metric |
| Overload recovery latency/completion | five healthy1s bins finish<=10s | extension5/5/10s | PASS |
| FD recovery | baseline+2 within10s, no rising tail | five bounded250ms samples finish0.962/1.232/0.991s; tailFD32,3threads | PASS observed metric |
| Query in-flight recovery | bounded | supplementary observer0 by0.500/0.896/0.844s; terminal admitted=completed | Supplementary PASS; formal binary not directly observed |
| Zero wrong DNS / oracle agreement | zero | stable mismatches remain; oracle coverage defect and unexplained cold overload cells | NOT ESTABLISHED; strict oracle agreement FAIL |

Capacity8k is a conservative tested latency/completion point under the unchanged
reference, not the exact maximum, nor a full product-correctness acceptance.
16k fails each-repeat correct gate in one run (99.8915%);32k extension one run
99.838625%;64k is mixed sender/SUT overload, native82.65–84.01% on-time correct.
Received p99 at64k excludes missing replies; its15.757ms median cannot imply
healthy operation or CPU efficiency. Main and extension have different cache
warm history; do not splice their rows into one capacity curve.

Native normal20p99 ranges13.767–15.305ms (one repeat over ceiling); the frozen
gate compares the median, hence PASS with visible scatter. Go controls normal
p95 medians13.032/12.482/11.188ms and F84.066/85.187ms p95/p99 are close to the
frozen baseline. Go8k p99 varies4.702→5.958ms between baseline/control medians;
Go32k p95 control range10.599–21.503ms. CPU ticks, scheduler, GC and short-tail
samples limit small-effect conclusions; no noise-based invalidation/rerun or
candidate-derived threshold adjustment. See separate
[Go controls main](go-control-main-public.json),
[extension](go-control-ext-public.json), [F](go-control-f-public.json).

## Correctness-reference limitation

Post-formal functional diagnostics checked all20,910 cells ×3 per binary.
Go62,730/62,730 agrees with its reference. Native462 fixed cells differ in each
pass:18A+444AAAA, expected Go answer vs native empty NOERROR; question/TTL/all
sections checked. Private snapshot rules place every cell in previously frozen
ASCII-case-insensitive uppercase `full:` nov4/nov6 rules; native empty results
conform to the Oct7 product contract. They are **reference disagreements, not
demonstrated product-contract violations**. [Diagnosis](oracle-diagnosis.json).

Formal main mismatch indices all belong to those462 cells; F has none. Extension
adds258/252/163 unexplained `late_wrong` replies, all in the first5s of32k;
every stable32k/64k mismatch index maps to the diagnosed set. Mapping an index
does not establish the actual overload reply's content. Timed response payloads
were not saved, so the extra cold/late disagreements' cause is unproven.
[Index-only evidence](oracle-load-index-diagnosis.json). Frozen timed oracle,
counts and gates remain unchanged; no post-result waiver or performance rerun.
External review must decide the reference correction/revalidation needed for a
full semantic acceptance. F latency and RSS independently fail regardless.

## Targeted profiling and resource evidence

F makes29,999 foreign HTTPS exchanges across3 runs, zero outer cache hits.
Native SOCKS accepts8,886/8,885/8,886 vs Go10/10/11. Source
`rust/native-host/src/assembly.rs:911–929` uses fresh exchange on reuse busy
NotSent; `rust/upstream-core/src/reuse.rs:1393` documents single outstanding
exchange. At100QPS with80ms upstream, frequent fresh TLS is consistent with
this path. Software cpu-clock499Hz/60s yields3K samples/lost0: RSA Montgomery
reduction self3.00%, SHA5122.84%, X25519 multiplication2.26%; callgraph reaches
RSA/webpki certificate and TLS1.3 signature verification. This supports a TLS
handshake CPU cause; it does not apportion the exact p99 excess or test HTTP/2.

At8kQPS90s, otherwise matched native diagnostic audit-on/off steadyRSS is
173.07/67.34MiB (difference105.73MiB), snapshot capacity100,000 records.
Final anonymous RSS165.83/56.18MiB, zero swap. Separate audit-on software profile
12K samples/lost0 has memcpy6.70%,malloc3.44%,sort3.39%,free3.10% self samples.
This identifies audit-associated retention and allocation/copy work, not an
allocator leak proof or exhaustive allocation attribution. Audit-off is a
diagnostic only, excluded from budgets. No runtime optimization performed.
Initial hardware-event perf attempts had zero samples despite success exit;
retained and never interpreted. [Profile statistics](profiling.json).

Formal FD traces and three separately built public-API observer runs are in
[resource-recovery.json](resource-recovery.json),
[observer build](observer-build-public.json). Observer runs are excluded from
all performance medians and use a distinct binary. They show query counters
drain, not proof that every background task is absent. Formal pinned binary
does not expose the in-flight counter through its HTTP API. RSS peaks are
250ms samples plus startup observations, not kernel maximum-RSS guarantees.

Native F first startup failed on a Go-style proxy field before any sample;
cleanup/attempt retained. Only the native disposable entry was corrected to
`use_proxy=true` with existing main_proxy; Go F bytes/freeze unchanged. Three
Go F schema rechecks retained, not substituted for the original baseline.
[Rechecks](go-f-recheck-public.json). Initial and formal sender/tool versions
are retained privately; [hashes](environment.json) and matrix continuation
record disclose the correction. No successful formal measurement discarded.

## Stop2 delivery

Overall frozen budget **FAIL**, pending external Claude review.5D-2/3,Phase6,
scaffolding retirement and production replacement remain unperformed.
Tool tests9/9 and E2E helper tests10/10 pass on the designated Linux VM;
py_compile and diff whitespace checks pass. These are implementation tests,
not acceptance/self-review rounds. [Tools](../../../../tools/5d-1/README.md).
Private raw requests/configs/oracles/full probe replies/profiles/build logs and
tool versions are archived outside the repository before VM root deletion.
No production domains/ECS/private credentials enter this record.
Final process/namespace/service/archive receipts: [Stop2 cleanup](stop2-cleanup.json).
Original service active/PID827437 unchanged. No commit, push, deployment, task
archive or next-task work. Stop here awaiting external review.

## 外部审查结论（2026-10-09）

测量方法与证据通过（开环、单核绑定、交错、先冻结后测、阈值未改）。**5D-1 作为测量任务验收；产品对冻结预算为 FAIL**，两项均为真实产品缺陷：
1. DoH 并发：单条串行复用连接忙时每个请求新建一次性 TLS 连接（F 场景约 8,886 次代理连接 vs Go 约 10 次），F p99 三轮均超限、CPU 2.2×；
   修复方向：复用连接上的并发（HTTP/2 多路复用 / HTTP/1.1 有界连接池）。
2. 稳态 RSS 超 128 MiB 绝对上限：审计保留约 106 MiB（容量 100k，与生产一致，约 1 KiB/条）；阈值不放宽，修复方向：压缩审计记录表示。
修复后用 `tools/5d-1` 在同一冻结阈值下重跑受影响场景。462 个单元格属已批准的 `full:` 大小写偏差（后续工具中补全 oracle 批准集）；
冷启动 32k 过载前 5 秒的 late_wrong 包原因未证明，归 5D-2 保留应答内容排查。
