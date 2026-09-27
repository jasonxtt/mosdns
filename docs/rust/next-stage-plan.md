# Rust-native 下一阶段任务顺序（2026-09-27）

本页是截至 `rust` 分支当前已审查证据的执行顺序，不修改总路线图的 Phase 5A–5D/6 关口。目标仍是 Linux amd64 上纯 Rust-native 完整 MosDNS；优先 DNS 正确性、p95/p99 响应、有效吞吐、多并发和稳定性，内存作为第二指标。每一阶段只按真实证据升级状态，不把 W1/W2/W3 子集称为完整版。

## 当前事实与顺序

W1 UDP/TCP 转发、W2 简单缓存、W3 受限域名/IP 分流和首轮原生进程 Go/Rust 对照已有归档证据。首轮对照只有 12/21 组形成三次有效配对，未找到客观过载点；服务恢复和多核容量不能据此判定。详见 [首轮结果](phase5a-native-comparison.md)。基础观测任务已验收归档：严格 W1/W2/W3 接受 `enable_audit: true`，提供最终查询审计与基础指标；当前运行时为单线程 local task set，这也是后续多核目标必须单独验证的架构限制。

| 顺序 | 独立交付及结束条件 | 依赖和下一步判定 |
| --- | --- | --- |
| 1：5A 基础观测 | 执行 [native query observability 任务](../../.trellis/tasks/archive/2026-09/09-24-rust-phase5a-native-query-observability/prd.md)：W1/W2/W3 每个已受理查询的最终结局、实际路由、缓存状态、基础指标及有界审计；Linux 正确性和观测开销有证据。 | 先让后续性能/功能工作可诊断。完整审计 API/UI 仍在 5C。此任务已获 M10-FINAL-001 PASS，并于 2026-09-26 经授权完成归档；仅关闭基础观测子集。 |
| 2：5A 测量可信度与热点分析 | [measurement reliability 任务](../../.trellis/tasks/archive/2026-09/09-27-rust-phase5a-measurement-reliability/prd.md) 已终止归档，结果为 **incomplete matrix**：工具与校准已交付，W1 仅两个有效 pair；Go r3 健康门槛失败后停止，W2/profiling 未执行。见 [收口报告](phase5a-measurement-reliability.md)。 | 重跑预算耗尽，不补跑、不改阈值；原定 A4/A5 未通过，不形成容量、恢复或热点结论。保留性能证据缺口，进入 5B 功能规划；没有依据先做 runtime/`Send` 改造。 |
| 3：5B 查询功能分批闭环 | 先冻结真实代表配置与 [功能覆盖表](feature-coverage.md) 的缺项，再按依赖切成配置/sequence/provider 与匹配器、上游策略和余下协议/listener、跨插件组合等可独立验收的任务。每批必须有实际 YAML → 最终 DNS/路由的 E2E、故障与性能趋势。 | 不以单插件单测替代组合契约；原生实现不延伸 Go/cgo bridge。某一批若依赖 Phase 4 传输能力，先补该依赖再继续。 |
| 4：5C 管理与状态闭环 | 按 API/文件/运行时/WebUI 数据流交付完整审计与 `/metrics`、`special_groups`、规则下载保存/reload、dump、配置生成及重启恢复，逐项封闭覆盖表。 | 5A 观测的 typed 数据模型在此接入正式接口；保留现有 Vue UI，验证管理动作不阻塞查询。 |
| 5：5D 完整配置整机验收 | 在 5B/5C 全部条目关闭后冻结 SLA/资源预算，做整机负载扫描、长稳、故障注入、热更新与性能剖析优化。 | p95/p99、正确且按时响应吞吐、恢复、CPU/RSS 和长稳均有 Linux amd64 证据，阻塞未决项为零。 |
| 6：Phase 6 迁移脚手架退役 | 先证明原生路径等价，再删 Go/cgo adapter、selector、mirror/fallback 等过渡层，重跑完整 E2E 和发布 gate。 | 只有此后才能考虑默认/生产替换；部署仍遵守测试机与用户确认流程。 |

顺序 1 已验收归档。顺序 2 按 C2C 裁决与用户确认关闭实验，并提交归档收口；生命周期关闭不代表原验收全部 PASS。顺序 3 可以开始规划，首批建议为可组合 YAML/sequence 原生接入：解除固定 W1/W2/W3 配置图限制，复用已有模块，冻结引用、顺序、所选控制流、错误与取消/关闭契约，并用真实 YAML → 最终 DNS/路由验收。具体范围依覆盖表 P44、L01/L02、C01 子集确定，不打包完整 5B；本轮不创建任务或开始实现。容量、恢复、热点和长稳仍须后续验证，5D/Phase 6/生产门禁不变。
