# Cache planning review — 2026-10-01

本记录核查用户转交的其它对话意见，不是实施批准。没有派发 agent、修改产品源码或执行构建。

早期 review/建议按历史保留；2026-10-01 的最新批准与已收敛条款以 PRD/design 为准，不把下文历史“待选择”当当前门禁。

## Review disposition

1. **P1 多缓存发布边界：成立。** `config.rs:407,789` 是单实例配置；`execution.rs:1167` 是单个 PendingStore；`sequence-core/src/engine.rs:449–463` 明确拒绝第二个 watch。只换成 CacheId 集合不能解决嵌套。规划须支持 A miss → B miss → forward，先完成 B，再完成 A，各自捕获所属 successor 的结果；A hit 不执行 B。每次 dispatch 有独立 publication frame，同一缓存重复出现不能由请求级 seen-ID 集合静默跳过。重复调用的具体 boundary token/同 scope 完成顺序必须在 design 冻结，失败、取消不发布不完整结果，循环受共同有限 fuel 约束。不同 fallback sibling 的 frame 相互独立。
2. **P1 lazy owner 与控制预算：成立，产品选择已于 2026-10-01 确认。** 采用 owner-managed refresh：复制 cache dispatch 时的状态，在设置 stale 响应之前形成 successor；只运行到其 enclosing boundary。使用 cache owner cancellation tree、独立有限 root fuel 与 5 秒预算；嵌套 fallback/preference 共用该 refresh root 的 fuel，不重新获得预算。不得继承终止的 client root，也不得重新进行 client audit/ranking admission 或写回封存 facts。关闭顺序为停止 admission → cancel/join refresh → 最终 dump → closed。同 key 合并、每 owner 256 并发、不排队。每实际 refresh root 的 fuel 固定为 DEFAULT_FUEL=64，来源 execution.rs:33；内部 fork 共用此 root。
3. **P1 key 兼容：成立；不能无条件声称全量双向兼容。** Go `cache.go:1068` 是 flags/QTYPE/文本 qname/可选 ECS；native `cache.rs:234–300` 是 IN-only、无 additional、wire 名及私有前缀。推荐统一逻辑 key 到产品模型，而不是保留私有模型后做两套长期 codec。必须定义 flags AD/CD/DO、文本名大小写/转义/尾点、支持的 class 以及 EDNS/ECS 范围。采用 Go wire schema 不等于兼容所有旧 dump；不能表示的条目必须有公开策略，不能静默丢 ECS 后成为普通查询缓存。范围未冻结前 A3 仅是目标，不是可执行验收。
4. **P1 时间/domain_set：成立。** host `cache.rs:14–41` 是 elapsed epoch，`:222` 存空 domain_set。持久化使用 Unix timestamps，运行期过期继续单调判断；导入时用一次 wall/mono snapshot 换算剩余寿命与 TTL age，避免每次 lookup 随系统时间回跳。对未来 stored time、溢出及过期条目须冻结 reject/skip 分类，使用注入双时钟测试。保存所属 successor completion 的 routing.domain_set；缓存命中非空值覆盖当前 domain_set、空值保留现有值，与 Go `cache.go:420,451,476` 一致。不要凭一个 protobuf 字段宣称已恢复审计行为。
5. **P1 inventory/metrics：成立。** 当前 `api.rs` 没有 /metrics，plugin action 只识别 show/save/post (`:766`)；Vue `DataManagementManager.vue:238` 按既有四个指标解析，缓存列表使用固定标签。推荐新增窄 `GET /api/v1/cache/inventory`，返回 `{schema_version:1,caches:[{tag}]}`，仅列 named cache，tag 为唯一管理身份。Vue 支持该响应时使用实际 inventory；仅明确 unsupported/404 时退回旧 Go 列表，超时/500/坏 schema 显示加载失败，不能退回伪零数据。新增 /metrics 的缓存指标子集，不据此宣称全套原生 Prometheus 完成；正确转义 tag。路由不得遮蔽其它插件同名 action。
6. **P1/P2 named/quick/default/path：成立。** Go `Args.init` (`cache.go:171`) 使用 size=1024/dump_interval=600；`utils/config_helper.go:34` 对 <=0 取默认。quick 每编译 callsite 独立，生命周期受 host 管理，无 named API、dump 或公开 tag metrics。named 可管理。相对 dump 路径必须有明确基准；native 已有 `compile_yaml_with_base` (`config.rs:478–494`) 和 RawPlugin.base_dir (`:1166`)，推荐与其它插件文件路径一致，基于声明该插件的配置文件目录，含 include 场景；不能照搬 Go 全进程 Chdir。这个差异须在最终兼容摘要列明。
7. **P2 persistence failure matrix：成立，但durable-first 建议已于 2026-10-01 单独批准。** Go `cache.go:809–830` 先清内存后 dump；写失败可在重启恢复旧数据。推荐 native 有 dump_file 的 flush 在 generation transaction 下先原子替换 durable empty dump，再清内存/失效旧 publication；失败保留原内存与旧文件并 500。这是已单独批准的 intentional deviation，不是 Go flush 顺序兼容。交易期间如何处理 lookup、miss publication、import、periodic snapshot 必须写入 design，不能只锁文件写入。所有旧 generation publication 包括前台 miss 和 lazy refresh 都要被 gate 拦截。

