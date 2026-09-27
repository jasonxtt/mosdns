# 完整 Rust MosDNS 功能覆盖总表

规划盘点日期：2026-09-20。源码基线：`4fe08c0eebed45efd304f9b197ce1d6e5d2608d5`；盘点时存在无关工作树改动，本次不将它们计为迁移成果。

本表是现有功能的迁移归属与验收台账，不是 Rust 功能完成声明。架构和阶段见 [重写方案](../ai/rust-rewrite-plan.md)，测量方法见 [性能验收方案](performance-validation.md)。后续 main 同步必须复查增删条目并记录参考 commit。

## 1. 状态与维护规则

- **基础证据**：已有 core、ABI、局部契约或模块测试；只证明其对应范围。
- **原生接入**：同一 Rust 主程序可由既有 YAML/API 配置驱动真实功能，不依赖 Go。
- **产品验收**：正确性、组合行为、失败/恢复、持久化和适用性能证据齐全。
- 当前所有条目的“原生接入/产品验收”均为 **待验收**；不能根据 crate 名、测试数量或历史 task archive 自动改为完成。
- 下表“归属”是路线图阶段，不是已创建/批准任务。对应实现任务规划时补上任务路径、参数/默认值/错误行为、预定原生实现位置和验证计划；实现后补齐实际路径、测试与报告链接。缺少映射不得进入该条目的完成验收。
- 每条参数、别名、quick setup、预置入口和插件自有 API 都属于盘点对象。注册包覆盖完成不等于参数/endpoint 级契约已全部冻结；后者是对应阶段任务的前置门槛。
- 未支持功能在 5A 必须明确报错，不能静默忽略。5B/5C 交付前不得以“不常用”删除条目；排除需要用户明确决定。

2026-09-27 执行方式调整：71 项是完整性台账，不是 71 个独立任务。按真实配置链和共享机制成批推进；先明确本批实际需要的语法/参数，未用条目在其拥有任务补全，不在每批重做全表契约冻结。交付记录新增可运行配置与参数子项、未完成部分和稳定测试入口；基础证据/局部接入不自动升级为整项产品验收。

### 5B 代表链实现子项（本地通过；指定 Linux E2E 子项通过，完整验收待审）

任务 [09-27-rust-phase5b-config-sequence-composition](../../.trellis/tasks/09-27-rust-phase5b-config-sequence-composition/prd.md) 把原生配置从固定 W1/W2/W3 图改为收集后解析的通用编译器，并让一条从本地配置包裁剪的代表链在 native host 上运行。以下只登记本批实际交付的子项与明确延期；整行仍是“待验收”。

| 条目 | 本批实际交付 | 仍延期 |
| --- | --- | --- |
| P02 `domain_set` | `exps`（full/domain/regexp/keyword，复用 matcher-core MixMatcher）与 `files`（UTF-8 文本、跳过空行/`#` 注释、相对声明文件目录解析）；缺失规则文件在 bind 前报路径错误 | 管理端下载/修改/删除/热更新、SRS/geodata、`sets` 引用、provider API |
| P15 `qclass` | 未纳入（本链不需要） | 整项 |
| P16 `qname` | `qname $domain_set` 命名引用与内联规则；未知名在加载期报错 | `&file` 形式、`sets` 组合、其它 matcher 类别 |
| P17 `qtype` | 多类型号匹配，非数字在加载期报错 | 整项参数冻结 |
| P12 `has_resp` | 已接入（无参） | 整项参数冻结 |
| P20 `resp_ip` | 仅单一 IPv4 字面量，保留旧 W3 语法 | CIDR/多值/IPv6、规则管理 |
| P26 `cache` | 单个 host-owned cache；`size` 为可配置正整数，`lazy_cache_ttl` 必须为 0；可在 entry 或子 sequence 包裹其后继，hit 结束后继链后父继续，miss 在 cache 所在后继自然完成点保存；重复动态访问受控失败 | 多实例、lazy/dump/exclude/ECS、`mosdns_cache_v2` 持久化、API/metrics |
| P34 `forward` | 多实例，各一 numeric UDP/TCP upstream，兼容旧 tag 形式；单/多 forward 统一合法响应校验与终态策略；effective upstream identity 在默认 tag 与显式 tag 间保持唯一 | 上游组、bootstrap、TLS/HTTPS/H3/DoQ、并发选择、超时重试/fallback |
| P44 `sequence` | 任意数量具名 sequence；direct `$sequence` 为独立 child scope（自然结束/accept/reject 返回父，exit 越过父遇 try 才捕获）；`jump`/`goto`/`return`/`try`/`accept`/`reject`/`exit`；exec scalar 与 list（multi-exec 为 synthetic inline scope） | 完整插件/quick setup、flow_setter、fallback/sleep、动态注册 |
| L01 | `accept`/`reject`/`return`/`goto`/`jump`/`exit`/`try` 经 YAML → 异步请求 → 最终响应集成；`reject` 默认 REFUSED，`0..=15` 实际渲染，`>15` 在加载期报 unsupported（不截断） | 完整 12-bit reject/EDNS 渲染（顺序：5B 后续），以及序列内其它插件解析 |
| L02 | `_true`/`_false`、单次 `!` 取反、`$tag` 命名引用、`matches` 单值或列表 | 完整 quick setup 集合、`&file` 引用 |
| C01 | 顶层 `include` 按声明文件目录解析、有序加载 plugins-only 子文件、嵌套 include 明确拒绝；每个 included definition 保留其 YAML source path/base directory（含相对规则文件解析和错误定位）；定义先收集再解析（重排/前后引用不改变有效分支）；重复 key/tag、缺失或跨类型引用、未知字段、未支持参数、坏规则文件均在 bind/查询前失败并带文件与字段/规则路径 | 递归 include、热更新、完整配置包加载、`api`/其它顶层段、preset 插件 |

