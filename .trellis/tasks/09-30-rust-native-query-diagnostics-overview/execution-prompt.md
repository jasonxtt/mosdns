# 可复制到另一个 Codex 对话的执行提示词

我批准实施已完成规划的 Trellis 任务：
`/Users/tom/github/mosdns-rust/.trellis/tasks/09-30-rust-native-query-diagnostics-overview`。
这是对该任务最终 PRD/design/implement 和 research/contracts.md 中所列提案的实施批准；不要再次询问是否创建任务或重做整套规划。

请先读取 AGENTS.md、docs/ai/project-context.md、config-notes.md、rust-handover.md、rust-rewrite-plan.md，以及该任务的 prd.md、design.md、implement.md、research/contracts.md、research/c2c-discussion.md、research/roadmap-and-baseline.md。确认工作区是 /Users/tom/github/mosdns-rust、分支 rust；该任务当前 planning，既有本地规则和审计控制任务已经完成，旧路线图中的过时状态不能作为重新实施依据。

按 Trellis 启动本任务，在同一个 PRD 内完整交付：真实最终 DNS 审计字段与响应详情、日志搜索/过滤/分页、四类排名和慢查询、QueryManager 列表/详情、Overview 排名下钻，以及相关并发/关闭回收/现有工作流回归。不要再拆成许多微型 PRD，也不要只完成后台接口就收工。当前模式 inline，由主对话执行实现和检查。

我一并批准本规划写明的新增契约：rank 最大 limit 500 和确定性同分排序；固定格式 native 请求 ID；最终响应 all-or-none 投影、非常见 RR 原始诊断与 answer_details_status/answer_decode_error；专用 logs/domain 精确下钻及仅404回退到旧Go查询；严格URL编码错误400；未知规则来源不伪造 unmatched_rule、不计入规则排名；保留独立 top300 慢查询历史；两个后台重读任务满额时返回503。普通 q/domain 搜索语义、已有审计控制/设置和真实DNS行为保持兼容。精确 provider 文件行来源仍延期。未批准截断答案、隐藏字节淘汰或降低审计容量；如测量证明必须改变这些产品契约，带具体证据提出选择。

所有构建、Cargo测试、UI构建和真实进程验证只在 ssh 别名 mosdns-rust 对应VM的隔离目录执行。保留当前全部无关脏改动，Trellis auto-commit 关闭，不部署生产，不改Go/cgo桥接、不重写Vue、不运行旧完整性能矩阵冒充本任务验收。实现中跑相关行为测试，最终一次完整Rust回归、fmt/clippy、Vue构建、真实UDP/TCP+HTTP+浏览器闭环；保留原始失败、修正、配置/命令和自有资源退出记录。

授权仅提交本任务的规划、实现、必要spec/coverage更新和证据，不包含无关路径；不要push。授权用 codex-with-chatgpt 在现有 mosdns-rust 项目中开展本执行对话的C2C完整独立审查，读取规划讨论链接并对精确完整提交范围做最终review，修正真实问题后复审到明确 FINAL: PASS。内部ownership选型和例行修复自行推进；重大范围/用户可见契约改变再征求同意。最终按工作流报告真实交付、验证、审查范围和仍延期功能；归档须符合届时用户授权，不宣称完整5B/5C、5D、Phase6或生产就绪。
