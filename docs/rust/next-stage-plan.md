# Rust-native 下一阶段安排（2026-09-28）

目标仍是 Linux amd64 上纯 Rust-native 完整 MosDNS，优先正确性、稳定运行、p95/p99、有效吞吐和并发，内存其次。本次按用户“避免过度设计”的要求调整推进方式，不删除功能或降低最终发布标准。

## 当前事实

W1 UDP/TCP、W2 简单缓存、W3 受限分流和基础观测有归档证据。首轮对照覆盖有限；后续测量任务已关闭为 [incomplete matrix](phase5a-measurement-reliability.md)，不补跑、不调阈值，不补称容量/恢复/热点已证明。单线程 LocalSet 的多核能力尚未验证；暂不凭猜测改 Send/runtime。

当前功能瓶颈是原生配置还受固定 W1/W2/W3 图限制，已有模块未充分接入实际查询链。下一批以新增可运行配置为成果，正式测量缺口在适当链路上补，不挡住功能规划。

## 后续顺序

| 次序 | 阶段与交付 | 结束条件 |
| --- | --- | --- |
| 1 | 已完成的 5B 代表链之后，执行既有隔离 `mos-test` Rust-native sidecar canary | 计划已有精确范围 C2C PASS；本轮路线图审查 PASS 后，还须冻结四项执行输入并由用户明确选择执行。canary 是功能/运行隔离验证，不是完整兼容或性能门禁 |
| 2 | 5B 第一批：native `fast_mark` matcher/executable + `flow_setter` 序列/观测集成 | YAML 编译错误、标志 OR/set/每查询隔离、真实分支和异步路由元数据有集成证据；canary 通过，或用户明确延期并允许在无远端结果时继续；本任务精确范围 review PASS |
| 3 | 5C 第一条闭环：单个有界 file-backed `domain_set` 的 `/show`、`/save`、`/post` → 持久化 → 下一 DNS 查询 | 默认在 5B 第一批精确范围 review PASS 后执行；两者无架构依赖。若 5B 明确延期或阻塞，须先取得用户明确的重排决定。HTTP/持久化/原子发布/并发读取/重启/关闭边界有真实 native 证据，并通过本任务精确范围 review |
| 4 | 补齐所有剩余 5B/5C 功能 | [覆盖表](feature-coverage.md) 的配置/插件/API/持久化/管理条目都有相应证据；复用现有 Vue UI |
| 5 | 5D 完整整机验收与有依据的优化 | 完整配置下正式 Go/Rust 对照、容量/恢复/并发、管理干扰、长稳和资源预算通过，阻塞项为零 |
| 6 | Phase 6 hybrid 退役和发布验证 | 去除过渡 Go/cgo/selector/mirror/fallback，必要完整回归、纯 Rust 构建/运行通过；随后才考虑生产确认 |

阶段编号表示最终责任，不要求所有 5B 条目完成才开始任何 5C 集成。剩余 Phase 4 能力按真实链路依赖接入，不先补齐全部协议再开始主程序。所有最终功能仍保留；switch/provider 等共享机制可成批实现，不能把每个覆盖行变成独立微型项目。

2026-09-28，同一 C2C 对话已在已验证的 `mosdns-rust` Project/workspace
（分支 `rust`，规划输入 HEAD `9fd0bc0c`）返回
`PLAN_STATUS: READY_WITH_EXPLICIT_EXECUTION_GATE`。本路线图和两个新的
Trellis 子任务正在落地，尚待同一对话对精确提交范围返回 `FINAL: PASS`。
计划审查通过前不启动下游任务；审查通过后，既有 canary 的四项输入和
执行/延期选择仍须先与用户明确冻结。此 C2C 路线图计划不会改变已有
canary 的候选版本、范围或独立执行门槛。

## 当前 5B 任务和第一小目标

沿用 [09-27 config/sequence 任务](../../.trellis/tasks/archive/2026-09/09-27-rust-phase5b-config-sequence-composition/prd.md)，revision 2。实现已获单独授权并完成；本地 workspace 验证及指定 Linux 上 UDP/audit-on 集成 E2E、TCP/audit-off CLI E2E 已通过。C2C 对 `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767` 的精确范围 review 返回 `FINAL: PASS`；短诊断不作性能 PASS。用户指定的 C2C 对 A6 更正范围 `098b4c5e2bc3427f591456d6725a04a8cb8bcc23..146849c042bfa90a5d61cc6fbe9712b78d562e94` 返回 `FINAL: PASS`；用户于 2026-09-28 授权归档已接受的 5B 任务范围。专门远端 fault/cancel/close E2E 仍延期，未声称通过；详细记录见归档任务的 `implement.md`。

本批支持 direct $sequence、一个 cache 在 entry/child 后继上的组合、reject 0..15（含常用 0/3）、顶层 include、provider 多规则/files，以及 qtype/has_resp。代表配置从本地 config_lite_all 裁剪，公网 aliapi 用已有受控 forward 替代；未支持部分有明确延期，不能声称原配置整体兼容。

