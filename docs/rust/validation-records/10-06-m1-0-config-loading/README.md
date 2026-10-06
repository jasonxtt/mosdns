# 10-06 M1-0 之后原生 `check` 对 `config_all` / 生产快照的原始输出

任务：`10-06-rust-native-m1-0-config-loading`。对照基线：[M1-0 之前的输出](../10-06-config-check/)
（`SUMMARY gaps=86 cascades=80 globals=3 definitions=126`）。结论汇总见
[M1 缺口清单](../../m1-gap-list.md)。

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` | `mosdns check -d <root> -c <root>/config_custom.yaml` 的 stdout（退出码 1） |
| `prod-snapshot-check.txt` | 对生产快照同样运行的 stdout（退出码 1），与上一份逐字节相同 |

两次运行的 stderr 均为空（此前 `ip_set` 的 "missing IP rule file, skipped" 警告随路径基准修正消失）。

## 结果

`SUMMARY gaps=64 cascades=80 globals=1 definitions=126`。唯一的 GLOBAL 是
`at least one forward plugin is required`：全部上游经 `aliapi` 定义，`aliapi` 属于 M1-b，
被剔除后没有任何 forward。其余 64 个直接缺口全部是插件类型或 sequence 项（M1-a/b/c）；
没有任何 `listen`/`enable_audit`/`idle_timeout`/`log`/路径类缺口。

## 输入与构建

- 配置包：sibling `file` 仓库 `mosdns/config/config_all`，HEAD `28c6493`，工作树干净；
  `config_custom.yaml` sha256 `ce0b4d857e7371d42021738a7eba6715e1ac9407257fdbc1bbb8a826329f211a`。
- 生产快照：与上一任务同一份（`mosdns-local-backups/prod-config-20261006/`，仓库外），
  `config_custom.yaml` sha256 与 `config_all` 相同。
- 二进制：`mosdns-rust` VM 上 `cargo test` 产生的 debug `mosdns`，sha256 `1628b65f2efbe22637e19ee1501325ab8379c7dc4ef70ad667c0549c7d123f55`；
  源码为 `7c7f02a7` + 本任务工作树改动。
- 运行方式：每份配置依次复制到 VM 同一临时目录 `<root>`（因此输出中的路径标签相同），
  `cd /root && mosdns check -d <root> -c <root>/config_custom.yaml`；运行后删除副本。
  输出只含 tag、类型、字段路径与编译器原因，无凭据值；已检索密码/token/key 类字样，无匹配。
