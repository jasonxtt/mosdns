# Rust Phase 4 QUIC/HTTP3/DoQ foundation

## Goal

为未来 Rust-native host 建立纯 Rust 的 **QUIC/HTTP3/DoQ 单次基础**：在已完成的
numeric UDP、fresh plain TCP、DoT/DoH one-shot、resolver/bootstrap（含双栈）、
连接复用 serial 基础之上，补齐客户端协议矩阵的最后一块缺口——DNS-over-QUIC
（RFC 9250）与 DNS-over-HTTP/3（DoH3），而不改变既有的服务身份分离、
caller-owned deadline、取消/owner close 语义。

用户价值：上游协议覆盖完整（UDP/TCP/DoT/DoH/DoQ/DoH3），后续 host 可以按配置
选择传输；所有新路径仍是 bounded one-shot，每次新建连接，与 secure foundation
的 one-shot 形态一致，复用/调优留给后续独立任务。

## Confirmed facts（来自当前代码，非推断）

### 已存在的传输与生命周期契约

- `rust/upstream-core/src/lib.rs` 定义 `Transport::{Udp,Tcp}`（`:37-41`）与
  `Endpoint`（numeric `SocketAddr` + transport，port 0 在构造期拒绝）。
- `ExchangeRequest` 借用 caller 查询字节并记录 `request_id`；文档明确
  "never copied, rewritten, or revalidated"——**原 DNS ID 必须保持**（复用任务
  PRD 已确认）。
- `ExchangeContext`（单绝对 deadline）与 `ExchangeControl`
  （caller/owner 双取消 token）；取消优先于 deadline 的 tie-break 已固定。
- `SideEffectState::{NotSent,MaybeSent,Sent}` 是闭合枚举；connect/setup
  失败为 `NotSent`，已发送后的失败保留状态。这是"可否安全换新连接"的唯一依据。
- `Lifecycle`（`Open/Closing/Closed` + `register*`/`drain`/
  `commit_final_response`）是唯一的提交线性化点与排空门；新传输必须复用它，
  不得建第二套门。
- `#![forbid(unsafe_code)]`（`lib.rs:1`）约束本 crate；新增依赖本身可含
  `unsafe`（如 ring/quinn 一样），但 `rust/upstream-core` 自身代码不得引入。
- workspace MSRV `1.85`（`rust/Cargo.toml`）；任何新增依赖的 MSRV 必须
  `<= 1.85`，否则不可引入。

### secure foundation 的形态（本任务对齐它）

- `secure/dot.rs` `DotUpstream`：每次 exchange 新建一条连接，
  "numeric connect → 认证 TLS handshake → 才写查询"；无池化、无重试、无协议
  fallback。
- `secure/doh.rs` `DohUpstream`：每次新建连接 + 单次 HTTPS `GET`
  （HTTP/1.1 或 HTTP/2，ALPN 决定，无 fallback/replay）；出站 copy 置零 ID，
  `restore_request_id` 在返回前恢复原 ID。
- `secure/endpoint.rs` `ServerIdentity` 与 numeric dial **分离**；
  `DotEndpoint::new(dial, identity)`、`DohEndpoint::new(service_url, dial)`。
- `secure/tls.rs` `TlsPolicy`：verified（显式 roots）/
  `insecure_skip_verify`（显式 opt-in，永不自动降级）；**0-RTT early data 与
  session resumption 已显式关闭**（`:224-227`），后续启用需独立评审需求。
- `secure/mod.rs` 模块文档明确：pooling/reuse、resolver/bootstrap、HTTP/3、
  listener/host composition 在 secure slice 之外——其中 pooling 与 resolver
  已由后续任务完成，**HTTP/3 仍是 open 项，正是本任务**。

### resolver 侧的消费者边界（只读，不修改）

- `resolver/owner.rs` `ResolverComposition::{endpoint,dot_endpoint,doh_endpoint}`
  只接受一个 `PublishedTarget`；`PublishedTarget::dial()` 给出 numeric
  `SocketAddr`。resolver 已完成（含双栈），本任务只消费其只读快照。
- QUIC 任务需新增的组合入口（如 `doq_endpoint`）只读消费
  `PublishedTarget`，与既有 `dot/doh_endpoint` 同形。

### Go characterization（仅为取证，不是 Rust 规范）

