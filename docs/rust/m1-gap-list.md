# M1 缺口清单：`config_all` 原生运行（2026-10-06）

来源：原生 `mosdns check` 对 `config_all`（`file` 仓库 `28c6493`，与 `86e6a65` 无差异）的
原始输出 [config_all-check.txt](validation-records/10-06-config-check/config_all-check.txt)，
以及仅用于看下一层的[派生探测](validation-records/10-06-config-check/README.md#派生探测probe)。
输入、构建与快照说明见该目录 README。本清单以 `check` 输出为准，取代
[下一阶段安排](next-stage-plan.md)中 2026-10-06 按 YAML 文本统计的 M1 表。

`SUMMARY gaps=86 cascades=80 globals=3 definitions=126`（M1-0 之前）。**M1-0 完成后**
（`10-06-rust-native-m1-0-config-loading`）对 `config_all` 与生产快照重跑，原始输出见
[validation-records/10-06-m1-0-config-loading](validation-records/10-06-m1-0-config-loading/README.md)：
`SUMMARY gaps=64 cascades=80 globals=1 definitions=126`，两份逐字节相同，stderr 为空。唯一
GLOBAL 是 `at least one forward plugin is required`（全部上游经 `aliapi`，属 M1-b）；其余
64 个直接缺口全部是下文 M1-a/b/c 的插件类型或 sequence 项，已无加载/listener/路径类缺口。
**M1-a1 完成后**（`10-07-rust-native-m1-a1-rule-providers`）：`SUMMARY gaps=45 cascades=66 globals=1`，
M1-a 的全部插件缺口（`domain_set_light`、`sd_set_light`、`domain_mapper`、`si_set`，以及移入本批的
`adguard_rule`）和它们的连带已关闭，原始输出与差分见
[validation-records/10-07-m1-a1-rule-providers](validation-records/10-07-m1-a1-rule-providers/README.md)。
剩余直接缺口：sequence 项 27、`domain_output` 8、`aliapi` 5、`fallback` 裸 tag 2、`rewrite`/`webinfo`/`requery` 各 1。
**M1-b2 完成后**（`10-07-rust-native-m1-b2-primitives`）：`SUMMARY gaps=16 cascades=66 globals=1`，
sequence 里的 `rcode`（6）、`tag_setter`（10）、`drop_resp`（10）、`sleep`（1）和 `fallback` 裸 tag（2）
全部关闭（直接缺口 45 → 16；`config_all` 与生产快照输出仍逐字节相同），原始输出见
[validation-records/10-07-m1-b2-primitives](validation-records/10-07-m1-b2-primitives/README.md)。
剩余直接缺口：`domain_output` 8、`aliapi` 5、`rewrite`/`webinfo`/`requery` 各 1；连带数不变（66）是因为
`sbnodefallback` 的连带消失、`prefer_foreign_nocache`（其 `primary: foreign` 已能解析）由直接缺口变成连带。
**M1-a2 完成后**（`10-07-rust-native-m1-a2-rule-management`）：`check` 结果不变（`SUMMARY gaps=16 cascades=66 globals=1`，
两份输出逐字节相同，运行前后目录不变）——本批不关闭 `check` 缺口，而是补齐 `check` 看不到的管理面：
`sd_set_light`/`si_set`/`adguard_rule` 的在线来源管理、下载（含 SOCKS5）、自动更新，`domain_set_light` 的
show/save/post，以及 `domain_mapper`/`si_set`/AdGuard 直接 matcher 的热更新；两个 Vue shell 的规则页在原生下可用。
记录见 [validation-records/10-07-m1-a2-rule-management](validation-records/10-07-m1-a2-rule-management/README.md)。
**M1-b1 完成后**（`10-08-rust-native-m1-b1-upstreams-overrides`）：`SUMMARY gaps=9 cascades=30 globals=1`（两份输出逐字节相同；含 M1-g 后为 `globals=0`）。`aliapi` 5、`rewrite` 1、
`webinfo` 1 个直接缺口关闭，GLOBAL `at least one forward plugin is required` 消失；剩余直接缺口只有 `domain_output` 8、`requery` 1（M1-c）。
GLOBAL 一度被缓存放置规则 `unsafe cache … successor may execute ecs_handler` 挡住（§1 **M1-g**），该规则按固定/客户端推导 ECS 修正后 GLOBAL 归零（`gaps=9 cascades=30 globals=0`）。范围按规划会话 2026-10-08 的调整执行：**运行时覆盖文件的应用（R1
`config_overrides.json`、R2 `upstream_overrides.json`）暂停**，等设置层重新设计后再做，见 §1 的 **M1-f**；`aliapi` 的取值只来自 YAML，并通过
`AliApiArgs::build` 这一个入口构造上游列表与 SOCKS5 路由。记录见
[validation-records/10-08-m1-b1](validation-records/10-08-m1-b1/README.md)，契约见
[native-aliapi-rewrite-webinfo](contracts/native-aliapi-rewrite-webinfo.md)。
M1-0 之前的读法如下：

- **直接缺口（GAP/GLOBAL）** 是要做的事；**连带错误（CASCADE）** 只因引用了被剔除的定义，
  对应插件补齐后自然消失，不单独排期。
- 每个定义（或 sequence 的每个 matcher/exec/rule 项）只报第一个错误；被剔除的定义其参数
  未被检查。补齐一批后须重跑 `check`，直至 `SUMMARY gaps=0 cascades=0 globals=0`。
- `check` 只覆盖 YAML 编译；WebUI/API/持久化工作流不在其输出中，M1 仍需按
  [重写方案第 6 节](../ai/rust-rewrite-plan.md)补齐这些面的原生证据。

## 1. 直接缺口（按批次）

### M1-a 规则 provider 与数据格式（**查询侧由 M1-a1、管理侧由 M1-a2 关闭**）

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `domain_set_light` | 11 | `rule_set.yaml` |
| 插件类型 `sd_set_light` | 4 | `cusnocn`、`cuscn`、`geosite_cn`、`geosite_no_cn`；`local_config` 指向 `srs/*.json` |
| 插件类型 `domain_mapper` | 2 | `unified_matcher1`、`special_upstream_matcher`（后者在 `special_groups.yaml`） |
| 插件类型 `si_set` | 1 | `geoip_cn`（`srs/geoipcn.json`）；连带 10 处 `resp_ip $geoip_cn` |

### M1-b 上游、响应与 sequence 项

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `aliapi` | 5 | **M1-b1 已关闭**：`domestic`、`foreign`、`foreignecs`、`nocnfake`、`cnfake`；标准 DNS 上游（udp/tcp/tls/https，tcp/tls/https 可走 SOCKS5），签名 AliDNS API 加载时明确报不支持 |
| 插件类型 `rewrite` | 1 | **M1-b1 已关闭**（含 `/show`、`/post`） |
| 插件类型 `adguard_rule` | 1 | |
| 插件类型 `webinfo` | 1 | **M1-b1 已关闭**（`GET`/`PUT /plugins/<tag>`；`clientname` 同时点亮 WebUI 的 `client.aliases` 能力） |
| exec `tag_setter` | 10 | **M1-b2 已关闭** |
| exec `drop_resp` | 10 | **M1-b2 已关闭** |
| matcher `rcode` | 6 | **M1-b2 已关闭** |
| exec `sleep` | 1 | **M1-b2 已关闭**（`for_singbox.yaml` 中的 `sleep 1000`） |
| `fallback` 的 `primary`/`secondary` 写裸 tag | 2 | **M1-b2 已关闭**：现在同时接受裸 tag 与 `$tag`（Go 只认裸 tag） |

### M1-f 类型化设置层（新增；设计已定，用户 2026-10-08 决定）

`check` 看不到、但决定生产实际行为的一块：Go 在加载配置时应用 `webinfo/` 下两份运行时覆盖文件，原生非托管模式目前**两份都不应用**。
生产快照的实际效应：`config_overrides.json`（`socks5`、`ecs`、`replacements`）按递归字符串/键替换改写所有插件参数（`socks5` 键的值
全部替换，包括 `socks5: ""`；`ecs …` 串替换为 `ecs <ecs>`）；`upstream_overrides.json` 按 `aliapi` tag **替换**该插件的上游列表（只取
`enabled` 条目，`concurrent` 取启用条目数，`use_socks_proxy` 推断，`foreign`/`foreignecs` 回退到全局 `socks5`）。因此用 YAML
原样启动的原生实例，`domestic`/`foreign` 等会用 YAML 里的上游，而 Go 实际用覆盖文件里的上游；上游/覆盖的管理 API（WebUI 上游页、覆盖设置页）
同样未实现。语义取证已写入任务 `research/go-semantics.md`；待设置层方案确定后，由新的设置层通过 `AliApiArgs` 提供值。

**设计决定（2026-10-08）**：Rust 版是新项目，配置文件允许改动、不要求兼容 Go。原生**不读取** Go 的
`upstream_overrides.json`、`config_overrides.json` 与通用 `replacements`。WebUI 可调整的值改为配置中的具名类型化对象
（如 `socks5_proxy`、`ecs_address`），由上游条目、规则源等用 `$tag` 显式引用（如 `proxy: $main_proxy`）；WebUI 通过类型化 API
修改，只把与 YAML 默认值不同的部分写入一个带版本号的 `webinfo/settings.json`（按 tag）。生效配置 = YAML + 设置，由一个函数计算，
`check` 与只读 API 可见并标注来源；修改经整代发布热生效，失败保留旧配置。范围另含 `domain_output` 的生成开关（取代 Go 的 `domain_generation_settings.json`，M1-c1 留有钩子）。首次替换使用由 `config_all` 改写的原生配置包
`config_rust`（维护在 `file` 仓库，附与 `config_all` 的差异说明），见重写方案第 6 节。

### M1-g 缓存放置规则与 ECS（**M1-b1 任务已处理**；被 M1-b1 暴露，用户 2026-10-08 决定）

原先 `cache_placement.rs` 拒绝“`cache` 的后继可能执行 `ecs_handler`”，`config_all` 的 `cache_all`/`cache_node` 之后经 `fallback` 到达写死
`ecs 2408:8888::8` 的节点而无法启动。现在只拒绝**依赖客户端**的 ECS（`forward: true`；`send: true` 且无 `preset`），固定地址与无地址 handler
被接受，`client_ip` 分支仍拒绝；这是让判断更准，不是放宽安全（固定 ECS 对所有客户端相同，缓存答案不会因它出错）。契约见
[native-client-ecs](contracts/native-client-ecs.md) S4。`check`（`config_all` 与生产快照）GLOBAL 归零，未暴露新缺口。

### M1-c 输出与再查询

**M1-c2 完成后**（`10-08-rust-native-m1-c2-requery`）：`SUMMARY gaps=0 cascades=0 globals=0 definitions=126`（`config_all` 与生产快照逐字节相同）。`check` 对 `config_all` 不再有缺口，可进行首次整链运行；M1-f/M1-e 等仍按各自条目推进。记录见 [validation-records/10-08-m1-c2](validation-records/10-08-m1-c2/README.md)。

**M1-c1 完成后**（`10-08-rust-native-m1-c1-domain-output`）：`SUMMARY gaps=1 cascades=0 globals=0`（`config_all` 与生产快照逐字节相同），唯一剩余缺口是 `requery`（M1-c2）；`domain_output` 的 8 个缺口及其 30 个连带错误全部消失。记录见 [validation-records/10-08-m1-c1](validation-records/10-08-m1-c1/README.md)。

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `domain_output` | 8 | **M1-c1 已关闭**（统计、文件互通、发布、`target_mapper` 即时生效、API；见 [契约](contracts/native-domain-output.md)） |
| 插件类型 `requery` | 1 | **M1-c2 已关闭**（见 [契约](contracts/native-requery.md)） |

### M1-d 配置加载与 listener（新增，原 M1 表未列；**M1-0 已全部关闭**）

| 缺口 | 数量 | 关闭方式 |
| --- | --- | --- |
| `log.file` 字段 | 1 | `log.file` 追加写入（相对路径按配置根目录），其余 `log` 字段仍报不支持 |
| `log.level: warn` | 1 | 接受 `debug`/`info`/`warn`/`error`，非法值回退 `info` 并警告 |
| `listen: ":port"` 形式 | 20 | `:port` 绑定 IPv6 通配并清除 `IPV6_V6ONLY`（双栈；无 IPv6 时回退 `0.0.0.0` 并写诊断） |
| `enable_audit` 必填 | 10 | 改为可选，默认 `false` |
| 多个 listener 需 `native_management` | 9（探测可见） | 去掉单 listener 限制；同协议同地址重复仍拒绝 |
| `tcp_server` 的 `idle_timeout` 必填 | 9 | M1-0 重跑 `check` 时才显现（此前被 `listen` 错误遮住）：改为可选，默认 10 秒（Go 默认值） |
| 相对路径基准 | 见 §3.1 | 配置根目录规则，`-d`；见下 |

## 2. 原生已接受的 `config_all` 用法

`check` 在这些用法上没有报错（不等于产品验收）：`sequence` 控制流（`exit`、`reject`、
`try`、`$ref`）、`qtype`、`rcode`、`tag_setter`、`drop_resp`、`sleep`、`fast_mark`（matcher/exec）、`flow_setter`、`switch1`–`switch17`
声明与 quick matcher、`client_ip`、`resp_ip`（字面量/文件形式）、`ttl`、`cname_remover`、
`prefer_ipv4`、quick `ecs`、`cache`（7 个，含 `dump_file`）、`ip_set`（2 个）、`api.http`。
M1-0 之后还包括：2 个 `domain_set`（`gen/*.txt`）、全部 20 个 `udp_server`/`tcp_server`
listener、`log.level`/`log.file`。`switch*`、`cache`、`ip_set`、`domain_set` 的相对路径现按
配置根目录解析（§3.1）。

原 M1 表列出的 `cname`、`mark` 在 `config_all` 中**未使用**；`cname_remover` 已原生接受。

## 3. 非插件差异（需决策）

### 3.1 相对路径基准（**M1-0 已解决**）

> 状态：已按下述用户决定实现。插件参数里的相对路径一律按配置根目录解析（`-d <dir>`，未给
> `-d` 时为主配置文件目录——有意偏差：Go 此时用进程当前目录）；`include` 路径与
> `webinfo`/托管状态根仍是主配置文件目录。`start` 与 `check` 共用同一规则，托管候选编译
> 使用启动时的同一个根。逐点审计见
> [research/path-audit.md](../../.trellis/tasks/10-06-rust-native-m1-0-config-loading/research/path-audit.md)（本地 Trellis）。
> 以下为决策前的背景描述。

原生按**声明文件所在目录**解析 include 文件里的相对路径（5B 设计，见
[feature-coverage C01](feature-coverage.md)）；`config_all` 依赖 Go 以**配置根目录**
（进程工作目录 `/cus/mosdns`）解析。于是 `sub_config/*.yaml` 中的 `rule/…`、`gen/…`、
`srs/…`、`cache/…` 都被解析到 `sub_config/` 下：

- `domain_set` 的 `gen/*.txt` 报缺失（2 处 GAP）；
- `ip_set` 的 `rule/direct_ip.txt`、`rule/client_ip.txt` 被当作缺失文件跳过（stderr 警告），
  **不报错但结果不同**；
- `switch*` 的 `initial_value: rule/switchN.txt` 与 `cache` 的 `dump_file: cache/*.dump`
  会**静默**指向 `sub_config/rule/`、`sub_config/cache/`。

这是产品契约问题，不是插件缺口。**用户决定（2026-10-06）：改原生行为，与 Go 一致，
所有相对路径（含 include 文件内的规则文件、switch 状态文件、cache dump 等）都从配置
根目录解析**；配置包不改。这推翻了 5B 起“按声明文件目录解析”的原生设计，需要同步更新
C01 契约描述和相关测试。依赖路径的插件在该改动完成后再实现。

### 3.2 `native_management`（**M1-0 已解决**）

> 状态：多 listener、`:port`、可选 `enable_audit`/`idle_timeout` 与 `sub_config/special_groups.yaml`
> 作为普通 include 均已在不开启托管时可用；不开启时不读写 `webinfo`、不生成或改写
> `special_groups.yaml`（测试对目录做前后快照）。`aliapi` 描述符规范化与 special_groups
> 生成仍只在托管时执行；`aliapi` 的原生支持属 M1-b，`domain_mapper` 属 M1-a。
> 以下为决策前的背景描述。

`config_all` 未开启 `native_management`，而原生的多 listener、`aliapi` 描述符规范化与
special_groups 生成都挂在该开关上。**用户决定（2026-10-06）：原生在不开启
`native_management` 时也必须接受 `config_all` 的这些形态**；配置包不增加该开关。
实施时需逐项确认哪些能力与开关解耦（多 listener、`aliapi`、`special_groups.yaml`
的处理等），以及解耦后不开开关时的托管/写入行为保持只读或与 Go 一致。

## 4. 生产快照与 `config_all` 的差异

对生产快照（Go v0.7.3，`/cus/mosdns`）的 `check` 输出
[prod-snapshot-check.txt](validation-records/10-06-config-check/prod-snapshot-check.txt)
与 `config_all` 的**逐字节相同**。快照与包内 `config_all` 的配置差异只有：
`sub_config/special_groups.yaml` 多一个空行（运行时生成）以及两个不被 include 的
`.bak` 文件。因此本清单同样适用于当前生产配置。

## 4.1 合并 main v0.7.3 后新增：M1-e special_groups v0.7.3 对齐

`10-06-rust-merge-main-v0.7.3` 把 Go v0.7.3 合入后登记了两项原生契约漂移（详见
[drift.md](validation-records/10-06-merge-main-v0.7.3/drift.md)），不改变 `check` 对
`config_all` 的结果，原生实现排在 M1-a/b/c 之后：

| 项 | 内容 |
| --- | --- |
| D2 | v0.7.2 special_groups 生成结构（共享 `sequence_special`、`domain_mapper`/`mark` 路由、`switch4` 控制 cache、`flow_setter … upstream=`）与原生托管生成器输出的差异；先由 M1-a 补 `domain_mapper`/`mark` 使非托管形态可加载 |
| D3 | 专用组上游引用/私有上游/刷新/无效引用告警、`/api/v1/upstream/sources`、`special_upstream_groups.json` 新字段；原生 UI 已通过 `groups.upstreams` 能力显式禁用 |

## 5. 不在 M1 的注册项

`config_all` 未使用、延后到首次替换之后：见 [feature-coverage](feature-coverage.md)
第 7 节“M1 范围标注”中标为“替换后”的行。
