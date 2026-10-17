# 10-17 5D-2c：UDP 接收缓冲、热应用停顿说明、CPU 触发项

任务 `10-17-rust-native-5d-2c-rcvbuf`，基线 `fa9f2fa5` + 本任务工作树。构建、测试、测量在 VM `mosdns-rust`（Linux x86_64，2 vCPU，`net.core.rmem_max = 4194304`）上进行，本机不构建 Rust。
生产快照只在 VM 私有目录使用（本记录不含域名、配置内容或凭据），不访问公网，原有 `mosdns` 服务运行前后只读核对（所有运行的 `cleanup.json` 均为 `original_service_unchanged: true`）。
测量输入在本任务内按 5D-1/5D-2b 流水线重新准备（只读生产快照 → Go oracle → 原生 oracle）。设计见任务的 `research/design.md`。

## 实现

`rust/native-host/src/bind.rs`：三条 UDP 绑定路径（IPv6 通配双栈、其它地址、IPv6 不可用时的 IPv4 回退）统一用 `socket2` 创建（后两条原先直接用 `tokio::net::UdpSocket::bind`），
创建后调整接收缓冲再交给 tokio。行为不变：UDP 仍不设 `SO_REUSEADDR`（地址冲突仍是 `AddrInUse`，有测试），双栈与回退的逻辑和告警文字不变。TCP 不动。

- 目标 `UDP_RECEIVE_BUFFER_TARGET = 4 MiB`（`getsockopt` 的有效值，Linux 会把请求值翻倍）：先请求 2 MiB（生产 `rmem_max = 2 MiB` 无需特权即可得到 4 MiB），
  若有效值仍低于目标则尝试 `SO_RCVBUFFORCE`（`rustix` 的安全封装，`rustix` 增加了 `net` feature，无新 crate），任一步失败只告警、保留系统给出的值，**绝不使启动失败**。
- 依据：5D-2b 实测默认有效缓冲 208 KiB 约容纳 170 个小包，500 qps × 0.65 s ≈ 325 个包溢出；4 MiB 约容纳 3 400 个包，是该突发的 10 倍。
- 有效值可见：启动时每个 UDP 监听一条 info 日志（不足目标则 warn，含原因）；`GET /api/v1/system/health` 新增 `udp_receive_buffers: [{listen, bytes}]`（读不到为 `null`，不影响 `ready`），
  由 `RuntimeControl` 在监听绑定时登记、监听结束时移除。VM 上实测 10 个 UDP 监听（含 53）全部为 **4 194 304 字节**。
- 内存：套接字缓冲由内核按需分配，不计入进程 RSS；10 个监听最坏占用 40 MiB 内核内存，仅在积压时发生。

### 测试
- 单元（`bind.rs`）：假内核覆盖 “`rmem_max` 足够（不强制）”“低于目标且有权限（强制到目标）”“低于目标无权限（保留受限值并告警）”“请求失败/读取失败（不报错）”；
  真实套接字覆盖三条路径（`[::]`、`127.0.0.1`、IPv4 回退的创建函数）有效值 ≥ 本机可达值，且重复绑定同一 UDP 地址仍为 `AddrInUse`。
- 集成（`m1_e_standard_groups.rs`）：健康文档含 `udp_receive_buffers` 且字节数为正；创建专用端口组后出现该端口的条目，删除后消失。

## VM 验证
| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 见下方“最终运行” |
| `mosdns check -d config_rust -c config_custom.yaml` | `SUMMARY gaps=0 cascades=0 globals=0 definitions=128`（与基线相同） |

## H1/H3 复测（500 qps，原生 ×3，5D-2b 同一方法与工具）
测了两批：第一批用实现完成时的二进制；自查后给 `UdpServer` 加了 `Drop`（让被丢弃的监听从 health 里消失，不在 DNS 路径上）并重建，第二批用**最终二进制**重跑（`mosdns-native-final`）。两批都各 6 次运行。

