# 10-09 M1-f2：原生配置包 `config_rust`、设置迁移工具与 WebUI 接线的验证记录

任务 `10-09-rust-native-m1-f2-config-rust`，基线 `ed9a5f1a` + 本任务工作树。Rust 构建、测试、`check`、迁移与浏览器冒烟的服务端都在 VM `mosdns-rust`（Linux x86_64）上进行，本机不构建 Rust；
两套 Vue 在本机用仓库脚本构建（与 Rust/Go 构建串行）。生产快照只在仓库外使用，本记录不含任何个人或生产地址。

## 用户决定（2026-10-09）
`socks5_proxy` 的 `addr` 允许为空，空 = 不使用代理；`config_rust` 的 `main_proxy` 默认留空，与 Go 默认包（全局 `socks5` 为空）一致。

## config_rust（`file` 仓库 `mosdns/config/config_rust/`，未提交）
- `mosdns check -d <config_rust> -c config_custom.yaml`：`SUMMARY gaps=0 cascades=0 globals=0 definitions=128`（`config_all` 的 126 + `main_proxy`/`client_ecs`），15 行 `EFFECTIVE` 全部 `source=yaml`，见 [config_rust-check.txt](config_rust-check.txt)。
- 与 `config_all` 的全部差异：[config_rust-vs-config_all.diff](config_rust-vs-config_all.diff)（`diff -ru`，去掉新增的 README）；差异说明与等价性论证：[config_rust-README-rust.md](config_rust-README-rust.md)；新增对象文件：[config_rust-settings.yaml](config_rust-settings.yaml)。
- 隐私自查：`config_rust` 与本目录文件里没有 `10.0.`、`192.168.`、生产 ECS 前缀等（`grep -rE` 只命中 `geoip_cn.txt` 里的公网 CIDR 数据）。
- 打包：`zip -r ../config_rust.zip . -x '.DS_Store'`（`README-rust.md` 的“打包”一节；`build_config_up.sh` 不适用）。zip 未提交。

## 迁移工具 `mosdns-settings-migrate`
- 单元（`settings_migrate.rs`，6 个；含“Go 运行自己 YAML 列表的组需人工确认”）：全局值落到唯一对象且与 YAML 相同不写；Go 的 `use_socks_proxy` 推断→显式 `use_proxy`（`foreign` 回退、显式 false、UDP 恒 false、条目字面量与全局相同、`domestic` 勾选但无地址实际直连）；
  无法映射项（`so_mark`、签名 API、条目自带不同代理、未知 tag、`replacements`）逐条进入报告而不是丢弃；与 YAML 相同的列表不写、重复标签改名；生成开关只写差异。
- 集成（`tests/m1_f2_settings_migrate.rs`，4 个；含写出失败不留残缺结果）：端到端生成 `settings.json` 与报告、输入文件逐字节不变、不写原生根目录、不覆盖已有输出；无法映射项使退出码为 1 且仍写出可映射部分；迁移后设置无法加载则出错且不写 `settings.json`。
- 生产快照迁移（仓库外；只记汇总）：退出码 0，0 项无法映射。全局 `socks5`、`ecs` 均映射到 `main_proxy`/`client_ecs`；`domestic`（7 条，启用 2）、`cnfake`（1）、`nocnfake`（1）写入 `upstreams`；
  `foreign`、`foreignecs` 与 `config_rust` 默认列表等价（不写入）；`foreignecs` 的 `google` 与 `foreign` 重复，改名 `google-foreignecs`；1 个停用的签名 AliAPI 条目被丢弃并报告。
- 对照：`scripts/go-effective-oracle.py` 用 Go 语义（`api_upstream.go` 推断与回退）独立计算生产快照的每组每条上游的地址与代理归类，与“用迁移结果跑 `check`”的 `EFFECTIVE`
  （地址、启用、`[proxy]` 标记）逐条比较：**5 个 `aliapi` 组全部一致（EQUIVALENT）**，代理地址与 ECS 的来源为 `settings`，其余为 `yaml`。
  反例（未迁移的 `config_rust` 对同一快照）：5 个组全部不一致，说明对照脚本能发现差异。规则源全部走代理由 `proxy: $main_proxy` 在 `main_proxy` 非空时保证（M1-f1 测试 `rule_sources_take_the_proxy_by_reference`）。

