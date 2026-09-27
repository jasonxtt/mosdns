# Phase 5A measurement reliability — closed, incomplete matrix

收口日期：2026-09-27。任务生命周期已关闭，实验结论为 **incomplete matrix**，不代表原定 A1–A6 全部验收通过。证据位于 [归档任务](../../.trellis/tasks/archive/2026-09/09-27-rust-phase5a-measurement-reliability/)。

## 交付与停止原因

已交付有版本的 reliability sender/assessment 入口、有界逐时隙账本、期限及日志阻塞负例、Linux 校准和 Go-only pilot。G1 工具范围及后续 health-basis 修复有独立 C2C PASS；这些 PASS 不等于正式矩阵或热点验收通过。

official-r3 的 W1-TCP 顺序为 Go/Rust、Rust/Go、Go/Rust。前四个 attempt 构成两个有效 pair。Go r3 完成五个阶段，但末段 dispatch-to-finish p95/p99 为 1231/2478µs，对应冻结门槛 1200/2500µs；仅 p95 越界 31µs（约 2.6%）。该 attempt 保留为 invalid，配对 Rust r3 未启动。W2 正式矩阵、独立 profiling 均未执行。

原始 [失败记录](../../.trellis/tasks/archive/2026-09/09-27-rust-phase5a-measurement-reliability/research/official-r3-control-failure.md) 和 [C2C 停止裁决](../../.trellis/tasks/archive/2026-09/09-27-rust-phase5a-measurement-reliability/research/c2c-official-r3-control-review.md) 已在 `b4c0edb64b17a6648e4e951254326c6a7f3fffa3` 推送。official-r2 后的一次 reviewed retry 已由 r3 消耗，按冻结预算停止，不改阈值、不继续 W2、不追加正式重跑。

阶段名 `recovery` 的末段是回落后的健康检查。各 attempt 的 service-recovery-assessment 明确写为 `indeterminate-no-overload-evidence`；本轮冻结 ladder 未提供客观过载证据判据。因此健康门槛失败不能升级为已证明的服务恢复失败，也不能据此归因 Rust runtime。两对 W1 数据仅作有限观察，不产生容量、稳定胜负、多核伸缩或热点结论。

## 原验收项状态

| 验收项 | 收口状态 | 边界 |
| --- | --- | --- |
| A1 | 已有 G1 工具范围验收 | 新入口/schema、分类及兼容回归；不扩展为全部产品验收。 |
| A2 | 已有 G1 工具范围验收及 W1 校准证据 | 有界 sender、账本、期限/阻塞/限额负例；不补称 W2 正式对照完成。 |
| A3 | 部分完成 | CLI/离线状态机及冻结健康判据有证据；正式实验没有客观过载/恢复结论。 |
| A4 | 未完成 | W1 仅两对，W2 未启动；不能用 pilot 或不同 revision 补足三对。 |
| A5 | 未完成 | 环境/校准有证据，独立 profile、热点分类和完整正式报告条件不足。 |
| A6 | 部分完成 | 原始证据和终止裁决保留；本次验证归档文件一致性及 r3 hash，不声称原计划的完整 CLI fresh derivation、全部 PID/start 退出回执和 full-scope 验收已通过。 |

`task.json` 的 `completed` 是已有 Trellis archive 的生命周期字段；`meta.closure.outcome=closed_incomplete_matrix` 区分实验结果。未完成项保持未完成，本报告不回写冻结门槛或原始测量。

## 本次收口复核

- 归档副本与收口前 HEAD 的原任务 717 个 tracked 文件逐一比较：无遗漏，原始文件字节保持一致；生命周期/收口文档的新增说明单独记录。
- official-r3 W1-TCP sidecar SHA-256 为 `c8460f6e80e55fe65995a24709fce3ad4539299f225dec98b6f20c1da44e90a4`；其 216 个文件（38,741,339 字节）全部校验通过。
- 本轮仅做本地文档、归档和 Git 收口；未 SSH、未产生查询流量、未重新验证线上 PID，也未修改产品代码。生产 PID 425 未触碰的事实来自既有执行记录，本轮不伪造新的远端退出证明。

## 5B 进入判定

可以进入 5B 的分批功能规划。5A 已有真实 W1/W2/W3 native 查询 E2E、基础观测和有限 Go/Rust 对照；这次矩阵不足是性能证据缺口，尚无证据证明它阻塞新增功能的正确性。容量、恢复、热点及长稳债务仍由后续代表性链路测量和 5D 承接，生产/默认切换门禁不变。

建议第一批为 **可组合 YAML/sequence 原生接入**：当前 `rust/native-host/src/config.rs` 仅接受固定 W1/W2/W3 图形，先解除该配置组合限制，复用已有 sequence-core、cache、matcher、forward，冻结命名引用、执行顺序、错误定位及所选控制流的异步语义。映射覆盖表 P44、L01/L02、C01 的明确子集；P02/P26/P34 仅复用已验收能力，不称完整插件迁移。

下一批的小目标是：至少两个不同组合的真实 YAML 经同一 Rust host 返回预期 DNS/路由，错误配置在打开 listener 前拒绝，网络等待后的控制流、取消和关闭回收有证据，既有 W1/W2/W3 保持通过。具体语法/插件子集及测试在下一任务规划时冻结；本次不建任务、不授权实现，不先做通用插件框架、线程池或 Rc→Arc 全局改造。
