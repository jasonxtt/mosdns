# 10-17 UI-1：原生去掉 `/log`，外置 UI 包 `/<name>/`

任务 `10-17-rust-native-ui-1-external-main`，基线 `8de28c5e` + 本任务工作树。构建、测试、冒烟在 VM `mosdns-rust`（Linux x86_64，2 vCPU）上进行，本机不构建 Rust；生产快照未使用；不访问公网。
设计见任务 `research/design.md`，契约见 [native-webui-hosting.md](../../contracts/native-webui-hosting.md)（“External UI packages”），使用说明 [docs/rust/external-ui.md](../../external-ui.md)。

**用户补充澄清（2026-10-17）：** 外置 UI 支持任意目录名，`ui/<name>/` ↔ `/<name>/`（`main` 只是示例）；包与挂载名无关（相对路径）；manifest 只检查带 manifest 的目录，无 manifest 的目录照旧作为通用静态挂载；
health 列出全部外置目录，内置 `/` 对所有已挂载且兼容的包显示链接。本任务按此实现，没有 `ui/main` 特例。

## A. 原生去掉 `/log`
- `build.rs`：只内嵌 `/`（`log.html`）与 `coremain/www/assets`，跳过 `assets/vue-log1`；`static_ui.rs` 去掉 `/log`、`/log/` 分支；`/log`、`/log/`、`/log1`、`/assets/vue-log1/*` 在原生都是 404（测试覆盖）。
- `scripts/build-rust-native.sh` 不再 `npm run build:log1`；`scripts/native-build-manifest.mjs` 不再哈希 `src-log1` 等，资产清单跳过 `vue-log1`（实测 `prepare` 只含 `/` 的 4 个资产文件）。
- Go 侧未改：`coremain/`、`webui-log/src-log1/`、`vite.log1.config.js`、`coremain/www/log1.html` 与 `assets/vue-log1` 的内容都没动（`build:log1` 构建通过，产物仅版本戳不同，已还原）；`log`/`log1` 仍是外置挂载的保留名。
- 文档：契约、`config-compatibility.md`、`AGENTS.md`、`docs/ai/project-context.md` 写明 “Rust 版只提供 `/`，`/log` 仅 Go 版保留”。

## B. 外置 UI 包
- **构建**：`npm run build:external`（`scripts/build-external-ui.mjs` + `webui-log/vite.external.config.js`）从同一份 `src/` 生成 `webui-log/dist-external/package/`（`index.html`、`app.js`、`app.css`、`ui-manifest.json`）和 `release/mosdns-ui-<版本>.zip`；资源是**相对路径**（`./app.js?v=<版本>`），因此与目录名无关；内置 `/` 的 `npm run build` 不变。
- **manifest**：`{"manifest_schema":1,"ui_version","built_at","api_level","requires_api_level"}`；宿主 `HOST_API_LEVEL=1`、`MIN_UI_API_LEVEL=1`，兼容 ⇔ `requires_api_level ≤ HOST_API_LEVEL` 且 `api_level ≥ MIN_UI_API_LEVEL`（`ui_manifest.rs`，Node 测试保证构建脚本的 `API_LEVEL` 与它相等）。
- **挂载（`external_ui.rs`）**：对 `ui/` 下每个目录：无 manifest → 通用静态挂载（原行为），health `kind:"static"` 并注明“非 mosdns UI 包（无 manifest）”；有 manifest 且兼容且 `index.html` 是普通文件 → 挂载（`kind:"mosdns_ui"`，带版本）；
  有 manifest 但不兼容/损坏/过大/是符号链接/缺 `index.html` → 不挂载（`/<name>/` 404）并给固定中文原因；保留名/非法名/符号链接目录 → `kind:"unusable"`；`ui/` 下的普通文件忽略。manifest 通过已固定的目录句柄 `openat2`（BENEATH+NO_SYMLINKS）读取，仍走原有的安全读取。
- **health**：`external_ui` 数组（按名字排序，≤64 项）：`name, mounted, kind, ui_version, built_at, reason`，不影响 `ready`。
- **内置 `/`**：系统页对每个已挂载的 mosdns UI 包显示“检测到外置 UI「name」版本 X，前往 /name/”（当前页面所在目录那项不显示），对被拒的包显示原因；WebUI 端口修改后的跳转保留页面自己的目录。