- `pkg/upstream/transport/conn_quic.go` DoQ 客户端行为：
  - 查询前 `copyMsgWithLenHdr`（2 字节长度前缀，与 TCP 同形），**wire ID 置零**
    （RFC 9250 §4.2.1），写完调用 `stream.Close()` 发 STREAM FIN（RFC 9250 §4.2）；
  - 响应用 `dnsutils.ReadRawMsgFromTCP` 读（同 2 字节前缀 framing），读到后
    **恢复原 QID**；
  - 错误码：`_DOQ_NO_ERROR=0x0`、`_DOQ_INTERNAL_ERROR=0x1`、
    `_DOQ_REQUEST_CANCELLED=0x3`（RFC 9250 §4.3）；
  - `quicQueryTimeout = 6s` 写在 stream 上——这是 Go 实现层超时，不是产品契约。
- `pkg/upstream/upstream.go`：
  - `case "quic", "doq"`：`tlsConfig.NextProtos = ["doq"]`，QUIC dial；
  - `EnableHTTP3`（helper protocol `h3`）走 `http3.Transport` + `DialEarly`，
    即 DoH3；注释明确"There is no fallback"；
  - `newDefaultClientQuicConfig()`：小收发窗口（stream 4KiB / conn 8–64KiB）、
    `MaxIdleTimeout 30s`、`KeepAlivePeriod 25s`——实现层调优，不是契约；
  - `IdleTimeout` 默认"TCP/DoT 10s，DoH/DoH3/Quic 30s"（`:91`）——实现层数值。
- 已归档 upstream-foundation 兼容矩阵把连接复用/pipeline 判为
  implementation-only；同理，上述 Go 的超时/窗口/keepalive 数值**不得**写成
  Rust 契约。

### 协议事实（RFC 9250，取证为协议契约而非 Go 行为）

- §3：DoQ ALPN token 为 `doq`。
- §4.2：客户端必须在选定 stream 上发查询并以 STREAM FIN 表示写完；每个查询
  用独立的双向 stream，**stream 映射即无歧义关联**——DoQ 没有 TCP pipeline
  式的 demux 问题（复用任务 Deferred §10.1 的前提在此不成立，这是协议结构
  差异，不是政策例外）。
- §4.2.1：DoQ wire 的 DNS Message ID 必须为 0。
- §4.3：DoQ 错误码（NO_ERROR/INTERNAL_ERROR/REQUEST_CANCELLED 等）。
- DoH3 = RFC 9114 HTTP/3 上的 DoH GET：ALPN `h3`，请求编码沿用既有
  `DohEndpoint::get_request_target`（origin-form + `dns` 参数），语义与
  DoH/H1/H2 一致，只是底层传输换成 H3。

## Requirements

- R1. 在 `rust/upstream-core` 内新增 bounded one-shot QUIC 传输基础：
  **DoQ**（RFC 9250）与 **DoH3**（H3 上的 DoH GET）。每次 exchange 新建一条
  QUIC 连接、完成一次查询、关闭；不做连接复用/池化（复用任务的 serial owner
  是 TCP/TLS 形态，QUIC 多路复用是后续独立任务）。
- R2. **QUIC 栈选型必须先审计**：license（须与 GPL-3.0-only 兼容）、MSRV
  `<= 1.85`、edition 2024 可用性、resolved 依赖图（无 1.85 以上传递依赖）、
  特性裁剪（只开客户端需要的特性），以及 **TLS 栈对齐**：QUIC 栈的 rustls
  版本必须等于 workspace 的 `rustls 0.23`（`TlsPolicy` 由此构建配置），
  不一致则审计不通过。候选为 quinn 系（DoQ）+ h3 系（DoH3），但以 Slice 0
  的审计结论为准，不预设通过。
- R3. **服务身份与 numeric 拨号保持分离**：DoQ 的 `ServerIdentity`（SNI/证书
  校验对象）与 DoH3 的 service URL authority 必须与 numeric dial 地址分离，
  沿用 `DotEndpoint`/`DohEndpoint` 的构造形态；hostname 永不出现在 socket
  dial 路径上。
- R4. **TLS 策略沿用 `TlsPolicy`**：verified/insecure 语义不变；ALPN 精确
  提供——DoQ 只提供 `doq`，DoH3 只提供 `h3`；**0-RTT early data 与 session
  resumption 保持关闭**（`tls.rs:224-227` 现状），启用需独立任务。
