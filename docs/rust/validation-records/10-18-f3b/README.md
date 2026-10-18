# F-3b：首次 Go→Rust 切换、回滚与最终代码在线升级演练

基线 `af9f067f`，分支 `rust`。仅新增部署脚本、测试与文档，**没有改 Rust 源码**。构建和实测仅在 SSH 别名 `mosdns-rust` 的 VM；未连接生产或 mos-test，未提交、推送、部署、创建 release。外部 Claude 审查待进行，没有执行自我审查轮次。

## 构建与输入

- Rust：最终基线源码、已提交的内置 UI，VM 离线 `cargo build --offline --locked --release -j2 -p mosdns-native-host --bins`，`CARGO_INCREMENTAL=0`、`CARGO_NET_OFFLINE=true`；分别设置 `MOSDNS_BUILD_VERSION=rust-v0.1.0/0.1.1`。两次构建退出 0（首次约 2m33s、第二次约 1m08s）。不变更 UI，因此不运行需联网的 npm ci，不在本机构建。
- Go：`tools/go-reference/build.sh` 固定 `75f2667703c83952e86833852440e07c63f33311`，只读本机 Go 仓库的 bundle 传入 VM，`MOSDNS_GO_REPO` 指向 VM 副本；VM 已缓存 Go 1.26.4、`GOPROXY=off GOSUMDB=off GOTOOLCHAIN=local`，构建成功。第一次误选缓存的 Go 1.26.1，因 go.mod 要求 1.26.4 拒绝，未联网下载。
- 输入：只读 `file` 仓库的 `config_all.zip/config_up.zip/config_rust.zip/config_rust_up.zip` 与仓库外生产快照。副本经 `tools/e2e-1/prepare.py` 做协议保留的本地上游映射，规则加 example.com 受控探针。源快照未改。Go 受管 YAML 与 schema4 原文的差异为 **0**，未使用 accept-diff 放行。
- 网络：单独 netns，仅 lo、无公网路由；沿用 E2E-1 的 TLS/SOCKS/DNS mock。临时 systemd 单元使用 `NetworkNamespacePath`、独立 `/tmp`、`Restart=always/RestartSec=120`，不改变 VM 原服务。探针答案为 `192.0.2.77`。

| 产物 | sha256 |
| --- | --- |
| Rust 0.1.0 | `ce6079f16baa1539508c9ac03b226edaf3836a6123e2d3ced2f2b443315d6420` |
| Rust 0.1.1 | `63c54038e5d69aa14e37714874d5cbaf7c123c34a7b3a1e39400a6247eaf5143` |
| settings migrate | `f1f8be287fc00bbac2aab02da67be41f35dbebbe57ab9a8af57a771f58adbe39` |
| Go 参照 | `d655172c52f375c0aec7b2888ac15a8faccf9e3ab508b39a57c46b2b2a904c6f` |
| cutover.py（最终七项实测源码） | `4b6bb757e60d2729515e09ec1797deddf2fdc2a10d7616acdef1a3488cdbb924` |

## 检查

| 项目 | 结果 |
| --- | --- |
| 两个入口分别 `bash -n` | 通过 |
| shellcheck | VM 未安装；未联网安装，未声称通过 |
| Python 事务边界回归测试 | **8 项通过**；路径越界/zip 链接、受管用户数据拒绝、缺失/手改差异、空间拒绝无写入、坏备份不动服务、原子状态/hash、inactive unit reset 失败后仍启动 Go、两个配置 rename 边界的数据保留恢复 |
| 原始 config_rust `mosdns check` | `SUMMARY gaps=0 cascades=0 globals=0 definitions=128` |
| Rust fmt/clippy/全量测试 | 本次未改 Rust 源码，按 PRD 的条件不重跑；不是本次通过项 |
| 待切换副本的用户数据逐字节检查 | 最终四个事务的 prepared 与初始 backup 对比，受管 YAML 外差异均 **0**（初始 116 个非受管文件，后续事务含运行期新增文件为 123 个） |
| 首次预检空间预算 | 同一 FS 合计约 **353.5 MB**，当时可用约 42.4 GB；不是“只有一个二进制大小” |

## 最终代码上的①–⑦（attempt4，全部符合预期）

独立 UDP 探针每 20ms 发一次、最多等 200ms；“DNS 静默”是相邻两次成功应答的最大间隔，含采样间隔，不能当作精确到毫秒的内核停服计时。无失败时写“未观察到中断”，同时列出普通应答间隔。控制阶段的 `outage_seconds` 另含停服、复制/校验与三次健康检查，不混用两者。

