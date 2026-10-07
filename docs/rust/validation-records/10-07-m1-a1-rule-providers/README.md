# 10-07 M1-a1：规则 provider / SRS / domain_mapper / mark 的验证记录

任务 `10-07-rust-native-m1-a1-rule-providers`。语义取证与分级见任务内 `research/go-semantics.md`；
行为契约见 [native-rule-providers](../../contracts/native-rule-providers.md)。

## 差分（R5）

oracle：`tools/m1a1-oracle`（Go，直接实例化现有 `domain_set_light`/`sd_set_light`/`si_set`/
`adguard_rule`/`domain_mapper` 并执行 `Exec`）；原生侧：`rust/native-host/examples/m1a1_oracle.rs`
（用 `-d` 根目录编译同一份定义，经 `CompiledConfig` 求值）；`tools/m1a1-oracle/run.sh` 一键运行。
规则文件与语料**不在仓库**，这里只有统计。

| 输入 | 语料 | 结果 |
| --- | --- | --- |
| 生产快照（Go v0.7.3，16 个 provider、2 个 mapper、1 个 `si_set`；148,532 条唯一规则） | 1,579,234 条（域名 1,549,028 + IP 30,206；见 [prod-corpus-stats.json](prod-corpus-stats.json)） | 原生与参考**逐条一致 1,579,234 / 1,579,234**，差异 0；原生唯一规则数 148,532 与 Go 导出一致 |
| 合成树（`tests/fixtures/m1_a1/tree`：无效 regexp、无前缀/未知类型规则、`@@`、通配、禁用/缺失/非 SRS 来源、空名来源…） | 46,777 条 | 一致 46,777 / 46,777（来源名连接顺序在 Go 侧每次运行随机，`compare.py` 只对来源名排序比较；各次运行该数在 0 与十几条之间波动，最终一次为 16 条），差异 0 |

语料生成：对 Go 导出的每条规则生成本体、子域、深子域、父域、相邻名、大小写变体（查询名恒以 FQDN 形式即带尾点给出，没有“无尾点”变体），keyword/regexp
的正例与反例（regexp 正例由模式机械推导并用 Python `re` 验证，推导不出的模式只有反例：生产 1 条、合成 2 条，见各 corpus-stats 的 `regexp_rules_without_a_derived_positive`；regexp 的 Go 保真另由 `regexp_parity.py` 覆盖），另有 20k 随机负例与边界名；IP 取每个 CIDR 范围的首/末地址、范围外相邻地址（范围用独立的
Python SRS 解码得到，不依赖任一实现）、随机 v4/v6 与特殊地址。

### Go regexp 保真（C2C 第 2 轮）

规则里的 regexp 不允许静默丢失：Go 的 `regexp` 为参考，`tools/m1a1-oracle/regexp_parity.py`（[结果](regexp-parity.json)）用 257 条模式 × 39 个名字对比：
Go 与原生都接受的 143 条匹配向量逐一相同；Go 拒绝的 77 条原生也拒绝（跳过，同 Go）；合法 Go 但原生不跑的 37 条
（`\p{L}`、`\Q…\E`、命名组、字面花括号、`\a` 等）使配置**启动失败并指明规则**（mapper 与直接 `qname $adguard` 两条路径都是），
违规（Go 合法却被当无效丢弃、或 Go 拒绝却被接受）0 条。`\d \w \s \b`、`(?:…)`、`(?i)`、POSIX 类、`\xHH`、惰性量词、计数重复等常见写法现已支持
（输入恒为 ASCII 名，Go 的 ASCII 类与 Rust 的 Unicode 类在 ASCII 上一致）。

### Manifest 解码保真（Codex 第 2 轮）

`tools/m1a1-oracle/manifest_parity.py`：53 种 manifest 写法（键大小写与重复键、`null`、类型错误、非法与边界 `last_updated`（Go 的 RFC 3339 规则）、
int64 溢出、非数组、尾随数据、空文件、`sd_set_light` 与 `si_set` 各自的字段集合差异）分别让 Go 与原生加载同一份 SRS，
`sd_set_light` 比较导出的唯一规则数、`si_set` 比较地址命中：**53/53 一致**（含 Go 的 RFC 3339 偏移范围 hh≤24、mm≤60、一位小时，以及 `ſ`/`K` 大小写折叠键）。`adguard_rule` 的 manifest（`id` 字段集合）只有原生单元测试覆盖，没有 Go 对照。

### 大小写偏差下的继承保真（Codex 第 10 轮）

