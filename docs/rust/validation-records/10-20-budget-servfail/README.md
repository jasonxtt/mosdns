# 活请求预算耗尽 SERVFAIL 验证统计

验证日期：2026-10-20 至 2026-10-21（Asia/Taipei）。外部审查待进行。

| 验证 | 结果 |
| --- | --- |
| 修复前 policy 回归 | 5 通过、6 失败；预算路径 UDP 无应答 |
| 修复前独立 ECS/TCP 回归 | 1 失败：EOF，无 DNS frame |
| 最终新增测试 | 12 通过，净增 12 个测试 |
| 新增场景 | 48：预算 24、既有结果 16、取消 8 |
| 既有 response policy 集成测试 | 24 通过 |
| VM fmt / 严格 clippy | 均退出 0 |
| VM workspace 全量（含 doctest） | 1519 通过、0 失败、1 ignored |
| ignored 子进程测试 | 1，由父级崩溃恢复测试驱动 |
| 离线内嵌 UI / release 构建 | 退出 0；构建并发数 2 |
| config_rust 检查 | gaps=0、cascades=0、globals=0、definitions=128 |
| rtdns 终止矩阵 | 20/20 通过（5 路径 × 2 引擎 × 2 协议） |
| rtdns ECS 静默上游 | Rust UDP/TCP 2/2 SERVFAIL，约 5001.7/5001.9 ms |
| rtdns Rust 审计 | 10 条：4 SERVFAIL、6 REFUSED、0 NO_RESPONSE、0 selected |
| 真实上游访问 | 0 |
| 隔离 Go/Rust 进程退出码 | 0 / 0 |
| 主服务重启次数变化 | 0 |
| 本任务残留进程 / 远端临时目录 | 0 / 0 |

结果统计不声明响应逐字节一致，也不代表生产替换或性能门禁通过。

## Planner verdict (2026-10-21)
Round 1 `FINAL: PASS`. Found by Shadow-1 (15 real queries went unanswered after an ECS-wrapped upstream timed out). Combined with
the release-check test change on the build VM: fmt, clippy and 1519/0/1. Follow-up (P2): locally synthesized replies should copy RD
(and CD) from the query, as Go's `SetReply` does.
