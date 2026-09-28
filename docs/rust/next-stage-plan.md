# Rust-native 下一阶段安排（2026-09-28）

目标仍是 Linux amd64 上纯 Rust-native 完整 MosDNS，优先正确性、稳定运行、p95/p99、有效吞吐和并发，内存其次。本次按用户“避免过度设计”的要求调整推进方式，不删除功能或降低最终发布标准。

## 当前事实

W1 UDP/TCP、W2 简单缓存、W3 受限分流和基础观测有归档证据。首轮对照覆盖有限；后续测量任务已关闭为 [incomplete matrix](phase5a-measurement-reliability.md)，不补跑、不调阈值，不补称容量/恢复/热点已证明。单线程 LocalSet 的多核能力尚未验证；暂不凭猜测改 Send/runtime。

当前功能瓶颈是原生配置还受固定 W1/W2/W3 图限制，已有模块未充分接入实际查询链。下一批以新增可运行配置为成果，正式测量缺口在适当链路上补，不挡住功能规划。

## 后续顺序

| 次序 | 阶段与交付 | 结束条件 |
| --- | --- | --- |
| 1 | 5B 首批代表链已完成；后续先做隔离 `mos-test` Rust-native sidecar canary，再据真实配置 blocker 选择下一批 | 代表链的本地与选定 Linux E2E 及精确范围 review 有证据；sidecar 计划完成并经 review，执行须另行授权 |
| 2 | 5B 按功能家族补齐，同时穿插一条 5C 管理闭环 | 常用 matcher/provider、响应处理、上游策略/协议按依赖成批交付；尽早验证改规则→保存/reload→下一查询生效 |
| 3 | 补齐所有剩余 5B/5C 功能 | [覆盖表](feature-coverage.md) 的配置/插件/API/持久化/管理条目都有相应证据；复用现有 Vue UI |
| 4 | 5D 完整整机验收与有依据的优化 | 完整配置下正式 Go/Rust 对照、容量/恢复/并发、管理干扰、长稳和资源预算通过，阻塞项为零 |
| 5 | Phase 6 hybrid 退役和发布验证 | 去除过渡 Go/cgo/selector/mirror/fallback，必要完整回归、纯 Rust 构建/运行通过；随后才考虑生产确认 |

阶段编号表示最终责任，不要求所有 5B 条目完成才开始任何 5C 集成。剩余 Phase 4 能力按真实链路依赖接入，不先补齐全部协议再开始主程序。所有最终功能仍保留；switch/provider 等共享机制可成批实现，不能把每个覆盖行变成独立微型项目。

## 当前 5B 任务和第一小目标

沿用 [09-27 config/sequence 任务](../../.trellis/tasks/09-27-rust-phase5b-config-sequence-composition/prd.md)，revision 2。实现已获单独授权并完成；本地 workspace 验证及指定 Linux 上 UDP/audit-on 集成 E2E、TCP/audit-off CLI E2E 已通过。C2C 对 `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767` 的精确范围 review 返回 `FINAL: PASS`；短诊断不作性能 PASS。当前工作树中的 5B `task.json` 仍标记 `review pending` / `in_progress`，与 implement 和覆盖表记载的 PASS 不一致；本规划不擅自修改或归档该任务，详细记录见其 `implement.md`。

本批支持 direct $sequence、一个 cache 在 entry/child 后继上的组合、reject 0..15（含常用 0/3）、顶层 include、provider 多规则/files，以及 qtype/has_resp。代表配置从本地 config_lite_all 裁剪，公网 aliapi 用已有受控 forward 替代；未支持部分有明确延期，不能声称原配置整体兼容。

第一小目标：本地和选定 Linux 环境实际运行 block、路由 cache miss/hit、默认分支和 child 后父继续。相关实现与指定远端 E2E 已完成并通过精确范围 review；远端完整 workspace/legacy suites、fault/cancel/close 变体、`local.only.test` 精确规则和完整配置兼容仍未测，不作为本次已证明事实。取消原六 slice/四门禁安排；真实命名观测和 cache 后继接缝随必要功能完成，不独立扩框架。

## 下一项已规划工作：isolated mos-test sidecar canary

[09-28 Rust-native isolated mos-test sidecar canary](../../.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/prd.md) 是独立的纯验证规划任务，固定候选 SHA `016103f3c21ed2d659694ce10e64aaf24b5c2767`。计划覆盖由 `config_lite_all` 只读快照裁剪的 include/relative-rules/sequence/cache/route 链，在 `mos-test` 上顺序验证 UDP/audit-on 与 TCP/audit-off 两个 loopback 高端口 sidecar；controlled peers 作为功能、路由和 cache oracle，并要求每次都回收自有 PID/socket、保持原服务基线不变。

该计划当前仍为 `planning`。首轮 C2C 规划 review 返回 `FINAL: FAIL`；修订补充了可复核的 exact-candidate lockfile provenance、included YAML 的精确相对路径布局、启动前 config/peer 自检、共享主机 PID 身份复核，并将 sibling config snapshot 标为本轮未独立验证的规划输入。第二轮 review 确认前述五项已关闭，但指出 port-53 基线条件冲突和一处残留路径措辞；两处已修正，等待再次固定范围 review。没有连接 `mos-test`、构建或启动 sidecar。任何 canary 执行都需要用户另行明确授权。它不要求外部读取 audit records，不包含 Go 构建，不覆盖完整 config package，不做性能 PASS，也不改变生产门禁。四项推荐执行默认值和实际配置快照身份需在启动前冻结；计划与未决输入见 task 的 `design.md`、`implement.md`、`research/canary-inputs.md`。

## 简化工作方式

- 复杂任务保留简明 PRD/design/implement，需求只写一处；research 记录真正的语义疑点和来源，不重复全部规划。
- 一次规划审查、一次最终完整审查为主；中间仅重大契约/范围变化或高风险问题追加审查，不逐步骤等待新 PASS。保留现有 reviewer 授权规则。
- 变更时跑相关测试，交付前一次完整回归和 Linux 功能 E2E；新修改/失败才重复。修复后的合理重验不套正式实验的有限重跑规则。
- 日常正确性检查不要求 benchmark；热路径有实质改变或代表链形成时做已有工具的轻量诊断；正式矩阵用于性能结论和最终验收。明显退步及时调查，不强制为每批重建工具/profile。
- 保存必要 source/config/命令、结果/失败和自有资源退出记录，不每次规划编辑建 hash 清单，不把归档校验扩成单独项目。
- 进度记录“新增哪些配置/链路和行为”，并更新覆盖子项；不以 slice/PASS/归档数量替代产品进展。

基础 DNS/cache 语义、取消/资源回收、TLS、持久化与最终独立审查保留。已冻结的旧实验和最终 5D/Phase 6/生产门禁不改。2026-09-27 的规划修订本身未启动实现；用户随后授权该 5B 任务，当前实作及交付状态以其任务记录为准。
