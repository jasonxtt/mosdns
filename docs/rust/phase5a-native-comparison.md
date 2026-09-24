# Phase 5A 首轮 Rust-native W1/W2/W3 对照结果

日期：2026-09-24
状态：官方矩阵和结果整理已完成，最终独立复审为 PASS；归档路径修正经窄范围复审为 PASS。结果只覆盖本报告列出的有效配对点。

## 结论

在 mosdns-rust 的 2 vCPU Linux amd64 VM 上，以 SUT 固定在 CPU 0、发生器和 fixtures 固定在 CPU 1，对 Go-only 与 Rust-native 进程完成了 24 次交错候选尝试。矩阵覆盖 W1 UDP/TCP、W2 cold/warm、W3 路由；每阶段持续 3 秒。21 个场景/阶段组中，12 组有三次有效配对，9 组因发生器 shortfall 或末尾 health-check 判据失败而少于三次。

有效配对点上，两端都达到冻结的 offered QPS，没有错误、迟到或超时响应。Rust 的 W1 TCP p95 在 200、800、1000 QPS 三个三次有效配对点中，每次都较低；W2 warm 200 QPS 与 cold 200 QPS 也各有三次较低的 Rust p95。其余点的配对差异有正有负或范围跨零。本轮只有三次重复，没有预先冻结业务性能 SLA，故不作整体性能胜负判定。

CPU、RSS 和 FD 取样支持这台 VM 上的**有条件单核对照**。CPU 时间来自 100 Hz 的进程 tick 计数，量化较粗；Rust 有 4 个有效 SUT 阶段只观察到 0 tick，不能把它们写成零 CPU。最高有效点是 1000 QPS 的 3 秒阶段，不是容量上限或多核吞吐结论。服务恢复评估按冻结方法保持 indeterminate-no-overload-evidence；末尾阶段只作为 health check。

## 冻结方案与构建身份

- VM：Linux amd64，2 vCPU；SUT 在 CPU 0，runner、load generator 和 fixtures 固定在 CPU 1。W1 TCP 使用每请求新连接。每阶段 3 秒，deadline 500 ms，late drain 100 ms。
- Go-only 源码：commit 5b1eca69e0668ad1ddb6db88c0f39202557d5b98；binary SHA-256 2cd2274b70e258e2ff6065ecf70f4d65d28b4049d7b41a2a01ed1225763cc069。
- Rust-native 源码：commit 605c30577b79d397b5695618dbd2980e550ca6f3；binary SHA-256 370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa。
- Helper：phase5a-baseline-helper/v8，binary SHA-256 df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065。
- 官方 manifest v2 SHA-256 3d8b76e8799bf709ef7e19df9edd6245936f05f9c4e09230dd49ecdaaf000f13。matrix driver SHA-256 d7cd9cc786afcf343a0546a1ffe97883a064a39dd3686769128fc1077dcb8c18。二者与审查通过的冻结版本一致；7 个固定配置/语料哈希与归档基线一致。
- 每场景 3 次 Go/Rust 交错。执行前检查冻结哈希、目标输出目录不存在、10 个 benchmark 端口空闲。执行命令：

    taskset --cpu-list 1 bash /root/mosdns-rust-phase5a-first-native-performance-605c305/evidence-official-v2/run-official-matrix-v2.sh --execute

完整 manifest、构建身份、driver transcript 和执行状态表见本任务的 [manifest v2](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/official-manifest-v2.json)、[构建身份](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-build-identities-v1.txt)、[driver transcript](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-driver-console.log) 和 [24 次尝试状态](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-attempt-status.tsv)。

## 正确性与阶段覆盖

共享严格 DNS oracle 检查 response bit、ID/opcode/question、rcode、answer set、截断和期限。W1 有效阶段的 correct-on-time 响应数与发送数一致；负响应也满足预期 rcode 和空 answer。W2 的 6 组 Go/Rust 运行分别通过 cold miss、warm prefill、warm 零增量和逐 key TTL 检查。每个缓存 key 的预填到最后响应最大间隔为 18.374 秒，低于 30 秒 TTL 减 500 ms 安全余量的 29.5 秒上限。

