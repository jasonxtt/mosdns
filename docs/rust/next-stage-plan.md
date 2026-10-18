# Rust-native 下一阶段安排（2026-09-28）

## 2026-10-18：F-3b 首次 Go→Rust 切换与回滚（实施与 VM 验证，待外部 Claude 审查）

新增 `scripts/cutover/{go-to-rust.sh,rust-to-go.sh,cutover.py}`：Go schema-4 受管原文校验、显式逐文件放行差异、保留用户数据、typed-settings 迁移、原生增量包状态、空间/inode 预检、停服最终快照、原子程序/整根替换、健康失败自动回滚与中断后的显式恢复。
切换 worker 独立于 SSH 会话，回滚保存 Rust 阶段数据；不自动清理现场文件。不改 Rust 源码、不构建本机程序、不连接生产或 mos-test、不发布。操作见 [cutover.md](cutover.md)，逐项结果与失败过程见 [F-3b 记录](validation-records/10-18-f3b/README.md)。本任务不做自我审查轮次，外部审查请求在本地任务 `research/review-request.md`。

## 2026-10-18：Phase 6b 退役 `native_management`（待 Codex 审查结论与规划会话确认）

删除托管模式的全部代码：开关解析、`ManagedProfile`/候选 profile、`ManagedStore` 事务日志与 staging/retired/lock、`compile_managed_candidate`/`apply_candidate` 及 `apply_config` 里的托管分支、托管版 special-groups / upstream / diversion / 手工规则 HTTP 处理、仅为它存在的测试与示例；共享的 `blocking_io` 工作线程、`CandidateInputSet`、`sha256`、`lexical_path`、`FileCommit` 提交路径与标准模式（M1-e special_groups、typed settings）原样保留。配置里写 `native_management` 现在加载/`check`/`start` 都明确失败（`native_management was removed; use the standard special_groups configuration`）；`webinfo/native_management.*` 残留文件不读不删。同时去掉不再成立的 `check` 输出行 `SKIPPED managed-store …`。清单、测试数量变化与验证见 [validation-records/10-18-p6b](validation-records/10-18-p6b/README.md)。

## 2026-10-18：Phase 6a `rust` 分支变成纯 Rust 项目（待 Codex 审查结论与规划会话确认）

删除 Go 源码（`coremain/ plugin/ pkg/ mlog/ main.go go.mod go.sum`、`tools/*.go`）、Go 构建与发布文件和工作流、`rust/runtime` 及全部混合脚手架、`/log` 源码；内置 UI 产物迁到 `rust/native-host/ui/`；Go 参照工具改由 `tools/go-reference/build.sh` 从 `../mosdns` 固定提交 `75f26677` 构建。只删除与迁移，Rust 行为不变；`native_management` 留给 6b。清单、验证与测试数量变化见 [validation-records/10-18-p6a](validation-records/10-18-p6a/README.md)。

## 2026-10-18：F-3a 原生在线更新、自动回滚与配置包管理实现完成，待 Codex 审查结论与规划会话确认

只认 `jasonxtt/mosdns` 的 `rust-v*` release；检查/状态/下载（程序、清单、`config_rust` 配置包）全部 sha256 校验、大小与时间有界；预检（候选自报版本、新配置 `check`）在**动服务之前**完成。
替换走事务：守护 = 旧程序 `exec` 进来的**同一 PID**（systemd 不察觉），候选是它的子进程，健康（连续 3 次 `ready` + 版本 + 配置 schema）才确认；候选崩溃/不健康/超时 → 秒级自动回滚旧程序与旧受管配置，不依赖 `Restart=always`/`RestartSec=2min`；
守护被强杀或断电 → 启动恢复回滚（实测最坏 = `RestartSec` + 0.5 s）。配置包 schema `1`、ID `rust-config-schema-1`，状态文件 `webinfo/rust_config_update_state.json`（不碰 Go 的）；`config/export`、`config/update_from_url` 与 Go 路由一致但只作用于本机配置根并走同一事务。
VM 实测（systemd `Restart=always`/`RestartSec=120`）：升级 DNS 两次各约 0.2 s 的静默、失败回滚一次约 1.2 s、进程号不变、`NRestarts` 不变。记录见 [validation-records/10-18-f3a](validation-records/10-18-f3a/README.md)，契约 [native-update.md](contracts/native-update.md)。首次从 Go 切换的部署脚本（F-3b）与发布（打 tag、传产物）仍待做。

## 2026-10-18：F-2b 日志抓取的逐步流程实现完成，待 Codex 审查结论与规划会话确认

抓取期间引擎在每个有名字的 matcher 求值后、每条规则执行动作前，写出与 Go `chain.go` 相同 msg 与字段的行（`dns query flows through matcher` / `… plugin`；`trace_id` 与汇总行一致），
覆盖嵌套序列、`call`/`goto`/`jump`、多 exec、fallback/preference；`matcher_name`/`plugin_name` 按 Go 的生成规则在编译期生成并保存（`anonymous_match(type: args)`、`not(...)`、`no_exec`、`multi_exec[...]`）。
未抓取时每步只多一次 `Option` 判断（门槛复测 20/100/500 qps 通过）；开启时 CPU 约 ×1.9、缓冲 32768 行/8 MiB 有界。与 Go 的逐步对照除 `jump` 目标的 logger 名外逐行一致；UI 无需改动。
顺带修了 5D 工具对已保存 WebUI 端口的处理。记录见 [validation-records/10-18-f2b](validation-records/10-18-f2b/README.md)。

