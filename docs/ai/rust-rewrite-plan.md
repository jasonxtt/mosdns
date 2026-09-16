# MosDNS Rust 渐进重写方案

最后更新：`2026-08-18`

## 1. 目标与边界

最终目标是得到**纯 Rust-native 的 mosdns 二进制和运行时**，而不是永久保留 Go 外壳或 Go/Rust 双后端。迁移完成前本 `rust` 分支不作为实际运行版本，因此 Phase 3B 起优先建设最终 Rust 架构，不再为了中途上线而新增 Go fallback、backend selector、cgo adapter 或双写状态。

最终 Rust 版本仍必须保持本项目的产品契约：

- YAML 配置、插件类型名、参数和 sequence 语言的用户语义保持兼容；
- `/` Vue WebUI、`/log` 兼容页面以及现有 HTTP API 的用户工作流保持兼容；
- `special_groups`、在线规则、本地规则、上游组绑定、最终 DNS 行为和审计/路由结果保持兼容；
- cache dump、指标名、关键日志字段、运行时 JSON 和发布/配置更新流程具有明确的迁移策略；
- 安全性、确定性或架构质量需要改变历史行为时，必须把差异记录为 intentional Rust deviation，并在最终 host 验证中证明不会破坏已冻结的用户契约；
- 继续排除本 fork 不采用的 `nft` / `eBPF` 方向。

### 兼容目标分级

后续阶段统一使用三类判定，禁止把“Go 当前怎么写”自动升级为 Rust 规范：

1. **Product contract — 必须保留**：用户配置语法、sequence/plugin 语义、最终 DNS 响应、路由/审计结果、WebUI/API 工作流、持久化格式和其他明确的外部可观察行为。
2. **Migration scaffolding — 临时存在**：Phase 1/2/3A 已建立的 cgo bridge、`MOSDNS_*_BACKEND` selector、Go mirror/fallback、双 generation、FFI handle 和 circuit breaker 只用于早期迁移验证；Phase 3B 起不得把它们当成最终架构要求，也不应继续扩张。
3. **Go implementation detail / accidental quirk — 不自动保留**：Go interface、`ChainWalker` 递归、`map[uint32]any`、内部 buffer/pool 布局、错误字符串、字符串命名触发的偶然副作用、固定内部并发数等，仅用于理解现状。若其是否属于用户契约不明确，先 characterization，再明确标记为 `preserve` 或 `intentional Rust deviation`。

阶段 0 已产生的 Go baseline/golden fixtures 继续作为**行为取证证据**，不是“所有 Go/Rust 差异必须为零”的永久约束。已归档 Phase 1/2/3A 历史记录不回写；其 hybrid 代码保留到 Rust-native host 全链路可运行后统一清理。

当前阶段不重写 WebUI。Vue 前端继续作为稳定控制面；最终 Rust host 必须承接现有控制面契约。

## 2. 总体架构：临时 Go 外壳 -> Rust-native host

采用渐进替换（strangler）方式建设 Rust 模块，但**Go 外壳只是迁移脚手架，不是目标架构**。Phase 1/2/3A 已利用 hybrid 边界验证 cache、matcher 和 query core；Phase 3B 起的新增模块默认只面向最终 Rust host。

迁移早期：

```text
Vue WebUI / YAML / HTTP API
            |
       Go coremain
            |
    Go plugin / sequence host
            |
      versioned C ABI
            |
  Rust cache / matcher / DNS core
```

Go 在早期继续负责：

- 配置加载、插件注册和生命周期；
- WebUI/API 与运行时文件管理；
- 尚未迁移的 sequence、upstream 和 server；
- 已完成早期模块的对照验证与临时 fallback。

这些职责会在 Rust-native host 接管后退出；Phase 3B 起不再为新模块新增 Go runtime fallback。

Rust 优先负责：

- 状态边界清晰、计算密集、可用输入输出做对照测试的模块；
- 缓存、域名/IP 匹配、规则编译和 DNS wire 处理；
- 后续再承接 sequence 执行、upstream transport 和 server runtime。