W3 的 6 组 Go/Rust 运行共完成 30 个阶段的逐请求 route-event 校验及 6 次完整 event-journal 尾部校验。所有已发送请求都匹配到唯一请求时间区间内的事件，并满足精确路由顺序：DOMAIN_HIT=[route-a]、IP_RULE_HIT=[route-b,route-a]、IP_RULE_MISS=[route-b,route-c]。W3 共发送并正确按期完成 46,798 个请求，对应 77,996 个 route legs；两个 1000 QPS overload 尝试各少一个调度时隙，对应 29,996 个 route legs，因此配对不完整，但已发送请求的 route-event oracle 仍通过。

### 上游计数交叉核对

下列数字是对应场景/阶段的六次 Go/Rust 候选尝试合计，保留 sender-shortfall 和 health-check 无效阶段。W1 每一行的正/负响应 upstream counter 增量之和都等于实际 sent 数；W2 六次候选尝试各自 cold 和 warm-prefill 均对两个 key 各产生一次 miss，五个 warm 测量阶段的每 key 增量均为零。Warm 命中由严格响应 oracle 加上游零增量确认；本轮没有独立的内部 cache-hit counter 读数。W3 每个阶段的逐请求预期 route legs 与带序号事件日志中的实际 route legs 按 upstream 和 qname 计数完全一致；每请求的顺序仍以逐请求 oracle 的结果为准。

| 场景 / 阶段 | 实际 sent | upstream counter 增量或 route-a/b/c legs |
|---|---:|---:|
| W1 UDP / normal-reference | 3,600 | forward 正/负：2,400 / 1,200 |
| W1 UDP / common-load | 7,200 | forward 正/负：4,800 / 2,400 |
| W1 UDP / near-saturation | 14,399 | forward 正/负：9,599 / 4,800 |
| W1 UDP / overload | 17,999 | forward 正/负：12,000 / 5,999 |
| W1 UDP / recovery health-check | 3,600 | forward 正/负：2,400 / 1,200 |
| W1 TCP / normal-reference | 3,600 | forward 正/负：2,400 / 1,200 |
| W1 TCP / common-load | 7,200 | forward 正/负：4,800 / 2,400 |
| W1 TCP / near-saturation | 14,400 | forward 正/负：9,600 / 4,800 |
| W1 TCP / overload | 18,000 | forward 正/负：12,000 / 6,000 |
| W1 TCP / recovery health-check | 3,600 | forward 正/负：2,400 / 1,200 |
| W2 cold / official-w2-cold | 3,600 | cache-a/cache-b misses：6 / 6 |
| W2 warm / warm-prefill | 12 | cache-a/cache-b misses：6 / 6 |
| W2 warm / normal-reference | 3,600 | cache-a/cache-b misses：0 / 0 |
| W2 warm / common-load | 7,200 | cache-a/cache-b misses：0 / 0 |
| W2 warm / near-saturation | 14,399 | cache-a/cache-b misses：0 / 0 |
| W2 warm / overload | 17,999 | cache-a/cache-b misses：0 / 0 |
| W2 warm / recovery health-check | 3,600 | cache-a/cache-b misses：0 / 0 |
| W3 / normal-reference | 3,600 | route-a/b/c：2,400 / 2,400 / 1,200 |
| W3 / common-load | 7,200 | route-a/b/c：4,800 / 4,800 / 2,400 |
| W3 / near-saturation | 14,400 | route-a/b/c：9,600 / 9,600 / 4,800 |
| W3 / overload | 17,998 | route-a/b/c：11,999 / 11,998 / 5,999 |
| W3 / recovery health-check | 3,600 | route-a/b/c：2,400 / 2,400 / 1,200 |

