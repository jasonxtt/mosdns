# Response policy contracts — 2026-10-02

用户已同意 IP 范围：ips 内联与普通文本 files；SRS/二进制/压缩格式、ip_set 的 sets 跨 provider 引用及文件管理热更新延期。resp_ip 的 `$named_ip_set` 引用在本轮范围内，与 ip_set 自己的 sets 不是一回事。

## Source contracts

- hosts `entries/files`，default full 域名模式。pkg/hosts/hosts.go:53 起：IN A/AAAA，已匹配且至少有一种地址时 SetReply；对应族所有地址 TTL10，缺该族返回 NOERROR + FakeSOA（TTL300，字段见 pkg/dnsutils/msg.go:144）。非 IN/其它 qtype/空地址规则/未命中不产生新响应。hosts executable 设置响应后继续 sequence，不暗自 accept；后续 has_resp/accept 决定短路。
- redirect `rules/files` 两字段规则，target FQDN。plugin/executable/redirect/redirect.go:95 起改写 query，ExecNext 后恢复 question，并前置 original→target CNAME TTL1。回传准确 terminal outcome；native 错误/取消不能因为遗留 response 变成成功。嵌套和缓存边界须在 design 显式管理。
- ttl 只有 quick `ttl N` / `ttl min-max`，uint32，0 对应无操作或未设 bound；range 先 minimum 后 maximum，因此 inverted range 合法并保留 max 最终覆盖语义，不擅自拒绝旧合法配置。pkg/dnsutils/msg.go:53 对 Answer/Authority/Additional 全部 RR（除 OPT）修改 TTL。无响应 no-op。不要引入 `type: ttl`。
- domain payload precedence，pkg/matcher/domain/matcher.go:293 起：full → domain → regexp → keyword；unprefixed 在 hosts/redirect 为 full，非 domain_set 的 default domain。相同模式后载入替换；load order 为 inline 然后 files。Go 多 regexp/keyword 命中受 map 遍历影响，native 明确采用首次登记顺序，替换同模式不改变位置，此为稳定选择，不能声称复现 Go 无序选择。
- resp_ip QuickSetup 的合法 token 在 plugin/matcher/base_ip/ip_matcher.go:133：IP/CIDR、`$tag`、`&file`；多个列表为 OR，匹配 Answer 的任意 A/AAAA，空响应 false。不得把 Authority/Additional 地址当 Answer 地址。
- ip_set `ips/files` 支持普通 IP/CIDR。pkg/matcher/netlist/load_helper.go:33：空行/注释、首 token、坏 prefix 带行号报错。ip_set.go:430 起缺文件报警跳过，而 hosts/redirect 文件缺失报错；本轮保留这些显式差异，不为统一代码静默改变行为。SRS magic/二进制或压缩内容不作为文本 fallback 接受，明确 unsupported。

## Composition hazards and frozen choices for final review

1. Original admission question 与当前执行 QueryView 分离：redirect 改写当前 wire/question，所有 nested matcher/forward/cache/preference 使用该视图；原 query audit 的入口名字不随 rewrite 变化。
2. redirect 是 scoped successor wrapper：本地 raw/header/question 副本、共享原 fuel/deadline/cancellation，结果回归同一 enclosing boundary；question 恢复与 CNAME 构建在外层 publication 之前完成。RuntimeErr/cancel 不装饰或发布；成功终态（包括 exit/accept/reject 若带 response）恢复/装饰后仍传播原终态。exit 的 cache publication 继续按现有 ScopeAborted 合同放弃。
3. cache → redirect 使用 original key 并保存恢复后的完整 response；redirect → cache 使用 target key 保存 target response，再外层恢复。两种结构都需 hit/lazy/restart 证明。
4. ttl 改 wire 的位置按 sequence：cache → forward → ttl 可在 boundary 保存修改后的 TTL；cache → forward 后、scope 外部 ttl 仅修改客户端结果。不要重新执行已经被 cache hit 短路的 successor。
5. TTL 修改不会自动延长已有 cache timestamp，用户配置在缓存返回后设置 TTL 可能使最终 lazy wire 不再 TTL5，这是显式 pipeline 行为；lazy 的自身 lookup 仍返回5。测试区分 lookup TTL与后续 policy TTL。
6. hosts 是本地新响应，清除当前 supplier selection但保留已发生 attempts；ttl/redirect 改既有 response 维持真实 supplier，不能标成另一上游。cache hit 来源按现有 Cache 契约，flow_setter 人工字段仍不能冒充真实诊断 supplier。
7. 规则 startup immutable；不产生新后台任务，不增 refresh root，不改 ECS 延期/缓存 generation/dump schema/production 门禁。

## Public test and mock boundary

公开 config compile、native request driver、真实 DNS listener、HTTP audit/详情、dump reader/restart；mock 仅受控上游、clock、临时 rules 文件。不能 mock query rewrite、publication、supplier merge 或 response codec。TDD 行为切片及依赖见 implement.md。