## 2026-10-17：F-2 诊断（日志抓取、指标对齐、pprof 决定）实现完成，待 Codex 审查结论与规划会话确认

- **日志抓取**：`POST /api/v1/capture/start`、`GET /api/v1/capture/logs` 按 Go 契约（默认 120 s、1–600 s、读一次即清空、重叠 start 重置）。抓取期间诊断阈值临时降到 debug，到点自动回到**配置的** `log.level`（Go 硬恢复 INFO，原生不改写配置级别）；
  缓冲 ≤4096 行且 ≤1 MiB，满了丢最旧；每个被审计查询产出一行 `dns query completed`（trace_id/域名/类型/缓存/路由/上游/耗时），只写内存缓冲，不在 DNS 线程做文件 I/O。Go 的逐 matcher 轨迹属实现细节，原生按查询粒度。
- **`/metrics`**：新增 `mosdns_aliapi_{query_total,error_total,upstream_winner_total,response_latency_millisecond(直方图)}`，label `addr/metrics_tag/tag`、桶与单位同 Go；cache、进程指标不变。
  明确不提供：`inflight_queries`、`conn_*`、`forward`/`metrics_collector` 系列（配置不用）、`go_*`。
- **`/debug/pprof`**：原生不提供（404），兼容决定与替代手段（日志抓取、`/metrics`、`perf`）已写入契约。
- 100/500 qps 冻结延迟门槛复测与浏览器冒烟结果见 [validation-records/10-17-f2](validation-records/10-17-f2/README.md)。5D-3 与 Go 侧 `/log` 的最终处理仍未开始。

## 2026-10-17：UI-1 原生去掉 `/log`、外置 UI 包实现完成，待 Codex 审查结论与规划会话确认

原生只提供 `/`（`/log`、`vue-log1` 资源不再嵌入，构建脚本不再 `build:log1`；Go 侧未改）。新增外置 UI 包：`npm run build:external` 从同一份 `src/` 生成与目录名无关的包（相对路径 + `ui-manifest.json` + zip），
放进 `<配置根>/ui/<任意名字>/` 后在 `/<名字>/` 打开；带 manifest 的目录做兼容检查（不兼容则不挂载，health 说明原因），无 manifest 的目录照旧作为通用静态挂载；health 的 `external_ui` 列出全部外置目录，
内置 `/` 的系统页对所有已挂载且兼容的包显示链接。VM 浏览器冒烟用 `pg`、`a` 目录名，在 `/pg/` 里做了审计停止/启动，并验证 manifest 兼容/缺失/不兼容与文件实时替换。
记录见 [validation-records/10-17-ui-1](validation-records/10-17-ui-1/README.md)，使用说明 [external-ui.md](external-ui.md)。完整原生 `/metrics`（F-2）、5D-3 与 Go 侧 `/log` 的最终处理仍未开始。

## 2026-10-17：F-1 系统与外观实现完成，待 Codex 审查结论与规划会话确认

四项能力从“尚未实现”改为真实：**外观（服务端）**（背景上传/历史/文字与按钮颜色，状态文件与 Go 互通，用 Go 处理器做了 34 个请求的双向逐字节对比）、**WebUI 端口**（启动时覆盖 `api.http`，保存后重启生效，优先级同 Go）、
**重启服务**（进程内原地重启：同 `SIGTERM` 的排空与最终保存后 `execve`，**PID 不变**，不依赖 systemd；生产形态配置实测 DNS 中断约 0.40 s）、**进程指标**（`/metrics` 的 `process_*` 与 `mosdns_process_threads`，不伪造 Go 专有项）。
范围按用户决定只改 `/` 的 Vue UI，`/log` 未动；浏览器冒烟原生与 Go 各一次，并因此发现并修复 WebUI 端口面板在原生下被隐藏。记录见 [validation-records/10-17-f1](validation-records/10-17-f1/README.md)。
`/log` 的移除、完整原生 `/metrics`（F-2）与 5D-3 仍未开始。

## 2026-10-17：5D-2c UDP 接收缓冲实现完成，待 Codex 审查结论与规划会话确认

所有 UDP DNS 监听（三条绑定路径统一用 `socket2`）把接收缓冲调到有效 4 MiB（先 `SO_RCVBUF`，必要时 `SO_RCVBUFFORCE`；失败只告警），有效值在启动日志和 `GET /api/v1/system/health` 的 `udp_receive_buffers` 中可见。
H1/H3 在 500 qps 下三次均 0 丢失、0 内核丢包（5D-2b 为丢 34–130 个）；停顿本身仍在（API 调用 0.42–0.63 s），窗口内最多约 1% 的查询晚于 500 ms，根治仍是 M2-3。
20/100/500 qps 冻结延迟门槛各 ×3 通过，阈值未改。5D-2b 的 CPU 触发档（扩展 warm-up）按冻结定义是非过载档位，已做 profiling：按冻结口径八次原生重复没有一次超过 1.5×（中位数比 1.20），无单一热点，不优化。
契约补充了保存时 DNS 约停顿 0.4–0.65 s 的说明。记录见 [validation-records/10-17-5d-2c](validation-records/10-17-5d-2c/README.md)。5D-3 与生产替换仍未开始。

## 2026-10-17：5D-2b 测量完成（无产品代码改动），待 Codex 审查与规划会话/用户决定