完整的 [132 行 upstream observation 表](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-upstream-observations.tsv)包含每候选、每重复、每阶段的实际计数、W3 预期/观测 leg 对照、无效原因、原始文件清单及逐行源证据 SHA-256。只读[汇总脚本](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/summarize-official-v2-upstreams.py)会先复验 718 个冻结原始文件；结果索引 SHA-256 为 `7dd7597fc175efa9f124ed62fccf4fee32aeaf365d90afe3684b1feb3b017ea8`，脚本 SHA-256 为 `35d5d96109f99ef463f7a386e4fd979cbdac12a94c725468c78f05d0c66ed6d4`，观察表 SHA-256 为 `6e04b326f3237e468178a86c9ad2528df97e1670adcffa8880a7fbd508896aeb`。远端导出命令为：

    python3 /root/mosdns-rust-phase5a-first-native-performance-605c305/evidence-official-v2/summarize-official-v2-upstreams.py --results-root /root/mosdns-rust-phase5a-first-native-performance-605c305/results/official-v2 --result-index /root/mosdns-rust-phase5a-first-native-performance-605c305/evidence-official-v2/official-results-index.sha256 --output /root/mosdns-rust-phase5a-first-native-performance-605c305/evidence-official-v2/upstream-observations.tsv

下表列出每个场景/阶段组的有效 Go/Rust 配对数。recovery 行只说明冻结的末尾 health-check 是否满足样本数及 p95/p99 判据；所有服务恢复结论仍为 indeterminate。

| 场景 | 阶段（目标 QPS） | 有效配对 | 少于三次的原因 |
|---|---|---:|---|
| W1 UDP | normal-reference (200) | 3/3 | — |
| W1 UDP | common-load (400) | 3/3 | — |
| W1 UDP | near-saturation (800) | 2/3 | r3 Go sender shortfall |
| W1 UDP | overload (1000) | 2/3 | r1 Rust sender shortfall |
| W1 UDP | recovery health-check (200) | 0/3 | health-check 判据失败 |
| W1 TCP | normal-reference (200) | 3/3 | — |
| W1 TCP | common-load (400) | 3/3 | — |
| W1 TCP | near-saturation (800) | 3/3 | — |
| W1 TCP | overload (1000) | 3/3 | — |
| W1 TCP | recovery health-check (200) | 0/3 | health-check 判据失败 |
| W2 cold | official-w2-cold (200) | 3/3 | — |
| W2 warm | normal-reference (200) | 3/3 | — |
| W2 warm | common-load (400) | 3/3 | — |
| W2 warm | near-saturation (800) | 2/3 | r2 Rust sender shortfall |
| W2 warm | overload (1000) | 2/3 | r1 Rust sender shortfall |
| W2 warm | recovery health-check (200) | 1/3 | 两次 health-check 判据失败 |
| W3 | normal-reference (200) | 3/3 | — |
| W3 | common-load (400) | 3/3 | — |
| W3 | near-saturation (800) | 3/3 | — |
| W3 | overload (1000) | 1/3 | r1 Rust、r3 Go 各有一个 sender shortfall |
| W3 | recovery health-check (200) | 1/3 | 两次 health-check 判据失败 |

实际保留全部 24 次尝试，其中 15 次 runner exit 为非零。非零状态由六个 sender-shortfall 阶段实例和 15 个 recovery health-check 阶段实例解释；同一尝试可能同时包含两种阶段失败。无效阶段未被删除或重跑，其他阶段按冻结的 stage-scoped 规则独立判断。

## 有效配对点的延迟和吞吐

每个延迟值为三次有效重复的 median [min–max]，单位为 µs；每次样本数为 600、1200、2400 或 3000，具体见逐次记录。配对差值为每次 Rust p95 − Go p95 的 median [min–max]。有效配对点上，双方 correct-on-time 有效吞吐都等于目标 QPS。

