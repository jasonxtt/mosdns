# Rust-native client context and ECS cache isolation

## Goal

一个PRD完成真实客户端身份、client_ip条件、ECS策略与ECS感知缓存/dump/刷新组合，使不同客户端子网的响应不会错用。只规划；用户已批准创建任务和与C2C讨论，不授权实施。

## Baseline

- 前置应答策略/IP任务已归档，代码13d60d3e，归档aa32270a；当前native已有startup文本IP快照与resp_ip、hosts/redirect/ttl，缓存/branch/QueryView均有真实证明。
- UDP/TCP listener已知peer，observer使用它，但ExecutionRequest没有client identity字段；matcher不能借审计开关获取执行数据。
- Go handler.go的forward/send/preset/mask4/mask6与旧quick ecs为参考；QOpt/ClientOpt/UpstreamOpt/RespOpt分离，native目前raw forwarding不能直接套addECS的已有ECS早退逻辑。
- 当前cache拒绝enable_ecs=true，查询含ECS绕过，dump拒绝ECS suffix。延展必须明确opt-in，不把旧safe default改成跨子网共享。

## Requirements

- R1 在执行入口传入可信UDP/TCP peer IP；unknown显式None。client_ip复用已有IP literal/CIDR/$ip_set/&text列表OR与否定语法。ECS/preset不改变真实peer，忽略代理头；mapped IPv4统一Unmap。metadata随redirect/fallback/prefer/后台refresh副本传递，无审计capture依赖。
- R2 named ecs_handler实现forward/send/preset/mask4/mask6，布尔缺省false、preset缺省空、mask缺省/0为24/48，范围4:0..32/6:0..128；legacy quick ecs保留首IP、忽略首mask/其余参数并告警。无新增语法。
- R3 明确incoming client ECS、current outbound ECS和final response ECS的所有权；handler自己的successor使用修改后的QueryView，共用有限root/deadline。current既有policy ECS不被后续handler覆盖；否则forward有效incoming优先，再preset，再send有效peer，最后none。直接forward无handler维持旧nativewire。嵌套/错误/取消不能污染父/sibling。
- R4 回传仅适用转发incoming ECS且客户端有OPT：从真实供应response复制合法匹配ECS；生成preset/send不回显，缓存hit没有upstream OPT不得伪造scope。保留DO/ID/question/允许其它options；不得改变真实supplier/client审计schema。
- R5 named cache enable_ecs=true显式开放，quick仍false；key为已有产品base+一字节ECS字符串长度+Go option.String格式的规范化网络suffix，读取该cache dispatch的current QueryView；无ECS不加suffix。false仍bypass带ECS。不同family/prefix/network/AD/CD/DO独立；不按响应scope扩大复用。
- R6 原mosdns_cache_v2/schema保持；true接受支持IPv4/6合法ECS suffix且完整校验后merge；false遇ECS条目完整拒绝。未知family/坏mask/address/重复ECS/非零query scope绕过cache，不静默去掉suffix；manual坏dump400无部分导入。保留原限额/time/domain_set/原子flush/generation。
- R7 刷新捕获dispatch时client与ECS视图，singleflight按完整key；同64fuel/5s/256/noqueue，flush/import拦旧publish；客户端断开仍由owner管理，不新增client audit/metrics。

## Frozen safety and compatibility additions

- R8 Configuration assembly rejects any named/quick cache whose miss successor to its enclosing publication boundary may transform ECS or branch on client_ip. This applies to enable_ecs=true/false. Conservative summaries cover call/jump/goto/try, fallback, preference and inherited continuation; unresolved reachability is unsafe. Diagnostics identify cache callsite and policy. Place policy before cache and use separate owners for different client routing branches; no automatic reordering.
- R9 ECS suffix uses Go String format with canonical masked network identity. Dump compatibility means format and canonical network semantics, not byte-identical Go prepack host-bit keys or guaranteed cache hits for those keys. Import masks valid legacy host-bit addresses; normalized collisions use last entry in dump order. Export canonical suffix. Noncanonical Go-generated keys may require refill; this is an intentional deviation.
- R10 Invalid/duplicate/mismatched supplier ECS is stripped while an otherwise valid DNS response remains successful. Invalid DNS follows existing response validation. Same-key refresh followers cannot overwrite the first admitted refresh's immutable peer/ECS/state snapshot.

## Acceptance

- A1 真实UDP/TCP客户端（包括IPv6及mapped）命中client_ip；伪造ECS不能改peer；unknown embedder行为；audit-off相同。
- A2 handler配置矩阵、legacy语法、forward/preset/send/none优先级、mask边界、current/client独立；controlledpeer读取实际ECS与masked wire。
- A3 caller有/无OPT、forwarding/generated、合法/坏response ECS、cachehit/forward/fallback/redirect/prefer、error/exit/cancel的回传与supplier正确。
- A4 同域同type不同prefix不同answers，peer count证明隔离与同keyhit；false保留bypass；generation与owner刷新终态正确。
- A5 实际Go/native ECS v2 dump双向格式与 canonical 查询命中 fixtures、旧无ECSfixture回归、坏末条目全量拒绝、restart/flush/time/limits覆盖；非 canonical Go prepack key 允许 refill。
- A6 mosdns-rust隔离loopback真实DNS/API/Vue最终答案/来源证明、必要Rust/UI回归；保留所有失败证据，不push/部署。

- A7 Unsafe cache-before-handler/client_ip configurations fail compilation across named/quick, true/false and every supported control-flow form. Safe policy-before-cache/separate-owner configurations pass.
- A8 Actual Go/native dump fixtures prove canonical query matching and parsing; explicitly demonstrate that noncanonical Go prepack keys can require refill. A5 is subject to R9, not a byte-identity promise.
- A9 Same-key followers preserve the first refresh snapshot; supplier ECS errors alone do not discard otherwise valid answers.

## Out of scope

DoH/PROXY/XFF listener身份、multi-listener、新ECS管理UI/审计schema、client subnet排名、SRS/二进制IP集/热更新、RFC scope网络范围cache、完整EDNS option生态、性能/生产切换/hybrid退休。

## Planning status

PLAN READY: C2C iteration 1 closed all five findings. See research/planning-review.md for disposition. Planning approval is not code PASS. The task remains planning; no implementation is started here. Sending executor-prompt.md to an executor authorizes the described implementation and C2C review loop.
