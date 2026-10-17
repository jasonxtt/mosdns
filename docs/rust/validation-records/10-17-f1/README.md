# 10-17 F-1：系统与外观（外观服务端、WebUI 端口、重启服务、进程指标）

任务 `10-17-rust-native-f1-system-appearance`，基线 `1cb84b0d` + 本任务工作树。构建、测试、测量在 VM `mosdns-rust`（Linux x86_64，2 vCPU）上进行，本机不构建 Rust。
生产快照只在 VM 私有目录使用（本记录不含域名、配置内容或凭据）；不访问公网；原有 `mosdns` 服务运行前后只读核对。
语义依据 `research/go-semantics.md`，设计 `research/design.md`，契约见 [native-webui-hosting.md](../../contracts/native-webui-hosting.md) 的 “System and appearance operations (F-1)”。

**范围调整（用户 2026-10-17，规划会话转达）：** Rust 版只保留 `/` 下的维护版 Vue UI；`/log` 兼容 UI（`webui-log/src-log1/`）不再需要。本任务没有改 `src-log1`，
浏览器冒烟只做原生 `/` 一次 + Go 一次；两套 Vue 构建照常通过（`/log` 那套构建通过，产物与基线只差版本戳，按范围已还原未提交）；移除 `/log` 由规划会话另开任务。

## 实现

| 项 | 实现 |
| --- | --- |
| 外观（服务端） | `appearance.rs`：`/api/v1/appearance/{panel-background[/upload,/image,/history[/{id}]],text-color,button-color}`；状态文件与 Go 同名同格式（2 空格缩进、无结尾换行、整数不带 `.0`、字段顺序同 Go 结构体）；同目录临时文件+rename 写入；上传 ≤20 MiB（仅此路由放宽请求体上限到 20 MiB+1 KiB），图片类型按 Go 的探测签名；历史无数量上限（与 Go 一致） |
| WebUI 端口 | `system_api.rs`：`GET/POST /api/v1/system/webui-port`；保存到 `webinfo/webui_port_settings.json`；`config.rs` 启动时用它覆盖 `api.http` 端口（文件优先于 YAML，与 Go 一致）；活动地址取管理监听实际绑定的地址 |
| 重启服务 | `restart.rs` + `main.rs` + `RuntimeControl`：`POST /api/v1/system/restart` 立即返回 `scheduled`；延迟后与 `SIGTERM` 同一条优雅停止路径（排空、最终保存），然后 `execve` 同一可执行文件与参数——**PID 不变**；预检失败 500、无记录 501；`execve` 失败则在同一进程内重新启动继续服务；最终保存失败只记录，重启照常 |
| 进程指标 | `process_metrics.rs`：`/metrics` 增加 `process_cpu_seconds_total`、`process_start_time_seconds`、`process_resident_memory_bytes`、`process_virtual_memory_bytes`、`process_open_fds`、`process_max_fds`、`mosdns_process_threads`；**不输出** `go_*` 与 `process_network_*` |
| 能力与 UI | 四项 `ui_operations` 按事实报告；`webui-log/src/`：概览与系统页的原生分支显示进程指标并标注 “GC / Go 版本 / goroutines 不适用于 Rust 运行时”；系统页的 WebUI 端口面板此前被整个包在 “非原生才显示” 的容器里，已改为原生也显示（只隐藏 Go 专有的两个模式面板）；Go 分支不变 |

## 测试

- 单元（`appearance/tests.rs` 14 项、`system_api.rs` 4 项、`restart.rs` 3 项、`process_metrics.rs` 4 项）：Go 写出的状态文件字节样本读取与回答、写出的版式、归一化规则、上传上限（恰好 20 MiB 通过、多 1 字节拒绝）与类型/空文件/缺少文件、
  multipart 解析（多个部分、带引号边界、二进制内容）、历史增删与清空（含删除当前背景后设置回到 none）、写失败保持旧状态且不留临时文件、损坏文件读作默认值、ID 校验与路由/方法区分、
  端口覆盖优先级与非法文件、POST 校验与占用检测、预检与 `execve` 失败、进程指标解析与数值范围。
