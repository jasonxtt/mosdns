# Rust-native response policy and IP-rule chain

## Goal

在一个 PRD 中补齐本地应答、域名重定向、TTL 调整和 IP 结果判断，形成可与 cache/fallback/forward 组合的原生查询链。用户已确认IP规则限定范围；当前只规划，未授权实施。

## Confirmed baseline

- 2026-10-02 缓存交付已提交 `343c3810`，归档 `2c059b0e`。独立收尾复查无新增阻塞项，重跑 30 Rust/4 Vue PASS；详见 ../archive/2026-10/10-01-rust-native-cache-lifecycle-management/research/closure-review.md。
- 当前 native config PluginKind 不含 hosts/redirect/ip_set；resp_ip 只接受一个 IPv4 literal（rust/native-host/src/config.rs:1922）。matcher-core 已有 IpPrefixList，但不代表 host 完成 IP provider 接入。
- hosts 旧契约参考 plugin/executable/hosts/hosts.go 与 pkg/hosts/hosts.go；entries/files、domain matcher、A/AAAA、TTL10与缺地址族空NOERROR/SOA行为见research/policy-contracts.md。
- redirect 旧契约参考 plugin/executable/redirect/redirect.go；它执行 successor 后恢复原问题并加入 CNAME，不能按简单 query rewrite 实现而丢掉返回/失败/取消边界。
- ttl 旧契约参考 plugin/executable/ttl/ttl.go；只有 quick 语法，固定值/上下限、0 no-op及 min>max 先min后max合同已记录，不能擅自新增 named plugin YAML。
- resp_ip 旧契约参考 plugin/matcher/resp_ip/resp_ip.go：对 Answer A/AAAA 按 IP matcher 判断；ip_set 支持 ips/sets/files，完整文件格式范围需研究，不以文本 loader 假称所有 ip_set 完成。

## Requirements

- R1 hosts：named entries/files，启动载入、相对路径及标准域名匹配；双栈应答，未命中继续原链；生成响应进入现有 DNS/audit 投影。
- R2 redirect：named rules/files，独立 successor 状态、原 question 恢复、CNAME/最终 answer 安全重建；与嵌套、fallback、prefer、owner refresh 共用有限预算，不新增 detached root。
- R3 ttl：既有 fixed/range quick 语法，修改 DNS RR TTL，OPT 不作 TTL；对无响应/畸形响应及合法边界明确结果。
- R4 IP 判断：resp_ip 支持 IPv4/IPv6、CIDR 和 named ip_set 引用；复用 matcher-core，在 immutable snapshot 上判断，不能临时逐请求读文件。
- R5 组合契约：按design明确 cache 前后插件顺序影响，分别验证 miss/hit/lazy/restart 的最终 wire、TTL、domain_set 与 supplier；本地响应不能伪造上游供应身份。
- R6 保留上一任务的 generation/publication/shutdown 合同；特别验证 redirect 改写后的 cache key/question 对应及恢复，不能由请求 raw 不变造成错缓存。
- R7 顺序与缓存失效：YAML sequence 决定顺序，不新增固定policy阶段；resp_ip只判断，不生成响应。合法hosts wire按普通cache placement入库，不新增policy key/version。规则只在启动时载入；修改policy后保留dump重启仍可能命中旧条目，立即失效需无回填条件下flush后切换，或完成停机后移除对应dump再启动新配置。本批不承诺规则删除自动清缓存。

## Observable acceptance slices

- A1 YAML/default/quick/path/rule fixture 对照；错误定位，旧配置回归。
- A2 实际 DNS hosts A/AAAA/未命中/非地址 query，最终 HTTP answers 同真实 wire。
- A3 redirect 正常、嵌套、失败/取消/燃料耗尽后的 question/CNAME/状态一致，无父分支污染。
- A4 TTL fixed/range/0/bounds、cache 前后顺序、lazy TTL 与后台更新无意外延寿。
- A5 resp_ip 双栈/CIDR/provider 判断按真实 Answer，缺响应 false；分支选择和缓存组合一致。
- A6 mosdns-rust 独占 loopback DNS/API/浏览器实际详情证明、必要 Rust/UI 检查；没有生产部署或性能门禁声明。
- A7 验证redirect的a→b→d保留CNAME链、target NXDOMAIN/SOA/TTL，不伪造success；验证空/缺失/坏IP文件的不同startup结果，以及修改规则保留dump与flush后的restart结果。

## Scope and compatibility boundaries

- 用户于2026-10-02同意ip_set仅ips内联与普通文本files；SRS/二进制/压缩、非空sets跨provider引用、IP管理API/热更新延期。resp_ip引用本轮named ip_set和&文本文件保留，不混淆两类引用。
- ECS、client_ip等其它matcher、special_groups配置生成/生产切换、Go hybrid扩展延期，不宣称完整ip_set/全部策略插件。
- hosts命中但缺请求族返回空NOERROR+旧FakeSOA；hosts设置response后Continue，ttl0不改TTL，range先min后max（允许inverted）。redirect独立当前QueryView、共享budget、恢复question与CNAME，准确传播terminal outcome。
- 兼容差异：规则路径使用声明配置目录；多regexp/keyword匹配采用稳定首次登记顺序（不复制Go map不确定性）；仅现有Rust regex支持范围；文本单行64KiB/每插件64MiB及1000000规则限制；范围外明确报错。缺失ip_set文件告警跳过、hosts/redirect文件错误失败保留旧显式差异。
- 保留普通cache key/retention语义，不自动policy-version失效；规则改动后旧dump的处理见R7和design具体矩阵，最终审批需涵盖这一限制。

## Validation and planning status

research/policy-contracts.md记录源码锚点、规则/wire/facts合同；design.md记录QueryView、scoped redirect、payload/IP catalog、缓存组合与兼容边界；implement.md按S1→S6列出可观察行为、公开测试接口、mock边界与依赖。保持一个任务，未拆child、未派发agent。

规划已收敛，等待最终整体审批。未修改产品源码，未构建/部署，不运行task.py start。前置缓存归档与既有review证据不等于本任务验证通过。

## Final approval — 2026-10-02

Human explicitly approved the latest complete planning summary with “批准”. Approval covers S1–S6, the frozen IP/text-only scope and retained-dump limitation, inline execution and same-chat per-slice C2C PASS before continuing. No repeated approval is needed for this unchanged scope. Planning/review gates are satisfied; task activation has not run because this turn still carries an explicit stay-in-planning workflow directive. This is a recorded approval, not implementation evidence.

## Current execution status — 2026-10-02

The final plan was explicitly approved by the human. Task activation succeeded after correcting the conditional planning breadcrumb; historical planning-only notes above describe earlier sessions. S1 config/loaders candidate is implemented and awaiting final checks and exact-commit C2C review. S2–S6 remain pending, each gated by the preceding slice PASS. No push or deployment.