本地证据入口：`rust/native-host/tests/slice3_composition.rs`（loader/include/规则路径/负例/重排和真实 listener 场景）与 `rust/native-host/src/execution.rs` 的 `the_representative_chain_blocks_rejects_and_routes_without_a_listener`（block/qtype-0/local miss/default 分支/child 后父继续/cache 后继不被父改写/entry cache/重复访问/取消）。首轮完整 workspace 回归为 891 passed；复审修复后的全量回归为 896 passed / 0 failed。

2026-09-28 在 `mosdns-rust`（Linux x86_64）用临时 Rust workspace 构建并运行代表链集成测试：UDP listener / audit on 与 TCP listener / audit off 均通过；另用 `mosdns start -c` 运行 TCP/audit-off CLI 代表链，并用既有 `dnsperf` 工具做 50 请求短诊断。请求、peer 计数、审计、构建/配置哈希、失败尝试、自有 PID 与端口释放证据见该任务 `implement.md` 的 A6 记录。短诊断经过 SSH 转发，结果不用于性能验收，没有性能 PASS。未在远端运行完整 workspace、故障/取消/关闭变体、`local.only.test` exact 分支或旧 W1/W2/W3 suites。完整 A1–A6 独立审查仍待执行，因此本表仍是“待验收”。

## 2. 启用插件注册包（71 项）

来源：[plugin/enabled_plugins.go](../../plugin/enabled_plugins.go)，名称取各包的注册常量/quick setup，而非猜测目录名。每行均需通过所属类别的验收契约（第 4 节），状态为上一节定义的“待验收”。5A 只接入其最小链路必需子集，最终完整归属如下。

