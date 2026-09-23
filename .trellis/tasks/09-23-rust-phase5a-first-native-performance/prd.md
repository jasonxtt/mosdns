# Rust Phase 5A — first native whole-process comparison

Status: **planning only**. The user will choose another conversation to execute. Planning/review does not authorize official benchmark runs or deployment.

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

每个场景先双边 smoke。校验请求 ID、qname/qtype、rcode、答案/路由类别和期限；合法负响应可计正确，错误、串包、超时、传输失败不可算有效吞吐。W2 校验冷/热 upstream 增量，W3 校验 route leg/order。任何未解释语义差异都暂停该场景性能结论，不能用宽松 Rust oracle。

### R3. Freeze a new VM manifest before official runs

短 pilot 仅检验发生器/fixture 余量并选择阶梯。正式样本前冻结新 manifest：源/binary/config/workload 哈希，启动命令，场景和 Go/Rust 交错顺序、QPS 阶梯、stage 时长、期限、warmup/prefill、TCP 连接策略、缓存/日志/审计设置、CPU 亲和性。SHA-256 必须由 official runner 校验。变更需新版本并双边重跑；不得覆写历史证据。

### R4. Valid load on a 2-CPU VM

固定速率发送与响应速度解耦；短 pilot 后冻结低负载→常用负载→接近饱和→过载→恢复点，不能在规划时伪造 QPS/容量。每个有效点至少三次，Go/Rust 交错先后；保留全部无效/失败尝试。记录发生器/fixture CPU、计划/实际发送和 headroom。可试 SUT 单核、发生器加 fixtures 另一核的公平**单核 SUT**对比，须证实发生端有余量。若共机干扰或发送不足，只报告有效低负载与不确定性，不宣称多核扩展或容量上限；真正多核结论需额外负载机或更大且可隔离的主机。

### R5. Metrics and honest interpretation

每 stage 保存 offered/scheduled/sent/received/correct/correct-on-time、预期负响应、错误答案/协议/传输错误、超时及 shortfall；p50/p95/p99 注明样本量和统计对象，超时单列且不可静默排除。记录 user+system CPU、CPU/有效查询、稳定/峰值 RSS、FD、上游计数。过载和恢复分开报告；大量失败时不能挑快速成功响应宣称低 p99。对比所有重复的分布/噪声；无预先冻结业务 SLA 时，本轮提供曲线和回归线索，不制造最终性能 PASS 百分比。差异在波动内写持平/不确定；功能缺陷优先于性能；W1/W2/W3 不可推广为全量产品。

## Acceptance criteria

- [ ] 双方 Linux amd64 独立 binary 的来源、构建、运行条件及 SHA-256 可追溯；生产机无变更。
- [ ] 原样 W1/W2/W3 配置/语料的双边 smoke、W2/W3 oracle、进程清理/端口回收通过；未通过者留缺陷证据且无性能结论。
- [ ] Pilot 与发生器/fixture headroom 获 review；本 VM 新 manifest 在正式运行前冻结，每次 official run 核验哈希。
- [ ] 各有效场景/负载点至少三次交错 Go/Rust 重复，全部无效尝试保留；条件不足者明确标记原因。
- [ ] 报告有逐 stage 的正确性、尾延迟、有效吞吐、CPU/RSS/FD、样本量、上游计数、波动、原始证据索引和复现命令。
- [ ] Reviewer 明确只接受首轮 W1/W2/W3 子集结论，不把本任务写成 Phase 5A 全部完成或生产放行。

## Execution boundary

规划提交后保持 `planning`。由用户另选执行对话；执行者先读 `design.md`、`implement.md`、`.trellis/workflow.md` 和仓库规范，做独立规划 review，再 `task.py start`。条件不足时保留证据并报告受限结果，不强行填满矩阵。
