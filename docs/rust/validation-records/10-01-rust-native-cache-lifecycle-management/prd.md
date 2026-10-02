> Historical product planning or evidence snapshot. Lifecycle status alone is not acceptance PASS. See [validation summary](../../validation-summary.md). Raw logs and local workflow state are retained outside Git tracking.

# Rust-native cache lifecycle and management workflow

## Goal

在一个任务内打通 Rust-native 缓存的数据面、生命周期、持久化和现有 Vue 管理流程，减少分散小任务之间的重复接线。用户已于 2026-10-01 授权实施；S1（catalog/config）与 S2（nested publication）已实现并通过独立 review，S3–S5 已实现并通过同一 C2C 对话 review；S6 已通过 C2C iteration 10 review，S7 已完成，C2C iteration 11 FINAL: PASS / DONE。

## Background and evidence

- 前置 fallback/address-preference 任务已经归档；当前 HEAD 为 `368daef0`。其真实组合验证记录位于 `../archive/2026-10/10-01-rust-native-query-fallback-address-preference/research/real-combination-evidence.md`，记录分组测试通过和磁盘不足的历史失败，不把后者宣称为通过。
- `rust/native-host/src/config.rs:1525` 只接受明确 size 与 lazy_cache_ttl=0；当前 assembly/execution 使用单缓存。core 已支持 Fresh/Lazy 状态，但 host 尚未开放 lazy 生命周期。
- `rust/native-host/src/cache.rs:234` 目前只缓存无 additional、IN-class 查询，key 为私有原生格式；时钟使用私有单调 epoch。兼容 dump 必须解决 key 与 Unix 时间转换，不能只套旧文件头。
- `plugin/executable/cache/cache.go:515` 的 lazy 回源按 key 合并，最多 256 个并发、5 秒独立预算，查询返回后继续执行；Go 的未管理 goroutine 和镜像不是 Rust 的实现规范。
- Go 管理契约见 `plugin/executable/cache/cache.go:667` 起，dump codec 见 `:907` 起；Vue 指标解析见 `webui-log/src/components/DataManagementManager.vue:238`。当前缓存页依赖固定标签和 special_groups 派生标签，不能假设它已支持任意原生 cache catalog。

## Requirements

R2 生命周期与 R4 durable-first flush 已由用户于 2026-10-01 分别确认；EDNS/DO 范围及 ECS 延期也已确认；整体任务已获实施授权；2026-10-02 用户明确要求接续修复，并依次实施全部剩余 slice，每个 slice 在同一 C2C 对话复审。

- R1 多实例：多个 named cache 独立 store、容量及公开统计；quick 每编译 callsite 独立，受 host 管理，但无 named API、dump 或公开 tag metrics。支持 A miss → B miss → forward 的嵌套 publication frame，B/A 分别保存所属 successor 结果；A hit 不执行 B。重复 cache dispatch 不得由 seen-ID 集合静默跳过；相同 scope 的独立 token 逆登记顺序完成；fallback sibling 独立边界见 design。默认 size=1024、dump_interval=600，缺省/<=0 使用默认，lazy_cache_ttl 缺省0/负值拒绝。
- R2 Lazy（已选择）：cache 判断 Lazy 后返回 TTL=5 旧值；刷新使用 dispatch 当时、写 stale response 前复制的执行状态，只运行 successor 到 enclosing boundary。刷新属于对应 CacheOwner，client 响应完成、断开或对象销毁不取消已 admission 的刷新；owner/host cancellation、deadline 和 generation invalidation 始终有效。每个实际刷新建立独立 root：DEFAULT_FUEL=64（execution.rs:33）、绝对 5 秒 deadline；所有嵌套执行共用这一个预算。same-key follower 不新建执行、不占额外 permit；每 owner 最多 256 并发，non-blocking、无队列，满载保留旧值供以后重试。只有正常完成、可缓存 wire、owner 未停止/关闭、generation 匹配、deadline 未过和 publication token 有效时才能发布；否则丢弃结果并保留旧 lazy value 到 cache expiration。停止刷新 admission → cancel → join/drain → 最终 snapshot → closed。刷新不进行 client admission、不增加 client query_total/latency/rank/duration sample、不新增或覆盖原 audit；真实上游工作统计与 client supplier 分离。named/quick 共用 owner/refresh 机制，但 quick 既有 `cache [size]` 语法没有 lazy 参数，默认 lazy=0，不新创语法或擅自打开 lazy。
- R3 持久化：原生实现 `mosdns_cache_v2` gzip/protobuf 格式；双向兼容仅针对明确冻结的 key/flags/class/EDNS/ECS 支持矩阵，禁止通过丢弃 ECS 等 key 维度制造命中。逻辑 key 采用 Go 产品布局，保留名称大小写/尾点/escaping。持久化 Unix 时间与运行期单调时间显式换算；所属 successor completion 捕获 domain_set，hit 非空值覆盖当前值、空值保留当前值。完整校验后 merge 导入，非法数据不得部分发布；原子文件替换，失败保留旧文件。startup 缺文件或坏文件报告后继续，periodic 失败保留 dirty generation 重试。时间异常和导入上限按 design §5：未来 stored time/溢出/坏条目完整拒绝，fully expired 跳过；输入16MiB/decoded staging64MiB/100000 entries 限制。关闭保存失败完成所有 owner 收敛后退出码2。
- R4 管理接口：现有 `/plugins/{tag}/flush`、`dump`、`save`、`load_dump`、`show` 的 method、响应及搜索分页兼容，包括 Vue 解析依赖的 show 文本标记。show/dump/size 过滤 fully expired 条目。flush/import 必须阻断旧 generation 的前台 miss 和后台刷新发布。有 dump_file 的 flush 采用已批准 durable-first：先成功原子替换 empty dump，再以不可失败内存操作推进 generation/清缓存；替换前持久化失败返回 500，保持原缓存和 generation。无 dump_file 时仅推进 generation/清缓存。成功提交点保证运行期与文件 snapshot 均为空，之后新 generation 的合法请求仍可填充。异常内存提交失败 fail-closed，不能返回成功或再以旧内存覆盖空 dump。这是明确偏离旧 Go 顺序，不称完全兼容 Go flush。相对 dump 路径基于声明插件的配置文件目录，含 include 场景，同一路径的不同 cache 配置拒绝。
- R5 指标与 Vue：新增 /metrics 的缓存指标子集，复用 query_total/hit_total/lazy_hit_total/size_current 的既有名称和 tag。新增 `GET /api/v1/cache/inventory`，schema_version:1，caches:[{tag}]，只列 named cache。Vue 使用实际 inventory；明确 unsupported/404 才回退旧 Go 列表，其它失败显示加载错误。查看条目、搜索分页和清理按真实 inventory 工作，批量清理报告部分失败，不宣称完整 Prometheus 迁移。
- R6 兼容范围（已选择）：支持基础 EDNS0/DO，AD/CD/DO 独立 key，入库 response 安全去 OPT；ECS 查询绕过缓存正常转发，enable_ecs=true 明确配置报错，ECS dump 完整拒绝导入。exclude_ip 支持旧 scalar/list 及报警跳过非法 CIDR。支持的 wire/key/time/default 矩阵见 design §5。暂不宣称全量 cache/ECS/Prometheus 或迁移最终门禁完成。