| | 5D-2b（基线 `f8bfbb2a`） | 5D-2c 第一批 | 5D-2c 最终二进制 |
| --- | --- | --- | --- |
| H1 丢失 | 130 / 42 / 70 | **0 / 0 / 0** | **0 / 0 / 0** |
| H3 丢失 | 79 / 47 / 34 | **0 / 0 / 0** | **0 / 0 / 0** |
| 53 端口 socket 内核丢包 / 命名空间 `RcvbufErrors`（全程） | 等于丢失数 | **0 / 0** | **0 / 0** |
| 错误答案 | 0 | 0 | 0 |
| 管理 API 调用耗时（≈ DNS 停顿） | 0.49–0.66 s（H1） | H1 0.42–0.58 s；H3 0.44–0.62 s | H1 0.43–0.63 s；H3 0.45–0.61 s |
| 停顿窗口内 P99 / 最大延迟 | 385–473 ms / 474–603 ms（H1） | H1 338–423 / 474–569 ms；H3 332–434 / 434–593 ms | H1 357–477 / 478–619 ms；H3 344–486 / 445–670 ms |
| 窗口内晚于 500 ms 的比例 | 0.1–1.9% | 0–0.5% | 0–0.9% |

两批里 H1 的新代理/新列表探测都全部正确（17892/tcp、还原后 17891/https），H3 的专用端口随保存出现、随删除消失。RSS 峰值：第一批 108–117 MiB（H1）、111–113 MiB（H3）；最终二进制 131–132 MiB（H1）、113–127 MiB（H3）。
停顿本身没有消失：窗口内仍有最多约 1% 的查询晚于 500 ms（`every_window_healthy` 的 ≥99.9% 判定在若干窗口里没有通过——这是延迟，不是丢失；最终二进制 6 次里仅 H3 第 1 次通过）。两批之间的差异在运行间波动范围内，没有单独的因果解释。
根治（编译移出 DNS 线程）是 M2-3。“丢失为 0”的验收标准已满足（12 次运行全部 0）。

## 20/100/500 qps 冻结延迟门槛复核（原生与 Go 各 ×3，种子 501/502/503，G/N、N/G、G/N，原冻结文件与 5D-1 发送器/比较代码不变）
（本节与下面的 CPU 部分用第一批二进制测的；之后的 `Drop` 改动只在监听被丢弃时清理 health 登记，不在 DNS 路径上，因此没有重测。）
冻结文件 SHA256 `afe441e9…8c70a9` 校验通过。`c_matrix.py --gate`：每次 warm-up 10 s + 20/100/500 qps 各 30 s + 100 qps 恢复 60 s。

| 档位 | 原生 P95 / P99 中位数（ms） | 冻结上限 P95 / P99（ms） | 结果 |
| --- | --- | --- | --- |
| 20 qps | 11.998 / 12.711 | 14.025 / 15.168 | 通过 |
| 100 qps | 11.892 / 12.805 | 13.853 / 15.006 | 通过 |
| 500 qps | 11.015 / 11.977 | 12.333 / 13.866 | 通过 |

每档三次的正确率与覆盖率均 ≥99.9%，0 个错误答案（预言一致）；稳态 RSS 42–48 MiB（远低于 128 MiB 与同输入 Go），采样峰值 88.6–94.9 MiB（<256 MiB）；恢复段 5/5/5 s（≤10 s）。
每次正确应答的 CPU：20 qps 原生/Go 比值 1.08、100 qps 1.09、500 qps 1.11，未触发 profiling。容量（≥2000 qps）本次没有重测（只跑了三个延迟门槛档位），沿用 5D-2b 的结论。缓冲变大没有劣化延迟。

## CPU 触发项（5D-2b：扩展组 warm-up 有一次重复 CPU > Go × 1.5）
- **是否过载档位：** 不是。冻结规则（`frozen-thresholds.json`）是“任何**非过载**档位的原生重复 > Go 三次中位数 × 1.5 须 profiling”；`budget.py` 的 `normal` 为该档有延迟上限，或各重复均“可持续”（延迟、正确率、覆盖率合格）。
  触发的是 100 qps、10 s、1000 个查询的 warm-up：它在冻结延迟表里没有上限，但 5D-2b 里 `sustainable_each_repeat=True`，负载远低于容量，因此按定义属于非过载档位，不能免除。
