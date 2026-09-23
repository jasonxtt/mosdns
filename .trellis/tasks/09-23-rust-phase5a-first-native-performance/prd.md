# Rust Phase 5A — first native whole-process comparison

Status: **in progress — Slice 0 PASS; Slice 1 manifest v1 review found and rejected a mutable-digest gap. Replacement manifest v2 pins its reviewed digest; regression and 24-tuple validation pass, awaiting Slice 1 review before official samples.** The user authorized execution after the planning review passed. No official measurements have started; production deployment is outside scope.

## Goal

在指定的 `ssh mosdns-rust` Linux amd64 VM 上，以相同 W1/W2/W3 配置、请求和受控上游，对独立 Go-only 与 Rust-native 进程做第一次**正确性优先**的可复现性能对比。交付冻结清单、全部有效/无效测量、对照报告和有证据的下一步建议。本任务不以预设百分比宣称 Rust 胜出，也不代表完整 MosDNS 验收。

## Starting evidence and comparator

- 已归档 Go-only 基线：`.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/`，报告 `docs/rust/phase5a-go-baseline.md`。复用其 W1 UDP/TCP、W2 cold/warm、W3 配置/语料和正确性 oracle。历史 5/10/20 QPS 是另一台 4 CPU/6 GiB QEMU 环境的低负载证据，不能作为本 VM 的 Go 对照或容量上限。
- 主要 Go 对照应在本 VM 从已冻结 Go-only 源码 `5b1eca69e0668ad1ddb6db88c0f39202557d5b98` 重建；如无法重建或不能接受同一语料，正式运行前记录原因、review 并冻结替代 Go commit。不得使用 hybrid/cgo selector 充当 Go-only。
- Rust 对照用测量前由 review 确认的精确 `rust` 分支 commit、`rust/native-host` 独立 release binary；不得经过 Go host、cgo bridge 或 `MOSDNS_*_BACKEND` selector。两边须接受**同一未修改**的测试配置/语料；如有不兼容，先转入缺陷处理，不修改语料掩盖差异。
- 2026-09-23 只读现况：`ssh mosdns-rust` 是 Linux x86_64、2 CPU、约 4 GiB RAM；`ssh mos` 运行中 Go binary 为 `/usr/local/bin/mosdns`，SHA-256 `b46d779d4c45ade5de80b1d2d85b24ca10ca13d07bfe84c1fa2a41dd3484e3a3`；启动配置 `/cus/mosdns/config_custom.yaml` 的 SHA-256 为 `ce0b4d857e7371d42021738a7eba6715e1ac9407257fdbc1bbb8a826329f211a`。生产配置使用 `include`；这些只用于现场版本/场景参考，尚不证明它与冻结 Go 源码相同。
- `mos` 上的文件只作只读参考；不可在那里压测、重启、替换文件，或把原始配置、密钥、日志、客户域名/地址复制入仓库。若后续确需获取 binary/config，放受控临时位置、记录哈希、先脱敏；本轮主要比较不依赖复制。

## Scope

包括：在测试 VM 的隔离端口同机顺序运行 Go/Rust；W1 UDP/TCP 最小转发（含预期负响应）、W2 UDP 冷未命中/显式预热后热命中、W3 UDP 域名/IP 规则路由；复用 `tests/phase5a-baseline/**` 和 `scripts/run-phase5a-baseline.sh`；预检、短 pilot、冻结新 manifest、至少三次交错重复和聚合报告；记录正确性、p50/p95/p99、correct-on-time 有效吞吐、错误/超时/sender shortfall、CPU/有效查询、RSS、FD、上游计数与机器/构建身份。原始冻结输入和归档证据只读。

不包括：生产 `mos` 负载/部署；新增或优化产品实现；DoT/DoH/DoQ/DoH3、完整插件/API/WebUI/持久化、公网 DNS、长期 soak、真实生产全配置；以历史 QEMU 数值和当前 VM 直接作胜负判断；Phase 5D/6 或生产切换声明。若测量发现产品缺陷，另行修复、review、重新冻结候选。

## Requirements

### R1. Fair and traceable builds

记录双方 commit、binary SHA-256、工具链、依赖锁文件哈希、构建命令/flags、`GOOS/GOARCH/CGO_ENABLED`、Rust target/profile、`GOMAXPROCS/GOGC/GOMEMLIMIT`、CPU/FD 限制及运行环境。Go 是 Go-only release，Rust 是 native-host release。双方在同一 VM 顺序运行，测试配置、数据、缓存容量/TTL、日志/审计设置和上游语义一致；不等价的行不能合并判定。

### R2. Correctness hard gate

每个场景先对 Go 与 Rust 分别运行相同的未修改 smoke cases。共享 oracle 校验完整 DNS 响应：response bit、request ID、匹配 opcode、单个 echoed question（name/type/class）、预期 rcode、无意外 truncation、精确预期 answer set；预期负响应只有在 rcode 正确且 answer 为空时才算正确。两端都执行相同期限、W2 cold 每个唯一 key 一次 miss / warm 零增量和 W3 必须/禁止 route legs 检查。错误、错配、迟到、超时或传输失败不可计入有效吞吐。任何未解释的语义差异都暂停该场景性能结论，不得为 Rust 放宽 oracle。

### R3. Freeze a new VM manifest before official runs

