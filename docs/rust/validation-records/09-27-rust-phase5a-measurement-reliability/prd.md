> Historical product planning or evidence snapshot. Lifecycle status alone is not acceptance PASS. See [validation summary](../../validation-summary.md). Raw logs and local workflow state are retained outside Git tracking.

# Rust Phase 5A measurement reliability and hotspot analysis

## Goal

建立可信、可复算的 W1 TCP 转发 / W2 暖缓存 Go–Rust 对照，区分负载器限制、服务退化和热点，为 runtime/Send 改造或继续 5B 功能集成提供依据。交付测量工具与有限实验报告，不要求 Rust 赢过 Go，也不以跑分替代完整功能。

用户于 2026-09-27 初始授权“创建任务，只制定规划，不开始实现”，随后授权使用 c2c 新建对话审查规划；之后已明确授权按冻结 slice 执行，并进一步授权在 `mosdns-rust` 安装 profiler。当前使用 inline 流程，C2C/ reviewer 只审查当前 gate，不派发其他 agent。Slice 1 已执行完成；Slice 2+ 仍须等待 G0 profiler remediation review。

## Confirmed background

- `docs/rust/next-stage-plan.md` 已将“测量可信度与热点分析”排在基础观测之后、5B 功能集成之前。基础观测及归档复验已通过，Git 锚点为 `5478015f7998be5335a7019915af558da5c74b4b`。
- 首轮对照只有 12/21 组形成三次有效配对；2-vCPU、3 秒、最高 1000QPS 不证明容量、服务恢复或多核伸缩。来源：`docs/rust/phase5a-native-comparison.md`。
- 现有 helper 在落后超过两倍发送间隔时丢弃时隙，`scheduled` 不包含这些时隙；W3 还会等待相同 question 的前一请求。来源：`tests/phase5a-baseline/cmd/phase5a-baseline/main.go:3105`、`:3116`、`:3129`。
- 现有 aggregator 要求所有请求正确按期完成，不能用来分析有效负载下的真实超时/迟到。来源：同文件 `:1102`。这属于旧对照方法，不能事后改写旧结果。
- Rust host 使用 current-thread + LocalSet、Rc/spawn_local；它是架构限制，尚未被证实为主瓶颈。来源：`rust/native-host/src/assembly.rs:93`、`:110`、`udp.rs:98`、`tcp.rs:104`。

## Requirements

### R1 — 独立测量契约与兼容

新增有版本的 reliability 入口/schema；保留旧 baseline/official/observability 入口及冻结证据。独立报告证据完整性、负载有效性、DNS 正确性、服务预算、过载和恢复，不能用一个 PASS 掩盖不同结论。

### R2 — 有界开环发送与完整账本

计划时隙独立于响应完成；每个时隙追溯为已启动、调度缺失或负载器自身限额拒绝。记录计划/启动/实际 DNS 发送/完成时间、发送滞后、并发峰值及互斥终态，正确性与有效吞吐可复算。队列、任务、FD 和日志内存有上限，不通过无限 goroutine、关闭 GC、丢时隙或写日志阻塞来隐去发送失败。

请求期限从计划时隙的单调时刻起算，排队/连接/收发共享同一绝对预算；迟到收集只服务于已完整发送的请求，不允许延长建立连接或新发 DNS 的窗口。持续阻塞且不报错的日志 sink 必须单独验证：明确负载器失效、账本守恒和有界退出，不能以 writer-error 测试替代。

返回 DNS 报文继续执行严格 oracle。错包、串包是阻塞；有效施压下的迟到/超时属于服务退化数据，不能整行丢弃。发送端限额或证据不足则不允许判断 SUT 过载。

### R3 — 客观过载及同进程恢复

候选正式测量前冻结按期率/尾延迟技术预算、发送有效性、安全上限、观察窗口与恢复规则。具体技术阈值只由 Go reference pilot、发生器校准及采样误差决定，不能用候选 Rust 数据反向调整。它们不是最终产品 SLA。

W1 连续增载/回落；只有先观测到有效负载下的客观退化才能评价恢复。无过载必须写 `indeterminate-no-overload-evidence`；达到安全上限则报告停止原因和有效下界。重启后的健康检查不能冒充同进程恢复。

### R4 — 小范围、同条件对照

