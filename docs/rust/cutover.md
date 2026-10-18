# 首次从 Go 切换到 Rust

这两个脚本用于 Linux amd64、systemd、root 环境中的首次切换。后续 Rust→Rust 升级走 [原生在线更新](contracts/native-update.md)。正式机器的切换须单独确认；不要直接在生产上试跑。

## 准备输入

把 `scripts/cutover/` 整个目录放在目标机器（`go-to-rust.sh`、`rust-to-go.sh`、`cutover.py` 必须在同一目录），准备以下本地文件；脚本不下载任何文件，不需要 jq：

- 发布的 Linux amd64 `mosdns`、同版本 `mosdns-settings-migrate`。
- 原生增量包 `config_rust_up.zip`（schema 1、`rust-config-schema-1`）。不用完整 `config_rust.zip`，避免把默认规则和用户状态覆盖到现有配置根。
- Go schema 4 的 `config_all.zip` 原文以及对应 `config_up.zip`。后者给出受管清单与 hash，前者用于确认现有受管 YAML 没有人手改过。
- 可选的输入 sha256 JSON：键为 `rust_binary`、`migrate`、`config_package`、`go_reference`、`go_package`，值为各文件的 sha256；用 `--sha256-manifest /绝对路径/inputs-sha256.json` 指定。hash 清单必须来自已核对的发布输入，不能只拿来证明下载文件与自己相同。

需有 Python 3、bash、systemd/systemd-run、dig；脚本只使用 Python 标准库。当前 Go 服务必须 active，health 返回就绪，DNS 查询可成功。原生候选的版本必须是 `rust-vX.Y.Z`。

配置根不能是独立挂载点，程序与配置根路径及其内部不能有符号链接/特殊文件/嵌套挂载；若预检因此拒绝，请先人工评估真实目录结构，不要绕过检查。目录整体 rename 需要配置根父目录可写，程序必须位于配置根外。

## 先 dry-run

以下为目标机上的示例路径；按实际安装修改。`--binary`、`--root` 必须和目标服务 ExecStart 一致；`--environment-file` 指定要备份的环境文件。默认健康端口为 9099，DNS 端口为 53。

```bash
bash /opt/cutover/go-to-rust.sh \
  --rust-binary /opt/input/mosdns \
  --migrate /opt/input/mosdns-settings-migrate \
  --config-package /opt/input/config_rust_up.zip \
  --go-reference /opt/input/config_all.zip \
  --go-package /opt/input/config_up.zip \
  --binary /usr/local/bin/mosdns --root /cus/mosdns \
  --service mosdns.service --environment-file /etc/sysconfig/mosdns \
  --health-url http://127.0.0.1:9099/api/v1/system/health \
  --dns-name example.com --timeout 60 --dry-run
```

dry-run 会复制、迁移、生成完整待切换配置、运行 `mosdns check`、备份并输出事务路径；不会停服务、替换运行程序或活动配置根。它会占磁盘空间，产物不会自动删除。

如果受管文件不一致，脚本列出文件名后退出。先人工核对；只有决定**用原生包版本替换这份手改 YAML**后，才在命令中逐个加 `--accept-diff sub_config/xxx.yaml`。它不是合并开关：手改原件留在 Go 备份，活动原生配置取包版本。未受管多余文件只列出并保留。

迁移退出码 1/2 会停止，私有事务目录里的 `migration-*/migration-report.txt` 和 `migration-*.log` 给出原因。不能忽略或自动批准不支持的设置。现有 `webinfo/settings.json` 或 `rust_config_update_state.json` 也会阻止首次切换，防止覆盖已存在的原生设置。

## 正式切换与成功判据

检查 dry-run 的迁移报告和 `check-initial.log` 后，用同样参数去掉 `--dry-run`。正式命令重新做预检，不复用可能过时的 dry-run 副本。

准备阶段 Go 仍运行。停服后会再备份最终配置根、重新迁移/check，捕捉准备期间的设置变化和 Go 退出时保存的缓存/自学习数据。之后原子替换程序，整体换配置目录，再启动。unit 和环境文件不为切换而修改。

命令会输出 `.mosdns-cutover-时间-随机值` 的事务目录与 `mosdns-cutover-…` worker 单元。正式切换由独立 systemd worker 执行：SSH 断开或监视命令 Ctrl-C 不会终止它。查看：

```bash
cat /cus/.mosdns-cutover-时间-随机值/state.json
tail -f /cus/.mosdns-cutover-时间-随机值/worker.log
```

`phase=verified` 且 `dry_run=false` 才是正式切换成功。健康判据是连续三次：服务 active、health HTTP 200/ready、版本等于候选、runtime=rust、applied_config_schema=1，且真实 `dig` 查询 NOERROR 并有应答；可加 `--expect-answer 192.0.2.77` 检查受控探针的 A 记录。`--dns-server` 只允许回环地址，可调整 `--dns-port`。请选安装中确定可应答的域名。

健康超时、启动失败或切换中普通异常会自动恢复 Go。正式切换命令在自动回滚后仍返回非零，`state.json` 为 `rolled_back` 并给出原因。`recovery_required` 表示回滚未能完成，不能当作可用；按下节恢复或根据错误人工处理。

## 回滚（当场或数天后）

**回滚恢复切换前的整个 Go 配置根，不把 Rust 下后来修改的设置、分流组、规则合并回 Go。** 当前 Rust 根会另存为事务内 `rust-root-…`，程序与 unit/env 也另存，供之后人工找回；脚本不删除它们。

```bash
bash /opt/cutover/rust-to-go.sh \
  --transaction /cus/.mosdns-cutover-时间-随机值 \
  --acknowledge-data-loss
```

用**正式切换**事务，不能用 dry-run。回滚验证备份 hash，停止服务，保存当前根，恢复 Go 程序、根、unit/drop-ins 与环境文件，重新启动并检查 Go 版本及 DNS。完成为 `phase=rolled_back`。如果切换 worker 仍在运行，回滚拒绝竞争，等 worker 结束后再执行。若需要中止 worker，用其确切单元名操作，不要匹配所有 mosdns 进程。

worker 被强杀/断电不会自动启动恢复 worker；状态持久化且每次 rename 前有 intent。恢复供操作者通过上述回滚命令完成，不要求 SSH 旧会话存活。不要在 `root_moving/root_moved/root_installing` 等状态下直接反复启动 DNS 服务，先恢复完整安装。脚本不会安装永久启动钩子。

unit/drop-in 集合若在切换后改变，或路径变为链接，回滚拒绝自动覆盖以免误删管理员的新文件；根据备份清单人工处理。已有被改内容会先保存，再恢复。备份目录权限 0700，含配置及可能的凭据，不得上传公开仓库。

## 空间不足

脚本预检按文件系统分别打印需要/可用字节并检查 inode：配置 FS 预算包含四份配置树、输入/解压数据、程序副本，另有 20% 与至少 32 MiB 余量；程序 FS 另算两个临时程序的空间，同 FS 合计。停服务前还会重新检查最后两份配置树的预算。空间不足时退出，**不会自行清理任何文件**。

请由管理员扩容或人工把确认不再需要的文件移走后重跑。成功或失败的事务、dry-run 副本、旧根都保留，避免脚本决定哪些用户数据可删；它们不会自动过期。不要在还有恢复需要时删除事务目录。输入包不是数字签名，仍需从可信发布源核对 sha256。

演练结果见 [F-3b 验证记录](validation-records/10-18-f3b/README.md)。
