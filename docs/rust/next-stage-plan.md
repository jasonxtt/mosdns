# Rust-native 下一阶段安排（2026-09-28）

## 2026-10-05 当前规划：原生开关状态与管理

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