短 pilot 仅检验发生器/fixture 余量并选择阶梯。正式样本前冻结新 manifest：源/binary/config/workload/helper 哈希，启动命令，场景和 Go/Rust 交错顺序、QPS 阶梯、stage 时长、期限、warmup/prefill、TCP 连接策略、缓存/日志/审计设置、CPU 亲和性、带事件时间的 W3 证据格式、stage 边界和 `recovery_assessment_mode=indeterminate-no-overload-evidence`。SHA-256 必须由 official runner 校验。变更需新版本并双边重跑；不得覆写历史证据。

### R4. Valid load on a 2-CPU VM

固定速率发送与响应速度解耦；短 pilot 后冻结正常参考、常用负载、接近饱和、高速探测和同进程末尾健康检查的阶梯，不在规划时伪造 QPS/容量。每个有效点至少三次，Go/Rust 交错先后；保留全部无效/失败尝试。各阶段在同一 SUT PID、同一 fixture session 内连续运行，中途不重启或重置。归档 fixture 的 A 应答 TTL 为 30 秒，正式样本前冻结每个 warm key 的预填时间和 TTL 安全余量；W2 warm 的最后一个测量响应必须在对应预填后、TTL 到期前完成。若完整连续序列无法在 TTL 安全余量内完成，可按 manifest 使用每阶段独立预填的 warm 点，但不得把它称为同进程恢复，也不得在中途重填缓存。由于 pilot 未建立稳定过载，本任务 manifest 冻结 `recovery_assessment_mode=indeterminate-no-overload-evidence`：末尾同速率阶段只作 post-sequence health check，所有 service-recovery 结论保持 indeterminate，即使阶段检查通过也不宣称“已恢复”。后续若需要 service-recovery 结论，须在新 manifest/新任务中预先定义并验证客观过载证据规则。冻结的样本数和 p95/p99 参考带只用于末尾健康检查判据。记录发生器/fixture CPU、计划/实际发送和 headroom。可试 SUT 单核、发生器加 fixtures 另一核的公平**单核 SUT**对比，须证实发生端有余量。若共机干扰或发送不足，只报告有效点与不确定性，不宣称多核扩展或容量上限；真正多核结论需额外负载机或更大且可隔离的主机。

### R5. Metrics and honest interpretation

每 stage 保存 offered/scheduled/sent/received/correct/correct-on-time、预期负响应、错误答案/协议/传输错误、超时及 shortfall；p50/p95/p99 注明样本量和统计对象，超时单列且不可静默排除。记录 user+system CPU、CPU/有效查询、稳定/峰值 RSS、FD、上游计数。W3 每个有效请求必须通过 fixture 事件的 `occurred_at` 落入客户端请求 `sent_at`→`finished_at` 区间进行逐请求关联，并精确核对 leg 数与顺序；同 question tuple 单飞用于消除边界歧义，上游 DNS ID 仅作诊断。W2 independent-prefilled 结果必须按生命周期读取对应 stage 文件、传播 prefill/TTL 无效原因，并将 recovery 标记为 indeterminate。此任务所有末尾同速率点都只报告 health-check 指标；service-recovery 结论按冻结模式保持 indeterminate。大量失败时不能挑快速成功响应宣称低 p99。对比所有重复的分布/噪声；无预先冻结业务 SLA 时，本轮提供曲线和回归线索，不制造最终性能 PASS 百分比。差异在波动内写持平/不确定；功能缺陷优先于性能；W1/W2/W3 不可推广为全量产品。

## Acceptance criteria

- [ ] 双方 Linux amd64 独立 binary 的来源、构建、运行条件及 SHA-256 可追溯；七个固定语料哈希与归档一致；生产机无变更。
- [ ] 原样 W1/W2/W3 配置/语料的双边 smoke 通过共享严格 DNS oracle、W2 counter oracle、逐请求 W3 route-event oracle（每条路径精确 leg 数及顺序）、进程清理和端口回收；未通过者留缺陷证据且无性能结论。
- [ ] Pilot 与发生器/fixture headroom 获 review；本 VM 新 manifest 在正式运行前冻结，official runner 每次校验 manifest 和全部固定输入哈希，并验证 harness/SUT 实际 CPU 亲和性。
- [ ] 每个同进程末尾 health-check 点都有同一进程的阶段身份、冻结样本/延迟判据和完整结果；本任务冻结的 service-recovery 结论一律为 indeterminate，因为未冻结客观过载触发条件。W2 warm 还须证明每个测量响应都在对应 key 的 30 秒 fixture TTL 与安全余量内完成；独立预填点不得称为同进程恢复。
- [ ] 各有效场景/负载点至少三次交错 Go/Rust 重复，全部无效尝试保留；条件不足者明确标记原因。
- [ ] 报告有逐 stage 的正确性、尾延迟、有效吞吐、CPU/RSS/FD、样本量、上游计数、波动、原始证据索引和复现命令。
- [ ] Reviewer 明确只接受首轮 W1/W2/W3 子集结论，不把本任务写成 Phase 5A 全部完成或生产放行。

## Execution boundary

任务最初以 `planning` 建立并经独立规划 review。用户随后授权执行，执行对话已用 `task.py start` 将任务置为 `in_progress`。Slice 0 的每次工具/方法修订都须绑定精确提交并复审；官方 manifest 只在 Slice 0 reviewer PASS 后冻结。条件不足时保留证据并报告受限结果，不强行填满矩阵。