- R5. **DoQ wire 与完成语义**：2 字节大端长度前缀；出站 copy 的 DNS ID 置零
  （RFC 9250 §4.2.1）；写完发 STREAM FIN；响应按同前缀 framing 读取后，
  **先校验 peer wire 的 ID 为 0**（非零 peer ID 是 `DOQ_PROTOCOL_ERROR`，
  终态，永不 commit）；成功路径还必须观测到 peer 响应侧 FIN 且恰好一个响应
  （缺 FIN 或多余第二个响应同样是 `DOQ_PROTOCOL_ERROR`，终态）。取消路径必须
  主动以 `DOQ_REQUEST_CANCELLED` 取消 stream 接收侧（RFC 9250 §4.3），而不只
  是返回本地 typed 错误。恢复 caller 原 ID 后，返回 `SecureResponse`
  （`transport == Doq`、`http_version == None`、
  `request_id == response_id == 原 ID`，见 design.md §3.1 冻结词汇表）。
- R6. **DoH3 完整 HTTP 契约（与既有 DoH 一致，只换传输）**：请求编码复用
  `DohEndpoint::get_request_target`（不得写第二套 `dns` 参数编码），且请求
  满足既有 DoH 请求契约（恰好一次 GET、无 body、无 User-Agent、无
  Content-Encoding、`Accept: application/dns-message`；`:authority`/path 沿用
  endpoint 访问器），请求发出后发发送侧 FIN；响应满足既有 DoH 响应契约
  （status 200、`application/dns-message`、identity 编码、head ≤ 16 KiB /
  ≤ 64 headers、完整有界 body ≤ 65535，否则为 `DohProtocol` 类 typed 错误）。
  H3 连接 driver 归属沿用既有 H2 child-tracking 模式（见 design.md §5.1）：
  driver 作为 exchange scope 的被追踪 child 运行、teardown 密封并排空、最终
  commit 在排空之后、无 detached 任务。
- R7. **绝对 deadline 与取消不被新传输改变**：每次 exchange 仍使用 caller 的
  原始 `ExchangeContext`；owner close → caller cancel → deadline 的优先级与
  `commit_final_response` 线性化点不变；`Lifecycle` 注册覆盖在途 exchange，
  `close()` 排空。
- R8. **失败分类闭合**：QUIC handshake/连接失败为 `NotSent`；stream 写出后的
  失败保留 `Sent`/`MaybeSent`；收到后已发送的失败是终态，不做通用重试、不做
  协议 fallback（DoQ 失败不回落 DoT/TCP，DoH3 失败不回落 DoH/H2——与 Go
  `EnableHTTP3` "no fallback" 注释一致，且是本任务的显式契约）。
- R9. **错误类型与既有并列不重叠**：新增 QUIC/DoQ/DoH3 错误变体与既有
  `UpstreamError`/`SecureError` 并列；QUIC stream 错误码到 typed 错误的映射
  必须显式（至少覆盖 NO_ERROR/INTERNAL_ERROR/PROTOCOL_ERROR/REQUEST_CANCELLED
  语义，其中 `DOQ_PROTOCOL_ERROR (0x2)` 覆盖非零 peer ID、缺响应 FIN、多余响应）。
  DoQ 与 DoH3 使用**各自**的错误码空间，不得互相重解释：DoQ 用 RFC 9250 §4.3
  的 `0x0`-`0x3`；DoH3 在 request/response stream 语境下按 RFC 9114 §8.1 的
  stream/connection context 做**显式逐码**判定（不是数值区间）：
  `H3_NO_ERROR (0x100)`/`H3_GENERAL_PROTOCOL_ERROR (0x101)`/
  `H3_INTERNAL_ERROR (0x102)`/`H3_REQUEST_CANCELLED (0x10c)` 各自映射到对应
  category；该语境下确实适用的已知 HTTP/3-family 码（`H3_STREAM_CREATION_ERROR
  0x103`，按已冻结评审契约保留；`H3_FRAME_UNEXPECTED 0x105`/
  `H3_FRAME_ERROR 0x106`/`H3_EXCESSIVE_LOAD 0x107`/
  `H3_REQUEST_REJECTED 0x10b`/`H3_REQUEST_INCOMPLETE 0x10d`/
  `H3_MESSAGE_ERROR 0x10e`/`H3_CONNECT_ERROR 0x10f`/
  `H3_VERSION_FALLBACK 0x110`；RFC 9204 §6 明确用于 request stream 的
  `QPACK_DECOMPRESSION_FAILED 0x200`）标为 `Other`；其余（RFC 9000 §20.1
  transport 低位码、保留 `0x1f * N + 0x21` grease 码、未知码，以及只定义在别的
  语境、在 request/response stream 上属于 unexpected context 的码：RFC 9114
  §8.1 的 `H3_CLOSED_CRITICAL_STREAM 0x104`/`H3_ID_ERROR 0x108`/
  `H3_SETTINGS_ERROR 0x109`/`H3_MISSING_SETTINGS 0x10a`，以及 RFC 9204
  `QPACK_ENCODER_STREAM_ERROR 0x201`/`QPACK_DECODER_STREAM_ERROR 0x202`）按
  RFC 9114 §8 的 MUST 视为等价于 `H3_NO_ERROR`，即低位 DoQ 码绝不映射成 H3
  protocol/cancel 错误。
