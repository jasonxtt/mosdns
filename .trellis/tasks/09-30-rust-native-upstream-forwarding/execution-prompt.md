# Execution prompt

Only sending the prompt below as a new user message approves this final plan;
its presence in the repository does not authorize implementation.

```text
继续 /Users/tom/github/mosdns-rust，保持 rust 分支。执行
.trellis/tasks/09-30-rust-native-upstream-forwarding 这一整个任务。

我批准该任务最新 PRD（R1–R8、A1–A9）、design.md、implement.md 和
research/forward-contracts.md 的完整范围及原生差异：默认/0 双栈解析且
IPv4 优先、4/6 强制单族；域名上游要求显式 bootstrap 或数字 dial_addr；
最多三条不同上游请求；选出结果后取消并回收其他工作；未实现参数明确报错。
保留现有单上游配置和诊断行为，不做连接失败后的跨族回退。
我也批准可选、schema_version:1 的 upstream_diagnostics v2 日志对象，
按 forward-contracts.md 展示真实 selected entry/peer/transport 和启动顺序
attempts；旧字段/Go 行为保留，缺失或未知版本不伪造诊断。

授权范围是一个完整交付单位：多上游/标签子集/quick forward、UDP TC→TCP、
bootstrap、TLS/HTTPS、串行复用和并发 busy 的受控新连接路径、最终 supplier
与真实 DNS/HTTP/Vue 证明。先读 AGENTS 和项目文档，加载 Trellis 规范，核对
最新源码、任务状态与脏改动。按清单逐项 RED→GREEN→重构，主对话直接执行。
遵循 research/planning-review.md 的修正规划：调用点 descriptor/ID 不扩
sequence-core 通用 External；started-entry ledger/RAII 与异步 drain 共用
事实源；audit-off 保持小型 ID 和基础 metrics，无 audit-only String；
attempts 按启动顺序，winner 独立按完成优先级选择。

实施授权仅在前置门槛满足后生效：为本任务解析并验证专用 c2c-web reviewer
绑定和原生传输，冻结上述整个授权单位与真实 automation snapshot，再用
task.py start 激活。不得使用普通 planning session.url、继承别的任务的
reviewer、伪造或补写旧 snapshot；若绑定缺失/不明确，报告具体阻塞并停在规划。

所有 Cargo/UI 构建和产品测试只通过 SSH 别名 mosdns-rust，在独立目录执行。
使用受控 UDP/TCP/TLS/HTTPS/bootstrap peers、synthetic CA 和自有进程/隧道；
保留源码/配置/命令、原始失败及清理证据。相关测试随修改运行，最终完整
workspace fmt/clippy/tests、native build、disposable npm build 和集成浏览器
证明。不得操作生产服务或公网 DNS，不宣称完整 5B/5C/5D 或容量验收。

允许针对本任务源码、测试、规范、规划和证据作必要本地窄范围提交，以支持
精确提交范围的最终 REVIEW_ONLY 审查；不得 git add -A、夹带无关脏改动、
push、部署、改 Go/cgo/default release 或自动归档。先核对本任务规划文件是否
已提交；必要时只提交本任务规划文件作为可审阅基线。

完成后向专用 reviewer 一次发送完整的确切 parent/head、changed paths、
validation 和 scope prohibitions，不贴 diff/文件全文；明确 FAIL 后只修当前
范围再复审，遵守五轮同根因与重大问题停止门槛。最终 PASS 后停下，报告新增
可运行能力、检查证据、提交及延期项，等待我决定归档或下一步。
```
