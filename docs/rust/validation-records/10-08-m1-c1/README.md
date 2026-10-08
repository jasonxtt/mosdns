# 10-08 M1-c1：原生 `domain_output` 的验证记录

任务 `10-08-rust-native-m1-c1-domain-output`，基线 `e3ff31c4` + 本任务工作树。构建、测试、测量全部在 VM `mosdns-rust`
（Linux x86_64，2 vCPU，glibc，THP `always`）上进行，本机不构建。生产快照（`prod-config-20261006`）只在仓库外使用。
契约见 [native-domain-output](../../contracts/native-domain-output.md)。

## 构建与测试

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0 |
| `cargo test -p mosdns-native-host --test m1_c1_domain_output` | 10 passed |
| `cargo test --workspace --no-fail-fast` | 1403 passed，0 failed，4 ignored（此前 1385） |

新增测试：

- 集成 `rust/native-host/tests/m1_c1_domain_output.rs`（真实 UDP 查询、真实管理 listener、本地 HTTP 接收端）：新名字下一次查询即带
  `target_mark`（无重建）、随后进程内发布到 `domain_set_light` 并重建后热条目仍在（两个 mark 同时生效）；`/show` 分页/过滤/`X-Total-Count`
  与加载（两字段/三字段/坏行/重复）；`/save` 三个文件的格式、`/flush` 清空并截断、方法/未知动作/`/restartall` 状态码；关闭时最终保存与重启读回
  （计数累加）；`enable_flags` 键；`max_entries` 提前写；写失败 500、其余文件照写、无临时文件残留、内存不变；外部 `domain_set_url` 的 HTTP POST
  请求行/头/体；非法参数；**参考实现写出的文件**（`tests/fixtures/domain_output/`，由 `tools/m1c1-interop` 生成）被原生读入后 `/show` 与参考一致、
  再保存后内容集合一致。
- 单元：`domain_output.rs`（AD/CD 取位、过期快照不覆盖新快照、文件排序/去重/格式、键规范化、键数上限、覆盖层注入一次）、`local_time.rs`。

## 文件互通（双向）

- **参考 → 原生**：`tools/m1c1-interop` 实例化参考 `domain_output`，执行查询并 `/save`，其 `stat.txt`/`rule.txt` 与 `/show` 输出作为 fixture
  （`rust/native-host/tests/fixtures/domain_output/`）；上面的集成测试读入并对照。
- **原生 → 参考**：`tools/m1c1-interop/native_to_reference.sh`：原生 host 处理一组查询，`/save`，SIGTERM 退出；参考插件加载原生写的统计文件，
  其 `/show` 与原生 `/show` 逐行一致（按计数与键排序后 `IDENTICAL 5 5`）；原生文件内容：大小写混合名被小写、`|AD`/`|AD|CD` 键、`full:` 规则文件均按参考格式。
- 取证发现：参考把查询交给插件前已用新 OPT 替换客户端 OPT，其 DO 判断恒为假（生产 `top_domains.txt` 只有 `|AD` 键），原生不把 DO 放进键。

## check

| 文件 | 内容 |
| --- | --- |
| `config_all-check.txt` | `mosdns check -d <root> -c <root>/config_custom.yaml`（`file` 仓库 `config_all`）stdout，退出码 1 |
| `prod-snapshot-check.txt` | 生产快照同样运行，与上一份逐字节相同 |

`SUMMARY gaps=1 cascades=0 globals=0 definitions=126`（M1-b1 之后为 `gaps=9 cascades=30 globals=0`）。`domain_output` 8 个直接缺口及 30 个连带错误全部消失；
唯一剩余缺口是 `requery`（M1-c2）。stderr：`config_all` 只有 adguard 缺少已下载 `*.rules` 的告警。输出只含 tag、类型、路径与编译器原因。

## 资源（release，`scripts/measure-domain-output.py`，生产快照的规则集 + 8 个 `domain_output` 实例 + 每个查询执行全部 8 个实例）

| 场景 | 启动稳态 RSS | 记录后 | 8 次 `/save`（进程内发布 → 8 次 mapper 重建）后 | 重建期间 VmHWM | 保存期间延迟（200 qps 单客户端） |
| --- | --- | --- | --- | --- | --- |
| 无 `domain_output`（基线），20,000 个不同名字 | 37.1 MB | 37.2 MB | — | — | — |
| 8 个实例，20,000 个不同名字（生产量级约 1–2 万） | 39.0 MB | 57.9 MB（+19 MB，约 120 B/键） | 79.1 MB | 195 MB | p50 241 µs，p99 3.3 ms，最大 21.5 ms，0 超时 |
| 8 个实例，100,000 个不同名字（压力；覆盖层分层改动之前的构建） | 38.7 MB | 123.1 MB | 162 MB | 476 MB | p50 190 µs，p99 21.4 ms，最大 38.9 ms，0 超时 |

记录吞吐 26–29k 查询/秒（顺序单客户端，含 8 个实例）。保存后的 RSS 增量主要是各实例发布的名字进入 mapper（每个实例 2–10 万条 `full:`）。
延迟样本来自与 `/save` 请求共用一个 Python 进程的客户端，最大值可能含客户端自身的 GIL 竞争；`/save` 的快照在 DNS 线程上克隆统计表
，其耗时随键数增长（压力场景 p99 升高与之一致，未单独计时），生产量级下每个 10 小时周期一次。键数上限 262,144 使统计表最多约 25 MB/实例。
原始 JSON：`rss-20k-names.json`、`rss-20k-names-without-outputs.json`、`rss-100k-names.json`。

## 已知差异与未做

见契约与任务内 `research/go-semantics.md`：键小写化、不记 DO、根名不记录、原子写、键数上限、`/restartall` 501、未知 `target_mapper` 为配置错误、
无 quick 形式、生成开关恒开（待 M1-f）、`flush`/`save` 写失败返回 500（参考恒 200）。覆盖层在重建后保留（参考会清空，属参考缺陷）。

## 审查

Codex（`gpt-6.1-sol`，medium，只读）第 1 轮 FAIL（5 个 P1、1 个 P2）：已发布任务未回收；`/flush` 后覆盖层不清；进程内发布与 fence 的竞争；
managed 安装丢失已学习条目；`127.0.0.x` 被误判为本机；`/save` 缺 Content-Type。全部修复（覆盖层按插件分层、flush 清层、启动时把已加载名字加入层、
fence 在发布未完成时不写规则文件、进程内调用不再被超时丢弃、`is_own` 精确化、回收 JoinSet、补 Content-Type）。第 2 轮逐项确认 FIXED，无 P0，
`FINAL: PASS`。遗留设计限制（已写入契约“生命周期”）：managed 应用恰好遇到发布进行中时，规则文件可能比统计晚一次写入。

## 规划会话处理（2026-10-08）

- `/restartall` 原生返回 501：WebUI 未使用；保持现状，自重启/服务管理并入后续 C14（Linux 服务生命周期）再定。
- 生成开关（Go `domain_generation_settings.json`）：并入 M1-f 类型化设置层，原生目前恒开。
- `native_management` apply 与进行中发布的遗留时序限制：接受（`config_all` 不用该模式，已写入契约）。
- 内存：生产快照 `gen/top_domains.txt` 约 1.05 万条，其余列表更小；2 万名字场景（重建峰值约 195 MB、8 次保存后稳态约 79 MB）
  已高于生产规模，10 万名字仅为压力参考。5D 冻结 RSS 预算时复测。