- R10. 保持既有 `UdpTcpPolicy`、`ResolverComposition`（既有三个入口）、
  `Endpoint`/`DotEndpoint`/`DohEndpoint` 构造契约不变；QUIC 是新增的传输面，
  不改变单次 exchange 语义。结果词汇表按 design.md §3.1 加法冻结：
  `Transport` 新增 `Quic`、`SecureTransport` 新增 `Doq`/`Doh3`、
  `SecureHttpVersion` 新增 `Http3`；既有所有枚举臂含义不变，不得把 QUIC 结果
  标注为既有传输。

## Acceptance Criteria

- [ ] A1. 依赖审计：新增 QUIC 相关依赖的 license/MSRV/传递依赖图有书面审计
      记录（精确版本 + lock），MSRV 证据链与 secure 任务同等（`cargo metadata`
      审计 + Linux 稳定工具链构建；若仍无法实际安装运行 Rust 1.85，必须如
      secure 任务一样显式记录为间接证据，不得声称实测）。
- [ ] A2. 端点构造：DoQ endpoint（numeric dial + `ServerIdentity` 分离）与
      DoH3 对 `DohEndpoint` 的复用有 deterministic 构造测试；hostname 永不
      进入 dial 路径；零端口拒绝。
- [ ] A3. DoQ loopback：一次 fresh-connection DoQ exchange 成功——服务身份、
      peer wire ID 为 0（先校验，后恢复；非零 peer ID 是 `PROTOCOL_ERROR` 终态）、
      原 ID 恢复、2 字节前缀 framing、请求侧 STREAM FIN 与响应侧 peer FIN 均可
      观测（fixture 断言 FIN；恰好一个响应），返回 `SecureResponse`
      （`transport == Doq`、`http_version == None`），且只接受一条新连接。
      否定测试：缺响应 FIN、多余第二个响应均为 `PROTOCOL_ERROR` 终态，
      永不 commit。
- [ ] A4. DoH3 loopback：一次 fresh-connection DoH3 GET 成功——`:authority`/
      path 与 `DohEndpoint` 同值、请求字节与同输入的 DoH 路径同形（除 H3
      传输封装外）、请求发送侧 FIN 可观测、响应满足完整 DoH 契约（200 +
      `application/dns-message` + identity 编码 + 完整有界 body ≤ 65535）、
      原 ID 恢复，返回 `SecureResponse`（`transport == Doh3`、
      `http_version == Some(Http3)`）。否定测试：非 200、错 media-type、
      压缩编码、超大/不完整 body 均为 typed 协议错误；driver 在 close/cancel
      后无残留（注册计数归零）。
- [ ] A5. 错误分类：handshake 失败为 `NotSent`；写出后失败为终态无重试；
      无跨协议 fallback（DoQ↛DoT/TCP，DoH3↛DoH/H2）的否定测试；stream 错误码
      映射有直接测试（NO_ERROR/INTERNAL_ERROR/PROTOCOL_ERROR/REQUEST_CANCELLED，
      含 `0x2` 的协议错误语义）。
- [ ] A6. deadline/取消/close：复用 secure foundation 的语义——命中 deadline、
      caller cancel、owner close 均为 typed 错误且优先级不变；DoQ 在途取消必须
      主动以 `DOQ_REQUEST_CANCELLED` 取消 stream 接收侧（本地 typed 错误之外）；
      `close()` 排空在途 exchange；重复 close 收敛；无 late success。
- [ ] A7. 无回归：既有全部 UDP/TCP/DoT/DoH/resolver/reuse 契约测试继续通过。
- [ ] A8. Rust focused/full gates、`git diff --check`、task validate，以及隔离
      Debian VM 上的 Linux/Rust 1.85.x loopback/MSRV 证据通过，且选定 reviewer
      返回明确 scoped PASS 后才允许归档。

