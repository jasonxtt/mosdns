# main v0.7.3 → rust：契约漂移登记

合并源 `75f26677`（v0.7.3），`rust` 此前基于 v0.7.1。**只登记，不实现原生功能**；原生实现归入
[M1 缺口清单](../../m1-gap-list.md) 的新批次 **M1-e**（排在 M1-a/b/c 之后）。

| 编号 | Go 新行为 | 原生现状 | 影响 | 归属 |
| --- | --- | --- | --- | --- |
| D1 | v0.7.2：配置结构升到 schema `4` / 包 ID `main-config-schema-4`；`coremain/config_update.go` 的 `requiredConfigSchema`/`requiredConfigPackageID` 随之改变，二进制更新事务会要求匹配的外部 `config_up.zip` | 原生没有配置包升级/更新事务（C07/C15），不读取也不校验 schema 常量；`config_all` 本身已是 schema 4（生产 v0.7.3） | 无运行影响；首次替换前原生必须能在不依赖 Go 更新事务的情况下被部署，并保证不会让 Go 侧校验把原生部署的配置判成过期 | C07/C15 切换门槛（执行顺序第 5 项）；发布/回滚方案里记录 |
| D2 | v0.7.2 生成的 `special_groups.yaml`：每组 `sequence_special_<slot>`（`switch4 'A'` 控制 cache、`flow_setter … upstream=special_upstream_<slot>`）；一个共享的 `sequence_special` 分发器（`mark <slot>` → `$sequence_special_<slot>` → `exit`）和 `domain_mapper` `special_upstream_matcher`（`ctx_mark`/`output_tag`）；`sequence_6666`、`sequence_requery` 在 DDNS 分支和普通路由前调用它 | 原生托管生成器（`special_groups.rs`）输出自己的结构：每组 `domain_set` 两个、`cache`、`forward`、`sequence_special_<slot>`（`flow_setter` 不带 `upstream=`、cache 无 `switch4` 条件）和直接 `qname` 路由；**没有** `sequence_special`、`domain_mapper`、`mark` 路由，`special_groups.yaml` 由主路由挂钩 `$special_upstream_matcher` 注入。非托管加载 `config_all` 的该文件时 `domain_mapper` 是 M1-a 缺口，`check` 已列出 | 原生与 Go 的生成结构不同：路由优先级（DDNS 之前调用分发器）、`switch4` 对专用组 cache 的影响、`flow_setter` 的 `upstream` 参数在原生托管输出中缺失。原生托管路径对 `config_all` 当前不生效（未开启 `native_management`） | M1-e（先由 M1-a 的 `domain_mapper`/`mark` 支持非托管形态，M1-e 再对齐托管生成器或明确记录偏差） |
| D3 | v0.7.3：专用组可 `upstream_sources`（引用整个上游组或组内单个上游）和 `upstreams`（组私有上游，`owned_upstreams`）；来源变化时刷新引用，无效引用产生 `upstream_warnings` 并排除；`GET /api/v1/upstream/sources`；`POST /api/v1/special-groups` 的新字段；运行时状态写入 `webinfo/special_upstream_groups.json` 的 `upstream_sources`/`owned_upstreams`；`effective_upstream_count`/`upstream_active` 等派生字段 | 原生 `special-groups` 接口只有 `slot/name/listen_port/custom_port_only`；`/api/v1/upstream/sources` 不存在；`special_upstream_groups.json` 的组记录用 `deny_unknown_fields` 解析（`special_groups.rs`），所以 Go v0.7.3 写入的 `upstream_sources`/`owned_upstreams` 会让原生托管启动失败 | 在 Go 上配置了组上游引用/私有上游后切换到原生托管：状态文件被原生拒绝（失败而非静默丢失，符合 fail-closed，但切换前必须先清理或由 M1-e 支持）。UI 在原生下不发出相关请求（见 D4） | M1-e：先冻结状态文件的读写契约（含 Go 写入的新字段在原生下的行为），再实现引用/私有上游/刷新/告警 |
| D4 | v0.7.3 的 UI 新操作：专用组编辑器的“上游设置”（引用已有上游、本组专属上游）、读取 `/api/v1/upstream/sources` | 新增 capability 操作 `groups.upstreams`：原生恒为 `supported:false`，原因“专用组引用上游与组私有上游尚未在原生运行时实现”；Go（无 `/api/v1/capabilities`）不受影响。Vue 在不支持时隐藏该区域并显示原因，不请求 `/api/v1/upstream/sources`，保存专用组时不发送 `upstream_sources`/`upstreams`。证据：Node 测试（门控、无网络访问）、Rust 能力矩阵测试（31 个操作）、浏览器冒烟（原生：编辑器显示原因、POST 成功、无 `sources` 请求；Go：完整功能，POST 成功） | 旧版原生二进制（无 `groups.upstreams`）配新 UI：能力响应缺少该 id 会被严格校验拒绝——UI 与原生二进制必须同批发布（两者本来就一起构建） | 本任务已完成门控；M1-e 实现后改为按实际能力返回 |

## 其它合并面

- Go 侧（`coremain`、schema 常量、`scripts/release-update-metadata.json`、CHANGELOG、发布说明）以 main 为准自动合并，无冲突。
- `webui-log/src/style.css` 自动合并；两套 Vue 与生成产物来自重新构建。
- 非目标保持不变：原生 DNS 行为未改，`mosdns check -d <root> -c <root>/config_custom.yaml` 对 `config_all` 的输出与 M1-0 逐字节一致（仅路径标签不同）。