第一小目标：本地和选定 Linux 环境实际运行 block、路由 cache miss/hit、默认分支和 child 后父继续。相关实现与指定远端 E2E 已完成并通过精确范围 review；远端完整 workspace/legacy suites、fault/cancel/close 变体、`local.only.test` 精确规则和完整配置兼容仍未测，不作为本次已证明事实。取消原六 slice/四门禁安排；真实命名观测和 cache 后继接缝随必要功能完成，不独立扩框架。

## 下一项已规划工作：isolated mos-test sidecar canary

[09-28 Rust-native isolated mos-test sidecar canary](../../.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/prd.md) 是独立的纯验证规划任务，固定候选 SHA `016103f3c21ed2d659694ce10e64aaf24b5c2767`。计划覆盖由 `config_lite_all` 只读快照裁剪的 include/relative-rules/sequence/cache/route 链，在 `mos-test` 上顺序验证 UDP/audit-on 与 TCP/audit-off 两个 loopback 高端口 sidecar；controlled peers 作为功能、路由和 cache oracle，并要求每次都回收自有 PID/socket、保持原服务基线不变。

该计划当前仍为 `planning`。经过三轮修订，C2C 对固定范围 `016103f3c21ed2d659694ce10e64aaf24b5c2767..82953751bdde89fa3fc2244cea2a86be4f6a3d06` 返回 `FINAL: PASS`，锁文件 provenance、相对路径、自检、PID/端口隔离和所有自有进程清理 finding 均已关闭。没有连接 `mos-test`、构建或启动 sidecar。任何 canary 执行都需要用户另行明确授权。它不要求外部读取 audit records，不包含 Go 构建，不覆盖完整 config package，不做性能 PASS，也不改变生产门禁。四项推荐执行默认值和实际配置快照身份需在启动前冻结；计划与未决输入见 task 的 `design.md`、`implement.md`、`research/canary-inputs.md`。

### 已建但未启动的后续子任务

- [09-28 Rust-native fast_mark and flow_setter sequence integration](../../.trellis/tasks/09-28-rust-native-fast-mark-flow-setter/prd.md)：第一批 5B 运行链。仅关闭实际交付的 `fast_mark` 与 `flow_setter` 子项，不声称整个 P11/P33/P44 或完整 5B。须保留 bit 48/49 约定、每查询标志隔离，并先冻结配置 metadata 与 host terminal metadata 的优先级。依赖 canary PASS；只有用户明确延期 canary 且允许忽略远端结果时才能提前。
- [09-28 Rust-native domain_set management save and query closure](../../.trellis/tasks/09-28-rust-native-domain-set-management/prd.md)：第一条 5C 管理闭环，限一个规则文件语义明确的 file-backed `domain_set`，以真实 HTTP、文件、DNS query 验证 `/show`、`/save`、`/post`、失败回滚、整代发布、重启和 listener 回收。默认在 5B 任务获得同一 C2C 对话 `FINAL: PASS` 后启动；若 5B 明确延期或阻塞，须另取得用户明确的重排决定。这里是计划顺序而非架构依赖；不声称全 P02/C04/C10/C11/C17 或完整 5C。

这两个子任务均处于 `planning`，只允许在本路线图精确范围 review PASS 后依门槛启动。它们不触碰 Go/cgo scaffold，不更改 Vue UI，不扩充 feature-coverage 整行状态。

## 简化工作方式

- 复杂任务保留简明 PRD/design/implement，需求只写一处；research 记录真正的语义疑点和来源，不重复全部规划。
- 一次规划审查、一次最终完整审查为主；中间仅重大契约/范围变化或高风险问题追加审查，不逐步骤等待新 PASS。保留现有 reviewer 授权规则。
- 变更时跑相关测试，交付前一次完整回归和 Linux 功能 E2E；新修改/失败才重复。修复后的合理重验不套正式实验的有限重跑规则。
- 日常正确性检查不要求 benchmark；热路径有实质改变或代表链形成时做已有工具的轻量诊断；正式矩阵用于性能结论和最终验收。明显退步及时调查，不强制为每批重建工具/profile。
- 保存必要 source/config/命令、结果/失败和自有资源退出记录，不每次规划编辑建 hash 清单，不把归档校验扩成单独项目。
- 进度记录“新增哪些配置/链路和行为”，并更新覆盖子项；不以 slice/PASS/归档数量替代产品进展。

基础 DNS/cache 语义、取消/资源回收、TLS、持久化与最终独立审查保留。已冻结的旧实验和最终 5D/Phase 6/生产门禁不改。2026-09-27 的规划修订本身未启动实现；用户随后授权该 5B 任务，当前实作及交付状态以其任务记录为准。
