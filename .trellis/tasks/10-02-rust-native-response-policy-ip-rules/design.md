# Native response policy design

完整规划已于2026-10-02获用户批准，按每切片独立C2C PASS门控实施。合同来源与特殊边界见 research/policy-contracts.md。

## Architecture

编译阶段分开两个 catalog：immutable typed payload domain rules（hosts/redirect），与 IP matcher snapshots（named ip_set、resp_ip 匿名列表）。复用 matcher-core 的 IP/index/域匹配规范，允许增加 native typed payload facade；不扩大 hybrid ABI、不调用 Go mirrors。named 插件引用仍 dispatch copyable executable ID；quick ttl 编译为有类型 policy descriptor，不把每次请求解析字符串作为实现。

domain 规则规范化和 category 优先级复用现有 Go golden fixtures/项目 domain_set contract；payload 筛选为 full → 最具体 domain suffix → regexp → keyword。同 regexp/keyword 多匹配按首次登记顺序确定，重复同 pattern 替换 payload、不换位置；inline 先 files 后。regex 采用项目现有 Rust regex 支持范围，超范围报配置错误，不悄悄退 Go。hosts 空地址 payload 不应误匹配成“去下一条规则”，按已匹配规则无新 response 处理。

IP provider immutable snapshot 在 startup 建成。`ip_set.ips/files`，默认空列表；sets 仅缺省/空允许，非空明确 unsupported。resp_ip 支持多个 IP/CIDR/`$named`/`&text_file` OR；错类型 tag/未知引用/空表达式编译错误。IPv4、IPv6 与 mapped 地址按 matcher-core/Go netlist golden contract处理，避免 string 比较。首 token 文本文件、# 注释、行号错误及缺失文件告警跳过保留 Go 合同；hosts/redirect 文件缺失则启动失败。

路径沿现有 RawPlugin.base_dir，包括 include 的声明目录，不使用偶然 CWD。文本 UTF-8，单行 <=64KiB；每插件所有 inline+files 合计 <=64MiB 与1000000规则，超限编译错误。SRS magic、gzip/zlib magic、NUL/坏 UTF8 明确 unsupported；不以文件后缀作为唯一判断。快照失败不部分发布，告警跳过的缺文件仍构建剩余合法列表。

## QueryView and scoped redirect

新增/统一 request-local current QueryView（owned/Rc raw wire、parsed question/header）与 immutable admission question。初始相同；redirect 只改自己的子视图。network exchange、cache begin_store/lookup、当前 query matcher、hosts、prefer 的 reference 构建都读取此视图；原 audit ID/admission qname不改变。不能仅改 ExecutionState 的显示 qname 或只改 parse metadata。

redirect 先验证 qclass/匹配规则，未匹配走既有 successor。匹配则保存 original name，安全解码/重建 query 为 target，保持 ID/qtype/class/flags及允许的 OPT，捕获 successor 到 enclosing boundary，用同一个 root fuel/deadline/取消树驱动。重定向实例嵌套每层有自己的 return frame，原父状态不原地修改。self redirect/跨 plugin cycle 不作 OS lookup，不新建无限预算；有限 root fuel 耗尽走现有失败终态。

successor 带 response 且没有 runtime error/cancel 时，原子重建 question 为本层 original并前置 CNAME(original,target,IN,TTL1)，保持其余 answer/authority/additional、flags/ID/rcode；先完整校验后安装，不能中途修改字节破坏 compression pointers。已存在该 CNAME 不去重（保持配置层次）；每层精确恢复对应 question。无 response 不凭空制造成功；runtime error/cancel/drop 不装饰遗留 response，原 outcome 继续传播。成功 exit/accept/reject 装饰后保留原 completion kind，不能转为自然完成使缓存错误发布。

外部 redirect publication 捕获恢复后的 wire；内部 target cache 保存 target wire。watch/token 完成顺序必须跟外层 restore frame 对齐，不消费另一 branch 的 token。若现有 capture successor 在 cache boundary 前后不能实现此顺序，允许增加 typed machine restoration/boundary notification，但不得削弱上一任务的 exactly-once / ScopeAborted 合同。