故障注入 F1–F6、负载下热更新 H1–H4、冻结阈值复测 C、冷启动 late_wrong 诊断 L 均在 VM 隔离环境完成，数据见
[validation-records/10-17-5d-2b](validation-records/10-17-5d-2b/README.md)。要点：F1–F6（含 F3 故障应答不进缓存、F4 无静默直连、F5 回退、F6 下载失败可见）与 H2、H4 通过；
C 的容量（16000 qps 可持续）、过载恢复（5–6 s）、稳态 RSS（≤125.3 MiB 且远低于 Go）和采样峰值全部在冻结阈值内，阈值未改；L 的全部 `late_wrong` 是过载下期限处的格式正确 SERVFAIL，没有答案内容错误。
**未通过项（需要决定）：** H1/H3 在 500 qps 下热应用阻塞 DNS 线程约 0.5–0.65 s，内核 UDP 接收缓冲溢出丢查询（H1 每窗口 6–93、H3 0–42，丢失数与 53 端口 socket 的 `drops` 逐项相等；100/250 qps 不丢但约 1% 晚到），
Go 参考 H1 零丢失。其余报告项：F5 慢速 HTTP/1.1 DoH 上限 12 连接导致 foreign SERVFAIL；故障后 RSS 高水位不回落。5D-3 与生产替换仍未开始。

## 2026-10-17：5D-2a 可用性加固实现完成，待 Codex 审查结论与规划会话确认

两项来自已通过任务审查的 P2：（A）已删除专用组的产物清理失败不再停 host——保存成功、host 保持 ready、
告警并列入 `GET /api/v1/system/health` 的 `pending_cleanup`，之后每次保存与每次启动重试；启动孤儿清理同样非致命；
其它 `CommittedCleanup`（旧代退休、托管 store 收尾）语义不变。（B）深嵌套（>= 16 层）延迟分支等待上游时确实空转：
修复前 40 层嵌套等待 200 ms/600 ms 时外层被轮询 10036/32720 次（随等待线性增长），修复后 20/20 次；`BranchPollQueue::pump` 只在
“分支已完成或仍有延后任务”时才再次唤醒。记录与数据见 [validation-records/10-17-5d-2a](validation-records/10-17-5d-2a/README.md)。
5D-2 其余项（故障注入与负载下热更新、容量/过载恢复复测、32k 冷启动 late_wrong）与 5D-3 仍未开始。

## 2026-10-17：5D-fix-B 受影响门槛通过，等待外部 Claude 审查

基线 `353371fa`（含已提交 fix-A）。原生审计保留改用紧凑行、共享不可变路由/
诊断与答案载荷、数字 trace 和两个固定4096槽弱缓存，公共 `AuditRecord` 与
既有 API 投影不变。100000条满环请求内存从991.76降至218.00字节/条（约−78%），
容量和答案完整性不变，未并入 Mem-2。VM fmt/严格Clippy/1482测试（0失败、4既有
忽略）通过；config_rust check gaps=0，输出与BASE逐字节相同。

用户确认旧私有归档丢失，明确批准更换测量输入及只读生产快照导出；配置包仍
0412666，候选前固定新Go/BASE参考，只有已有 full: ASCII大小写契约差异28格。
新Go与候选在同一新快照上按 G/N,N/G,G/N 三轮交错复测，原冻结阈值与档位/时序
不变。新结果不声称旧输入逐项复现；旧FAIL历史记录保留。

六次正式运行完整保留。2k稳态RSS三次57.969/57.734/57.887MiB，8k为
62.367/61.805/61.977MiB，逐次低于128MiB与同输入Go；采样峰值最高86.523MiB，
低于256MiB。20/100/500QPS p95/p99三轮中位数均通过原门槛，正常档位逐次正确率/
覆盖率100%，所有应答无错误/超时/迟到。20QPS第2轮p99 native83.479ms、Go82.873ms
高尾值未剔除（新语料同七格在时间窗内有80ms foreign ECS HTTPS响应）；门槛按
原三轮中位数判断，不宣称每轮延迟均过线。CPU未触发1.5倍profiling条件。

私有原始证据已归档并校验，VM任务目录/归档副本已删除，原mosdns仍active/PID827437。
等待用户转交外部Claude审查。[记录](validation-records/10-10-5d-fix-b/README.md)。
不提交、push、部署或
开始下一任务。5D-2/3、真实过载恢复、Phase6、脚手架退役与生产替换仍未接受。
下述旧日期条目保留当时状态。

## 2026-10-10：5D-fix-A 已复测，等待外部 Claude 审查

5D-1 已经 Claude 审查、用户确认并提交为 `cd0162cb`；方法被接受，产品 F
与高负载 RSS 预算失败。用户另行批准 fix-A 修订 DoH 串行策略：h2 独立
stream，HTTP/1.1 有界池12、原期限等待、取消隔离及 drain；TCP/DoT 保持原状。

[Fix-A 记录](validation-records/10-09-5d-fix-a/README.md)：同一冻结阈值，Go/native
交错各3次，F p95/p99 中位数84.017/84.915ms、常规20/100/500各延迟与按期率/
覆盖率/零错误 DNS 门禁 PASS。F SOCKS8886次中位数降至10次，CPU904降至416µs；
新 Go CPU392µs，未触发1.5倍 profiling。Go常规r1 q20一个未发送槽位，99.8%
覆盖率短缺原样保留，未替换或改变预算。VM检查1476通过、0失败、4既有忽略；
原config_rust check gaps=0。执行结束并清理后停在外部审查点，未提交/部署。

**待决**：Claude验收fix-A；fix-B的100k审计保留/高负载RSS预算；随后按用户授权
推进更宽5D。此次未重测容量或真实过载恢复，未启动5D2/3、Phase6、脚手架退役
或生产替换。下述10-09停止点记录保留为当时状态。