Phase 1/2/3A 已形成一个 Rust runtime 静态库和粗粒度 ABI，用于早期 hybrid 验证。该 ABI 必须保持安全直至退役，但 Phase 3B 之后不再把“继续扩展 C ABI”视为默认方案；纯 Rust sequence、transport、server 与 host 应优先通过 Rust crate/API 直接组合。最终纯 Rust 二进制完成后，过渡 ABI 与 Go adapter 进入统一 retirement gate。

## 3. ABI 与工程规则

以下约定继续约束**已经存在或确有必要维护的过渡 Go/Rust 边界**，但不是 Phase 3B+ 新模块必须创建 ABI 的授权。Phase 3B 起默认使用纯 Rust crate 边界：

- ABI 有显式版本和能力查询，Go 启动时先校验；
- Rust 只暴露 `extern "C"` 的固定宽度整数、opaque handle 和明确所有权的 byte buffer；
- 分配方负责释放，返回 buffer 必须携带足以正确释放的元数据，不能假设 `Vec` 的 capacity 等于 length；
- 所有 Rust panic 必须在 FFI 边界内被拦截并转换为错误码，不能跨越 C ABI；
- 锁中毒、空指针、长度溢出、重复 close 和并发 close 必须有确定行为；
- 热路径采用粗粒度调用，禁止为单条 DNS 记录产生大量 cgo 往返；
- 既有 Rust 错误在 Go adapter 中必须保持可观测且不能破坏当前 Go 默认路径；
- `main`/现行发布继续保持 Go 后端，直到完整 Rust-native binary 通过最终切换 gate；这不要求 `rust` 分支的新模块继续提供 Go fallback。

建议的目录形态：

```text
rust/
  Cargo.toml               # workspace（第二个 crate 接入时建立）
  cache-core/
  domain-matcher/          # 后续阶段
  dns-core/                # 后续阶段
scripts/
  build-rust-*.sh
plugin/.../rust_bridge_*.go
```

## 4. 旧 cache 项目审计结论

来源：`/Users/tom/github/mosdns-rust-cache`

### 已具备的能力

- Rust `staticlib` 和 C header 已存在；
- create/free、lookup/store、flush、dump import/export 已接通；
- 支持现有 key、ECS、`exclude_ip`、lazy cache TTL 和 `domain_set`；
- Go 侧有 Linux+cgo+build tag 桥接、运行时开关和 Go fallback；
- 有 Go/Rust 行为对照测试、benchmark 入口和 raw DNS response 快路径；
- 历史文档记录了 `2026-05-27` 在 Debian 13 x86_64 上的构建和运行验证；
- 当前本机验证：Rust 5 个单元测试通过，Go cache 包测试通过。

### 不能直接整仓搬入的原因

- 旧仓库基线是 `2026-05-27`；`rust` 分支基于 `main` 的 v0.7.1，后续同步必须以明确的 main commit 为准，不要把该基线当作当前 main 版本；
- 旧仓库最后一批 L1、Mutex 和 CI 变更仍是未提交工作树内容，不能只 cherry-pick 一个稳定提交；
- raw response 快路径还依赖 `pkg/query_context` 和 `pkg/server_handler` 的配套改动，不能只复制 cache 目录；
- 当前发布 workflow 已增加 Vue 预构建和更新 manifest，旧 workflow 不能覆盖复制；
- Rust 模式的 cache size 指标仍读取 Go backend，关闭 mirror 时会错误地显示为 0；
- Rust L1 查找按最多 51,200 个 slot 线性扫描，热路径是 O(n)，必须改为 O(1) 索引或分片实现；
- 整个 Rust cache 被单个 `Mutex` 串行化，高并发收益尚未成立；
- FFI buffer 释放按 `len == capacity` 重建 `Vec`，对 capacity 不等于 length 的返回值存在内存安全风险；
- `Mutex::lock().unwrap()` 和未捕获 panic 可能在 FFI 边界导致进程退出；
- L1、dump round-trip、FFI buffer、并发和 panic/fallback 缺少直接测试；
- `cargo clippy --all-targets -- -D warnings` 当前失败（1 个实现告警及 11 个 FFI safety 文档告警）。

因此结论是：cache 的 Rust **核心原型和接线已经完成**，但“完整 cache 插件已可直接接入最新版并发布”不成立。正确做法是以旧工作树为参考，按功能块移植并先完成硬化。