| 演练 | 具体步骤与结果 | DNS 探针（发送/失败；最大成功间隔） | 最终版本 / 服务 / NRestarts |
| --- | --- | --- | --- |
| ① dry-run | 完整准备、迁移、check 与备份；活动程序逐字节仍是 Go，主 PID 不变，未停服务 | 128/0；未观察到中断（0.0239s） | `reference-75f26677` / active / 0 |
| ② Go→Rust | 最终停服快照、重新迁移/check、原子替换、3 次 health+dig；verified；控制阶段 2.5286s | 165/10；**2.0231s** | `rust-v0.1.0` / active / 0 |
| ③ Rust→Rust | 本地 release HTTP + 真正构建的 0.1.1 + 17 文件增量包；令 applied_schema=0 以覆盖包应用；`installed.json` 确认；主 PID `1869921` 不变 | 203/5；**0.6216s** | `rust-v0.1.1` / active / 0 |
| ④ 手动回滚 | 先写 Rust 阶段规则哨兵；回滚 Go，哨兵不在活动 Go 根，在另存的 Rust 根中；程序/unit/env 恢复。控制阶段约 1.4182s | 53/5；**0.8216s** | `reference-75f26677` / active / 0 |
| ⑤ 失败自动回滚 | 候选包加一条与 stub 占用端口冲突的监听，check 可过、start 失败；健康超时 8s，自动回滚 Go，命令退出 1，事务 rolled_back；控制阶段 10.9780s | 214/52；**10.4389s** | `reference-75f26677` / active / 0 |
| ⑥ 空间不足 | VM 主挂载命名空间的小 tmpfs（32 MiB）上运行 Go 副本；脚本预检报 insufficient space、退出 1；未创建事务，未停服务，无自动清理 | 46/0；未观察到中断（0.0249s） | `reference-75f26677` / active / 0 |
| ⑦ 中途强杀恢复 | worker 到 `binary_installed`（新程序已替换、旧配置尚在）后 SIGSTOP+SIGKILL；用公开回滚入口恢复；无需等 RestartSec=120，人工恢复成功 | 166/15；**3.0265s**（含定位/强杀/发起恢复） | `reference-75f26677` / active / 0 |

上述 Go/Rust 版本均为隔离副本的实测版本，不代表生产部署。

## 失败记录与修复过程

没有隐藏之前失败的尝试，私有日志随证据保存在仓库外：

1. attempt1：①②成功，③实际已升级至 0.1.1，但测试误等 health.update.state=verified。最终进程启动清理后该值为 idle；正确的完成凭据是版本、`installed.json`、实际程序与 PID。停掉该轮，修正工具；同时把清理限制为确切的本任务单元，未执行全局 reset-failed，原服务计数未变。
2. attempt2：①②③通过；④在恢复文件后被 `systemctl reset-failed` 对已卸载 inactive unit 的非零退出中断，标记 recovery_required。脚本修为该操作不决定成败，后面的显式 start+Go health/DNS 才是依据；新增回归测试。
3. attempt3：①–⑤通过；⑥把 tmpfs 挂在控制器的私有挂载命名空间，临时 DNS 单元看不到其内容，启动失败。改为 VM 主挂载命名空间挂载，让控制器/服务/worker 共用同一 FS。脚本的手动回滚控制耗时也改为从本次回滚开始计时，避免包含前一轮升级时间。
4. attempt4：用最终脚本从新的私有副本重跑全部①–⑦，结果见上表。之后只新增了 rename 恢复边界的测试，产品脚本字节未变。

## 限制与清理

- 首次切换会有短暂停服；停服后的最终复制/迁移/check 属于这段中断。健康失败时还包含所选超时；本次 ⑤ 约 10.44s，不声称零中断。
- worker 与 SSH 分离；强杀/断电后由操作者执行回滚恢复，没有安装自动开机恢复钩子。⑦实测了程序替换后的强杀；配置根两个 rename 边界由回归测试覆盖。
- 回滚会恢复旧 Go 数据，Rust 阶段的新数据另存，不自动合并；备份/旧根/dry-run 副本不会自行过期删除。
- 原 VM 服务前后均 **PID 827437、active、NRestarts=1**；不是演练服务的 NRestarts=0。
- 任务私有原始证据归档在本机仓库外，未放入任何生产快照/值。VM 快照副本、所有本任务进程/临时单元/netns/tmpfs/任务目录在结束时清除；没有创建 loop 设备。

审查请求在本地任务 `research/review-request.md`；仅执行与验证，等待外部模型审查，不提交、不进入后续任务。

## 规划方审查结论（2026-10-18）

外部审查（Claude）第 1 轮 `FINAL: PASS`，无 P0/P1。逐段读了 `cutover.py`，核对了每个中断点的恢复、数据保留、停服前的空间预检与回滚完整性。
Go 参照 `75f26677` 即生产版本 `v0.7.3`，预检依赖的 health 字段在生产存在。

P2 与切换当天注意事项：
- 原生包 ID 与 schema 1 写死在脚本里；生产切换前若 Rust 版本升级了 schema，要同步改脚本。
- 回滚因空间不足被拒时（尚未停服、系统未改）也标为 `recovery_required`。
- 默认健康超时 60 s，切换失败时 DNS 约中断 60–70 s；`--dns-name` 请选一定能应答的域名。