## 2026-10-09：5D-1 已到停止点②，等待外部审查

用户确认冻结阈值后，先完成 Go 场景 F 三轮，再完成普通/扩展/F 的三轮
交错 Go/native 对照与定向 profiling。[公开记录](validation-records/10-09-5d-1/README.md)
显示普通20/100/500QPS延迟达标；F p99为96.644ms，超过93.339ms；稳态RSS在
2,000QPS起超过128MiB。容量延迟/按期率组件至少8,000QPS，容量比值仅报告。
参照oracle还存在既有大小写契约覆盖缺口，未改原始成绩或补称正确性门禁通过。
因此5D-1预算未通过，等待用户转交外部Claude审查。未启动优化、5D-2/3、
Phase6、混合脚手架退役或生产替换，也未提交或部署。

## 2026-10-06 规划调整：以自用 `config_all` 为首次替换门槛（当前）

用户于 2026-10-06 做出以下决定（规范记录见
[重写方案第 6 节](../ai/rust-rewrite-plan.md)）：

- 首次生产替换的功能门槛改为：原生 host **无修改加载并运行自用 `config_all`**
  （里程碑 **M1**），再经 5D、Phase 6 和用户确认；`config_all` 未使用的插件延后
  到替换之后补齐，不删除。
- 下一批任务改为跑通真实配置，不再按“请求内原语家族”补全全表。
- 审查强度维持现状。
- 同日补充：生产机为 1 vCPU / 957 MB，取消原定的并发模型 spike，保持单线程
  模型；内存提升为与延迟同等的优先目标，5D 设 RSS 预算并以单核实际负载为基准。

`config_all` 当前原生缺口（2026-10-06 按 YAML 文本统计，实施前以 `check`
输出为准）：

| 批次 | 内容 |
| --- | --- |
| M1-a 规则 provider | `domain_set_light`（11 实例）、`sd_set_light`（4）、`domain_mapper`（2）、`si_set`（1），SRS/geodata 格式 |
| M1-b 上游与响应 | 独立 `aliapi`（5 实例，签名 API 字段另行确认；`forward_nocn*` 使用 `socks5`，须同时支持 SOCKS5 上游）、`rewrite`、`adguard_rule`、`webinfo`；原计划中实际使用的 `rcode`、`cname`、`mark`、`tag_setter`、`drop_resp`、`cname_remover`、`sleep` |
| M1-c 输出与再查询 | `domain_output`（8 实例）、`requery` |

实际上游仅使用 https/tls/udp；H3/DoQ 不在 M1 关键路径上。

**2026-10-06 `check` 结果（以此为准，取代上表）**：详见 [M1 缺口清单](m1-gap-list.md)。
与上表的差异：

- M1-a、M1-c 的插件类型与实例数一致；`si_set` 另连带 10 处 `resp_ip $geoip_cn`。
- M1-b：`cname`、`mark` 在 `config_all` 中未使用，`cname_remover` 已被原生接受，三者移出；
  实际缺口为 `rcode`（6）、`tag_setter`（10）、`drop_resp`（10）、`sleep`（1），并新增
  `fallback` 的 `primary`/`secondary` 裸 tag 形式（2）。
- 新增 **M1-d 配置加载与 listener**：`log.file`、`log.level: warn`、全部 20 个
  listener 的 `listen: ":port"`、`enable_audit` 必填、未托管时只允许一个 listener。
- 两项用户决定（2026-10-06）：相对路径改为与 Go 一致、从配置根目录解析（改原生，
  不改配置包）；原生在不开启 `native_management` 时也接受 `config_all` 的形态（多
  listener、`aliapi` 等）。两者先于依赖路径的 M1 插件实施。
- 生产快照（Go v0.7.3）的 `check` 输出与 `config_all` 逐字节相同。

### 执行顺序