## Hosts and TTL wire policy

hosts IN A/AAAA 根据 payload 制造 SetReply 等效 response，RR TTL10，多地址保持配置顺序；域名匹配但缺请求族则 NOERROR、Answer空、FakeSOA 完整字段与 Go一致（TTL300）。匹配规则无任何地址、不匹配、非 IN/其它 qtype 不设置新 response，保留既有 response。成功设置响应后仅 Continue，不隐式 accept。Local facts 清 supplier selection，保留实际 attempts/routing domain_set。

ttl quick 语法为 uint32 fixed 或 min-max，缺参数/负数/溢出/多余参数报配置错误。fixed=0 no-op；range 0 表未设边界，先 min 后 max，包括 min>max。对当前 Raw response 完整解析后修改所有非 OPT RR TTL；优先复用 dns-core wire TTL offsets 的验证遍历，不为 TTL 改写其它 flags/rcode/question。无 response no-op，畸形 wire 按既有 runtime error contract失败、保持原 wire，不部分变更。该 policy 不修改 cache 原始 wall/mono 时间戳或 dump。

## Facts and composition

hosts 新响应设置 ResponseSource::Local；redirect/ttl 继承真实 wire supplier，继承该 successor 的 branch facts 与 upstream attempts。不同 sibling不能吸收取消方的 selected supplier。原 query question/filters仍用 admission数据，最终 answers/flags/rcode 使用最终 wire，CNAME可在现有 Vue详情直接展示；不加新 schema。

后台 cache refresh 使用同样 QueryView/restore frame，受既有64fuel/5s约束，没有新的 detached task；缓存命中仍按原作用域短路。测试分别证明 hosts→accept、cache→hosts→accept、cache→redirect→forward、redirect→cache→forward、两者嵌套、TTL在cache内/外、fallback/prefer所有正常与取消终态。

### Explicit sequence and final-wire examples

执行顺序由 YAML sequence 决定，不增加固定的 policy→cache→fallback 全局阶段。`resp_ip` 是只读 matcher，不生成/改写响应；它的判断可以选择后续 sequence，但不能自行写 cache。

| 配置结构 | 执行与缓存结果 |
| --- | --- |
| hosts → has_resp/accept → forward | hosts 命中后 accept，forward 不执行；未命中按配置继续 |
| hosts → forward（无短路） | hosts 设置后 Continue，实际执行的 forward 可替换旧 response；最终 supplier 随最终 wire 更新 |
| cache → hosts → accept | hit 短路 hosts；miss 的有效本地响应在该 cache 自己的合法 completion boundary 入库，不按 Local 来源一律排除 |
| cache → redirect → forward | original-name key；miss 保存 question 恢复后的 CNAME+answer；hit 不重新 redirect |
| redirect → cache → forward | target-name key；cache hit 可供应 target response，本层 redirect 仍恢复 original question并前置 CNAME |
| policy → conditional fallback | 是否调用、替换旧响应由 matcher/sequence及已有fallback合同决定，不凭插件种类设优先级；fallback terminal error继续失败 |

客户端问 `a.example A`，本层重定向到 `b.example`，上游 Answer 为 `b.example CNAME d.example`、`d.example A address`：最终 Question=`a.example A`，Answer=`a.example CNAME b.example`(TTL1) + 原 `b.example CNAME d.example` + 原地址记录。不扁平化为 a→d，不把已有 owner name 全部改成 a。原上游 RR TTL不重新起算，只有显式 ttl executable或cache年龄处理才能改变它。

target NXDOMAIN/NOERROR-empty：保留其 RCODE、Authority SOA（含owner/字段/TTL）及其它记录，Question恢复到original；仍按旧redirect合同前置a→b CNAME。不得把negative response改成positive A或凭空改SOA归属。无response/错误/取消按前述terminal合同，不生成CNAME补成成功。