## 5. KixDNS 复用评估

候选上游：`https://github.com/olicesx/kixdns`

审计基线：`2da3a2d`（`2026-08-12`）

### 项目成熟度

- KixDNS 与本项目同为 GPL-3.0，代码复用在许可证上兼容；引入源码时仍需保留来源、版权和 GPL 要求；
- Rust 2024、Tokio、Hickory DNS、Moka、DashMap、ArcSwap、rustls、Quinn 等技术栈与本项目的目标方向一致；
- 仓库历史约 139 个提交，已有 `v0.1.0`、`v0.1.1` 两个 tag，仍属于早期版本；
- 当前本机验证通过：134 个 library tests、6 个 binary tests、9 个 DoH integration tests，共 149 个测试；
- `cargo clippy --all-targets -- -D warnings` 通过；仓库 CI 还覆盖 fmt、Linux GNU/musl amd64/arm64、Windows 和 FreeBSD 构建；
- 上游仍在活跃修复 GeoIP、DoH 连接恢复、cache 隔离、UDP 截断和并发死锁问题；
- README 明确说明初始实现和文档主要由 AI 生成，并指出仍有反序列化后未被 runtime 使用的配置字段，因此所有复用代码仍需逐模块审计，不能只依据测试数量判断生产可靠性。

### 优先直接复用或抽取的部分

1. `proto_utils.rs`
   - DNS query/response 快速解析；
   - raw wire TXID、TTL 修正；
   - UDP 截断和 EDNS payload 处理；
   - 这些能力可以替代旧 cache 原型中每次通过 Hickory 完整反序列化/重新编码的热路径。
2. `ecs.rs`
   - ECS 注入、删除、prefix masking 和 cache 隔离 key；
   - 需要用 mosdns 当前 ECS key 语义做 golden parity 后再接入。
3. Moka/`Bytes` cache 结构和并发模式
   - 可以替代旧原型的全局 `Mutex`、线性 L1 slot 扫描和手写淘汰；
   - KixDNS 的 cache entry 使用 hash key，并保存 qname/pipeline/qtype 做碰撞校验，适合参考其零分配查找方式。
4. GeoSite/GeoIP loader、索引和热替换思路
   - 支持 V2Ray dat/JSON、属性过滤、IPv4/IPv6 和热加载；
   - 可作为 `special_groups`、domain set 和 IP set Rust 化的基础，但匹配边界和规则优先级必须以 mosdns 为准。
5. compiled pipeline/rule index
   - 可以复用候选索引、首个规则优先和不可安全快进时退出 fast path 的设计；
   - 不直接采用 KixDNS 的 JSON pipeline 语义。
6. upstream transport 实现
   - UDP/TCP multiplexing、DoH/DoT/DoQ、连接池、自恢复和 0-RTT 处理可在后期提取；
   - 这部分与 KixDNS Engine 耦合较深，应在 sequence/query context 兼容层稳定后再动。

### 不能直接替换的部分

- KixDNS 使用 JSON pipeline，本项目使用 mosdns YAML、插件注册和生成配置工作流；
- KixDNS 没有本项目的 `coremain`、Vue WebUI API、运行时 JSON 状态、配置包更新和事务状态管理；
- KixDNS 没有本项目要求的 Prometheus 指标、query audit 和最终路由字段；
- KixDNS cache 是内存 cache，没有兼容 `mosdns_cache_v2` 的 dump/load/show/flush API；
- KixDNS cache entry 的 pipeline/upstream 语义不能直接代替本项目的 `domain_set`、`effective_tag`、`final_sequence` 和 dedicated upstream group；
- KixDNS 的 rule/action 集合与 mosdns sequence 插件不是一一对应；
- KixDNS 当前没有 inbound DoT/DoQ，而本项目已有的 server 能力不能因此倒退；
- KixDNS 是单 crate、约 2.2 万行代码，cache 语义分散在 engine execution/phases/rules 中，不能把 89 行的 `cache.rs` 当作完整 cache 插件直接复制。

### 调整后的复用原则