| 次序 | 交付 | 结束条件 |
| --- | --- | --- |
| 1 | 原生 `check -c <config>` 与台账收敛（任务 `10-06-rust-native-config-check-ledger`）：**已完成，C2C 第 2 轮 PASS，已归档** | 一次列出全部不支持的类型/参数；`config_all` 输出成为 M1 清单，`feature-coverage.md` 据此更新 |
| 1.5 | **M1-0** 配置根目录路径基准 + 非托管加载兼容（任务 `10-06-rust-native-m1-0-config-loading`，即 M1-d）：**已完成（`26accd14`），C2C PASS，已归档**；`check` gaps 86→64、globals 3→1（剩余 GLOBAL 为 aliapi 缺失的连带结果） | `-d` 支持，插件参数相对路径按配置根目录解析；非托管多 listener、`":port"`、`enable_audit` 可选、`log.level`/`log.file`；重跑 `check` 后 M1-d 无剩余 |
| 2 | 合并 `main`（v0.7.1 → v0.7.3；任务 `10-06-rust-merge-main-v0.7.3`）：**已完成（`a879755a`），已归档** | 真实 merge 提交；v0.7.3 新 UI 操作在原生下禁用+原因；契约漂移见 `validation-records/10-06-merge-main-v0.7.3/drift.md`，原生对齐归 M1-e |
| 3 | M1-a1 规则 provider 查询侧（`3ea29ebf`）、M1-b2 请求内原语（`e75153db`）、M1-a2 规则源管理与热更新（`7a174484`）：**均已完成并归档** | `check` 直接缺口 86→16；剩余 `aliapi` 5、`domain_output` 8、`rewrite`/`webinfo`/`requery` 各 1 |
| 3.1 | **原生内存优化（2026-10-08 提前至此）** | M1-a2 后热重建 RSS 稳定约 350 MB（启动 114 MB；Go 生产进程约 236 MB；生产机 957 MB）。先测量内存去向与分配器行为，再优化数据结构与重建峰值，在同一生产快照上与 Go 对比；结果作为 5D RSS 预算依据。拆为 Mem-1（测量 + 分配器/重建峰值）与 Mem-2（数据结构压缩，按 Mem-1 结论）。**Mem-1 已完成并提交**：glibc 保留 + `malloc_trim`、固定 mmap 阈值、进程关闭 THP、重建临时数据借用规则串、旧代在 DNS 线程外释放；同一快照启动稳态 114→37 MB、启动峰值 201→95 MB、6 次重建后 365→40 MB、重建峰值约 310–375→120–133 MB、重建 0.8→0.35 s，DNS 结果不变（Go v0.7.3：启动 121–144、重建后 170–174、空闲 300 s 后 113–117 MB）。记录 `validation-records/10-08-mem-1/`。**Mem-2 建议范围**：紧凑 domain trie（matcher 21→约 6 MiB，稳态约 −13 MB）+ 按 provider 分组的构建输入（峰值约 −30 MiB），顺带移交 provider 规则串（−7 MiB）。**用户决定（2026-10-08）：Mem-2 不立即做，挪到 5D 阶段按 RSS 预算需要再定；下一步 M1-b1。** |
| 3.2 | M1-b1 上游与网络类：**已完成（`1f249571`），已归档** | `aliapi`（含 SOCKS5）、`rewrite`、`webinfo`、能力标签；缓存放置规则按固定/客户端推导 ECS 修正（M1-g）；Go 覆盖文件不应用（由 M1-f 取代）；`check` gaps 16→9、globals 0 |
| 3.3 | M1-c1 `domain_output`（8）→ M1-c2 `requery`（1） | 拆为两个任务以控制单次规模；完成后 `check` 对 `config_all` 无缺口，可进行首次整链运行 |
| 3.35 | M1-f 类型化设置层与原生配置包 `config_rust`：**M1-f1 已提交（`ed9a5f1a`）；M1-f2 已提交（`2b9d0bea`）** | 具名类型化对象 + `$tag` 引用、`webinfo/settings.json`、类型化 API 与热生效（f1）；`config_rust` 包（`file` 仓库 `0412666`）、迁移工具、两套 WebUI 页面接线（f2） |
| 3.36 | **E2E-1 第1轮修复及同语料重跑完成，待第2轮外部审查** | exit成功发布、lazy/失败门禁、嵌套缓存标签、N1 fuel耗尽及热effective_tag已修复；用户2026-10-09批准前台256/lazy64。各8156条：语义一致3681、差异4475（Go偶然行为4471、2026-10-07批准full:偏差4，未解释0）；DNS只剩2条批准差异，3099四格正确，主缓存3812hit/entries，domain_output两边2。VM全检查通过，临时资源已清理；尚未验收/提交/关闭M1。见 [验证记录](validation-records/10-09-e2e-1/README.md) |
| 3.4 | M1-e special_groups v0.7.3 对齐：已实施，VM 验证通过，待外部审查 | 标准配置下 schema-4 生成、引用/私有上游、无效引用排除、整代热生效；`native_management` 冻结。记录见 [M1-e](validation-records/10-09-m1-e/README.md)；未经外部审查不记完成 |
| 4 | 5D 整机验收 | **5D-1 基线与冻结阈值已完成（`cd0162cb`）**；冻结预算的两项 FAIL 已由 5D-fix-A DoH 并发复用（`353371fa`）与 5D-fix-B 审计内存压缩（`20d18764`）在不改阈值下解决。剩余：5D-2 故障注入与负载下热更新（含容量/过载恢复复测、清理失败非致命化、深嵌套轮询空转测试、32k 冷启动 late_wrong 排查）、5D-3 长时间稳定运行 |
| 5 | Phase 6 + 生产切换/回滚方案 | 退役脚手架，纯 Rust 构建/运行；配置包升级（C15）与回滚步骤明确；`mos-test` 验证后经用户确认替换 `mosdns` |
| 6 | 未使用插件补齐 | 首次替换后进行，不阻塞替换 |
| 7 | M2（首次替换之后） | 见 [M2 规划](plans/m2-backlog.md)：M2-1 具名带类型功能开关取代 `switch1`–`switch17`；M2-2 域名自学习重新设计（单一学习存储、逐条时效与复核、声明式学习规则、可见可控）。不阻塞首次替换 |

此前延期且无归属的“专门远端 fault/cancel/close E2E”并入 5D。

## 2026-10-06 原生核心查询原语与迁移台账收敛（已被上方调整取代）

> 历史状态记录：本段原规划已由上方 2026-10-06 规划调整取代。台账收敛并入
> 次序 2；原语中 `config_all` 实际使用的部分并入 M1-b，其余延后到首次替换后。

[公开规划摘要](plans/native-core-query-primitives.md)对应新的独立顶层任务
`10-06-rust-native-core-query-primitives`。它先以当前 accepted native source
和验收证据收敛 feature ledger，再处理 qclass、ptr_ip、random、rcode、
has_wanted_ans、cname、mark、tag_setter、drop_resp、black_hole、cname_remover
命名/预设补齐，以及 sleep 的命名/quick/预设形式。

