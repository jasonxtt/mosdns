# 10-08 M1-c2：原生 `requery` 的验证记录

任务 `10-08-rust-native-m1-c2-requery`，基线 `59d050aa` + 本任务工作树。构建、测试在 VM `mosdns-rust`（Linux x86_64，2 vCPU）上进行，本机不构建。
生产快照（`prod-config-20261006`）只在仓库外使用。契约见 [native-requery](../../contracts/native-requery.md)。

## 构建与测试

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test -p mosdns-native-host --test m1_c2_requery` | 12 passed |
| `cargo test --workspace --no-fail-fast` | 1421 passed，0 failed，4 ignored（M1-c1 之后为 1403） |

既有测试改动：`tests/config_check.rs` 与 `tests/fixtures/check/gaps/config.yaml` 把 `requery` 当作“不支持类型”的示例，现改为 `reverse_lookup`。

新增测试（真实 UDP/TCP 套接字、真实管理 listener，不访问公网）：

- 集成 `rust/native-host/tests/m1_c2_requery.rs`：一次完整任务（先 save 再合并过滤 再 flush 再 A/AAAA 限速重查询，经主机自己的 listener、cache、上游，再 save；
  只查日期内的名字、AD 标志带上、重查询后统计为 2 次/名、cache 已保存、状态文件为 idle）；重复触发 409 与取消（取消后无新查询、可再次启动、URL 步骤中取消按参考实现为 failed）；
  启动时 `running` 残留 -> `failed` 并补默认值；状态文件缺失时创建默认文件（`check`/编译不写）；调度器在开始时间触发一次并随 `scheduler/config` 更新
  （`date_range_days` 仅正值生效）；URL 失败的参考错误文本；`/stats/source_file_counts`；其它主机的 URL 走 HTTP；**参考实现写出的状态文件**原生读入后
  `GET /`、`GET /status`、`POST /scheduler/config` 与参考工具逐字节一致；主机停止时取消运行中任务并记录；坏文件/缺 `file` 为配置错误。
- 单元：`requery.rs`（调度规则、Go 时间格式、合并过滤与去重、查询报文与名字编码）、`local_url.rs`（两种本机 URL 判定）。

## 文件互通（双向）

- **参考 → 原生**：`tools/m1c2-interop` 实例化参考 `requery`，用生产快照的 `requeryconfig.json`（仅本机 URL 与别名）经参考 API 取得 `GET /`、`GET /status`、
  `POST /scheduler/config` 输出，存为 fixture（`rust/native-host/tests/fixtures/requery/`），原生测试逐字节对照。
- **原生 → 参考**：`tools/m1c2-interop/native_to_reference.sh`：原生 host 加载含 `running` 的状态、经 API 改调度器并触发一次任务后停止；参考插件加载原生写的文件，
  其 `GET /` 与 `GET /status` 与原生输出逐字节相同（`IDENTICAL`）。

## check

`config_all-check.txt`、`prod-snapshot-check.txt`（根目录前缀替换为 `/root/c2chk/root`，两份逐字节相同）：
`SUMMARY gaps=0 cascades=0 globals=0 definitions=126`（M1-c1 之后为 `gaps=1`）。`config_all` 不再有缺口。stderr：生产快照为空，`config_all` 仅 adguard 缺少已下载 `*.rules` 的告警。

## 浏览器冒烟

VM 上运行 debug 原生 `mosdns`（缓存 `cache_all`、两个 `domain_output`、`requery`，上游为本地 python 桩），经 SSH 隧道把本机两套 Vite 开发服务器
（`webui-log` 的 `/log` 壳 与 `log1` 壳，`MOSDNS_DEV_TARGET` 指向隧道）接到它，在内置浏览器中打开“数据管理”：

- 两个壳的“刷新分流缓存”面板都读到状态与配置（`GET /plugins/requery`、`/status`），能力矩阵 `cache.requery` 为 supported，面板未被能力门控。
- `/log` 壳：点“开始全新任务”并确认 -> 任务完成，面板显示“完成于 刚刚（耗时 3秒）”，“刷新域名 3 条”；主机状态 `idle`，3/3，统计文件计数各 +2（A、AAAA）。
- `log1` 壳：面板同样加载上次运行与缓存条目；点“开始全新任务”确认后面板显示“正在执行… 取消任务（0%）”，任务在主机侧完成（3/3）。
- 未点击面板上的“取消任务”（任务 3 秒即完成，取消路径由集成测试覆盖）；控制台里 `my_fakeiplist` 等未配置标签的 404 是 UI 对未配置 tag 的既有探测。
- 冒烟后停掉主机、桩、隧道与两个 Vite 进程；Vite 开发服务器改动的 `coremain/www/*` 已用 `git checkout` 还原。

## 已知差异与未做

见契约“Known differences”与任务内 `research/go-semantics.md`：单一调度器定时器、名字按首次出现顺序、至多 1024 个重查询在途、保存失败时恢复内存设置、
主机停止时记 `cancelled`、编译只读（创建/默认值/恢复在开始服务时）。设置层（M1-f）之前 `execution_settings`/`scheduler` 仍是 UI 维护的运行时文件。

## 审查

Codex（`gpt-6.1-sol`，medium，只读）第 1 轮 FAIL（2 个 P1、2 个 P2）：前一任务结束时清掉后一任务的取消令牌；关闭时 `begin()` 在初次写盘后仍会派生任务而未被 join；
主机停止在 URL/合并步骤中被记为 `failed`；下载对非 200 应答先读响应体。全部修复（任务编号、`begin()` 在写盘后复查生命周期、停机取消优先于步骤失败、
`Call.error_body`，并补测试 `stopping_the_host_during_a_url_step_is_recorded_as_cancelled`）。第 2 轮逐项确认 FIXED，无 P0，`FINAL: PASS`。
