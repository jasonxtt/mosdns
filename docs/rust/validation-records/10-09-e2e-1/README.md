# E2E-1：第 1 轮审查修复与完整重跑（2026-10-09）

**修复与验证完成，等待外部 Claude 第 2 轮审查，尚未验收。**
BASE/HEAD 为 `2b9d0beafc13589a408a09b5ac97856af1677312`，分支 rust；
Go 参考 `75f2667703c83952e86833852440e07c63f33311`（v0.7.3）；配置包
`0412666b9648a1f52d831e43d7817abd7a5ab556`。最终原生包含本次未提交补丁。
修改为 Rust 执行/缓存、回归测试、工具、契约与记录；Go 和原配置包未改。
没有提交、push 或部署，任务保持 in_progress。第 1 轮 FINAL: FAIL 保留在
[round-1.md](round-1.md)、[summary-round-1.json](summary-round-1.json) 和
[coverage-round-1.json](coverage-round-1.json)，不能作为当前结论。

## 环境与映射

所有构建与测试在 SSH 别名 mosdns-rust VM 完成；Cargo jobs=2、incremental=0，
两套 Vue UI 先于各自嵌入二进制编译。Go/native 串行运行，各有仅 loopback、
无公网路由的 netns、私有 /tmp mount、独立模拟 DNS/SOCKS5；Go GOMAXPROCS=1，
原生 current_thread。未修改安装路径、系统信任库或原服务。

原生为 config_rust + 从未改动生产快照迁移的 settings；Go 为 config_all + 同一
快照 webinfo 副本，再分别映射端点。规则、生成统计、SRS、AdGuard、开关相同；
启动 cache dump 清空，缓存开启。保留 Go API80 / 原生9099；rewrite 域名目标仍经
真实内部监听 127.0.0.1:2222，未绕过嵌套序列。

下表都是构造的模拟端点，同组竞速条目返回相同答案。

| 组 | UDP/TCP 端口 | DoT 端口 | DoH 端口 | A / AAAA 标识 |
| --- | --- | --- | --- | --- |
| domestic | 15000 | 15001 | 15002 | 223.5.5.101 / 2400:3200::101 |
| foreign | 15010 | 15011 | 15012 | 203.0.113.20 / 2001:db8:2::20 |
| foreignecs | 15020 | 15021 | 15022 | 203.0.113.30 / 2001:db8:3::30 |
| cnfake | 15030 | 15031 | 15032 | 30.0.0.10 / 2400::10 |
| nocnfake | 15040 | 15041 | 15042 | 28.0.0.10 / 2001:2::10 |

SOCKS5 127.0.0.1:17891 只允许模拟目标；HTTPS 使用 localhost + loopback dial_addr，
临时 CA/独立 leaf 通过进程 SSL_CERT_FILE 验证，没有 insecure_skip_verify。
实际生效协议为 UDP/HTTPS；未声称实测 TCP 客户端、DoT 或定时更新检查。
更新/下载即使触发也不能访问公网。[mapping.json](mapping.json) 仅含模拟组；
实际条目别名表私有保存。

## 语料与比较

语料与第 1 轮最终回放逐字节相同，SHA-256：
`a17ef5136f82adf08f701b5990c99354c9436be6e578ecf1c26e8a263dee2ab4`。
2000 个顺序抽取的唯一生产域名 × A/AAAA，加60条规则/客户端探针和18条辅助监听探针，
每遍4078条，每边冷热两遍 **8156条**。临时副本对称追加保留域规则/客户端/rewrite/
AdGuard 探针，原快照和开关未改。源127.0.0.1/127.0.0.2；UDP端口
53、2222、3066、3077、3099、3111、3333、4444、7777、8888。

比较 rcode、排序后的 owner/type/class/data、逐路径TTL、模拟答案组、domain_set、
effective_tag、final_sequence 和按启用条目映射的 **final_upstream_group**。
google-foreignecs 归 foreignecs；原始 final_upstream 只计诊断，不决定语义一致。
DNS大小写、审计IPv4-mapped归一；标签字符串顺序仍作为差异记录。

TTL 覆盖非OPT的 Answer/Authority/Additional：rewrite必须5，模拟上游≤300，
3099配置TTL≤5；热缓存按单调时间校验递减，容许一秒取整，仅日志证明本次重新转发
才允许重置。空本地应答无TTL、FakeSOA按冻结300校验、未知本地记录失败。
[ttl-policy.jsonl](ttl-policy.jsonl) 保留16312条逐路径结果，**违规0**。
热递减实查Go3841 / 原生3840条，违规均0。

两边预期审计各8132条，缺失0；另24条未启用审计。查询错误0、TLS mock错误0、
意外SOCKS目标0。覆盖黑名单、AdGuard、两类rewrite、DDNS、realip/fakeip、client_ip、
规则/订阅直连代理、当前开关、domain_output、ECS。ECS仅foreignecs两次且token相同；
生产si_set包含domestic模拟标识。Go SOCKS CONNECT4次 / 原生12次，均只去
foreign/foreignecs DoH，连接次数为资源诊断。见[coverage.json](coverage.json)。
没有覆盖压力、长稳、全部WebUI、在线下载成功、5D或Phase6。

## 修复与最终差异