任务目前仍为 `planning`，实施授权为 false；尚未启动产品代码、构建、测试、
部署或默认/生产切换。规划基线为干净的 `rust` HEAD
`83b86cd90cc15f23b3a882226e06551b909045ef`，实施前必须重新冻结 HEAD、工作树、
accepted evidence 和 roadmap succession。

旧 `09-28-rust-next-step-roadmap` 已作为历史协调任务标记 `superseded` 并归档；
其 canary、限定 fast_mark/flow_setter、domain_set 管理和 bundled delivery 均保留
各自归档证据，不再作为待执行下一项。完整 5B/5C、5D、Phase 6 和生产切换门禁
仍然保留。

## 2026-10-05 当前规划：原生开关状态与管理

> 历史状态记录：本段保留 2026-10-05 的规划快照；switch 状态实现随后已完成
> 并归档，不再是当前待启动任务。

专用组/上游管理和 WebUI 运行时能力均已完成验收归档；当前 accepted dirty
源码为 c98dc5d1012ce73aed318504c1053b04ab15de08，真实 HEAD 仍79d93ae1。
下一批 [switch1–17 公开规划](plans/native-switch-state-management.md) 已与
同一 C2C 对话收敛范围，第2轮完整规划复审已返回 PLAN READY；本地任务处于
planning，等待后续明确实施批准，未启动实现。
六阶段覆盖声明/查询快照、持久化 owner、DNS/运行时边界、API/能力 inventory、
两套 UI 和整链验证。后台 Lazy refresh 保留完整既有 recipe；不重读开关、不增
一般缓存失效或 requery。外观/媒体、webinfo/别名和诊断日志留给独立后续任务。
规划 READY 和后续人类实施批准是不同门槛；下方旧日期状态保留历史。

## 2026-10-02 下一阶段规划：专用组与上游管理

用户已选择标准 UDP/TCP/DoT/DoH、本地文本规则和主入口/专用端口闭环。
[公开规划摘要](plans/special-groups-upstream-management.md) 已完成 C2C
规划复审，结论 PLAN READY；本地任务仍处于 planning，等待最终完整规划批准。
七个切片集中完成生成配置、查询快照与监听器所有权、事务恢复、缓存依赖失效、
管理 API、现有 Vue 和真实进程证明。未启动实现，不表示完整 5B/5C、5D 或
生产切换通过。下方旧日期安排保留历史证据，不代表当前待执行任务。


## 2026-09-30 当前推进位置

此节更新当前状态；下方旧日期段落保留当时的规划/证据，不表示仍待执行。
本地规则编辑、审计控制/DNS 卡片，以及查询诊断/排名下钻的限定交付均已
完成归档。最近查询诊断源码 HEAD 为 `860c6253`；完整范围、VM 和浏览器证据
见 [已归档查询诊断任务](validation-records/09-30-rust-native-query-diagnostics-overview/prd.md)。
这些子项不代表完整 P02/P34/C08/C10/C11 或整个 5B/5C 通过。

下一批为 [Rust-native upstream forwarding workflow](validation-records/09-30-rust-native-upstream-forwarding/prd.md)，
一个 PRD 集中接入多上游选择、UDP TC→TCP、bootstrap、DoT/DoH、串行连接
复用、实际最终 supplier/attempt 诊断与真实 DNS/HTTP/Vue 证明。用户已批准
host 默认/0 双栈解析、IPv4 优先，4/6 强制单族；此修改只映射原生 host
配置，不改已归档 resolver 的默认契约或 Go 运行路径。

任务仍为 planning；PRD/design/implement/contracts 和执行提示词已落盘，
整个任务范围及其余列明偏差须经最新规划摘要批准后激活。专用 reviewer
绑定与真实授权 snapshot 是 start 前置条件，不继承旧规划对话。
规划反馈中的五个问题已落实到任务文档：调用点 descriptor 保持 ID-only
dispatch、started-entry ledger/RAII 与异步 drain、audit-off 小型事实与启动顺序、
resolver 规范纠错，以及用户另行批准的可选 schema1 upstream_diagnostics
对象和真实详情显示。此处仍是修订规划，不表示 API/UI 已实现或最终复审 PASS。
本批不含连接失败后的跨族回退、QUIC/H3、pipeline、系统 hostname resolver、
上游编辑/完整指标面板、生产部署或最终切换。5D、Phase 6 与生产确认保留。

目标仍是 Linux amd64 上纯 Rust-native 完整 MosDNS，优先正确性、稳定运行、p95/p99、有效吞吐和并发，内存其次。本次按用户“避免过度设计”的要求调整推进方式，不删除功能或降低最终发布标准。

项目执行约定（2026-09-28）：用户指定所有本项目需要的构建与测试验证均使用 `mosdns-rust` SSH 别名对应的 VM；后续 Rust 构建、Cargo 测试、集成与 E2E 均连此别名执行。远端预检和传输也只用别名，不使用直连 IP 或其他 VM。

2026-09-29 状态更新：下方 canary 和 `fast_mark`/`flow_setter` 的详细段落保留其当时的规划与证据轨迹，不再表示待执行状态。Canary 的替代复审任务已归档并取得 C2C `FINAL: PASS`；5B `fast_mark`/`flow_setter` 子任务也已归档，修正范围取得同一对话 `FINAL: PASS`。下一个规划任务是扩大的 [Rust-native local rule editing workflow](validation-records/09-28-rust-native-domain-set-management/prd.md)：在同一 PRD 中完成限定 `domain_set` 的 native HTTP/持久化/DNS 热生效与维护中 Vue `/` 本地规则页的隔离端到端编辑。该任务仍处于 planning，须按其修订后的 PRD/design/implement 复审并获后续实施批准；不代表完整 Vue/5C 或生产可用。