## Acceptance criteria

- A1 验证 A/B 双 miss 后分别存值、A hit 短路、重复 cache dispatch、fallback sibling 独立发布；quick 无公开管理身份，单缓存旧场景回归。
- A2 用可控时钟和上游验证 fresh、lazy TTL、过期 miss；同 key 合并、不同 key 限流、失败不破坏旧值、关闭无 owner pending work；查询审计保持最终一致。
- A3 Go/native 双向 dump fixtures 按冻结的支持矩阵验证 DNS、key、时间和 domain_set；不可转换条目明确处置。覆盖损坏、超限、过期/lazy window、wall rollback/overflow 和 I/O 失败；miss/hit/restart 的最终 HTTP/audit domain_set 一致。
- A4 HTTP body/search/pagination/flush/save/load 与实际 DNS 命中一致；flush/import 期间旧 miss/refresh 不复活条目；保存失败、清理失败及重启结果符合批准后的 failure matrix。
- A5 实际 Vue 页面证明多实例统计、条目查看和清理；重启后真实 DNS 命中与 dump 一致。
- A6 在 mosdns-rust 隔离目录完成必要 Rust/UI 检查及真实 loopback 证明；保留失败与限制证据，不在本机构建，不触碰生产或公共 DNS。

## Out of scope

- 生产切换、hybrid 脚手架退休、Go mirror/新 ABI 扩展。
- 全量插件迁移、special_groups 完整实现、ECS handler 等其它 DNS 策略插件。
- 新缓存管理产品界面；优先打通现有页面，新增控件须明确范围。
- 不相关脏改动、push、生产部署或无关目录清理。

## Initial planning and first-round history (superseded by current progress)

- Lazy 生命周期、durable-first flush、基础 EDNS0/DO 与 ECS 延期已分别确认；实施授权由用户在
  2026-10-01 的会话消息中直接给出（单执行者、不派发实现 agent，reviewer 以完成报告中的精确
  diff 范围交付）。`task.json.meta.automation_required` 因此置为 `false`，并附原因说明。
- design.md 与 implement.md 已补齐 ownership、publication、时间/限额/失败矩阵、公开测试接口、mock 边界和可独立验证切片，依赖 S1→S7 明示；维持一个任务，不拆 child、不派发 agent、不重开 C2C。
- 首轮实施只完成 S1（catalog/config）与 S2（nested publication）；S3–S7 未实施，差异、证据与
  临时 fail-closed 行为见 `research/implementation-evidence.md`。
  兼容差异清单（flush 顺序、配置目录相对路径、ECS 拒绝、未来时间/超限 dump 拒绝、关闭保存失败
  退出码2）中，本任务已实的只有 ECS 拒绝（`enable_ecs: true` 报错）。「配置目录相对路径」在
  `dump_file` 被拒绝后只剩错误信息中的路径解析，不再算已交付；其余属于未实施的 S3–S5，不能声称完成。
- 临时 fail-closed 拒绝项：正值 `lazy_cache_ttl`（需 S4 的 refresh owner）、`dump_file`（需 S5
  的持久化）、`exclude_ip` 已在 S3 实现答案过滤，不再临时拒绝。三者都先按原产品语义完成形状校验再拒绝，
  以免把原本会显式失败的配置变成静默无效；`dump_interval` 在没有 dump 目标时仍接受（与参考实现
  一致），`size`/`lazy_cache_ttl`/`dump_interval`/`enable_ecs` 的显式 null 视为缺省。