- 集成（`tests/f1_system.rs`，真实 `mosdns` 二进制副本 + 本地 UDP 桩上游）：**重启保持 PID、`/proc` 启动时间不变、重启后缓存 dump 被加载（上游已关闭仍能应答）、设置保持、之后 `SIGTERM` 仍正常退出**；
  预检后撤销执行权限导致 `execve` 失败时同一进程继续服务、之后的请求被预检拒绝（500）而不停服务、恢复权限后重启成功；最终保存失败（缺目录）时重启照常（日志写明）而无重启请求的同一故障仍是退出码 2；
  保存的 WebUI 端口在下次启动生效且优先于 YAML、旧端口关闭、占用端口 409、越界 400；3 MiB 上传（超过默认 1 MiB 上限）成功，21 MiB 被拒且服务不受影响；
  **同一时刻只读取一个上传**（Codex 第 1 轮指出多个并发的 20 MiB 请求体可无界占内存：现在在读请求体之前先占用唯一的上传名额，第二个并发上传立即 503，停滞的第一个上传超时/断开后名额释放；请求体不再多拷贝一份）；`/metrics` 含进程指标且没有 `go_goroutines`/`go_info`。
- 既有测试的对应修改：`slice6_management_http.rs`（四项能力中三项现为 supported、路由不再 404、重启在进程内测试主机上 501）；`cache_http.rs`（`/metrics` 的缓存部分仍是 8 行，进程指标是单独段落）。
- Node（`webui-log/tests/`）42 项通过（新增 `systemProcessMetrics.test.mjs` 5 项：路径到能力的映射、能力校验、原生分支进程卡片与 Go 项标注、WebUI 端口面板不再被隐藏、`/log` 壳文件未被改动）。

## Go 互通（`tools/f1-interop`，VM，Go 参考二进制由同一仓库构建）
`interop.py`：同一组 34 个请求（颜色、背景各模式、非法输入、3 次上传含文本/空文件、选用/删除/清空历史……）分别经 Go 处理器与原生主机执行：**状态码与响应体逐字节一致**（上传 ID、`?v=` 版本戳与时间戳归一化后），
**最终状态文件逐字节一致**，历史文件列表与图片哈希一致；再做交叉读取：Go 读原生写的目录、原生读 Go 写的目录，响应一致；
WebUI 端口：原生保存的文件被 Go 的 `ResolveWebUIPortOverride` 解析为同一端口，Go 格式的端口文件被原生读取。结果 `INTEROP OK`。

## VM 真实验证

**重启（生产形态配置：生产快照迁移出的 `config_rust`、模拟上游、隔离 netns；探测每 10 ms 一个 UDP 查询，6 s 时请求重启 `delay_ms=300`，×3）**

| 运行 | 请求→状态 | 最长无应答 | 无应答探测 | 管理 API 不可用 | PID | 最终保存的 dump |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 200 | 0.395 s | 39 / 4000 | 0.314 s | 不变，`/proc/<pid>/exe` 不变 | 7 个，16.9 KB |
| 2 | 200 | 0.409 s | 41 / 4000 | 0.401 s | 同上 | 7 个 |
| 3 | 200 | 0.401 s | 40 / 4000 | 0.400 s | 同上 | 7 个 |

即真实重启时 DNS 中断约 **0.40 s**（`delay_ms` 的 300 ms 等待期间 DNS 仍在服务，中断从监听关闭开始）；与 5D-2c 的热应用停顿（进程内、不关监听，约 0.4–0.65 s）量级相同但性质不同——这里关闭了监听，窗口内的查询被丢弃，客户端重试即可。
每次运行的 `cleanup.json` 均为全真（自有进程已退出、netns 已删除、原服务不变）。