官方场景仅 W1 TCP fresh connection/request 和 W2 UDP 两 key warm-cache，audit 关闭，两端条件一致。W1 UDP/W3 仅执行已有回归，不新增容量矩阵；W2 cold 仅 correctness 回归。

W2 每个负载点独立启动/预热，验证逐 key TTL 安全窗口和上游零增量，不改变历史配置/TTL/语料来延长窗口；独立重启的负载点不能组成恢复证据。两 key 结论不代表完整缓存工作集。

### R5 — Linux 测量与可审查归因

使用已指定的 `ssh mosdns-rust`，执行前盘点 CPU/cgroup/亲和性、内存/FD、隔离和 profiler 能力。优先足够 CPU 隔离；若现有 2 vCPU 的发生器/fixture 校准通过，只允许受限单核对照，不宣称多核容量。新增 host、VM 扩容及本机 VM 不在授权范围。

W1 fresh-TCP 发生器资格必须同时覆盖最高发送速率与整条正式 ladder 的持续时间/累计连接压力，保留临时端口范围、TIME_WAIT/reuse、连接错误及预算余量证据；瞬时速率通过不能代替累计资格。

SUT、发生器及 fixture 分别采样，正式延迟与 profile 运行分开。提供能定位调用栈的热点证据；CPU top stack 不能单独证明 I/O 等待根因。结论分为已证实、证据支持的假设和未知。本任务不修改 Rust 产品实现验证优化收益。

### R6 — 有限实验与归档可复验

保存所有身份、环境、判据、工具/输入哈希、命令、attempt、失败和 PID/start cleanup 回执。原始数据不覆盖；新 summary 在全新目录离线生成，归档形态下仍验证指定历史 Git 对象与输入哈希。

pilot/official/profile 都有预算，正式重跑最多一次。预算耗尽、资源不足或正确性失败就停，保留未通过项，不反复筛样取得漂亮结果，不虚报 completed。

## Acceptance Criteria

| ID | 可观察结果 | Requirements |
| --- | --- | --- |
| A1 | 新入口/schema 与兼容回归通过，各判定轴独立，旧报告不被新标准重解释。 | R1 |
| A2 | 调度停顿、慢/丢响应、日志阻塞、自身限额等负例中逐时隙账本守恒；发生器失效不被判为服务过载；上限可执行。 | R2 |
| A3 | 公共 CLI/离线入口覆盖过载、恢复、无过载、未恢复、重启和缺证据；候选前已有冻结判据及参考数据。 | R3 |
| A4 | 每场景二至四个共同负载点，每点至少三次预排定 Go/Rust 有效配对，否则明确未通过；严格 response、TCP 策略、W2 TTL/零增量及身份成立。 | R4 |
| A5 | Linux 环境和发生器校准通过，正式结果、独立 profile、资源曲线及瓶颈分类齐备；未知容量只报告下界，无过载不声称恢复。 | R3/R5 |
| A6 | 所有尝试与进程退出回执保留；raw → summary 一致，完整归档入口及篡改/缺失/歧义负例通过；full-scope review 与下一步建议完成。 | R1/R6 |

正确报告 indeterminate 可以满足 A5 的有限报告要求，但不能替代 A2/A4 的有效发送与对照证据。环境、配对或 profiler 不足时对应验收项保持未完成。

## Out of scope

不迁移 runtime/Send，不加 listener 线程池、Rust 过载策略或连接池；如测量支持这些工作，单独规划。新协议/provider/plugin 归 Phase 4/5B，API/WebUI/Prometheus/dump 归 5C。完整配置、整机容量、多核伸缩、长稳归 5D；不退役 hybrid、不发布/部署生产，不访问 `ssh mos` 采集生产配置。首先使用可重建的已冻结 Go-only 来源。

## Planning gate

目标和范围已明确，无阻塞规划的产品决策。远端实际资源、profiler 权限、精确扫描上限属于 Slice 0–2 的执行前技术门禁，当前未测、未冻结。本轮只写规划并检查一致性，不执行 task.py start、构建、SSH 或查询流量。

## Notes

- 主规划顺序不变；本任务只关闭“测量可信度与热点分析”的受限范围。
- 归档证据和无关 dirty changes 保留；Trellis auto-commit 继续关闭。
