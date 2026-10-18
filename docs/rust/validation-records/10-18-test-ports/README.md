# 测试端口竞争修复（2026-10-18）

## 外部审查第 2 轮：仅 F1

第 1 轮要求修复的失败回滚端口检查，改为读取 Linux 临时端口范围，从下界以下随机选择两个 TCP/UDP 均空闲的不同端口。只改测试 helper 和 `m1_0_config_loading::a_failed_bind_releases_the_listeners_that_already_bound` 的 a/b 来源，两个释放断言原样保留，无断言重试。其它失败后释放测试的探针已覆盖对应协议，未扩用；F3a 超时未改。

procfs 不可读/解析失败，或下界 ≤1026（不足两个非特权端口）时回退 TCP bind(0)+同号 UDP 探测，不能保证消除 bind(0) 抢占。VM 范围为 `32768 60999`，本轮实际使用 1025–32767，未触发回退。

VM offline/locked/-j2 的最终 fmt、clippy、全量均退出 0，**1491 passed / 0 failed / 1 ignored**，数量不变。随后同样 2 lane、每 lane 每文件 15 次、默认测试线程，`f1_system` **0/30 失败**，`m1_0_config_loading` **0/30 失败**；保留全部 60 次原始日志，没有重跑覆盖。第 1 轮对应结果分别 0/30、1/30，下面原始记录保留。F3a 的第 1 轮确认超时留给规划者，本轮不宣称解决。

本地/VM 最终测试源码 SHA256 一致；初次 clippy 转换告警与格式换行诊断及修正记录保留。VM 任务目录及归档已清除，目录所属进程 0，原服务前后 `MainPID=827437 / ActiveState=active / NRestarts=1` 不变。原始证据已归档本机仓库外，见本地任务 validation.md。未提交/部署，无自我审查轮次，等待外部第 2 轮结论。

## 第 1 轮原始记录（保留）

BASE `b9f4615e4d8c56691e639e8920f1487c99f1e302`，只改测试与测试辅助代码。没有产品代码、断言阈值或测试数量变更；等待外部 Claude 审查，没有运行自我审查轮次。完整 a/b/c 盘点在本地任务 `research/design.md`。

## 处理

- `tests/common/ports.rs`：启动最多 5 次；只对实际启动错误中的 `Address already in use`/`AddrInUse` 重新分配端口。其它错误立即失败，耗尽保留每次错误。启动失败先 kill/wait，不能让旧子进程占着端口进入下一次。
- 接入 F1、F2、F3a 初次启动、cache HTTP 的 SIGTERM 场景、配置加载日志场景、basename hosting。F1 保存新 WebUI 端口后的启动同样有界重试。原有 Text-file-busy 的专门 spawn 重试保留。
- F1/F2/F3a 的 upstream stub 直接持有 bind(0) 得到的 UDP socket，再传线程；不再探号后重新绑定。原本故意占端口的日志失败测试直接持有 TCP bind(0)。
- `upstream-core/tests/slice2_doh3.rs`：TCP probe 与 QUIC 共用端口号时，先持有 UDP socket，匹配 TCP（仅 AddrInUse、最多 5 次），把该 UDP socket 直接 move 给另一运行时的 `Endpoint::new`。
- 同进程/同 HostAssembly runtime 的 b 类只盘点；原来就持 socket 的 c 类保留。故意占端口、更新失败与回滚、原端口重启的断言不由通用重试接管。

## 方法与临时故障注入

所有构建/测试仅 VM `mosdns-rust`，offline、Cargo `-j2`、关闭 dev/test debug info 与 incremental；VM 缓存 Rust 1.95.0（不是 GitHub 的 1.96.0）。原 VM 服务只读比较身份，不连接生产/mos-test、不访问公网、不提交或部署。

固定压力批次由 `tools/test-ports/stress.py` 执行：F1/F2/F3a/m1_0_config_loading 各 30 次，2 个并发进程 lane，每 lane 每文件 15 次；每个进程默认测试线程并发，不绑单核、不把失败重跑覆盖。每个 invocation 保存退出码/耗时/原始输出，批次开始/结束核对测试二进制 hash。修前后使用独立的源码和可执行文件。

另在独立 VM 测试源码副本给 F1 上传测试的初次 child 启动加入真实 DNS 端口占用，注入不进入最终源码、不增加 `#[test]`：

| 输入 | 结果 |
| --- | --- |
| 修前，第一次占用 DNS 端口 | 失败，1 次启动，退出 101 |
| 修后，第一次占用 DNS 端口 | 成功，2 次启动，退出 0 |
| 修后，每次占用 DNS 端口 | 失败，严格 5 次启动后退出 101 |
| 修后，非地址类的坏 YAML | 立即失败，1 次启动，退出 101 |

以上预期失败验证的是重试边界，不能计入正常套件的失败数。修前自然压力没有失败也必须如实写 0，不能把注入结果叫作自然复现。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| BASE 全量 | 1491 passed / 0 failed / 1 ignored，退出0 |
| 最终源码 fmt / clippy（workspace/all-targets/-D warnings） | 均退出0 |
| 最终源码完整套件 | **1491 passed / 0 failed / 1 ignored**，退出0，与BASE相同 |
| Rust改动范围与身份 | 7个测试文件，测试名/数量不变；9份Rust测试/共享辅助源码本地与VM最终检查逐字节相同；没有改src产品代码 |

| 文件 | 修前失败/调用 | 修后失败/调用 |
| --- | --- | --- |
| f1_system | 0/30 | 0/30 |
| f2_diagnostics | 0/30 | 0/30 |
| f3a_update | **1/30** | **1/30** |
| m1_0_config_loading | 0/30 | **1/30** |

压力批次修前退出1、修后退出1，不能称“压力全绿”。没有为了取通过结果重跑替换这两个固定批次。

- 修前F3a：lane2/第1次，94.219s，`a_configuration_update_from_a_url_goes_through_the_guard_and_stays` → `timed out: confirmed`。
- 修后F3a：lane1/第7次，97.678s，`a_full_update_replaces_program_and_config_in_the_same_process_and_is_confirmed` → `timed out: the update is confirmed`。两次均没有AddrInUse证据，**未归因**；没有把更新确认断言改为重试或加长超时。
- 修后配置加载：lane1/第5次，`a_failed_bind_releases_the_listeners_that_already_bound` 在 `earlier UDP socket released` 断言遇到errno98/AddrInUse。此处是设计盘点中的**b类释放后重新绑定验证**，不在a类child初次启动重试内。不能凭这个错误区分外部抢占与真实未释放；未改这条断言、未给它加重试。

待外部审查决定：上述b类断言是否另行隔离、F3a确认超时是否另立调查。当前只完成a类处理与规定的验证，不宣称所有端口竞争或所有压力失败都已消除。

## 清理与交接

本任务临时进程、源码/构建/注入副本及VM目录结束时清除；原服务前后 `MainPID=827437 / ActiveState=active / NRestarts=1` 不变。原始日志保存在本机仓库外，详情见本地任务 `research/validation.md`。没有连接生产/mos-test，没有提交、push或部署。交接文件为 `research/review-request.md`；等待外部Claude结论。

## Planner verdict (2026-10-19)
Round 1: one test-only change requested (release checks must use ports outside the ephemeral range). Round 2 `FINAL: PASS`. The occasional
f3a "confirmed" timeout under 2-lane stress (1/30 both before and after) is unrelated to ports. It is tracked in the Shadow-1 online-update event.
