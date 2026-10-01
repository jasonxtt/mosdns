# 可交给执行对话的提示词

以下正文仅在用户审阅并批准最终规划后，由用户复制到执行对话。本文档本身
不是授权 snapshot，不表示 task 已 start，不自动授权 commit/push 或部署。

---

我批准执行 `.trellis/tasks/10-01-rust-native-query-fallback-address-preference`
的最终规划。请先完整读取 AGENTS.md、项目必读文档，以及此任务的 prd.md、
design.md、implement.md、research/branch-contracts.md 和
research/c2c-discussion.md；以其 R1–R6/A1–A8
为本次完整范围，在 rust 分支执行，不将范围拆成需逐个批准的微型任务。

已确认产品决策：fallback 省略 threshold 为 500ms，显式 0 立即启用备用；
负值保留 500ms 兼容默认。新分支查询用 schema 2 诊断及现有详情显示，保留
schema 1 读取兼容。只扩现有详情 renderer，不新增 trace 页面。完成 fallback、prefer_ipv4/prefer_ipv6、分支状态隔离、
根预算/取消回收、缓存和最终来源归属、真实 DNS/HTTP/Vue 组合验证。

当前规划基线 601b65a45e10b8b534fda5c5052ff310815dec66。先核对实际 HEAD 和
所有脏路径：后续提交需重新审视相关接口，不能默默覆盖他人修改。解析此任务
自己的 dedicated c2c-web reviewer 绑定并验证原生 transport；缺失/变化/歧义
必须说明，不能继承上一任务 reviewer 或使用普通 c2c session.url。依据本条
用户消息冻结真实授权范围 snapshot，再通过 task.py start 启动；不能补造授权。

主会话直接实现/检查，不派实现或检查 agent。按 implement.md 的行为切片、
公开接口和 mock 边界走 red -> green -> refactor，产品代码修改前读取
 trellis-before-dev，完成后执行 trellis-check 与必要 spec 更新。构建/测试全部
经 SSH 别名 mosdns-rust 在隔离自有目录执行，先查磁盘/inode；完整 workspace
包含集成与 doctests，不把 --lib 或磁盘/linker 故障当作全绿。保留必要原始证据，
回收自有 PID/socket；不清理无关文件/进程，不碰生产、不扩 Go/cgo scaffold。

经过完整验证后遵循阶段 3.4 与授权策略提交精确任务路径，记录完整交付的
parent/head SHA、修改路径、证据和限制，用一个自包含原子请求请本任务
专用 c2c reviewer 审整个提交范围。等待最终结果，不中途补消息；限定 FAIL
在范围内修复并重新审核，重大范围变更先回到规划。最终报告功能、验证、
提交和 reviewer 结果；不要自动 push、部署或归档。