## Rust 构建与测试（VM）
| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 1455 passed，0 failed，4 ignored（M1-f1 之后 1444；新增空代理地址 1、迁移单元 6、迁移集成 4） |

附带的 Rust 改动与对应测试：空代理地址（`an_empty_proxy_address_means_no_proxy`）；设置版 GET 上游条目不含 `use_proxy`（浏览器冒烟发现：多出的字段使每个条目被 WebUI 判为“只读”）；`unsupported_features` 随设置版上游页去掉 `socks_proxy`
（能力矩阵测试）；aliapi 的 `EFFECTIVE` 对真正走代理的条目加 ` [proxy]`（`check` 相关测试已更新）。

## WebUI（`webui-log/`，两套 shell）
- Node 测试：`node --test tests/*.mjs` 37 个全部通过（新增 `settingsWiring.test.mjs`：代理开关能力判定、只读判定、`use_socks_proxy` 只对 TCP 类协议为真、来源标记、两套 `SystemControlManager` 的原生/Go 分支静态检查、`/api/v1/settings/effective` 不受能力门控）。
- 构建：`npm run build`、`npm run build:log1` 均成功（串行，Rust/Go 构建未并行）；产物在 `coremain/www/`。
- 浏览器冒烟（内置浏览器经 SSH 端口转发访问 VM 上的服务；结束后已关闭转发并停止服务）：
  - 原生（`/`，fixture 配置）：上游页显示“已修改（与配置文件不同）/恢复默认”；条目可编辑；编辑 `foreign` 的 TCP 条目时显示“使用 socks 代理”开关（无字面量 Socks5 输入、提示指向覆盖设置），取消并保存后 `webinfo/settings.json` 只含该组差异，`/api/v1/settings/effective` 去掉该条目的 `[proxy]`；点“恢复默认”后该组从 `settings.json` 消失；
    系统设置页：覆盖面板显示“保存后立即生效，无需重启”，“已修改/恢复默认”可用并恢复为 YAML 值；“高级替换规则”面板与开关在原生下隐藏并显示原因；域名生成面板正常。
    首次冒烟发现 GET 上游条目带有 `use_proxy` 使全部条目变只读，已在后端修复并复测。
  - 原生兼容页（`/log`）：上游页同样接线；运行维护页显示覆盖说明与替换规则不支持的提示。
  - Go（参考二进制，最小配置加覆盖文件）：上游页仍只对 `foreign` 组显示开关与字面量 Socks5 输入及“继承系统设置”提示；系统页仍有“高级替换规则”面板；没有“已修改/恢复默认”。
    注：参考 Go 二进制自己的启动更新检查会访问公网（页面显示“最新版本”），这是参考二进制的行为，不是本任务的测试流量。

## 已知差异与未做
见 `config_rust-README-rust.md` 的“已知差异”与契约 `native-settings.md`。首次整链运行与 Go 差分（E2E-1）、托管模式下的上游覆盖（M1-e）不在本任务内。

## 规划会话处理（2026-10-09）

- 迁移工具对“覆盖文件缺组或组内全停用”一律退出码 1 并要求人工确认（默认包的 `cnfake` 总会触发）：接受保守做法。
- 覆盖页留空 = 回到 YAML 默认，YAML 默认非空时无法在页面切成直连：与 Go 一致，接受。
- `config_rust` 上游列表只含启用项，停用预置以注释保留：接受（已写入其 README 的已知差异）。
- 执行中少数文件用 `sed -i` 修改，违反 AGENTS.md“优先 Edit”：记录，后续任务提示中再次强调。
- 已核对 `config_rust` 不含个人/生产地址（代理、ECS、内网 IP 关键字扫描为空）。`file` 仓库的提交待用户确认。
