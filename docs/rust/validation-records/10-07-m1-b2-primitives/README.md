# 10-07 M1-b2：`rcode` / `tag_setter` / `drop_resp` / `sleep` / `fallback` 裸 tag 的验证记录

任务 `10-07-rust-native-m1-b2-primitives`。均为无状态、请求内的 matcher/executable。

## check

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` | `mosdns check -d <root> -c <root>/config_custom.yaml` 的 stdout（退出码 1） |
| `prod-snapshot-check.txt` | 对生产快照（Go v0.7.3，`/cus/mosdns`，仓库外保存）同样运行的 stdout（退出码 1），与上一份逐字节相同 |

`SUMMARY gaps=16 cascades=66 globals=1 definitions=126`（M1-a1 之后为 `gaps=45 cascades=66 globals=1`）。

本任务的缺口全部消失：`rcode` 6、`tag_setter` 10、`drop_resp` 10、`sleep` 1，以及 `fallback`
的 `primary`/`secondary` 裸 tag 2（`prefer_foreign_nocache`、`sbnodefallback`）。直接缺口 45 → 16，剩余：
`domain_output` 8、`aliapi` 5、`rewrite`/`webinfo`/`requery` 各 1；唯一 GLOBAL 仍是
`at least one forward plugin is required`（上游全部经 `aliapi`）。连带数仍为 66：`sbnodefallback` 不再被剔除，
其下游 `sequence_sbnode` 的连带消失；`prefer_foreign_nocache` 的 `primary: foreign` 现在能解析，它由直接缺口变成连带
（`foreign` 是 `aliapi`）。

输入：`file` 仓库 `mosdns/config/config_all` HEAD `28c6493`；`config_custom.yaml` sha256
`ce0b4d857e7371d42021738a7eba6715e1ac9407257fdbc1bbb8a826329f211a`（与生产快照相同）。两份配置依次复制到 VM
同一临时目录 `/root/b2chk/root`（因此输出中的路径标签相同），`cd /root` 后运行，结束后删除副本。输出只含 tag、
类型、字段路径与编译器原因，无凭据值。`config_all` 运行的 stderr 只有 adguard 缺少已下载 `*.rules` 的告警（Go
启动时联网下载，原生不下载）；生产快照 stderr 为空。构建：`mosdns-rust` VM debug `mosdns`，sha256
`0b7f1d036b065e15cb767a4d4e9c080c4de93bd682f0e3b22e73d459d4815ad4`（源码 `97bc97a7` + 本任务工作树）。

## 语义取证与分级

见任务内 `research/go-semantics.md`。要点：

- `rcode`：响应码 = 头部低 4 位 | OPT 扩展码（最后一个 OPT），`reject N` 的合成码原样比较（至 0xFFF）；无响应/无法解码为不匹配；
  无参数被 Go 接受且永不匹配（原生同）。
- `tag_setter`：整段参数（可含空格）写入审计 `domain_set`（覆盖），空参数清除；`effective_tag` 由它派生；不碰其他 routing
  字段。Go 源码注释称 `tag_setter ""` 清空，但实际 `""` 是两个引号字符，会被当作标签存入——原生与实际行为一致（待用户确认，见任务报告）。
- `drop_resp`：只清响应；有意的审计偏差——响应被清除（或被本地 `reject` 取代）后不再把先前上游报告为最终/选中上游（响应来源记为 local）。
- 审计路由事实：响应已提交后、同一响应仍有效期间写入的路由字段（`tag_setter`、`flow_setter` 等，常见于 `sleep`、cache
  作用域边界或响应策略之后）现在逐字段叠加到该响应的路由快照上，不再丢失；这是 `config_all` 里 `$sequence_local`（含 cache）
  之后接 `tag_setter … exit` 的真实形态（审查发现，已有回归测试）。
- `sleep`：quick、命名（`duration`）、`$_sleep_500ms`；等待上限为请求期限，响应请求取消，不派生任务；非正数不等待。
- `fallback`：目标同时接受裸 tag（Go 形式）与 `$tag`。

## 用户决定（2026-10-08）

- `qname` 命中写审计标签的策略与 Go 不同（Go 先到先得；原生为最终命中的规则决定），属既有行为，
  `config_all` 与生产配置均未使用 `qname`：保持现状，记为已知差异，首次替换后再统一处理。
- quick `tag_setter ""`：按 Go 实际行为把两个引号字符存为标签（Go 注释所称的“清空”并未实现），不改。
