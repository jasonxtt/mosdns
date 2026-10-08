# 10-08 M1-f1：类型化设置层（后端）的验证记录

任务 `10-08-rust-native-m1-f1-typed-settings`，基线 `45e75115` + 本任务工作树。构建、测试、测量在 VM `mosdns-rust`（Linux x86_64，2 vCPU）上进行，本机不构建。
生产快照只在仓库外使用。契约见 [native-settings](../../contracts/native-settings.md)。

## 构建与测试

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test -p mosdns-native-host --test m1_f1_typed_settings` | 19 passed |
| `cargo test --workspace --no-fail-fast` | 1444 passed，0 failed，4 ignored（M1-c2 之后为 1421） |

既有测试需要对齐的两处（都因“不再一律不支持”）：`slice6_management_http` 的能力矩阵用例（无对象配置下 `/api/v1/overrides`、`/api/v1/domain-generation` 仍应 404，所以这两条路由只在配置有
对应对象时才提供，与能力矩阵同一判据）；`special_groups_supervisor` 的“无 profile 时 `POST /api/v1/upstream/config` 为 400”（无 `aliapi` 的配置保持原行为，设置路径只在配置有
`aliapi` 时启用）。两个测试文件本身未改。

新增测试（真实 UDP/TCP 套接字、本地 DNS 桩与本地 SOCKS5 中继，不访问公网）：

- 集成 `rust/native-host/tests/m1_f1_typed_settings.rs`（19 个）：对象与引用的解析、来源报告（yaml/settings）、覆盖后的上游列表与代理实际落到条目上；引用错误/类型错误/互斥写法/无效值/设置文件各类错误的路径与原因；
  quick `ecs $ref` 及其随设置变化；`check` 输出 `EFFECTIVE` 行且只读、坏设置文件报 GLOBAL；新语法 fixture（`tests/fixtures/settings/`）`check` 无缺口；规则源 `proxy`；
  **热生效**：下一条查询立即用新上游、**在途查询（上游慢 900 ms）仍得到旧上游答案**、设置文件只含差异、API 回读、进程重启后保持、空列表恢复 YAML；
  被拒的变更（坏 JSON、缺字段、未知标签、签名 API、`so_mark`、全部禁用）文件与运行配置都不变；写盘失败（`webinfo` 被占用）不发布、修复后可再次成功；
  **覆盖页**改代理后下一条查询经新的 SOCKS5 中继（连接计数）、清空值回到 YAML、`replacements`/坏地址被拒；同类多个对象为 409 `ambiguous`；
  域名生成开关的 5 键形状、关闭时清空该列表并停止记录、主开关；能力矩阵随配置变化；改 ECS 地址使依赖它的缓存的 dump 被清空且新一代重新向上游查询；
  **应用期间不丢查询**（5 轮 × 每 2 ms 一条）；字面量 `socks5` 与 `proxy`/`use_proxy`（含 `false`）/设置上游混用均报错；只存差异（与 YAML 相同的值被移除）；YAML 关闭的生成列表显示为关、开启后才记录；`native_management` 下不提供设置路由。
- 单元：`settings.rs`（文件往返与拒绝、条目校验、地址校验）、`runtime_snapshot.rs`（`FileCommit`：期望字节不符即冲突且什么都不写）。

## check

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` / `prod-snapshot-check.txt` | `mosdns check` 对 `config_all` 与生产快照，逐字节相同：`SUMMARY gaps=0 cascades=0 globals=0 definitions=126`，另有 8 行 `domain_output`、5 行 `aliapi` 的 `EFFECTIVE`（来源均为 yaml） |
| `new-syntax-fixture-check.txt` | 新语法 fixture（含 `webinfo/settings.json`）：`gaps=0`，`EFFECTIVE` 行显示 `main_proxy` 地址、`domestic` 上游、`my_realiplist` 开关来自 `settings`，其余来自 `yaml` |

## 热生效的代价（release，生产规则集，`scripts/measure-settings-apply.py`；原始数据 `apply-cost.json`）

配置：生产 `rule_set.yaml`/`adguard.yaml` 的全部 provider（去掉 `rewrite`，`unified_matcher1` 148k 条）+ 一个 `socks5_proxy` + `aliapi`（本地桩）+ API；200 qps 的单并发客户端；每次 `POST /api/v1/overrides` 改代理地址。

| | 启动稳态 RSS | 每次 apply 的 API 耗时 | apply 期间最慢的一条查询 | apply 后峰值 VmHWM | apply 后稳态 RSS | 丢失/超时的查询 |
| --- | --- | --- | --- | --- | --- | --- |
| 4 次 | 38.0 MB | 350–422 ms | 331–394 ms | 117.8 → 138.8 MB | 64.8 → 65.7 MB（不再增长） | 0 |

- 候选配置的编译在 DNS 线程上进行（`CompiledConfig` 是 `Rc` 结构），因此一次 apply 期间正好到达的查询最多等约 0.35–0.4 s（其余不受影响）；p99 1.1 ms。
- 发现并修复的两个问题：(1) 复用托管模式的 `pause_admission` 会在提交窗口内**丢弃** DNS 查询（每次 apply 丢 1 条）→ 新增只暂停管理面的 `pause_management`，DNS 持续接收；(2) apply 之后没有 trim，
  旧代释放后 RSS 留在 78–95 MB → 退休后立刻 trim，并在最后一个旧查询必然结束后（8 s）再 trim一次，稳态降到约 65 MB（比启动稳态多 27 MB，不随次数增长）。

## 已知差异与未做

见契约“Known differences”：不读 Go 的覆盖文件与 `replacements`；无隐式 `foreign` 代理；`concurrent` 不再按启用数重算；设置立即生效（无需重启）；设置按对象 tag 存储。
托管模式的上游覆盖仍走 `upstream_overrides.json`（M1-e）；WebUI 页面接线、`config_rust`、首次整链运行是 M1-f2。

## 规划会话决定（2026-10-09，用户授权规划会话决策）

Codex 第 3 轮剩余的两点按 **P2 / 已知限制** 接受，不开第 4 轮：
- 清空缓存 dump 时目录 fsync 失败被忽略，若随后断电，重启可能读回旧缓存；
- 首次创建 `webinfo` 目录时上一级目录 fsync 失败，重试会因目录已存在而跳过同步。
二者都需要“目录 fsync 返回错误”且紧接着断电/崩溃才会出现，不属于 P1 定义中的现实可达输入；正常路径（含 rename 作为提交点、
rename 后目录 fsync 失败不回滚）已在第 3 轮确认修好。后续若改动该持久化路径，顺带让缓存 dump 目录 fsync 失败中止本次变更。

另记两项供 5D 参考：设置 apply 期间单条最慢查询 331–394 ms（管理操作，频率低）；多次 apply 后 RSS 稳态约 65 MB（启动 38 MB，不随次数增长）。