## In scope

- `rust/upstream-core` 内新增的 QUIC 传输模块（endpoint 构造、DoQ stream
  exchange、DoH3 GET driver）、ALPN 常量、wire 编解码 helper、typed 错误变体，
  以及 design.md §3.1 的加法结果词汇表扩展（`Transport::Quic`、
  `SecureTransport::Doq/Doh3`、`SecureHttpVersion::Http3`）。
- 精确裁剪的新依赖（QUIC 客户端 + H3）及其 license/MSRV/依赖图审计记录。
- 与既有 `Lifecycle`/`ExchangeContext`/`SideEffectState`/`ServerIdentity`/
  `TlsPolicy`/`DohEndpoint::get_request_target` 的集成（复用，不重写）。
- 面向 resolver `ResolutionSnapshot` 的只读消费边界（新增组合入口只读
  `PublishedTarget::dial()`），以及文档化的复用/连接迁移边界。

## Out of scope

- **本任务不是"把 QUIC 做完整"**。明确排除：
- QUIC 连接复用/池化、多路复用调优（优先级/流控）、连接迁移；
- 0-RTT early data、session resumption、client 证书（mTLS）、自定义 cipher；
- SOCKS/local bind/socket marks/源地址策略；
- UDP 侧重传策略变更（bootstrap UDP 契约不变）；
- server listeners（DoQ/DoH3 inbound）、inbound 处理；
- host/YAML/config loader/plugin/sequence 接线、API/WebUI、生产或默认选择、部署；
- Go/cgo/FFI/C ABI、`MOSDNS_*_BACKEND` selector、Go mirror 或任何 fallback 路径；
- 泛化的跨协议 fallback、连接失败后的跨地址/跨族竞速；
- 指标/审计/日志接线、性能基准/soak。

## Deferred（需要独立任务与评审，不在本任务授权内）

- QUIC 连接复用与多路复用（每连接多未决、stream 并发深度、H3 复用策略）——
  DoQ 的 stream 关联虽无歧义，但并发深度、backpressure、复用 key
  （dial+transport+identity+ALPN`doq`/`h3`）仍需独立设计。
- 0-RTT、session resumption、连接迁移——各需独立的安全/正确性评审。
- 连接失败后的跨地址重试与 Happy Eyeballs（用户已排除）。
- QUIC 传输的性能基准与长期运行/soak 证据。
- 把 QUIC 相关调优参数（idle/keepalive/窗口）提升为可配置项。

## Risks

- **依赖风险（最高）**：QUIC 栈体积大、传递依赖多、MSRV 可能高于 1.85。
  Slice 0 的审计是硬门：审计不通过则任务停在 Slice 0，不得"先写代码再补审计"。
- **安全风险**：ALPN 错配（把 `h3` 连接用于 DoQ 或反之）、跨身份复用（本任务
  无池化，从结构上排除，但 endpoint 构造仍需分离测试）。
- **ID 语义**：DoQ wire ID 必须为 0，但返回给 caller 的 wire 必须恢复原 ID；
  若恢复遗漏，下游 comparator 会把响应判为 mismatched。A3/A4 必须断言恢复。
- **把 Go 当规范**：Go 的 6s stream 超时、30s idle、25s keepalive、小窗口是
  implementation-only，不得写成 Rust 契约；本任务只对 RFC  wire/ALPN/FIN
  做协议级 preserve。
- **范围蔓延**：复用、迁移、0-RTT、listeners、fallback 都必须停在 Out of scope
  之外；DoH3 不得顺势引入 H3 通用客户端或 server 能力。

## Notes

- 本任务**planning-only**：`design.md` 与 `implement.md` 完成并经 reviewer 批准、
  且执行 `task.py start` 之前，不写任何实现代码，不改依赖。
- 依赖的既有任务已归档：`08-17-rust-phase4-upstream-foundation`、
  `09-16-rust-phase4-secure-upstream-foundation`、
  `09-17-rust-phase4-endpoint-resolution-foundation`、
  `09-18-rust-phase4-dual-stack-endpoint-selection`、
  `09-18-rust-phase4-connection-reuse-pipeline`。resolver 与复用部分**已完成**，
  本任务只读消费，不修改它们。
- 保留 worktree 中已有的无关 dirty 文件与 `.DS_Store`；不使用 `git add -A`；
  Trellis auto-commit 保持关闭。