| 场景 / 阶段（目标 QPS） | Go p50 / p95 / p99 | Rust p50 / p95 / p99 | 配对 Δp95 |
|---|---|---|---|
| W1 UDP / normal-reference (200) | 100 [96–130] / 238 [230–301] / 367 [299–402] | 91 [85–98] / 245 [232–258] / 314 [313–334] | -6 [-43–15] |
| W1 UDP / common-load (400) | 79 [72–111] / 182 [179–248] / 333 [321–358] | 78 [71–101] / 208 [150–221] / 344 [269–354] | +29 [-98–39] |
| W1 TCP / normal-reference (200) | 271 [227–282] / 486 [412–519] / 654 [564–720] | 194 [191–209] / 326 [323–367] / 480 [425–515] | -152 [-160–-89] |
| W1 TCP / common-load (400) | 197 [194–304] / 323 [320–480] / 474 [466–690] | 209 [171–221] / 315 [302–368] / 447 [412–491] | -21 [-165–48] |
| W1 TCP / near-saturation (800) | 204 [180–250] / 303 [279–386] / 457 [440–570] | 183 [161–194] / 289 [265–319] / 396 [362–538] | -14 [-67–-14] |
| W1 TCP / overload (1000) | 220 [201–231] / 385 [331–391] / 507 [416–617] | 158 [132–190] / 259 [220–293] / 374 [298–459] | -126 [-171–-38] |
| W2 warm / normal-reference (200) | 89 [60–102] / 196 [156–213] / 254 [218–277] | 77 [49–93] / 165 [122–185] / 211 [170–220] | -34 [-48–-11] |
| W2 cold / official-w2-cold (200) | 73 [71–116] / 217 [176–224] / 366 [259–378] | 54 [51–74] / 132 [129–170] / 178 [171–223] | -85 [-95–-6] |
| W2 warm / common-load (400) | 54 [44–108] / 119 [103–205] / 193 [163–335] | 73 [38–88] / 130 [89–154] / 182 [174–225] | -30 [-51–27] |
| W3 / normal-reference (200) | 166 [165–230] / 393 [376–429] / 525 [463–576] | 201 [162–213] / 389 [337–398] / 504 [487–571] | -4 [-92–22] |
| W3 / common-load (400) | 165 [145–208] / 347 [330–405] / 608 [534–612] | 152 [143–283] / 313 [307–483] / 464 [448–623] | -17 [-98–136] |
| W3 / near-saturation (800) | 152 [143–182] / 314 [305–355] / 461 [451–469] | 141 [140–226] / 285 [281–439] / 407 [407–696] | -20 [-74–125] |

每次有效配对的原始 p50/p95/p99、样本数、correct-on-time 数、有效吞吐、错误计数、runner exit 和配对有效性都在 [126 行 stage observation 表](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-stage-observations.tsv)。每行保留一方的一次尝试；相同 repetition 的 Go/Rust 行即可复算逐次配对差值。每个 600 样本行的 p99 只由尾端少量观测决定；三次重复范围比单一汇总数更能显示本轮不确定性。

## 资源观察

以下统计只包括上表 12 个三次有效配对组。每个候选有 36 个 SUT stage 资源观察；RSS 为每个 3 秒采样窗口中的峰值，再对这些窗口取中位数和范围。

| 候选 | SUT 单核 CPU% 中位数（范围） | 非零 tick CPU / correct-on-time 中位数（范围） | SUT RSS 窗口峰值 MiB 中位数（范围） | FD 窗口峰值范围 |
|---|---:|---:|---:|---:|
| Go | 2.331% (0.666–14.318%) | 66.7 [16.7–208.3] µs/query，36/36 个非零窗口 | 38.66 (31.95–55.41) | 8–11 |
| Rust | 1.665% (0–8.326%；4/36 无 tick) | 50.0 [16.7–108.3] µs/query，32/36 个非零窗口 | 3.11 (3.03–3.23) | 7–8 |

/proc CPU 时钟为 100 ticks/s，一个 tick 是 10 ms；一个 tick 对应每阶段 0.333 个单核 CPU 百分点，以及按该阶段响应数折算的 3.3–16.7 µs/query 上界。Rust 的 4 个 0-tick 窗口只支持“小于一个 tick”（低于 0.333% 单核）的上界，不是 0 CPU。表中的 CPU/query 中位数只统计非零 tick 窗口；这个分辨率不足以据此排名 CPU/query。

