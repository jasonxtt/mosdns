> Historical product planning or evidence snapshot. Lifecycle status alone is not acceptance PASS. See [validation summary](../../validation-summary.md). Raw logs and local workflow state are retained outside Git tracking.

# Rust Phase 5B — representative native query chain

## Goal and authorization

让同一个纯 Rust-native host 运行一条从现有配置裁剪的典型查询链：规则分流 → 直接调用子 sequence → 缓存命中/未命中 → 实际上游查询，并支持所需拒绝响应、基础审计和正常停止。进度以新增可运行的配置行为衡量。

2026-09-27 用户授权修订后续规划及现有任务，强调避免过度设计。本版 revision 2 替代 revision 1 的范围及 G0–G3 分段门禁。沿用现有任务路径；本 PRD 首次落地时仍处于 planning，随后用户于 2026-09-27 明确授权实现，执行记录见 `implement.md`。完整功能目标和最终发布条件不变。

## Background

源码基线 11bd56c40d255d6ae93b0a2eba1c85214300b149，分支 rust。5A W1/W2/W3 和基础观测已交付；测量任务关闭为 incomplete matrix，容量/恢复/热点未知，不重新启动该实验。

- config.rs:141–145、201–202、230–234、398–419 按插件数量/固定拓扑编译。
- execution.rs:216、231、297–323 存在按 forward 数选择策略、固定入口 provenance、cache hit Accept/根终态单 token 的假设。
- 本地 ../file/mosdns/config/config_lite_all/config_custom.yaml:76–87 使用条件缓存与 $sequence_main/$sequence_other；sub_config/forward_nocn.yaml:17–23 使用缓存后继查询；config_custom.yaml:10–23,26–48 使用 include/reject。它们是配置包参考，不是本轮新读取的生产配置。

链路、裁剪说明和独立预期见 [代表配置](research/example-compositions.md)。原配置的 aliapi、switch、flow_setter、lazy/dump 等未完整迁移，不宣称原配置包整体可加载。

## Requirements

### R1 — 链路所需配置能力

| 表面 | 本批范围 | 边界 |
| --- | --- | --- |
| 配置 | 多个 sequence/forward/domain_set；一个 UDP 或 TCP listener；按类型解析命名引用 | 不按插件数/场景名选程序；多 listener 后续补全 |
| 文件 | 顶层 include 按顺序加载 plugins-only 子文件，相对 include 按声明文件目录解析；domain_set 支持 exps/files | 不递归 include/热更新；子文件未支持字段报错；规则文件相对路径定向核对当前加载约定 |
| provider/matcher | 多条规则，复用 matcher-core 的 ASCII full/domain/regexp/keyword；qname $provider、qtype、has_resp、AND/单次取反；保留旧 resp_ip、_true/_false | 不新增 SRS/geodata 或规则管理；其他 matcher 后续补全 |
| executable | $sequence 直接调用、$forward、$cache；jump/goto/return/accept/exit/try-sequence；exec scalar/list/no-op | direct call 保留独立 child scope，不冒充 jump/try；try-external 后续补全 |
| cache | 一个实例，正整数 size、lazy_cache_ttl=0；可放 entry 或子 sequence，后继可调用 sequence；每查询最多一次访问 | 消除 W2-only placement；重复动态访问明确受控失败，不覆盖 token；多实例/lazy/dump/exclude/ECS 参数后续补全 |
| forward | 多实例，各一个 numeric UDP/TCP upstream，兼容旧 W1/W3 tag 形式 | 高级协议/bootstrap/上游组按后续真实链路依赖接入 |
| reject | 默认 REFUSED 及显式 0..15，至少验证实际常用 0/3 | 16..4095 本批加载时报 unsupported，不能截断；完整 12-bit/EDNS 属后续 5B，sequence-core 既有完整范围不缩减 |

### R2 — 加载行为与兼容

重复 key/tag、缺失/跨类型引用、未知字段/未支持参数、坏规则文件在 bind/查询前失败，错误包含文件和字段/规则路径。定义先收集再解析，include/规则顺序保持，不依定义位置绕过引用。旧合法 W1/W2/W3 保持；只修改扩展后已支持的旧拒绝测试。

### R3 — 执行、缓存与故障

direct child 自然结束/accept/reject 返回调用者，exit 向外传播，try 只捕获 child exit，普通错误仍终止。多 exec 作用域保持既有语义。缓存 hit 跳过其所在后继链，保留 ID/TTL；miss 在该缓存后继完成时保存合格响应，不能统一等父序列任意改写后才缓存。错误、坏响应、取消、期限过期不得把残留中间响应当成功或写入缓存。

每查询共用绝对 deadline，沿用 fuel=64 与取消/关闭。单/多 forward 统一合法响应校验和终态策略；无响应沿 REFUSED、普通错误沿 SERVFAIL。不增加第二解释器、线程池、Go bridge 或未经测量的优化。

### R4 — 基础观测

audit 开关不改变响应/调用顺序；缓存状态、实际 attempts、最终来源和命名执行位置真实，不能固定回填 entry 或显示 synthetic inline。audit-off 不新增逐查询 provenance 字符串或高基数 metric。只补链路必需接缝，不独立建设观测框架。

### R5 — 适当验证

代表配置经 CLI/listener 验证 block、分流 miss/hit、默认分支和 child 后父继续；小变体补控制流、错误、取消和 cache continuation。UDP/TCP、audit on/off、旧 W1/W2/W3 有适当回归。本地通过后在 mosdns-rust 隔离端口做一次功能 E2E。热路径实质改变时复用已有工具做轻量诊断，不设微秒门槛、不强制配对矩阵/profile 或新压测平台。

## Acceptance criteria

| ID | 可观察结果 | 对应 |
| --- | --- | --- |
| A1 | 代表配置的 include/provider/direct child/cache/分流实际运行；增加/重排定义不触发固定拓扑限制。 | R1/R2 |
| A2 | 加载负例在 bind/查询前定位失败；旧配置接受；延期能力明确报错。 | R1/R2 |
| A3 | block 无上游；路由 miss 正确调用、hit 无上游且 ID/TTL 正确；默认路由正确；child accept/reject 返回父，exit/try 正确。 | R3/R5 |
| A4 | 子缓存快照不被父后续改写污染；entry 缓存后继正确；错误/超时/取消无旧成功、错误 publication/后继请求；停止回收资源。 | R3/R5 |
| A5 | wire/trace 不受 audit 开关影响，缓存/来源/命名执行位置真实，旧观测不回退。 | R4 |
| A6 | 相关 Rust 检查和 Linux 功能 E2E 通过；覆盖表准确记录新增子项/延期/证据；最终完整审查后收口。 | R1–R5 |

## Out of scope and readiness

完整 5B 插件/协议、5C API/WebUI/special_groups/状态、容量/恢复/长稳、runtime/Send 改造、hybrid 退役和生产部署均未授权。延期保留在覆盖表，不删除完整版要求。

方向明确，无需新增过程问题。本版无独立 PASS，实现需要后续明确批准。正常任务以一次规划审查和一次最终完整审查为主；技术澄清/局部修复不逐步骤追加审批，重大契约/范围变化才回规划。
