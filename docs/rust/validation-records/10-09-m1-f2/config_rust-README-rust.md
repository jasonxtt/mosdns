# config_rust：原生（Rust）版 MosDNS 配置包

`config_rust` 是由 `config_all` 改写的**原生运行时专用**配置包：路由、规则、缓存、开关、分流序列都与 `config_all` 相同，
只把“WebUI 里可调整的值”（代理地址、ECS 地址、各上游组的上游列表、域名生成开关）改成了**具名对象 + `$标签` 引用**，
由原生运行时统一管理。Go 版 MosDNS 不能加载本包（不认识 `socks5_proxy` / `ecs_address`，也不认识 `proxy:` / `use_proxy:` 写法）。

## 可调整的值放在哪里

| 值 | YAML 里的对象 | WebUI 位置 | 修改后保存在 |
| --- | --- | --- | --- |
| SOCKS5 代理地址 | `sub_config/settings.yaml` 的 `main_proxy`（`socks5_proxy`） | 系统设置 → SOCKS5 / ECS 覆盖 | `webinfo/settings.json` |
| ECS 地址 | 同上 `client_ecs`（`ecs_address`） | 同上 | 同上 |
| 各组上游列表、是否走代理 | 各 `aliapi` 的 `upstreams`（条目 `use_proxy`） | 上游设置 | 同上 |
| 域名生成开关 | `domain_output` 插件的 `enabled` | 系统设置 → 域名生成 | 同上 |

- `webinfo/settings.json` 只保存**与本包 YAML 不同**的值；删除该文件即回到本包默认。修改在 WebUI 保存后**立即生效**，不需要重启。
- **代理默认为空 = 不使用代理**（与 `config_all` 在 Go 下默认的“全局 socks5 为空”相同）。填写 `main_proxy` 后，所有规则源下载以及
  `foreign` / `foreignecs` 里 `use_proxy: true` 的上游才经它连接；其余上游不受影响。
- 原生运行时**不读取** Go 版的 `webinfo/upstream_overrides.json`、`config_overrides.json`、`domain_generation_settings.json`。
  从 Go 版迁移时用一次性工具 `mosdns-settings-migrate`（见文末）。

## 与 config_all 的差异

除下列文件外，其余文件与 `config_all` **逐字节相同**。

### 新增 `sub_config/settings.yaml`，`config_custom.yaml` 的 include 首位加入它
定义 `main_proxy`（`addr: ""`，无代理）与 `client_ecs`（`2408:8888::8`，即 `config_all` 里 `ecs` 的原值）。

### 规则源：`sub_config/rule_set.yaml`（5 处）、`sub_config/adguard.yaml`（1 处）
`socks5: ""` → `proxy: $main_proxy`。
理由：Go 的全局 `socks5` 覆盖会替换**所有**名为 `socks5` 的参数（包括值为空串的）。因此全局代理为空时规则源直连，非空时全部走代理；
`proxy: $main_proxy` 在 `main_proxy` 为空/非空时分别得到同样的结果。

### `sub_config/forward_local.yaml`（`domestic`）
- 上游列表写成 Go 在 `config_all` 下**实际生效**的列表：随包发布的 `webinfo/upstream_overrides.json` 启用了 `阿里UDP`（`udp://223.5.5.5`）与
  `阿里DOH`（`https://dns.alidns.com/dns-query`，`dial_addr: 223.5.5.5`）；YAML 里原有的 `223.5.5.5 / 223.6.6.6 / 114.114.114.114` 在 Go 下被它整体替换。
- `concurrent: 3` → `2`：Go 对覆盖列表按启用数重算（2 个启用 → 2），原生不重算，所以直接写成 2。
- 加插件级 `proxy: $main_proxy`：让 WebUI 可以给本组任意条目勾选“走代理”；本组条目默认都不走代理（与 Go 下 `domestic` 默认一致）。
- 原包里默认停用的预置上游（`阿里TCP`、`阿里DOT`、`运营商`、停用的签名 AliAPI）不在列表里（YAML 只写启用项，设置层列表只保存差异），
  已以注释保留在文件里，需要时在 WebUI 上游页重新添加即可。

### `sub_config/forward_nocn.yaml`（`foreign`）
- 上游写成 Go 实际生效的 `google`（`https://dns.google/dns-query`，`dial_addr: 8.8.8.8`）与 `cloudflare`（`https://1.1.1.1/dns-query`）；
  YAML 原有的 `1.1.1.1 / 8.8.8.8`（后者 `upstream_query_timeout: 1000`）在 Go 下被覆盖列表替换。
- 两个条目写 `use_proxy: true`，插件级 `proxy: $main_proxy`。理由：Go 对 `foreign` / `foreignecs` 的 `tcp/tls/https` 条目在未显式关闭时默认使用全局代理；
  原生去掉了这条按组名的隐式规则，改成显式的 `use_proxy`。`main_proxy` 为空时仍然直连，结果相同。