## Proposed failure matrix

| 操作 | 成功 | 失败/特殊情况 |
| --- | --- | --- |
| startup load | 全量校验后 merge，过滤 fully expired | 文件不存在空缓存启动；坏文件报告错误、空缓存启动；不部分导入 |
| periodic dump | 同目录临时文件完整写入后原子替换 | 旧文件不变，保留 dirty generation 下一周期重试 |
| shutdown | 停刷新、join 后保存最终 snapshot | 报告保存失败、旧文件保留；具体 host exit contract 在 design 冻结 |
| GET save | 同 periodic 的写入安全性 | 无 dump_file 400，I/O 500 |
| POST load_dump | 完整验证后 merge，碰撞键替换；generation barrier | 格式/上限/不可转换条目策略须冻结；坏 payload 400，原缓存不变 |
| GET flush（无 dump_file） | 清内存并阻断旧 generation 发布 | 无磁盘操作 |
| GET flush（有 dump_file，已批准变更） | durable empty dump 成功后清内存并阻断旧 generation | 替换前失败 500，保持原状；post-commit invariant failure fail-closed |

## Body/visibility contracts and required tests

- show 保留 Vue 解析依赖的 `----- Cache Entry -----`、`Key:`、`DomainSet:`、时间行、`DNS Message:`；分页和搜索基于同一有效条目集合，不能只冻结 HTTP status。
- show/dump/size_current 都按当前时间过滤 fully expired；Lazy window 条目仍可见，不能直接把 NativeCache.len 当 live size。
- 可观察切片：嵌套 miss 双发布、外层 hit 短路、重复 cache、fallback sibling；lazy query 已结束且刷新仍受限/可关闭；flush 与旧 miss/refresh race；Go/native eligible key fixture 与明确不可转换条目；wall rollback/重启 lazy window；miss/hit/restart 的最终 HTTP domain_set 一致；实际 inventory、metrics、Vue partial clear。
- 公开测试边界：compile API、sequence execution public machine、host DNS listener、HTTP routes、dump reader/writer、现有 Vue 页面。只注入时钟、受控上游与文件 I/O 故障；不 mock cache publication、generation gate 或 codec 本身。

## Remaining gate

owner-managed lazy 产品选择已关闭（完整条件见 PRD R2）；继续完成 key/EDNS/ECS 范围、时间异常分类、默认/路径兼容摘要及 flush deviation 冻结，再写 design/implement。任务保持 Planning NOT READY。其它对话说“其余都不需要产品问题”不能替代用户对明确兼容行为改变的最终批准。

## Quick-cache clarification

Go quickSetupCache (`cache.go:221`) 只有 size 参数，lazy TTL 缺省 0。quick 与 named 共用 refresh 生命周期实现，不代表给 quick 发明新的 lazy 参数或默认开启 lazy；quick 所属 owner 仍参与 stop/cancel/join，且没有公开 inventory/API/dump/tag metrics。

## Approved flush decision — 2026-10-01

用户已单独批准 durable-first；无 dump_file 仅 gate 下切 generation/清内存，有文件时先准备并原子替换 empty dump，再切 generation/清内存。先完成可失败资源准备，post-rename commit 不允许 recoverable failure。异常内存 invariant failure fail-closed，禁止最终 dump 写回旧内存。成功“为空”描述的是 transaction commit 点，而不是禁止以后缓存新请求。完整异常边界见 design §4。

## EDNS/ECS scope proposal — awaiting product choice

`rust/dns-core/src/edns.rs:62–70` 已暴露 do_bit 和 ecs，`query.rs:101` 支持最多一个 additional；不代表 native cache 已使用这些字段。Go `getECSClient` (`cache.go:1057`) 读取 qCtx.QOpt() 的首个 ECS，并以 Go option.String() 编码 key suffix；native 解析结构不自动保证与这一字符串格式一致。

推荐本轮包含基础 EDNS0/DO 缓存（AD/CD/DO 独立维度、response OPT 安全处理），暂不增加 ECS 缓存隔离与 key-string 兼容：enable_ecs=true 明确拒绝配置；含 ECS 查询绕过 cache，仍走既有 forward；带 ECS suffix 的 dump 完整拒绝，不部分导入，也不删 suffix。这样双向 dump 声明只覆盖支持的 IN、无 ECS 产品 key。用户若选择同时完成 ECS 隔离，须扩充 ECS option 字符串兼容、key、dump 与刷新状态复制矩阵，不能仅给 parser 添加字段。

这个限定是范围建议，尚未批准；不能以“已有 parser”为理由宣布兼容矩阵收敛。

## Convergence — 2026-10-01

用户已选择基础 EDNS0/DO、暂缓 ECS 缓存隔离；含 ECS 查询 bypass cache，enable_ecs=true 配置拒绝，ECS-suffix dump 全量拒绝。design §5 已给出 key、时间异常、导入限额及配置矩阵，§6 route/body/inventory 矩阵，§7 shutdown exit 2。PRD/design/implement 已收敛，等待最终整体审批；未运行 task.py start。

TDD 切片和公开接口/模拟边界见 implement S1–S7。审查覆盖原复核七点及 show/expired-size 两个附加点。期间只修改本任务规划文件，不构建、不改产品源码、不 dispatch agent。
