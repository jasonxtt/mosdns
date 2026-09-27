# Source audit — 2026-09-27

本轮仅本地只读研究，未 SSH、未构建、未测量。source anchor `5478015f7998be5335a7019915af558da5c74b4b`，branch=rust，已有 dirty 路径均非本任务改动。

| Source | Finding / consequence |
| --- | --- |
| docs/rust/next-stage-plan.md order2 | 观测后做测量可信度/热点，条件决定 runtime/Send，随后 5B。 |
| docs/rust/phase5a-native-comparison.md | 12/21 有效组、2vCPU、3s，没有过载/恢复/多核依据；不重解释旧结果。 |
| helper main.go:3105–3148 | interval 时隙、lag>2interval skip、Scheduled 不含 skip、W3 同 question 等待、每请求 goroutine。新方法要 planned 账本、有界 worker、lag；W3 高负载移出本轮。 |
| helper main.go:1102–1126 | 旧 paired invalid 判断全成功；effectiveQPS 使用 DurationMS。复用分母但不能删真实过载失败。 |
| helper executeRequest/requestRecord | SentAt 在 exchange 前，Sent 使用 exchange 实际 sent 布尔；不能把字段名直接当 on-wire 时间。 |
| helper main.go:3275–3277, :3857–3862 | exchange 接收 deadline+lateDrain；TCP dial 后从当前时刻重新设置完整 I/O timeout，逻辑预算可被分段延长。新 reliability 使用 planned-at 单调绝对期限，legacy 保留原行为。 |
| helper main.go:2982–2993, :3233 | 原 ledger write 为同步 mutex/buffer write 且在请求完成路径调用；新入口必须真实覆盖 sink 阻塞而不返回错误，不只 writer-error。 |
| helper main.go:3857–3861 | 每请求 dial/close，W1 连续 ladder 有累计端口/TIME_WAIT 压力；校准需覆盖峰值、整段时长及连接总量。 |
| helper main.go:2361 | 恢复主动 indeterminate；旧 health-check 不是恢复。新版本必须有客观前序过载。 |
| helper main_test.go | 已有 sender 拒绝、采样、DNS、TTL、route/manifest 测试，继续复用公共 CLI。 |
| scripts/run-phase5a-baseline.sh | legacy/official/m3/m4 与 helper version 限制；新入口独立窄范围，不翻修所有历史 profiles。 |
| native assembly.rs:93–110; udp.rs:98; tcp.rs:104 | current-thread/LocalSet/Rc/spawn_local 已成立，主瓶颈未被证实。 |
| native observer.rs | typed snapshots 未接正式 HTTP；本任务用 external proc/stack，不借压测加管理 API。 |
| archived observability repo_paths.py / run-m10-w3.py | marker root、指定历史对象/hash、82 项归档回归已通过；继承契约，不动冻结工具。 |
| .trellis/config.yaml:33 | session_auto_commit=false；新 task 默认 base_branch=main 已纠正为 rust。 |

复用 fixed workloads、严格 oracle、TTL/counter 和 proc sampling；精确 worker 数、QPS/lag/资源上限、host 布局与 profiler 由执行前门禁冻结。本轮不引入新依赖、不假设机器吞吐，不授权扩容。

## Primary references checked on 2026-09-27

- Linux kernel [proc documentation](https://docs.kernel.org/filesystems/proc.html)：进程/线程信息来源；保留采样分辨率、缺口与资源字段解释。
- Linux kernel [perf access control](https://docs.kernel.org/admin-guide/perf-security.html)：事件、权限及采样资源限制；先做 preflight，不能假设 VM 有 PMU，不自动降低全局权限。

CPU profile 用于定位热点；等待/调度根因还需要受控证据。这些是方法依据，不是已经收集到的 Linux 结果。
