# External planning review disposition — 2026-10-02

来源：用户指定的当前已打开ChatGPT对话“复审规划任务”：
https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abf396d-df64-83ee-95cb-c67ce9008620

通过C2C工作流检查连接后，用同一内置浏览器tab读取DOM，没有发送消息、重绑会话或新建reviewer。对方明确说“按你提供的规划摘要”复审，并未声称读当前完整PRD/design/source。其APPROVED FOR IMPLEMENTATION是外部意见，不是用户实施授权。本地仍planning，未运行task.py start/改产品源码/构建。

| 意见 | 本地判断/处置 |
| --- | --- |
| P1-1 冻结policy/cache/fallback顺序 | 需要可执行例子，已补design表和测试；不接受固定全局阶段，因为sequence顺序本来由YAML决定。resp_ip不生成synthetic response；hosts仅Continue，显式has_resp/accept才短路；后续已执行forward/fallback可以按原合同供应最终wire。 |
| P1-2 redirect owner/question/CNAME/negative/TTL | 原design已有scoped return frame与wire字段保留，补a→b→d及NXDOMAIN/SOA具体例子、真实peer和client双向验证。仅恢复Question，不扁平化已有CNAME，不改上游SOA/TTL/rcode。 |
| P1-3 synthetic response禁入cache或新增policy generation | 规则改变保留dump的残留风险成立，已明确行为与restart/flush验证；不采纳 blanket synthetic禁入或新policy key/version，二者改变已冻结普通cache/hosts placement与v2互通，且本轮无policy热更新。已有generation gate管flush/import，不能冒充规则版本。立即废除旧值使用无回填条件下已有flush，或停机后移除dump再启动，保留普通retention语义；限制列入最终摘要。 |
| P2-1 IP文件startup/refresh | 补empty/missing/invalid/I/O矩阵；保留Go缺IP文件告警跳过（ip_set.go:430）、hosts/redirect缺文件报错。坏文件失败。文件只startup加载，DNS后台refresh复用immutablesnapshot，不存在runtime文件刷新失败清空问题。 |
| P2-2 最低测试矩阵 | 采纳并与S1–S6绑定；补真实UDP/TCP、negative链、IP边界、empty/bad文件、重复query/fallback/refresh/restart。 |

依据：plugin/matcher/resp_ip/resp_ip.go:50（只读Answer matcher）；native cache PendingStore::publish_with_domain（wire校验/exclude_ip/generation，无Local来源禁入）；Go cache.go:1190（有效response admission，无synthetic禁入）；redirect.go:95（query恢复/CNAME，error传播）；ip_set.go:430（missing告警跳过）。完整源码核查与其余固定合同见policy-contracts.md。

规划未增加新插件/API或扩大IP格式范围，未改用户已选择的范围。已补充的缓存保留限制、例子与测试矩阵需随最新规划整体审批；外部“附条件批准”不触发Trellis执行。

## Complete-artifact C2C planning review — iteration 0

New conversation explicitly requested by user: https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abf42c0-0b24-83e8-a56e-cdd6427a976a . Workspace verified as mosdns-rust. Reviewer independently read current PRD/design/implement, research contracts/disposition and relevant current code through the connector. Returned FINAL: PASS on 2026-10-02; no actionable planning finding. Confirmed sequence-defined policy placement, scoped QueryView/redirect restoration, startup immutable rules and old-dump retention limitation, IP text-only boundaries and public test/mock matrix.

Implementation watch items: QueryView is not yet implemented; identify real query-wire ownership before S3, protect admission audit and all forward/cache/prefer consumers. S1 remains typed config/loaders/validation; no temporary redirect runtime shortcut. Reviewer proposed S1 public fixture/limits tests and its acceptance gate. Its planning PASS is independent review evidence, not human phase-transition approval or implementation test evidence.
