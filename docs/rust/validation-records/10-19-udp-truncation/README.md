# UDP 应答预算与截断（Shadow-1 P1 修复）

基线 `dba69d1e`，分支 `rust`。实现与规定验证完成，**等待外部 Claude 审查**；
没有自我审查轮次、提交、推送、部署或创建 release。Shadow-1 保持暂停。

## 改动

在 UDP 执行/缓存发布之后应用原客户端的 `max(UDPSize,512)` 预算。
超限应答复用 Hickory 名字压缩，预留回应 OPT，按 Answer→Authority→Additional
保留完整 RR 前缀；首条放不下后停止，任一丢弃置 TC，保留已有 TC。TSIG 不重打包。
详细审计的 TC/答案列表取最终发送 buffer；缓存保持完整，TCP 路径不改。
尺寸已经合规的应答直接复用原 Vec，不额外解码/复制回应；客户端 OPT 仍有有界解析。

契约：[UDP response](../../contracts/native-udp-response.md)。

## 第2轮：仅修复外部审查F1

超限应答无法由Hickory解码/重编码，或Question+OPT无法放入预算时，发送最小TC应答，
保留原回应header、设置QD=1与AN/NS/AR=0，附原客户端已验证的问题。
名字指针展开后再写入，避免引用已丢弃的数据；详细审计重新投影最终TC/空答案。
正常截断、TSIG例外、缓存发布与TCP实现不变。

新增3项：合成CNAME解码拒绝、Question+OPT超预算、真实UDP回退/审计/完整缓存/TCP。
socket场景4次UDP与1次TCP仍只调用上游1次；小UDP为最小TC应答，
大预算UDP与TCP保留全部96条答案和原Additional。

仅在指定VM、离线2 jobs：fmt与严格workspace/all-targets clippy通过；
聚焦 **18/18**；全量 **1504通过、0失败、1既有忽略**，92个测试/文档测试目标。
native-host单元用例325→328（327通过、1忽略）；189个Rust源文件与本机hash一致。
较第1轮通过数量增加3，无新增忽略。本轮按F1要求重跑聚焦/全量检查，
未重跑下列第1轮已接受的Go oracle、release构建、config check与私有样本回放。

验证任务进程0；本轮VM临时目录已删除；证据在仓库外归档。
没有自我审查、提交或部署，Shadow-1保持暂停，等待第2轮外部审查。

## 第1轮 VM 检查（修复F1前）

仅在 SSH 别名 `mosdns-rust` 的独立临时目录，离线、2 jobs；未连接 mos/esxi，未改 rtdns。
Rust 1.95.0；Go oracle 使用缓存 Go1.26.4、miekg/dns v1.1.72（独立验证模块，非运行时依赖）。

| 检查 | 结果 |
| --- | --- |
| 旧代码真实 UDP 回归 RED | 无 EDNS 回答1571字节，断言超过512失败 |
| UDP 聚焦测试 | 15通过，包括10项新增与5项现有测试 |
| cargo fmt --all --check | 通过 |
| workspace/all-targets clippy -D warnings | 通过 |
| workspace --no-fail-fast 全量测试 | **1501通过、0失败、1忽略**，92个测试/文档测试目标 |
| native-host 单元测试 | 用例总数315→325；本轮324通过、1忽略 |
| Rust源码对应 | VM与工作树全部189个Rust源文件hash一致 |
| 标准native构建脚本 | 离线npm ci/build后顺序Cargo release，Rust release阶段约2m29s |
| 完整config_rust check | gaps=0、cascades=0、globals=0、definitions=128 |

唯一忽略项为 `switch_crash_child`，由通过的父测试 `crash_boundaries_are_recoverable`
启动。未新增忽略。没有在本机编译Rust或Go，VM生成的UI产物未回写本机工作树。
验证二进制sha256：`a18090cd20cd1dc6d2cf9fa4f98ff26969af533c671066e0d81107f33b5812a7`。

## Go 对照

11个合成向量的三节记录数/TC全部一致，9个逐字节一致。
覆盖512/1232/4096、刚好512、压缩后完整容纳、仅Additional丢弃、单RR超限、
OPT预算、已有TC及TSIG。真实socket测试另覆盖相同缓存跨客户端大小、TCP完整回答与最终审计，
所有5次查询仅调用上游1次。

两项字节差异有意保留：合规的已有压缩应答及TSIG保留原wire；Go oracle在重新Pack时
可能选择无压缩编码。记录/TC不变，普通回应仍在预算内；TSIG遵从不截断例外。

## 三条私有样本：新二进制真实 UDP 回放

仅在loopback-only网络命名空间中运行新程序，以保存的完整旧Rust回答作受控上游，
查询payload不变；本次验证UDP出口，不声称复测生产路由/原客户端身份。
三条均与保存的Go回答及独立Go oracle的长度/TC/三节计数完全一致，最终审计匹配。

| 样本序号 | 修复前Rust字节/TC | 修复后字节/TC | Answer/Authority/Additional |
| --- | --- | --- | --- |
| 1 | 1972 / false | 507 / true | 4 / 0 / 17 |
| 2 | 2705 / false | 504 / true | 8 / 0 / 13 |
| 3 | 967 / false | 503 / true | 13 / 0 / 3 |

上游调用3、mock错误0；验证进程自然结束后namespace内进程0，无需强杀，命名空间已删除。
所有原始样本、域名、地址与审计只在仓库外私有证据，不进入公开记录。

## 清理与边界

任务VM进程0，namespace已删除，任务临时目录已删除；私有证据与验证二进制已归档到本机
仓库外0700目录、归档文件0600。没有动Shadow-1的VM目录、脚本或rtdns安装。
未开始1小时smoke/48小时长测，不作生产性能或长期稳定性结论。

## Planner verdict (2026-10-19)
Round 1: one P1. An oversized response that could not be decoded was dropped instead of being sent. Round 2 fixed it
with a minimal TC reply, so the client retries over TCP and the full cached answer is served. `FINAL: PASS`.
Found by the Shadow-1 real-traffic replay (3 real no-EDNS queries); the shadow run resumes on a rebuilt binary.
