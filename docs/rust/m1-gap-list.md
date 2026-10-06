# M1 缺口清单：`config_all` 原生运行（2026-10-06）

来源：原生 `mosdns check` 对 `config_all`（`file` 仓库 `28c6493`，与 `86e6a65` 无差异）的
原始输出 [config_all-check.txt](validation-records/10-06-config-check/config_all-check.txt)，
以及仅用于看下一层的[派生探测](validation-records/10-06-config-check/README.md#派生探测probe)。
输入、构建与快照说明见该目录 README。本清单以 `check` 输出为准，取代
[下一阶段安排](next-stage-plan.md)中 2026-10-06 按 YAML 文本统计的 M1 表。

`SUMMARY gaps=86 cascades=80 globals=3 definitions=126`。读法：

- **直接缺口（GAP/GLOBAL）** 是要做的事；**连带错误（CASCADE）** 只因引用了被剔除的定义，
  对应插件补齐后自然消失，不单独排期。
- 每个定义（或 sequence 的每个 matcher/exec/rule 项）只报第一个错误；被剔除的定义其参数
  未被检查。补齐一批后须重跑 `check`，直至 `SUMMARY gaps=0 cascades=0 globals=0`。
- `check` 只覆盖 YAML 编译；WebUI/API/持久化工作流不在其输出中，M1 仍需按
  [重写方案第 6 节](../ai/rust-rewrite-plan.md)补齐这些面的原生证据。

## 1. 直接缺口（按批次）

### M1-a 规则 provider 与数据格式

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `domain_set_light` | 11 | `rule_set.yaml` |
| 插件类型 `sd_set_light` | 4 | `cusnocn`、`cuscn`、`geosite_cn`、`geosite_no_cn`；`local_config` 指向 `srs/*.json` |
| 插件类型 `domain_mapper` | 2 | `unified_matcher1`、`special_upstream_matcher`（后者在 `special_groups.yaml`） |
| 插件类型 `si_set` | 1 | `geoip_cn`（`srs/geoipcn.json`）；连带 10 处 `resp_ip $geoip_cn` |

### M1-b 上游、响应与 sequence 项

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `aliapi` | 5 | `domestic`、`foreign`、`foreignecs`、`nocnfake`、`cnfake`；全部上游都经它定义，剔除后无任何 forward（探测层 2） |
| 插件类型 `rewrite` | 1 | |
| 插件类型 `adguard_rule` | 1 | |
| 插件类型 `webinfo` | 1 | |
| exec `tag_setter` | 10 | |
| exec `drop_resp` | 10 | |
| matcher `rcode` | 6 | |
| exec `sleep` | 1 | `for_singbox.yaml` 中的 `sleep 1000` |
| `fallback` 的 `primary`/`secondary` 写裸 tag | 2 | 原生只接受 `$tag`；Go 接受 `primary: foreign` 形式 |

### M1-c 输出与再查询

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| 插件类型 `domain_output` | 8 | |
| 插件类型 `requery` | 1 | |

### M1-d 配置加载与 listener（新增，原 M1 表未列）

| 缺口 | 数量 | 说明 |
| --- | --- | --- |
| `log.file` 字段 | 1 | GLOBAL |
| `log.level: warn` | 1 | 原生只接受 `error` |
| `listen: ":port"` 形式 | 20 | 全部 udp/tcp listener；原生只接受数字 IP |
| `enable_audit` 必填 | 10 | 探测层 2：子配置中的 10 个 listener 未写该字段；Go 为可选 |
| 多个 listener 需 `native_management` | 9（探测可见） | 探测层 2：未开启托管时只允许一个 listener；补齐 `enable_audit` 后预计为 19 |
| 相对路径基准 | 见 §3.1 | 原配置中 `domain_set` 2 处因此报文件缺失 |

## 2. 原生已接受的 `config_all` 用法

`check` 在这些用法上没有报错（不等于产品验收）：`sequence` 控制流（`exit`、`reject`、
`try`、`$ref`）、`qtype`、`fast_mark`（matcher/exec）、`flow_setter`、`switch1`–`switch17`
声明与 quick matcher、`client_ip`、`resp_ip`（字面量/文件形式）、`ttl`、`cname_remover`、
`prefer_ipv4`、quick `ecs`、`cache`（7 个，含 `dump_file`）、`ip_set`（2 个）、`api.http`。
`config_all` 的 2 个 `domain_set` 实例都因相对路径基准报直接缺口（§3.1），不在此列。
`switch*`、`cache`、`ip_set` 虽被接受，其相对路径同样受 §3.1 影响。

原 M1 表列出的 `cname`、`mark` 在 `config_all` 中**未使用**；`cname_remover` 已原生接受。

## 3. 非插件差异（需决策）

### 3.1 相对路径基准

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

### 3.2 `native_management`

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

## 5. 不在 M1 的注册项

`config_all` 未使用、延后到首次替换之后：见 [feature-coverage](feature-coverage.md)
第 7 节“M1 范围标注”中标为“替换后”的行。