**WebUI 端口（同一生产形态配置）：** 快照里保存的端口 80 在原生启动时生效（`active_port=80`，对应 Go 同样的行为；这也使 5D 工具里的 API 端口不再是 YAML 的 9099，`harness.py` 新增 `saved_port_applies`）；
POST 8081 → `pending_restart=true`，保存不切换监听；重启后 API 在 8081、80 已关闭、PID 不变；该配置里的生产外观文件与历史图片被原生读出；`/metrics` 的进程指标有值（RSS 43.5 MB、FD 34、线程 5）。

**浏览器冒烟（VM 上原生与 Go 各起一个最小配置，经 ssh 隧道由 Chrome 窗格访问，测试用 PNG 为合成图）：**
- 原生 `/`：系统页显示运行时 Rust、产品版本、启动时间、CPU 时间、RSS、打开文件描述符、线程数与 “GC、Go 版本、goroutines：不适用于 Rust 运行时”，“重启 MosDNS” 可点；
  外观面板列出上传的历史图片，选用并应用后整页背景变为所选图片、`appearance_settings.json` 里 `upload_id` 正确；删除历史图片经确认框后文件被删除；
  WebUI 端口面板（修复前被隐藏）出现，输入 19097 → 确认 → 页面在重启后自动跳转到新端口，同一 PID；背景在重启后保持。
- Go `/`：系统页保持原样（Go 版本、待用堆内存等 Go 行），端口面板与重启按钮可用。
- 一个观察：在原生页面里把 `19099` 追加成 `1909919097` 会被 UI 自己的 1–65535 校验拒绝（输入框预填了当前端口），属 UI 既有行为。

## 构建与检查

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 见下方“最终运行” |
| `mosdns check -d config_rust -c config_custom.yaml` | 见下方 |
| 两套 Vue 构建（`npm run build`、`npm run build:log1`，与 Rust/Go 构建串行） | 均成功；`/` 的产物更新并入本任务，`/log` 的产物无源码变化已还原 |
| Go 构建 | 成功（用于互通与冒烟） |

## 已知限制（如实记录）
- 重启窗口内（排空结束到 `execve` 的约 0.3 s）收到的 `SIGTERM` 会被吞掉，进程仍会重启；此时 `systemctl stop` 要等到超时。
- 历史图片无数量上限（与 Go 一致；WebUI 无鉴权属受信管理面），单次上传 ≤20 MiB。
- 不支持图片路由的 Range/条件请求；不迁移旧位置（`state/`、根目录）的状态文件，只读 `webinfo/`。
- `process_start_time_seconds` 是进程的启动时间，原地重启后不变（Go 同样）。
- `/log` 兼容 UI 未改动（范围调整）。
- 本任务中有一次用 shell 追加（`cat >>`）而不是编辑工具往测试文件加一个用例；VM 上的 `rustfmt` 格式化结果被拉回本地（不是脚本改写源码逻辑）。

## 最终运行
最终工作树：`cargo fmt --all -- --check` 退出码 0；`cargo clippy --workspace --all-targets -- -D warnings` 退出码 0；`cargo test --workspace --no-fail-fast` **1522 passed，0 failed，4 ignored**（基线 1491，新增 31）；
`mosdns check -d config_rust -c config_custom.yaml`：`SUMMARY gaps=0 cascades=0 globals=0 definitions=128`；Go 互通 `INTEROP OK`；Node 42 项通过。
说明：重启实测与 WebUI 端口实测用的是修复端口面板之前构建的 release 二进制，之后只改了 UI、测试与文档，Rust 生产代码没有变化。

## 规划会话核对（2026-10-17）

Codex（gpt-6.1-sol）第 2 轮 FINAL: PASS；规划会话核对后验收。范围按用户决定只涉及 `/` UI，`/log`（src-log1 及其产物）未改动。
原地重启实测最长无应答约 0.40 s，PID 不变，不依赖 systemd（生产 RestartSec=2min）。