### `sub_config/forward_nocn_ecs.yaml`（`foreignecs` 与 `sequence_google_node`）
- 上游写成 Go 实际生效的 `google`；YAML 里的字面量 `socks5: "127.0.0.1:7891"` 在 Go 下被覆盖列表替换而**不会被使用**（包内 `foreignecs` 有启用的覆盖条目），所以本包不保留它。
- 条目标签 `google` → `google-foreignecs`：原生要求上游标签**全局唯一**（用于观察、缓存来源记录），`foreign` 已使用 `google`。
- `exec: ecs 2408:8888::8` → `exec: ecs $client_ecs`（值相同）。

### `sub_config/forward_1.yaml`（`nocnfake`、`cnfake`）
- `nocnfake` 的上游写成 Go 实际生效的 `sing-box`（`udp://127.0.0.1:6666`；YAML 原值 `7874` 在 Go 下被覆盖列表替换）。
- `cnfake` 的覆盖条目全部停用，Go 沿用 YAML 列表，所以仍是 `udp://127.0.0.1:1053`，仅补上标签 `cnfake`（WebUI 要求每个上游有标签）。
- 两个组都加插件级 `proxy: $main_proxy`（默认不走代理）。

### `webinfo/`
删除 `upstream_overrides.json` 与 `config_overrides.json`（原生不读取，保留会让人误以为它们仍然生效）。其余文件不变。

## 等价性论证（路由、上游、代理、ECS）

- **路由与序列**：除上面列出的 `aliapi` 参数和 `ecs` 写法外，所有 `sequence`、规则、缓存、开关、监听、`domain_output`、`requery` 等定义没有任何改动。
- **上游**：每个 `aliapi` 组的有效上游列表、协议、地址、`dial_addr`、`concurrent` 与 `config_all`（含随包覆盖文件）在 Go 下的实际值一致；仅 `foreignecs` 的上游标签加了后缀。
- **代理**：Go 默认（全局 socks5 为空）下所有连接都是直连；本包默认 `main_proxy` 为空，同样全部直连。填写代理后，Go 下走代理的集合是“全部规则源 + `foreign`/`foreignecs` 的 tcp/tls/https 条目”，
  本包与之相同（规则源 `proxy: $main_proxy`，两组条目 `use_proxy: true`）；UDP 条目在两边都不使用代理。
- **ECS**：`ecs $client_ecs` 的默认值就是原来的 `2408:8888::8`。Go 下全局 `ecs` 覆盖会替换 `ecs ` 开头的字符串，等价于修改 `client_ecs`。
- 用 Go 语义的独立实现（`scripts/go-effective-oracle.py`，在 mosdns-rust 仓库）对迁移后的有效配置做过逐条对照，结果记录在 mosdns-rust 的 `docs/rust/validation-records/10-09-m1-f2/`。

## 已知差异

1. 默认停用的预置上游不在列表里（见 `forward_local.yaml` 的注释）。
2. `concurrent` 是 YAML 里的固定值，WebUI 增删启用上游不会重算它（Go 会重算为 `min(启用数, 20)`）。
3. 上游标签全局唯一（`foreignecs` 的 `google-foreignecs`）。
4. 原生没有“高级替换规则”（任意字符串替换）；需要的改动请直接修改 YAML。
5. 专属分流组（special_groups）的托管、更新检查、重启、WebUI 端口、配置包管理等页面按原生运行时当前能力显示，与本包无关。

## 部署

1. 解压 `config_rust.zip` 为配置目录，用原生 `mosdns start -d <目录> -c config_custom.yaml` 运行；先用 `mosdns check -d <目录> -c config_custom.yaml` 检查
   （应为 `gaps=0 cascades=0 globals=0`，所有 `EFFECTIVE` 行的来源为 `yaml`）。
2. 从 Go 版迁移已有的覆盖设置（代理、ECS、上游列表、域名生成开关）：

   ```bash
   mosdns-settings-migrate --from <Go 配置目录> -d <config_rust 目录> -c config_custom.yaml --out <输出目录>
   ```

   工具只读 Go 目录里的三个覆盖文件，不修改任何输入，在 `<输出目录>` 写 `settings.json` 与 `migration-report.txt`。
   退出码 0 表示全部映射；1 表示报告里列出了无法映射或需要人工确认的项（例如启用的签名 AliAPI、`so_mark`、条目自带的不同代理地址、`replacements`；
   以及覆盖文件里没有某个组、或该组条目全部停用——这时 Go 实际运行的是它自己 YAML 里的上游列表，工具看不到，需要你核对原生默认列表是否与之一致），需人工处理；2 表示出错，此时不会留下残缺的输出。
   确认报告后把 `settings.json` 复制到 `<config_rust 目录>/webinfo/settings.json`，再次 `check` 并启动。
3. Go 版 `special_upstream_groups.json` 等其它状态文件不在迁移范围内。

## 打包

```bash
cd mosdns/config/config_rust && zip -r ../config_rust.zip . -x '.DS_Store' -x '*/.DS_Store'
```

（`build_config_up.sh` 面向 Go 版的增量更新包 `config_up`，不适用于本包。）
