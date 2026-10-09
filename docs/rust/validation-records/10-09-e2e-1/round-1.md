# E2E-1：首次生产形态整链与 Go 差分（2026-10-09）

**历史记录：第 1 轮外部审查为 FAIL。** 下文保留原始数值与当时归类，
缓存／热映射的“已批准偏差”分类已被否决，不能再引用作批准依据。
用户于 2026-10-09 批准成功 `exit` 发布、N1 修复及完整重跑；最新结论见 README.md。

**已执行，未通过最终验收；等待外部 Claude 审查和待决项处理。**
原生基线 `2b9d0beafc13589a408a09b5ac97856af1677312`；Go 参考
`75f2667703c83952e86833852440e07c63f33311`（v0.7.3）；配置包仓库
`0412666b9648a1f52d831e43d7817abd7a5ab556`。
没有改产品代码、Go 代码或原配置包，没有提交、push、部署。

## 环境与映射

所有 UI/Go/Rust 构建在 SSH 别名 `mosdns-rust` VM 完成。两套 UI 先于各自
二进制编译；Rust release jobs=2、incremental=0，Go GOMAXPROCS=2 构建。
Go 和原生分别在只有 loopback、无外网路由的 netns 中运行；每套都有独立
/tmp mount、模拟 DNS 与 SOCKS5，未使用 VM 原有服务端口、安装路径或信任库。
回放串行，Go 运行 GOMAXPROCS=1，原生保持 current_thread。

原生使用 `config_rust` + 从未改动生产快照迁移的 settings，再映射端点；
Go 使用 `config_all` + 同一快照 webinfo 的副本，再映射端点。
规则、生成统计、SRS、AdGuard、开关状态相同；启动缓存 dump 清空，缓存保持开启。
保留实际 API 端口差异：Go webinfo 覆盖为 80，原生 YAML 为 9099。
rewrite 的域名目标仍经真实内部监听 `127.0.0.1:2222`，未绕过嵌套序列。

下表全部是**构造的模拟地址**，不含生产目标地址。同组竞速条目返回相同答案。

| 组 | UDP/TCP loopback 端口 | DoT 端口 | DoH 端口 | A / AAAA 标识答案 |
| --- | --- | --- | --- | --- |
| domestic | 15000 | 15001 | 15002 | 223.5.5.101 / 2400:3200::101 |
| foreign | 15010 | 15011 | 15012 | 203.0.113.20 / 2001:db8:2::20 |
| foreignecs | 15020 | 15021 | 15022 | 203.0.113.30 / 2001:db8:3::30 |
| cnfake | 15030 | 15031 | 15032 | 30.0.0.10 / 2400::10 |
| nocnfake | 15040 | 15041 | 15042 | 28.0.0.10 / 2001:2::10 |

SOCKS5 `127.0.0.1:17891` 仅允许上述 TCP 类模拟目标；HTTPS 地址为 localhost，
numeric dial_addr 为 loopback。临时 CA 与单独 server leaf，通过进程级
SSL_CERT_FILE 正常验证 TLS；未设置 insecure_skip_verify。
生产实际协议为 UDP/HTTPS，TCP/DoT 模拟支持未被本次生效列表使用。
Go 自带更新检查即使触发也无法到达公网；本次短回放的 warn 日志没有更新失败记录，
**没有声称实测其定时触发**。规则下载同样被 netns 隔离。

## 语料、覆盖与比对

生产 top_domains 按文件顺序抽取 2000 个唯一合法域名，每个 A/AAAA；另外 60 条
规则/客户端探针与 18 条辅助监听探针。每遍 4078 条，每边冷、热两遍共 **8156** 条。
两边顺序、源 IP、问题、目标端口相同；源 UDP 临时端口不作为客户身份或比较字段。
辅助 UDP 端口：2222、3066、3077、3099、3111、3333、4444、7777、8888。
在副本中追加 11 个明确 local/generated 规则、client_ip 的 127.0.0.2/32、两条
rewrite 与一条 AdGuard 探针；example.com 保留域探针不进入原包或生产快照。
原开关状态没有改动；未宣称覆盖每个开关的另一状态。

比较 rcode、排序后的 owner/type/class/data、TTL 合理范围、答案模拟组，以及
domain_set/effective_tag/final_sequence/final_upstream。DNS 名大小写归一，
审计的 IPv4-mapped 表示归一；比较不抹掉热缓存的审计差异。
配置启用审计的端口逐条关联，**每边 8132 条预期审计，缺失 0**；剩余 24 条是
配置未启用审计的辅助请求。无查询错误、无崩溃，模拟服务日志最终为 0 字节。

实际覆盖：黑名单、AdGuard、rewrite IP/域名、DDNS、realip/fakeip、client_ip、
本地/生成规则、sd_set_light 的订阅直连/代理、当前开关分支、domain_output、ECS。
独立 SRS 解码确认 domestic 标识在生产 si_set 中；两边 ECS 出站 token 一致，
仅 foreignecs 发送 ECS。SOCKS5 仅连接 foreign/foreignecs 的 DoH 端口，意外目标 0。
Go CONNECT 3 次，原生 15 次（复用/竞速/缓存不同），连接次数不作为契约相等条件。
细项见 [coverage.json](coverage.json)。本次只回放 UDP；未覆盖 TCP 客户端、故障压力、
长期运行、在线规则成功下载、全部 WebUI 工作流或 5D/Phase 6。