- KixDNS 作为 Rust 数据面首选上游，旧 `mosdns-rust-cache` 作为 mosdns ABI、dump、API 和 fallback 兼容参考；
- 不 fork 后整体改名，也不复制其 JSON 配置和主程序；
- 按固定 commit 引入经过审计的模块，并在源文件/NOTICE 中保留来源；
- 每次同步 KixDNS 都单独审查 upstream diff，不自动跟随 main；
- 若只需要第三方 crate 已提供的能力，优先直接依赖 crate，例如用 Moka 代替复制 KixDNS 的薄 cache wrapper；
- mosdns **产品契约**始终优先。KixDNS 测试和当前 Go 行为都只是取证来源，不能替代显式的 Rust contract；Go characterization 的结果必须分类为 `preserve` 或 `intentional Rust deviation`，而不是默认要求逐项 parity。

## 6. 分阶段实施

### 阶段 0：冻结产品契约、行为基线与基准

阶段 0 已完成的 Go baseline/golden fixtures 保留为行为取证材料。它们帮助识别用户依赖，但不意味着 Go 的每个内部或偶然行为都必须在 Rust 中复刻。

工作内容：

1. 在 `rust` 分支记录与 `main` 的同步点，后续定期将 `main` 合入 `rust`；
2. 建立 Go baseline：全量测试、构建、核心配置启动、DNS 行为和 WebUI/API 冒烟；
3. 保存 cache、matcher、sequence 的 golden fixtures；
4. 固化性能基准环境、命令、数据集和指标；
5. 列出必须保持的配置字段、API、Prometheus 指标、审计字段和 dump 版本。
6. 建立 KixDNS 模块映射表，逐项标注 `direct dependency / extracted code / adapted design / reject`，并记录固定上游 commit。

完成标准：同一套取证测试能够记录 Go 基线，并让后续 Rust 实现对差异做结构化分类：`product-contract preserve`、`intentional Rust deviation` 或 `implementation-only/not applicable`。

### 阶段 1：移植并硬化 Rust cache

以 KixDNS 的 Moka/Bytes/raw-wire 方案为并发与热路径基础，以旧 cache 项目的 mosdns bridge、dump 和 API 为兼容参考。只移植以下功能块，不覆盖任何一个旧仓库：

1. 新的 `rust/cache-core`、header 和构建脚本；
2. cache bridge 新文件以及 `cache.go` 的最小接入点；
3. 从 KixDNS 审计后抽取的 quick DNS parser、TTL patch、UDP truncate 和 ECS 工具；
4. raw response 所需的 query context/server handler 改动及测试；
5. `mosdns_cache_v2` dump、show/load/flush API、`domain_set` 和指标兼容层；
6. 在当前 workflow 上增量加入 Rust 构建，保留 Vue 构建与 manifest 流程。

在启用前必须完成：

- 修复 FFI buffer 所有权、panic 边界和锁中毒处理；
- 使用 Moka/分片并发结构替代旧原型的线性 L1 和全局单锁；
- Rust backend 暴露正确的 len/metrics；
- 为当时的 experimental hybrid bridge 明确“本次 Go fallback”与“禁用 Rust backend”策略；该策略属于已完成 Phase 1 的迁移脚手架，不是最终 Rust-native cache 的运行时要求；
- 增加 TTL、NXDOMAIN、SERVFAIL、空回答、ECS、exclude、lazy update、dump、API、并发和 raw UDP/TCP/HTTP 路径测试；
- Rust tests、clippy、Go tests、race tests 和 Linux bridge tests 全部通过；
- 比较 Go/Rust 的 QPS、p50/p95/p99、CPU、RSS 和分配次数。

发布门槛建议：Rust 热路径不能出现统计显著的吞吐回退；p99 延迟、CPU 或内存若恶化超过 10%，先停留在实验产物，不进入生产默认路径。

### 阶段 2：迁移 domain/IP matcher 与规则编译

这是 cache 后最合适的模块：输入输出清晰、状态主要为只读索引，并且直接服务 `special_groups`、在线规则和本地规则。

边界：

