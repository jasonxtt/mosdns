# Rust Phase 4 upstream connection reuse and pipeline foundation

## Goal

为未来 Rust-native host 建立纯 Rust 的**受控连接复用与 pipeline 基础**：在已完成的
numeric UDP、fresh plain TCP、DoT/DoH one-shot、resolver/bootstrap 与双栈候选
选择之上，让同一个 numeric dial 目标与同一服务身份可以被多次 exchange 复用，
而不改变 TLS/HTTP 服务身份、caller-owned deadline、取消/owner close 语义。

用户价值：减少每次查询都重建 TCP/TLS 连接的开销，同时保持既有冻结契约——数字
拨号地址与服务身份分离、原 DNS ID 不被改写、caller 的绝对 deadline 不被重置、
owner close 能确定性排空。这是数据面的性能基础，不是"把所有 upstream 协议做完"。

## Confirmed facts（来自当前代码，非推断）

### 已存在的传输与生命周期契约

- `rust/upstream-core/src/lib.rs:35-65` 定义 `Transport::{Udp,Tcp}` 与
  `Endpoint`（numeric `SocketAddr` + transport，port 0 在构造期拒绝）。
- `lib.rs:87-115` `ExchangeRequest` 借用 caller 查询字节并记录 `request_id`；
  文档明确"never copied, rewritten, or revalidated"——**原 DNS ID 必须保持**。
- `lib.rs:159-248` `ExchangeContext`（`deadline()`、`check_at()`）与
  `ExchangeControl`（`caller_cancellation()`、`owner_cancellation()`）；取消优先于
  deadline 的 tie-break 在 `lib.rs:185` 与 `check_at` 内固定。
- `lib.rs:250` `SideEffectState::{NotSent,MaybeSent,Sent}` 是闭合枚举；connect/setup
  失败为 `NotSent`，已发送后的失败保留状态。这是"可否安全换新连接"的唯一依据。
- `lib.rs:563-580` `LifecycleState::{Open,Closing,Closed}`；`Lifecycle`
  （`lib.rs:589-801`）用**单个 `std::sync::Mutex`** 同时保护 admission 与注册计数，
  `commit_response`（`:643`）与 `commit_final_response`（`:668`）是唯一提交
  线性化点；`register`（`:716`）、`register_shared`（`:729`）、`register_owned`
  （`:745`）、`drain`（`:770`）、`SharedInFlightGuard`（`:803`）。
- `lib.rs:953-1120` `Upstream` 暴露 `begin_close`/`finish_close`/`close`/
  `prepare_exchange`/`exchange`；每次 exchange 自带 owner 生命周期。
- `lib.rs:504-559` `ExchangeResponse` 拥有完整返回 wire，含 `request_id`/
  `response_id`/`transport`/`truncated`。

### 当前每个传输都是"每次新建、连接不保留"

- `tcp.rs:1-18` 模块文档明确：`exchange` "composes those helpers with exactly one
  fresh `TcpStream` per call. It is deliberately independent of policy, fallback,
  **pooling, reuse, pipelining**, and retry: there is no second framing
  implementation, **no connection is kept after the call**"。
- `tcp.rs:145-238` 是唯一 framing 实现：`encode_frame`/`write_frame`/
  `flush_bytes`/`read_frame`（2 字节大端前缀），`race_control`（`:239`）与
  `race_io`（`:274`）把 deadline/取消与 I/O 竞速。**这些必须被复用，不得重写第二套**。
- `secure/dot.rs:1-31` `DotUpstream`（`:178`）与 `DotPhase`（`:196`）每次 exchange
  新建一条连接；顺序契约是"numeric connect → 认证 TLS handshake → 才写查询"。
- `secure/doh.rs:1-29` `DohUpstream`（`:368`）、`DohPhase`（`:523`）；文档明确
  "no path ... retries, **pools the connection**, or falls back to another
  protocol"。已存在 H2 子任务托管设施：`H2Children`（`:110`）、
  `TrackedH2Executor`（`:119`）、`H2ScopeLease`（`:126`）、`H2ChildGuard`（`:132`）、
  `H2TeardownPause`（`:139`），以及 `restore_request_id`（`:946`）。