- **重新测量（冻结口径）：** `tools/5d-2/warmup_profile.py` 在隔离环境里运行**未改的 5D-1 发送器**（`load.measure`：warm-up 100 qps、10 s 后接一个 3 s 的 20 qps 段和 1 s 恢复段），
  取 warm-up 阶段的 `cpu_us_per_correct`——与 `budget.py` 完全相同的算法（跳过开头 1.67 s 的稳定窗口内 CPU 增量 ÷ 稳定窗口正确应答数 833）。Go ×5、原生 ×5（最终二进制）：
  Go 168/180/180/192/192（中位数 180 µs），原生 204/204/216/216/228（中位数 216，比值 1.20；最大重复/Go 中位数 = 1.27）。门槛矩阵（同一发送器、同一算法）里的 warm-up：Go 180/204/216，原生 252/216/264（最大/Go 中位数 = 1.29）。
  **八次原生测量（5 + 门槛矩阵 3）里没有任何一次超过 1.5 倍，5D-2b 那一次（>1.5×）没有复现。** 5D-2b 的该次原始数据已按规程删除，不能回头查那一次的具体值与成因。
  （本工具的第一个版本用另一个发送器和更宽的窗口算出了类似的比值，因口径与冻结算法不同已废弃，上面的数字全部来自冻结口径。）
- **profile（原生，`perf record -e cpu-clock -F 999 --call-graph dwarf`，每次约 260–280 个样本；Go 二进制按 5D-1 的构建是 stripped 的，没有可比 profile）：**
  用户态约 53–60%、内核 24–25%、libc 16–19%，没有单一热点。已证明（包含式占比，范围来自两次带调用链的 profile）：UDP `sendto` 8–12%、上游交换 14–19%、`ExecutionMachine::drive` 9–11%、`epoll_wait` 5–6%、
  每次上游调用新建/绑定的临时 UDP socket（`socket` 3–6% + `bind` 1–2%）、观测记录 `AdmittedQueryGuard::finish` 7–10%（其中对结果的稳定排序 `driftsort` 约 2.5–5%）。
  **假设（未证明）：** 与 Go 的约 20% 差距主要来自每次上游调用新建 socket 的系统调用与逐查询观测记录开销；没有 Go 的 profile，不能量化各项对差距的贡献。不做优化，作为后续优化线索上报。
- **结论：** 不免除；profiling 已做；warm-up 平均比 Go 高约 20%，未超过冻结的 1.5 倍触发线；没有发现需要立即修复的缺陷。

## 最终运行
同一份最终工作树上：`cargo fmt --all -- --check` 退出码 0；`cargo clippy --workspace --all-targets -- -D warnings` 退出码 0；
`cargo test --workspace --no-fail-fast` **1491 passed，0 failed，4 ignored**（基线 1486，新增 5 个 `bind` 单元测试；集成测试里的两处断言并入既有测试）；`check` 如上。
说明：加 `Drop` 之后的第一次完整运行有 3 个测试以 `Address already in use` 失败（`m1_f1_typed_settings`、`special_groups_supervisor`、`special_groups_whole_chain` 各一个，测试里“先取空闲端口再绑定”的既有竞争形态，
与缓冲改动无关的证据是：这三个测试文件单独连跑 3 次全部通过，随后的完整重跑 1491 passed / 0 failed）。上面的数字是该次重跑的结果；失败的那次运行日志保留在 VM 任务目录，任务结束随目录删除。
工具测试（`tools/5d-2`）见任务 `research/validation.md`。

## 停顿说明
契约已更新：`native-webui-hosting.md`（新增 “UDP receive buffer and apply pause” 一节与 health 字段）、`native-settings.md`（Hot apply）、`native-special-groups.md`（Save pause）：
保存设置或专用组时 DNS 约停顿 0.4–0.65 s（本复测的 API 调用耗时 0.42–0.62 s，生产规则集），期间的 UDP 查询在监听的 4 MiB 内核缓冲中排队，缓冲容量内不丢失；根治为 M2-3。

## 规划会话核对（2026-10-17）

Codex（gpt-6.1-sol）第 2 轮 FINAL: PASS；规划会话核对后验收。H1/H3 在 500 qps 下 12 次运行 0 丢失、0 内核丢包；停顿延迟（约 0.4–0.65 s）仍在，
根治归 M2-3。CPU 触发项按冻结口径复测未复现（原生/Go 中位数比 1.20，8 次均 < 1.5×），profiling 线索（每次上游调用新建 UDP socket、`sendto`、
观测记录收尾）记为后续优化候选，不阻塞。`Drop` 修复后延迟门槛与 CPU 未重测（不在 DNS 路径），容量本次未重测，均由 5D-3 覆盖。