| ID | 配置/调用名称 | Go 来源 | 已有 Rust 基础证据范围 | 完整归属 |
| --- | --- | --- | --- | --- |
| P01 | `domain_mapper` | [plugin/data_provider/domain_mapper](../../plugin/data_provider/domain_mapper/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P02 | `domain_set` | [plugin/data_provider/domain_set](../../plugin/data_provider/domain_set/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P03 | `domain_set_light` | [plugin/data_provider/domain_set_light](../../plugin/data_provider/domain_set_light/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P04 | `ip_set` | [plugin/data_provider/ip_set](../../plugin/data_provider/ip_set/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P05 | `sd_set` | [plugin/data_provider/sd_set](../../plugin/data_provider/sd_set/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P06 | `sd_set_light` | [plugin/data_provider/sd_set_light](../../plugin/data_provider/sd_set_light/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P07 | `si_set` | [plugin/data_provider/si_set](../../plugin/data_provider/si_set/) | matcher-core/Phase 2；不包含完整管理生命周期 | 5B 查询 + 5C 管理 |
| P08 | `client_ip` | [plugin/matcher/client_ip](../../plugin/matcher/client_ip/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P09 | `cname` | [plugin/matcher/cname](../../plugin/matcher/cname/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P10 | `env` | [plugin/matcher/env](../../plugin/matcher/env/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P11 | `fast_mark` | [plugin/matcher/fast_mark](../../plugin/matcher/fast_mark/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P12 | `has_resp` | [plugin/matcher/has_resp](../../plugin/matcher/has_resp/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P13 | `has_wanted_ans` | [plugin/matcher/has_wanted_ans](../../plugin/matcher/has_wanted_ans/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P14 | `ptr_ip` | [plugin/matcher/ptr_ip](../../plugin/matcher/ptr_ip/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P15 | `qclass` | [plugin/matcher/qclass](../../plugin/matcher/qclass/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P16 | `qname` | [plugin/matcher/qname](../../plugin/matcher/qname/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P17 | `qtype` | [plugin/matcher/qtype](../../plugin/matcher/qtype/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P18 | `random` | [plugin/matcher/random](../../plugin/matcher/random/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P19 | `rcode` | [plugin/matcher/rcode](../../plugin/matcher/rcode/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P20 | `resp_ip` | [plugin/matcher/resp_ip](../../plugin/matcher/resp_ip/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P21 | `string_exp` | [plugin/matcher/string_exp](../../plugin/matcher/string_exp/) | 需将具体 matcher 接到 sequence；通用状态/索引不等于该插件完成 | 5B |
| P22 | `adguard_rule` | [plugin/executable/adguard](../../plugin/executable/adguard/) | 无逐插件 native 验收证据 | 5B |
| P23 | `aliapi` | [plugin/executable/aliapi](../../plugin/executable/aliapi/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P24 | `arbitrary` | [plugin/executable/arbitrary](../../plugin/executable/arbitrary/) | 无逐插件 native 验收证据 | 5B |
| P25 | `black_hole` | [plugin/executable/black_hole](../../plugin/executable/black_hole/) | 无逐插件 native 验收证据 | 5B |
| P26 | `cache` | [plugin/executable/cache](../../plugin/executable/cache/) | cache-core/Phase 1；5A bounded W2 native-host/cache adapter；dump/ABI 局部契约 | 5A 子集 -> 5B；5C API/持久化 |
| P27 | `cname_remover` | [plugin/executable/cname_remover](../../plugin/executable/cname_remover/) | 无逐插件 native 验收证据 | 5B |
| P28 | `debug_print` | [plugin/executable/debug_print](../../plugin/executable/debug_print/) | 无逐插件 native 验收证据 | 5B |
| P29 | `domain_output` | [plugin/executable/domain_output](../../plugin/executable/domain_output/) | 无逐插件 native 验收证据 | 5B |
| P30 | `drop_resp` | [plugin/executable/drop_resp](../../plugin/executable/drop_resp/) | 无逐插件 native 验收证据 | 5B |
| P31 | `prefer_ipv4`, `prefer_ipv6` | [plugin/executable/dual_selector](../../plugin/executable/dual_selector/) | 无逐插件 native 验收证据 | 5B |
| P32 | `ecs_handler` | [plugin/executable/ecs_handler](../../plugin/executable/ecs_handler/) | 无逐插件 native 验收证据 | 5B |
| P33 | `flow_setter` | [plugin/executable/flow_setter](../../plugin/executable/flow_setter/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P34 | `forward` | [plugin/executable/forward](../../plugin/executable/forward/) | upstream-core/Phase 4，非完整 forward 插件 | 5A 子集 -> Phase 4/5B 完整 |
| P35 | `forward_edns0opt` | [plugin/executable/forward_edns0opt](../../plugin/executable/forward_edns0opt/) | 无逐插件 native 验收证据 | 5B |
| P36 | `hosts` | [plugin/executable/hosts](../../plugin/executable/hosts/) | 无逐插件 native 验收证据 | 5B |
| P37 | `metrics_collector` | [plugin/executable/metrics_collector](../../plugin/executable/metrics_collector/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P38 | `query_summary` | [plugin/executable/query_summary](../../plugin/executable/query_summary/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P39 | `rate_limiter` | [plugin/executable/rate_limiter](../../plugin/executable/rate_limiter/) | 无逐插件 native 验收证据 | 5B |
| P40 | `redirect` | [plugin/executable/redirect](../../plugin/executable/redirect/) | 无逐插件 native 验收证据 | 5B |
| P41 | `requery` | [plugin/executable/requery](../../plugin/executable/requery/) | 无逐插件 native 验收证据 | 5B |
| P42 | `reverse_lookup` | [plugin/executable/reverse_lookup](../../plugin/executable/reverse_lookup/) | 无逐插件 native 验收证据 | 5B |
| P43 | `rewrite` | [plugin/executable/rewrite](../../plugin/executable/rewrite/) | 无逐插件 native 验收证据 | 5B |
| P44 | `sequence` | [plugin/executable/sequence](../../plugin/executable/sequence/) | sequence-core/Phase 3B，同步控制流 foundation | 5A 异步组合 -> 5B 完整 |
| P45 | `fallback` | [plugin/executable/sequence/fallback](../../plugin/executable/sequence/fallback/) | 无逐插件 native 验收证据 | 5B |
| P46 | `sleep` | [plugin/executable/sleep](../../plugin/executable/sleep/) | 无逐插件 native 验收证据 | 5B |
| P47 | `switch1` | [plugin/executable/switcher1](../../plugin/executable/switcher1/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P48 | `switch10` | [plugin/executable/switcher10](../../plugin/executable/switcher10/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P49 | `switch11` | [plugin/executable/switcher11](../../plugin/executable/switcher11/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P50 | `switch12` | [plugin/executable/switcher12](../../plugin/executable/switcher12/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P51 | `switch13` | [plugin/executable/switcher13](../../plugin/executable/switcher13/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P52 | `switch14` | [plugin/executable/switcher14](../../plugin/executable/switcher14/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P53 | `switch15` | [plugin/executable/switcher15](../../plugin/executable/switcher15/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P54 | `switch16` | [plugin/executable/switcher16](../../plugin/executable/switcher16/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P55 | `switch17` | [plugin/executable/switcher17](../../plugin/executable/switcher17/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P56 | `switch2` | [plugin/executable/switcher2](../../plugin/executable/switcher2/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P57 | `switch3` | [plugin/executable/switcher3](../../plugin/executable/switcher3/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P58 | `switch4` | [plugin/executable/switcher4](../../plugin/executable/switcher4/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P59 | `switch5` | [plugin/executable/switcher5](../../plugin/executable/switcher5/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P60 | `switch6` | [plugin/executable/switcher6](../../plugin/executable/switcher6/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P61 | `switch7` | [plugin/executable/switcher7](../../plugin/executable/switcher7/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P62 | `switch8` | [plugin/executable/switcher8](../../plugin/executable/switcher8/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P63 | `switch9` | [plugin/executable/switcher9](../../plugin/executable/switcher9/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P64 | `tag_setter` | [plugin/executable/tag_setter](../../plugin/executable/tag_setter/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P65 | `ttl` | [plugin/executable/ttl](../../plugin/executable/ttl/) | 无逐插件 native 验收证据 | 5B |
| P66 | `webinfo` | [plugin/executable/webinfo](../../plugin/executable/webinfo/) | 无逐插件 native 验收证据 | 5B 查询 + 5C API/状态/观测 |
| P67 | `mark` | [plugin/mark](../../plugin/mark/) | sequence-core marks 状态；配置入口仍需接入 | 5B |
| P68 | `http_server` | [plugin/server/http_server](../../plugin/server/http_server/) | 无原生 listener 主程序集成证据 | Phase 4/5B |
| P69 | `quic_server` | [plugin/server/quic_server](../../plugin/server/quic_server/) | 无原生 listener 主程序集成证据 | Phase 4/5B |
| P70 | `tcp_server` | [plugin/server/tcp_server](../../plugin/server/tcp_server/) | 无原生 listener 主程序集成证据 | Phase 4/5B（UDP/TCP 最小接入归 5A） |
| P71 | `udp_server` | [plugin/server/udp_server](../../plugin/server/udp_server/) | 无原生 listener 主程序集成证据 | Phase 4/5B（UDP/TCP 最小接入归 5A） |

## 3. 注册表之外的语法、别名与预置入口

| ID | 入口 | Go 来源 | 归属与验收 |
| --- | --- | --- | --- |
| L01 | `accept`、`reject`、`return`、`goto`、`jump`、`exit`、`try` | [sequence 注册](../../plugin/executable/sequence/sequence.go)、[内置执行](../../plugin/executable/sequence/built_in.go) | 5A/5B；已有 3B 控制流基础，仍需 YAML -> 异步请求 -> 最终响应的集成 |
| L02 | `_true`、`_false`、匹配取反、命名引用、quick setup | [sequence 配置](../../plugin/executable/sequence/config.go)、[注册](../../plugin/executable/sequence/quick_setup.go) | 5A/5B；引用解析、顺序、参数和错误契约；不得仅测试程序化 fixture |
| L03 | `ecs` 旧入口 | [ecs_handler](../../plugin/executable/ecs_handler/handler.go) | 5B；与 `ecs_handler` 分别固定参数及行为 |
| L04 | `_remove_cname` 预置插件 | [cname_remover](../../plugin/executable/cname_remover/cname_remover.go) | 5B；预置加载和命名调用 |
| L05 | `_sleep_500ms` 预置插件 | [sleep](../../plugin/executable/sleep/sleep.go) | 5B；预置调用、等待、期限及取消 |

`prefer_ipv4`/`prefer_ipv6` 已在 P31 纳入；它们与上游 endpoint resolver 的双栈地址选择是不同产品能力，不能相互替代验收。

## 4. 插件类别验收契约

| 类别 | 每项至少覆盖 |
| --- | --- |
| provider（P01–P07） | 参数/文件/格式/过滤/优先级、加载和热替换、关联消费者；管理端下载/修改/删除/自动更新、失败保留、重启恢复和插件 API。具体支持格式以各包为准，不能假定所有 provider 都支持所有格式。 |
| matcher（P08–P21） | 正/负/边界输入，取反和组合顺序，查询/响应字段来源，状态变更与最终标签；随机或环境相关行为用受控输入，不要求无意义的逐次随机值相同。 |
| executable（P22–P66） | 配置/quick setup、响应修改、后续 sequence 是否继续、错误/取消、状态和审计副作用；按插件覆盖 fallback/并发、缓存、外部请求、文件/API 和重启。 |
| mark（P67） | matcher 与 executor 两种入口，数值范围/多 mark，query 生命周期和其它 fast mark/switch 位的关系。 |
| server（P68–P71） | 地址/配置、实际支持协议与 TLS、请求关联、EDNS/截断、并发/期限、连接关闭及畸形输入；UDP/TCP 最小链路先在 5A 验证，其余在 5B 完整覆盖。 |

switch1–17 逐项保留配置和持久化语义；位号必须遵守 [config-notes](../ai/config-notes.md) 的共享命名空间，特别是 fast_mark 48 和 switch17/bit 49。不能通过调整内部布局改变生成规则结果。

### 5A 基础观测的有界证据（2026-09-26）

已归档任务 [native query observability](../../.trellis/tasks/archive/2026-09/09-24-rust-phase5a-native-query-observability/prd.md)
已在严格 W1 UDP/TCP、W2/W3 单 listener 子集中接入审计开关、终态记录、
基本指标和有界快照，Linux 功能回归通过。M8 W1 与 M9 W2 暖缓存仅通过
100QPS 筛查；冷缓存只验证正确性。M9 错误夹具 W3 仍无效。另行授权的
M10 九场 W3 完成 27000 个正确查询，45000 条路由事件离线核对通过，四项
配对中位数门槛通过。原始驱动 FAIL 和退出竞态日志保留；独立 DNS ID 路由
证明及完整退出回执获 M10-FINAL-001 明确 PASS，有限基本可观测性 A1–A6
验收通过；2026-09-26 经用户授权完成归档，完整 5A 和生产部署尚未验收。见 [当前验收报告](../../.trellis/tasks/archive/2026-09/09-24-rust-phase5a-native-query-observability/research/m10-w3-assessment.md)。
这不将 C08、P37/P38 或任何完整产品验收条目改为完成；完整审计 API、
Prometheus、WebUI、持久化仍归5C，容量和整机验收仍待后续阶段。

## 5. 跨模块和管理面覆盖

以下均为待原生接入/验收。Go 来源提供发现入口；具体参数、HTTP method/path、JSON schema、指标和格式清单由拥有任务从源代码及基线提取并冻结。

| ID | 产品面及来源 | 归属 | 验收闭环 |
| --- | --- | --- | --- |
| C01 | YAML/include、插件注册/依赖/启动关闭：[config.go](../../coremain/config.go)、[plugin.go](../../coremain/plugin.go)、[mosdns.go](../../coremain/mosdns.go) | 5A 子集 -> 5B/5C | 原配置加载 -> 真正执行；错误定位、引用/顺序、初始化失败清理、所有参数实际生效 |
| C02 | DNS wire/query/server handler：[pkg/query_context](../../pkg/query_context/)、[pkg/server_handler](../../pkg/server_handler/)、[pkg/server](../../pkg/server/) | 5A/5B | 问题/响应/ID/标志/记录、TTL、EDNS/ECS、截断和错误行为；dns-core 局部支持不能代替完整报文支持 |
| C03 | 上游配置及策略：[pkg/upstream](../../pkg/upstream/)、[forward](../../plugin/executable/forward/) | Phase 4/5B | UDP/TCP/TLS/HTTPS/H3/DoQ、pipeline 别名、bootstrap/双栈、连接复用、并发选择、超时/重试/fallback、SOCKS/local bind/socket policy；按支持配置验证，不把无 fallback 的 foundation 当成最终策略 |
| C04 | 数据格式与规则下载：P01–P07、[adguard](../../plugin/executable/adguard/) | 5B/5C | 逐格式解析/优先级/过滤/持久化，下载失败与热更新；main 后续新增格式补录 |
| C05 | `special_groups`：[api_special_groups.go](../../coremain/api_special_groups.go) | 5B/5C | 保存/排序/绑定上游 -> 生成配置 -> DNS 路由 -> 最终审计；同名内部标签不等于实际生效 |
| C06 | 上游管理：[api_upstream.go](../../coremain/api_upstream.go)、P23 `aliapi` | 5B/5C | tags/config/runtime API、配置和运行状态一致，上游组修改后查询结果可验证 |
| C07 | 配置导出/更新/覆盖/生成：[config_manager.go](../../coremain/config_manager.go)、[config_update.go](../../coremain/config_update.go)、[api_overrides.go](../../coremain/api_overrides.go)、[api_domain_generation.go](../../coremain/api_domain_generation.go) | 5C | schema/package ID、managed_files、生成顺序、失败回滚和重启；保留用户数据与运行状态 |
| C08 | 查询审计 v1/v2：[api_audit.go](../../coremain/api_audit.go)、[api_audit_v2.go](../../coremain/api_audit_v2.go)、[audit.go](../../coremain/audit.go) | 5A 基础 -> 5C 完整 | 启停/容量/清空/查询、统计/排名/窗口、最终路由/上游字段；正常及高负载下不静默缺失要求记录的事件 |
| C09 | 日志/capture：[api.go](../../coremain/api.go)、[capture.go](../../coremain/capture.go)；统计/摘要 P37/P38 | 5C | 启停和查询、关键字段、容量/并发行为、DNS 热路径开销 |
| C10 | Prometheus 和插件 API：[mosdns.go](../../coremain/mosdns.go)、各插件注册的 handler/metric | 5C | 指标名/label/单位和插件 `/plugins/{tag}` 路径、请求/响应；加载完整代表性配置枚举路由，不能只覆盖 coremain 的 `/api` |
| C11 | Vue `/`、兼容 `/log`、assets、外部 UI 挂载：[mosdns.go](../../coremain/mosdns.go)、[webui-log](../../webui-log/) | 5C | 保留既有静态资源构建/服务与完整页面操作、保存/刷新/重启；包含外部 UI 路径及保留名称规则 |
| C12 | 外观与系统设置：[api_appearance.go](../../coremain/api_appearance.go)、[api_system.go](../../coremain/api_system.go)、[webui_port.go](../../coremain/webui_port.go) | 5C | 上传/历史/颜色、health/端口/重启；持久化与界面效果一致 |
| C13 | cache dump 与运行时文件：[cache](../../plugin/executable/cache/)、[state_files.go](../../coremain/state_files.go)、[file_save.go](../../coremain/file_save.go) | 5B/5C | `mosdns_cache_v2` 导入/导出/flush、runtime JSON、`/cus/mosdns/webinfo` 迁移/优先级、重启恢复、写失败与备份 |
| C14 | CLI/服务生命周期：[run.go](../../coremain/run.go)、[service.go](../../coremain/service.go)、[openwrt.go](../../coremain/openwrt.go) | 5A 启停 -> 5C；其它平台另排 | Linux amd64 CLI/config/信号退出/服务管理；平台特有操作单独标注，不能误报已验证 OpenWrt |
| C15 | 二进制/配置更新和发布：[update_manager.go](../../coremain/update_manager.go)、[update_guard.go](../../coremain/update_guard.go)、[api_update.go](../../coremain/api_update.go)、[workflows](../../.github/workflows/) | 5C/5D/6 | manifest/校验/失败恢复/用户状态保留、Vue 打包、可回退发布产物；无 Go/cgo runtime 依赖 |
| C16 | 诊断接口 `/debug/pprof`：[mosdns.go](../../coremain/mosdns.go) | 5C 兼容分类 | Go profiler 格式与内部机制不自动成为 Rust 实现要求，但端点可见性/使用依赖必须盘点；若要删除或替换用户依赖的诊断能力，单独提出兼容决策，不能直接消失 |
| C17 | 本地/远端引用资源和配置包：[config-notes](../ai/config-notes.md)、外部配置包仓库 | 5A 选取子集 -> 5B/5C | 记录实际包版本和文件边界，完整代表性配置可加载并运行；不能只使用手写最小 fixture 宣称全兼容 |

C16 是未来对应任务需完成的兼容分类，不是本轮删减批准；若分类涉及用户可见行为改变，必须在该变更实现前取得用户决策。现有 fork 排除的 nft/eBPF 不进入本表，平台新增适配由总规划单独排期。

### 6.1 5A bounded W2 evidence

`rust-phase5a-native-cache` 的 Slice 3 在 reviewed source
`b558d153cad9ad8e3ffaf18a6e2dde82329e32e0` 上完成了 Linux amd64 的
strict-W2 correctness gate：单一 host-owned cache、plain UDP hit/miss/
expiry、W1 UDP/TCP forwarding preservation，以及 Go cgo regression。证据
与精确命令见 [task execution state](../../.trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-cache/research/execution-state.md)。
最终 reviewer 已对 `c6b7f80226a13fa9ab81945fe782fe2c7c6bb5d0` 返回 PASS，任务已获用户授权归档。
这只关闭表中 P26/C01/C02 的受限 5A 子集；完整 lazy/EDNS 产品语义、cache
dump/持久化、API/WebUI/metrics、完整插件与跨模块组合仍保持“待验收”，归
入后续 5B/5C/5D/Phase 6 门槛，不能由本次测试升级状态。

### 6.2 5A bounded W3 evidence

`rust-phase5a-native-routing` 的 Slice 3 Linux amd64 correctness evidence
已在 reviewed source `33e826ccd89a5db039bfc4d92aaf0593907dd95b` 收集：严格
W3 YAML、真实 UDP A/B/C 路由、冻结 `routing.jsonl` 三行语料、W1/W2 回归、
关闭/取消/rebind 以及 sequence/runtime ABI tests 均通过。W3 事件 oracle
验证了 `A`、`B -> A`、`B -> C` 的精确顺序和 forbidden-leg 计数；完整命令、
远端环境、输入 digest 和实际限制见该任务的
[implement.md](../../.trellis/tasks/09-22-rust-phase5a-native-routing/implement.md)。
这只增加 P26/P27 与 C01/C02 的受限 5A native-routing evidence，不表示
完整 matcher/plugin、观测、性能或生产就绪。Linux tagged cgo cache/query/
matcher 组及 race 证据通过；未修改的 `matcher_adapter` typed-nil interface
测试仍会 panic，因此不能声称完整 legacy cgo matcher suite 已通过。最终
reviewer gate 尚待实际返回，且没有启用 production/default wiring。

## 6. 跨功能组合与关闭规则

单项之外至少覆盖：规则/provider + sequence + cache + 上游组；ECS + cache 隔离；fallback/dual selection + 取消 + 最终审计；switch/fast mark + special_groups；规则热更新 + 在途查询；API 保存 + 配置生成 + 重启恢复；更新失败 + 用户数据保留。

每个拥有任务在本表条目下补充或链接：

`条目 ID -> 参数/操作契约 -> task -> Rust 路径 -> 原生集成测试 -> 性能/故障/恢复报告 -> 验收状态`

同一条目由多个阶段负责时，各阶段交接列出未完成子项及接收任务/阶段，不能因某个子任务归档而关闭整行。5B 关闭全部查询条目，5C 关闭管理/状态条目；5D 汇总完整 E2E 和性能证据，Phase 6 删除桥接后再确认所有条目有效。未映射、未测或只测 hybrid 的条目保持待验收。
