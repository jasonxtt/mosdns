# 10-08 M1-b1：`aliapi`、`rewrite`、`webinfo` 的验证记录

任务 `10-08-rust-native-m1-b1-upstreams-overrides`。基线 `304a206f` + 本任务工作树。范围按规划会话 2026-10-08 的调整：
**运行时覆盖文件（`config_overrides.json`、`upstream_overrides.json`）的应用暂停**，`aliapi` 取值来自 YAML；
契约见 [native-aliapi-rewrite-webinfo](../../contracts/native-aliapi-rewrite-webinfo.md)。

## 构建与测试

VM `mosdns-rust`（Linux x86_64，2 vCPU），源码 rsync 到 `/root/mosdns-rust-m1b1-20261008/src`，`CARGO_INCREMENTAL=0`，`-j2`，
本机不构建。无公网访问：上游/DoH/DoT/SOCKS5 代理/DNS 均为本地测试进程。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test -p mosdns-native-host --test m1_b1_aliapi_rewrite_webinfo` | 15 passed |
| `cargo test --workspace --no-fail-fast` | 1385 passed, 0 failed, 4 ignored（M1-a2 之后为 1363：+15 集成、+6 单元，含一个改过夹具的 `config_check`；M1-g 再 +1：`a_fixed_address_ecs_may_follow_a_cache_but_a_client_derived_one_may_not`） |

新增测试：

- 集成 `rust/native-host/tests/m1_b1_aliapi_rewrite_webinfo.rs`：`aliapi` 生产形态编译、`concurrent` 1..20 与插件级默认合并、
  不支持项明确拒绝、`forward` 仍拒绝 `socks5`；UDP 竞速（含 `upstream_query_timeout`、SERVFAIL）；SOCKS5 下的 TCP、DoT（主机名由代理解析）、
  DoH（数值 `dial_addr`、主机名由代理解析）；UDP 忽略代理、代理不可达时 TCP 上游失败且不直连；`rewrite` 静态/域名目标、accept、EDNS、
  TTL 5、SERVFAIL、审计标签 `重定向`、`/show`/`/post`（含坏规则、写失败）、缺失文件；`webinfo` 读写、转义、原子保存、写失败保留旧文档、
  405/404、`client.aliases` 能力。
- 单元：`rewrite.rs`（规则语法）、`webinfo.rs`（输出格式）、`socks5.rs`（`Route`/`dial`）。
- 既有 `config_check` 夹具中把 `rewrite` 当作“不支持类型”的一项改为 `requery`。

## check

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` | `mosdns check -d <root> -c <root>/config_custom.yaml`（`file` 仓库 `28c6493` 的 `config_all`）的 stdout，退出码 1 |
| `prod-snapshot-check.txt` | 生产快照（Go v0.7.3，仓库外）同样运行的 stdout，与上一份逐字节相同；stderr 为空 |

`SUMMARY gaps=9 cascades=30 globals=0 definitions=126`（M1-a2/M1-b2 之后为 `gaps=16 cascades=66 globals=1`；M1-g 之前本任务中间结果为
`gaps=9 cascades=30 globals=1`）。`aliapi` 5、`rewrite` 1、`webinfo` 1 个缺口全部消失；剩余直接缺口为 `domain_output` 8、`requery` 1。
GLOBAL 归零：中间状态曾被 `unsafe cache … successor may execute ecs_handler` 挡住（见 [M1 缺口清单](../../m1-gap-list.md) M1-g），
M1-g 放置规则修正（固定地址 ECS 不再视为 unsafe）后该 GLOBAL 消失，`check` 不再在此停止，之后**没有**暴露新的缺口或 GLOBAL
（缺口与连带错误的集合与此前相同，只是少了这一行 GLOBAL）。

输入与方法与 M1-b2 相同：配置依次复制到 VM 同一临时目录，`cd /root` 后运行，结束后删除；运行前后文件树 sha256 不变。
stderr：`config_all` 只有 adguard 缺少已下载 `*.rules` 的告警（Go 启动时联网下载，原生不下载）。
构建：debug `mosdns`，sha256 `37632a8eb0d0ef3e15ac21695c19245f37ced9e47bc1b16524abf0d4662e7535`（含 M1-g）。
输出只含 tag、类型、字段路径与编译器原因，无凭据值。

## M1-g 缓存放置

`cache_placement.rs`：cache 后继中的 `ecs_handler` 只在**依赖客户端**时才算 unsafe（`forward: true`，或 `send: true` 且无 `preset`）；
固定地址（quick `ecs <ip>`、有 `preset` 的 handler，含 `send: true`）和无地址 handler 不再拒绝，`client_ip` 分支仍拒绝。
依据 `QueryView::with_ecs` 的选取顺序（转发的入站 ECS → preset → `send` 的对端）。测试见 `ecs_cache.rs`。
契约（用户 2026-10-08 决定）：缓存对同一 key 返回先存入的答案，不论来自哪条后继路径；固定 ECS 与“选哪个上游”一样属于路由差异，不视为错误（与 Go 一致）。
放置规则只防止“答案随客户端变化”的泄漏。曾试加“同一 cache 的固定 ECS 集合必须一致”的检查，Codex 第 3 轮发现它仍需覆盖 jump/fallback 间接路径，已按上述决定撤销。

## 语义取证

见任务内 `research/go-semantics.md`（本地 Trellis）。要点已写入契约：`aliapi` 与 `forward` 同一竞速语义；SOCKS5 只用于 TCP 类协议，
带主机名时由代理解析；`rewrite` 命中即 accept，IP 目标 TTL 5，域名目标经 UDP 查询 `dns`；`webinfo` 失败不破坏文件。

## 已知差异与未做

- 覆盖文件的应用与上游/覆盖管理 API：暂停（M1-f）。
- `webinfo` 保存失败时内存保持旧文档（Go 留下新文档）；`webinfo` 目录在首次 PUT 时创建。
- 不支持项加载时报错：签名 AliDNS API、`so_mark`、`bind_to_device`、`enable_pipeline`、`enable_http3`、正的 `idle_timeout`、`quic://`。