“小写化 Go”的差分会把 `full:` 规则的继承一并小写化，盖住继承上的差异。因此另有 `tools/m1a1-oracle/inheritance_parity.py`
（[结果](inheritance-parity.txt)）：6 组带大写 `full:` 与不同大小写 `domain:` 的规则，让**原版** Go 回答 full 规则的原样拼写，
原生回答该拼写及其小写、大写、反转大小写，答案必须与 Go 逐项相同：**6/6 一致**。每个 `full:` 变体仍按自己的拼写继承，变体合并发生在继承之后。

### 一个有意偏差（用户 2026-10-07 批准，选项 B）

参考 mapper 对 `full:` 规则走区分大小写的精确表，其它规则不区分。生产规则里有约 1000 条含大写的 `full:`
规则（realip 532、nov6 458、nov4 18；964 条没有小写形式）。原生**不区分大小写**，同键规则合并。
因此对比用两份 Go 答案：原样快照（`--go`）和把 `full:` 文本规则全部小写后用小写名查询
（`--go-lower`，即“参考行为 + 不区分大小写 full”）。验收要求原生 = `--go-lower`（全部一致）；
两份 Go 答案不同的语料条数（偏差影响面）：生产 8,751 条，合成 5 条。

差分过程中发现并修复的问题：(1) 原生最初用仅限主机名字符的 qname 助手，`_http._tcp.…`/前导连字符名
不命中（真实 apt SRV 查询会受影响），改为与参考一致的呈现格式名；(2) `ValuedDomainMatcher` 对仅大小写不同的
`full:` 键后者覆盖前者，改为合并。

## check（R7）

`mosdns check -d <root> -c <root>/config_custom.yaml`：[config_all](config_all-check.txt) 与
[生产快照](prod-snapshot-check.txt) 输出除根目录路径前缀外逐行相同。

| | gaps | cascades | globals |
| --- | --- | --- | --- |
| M1-0 之后 | 64 | 80 | 1 |
| **M1-a1 之后** | **45** | **66** | **1** |

M1-a 的 `domain_set_light`（11）、`sd_set_light`（4）、`domain_mapper`（2）、`si_set`（1）、`adguard_rule`（1）及其
连带全部消失（含 `resp_ip $geoip_cn` 的 10 处连带）。剩余 45 个直接缺口：
`sequence` 项 27（`drop_resp`/`rcode`/`tag_setter`/`sleep`，M1-b）、`domain_output` 8（M1-c）、`aliapi` 5、
`fallback` 裸 tag 2、`rewrite`/`webinfo`/`requery` 各 1；唯一 GLOBAL 仍是
`at least one forward plugin is required`（上游全部经 `aliapi`）。
config_all 包里的 `adguard/config.json` 带启用列表但没有已下载的 `*.rules`，所以该运行的 stderr 里
有 `adguard: skipping enabled rule … cannot open …` 告警（Go 启动时会联网下载，原生不下载，见契约）；
生产快照里文件齐全，stderr 为空。构建：debug `mosdns`，sha256 `1ba2a3ea05c05548a827f09a94142a4e7da7f64855f279287938ddb2baecfdf7`（源码 `a88393ba` + 本任务工作树）。
config_all：`file` 仓库 `28c6493`，`config_custom.yaml` sha256 `ce0b4d85…211a`（与快照相同）。

## 资源（R6，诊断，不设门槛）

同一 VM（`mosdns-rust`，Linux x86_64）、同一生产快照、只加载这 16 个 provider + 2 个 mapper + `si_set`
（Go：`tools/m1a1-oracle`，release；原生：`m1a1_oracle` example，release；各 5 次，`wait4` 的 maxrss）：

| | 启动到加载完成 | 峰值 RSS | 加载后稳态 RSS |
| --- | --- | --- | --- |
| 原生 | 0.64–0.67 s | 167 MB | 90 MB |
| Go v0.7.3 代码 | 0.44–0.47 s | 144–150 MB | 144 MB（强制 `FreeOSMemory` 后 48 MB） |

原生稳态约为 Go 强制回收后的 1.9 倍，峰值略高。主要占用来自 148k 条规则的 `ValuedRule`（每条三个 String）、聚合表和每标签一个
`HashMap` 的域名树；已知优化方向：紧凑后缀树、避免逐条克隆标签/来源字符串、构建期分块释放。这只是 5D 预算输入，
本任务不设阈值、不做优化。未测整进程（原生尚不能加载全部 config_all）。
