# Rust Phase 4 native dual-stack endpoint selection

## Goal

为未来 Rust-native host 建立纯 Rust 的多地址 endpoint resolution foundation：
在同一个 caller-owned deadline 和取消/关闭模型下获取 A 与 AAAA 候选，优先
选择 A、在没有可用 A 时使用 AAAA，交付一个可拨号地址，同时保留 TLS SNI、
DoH authority/path 等服务身份。不做 Happy Eyeballs 或双连接竞速；现有单族
解析仍须兼容，生产 host/config/API/WebUI 接线不属于本任务。

## Confirmed Facts

- `rust/dns-core::AddressFamily` 当前只有 `Ipv4` 与 `Ipv6`；resolver wire
  codec 一次只构造并解析一个 RR family。
- 已归档的 resolver foundation 将 `ConfigVersion::from_u8(0 | 4 | 6)`
  固定为 `0/4 -> IPv4/A`、`6 -> IPv6/AAAA`，未知值 typed-reject；数值
  `dial_addr` 不经过 DNS。
- 当前 `BootstrapResolver`、publication state、single-flight generation 和
  TTL 元数据均按一个解析目标/一个地址建模；`ResolverComposition` 把一个
  numeric dial address 交给现有 UDP/TCP/DoT/DoH constructors。
- bootstrap peer 必须是 numeric UDP endpoint，且其传输族与目标答案族独立；
  hostname resolution 使用 caller-owned runtime、absolute deadline 和 typed
  cancellation/close，不使用系统 resolver 或隐藏 runtime。
- Go `pkg/upstream/bootstrap` 目前只查询一个 family，并明确保留
  `0,4 -> A`、`6 -> AAAA`；Go 代码将 dual-stack 标为 TODO。它是兼容性参考，
  不是 Rust 内部实现模板。
- `bootstrap_version` 只选择 DNS answer QTYPE，不选择 bootstrap peer 的传输
  family：IPv6-only numeric bootstrap 可以通过 IPv6 UDP 查询 A（`0/4`），也
  可以查询 AAAA（`6`）。这要求本机具备到该 IPv6 peer 的路由；若目标域名
  没有所选族的记录，失败原因是 answer-family 不匹配/无地址，而不是
  bootstrap peer 本身不可达。
- 具体兼容性场景：`addr: tls://dns.example.com:853`、`bootstrap:
  223.5.5.5`、`bootstrap_version: 0` 时，若 `dns.example.com` 只有
  `AAAA 2001:2::...` 而没有 A，正常 DNS 响应不会提供可用 A，bootstrap
  无法生成 TLS 拨号地址，上游不可用；双栈策略必须覆盖“单族查询失败、另一族
  有地址”的情况。
- 用户已确定本任务不做连接竞速：`bootstrap_version=0` 的新 Rust foundation
  语义是独立获取 A 与 AAAA 候选，A 可用时优先 A，仅有 AAAA 时使用 AAAA；
  `4` 继续只选 A，`6` 继续只选 AAAA。这个变化只在本任务的纯 Rust contract
  中定义，不能未经 host/config 迁移评审接入现有 Go 默认路径。
- 用户已确定缺省配置值为 `4`：未提供版本时等价于 A-only；显式配置 `0` 才
  进入 A+AAAA 收集模式。Rust 配置边界必须能区分 omitted/`None` 与显式 `0`，
  不能把缺省值和 dual-stack 入口都压成整数零。
- 本任务继承前序任务中仍相关的覆盖项：A+AAAA 查询、地址排序、按族失败记忆、
  多地址 cache shape、QUIC/HTTP3 交互边界，以及 `bootstrap_version` 兼容性；
  Happy Eyeballs 和连接竞速已由用户明确排除。

## Requirements

- R1. 保留数值目标的零 DNS/零 bootstrap 流量 fast path；缺省值与 `4` 都是
  A-only，显式 `6` 是 AAAA-only，显式 `0` 才进入 A+AAAA 收集模式。
- R2. 对 hostname 目标在一个 caller-owned absolute deadline 内独立获取 A 与
  AAAA 候选；不建立两条目标连接，不做 Happy Eyeballs、连接失败后的跨族
  fallback 或协议 fallback，不依赖 OS/system resolver 的隐含排序。每个
  bootstrap 查询继承相同的取消、owner close、ID 随机性、TC/rcode/
  response-correlation 和 no-hidden-runtime 契约。
- R3. 定义可测试的地址选择策略：新鲜可用 A 始终优先；没有可用 A 时选择
  新鲜可用 AAAA；同族多个地址的顺序稳定且显式。解析失败、取消和 deadline
  tie 的优先级必须可观察。
- R4. 用多地址 publication/cache shape 保留每个候选的 family、address、
  TTL/expiry 和 generation 信息；单个地址失败不得破坏仍然新鲜的其他候选，
  过期候选不得作为 fresh success 返回。
- R5. 记录按族、按候选或按 generation 可区分的失败记忆，避免一个 IPv4
  失败把可用 IPv6 永久屏蔽，也避免旧 generation 的晚到结果污染新状态。
- R6. 交付的成功结果仍只包含 numeric dial destination；DoT service identity
  与 DoH URL authority/path 必须保持原始配置，解析层不新增 YAML/plugin/API/
  WebUI/生产选择逻辑。
- R7. 明确 QUIC/HTTP3 的消费边界：本任务至少定义多地址结果如何交给未来
  secure transport、如何共享 deadline/cancellation，以及哪些连接/协议竞速
  留给独立 QUIC/HTTP3 任务；不得借本任务引入 QUIC 实现或连接池。

## Acceptance Criteria

- [ ] A1. omitted/缺省与 `4` 都映射为 A-only；显式 `0` 明确表示 A+AAAA、A
      优先、无可用 A 时选择 AAAA；`6` 仍为 AAAA-only，未知值仍 typed-reject。
- [ ] A2. A 与 AAAA 的 lookup 行为有 deterministic tests，覆盖仅 A、仅 AAAA、
      双成功、单族失败后另一族成功、两族均失败、取消和 absolute-deadline。
- [ ] A3. 地址选择测试证明 A-preferred 与稳定的同族排序，不建立双目标连接、
      不做连接失败后的跨族重试，不依赖 wall-clock sleeps 或系统 DNS。
- [ ] A4. 多地址 publication/cache 测试覆盖独立 TTL、部分过期、按族失败记忆、
      generation replacement、single-flight 和 close/drain。
- [ ] A5. Numeric target 与既有 `4/6` 单族 contract tests 全部保持通过，并为
      显式 `0` 更新/新增双族 contract；DoT SNI、DoH authority/path 和 plain
      numeric endpoint 身份分离不回归。
- [ ] A6. QUIC/HTTP3 boundary contract 与 deferred matrix 已写清，代码不引入
      QUIC/HTTP3、host/config/API/WebUI 或 production wiring。
- [ ] A7. Rust workspace focused/full checks、isolated Debian VM loopback/MSRV
      evidence 和选定 reviewer 的明确 PASS 完成后才允许归档。

## Out of Scope

- YAML/config loader、Go/cgo/FFI、backend selector/fallback、host/plugin/sequence
  ownership、API/WebUI、默认生产切换、监听器和部署。
- QUIC/HTTP3/DoQ 的协议实现、连接池/reuse/pipeline、proxy/socket policy。
- 将新的显式 `0` 语义接入现有 Go 默认路径、YAML/API/WebUI 或生产选择；这些
  需要独立迁移/兼容证据。

## Open Questions

无。用户已确定本任务只做解析阶段的候选收集与 A-preferred 选择，不提供
连接失败后的跨族 fallback。

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
