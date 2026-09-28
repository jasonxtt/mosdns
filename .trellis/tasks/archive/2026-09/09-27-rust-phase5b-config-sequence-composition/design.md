# Design — one representative query chain

## Architecture

配置文件/include/规则 → strict decode/source paths → 定义/类型引用 → 单一 ProgramSpec → typed catalogs/HostAssembly → 现有 listener → ExecutionMachine step/await/resume → response/cache/基础观测。

复用 matcher-core/cache-core/upstream-core，保留 current-thread/LocalSet。允许修改 native-host config/assembly/execution/matchers/cache、sequence-core program/engine 及必要测试，只为 direct call/cache 后继完成/真实位置补最小能力。必要的常用 reject wire 修正可涉及 dns-core，不扩完整 EDNS。不能因旧文件限制把链路必需能力全部延期，不建设通用插件框架、第二 dispatcher 或新 runtime。

## Config and provider

保留重复 key/严格类型/未知字段检查；文件加载入口保留 base directory 和 source paths。compile_yaml 等内存入口不能偷偷用随机 cwd 读 include。plugins-only 子文件有序加载后收集定义，规则顺序保持；嵌套 include 明确拒绝，不先建设递归加载/热更新系统。

domain_set 使用现有 MixMatcher 的语义和规范化，不重写匹配引擎。接入 files/exps、qname/qtype/has_resp；文件相对路径、注释/空行按当前 provider/loader 定向确认。只对有歧义行为补 characterization，不建立全面 Go parity 框架。

## Direct calls and async controls

给现有 machine 增加 named child call 的最小表示/作用域：child 自然完成、accept/reject 后父继续；exit 跨普通 call/inline 传播，遇 try 才捕获；普通错误不被 try 吞掉。jump continuation/goto 清栈保持原语义，不把 direct call lower 成 jump/try。

所有 await 共用 admission 绝对期限。executable ID 选择真实 owner，forward 数量不决定错误策略。后继成功 response/source 替换旧值；错误/坏包终止为 SERVFAIL，不输出早先成功 wire。无响应终止 REFUSED，fuel/取消/close 有界。单/多 forward 校验统一作为明确的安全性修正记录，不重写旧测量。

## Cache continuation

一个 host-owned cache，size 按记录数传入现有有界容量，lazy=0；不再要求全图等于 W2。每查询只创建一个访问/待写令牌，第二次动态访问明确执行失败，不覆盖前一个 token。

hit 完成缓存所在后继链，不能无条件结束整个父查询。miss 记录有归属的后继完成点，跨 child call/await 后在该点取得合格响应快照并写入，然后再运行调用者。父后续覆盖 response 不污染已完成 child cache；entry cache 包裹下游则保存该后继的最终响应。

用 machine 的最小 continuation/完成通知承接，不在 host 重演 AST。不预建多 cache 嵌套框架。取消/deadline/错误/坏响应不发布；W2 的 key/TTL/ID/admission 保持。Go cache.go:490–512 是后继返回后保存的发现参考；本批禁止错误残留发布的安全收紧须说明，不复制 Go 内部生命周期。

## Reject and observation

existing synthesized 路径输出支持的 0..15，保留 question/ID/RCODE；>15 加载报 unsupported，不截断成 u8。完整 12-bit 是产品最终要求，留后续；本批不建设完整 EDNS 扩展工程。

用配置级 SequenceId/origin mapping 和实际执行位置记录 named sequence，synthetic inline 继承所属名字，纯栈返回不伪造一次执行。沿用“末次实际 matcher/exec 所属命名 sequence”的 R4 语义，audit 开启才 materialize 名称。最终 response source 与 ordered attempts 分开；完整 flow_setter/C08/API 后续补全。

## Validation and rollback

主验收是 research/example-compositions.md 的现有配置派生链；控制边界用小变体补足，以 wire/peer counters/父子后继为独立 oracle，不由候选结果反向生成预期。

本地相关检查与一次完整回归、Linux 功能 E2E、一次最终完整审查。不每步设置 PASS/input digest；保留 source/config/命令、失败及自有进程退出记录。修复重跑相关项，新修改/失败才重复完整回归。回退只涉及本批改动/自有进程，保留无关 dirty 和旧 frozen 证据。