### Synthetic cache and rule changes

本轮沿既有普通cache key/admission合同缓存合法wire，key不新增policy identity，dump也不新增policy generation。hosts响应是否入库由cache placement及publication boundary决定，仍受TC/question/rcode/exclude_ip/owner generation等已有条件约束；没有“所有synthetic默认禁入”的新默认。

规则只在启动时载入。运行中修改/删除磁盘规则不改变当前snapshot，后台DNS refresh也使用该owner所绑定的同一snapshot，不读文件或清空provider。重启若保留原dump并修改policy，同key旧条目仍可能在retention/lazy window内命中；不承诺policy删除立即废除旧缓存。要求立即生效的配置变更需在停止查询admission、无新请求回填的条件下完成已有flush再切换，或完成停机/drain/final-save后移除对应dump，再启动新配置；运行中先删文件再普通重启可能被最终save重写，不能作为安全步骤。新配置开始接受查询前不得载入旧snapshot。规则版本化缓存失效/热更新整体留后续；不可偷偷加入不同dump/key版本。本兼容限制须出现在最终规划摘要和restart/flush验证中。

### Startup file outcomes

| 情况 | ip_set/resp_ip匿名IP文本 | hosts/redirect规则文本 |
| --- | --- | --- |
| 空文件/仅注释 | 有效空snapshot | 有效空snapshot |
| 文件不存在 | 告警跳过此文件，保留其它合法规则 | 编译失败，不启动candidate |
| 权限/I/O/无效UTF8/坏文本/超限 | 编译失败，带路径/可用行号 | 编译失败，带路径/可用行号 |
| SRS/压缩/二进制内容 | 明确unsupported，不fallback成文本 | 明确unsupported |

无runtime文件刷新或refresh-failure替换provider路径。后台cache刷新失败保留原lazy cache value，不能与文件IP snapshot生命周期混为一谈。

## Validation, errors and deferred work

范围内所有配置错误给出原配置/规则行位置；禁止 silent unsupported。named hosts/redirect/ip_set没有新 quick aliases或管理 API；ttl不新增 named插件。ip_set sets非空、SRS/压缩/二进制、ECS、client_ip、热更新与文件管理页均延期。

新规则查找不逐 query I/O；startup limits与重复模式/regex稳定选择列为兼容摘要，最终审批涵盖这些有限实现边界。不声称整个IP provider格式或5B/5C完成。

变更仅查询相关 Rust/package specs与必要证据，不改 Go执行行为。验证在 mosdns-rust任务独占目录；实际 browser消费原生HTTP最终DNS数据。回退只针对本任务新增能力，不删除缓存/转发已有交付。

## Final approval — 2026-10-02

Human explicitly approved the latest complete planning summary with “批准”. Approval covers S1–S6, the frozen IP/text-only scope and retained-dump limitation, inline execution and same-chat per-slice C2C PASS before continuing. No repeated approval is needed for this unchanged scope. Planning/review gates are satisfied; task activation has not run because this turn still carries an explicit stay-in-planning workflow directive. This is a recorded approval, not implementation evidence.

## Current execution status — 2026-10-02

The final plan was explicitly approved by the human. Task activation succeeded after correcting the conditional planning breadcrumb; historical planning-only notes above describe earlier sessions. S1/S2/S3 (including jump-continuation supplemental correction) have independently passed exact-commit C2C review. S4 real response-IP matcher is undergoing final validation/review; S5/S6 remain pending. Every next slice stays gated by the preceding PASS. No push or deployment.

## S6 final evidence candidate

S1–S5 independently passed C2C before the next slice. S6 has actual UDP/TCP, public API, maintained Vue details, real SIGTERM/process restart and retained-dump/Flush proof; Linux workspace 1,070 tests pass with no ignored tests, fmt/clippy pass. See research/public-proof/README.md. Whole-task exact-commit final review remains pending. No new product behavior, production promotion or push.