## 测试与构建
| 项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | **1529 passed，0 failed，4 ignored**（基线 1522，新增 7） |
| `mosdns check -d config_rust -c config_custom.yaml` | `gaps=0 cascades=0 globals=0 definitions=128` |
| Node（`webui-log/tests/*.test.mjs`） | 48 通过（新增 6：manifest 构建纯函数与级别一致、外置构建配置名字无关、构建脚本不再构建/哈希 `/log`、链接面板与端口跳转的源码约束……） |
| Vue 构建（`npm run build`、`npm run build:log1`、`npm run build:external`，串行，早于 Rust/Go 构建） | 均成功；`/` 的产物更新并入本任务，`/log` 产物还原 |
| Go 构建 | 成功（只用于冒烟） |

新增/改动的 Rust 测试：`ui_manifest`（3：兼容矩阵、固定原因、版本显示）；`external_ui`（3：`pg` 与 `a` 同一个包两个名字、无 manifest、不兼容/坏 JSON/缺 index、保留名、README 被忽略、符号链接 manifest/过大 manifest/符号链接目录）；
`slice6_management_http`：`external_ui_packages_use_any_directory_name_and_a_manifest_decides_mounting`（真实 HTTP：health 列表、`/pg/`、`/a/` 的 index/资源/MIME/`no-store`/301、文件替换实时生效、无 manifest 目录照常服务、被拒包与保留名 404、`/` 正常）与更新后的内置 UI 测试（`/log*`、`vue-log1` 资源 404）。

## VM 浏览器冒烟（最小配置 + ssh 隧道，Chrome 窗格；外置包 `ui1-smoke-2` 放在**非 main** 的目录名 `pg` 与 `a` 下，`plain` 是无 manifest 的静态页）
- health 列出 `a`、`pg`（`mosdns_ui`，已挂载，版本 `ui1-smoke-2`）与 `plain`（`static`，“非 mosdns UI 包（无 manifest）”）。`/`、`/pg/`、`/a/`、`/plain/` 为 200，`/log`、`/log/`、`/assets/vue-log1/app.js` 为 404。
- 内置 `/` 系统页显示两条链接“检测到外置 UI「a」…前往 /a/”“…「pg」…前往 /pg/”（`plain` 没有链接）；点击后进入 `/pg/`，外置页的系统页只显示指向 `/a/` 的链接（不链接自己）。
- **在 `/pg/` 里完成真实操作**：点击“停止审计”→ `/api/v1/audit/status` 变为 `capturing:false`，再点“启动审计”→ `true`（外置页对同一宿主做读写）。
- 文件替换实时生效：给 `ui/pg/index.html` 追加标记，下一次请求即返回新内容。
- manifest 三种情况（改动后经 `POST /api/v1/system/restart` 原地重启重新评估）：① **兼容**：初始的 `pg`、`a`（挂载、有链接）；② **缺失**：删除 `a` 的 manifest → `a` 变为 `static`（`/a/` 仍 200，作为通用静态页，不再有链接），`pg` 保持；
  ③ **不兼容**：把 `pg` 的 `api_level`/`requires_api_level` 改成 2 → `pg` 为 `mounted:false`、原因“此 UI 需要接口级别 2，当前程序只提供 1，请升级程序”，`/pg/` 与 `/pg/app.js` 为 404，`/`、`/a/`、`/plain/` 不受影响；内置 `/` 系统页显示“外置 UI「pg」未挂载：…”。
- Go：`/` 系统页保持 Go 版样子（Go 版本、待用堆内存），`/log` 仍为 200。

## 已知限制（如实记录）
- 挂载与 manifest 判定只在启动时做：新包的版本/兼容性要重启一次才会重新评估（F-1 的原地重启约 0.4 s）；已挂载包的同版本文件替换实时生效。
- 目录名无效或为保留名的目录只是在 health 里列为 `unusable`，不报错。health 最多列 64 个目录。
- `/log` 的 Go 侧保留；Go 仍内嵌 `log1`，`src-log1` 源码未删（Go 过渡代码在 Phase 6 统一处理）。
- 过程偏差：给 `.gitignore` 加一行时用了一次 shell 追加（`printf >>`），违反“改文件用编辑工具”；VM 上 `rustfmt` 的格式化结果被同步回本地（只是格式）。

## 规划会话核对（2026-10-17）

Codex（gpt-6.1-sol）第 1 轮 FINAL: PASS；规划会话核对后验收。Go 侧（`src-log1`、`vue-log1`、`log1.html`、`coremain` Go 代码）未改；
`coremain/www` 的变更仅为 `/` UI 构建产物。外置目录名任意（`ui/<name>/` → `/<name>/`），已用 `pg`、`a`、无 manifest 的 `plain` 验证。