- Go 继续负责文件下载、API 保存、reload 生命周期和审计标签；
- Rust 负责规则解析、编译后的只读 matcher、批量 match，优先复用 KixDNS 的 GeoSite/GeoIP loader 与 compiled rule index；
- reload 采用构建新 handle 后原子替换，失败时保留旧 matcher；
- 用现有 Go matcher 测试样本和真实规则集做逐条 parity 与内存/加载时间比较。

完成标准：属于产品契约的规则语法、优先级、匹配结果和 `special_groups` 路由标签一致；Phase 2 的 Go fallback/generation pairing 作为过渡验证脚手架保留到统一 retirement gate，不约束最终 Rust-native matcher 的内部结构。

foundation 状态（`2026-08-13`）：matcher foundation task 的 Slice 0–5
已完成实现和验证，independent review passed on 2026-08-13，task 已归档/完成。
Phase 2 expansion task 的 Slices 0–5 也已完成实现、root review 和验证；
`sd_set`、`si_set` 与 valued `domain_mapper` 仍通过
`MOSDNS_MATCHER_BACKEND=rust` opt-in，Go fallback 和默认 Go-only 不变。
扩展证据记录在 `docs/rust/matcher-compatibility.md`、
`docs/rust/benchmarks/matcher-phase2-expansion.md` 和
`docs/rust/test-host-matcher-phase2-expansion.md`。这些门禁不授权默认切换、
生产部署或阶段 3。

### 阶段 3：建立 Rust DNS/query 执行核心

在 cache 和 matcher 两个模块验证 ABI 约定后，才建立统一 Rust workspace/runtime，并迁移：

1. DNS wire 解析、TTL/EDNS/ECS 工具；
2. Rust query context 的稳定数据模型；
3. matcher 调度和无网络 executable；
4. sequence 的控制流、`jump/goto/return/exit/try` 和审计事件。

这一阶段是架构拐点。Phase 3A 已完成 hybrid query/wire 取证边界；Phase 3B 起直接建立未来 Rust host 使用的 owned query/sequence state，不再新增 Go/Rust live bridge。必须保证一条最终请求在 Rust-native host 内只有一个执行所有者。

完成标准：冻结的产品契约（配置/sequence 语义、wire response、最终 upstream 标签、`domain_set`、`effective_tag`、`final_sequence` 等）由 Rust-native execution model 表达；Go 偶然行为可以作为显式 deviation 改进，不要求内部逐项 parity。

### 阶段 4：建立纯 Rust upstream 与 server 数据面

Phase 4 起不新增 Go upstream adapter、transport C ABI、Go pool-buffer ownership 或 runtime fallback。Go upstream 只用于发现既有用户/协议行为；Rust transport 直接面向未来 Rust sequence/host。

顺序建议：

1. UDP/TCP upstream；
2. TLS/HTTPS；
3. QUIC/HTTP3；
4. bootstrap、连接复用、协议内 fallback（如 UDP TC→TCP）、超时与统计；
5. UDP/TCP/TLS/HTTP/QUIC server listeners。

完成标准：协议矩阵、超时/取消、连接复用、截断、EDNS、并发和故障注入测试通过，且 test host 长时间运行无回归。

### 阶段 5：形成可独立运行的 Rust-native host

最后迁移：

- YAML 配置和插件注册；
- coremain 生命周期、HTTP API、指标和运行时文件管理；
- 服务管理、更新流程和嵌入式 Vue 静态资源。

Vue 源码无需因后端语言变化而重写。Phase 5 的目标是让 Rust-native binary 在不依赖 Go runtime/cgo fallback 的前提下覆盖冻结的产品契约，并能够完成完整端到端验证。

### 阶段 6：Hybrid scaffolding retirement 与纯度 gate

只有在 Phase 5 Rust-native host 已能完整执行配置加载 -> listener -> query -> sequence -> matcher/cache -> upstream -> response -> audit/API 工作流后，才统一清理早期迁移脚手架。不要在 Phase 3B/4 边开发边拆，避免同时扩大变量。

清理范围至少包括：

- `MOSDNS_CACHE_BACKEND`、`MOSDNS_MATCHER_BACKEND`、`MOSDNS_QUERY_BACKEND` 等仅用于 hybrid 验证的 selector；
- Go↔Rust cgo adapter、C header/FFI runtime symbols 和仅为 Go handle 生命周期服务的 registry（若最终 Rust host 无其他外部 ABI 使用者）；
- Go mirror、Go fallback、same-generation Go/Rust publication、circuit breaker 与 transitional fault-injection 路径；
- 仅为 experimental hybrid binary、bridge benchmark/smoke 服务且最终 native 路径不再使用的构建和测试入口。