## 结果与归类

完整汇总见 [summary.json](summary.json)。原始字段完全一致 **3679** 条，差异 **4477** 条：

| 主分类（逐条互斥） | 条数 | 原因 |
| --- | ---: | --- |
| 已批准偏差 | 4050 | full: 大小写不敏感；热映射层；现有缓存自然完成/exit 作用域契约 |
| Go 偶然行为 | 424 | 标签集合顺序；Go 空路由提示与原生实际序列/供应条目信息 |
| 原生缺陷（待决 N1） | 3 | 3099 入口应返回国内 FakeIP，却返回 domestic 真实地址 |
| 最终轮次测试环境差异 | 0 | 工具/模拟环境问题已在前置轮次修正 |

原因可重叠：标签顺序 801、Go 缺序列提示 237、full: 大小写 4、Go 缺供应提示 4、
热映射/缓存放置 1449、exit 不发布缓存 4045、N1 3。
4477 条脱敏的 pass/index/字段/分类明细位于任务 research/mismatch-ledger.jsonl；
原始域名、规则与 wire/audit 明细保留在仓库外私有归档，未写入此目录。

### DNS 差异：5 条，不能称为 DNS 全一致

- pass1/index3787 与 pass2/index3787：同一生产 AAAA 查询。原生还命中不同大小写
  full: 的“记忆无V6”，Go 未命中，原生空答、Go domestic AAAA。
  原快照 nov6rule 与 realiprule 都有 case-normal 相等规则，nov6rule 非原始大小写精确相等。
  按 [native-rule-providers](../../contracts/native-rule-providers.md) 已批准偏差归类。
- **N1：pass1/index4066（A）、pass1/index4067（AAAA）、pass2/index4067（AAAA）**：
  3099（clashmi）入口，Go cnfake，原生 domestic。冷 A 原生最终序列仍为 sequence_local；
  两条 AAAA 的审计甚至指向 cnfake，但实际供应者/答案仍是 domestic。
  热 A 已变成 cnfake。该问题影响真实配置入口的 DNS 与审计，归类原生缺陷，不能豁免。
  与 try/fallback、继承响应、cache successor/exit 有关；**确切根因尚未证明**，不能称已修复。
  其修复边界可能跨执行续链与冻结作用域合同，按 R5 交用户/外部审查决定，未改产品代码。

### 已冻结契约在生产配置上的实际影响（另一个待决项）

Go cache_all：7626 查询、3813 hit、3813 entries；原生：7624 查询、**0 hit、0 entries**。
原生的 exit-unwound watch 不发布，符合
[native-cache-lifecycle](../../contracts/native-cache-lifecycle.md) 的自然完成合同，但使这份
大量使用 exit 的配置不能获得 Go 的主缓存效果。这不是 M1 生产适用性通过的证据；
建议用户决定是否重评缓存成功完成语义，而不是直接删除防错误/取消发布的门禁。
域名生成探针计数 Go=2、原生=4（热遍重走序列），同样反映缓存差异。

## 资源诊断（不是性能验收）

| 运行时 | 启动 / 冷遍后 / 热遍后 RSS KiB | 两遍 CPU 增量 s | 冷 p50/p95/p99 ms | 热 p50/p95/p99 ms |
| --- | --- | ---: | --- | --- |
| Go | 154992 / 140648 / 146564 | 1.60 | 0.236 / 0.353 / 0.454 | 0.067 / 0.145 / 0.246 |
| 原生 | 42352 / 47764 / 53556 | 2.32 | 0.262 / 0.399 / 0.532 | 0.211 / 0.353 / 0.440 |

CPU 包含审计 HTTP 查询开销，延迟只括 DNS send/receive；未设 affinity 或性能阈值。
原生热遍主缓存不命中，所以两边并非相同缓存工作量，不能据此下语言性能结论。
VM 初始可用磁盘约 41 GB，任务文件总量约 1468 MiB，全部临时资源清理后释放。

## 验证与清理

VM 构建双方 release 成功；最终工具 unittest 3/3、py_compile 成功。
原包与映射后的真实快照配置均 `SUMMARY gaps=0 cascades=0 globals=0 definitions=128`。
未改产品代码，故没有重跑 workspace fmt/clippy/test，也没有声称这些 gate 在本任务跑过。
run1 是 mapped-client 审计关联/监听就绪失败；run2 的 .test 被屏蔽且 DoH mock 有误；
run3 修正 HTTPS/探针后用于定位；run4 是最终完整采样，前置失败不被覆盖。
任务拥有的进程、netns、VM 工作目录已删除；原服务 MainPID=827437、active 保持不变。
外部审查尚未进行；任务保留 in_progress 等待用户转交结论。