- `secure/endpoint.rs:23-40` `ServerIdentity` 与 numeric dial **分离**；
  `DotEndpoint::new(dial, identity)`（`:184`）、`DohEndpoint::new(service_url, dial)`
  （`:227`）。这是不能被复用破坏的安全契约。
- `composite.rs:34-63` `UdpTcpPolicy::new(udp_endpoint)` 复用同一 `SocketAddr` 派生
  TCP endpoint；`in_flight_exchanges()`（`:63`）是两个 leg 注册数之和。

### resolver 侧的多地址消费者边界

- `resolver/owner.rs:857-903` `ResolverComposition::{endpoint,dot_endpoint,doh_endpoint}`
  只接受一个 `PublishedTarget`；`ResolvedUpstream`（`:905-938`）用
  `published.dial()` 得到 numeric `SocketAddr`。
- resolver foundation（已归档 `09-18-rust-phase4-dual-stack-endpoint-selection`）
  提供 `ResolutionSnapshot`：按 family 的候选/诊断/`generation()`，且
  `selected_target(now)` 只返回新鲜候选。**resolver 已完成，不在本任务范围内修改。**

### Go characterization（仅为取证，不是 Rust 规范）

- `pkg/upstream/upstream.go:52-55` `pipelineConcurrentLimit = 64`（注释引用 RFC 7766
  §7 Response Reordering，并标 `TODO: Make this configurable?`）。
- `upstream.go:321-323` TCP `idleTimeout` 默认 10s；`upstream.go:352` 使用
  `transport.NewReuseConnTransport`；`upstream.go:405-407` DoH `idleConnTimeout`
  默认 30s。
- `pkg/upstream/transport/` 含 `reuse.go`、`pipeline.go`、`conn_lazy_dial.go`。
- 已归档 upstream foundation 的兼容矩阵（
  `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/design.md:520-521`）
  已把 "TCP connection reuse" 与 "TCP pipelining and pending demux" 分类为
  **implementation-only / defer**，即 Go 的资源优化，**不是产品契约**。

## Requirements

- R1. 在 `rust/upstream-core` 内建立显式的复用边界：一个由 owner 持有的连接集合，
  以 **(numeric dial 地址, transport 种类)** 及其安全相关身份为 key。复用只发生在
  numeric 目标之上——解析（resolver）不进入 pool key。
- R2. **服务身份与 numeric 拨号保持分离**：DoT 的 `ServerIdentity` 与 DoH 的
  service URL authority 必须参与复用 key，禁止跨身份复用一条已认证连接。ALPN 协商
  结果（HTTP/1.1 vs HTTP/2）也是 key 的一部分。
- R3. **每连接串行（已裁定）**：本任务正式采用 `max_pending_per_connection = 1`
  ——一条被复用的连接在任一时刻只承载**一个**未决查询。不得改写 caller 的 DNS ID，
  不得引入共享 ID 重写或 UDP 式 demux。同连接多未决、响应重排（RFC 7766 §7）与按原
  ID 的 demux 属**后续独立任务**（见 Deferred），不在本任务授权内；本任务不得以
  "临时改写 ID" 作为实现手段。
- R4. **绝对 deadline 与取消不被复用改变**：复用命中不得给 caller 新的超时预算；
  每次 exchange 仍使用 caller 的原始 `ExchangeContext`。owner close 必须排空并丢弃
  空闲连接，且已被 close 的连接不得被归还到集合。
- R5. **连接失效与重建有明确分类**：只在 `SideEffectState::NotSent`（查询字节尚未
  写出）时才允许换一条新连接再试一次；任何已发送后的失败是终态，不做通用重试、
  不做协议 fallback。空闲连接被对端半关闭属于 `NotSent` 情形。