retirement 必须先证明对应 Rust-native 路径已有等价产品契约测试，删除后再跑完整 Rust-native E2E、配置/API/WebUI/audit/dump/重启恢复以及性能/长期运行 gate。Phase 6 完成前不得把 `rust` 分支作为最终替代版本发布。

## 7. 验证与发布矩阵

验证策略随阶段变化：

```text
Phase 0-3A（已完成 hybrid 取证）
  本机/CI -> Go/Rust characterization/parity -> isolated mos-test evidence

Phase 3B-4（纯 Rust foundation，尚无最终 host）
  Rust 单元/属性/并发/协议测试 -> 必要的 Linux isolated integration/benchmark
  -> 不新增 Go selector/fallback，不做“中途上线”验证

Phase 5-6（Rust-native host 与清理）
  完整 Rust-native E2E -> Linux 长时间运行/性能 -> 配置/API/WebUI/audit/dump/重启恢复
  -> 用户确认 -> 最终替代/发布验证
```

首批 Rust 产物只支持 Linux amd64。Linux arm64 在 cache 稳定后加入；OpenWrt、lite 和 docker 分支的 fan-out 不在首个里程碑内，避免同时扩大平台和功能变量。

## 8. 分支和同步策略

- `rust` 是长期迁移分支，基线为 `main` v0.7.1（`3896a4a`）；
- `main` 的业务修复继续正常推进，定期 merge 到 `rust`；
- Rust 未达到单个里程碑验收门槛前，不反向合回 `main`；
- 每个迁移模块独立提交：契约测试、Rust core、必要集成、构建/CI、文档分开；Phase 3B+ 不以新增 Go bridge 作为默认交付物；
- 不从 `/Users/tom/github/mosdns-rust-cache` 整仓 merge，也不覆盖当前 workflow 或前端产物；
- 不从 KixDNS 整仓 merge；按固定 commit、模块和许可证记录抽取，后续同步逐次审查；
- 旧仓库保留只读，直到 cache 移植完成并核对所有未提交差异。

## 9. 当前 Rust migration 状态

阶段 1 的 cache 基础实现已经完成；cache 仍保留独立的
replay/soak、sanitizer/Miri 和扩展测试机验证门槛，且不得成为默认后端。

`.trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation/` 的 Slice 0–5
已在 `rust` 分支完成，independent review passed on 2026-08-13；其归档不
结束或归档整个 Rust 重写计划。Phase 2 expansion 也已归档完成。

Phase 3A（query/wire foundation）已获得授权并完成；其 task
`08-15-rust-phase3-query-execution-core` 已于 2026-08-17 归档。整体 Phase 3
尚未完成：Rust matcher dispatch、无网络 executable、sequence 控制流
（`jump`/`goto`/`return`/`exit`/`try`）以及 query-context 执行所有权仍由 Go
持有。当前请求路径仍是 `Go EntryHandler -> Go query_context -> Go sequence
-> Go upstream`，Phase 3A 的 query ABI/adapter 仍只是 experimental opt-in
旁路，默认后端保持 Go-only。

Phase 3B 是下一项工作，必须先完成并 root-review/归档后才能进入 upstream
transport。`08-17-rust-phase4-upstream-foundation` 目前只有 planning PRD，已
明确 NO-GO/deferred；它不能 `task.py start`，也不授权任何 Phase 4 实现。Phase
3B 起执行新的 Rust-native policy：Go 只用于产品契约取证，不再新增 Go
fallback/cgo/runtime selector。Phase 4 将采用纯 Rust transport foundation，
冻结 cancellation、post-side-effect failure、协议/socket 产品语义和共享 async
runtime，而不是设计 Go pool ownership、transport C ABI 或新的 hybrid handle
namespace。Phase 5 建立完整 Rust host，Phase 6 再统一删除 Phase 1/2/3A 的 hybrid
scaffolding。