P1-1：exit穿过cache successor按成功发布当时应答，lazy同规则；错误、取消、
截止/fuel耗尽、缺失/无效应答仍不发布。嵌套cache保存应答所属标签，避免内层完成
清掉父路由候选后外层缓存丢metadata。契约和entry/child、exit/try、fallback、lazy、
失败不发布、metadata恢复回归同步更新。

P1-2：修复P1-1后N1冷A/AAAA仍错误；四格最小复现证明fuel64在cnfake入口或forward前
耗尽，留下继承的domestic应答。用户 **2026-10-09** 明确批准：
“批准前台 256，lazy 保持 64（推荐）”。前台256/lazy64，共享计数、循环、取消、
截止保留。提高上限还暴露既有redirect cycle默认栈溢出，通过执行future内部的分段
轮询限制调用深度，补深链主/备顺序回归；没有扩大测试栈或另起后台任务。
最终A/AAAA × 冷/热四格均cnfake，原生wire、供应组、序列、标签一致；
冷TTL5、热TTL3，热遍不转发。Go热缓存不恢复序列/供应提示，差异独立保留。

重推热标签发现899条effective_tag缺少既有首个允许路由标签回退，已做小范围修复，
补无/具名/组提示、无V标志、未知标签回归，随后重跑全量检查与完整语料。

[summary.json](summary.json)：语义字段一致 **3681**，差异 **4475**。

| 逐条互斥分类（等待外部确认） | 条数 | 依据 |
| --- | ---: | --- |
| Go偶然行为 | 4471 | 同标签集合的顺序；Go缓存缺少序列/供应提示 |
| 已批准偏差 | 4 | 2026-10-07用户批准full:大小写不敏感，契约第57行 |
| 未解释差异 / 已知未修原生缺陷 | 0 | 每个不同字段均有独立理由 |

非互斥原因：标签顺序806、Go缺序列提示4285、Go缺供应提示3818、full:规则4。
字段差异：domain_set810、effective_tag8、final_sequence4286、final_upstream_group3819、
answers2、marker_group2。原始供应条目字符串差异3821仅诊断；其中两条别名已归同组。
[mismatch-ledger.jsonl](mismatch-ledger.jsonl) 的4475行按pass/index/字段记录理由，
不含生产域名。**仅剩两条DNS答案差异**为index3787冷/热AAAA：原生额外“记忆无V6”
空答、Go domestic AAAA；同名A两条只有标签差异。批准依据为
[native-rule-providers](../../contracts/native-rule-providers.md)第57行。
这些是执行者按证据归类的结果，不能替代外部FINAL: PASS。

撤回第1轮“缓存/热映射已批准偏差”和“Go热层遮蔽静态”的解释；pinned Go mapper
实际合并hot+static。最终domain_set只余806条顺序与4条full:差异；
effective_tag只余4条顺序与4条full:差异，没有继续豁免热行为。
domain_output探针 **Go=2 / 原生=2**。cache_all：Go3813hits/3813entries，
原生 **3812/3812**，少一个源于上述已批准AAAA提前空答；cache_cn Go42/3188、
原生42/3187，其余活跃具名cache一致。

## VM检查、资源与清理

最终fmt --check、workspace/all-targets clippy -D warnings、workspace全量测试、
原包config_rust check均exit0。**95 suites，1462 passed、0 failed、4个既有ignored**；
工具8测试、py_compile通过。check gaps=0/cascades=0/globals=0/definitions=128。
裸包缺失启用AdGuard本地文件的既有诊断保留，运行副本使用快照文件。
[test-gates.json](test-gates.json) 记录最终检查；前置失败日志私有保留。

| 运行时 | 启动 / 冷后 / 热后 RSS KiB | 两遍CPU增量s | 冷p50/p95/p99 ms | 热p50/p95/p99 ms |
| --- | --- | ---: | --- | --- |
| Go | 161124 / 175764 / 202796 | 2.10 | 0.282 / 0.432 / 0.645 | 0.112 / 0.194 / 0.280 |
| 原生 | 42356 / 53060 / 55984 | 2.27 | 0.297 / 0.421 / 0.529 | 0.094 / 0.186 / 0.302 |

CPU含审计HTTP开销，延迟只含DNS send/receive；没有affinity、阈值或性能验收结论。
最终native SHA-256 `da327acd9464f20537470874f2c33529e785fe360ac0bb501f5dd605945192c0`；
Go `02ad7202d2689c0cb58e4cd672cc8eff7cd5a0ae7e67b7479717ff9840a98e9a`。
VM受检五个Rust源文件与工作区hash一致。私有wire/audit/mock/cache/诊断日志归档到
仓库外，没有完整配置树或TLS私钥。2026-10-09 03:38 UTC已清理自有进程、netns、
VM临时目录；原服务active、MainPID827437。见[cleanup.json](cleanup.json)。
等待第2轮外部审查。

## 外部审查结论（2026-10-09）

第 1 轮（Claude，合并另一份独立审查）FAIL：缓存 exit 发布、N1、TTL 策略、上游组归一化。第 2 轮（Claude）**FINAL: PASS**：四项均已修复。
遗留 P2：深度 ≥16 的延迟分支轮询可能在等待 I/O 时空转（需加测试确认）；原生在 Go 空提示处填写 `final_sequence`/`final_upstream`
应记入审计契约。详见任务 research 中的 claude-review-round-1/2。