## 当前事实

W1 UDP/TCP、W2 简单缓存、W3 受限分流和基础观测有归档证据。首轮对照覆盖有限；后续测量任务已关闭为 [incomplete matrix](phase5a-measurement-reliability.md)，不补跑、不调阈值，不补称容量/恢复/热点已证明。单线程 LocalSet 的多核能力尚未验证；暂不凭猜测改 Send/runtime。

5B 代表链已突破固定 W1/W2/W3 图，`fast_mark`/`flow_setter` 的限定集成也已完成。当前瓶颈是原生配置仍只覆盖完整产品的子集，管理 HTTP/持久化/UI 闭环尚未接上。下一批以新增可运行操作闭环为成果，正式测量缺口在适当链路上补，不挡住功能规划。

## 后续顺序

| 次序 | 阶段与交付 | 结束条件 |
| --- | --- | --- |
| 1 | 隔离 `mosdns-rust` Rust-native sidecar canary：已完成 | 修正运行和替代精确范围复审已 PASS；仅是限定功能验证，不是兼容或性能门禁 |
| 2 | 5B `fast_mark` matcher/executable + `flow_setter` 集成：已完成限定范围 | 修正范围同一 C2C 对话 `FINAL: PASS`；不等于完整 5B |
| 3 | 扩大的第一条 5C 闭环：file-backed `domain_set` native HTTP/持久化/下一 DNS 查询，加现有 Vue `/` 本地规则页隔离编辑：规划中 | 同一任务的独立行为切片覆盖 HTTP、整代发布、失败回滚、并发、重启/关闭和浏览器编辑；修订规划复审与后续实施批准后执行，完成后再做精确范围 review |
| 4 | 补齐所有剩余 5B/5C 功能 | [覆盖表](feature-coverage.md) 的配置/插件/API/持久化/管理条目都有相应证据；复用现有 Vue UI |
| 5 | 5D 完整整机验收与有依据的优化 | 完整配置下正式 Go/Rust 对照、容量/恢复/并发、管理干扰、长稳和资源预算通过，阻塞项为零 |
| 6 | Phase 6 hybrid 退役和发布验证 | 去除过渡 Go/cgo/selector/mirror/fallback，必要完整回归、纯 Rust 构建/运行通过；随后才考虑生产确认 |

阶段编号表示最终责任，不要求所有 5B 条目完成才开始任何 5C 集成。剩余 Phase 4 能力按真实链路依赖接入，不先补齐全部协议再开始主程序。所有最终功能仍保留；switch/provider 等共享机制可成批实现，不能把每个覆盖行变成独立微型项目。

2026-09-28，同一 C2C 对话已在已验证的 `mosdns-rust` Project/workspace
（分支 `rust`，规划输入 HEAD `9fd0bc0c`）返回
`PLAN_STATUS: READY_WITH_EXPLICIT_EXECUTION_GATE`，并在第 3 轮对累计提交范围
`9fd0bc0c061fb440c88781949f5088652aca70b9..3887af32d6ef17624f8b1d198e0dcb0bed4a28d7`
返回 `FINAL: PASS`。路线图和两个新 Trellis 子任务均已落地。执行既有
canary 已获用户授权，四项默认输入已冻结。用户随后指定 `mosdns-rust` 作为
项目测试 VM。该 VM 的只读预检和配置快照哈希复核已完成；在任何远端构建或
测试前，先审核这个目标变更。本路线图不改变候选版本、功能范围或隔离边界。

## 当前 5B 任务和第一小目标

沿用 [09-27 config/sequence 任务](validation-records/09-27-rust-phase5b-config-sequence-composition/prd.md)，revision 2。实现已获单独授权并完成；本地 workspace 验证及指定 Linux 上 UDP/audit-on 集成 E2E、TCP/audit-off CLI E2E 已通过。C2C 对 `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767` 的精确范围 review 返回 `FINAL: PASS`；短诊断不作性能 PASS。用户指定的 C2C 对 A6 更正范围 `098b4c5e2bc3427f591456d6725a04a8cb8bcc23..146849c042bfa90a5d61cc6fbe9712b78d562e94` 返回 `FINAL: PASS`；用户于 2026-09-28 授权归档已接受的 5B 任务范围。专门远端 fault/cancel/close E2E 仍延期，未声称通过；详细记录见归档任务的 `implement.md`。

本批支持 direct $sequence、一个 cache 在 entry/child 后继上的组合、reject 0..15（含常用 0/3）、顶层 include、provider 多规则/files，以及 qtype/has_resp。代表配置从本地 config_lite_all 裁剪，公网 aliapi 用已有受控 forward 替代；未支持部分有明确延期，不能声称原配置整体兼容。

第一小目标：本地和选定 Linux 环境实际运行 block、路由 cache miss/hit、默认分支和 child 后父继续。相关实现与指定远端 E2E 已完成并通过精确范围 review；远端完整 workspace/legacy suites、fault/cancel/close 变体、`local.only.test` 精确规则和完整配置兼容仍未测，不作为本次已证明事实。取消原六 slice/四门禁安排；真实命名观测和 cache 后继接缝随必要功能完成，不独立扩框架。

## 下一项已规划工作：isolated mosdns-rust sidecar canary

