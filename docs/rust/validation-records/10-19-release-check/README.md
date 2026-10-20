# 失败回滚后的 socket 释放检查（2026-10-20–21）

BASE：`8abf3192ccb0e2433330f66b76c1c46debf99bab`，分支 `rust`。本任务只修改三个测试/测试辅助文件；不改产品代码、依赖、绑定失败条件或永久测试数量。

## 改动

- `rust/native-host/tests/common/ports.rs`：Linux 按 UDP/TCP 分别读取 `/proc/net/{udp,udp6,tcp,tcp6}` 的本地端口和 inode，再与 `/proc/self/fd/*` 的 `socket:[inode]` 取交集。只把本进程持有的 fd 算作未释放；其它进程占用或没有本进程 fd 的 TIME_WAIT 不算。读取/解析失败直接 panic，枚举期间消失的 fd 仅忽略 NotFound。
- `m1_0_config_loading::a_failed_bind_releases_the_listeners_that_already_bound`：在 bind_host 前用真实 UDP/TCP socket 正向自检，回滚后断言本进程不再持有对应 socket。b 直接持有 TCP bind(0)，a 使用现有双协议探针；删除不再需要的随机选端口 helper。
- `slice6_management_http` 的 `a_failed_management_bind_releases_the_dns_listener`、`a_failed_dns_bind_releases_the_management_listener`：分别加入 UDP/TCP 正向自检并使用同一释放断言。
- 非 Linux 编译原有回环重新 bind 检查。`getrandom` 仍被产品代码使用，保留依赖。其它正常关闭后的重新 bind 测试保持原样。

## 验证

所有构建、测试仅 SSH 别名 `mosdns-rust`；Cargo offline、locked、`-j2`，Rust 1.95.0，关闭 dev/test debug info 与 incremental。源码由 BASE 独立快照叠加上述三个文件组成，未混入另一并行任务的产品改动；最终三个文件本地/VM SHA256 一致。

| 检查 | 结果 |
| --- | --- |
| BASE 全量 | 1507 passed / 0 failed / 1 ignored |
| 最终 cargo fmt --check | 退出 0 |
| 最终 cargo clippy --workspace --all-targets --locked -j2 -- -D warnings | 退出 0 |
| 最终 cargo test --workspace --no-fail-fast --locked -j2 | **1507 passed / 0 failed / 1 ignored**，与 BASE 一致 |
| 配置加载目标文件 | 18 passed |
| 管理 HTTP 目标文件 | 16 passed |
| 配置加载压力批次 | **0/30 失败** |
| 管理 HTTP 压力批次 | **0/30 失败** |

每个压力文件固定 2 lane × 15 次，每次进程内部保持默认测试线程；保留全部 60 次日志、退出码及二进制 hash，没有失败重跑替换。

VM 临时补丁（全部恢复，未进入最终源码）：

| 注入 | 结果 |
| --- | --- |
| 回滚后 Python 子进程占用 a 的 UDP 端口，握手确认后持续持有至断言结束 | 测试通过，退出 0；子进程关闭并 wait 回收 |
| 回滚后本进程持有 a 的 IPv4 UDP / TCP socket | 两项均在释放断言失败，退出 101 |
| 把 detector 的成功匹配临时改成 false | bind_host 前正向自检失败，退出 101 |
| 正向自检 socket 改绑 ::1 | UDP6/TCP6 均检出，测试退出 0 |
| 回滚后本进程持有 a 的 IPv6 UDP / TCP socket | 两项均在释放断言失败，退出 101 |
| 临时切换 cfg，在 Linux 上编译运行非 Linux 回退分支 | 三个回滚测试通过；不是原生非 Linux 平台验证 |

注入证明断言可区分本进程持有和外部占用，不表示复现了 HostAssembly 产品泄漏。Linux 依赖可读有效的 procfs（含 IPv6 表），扫描不是原子快照；同一测试进程内其它线程持有同协议/端口仍计作本进程持有。非 Linux 保留原 rebind 检查的外部占用限制。不宣称消除了初始探号到 bind 的全部竞争。

## 交接

未进行任何自我审查轮次，等待外部模型审查。没有提交、push、部署或连接生产/mos-test，没有访问公网。原始日志、临时补丁脚本、源码 diff/hash 与审查请求保存在本地可选 Trellis 任务的 research 中。

原始证据已归档本地；全部命令/压力进程均结束，清理前按 cwd、exe、命令参数检查本任务目录所属进程为 0，VM `/tmp/release-check-20261020` 已删除。清理前后原 mosdns 服务均为 `MainPID=827437 / NRestarts=1 / ActiveState=active`，启动时间 `2026-10-07 07:57:54 UTC` 一致。未触碰另一任务的目录或进程。BASE/最终 1508 个测试名及其出现次数也已比对一致（含 1 ignored）。

## Planner verdict (2026-10-21)
Round 1 `FINAL: PASS`. Combined with the budget-SERVFAIL change on the build VM: fmt, clippy and 1519/0/1.