有效三次配对组中，load generator 与全部 fixtures 的合计 CPU 峰值为 harness 单核的 15.979%（最高样本在 W3 near-saturation 800 QPS）。同时，六个候选尝试仍出现 sender shortfall；因此这个观测只说明有效样本中未见 harness 接近单核饱和，不能抹去 shortfall 或保证所有尝试均有足够发送余量。完整角色和逐阶段资源见 [438 行 resource observation 表](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-resource-observations.tsv)。

## 结果整理、无效尝试和复核

原始结果树位于：

    /root/mosdns-rust-phase5a-first-native-performance-605c305/results/official-v2

原始树有 718 个文件、83 MiB。结果索引有 718 条目且全部复验通过，索引文件 SHA-256 为 7dd7597fc175efa9f124ed62fccf4fee32aeaf365d90afe3684b1feb3b017ea8。driver transcript SHA-256 为 ca5f0741eae74718629f5c487410ed2de5c4943651b9fc666c7e93fcdee6d1fa。24 行候选状态表和无效阶段清单均保留。

首次直接调用 paired aggregator 时，它因每个原始 attempt 目录没有 attempt-exit-status.txt，而把阶段判为 missing attempt exit status。矩阵 driver 已在原始结果根目录记录 attempt status。该初始汇总被保留为 rejected，SHA-256 e8493b5d470fe36e727832d5ff9c577abfa03c05ea94de62dbaef661ac1f0143，不用于结论。随后从根状态表生成带 manifest、driver 和源状态哈希的映射，在 raw tree 外创建只读 symlink analysis overlay，并只在 overlay 写入 aggregator 所需的 per-attempt exit-status sidecar。原始 718 个结果文件没有更改。

最终 reconciled paired aggregation SHA-256 为 e2079495f10848a19d783a39ad06072a6240618feb77904fcf949d0b29c4d6ba；status map SHA-256 为 1416ac51bfe262128d2c7224249824c0104639696229a2705fa9a090a6f557cb。stage/resource observation 表分别为 3a78e9ace9ff59430af5ecd13b77111da4e7b7bdc3246e04322ecfa95c7ed21d 和 0ae00f044581e34c025378910abea602156a030de076cb2b11cc5f271df7d517。overlay 的三项回归测试通过，覆盖 24 次状态映射、原始结果不变、拒绝错误 driver hash 和拒绝不同 schedule。

复核明细见 [W2/W3 raw-result correctness audit](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-correctness-audit.md)、[reconciled aggregate](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-paired-aggregation-reconciled.json)、[rejected aggregate](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-paired-aggregation-rejected.json) 和 [status map](../../.trellis/tasks/archive/2026-09/09-23-rust-phase5a-first-native-performance/research/slice1-official-v2-attempt-exit-status-map.tsv)。

运行结束后，718/718 原始索引仍通过；没有任务候选/helper 进程，10 个 benchmark 端口均无 listener。测试 VM 上既有的 mosdns 服务仍在运行，本任务没有操作它；生产 mos 没有压测或改动。

## 限制与建议

- 只比较固定测试配置和受控 loopback upstream 下的 W1 UDP/TCP、W2 cache、W3 routing 子集；不代表完整 MosDNS 兼容性、生产配置、长时间稳定性或发布门槛。
- 末尾 recovery 点的冻结模式是 indeterminate；没有客观过载触发证据，本轮不能证明服务恢复。800/1000 QPS 是短阶段的冻结标签，未证明饱和点或容量上限。
- 本 VM 只支持条件式单核对照。多核吞吐需要独立负载机或更大且可隔离的测试主机。
- 官方矩阵没有采集 profiling 数据。虽然部分 W1 TCP/W2 p95 差异方向一致，但三次重复的端到端结果不能定位代码热点；事后添加 profiler 会成为新的测量活动，不能回填为本次官方样本。后续应先单独冻结一项小型 profile 任务，在 W1 TCP 和 W2 基线中确认热点，再决定是否做窄范围优化，并保留严格 DNS/W2/W3 oracle 与延迟、CPU、RSS 回归门槛。

本报告和证据支持首轮受限对照结论；它不表示 Phase 5A 全部完成、服务恢复通过、多核性能达标、生产可用或允许部署。