- R6. **有界资源（已确认的 task-local 实现常量，不可配置）**：用户已确认本任务的
  实现常量取值为 `MAX_IDLE_PER_KEY = 1`、`MAX_IDLE_TOTAL = 8`、
  `IDLE_TIMEOUT = 10s`、`MAX_PENDING_PER_CONNECTION = 1`。它们是**实现层
  task-local bounded constants**：本任务**不**引入配置文件、环境变量或 API 来调整
  它们，因此它们**不是产品配置契约**，也**不承诺**与 Go 的 64/10s/30s 数值 parity
  （那些值已被判为 implementation-only）。design.md §6 记录取值与理由。
  backpressure 语义固定为**无等待队列**：无法获得空位时返回 typed 错误（或由调用方
  自建新连接），不静默排队；排队式准入延期（见 Deferred）。
- R7. **错误分类闭合**：新增复用相关错误变体与既有 `UpstreamError`/`SecureError`
  并列且不重叠；不得把复用失败重新标记为协议错误。
- R8. 复用只对 hostname 上游的**已解析 numeric 目标**生效；numeric `dial_addr`
  目标同样可复用。resolver 的多地址快照通过 `selected_target(now)` 提供 dial 地址，
  已建立的连接不因 resolver 刷新而失效——只有**新的拨号**使用新地址。
- R9. 保持既有 `UdpTcpPolicy`、`ResolverComposition`、`Endpoint`/`DotEndpoint`/
  `DohEndpoint` 构造契约不变；复用是新增的 owner/组合层，不改变单次 exchange 语义。

## Acceptance Criteria

- [ ] A1. 复用 key 的契约有 deterministic 测试：不同 numeric 地址、不同 transport、
      不同 `ServerIdentity`、不同 DoH authority、不同 ALPN 协议**不得**共用一条连接；
      相同 key 可以命中同一连接。
- [ ] A2. 安全回归：一条以身份 X 认证的连接**永不**被用于身份 Y 的 exchange；测试
      必须直接证明跨身份请求会新建连接而不是复用。
- [ ] A3. 原 ID 与 framing：复用路径上 `request_id`/`response_id` 相等且等于 caller
      原 ID；2 字节前缀 framing 仍来自 `tcp.rs` 唯一实现；响应不因复用而串线。测试
      还必须证明**每连接串行**：同一连接在任一时刻只有一个未决查询（第二个并发请求
      得到 typed 错误或另开连接，且不出现同连接上的 ID 改写）。
- [ ] A4. deadline/取消/close：复用命中不重置 deadline；`NotSent` 前失败允许一次新
      连接；`Sent` 后失败终态；owner close 排空并丢弃空闲连接，close 后不再归还，
      重复 close 收敛。
- [ ] A5. 资源边界：已确认常量 `MAX_IDLE_PER_KEY = 1`、`MAX_IDLE_TOTAL = 8`、
      `IDLE_TIMEOUT = 10s`、`MAX_PENDING_PER_CONNECTION = 1` 的淘汰/超限行为均有
      测试；超限行为是 typed 而非 panic/静默丢弃。这些上限以 task-local 常量实现且
      不可配置，因此测试只断言其**行为**，不断言与 Go 数值的 parity，也不通过配置
      改变它们。
- [ ] A6. 既有全部 UDP/TCP/DoT/DoH/resolver 契约测试继续通过，证明单次 exchange
      语义与既有身份/生命周期行为无回归。
- [ ] A7. Rust focused/full gates、`git diff --check`、task validate，以及隔离
      Debian VM 上的 Linux/Rust 1.85.1 loopback/MSRV 证据通过，且选定 reviewer 返回
      明确 scoped PASS 后才允许归档。

## In scope

- `rust/upstream-core` 内新增的连接集合 owner、key、空闲管理、**每连接串行**
  （`max_pending_per_connection = 1`，已裁定）与 typed 错误。