[09-28 Rust-native isolated mosdns-rust sidecar canary](validation-records/09-28-rust-mos-test-native-sidecar-canary/prd.md) 是独立的纯验证规划任务，固定候选 SHA `016103f3c21ed2d659694ce10e64aaf24b5c2767`。计划覆盖由 `config_lite_all` 只读快照裁剪的 include/relative-rules/sequence/cache/route 链，在 `mosdns-rust` 上顺序验证 UDP/audit-on 与 TCP/audit-off 两个 loopback 高端口 sidecar；controlled peers 作为功能、路由和 cache oracle，并要求每次都回收自有 PID/socket、保持原服务基线不变。

原始 canary 计划经过三轮修订，C2C 对固定范围 `016103f3c21ed2d659694ce10e64aaf24b5c2767..82953751bdde89fa3fc2244cea2a86be4f6a3d06` 返回 `FINAL: PASS`。该审核覆盖原计划范围，不覆盖当前 `mosdns-rust` 目标变更。用户已授权 canary 并冻结四项默认值；配置源七个文件哈希与记录一致。`mosdns-rust` 只读预检已确认服务和工具链，但所有观察仍须在执行时刷新。目标变更已由 bootstrap reviewer 返回 `FINAL: PASS`（工作树补丁 SHA-256 `9ea51551398c804dbf76c1a7d4dd58ded1a5b9468244b4de52c0bdc78f1a88e6`）。第一次尝试在 Rust sidecar 启动前因过严的监听行比较器停止并安全清理。第二次尝试的 12 个逐条 DNS/peer oracle 全部通过，但 final aggregate assertion 写错；同一 C2C 对话第 8 轮审核认定为 `STOP / harness invalid` 并批准修正为 `local_udp=2, default_tcp=1`。第三次重跑在 `mosdns-rust` 上返回 `PASS`：UDP/audit-on 与 TCP/audit-off 各 6 个查询通过，peer、PID/socket 和服务基线均核验，临时目录在证据捕获后删除。Canary 不要求外部读取 audit records，不包含 Go 构建，不覆盖完整 config package，不做性能 PASS，也不改变生产门禁。相同 C2C 对话的 host-level 最终审核仍待完成；计划与门槛见 task 的 `design.md`、`implement.md`、`research/canary-inputs.md`。

### 已建但未启动的后续子任务

- [09-28 Rust-native fast_mark and flow_setter sequence integration](validation-records/09-28-rust-native-fast-mark-flow-setter/prd.md)：第一批 5B 运行链。仅关闭实际交付的 `fast_mark` 与 `flow_setter` 子项，不声称整个 P11/P33/P44 或完整 5B。须保留 bit 48/49 约定、每查询标志隔离，并先冻结配置 metadata 与 host terminal metadata 的优先级。依赖 canary PASS；只有用户明确延期 canary 且允许忽略远端结果时才能提前。
- [09-28 Rust-native domain_set management save and query closure](validation-records/09-28-rust-native-domain-set-management/prd.md)：第一条 5C 管理闭环，限一个规则文件语义明确的 file-backed `domain_set`，以真实 HTTP、文件、DNS query 验证 `/show`、`/save`、`/post`、失败回滚、整代发布、重启和 listener 回收。默认在 5B 任务获得同一 C2C 对话 `FINAL: PASS` 后启动；若 5B 明确延期或阻塞，须另取得用户明确的重排决定。这里是计划顺序而非架构依赖；不声称全 P02/C04/C10/C11/C17 或完整 5C。

这两个子任务均处于 `planning`，只允许在本路线图精确范围 review PASS 后依门槛启动。它们不触碰 Go/cgo scaffold，不更改 Vue UI，不扩充 feature-coverage 整行状态。

## 简化工作方式

- 复杂任务保留简明 PRD/design/implement，需求只写一处；research 记录真正的语义疑点和来源，不重复全部规划。
- 一次规划审查、一次最终完整审查为主；中间仅重大契约/范围变化或高风险问题追加审查，不逐步骤等待新 PASS。保留现有 reviewer 授权规则。
- 变更时跑相关测试，交付前一次完整回归和 Linux 功能 E2E；新修改/失败才重复。修复后的合理重验不套正式实验的有限重跑规则。
- 日常正确性检查不要求 benchmark；热路径有实质改变或代表链形成时做已有工具的轻量诊断；正式矩阵用于性能结论和最终验收。明显退步及时调查，不强制为每批重建工具/profile。
- 保存必要 source/config/命令、结果/失败和自有资源退出记录，不每次规划编辑建 hash 清单，不把归档校验扩成单独项目。
- 进度记录“新增哪些配置/链路和行为”，并更新覆盖子项；不以 slice/PASS/归档数量替代产品进展。

基础 DNS/cache 语义、取消/资源回收、TLS、持久化与最终独立审查保留。已冻结的旧实验和最终 5D/Phase 6/生产门禁不改。2026-09-27 的规划修订本身未启动实现；用户随后授权该 5B 任务，当前实作及交付状态以其任务记录为准。

## 2026-10-02 response-policy/IP bounded increment

The approved [response-policy/IP task](validation-records/10-02-rust-native-response-policy-ip-rules/prd.md) implements hosts, scoped redirect, TTL and multi-value IPv4/IPv6/CIDR response predicates directly in the native host. S1–S5 exact-commit C2C PASS; S6 real DNS/API/Vue/process-restart and full workspace checks pass, final cumulative review FINAL: PASS (2c059b0e..13d60d3e, iteration 7); task archived. Plain text/inline rules only; provider sets, SRS/compression and rule-management reload remain future work. Cache dumps are not policy-versioned, requiring quiescent live-owner Flush before shutdown for immediate changed-rule behavior. This increment does not close 5D/Phase 6 or authorize a default/production release.
