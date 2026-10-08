# 10-07 M1-a2：原生规则源管理与 mapper 热更新的验证记录

任务 `10-07-rust-native-m1-a2-rule-management`（基线 `e75153db`）。语义取证与分级见任务内
`research/go-semantics.md`，设计见 `research/design.md`；行为契约见
[native-rule-providers](../../contracts/native-rule-providers.md) 的 “Management (M1-a2)” 一节。
构建与测试全部在 VM `mosdns-rust`（Linux x86_64，2 vCPU）上进行；测试不访问公网（本地 HTTP/HTTPS
测试服务器与本地 SOCKS5 测试代理）。

## 测试

| 命令（`rust/`） | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace --no-fail-fast` | 1363 passed，0 failed，4 ignored（M1-b2 后为 1343）；Codex 第 2 轮修复后 `mosdns-native-host` 527 passed（+1 单元测试，其余 crate 未改） |
| `node --test tests/`（`webui-log`） | 30/30 |
| `npm run build`、`npm run build:log1` | 通过（与 Rust 构建串行） |

新增 Rust 测试：`native-host/tests/m1_a2_rule_management.rs`（11 个，真实 API + 真实 DNS + 本地下载）、
`rule_sources.rs` 单元测试 7 个（时间解析、到期判定、清单写出格式、跨代共享写锁、同一文件判定、UUID、原子写）、`upstream-core/src/socks5.rs`
单元测试 3 个。覆盖：经 SOCKS5 与 302 重定向的下载并热切换 mapper；HTTPS 证书校验（受信根成功、不受信失败）；
下载失败（404、非 SRS、超限、超时、本 host 启动会拒绝的 regexp、连接拒绝、SOCKS5 代理不可达）均保留旧文件/清单/规则且
无临时文件残留；同一来源重复更新最多一次补跑；`domain_set_light` show（`q`/`limit`/`offset`）/save/post 与拒绝路径；
AdGuard 增删改、全部/单条更新、直接 matcher 与 mapper 同步；`si_set` 列表热替换（`client_ip $geoip`）；
重建在 blocking 侧被挂起时 DNS 照常应答、在途查询保持旧代、构建失败保留旧代；自动更新调度（到期/未到期/关闭
自动更新）与关闭时回收挂起中的下载（<2 s）；Go 写出的清单被原生读取、改写后逐字节一致。

## 清单互通（Go ↔ 原生）

- Go → 原生：`tests/fixtures/m1_a2/go-sd.json` 由参考实现经自身 API 写出（`tools/m1a2-interop`，在含一条
  真实 `config_all` 来源的清单上 `PUT` 新来源）；原生列出的 JSON 与之相同，增删一个来源后文件逐字节回到原样。
- 原生 → Go：原生 host 经 API 修改 `sd_set_light`（含下载后的新 `last_updated`）、`si_set`、`adguard_rule`
  清单后停止，`tools/m1a2-interop` 用参考插件读取并经参考 API 列出：四份清单与原生列出的 JSON 全部相等，
  见 [interop-native-to-go.txt](interop-native-to-go.txt)。

## 浏览器冒烟

VM 上运行 debug 原生 `mosdns`（内嵌本次构建的两套 Vue），配置含 `domain_set_light`×2、`sd_set_light`×2、
`si_set`、`adguard_rule`（tag `adguard`）与 mapper；下载源为 VM 本地 `python3 -m http.server`。经 SSH 隧道在桌面
浏览器中操作：

- `/`：本地规则（白名单追加并保存 → 文件与 DNS 结果改变）；订阅规则（列表、单条更新 → 规则数/时间刷新、
  启停、新增中国 IP 来源（URL 自动识别）→ 下载生效、编辑更新间隔 → 回读、删除）；广告拦截（新增 → 下载并生效于
  DNS、停用、更新全部、删除）。
- `/log`：本地规则（黑名单追加并保存 → 文件改变）、订阅规则“更新全部”、广告拦截列表。
- 无能力门控拒绝的请求；控制台错误仅为 UI 对未配置 tag 的既有探测（`/show`、`/config` 的 404）。
- Go 后端（同一份 UI 构建的 Go 二进制，相同配置）：订阅规则列表与更新、广告拦截列表均按原 Go 路径请求
  （能力发现 404 → legacy），行为不变；Go 直接读取原生写出的清单。

冒烟中发现并修复：`202` 状态行原因短语缺失（显示为 `Internal Server Error`），已补 `Accepted`。

## check

`mosdns check -d <root> -c <root>/config_custom.yaml`（debug 构建 sha256 `b1d46016…f245`），输入 `file` 仓库
`config_all`（`28c6493`）与生产快照（仓库外，用后删除）：[config_all-check.txt](config_all-check.txt)、
[prod-snapshot-check.txt](prod-snapshot-check.txt)，根目录前缀替换为 `<root>`。`SUMMARY gaps=16 cascades=66
globals=1`，两份逐字节相同，且与 M1-b2 记录除根目录前缀外相同；运行前后两棵目录的全部文件 sha256 不变（`check`
只读）。生产快照 stderr 为空；`config_all` stderr 仍只有 adguard 缺少已下载 `*.rules` 的告警（启动运行时才会补下）。

## 内存（R3：新旧快照共存）

release 构建（sha256 `5b794b48…0aa9`），生产快照中 `rule_set.yaml` 全部 provider（去掉 `rewrite`）+ `adguard` +
`unified_matcher1`（148k 条规则）+ 一个 UDP listener + API。每轮 `POST /plugins/blocklist/post`（原列表 + 1 条）触发
`unified_matcher1` 整体重建；每轮前 `clear_refs=5` 重置 HWM，3 s 后读 `VmHWM`/`VmRSS`（kB）：

| | 启动后稳态 RSS | 启动峰值 | 第 1 次重建后 | 第 2 次 | 第 3–6 次 |
| --- | --- | --- | --- | --- | --- |
| 默认 glibc，专用构建线程（最终代码） | 113,804 | 201,052 | 310,588 | 349,324 | 349,324–355,472（不再增长） |
| 默认 glibc，tokio blocking pool（初版） | 113,728 | 198,696 | 311,984 | 366,936 | 364,888–373,084 |
| `MALLOC_ARENA_MAX=1`，专用构建线程 | 113,184 | 200,040 | 225,696 | 241,040 | 260,340–269,932 |

结论：重建期间的新旧共存峰值约为稳态 + 200 MB；重建结束后 RSS 不回落到启动水平（glibc 不把非主 arena 的空闲
内存还给系统），默认分配器下约稳定在 350 MB，有上界、多次重建不再增长。重建改为在一个专用线程上串行执行（不并发、
复用同一 arena），比 blocking pool 略低。这一数字明显高于 Go（M1-a1 记录：Go 加载后 144 MB），作为 3.9 内存优化
的输入（规则文本与 `ValuedRule` 的临时副本、arena 回收）；本任务不做优化（PRD 非目标）。

## 审查

自查（`/code-review high`）9 条：修复 2 条（托管切换时旧代 rule-source 任务的收束与新代启动工作、启用正则前校验
源文件），其余为性能/边界项，记入契约或任务内 `deferred-p2.md`。Codex（`gpt-6.1-sol`/medium，只读）：第 1 轮
FAIL（P1：托管切换后旧代覆盖新代清单；删除来源时误删他源共用的规则文件）→ 修复（跨代共享的每文件写锁 + 安装时
同步隔离旧代；删除前判定文件是否仍被引用）；第 2 轮：前者 FIXED，后者因路径写法不同（`srs/../srs/cn.srs`）未修全 →
改为按 inode/规范化路径判定；第 3 轮 `FINAL: PASS`。

## 用户决定（2026-10-08）

- 内存：热重建后 RSS 稳定在约 350 MB（启动 114 MB），生产机仅 957 MB，不可接受。原生内存优化从
  “M1 之后、5D 之前”提前到 M1-a2 之后、M1-b1 之前；尚未部署，不先加 `MALLOC_ARENA_MAX`。
- 环境变量代理（`HTTP(S)_PROXY`）不支持，记为已知差异：生产 mosdns 进程环境中无代理变量，
  `config_all` 的 `socks5` 为空。
- 能力 `unsupported_features` 中 `srs_geodata`、`remote_download`、`auto_update` 的标签已不准确，
  在 M1-b1 中修正。
- 仅 `native_management` 模式下后台规则写入与托管 apply 的可重试冲突：接受（已写入契约）。