- plain TCP 与 DoT 的复用；DoH 的复用（按 ALPN 协议分离）。
- 与既有 `Lifecycle`/`ExchangeContext`/`SideEffectState` 的集成，以及既有
  `tcp.rs` framing 与 `race_*` helper 的**复用**而非重写。
- 面向 resolver `ResolutionSnapshot` 的只读消费边界（读取选中的 numeric dial
  地址），以及文档化的 QUIC/HTTP3 消费者边界。
- task-local bounded constants 形式的资源上限（实现层，不可配置，见 R6）。

## Out of scope

- **本任务不是"把所有 upstream 协议都做完"**。明确排除：
- QUIC/HTTP3/DoQ 的任何实现或依赖；
- SOCKS/local bind/socket marks/接口或源地址策略；
- UDP 侧重传策略与 UDP 连接复用（UDP 无连接，`slice1_udp` 契约不变）；
- server listeners、inbound 处理；
- host/YAML/config loader/plugin/sequence 接线、API/WebUI、生产或默认选择、部署；
- Go/cgo/FFI/C ABI、`MOSDNS_*_BACKEND` selector、Go mirror 或任何 fallback 路径；
- 泛化的跨协议 fallback 与连接失败后的跨地址/跨族竞速（Happy Eyeballs 已由用户
  在上一任务排除，此处同样不做）；
- 指标/审计/日志接线。

## Deferred（需要独立任务与评审，不在本任务授权内）

- **同连接多未决请求、响应重排（RFC 7766 §7）与按原 DNS ID 的 demux**：用户已裁定
  这属于**后续独立任务**。本任务只交付每连接串行复用（R3）；后续任务必须先给出
  在"不改写原 ID"约束下的无歧义关联策略，再讨论并发深度与重排。见 design.md §7 与
  §10。
- 全局连接预算与**准入队列式 backpressure**（等待空位而非立即 typed 失败）。
- HTTP/2 多路复用下的优先级/流控调优、0-RTT、连接迁移。
- 池化后的性能基准与长期运行/soak 证据（Phase 4 只要求 foundation 正确性）。
- 空闲连接的心跳/keepalive 与对端半关闭的主动探测。
- 把 task-local 资源上限提升为**可配置**项（YAML/config/API），以及与之相关的
  `requiredConfigSchema`/WebUI 工作流变更。

## Risks

- **安全风险（最高）**：跨身份复用一条已认证连接会把身份 X 的信任关系带给身份 Y。
  key 必须包含身份与策略，且需要直接的安全回归测试（A2）。
- **响应串线**：本任务以"每连接串行"从结构上排除该风险（R3）。同连接多未决是把
  该风险重新引入的前提，因此被划入后续独立任务，并要求先证明不改写原 ID 的无歧义
  关联策略；本任务不得为了让 demux 更容易而放宽 ID 契约。
- **资源泄漏**：空闲连接、在途 exchange 的归还、以及 close 时未排空的连接都是泄漏
  面；`Lifecycle` 注册计数必须覆盖"已借出"和"空闲保留"两类资源。
- **把 Go 当规范**：Go 的 64/10s/30s 是 implementation-only（见 archive 矩阵），
  不得据以声称 parity 或把其数值写成产品契约。
- **范围蔓延**：复用很容易顺势引入 fallback、重试、跨族竞速与 QUIC，这些都必须停在
  本 PRD 的 Out of scope 之外。

## Notes

- 本任务**planning-only**：`design.md` 与 `implement.md` 完成并经 reviewer 批准、
  且执行 `task.py start` 之前，不写任何实现代码。
- 依赖的既有任务已归档：`09-17-rust-phase4-endpoint-resolution-foundation`、
  `09-18-rust-phase4-dual-stack-endpoint-selection`、`09-16-rust-phase4-secure-upstream-foundation`、
  `08-17-rust-phase4-upstream-foundation`。resolver 部分**已完成**，本任务只消费其
  只读快照，不修改它。
- 保留 worktree 中已有的无关 dirty 文件与 `.DS_Store`；不使用 `git add -A`；Trellis
  auto-commit 保持关闭。
