# 10-06 原生 `check` 对 `config_all` / 生产快照的原始输出

任务：`10-06-rust-native-config-check-ledger`。结论汇总见 [M1 缺口清单](../../m1-gap-list.md)。

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` | `mosdns check -c config_custom.yaml` 的 stdout（退出码 1） |
| `prod-snapshot-check.txt` | 对生产快照同样运行的 stdout（退出码 1），与上一份逐字节相同 |
| `config_all-probe-check.txt` | **派生探测**，不是原配置结果：见下文 |
| `config_all-check.stderr-summary.txt` | 编译器写到 stderr 的警告（`sort \| uniq -c`），两份运行相同 |

## 输入

- 配置包：sibling `file` 仓库 `mosdns/config/config_all`，HEAD `28c6493`；该目录与规划时记录的
  `86e6a65` 无差异（`git diff --quiet 86e6a65 28c6493 -- mosdns/config/config_all`）。
- 生产快照：2026-10-06 从生产 `mosdns`（Go v0.7.3）只读复制的 `/cus/mosdns`，保存在仓库外
  `mosdns-local-backups/prod-config-20261006/`；`config_custom.yaml` sha256
  `ce0b4d857e7371d42021738a7eba6715e1ac9407257fdbc1bbb8a826329f211a`。快照未进入仓库；
  VM 上的临时副本运行后已删除。输出中没有密钥/口令/token 值（无需遮蔽：check 只输出
  tag、类型、字段路径与编译器原因，且两份配置都未开启 `native_management`，`webinfo`
  未被读取）。
- 构建：`mosdns-rust` VM，`cargo test -p mosdns-native-host` 产生的 debug 二进制，
  sha256 `2314d293bbe68fb611bbd25838f0f516da29b9532aee8d295fd71dd360226dc0`（review 第 1 轮修复后重跑，输出不变）；源码为
  `83b86cd9` + 本任务工作树改动。运行目录为配置根目录，`-c config_custom.yaml`。
  自查修复前的二进制（`7f6c781e…`）对两份配置产生逐字节相同的输出；探测输出由该较早
  二进制产生（修复只影响无法解码的定义和迭代上限，`config_all` 两者都不涉及）。

## 派生探测（probe）

原配置中 20 个 listener 全部因 `listen: ":port"` 被剔除，随后 `missing listener plugin`
作为 GLOBAL 结束检查，后续层次不可见。为看到下一层，在 VM 临时副本上做了两处**仅用于
探测**的改动后重跑：

1. `listen: ":N"` → `listen: "0.0.0.0:N"`；
2. 在 `sub_config/` 下建立 `rule`、`gen`、`srs`、`cache`、`adguard` → `../` 的符号链接，
   模拟 Go 以配置根目录解析相对路径（见清单 §3.1）。

探测结果揭示：`enable_audit` 原生为必填、多个 listener 需要 `native_management`，以及
全部 `aliapi` 被剔除后 `at least one forward plugin is required`。再往下需要先实现 M1-b，
因此探测止于此。探测输出不能当作原配置的检查结果引用。
