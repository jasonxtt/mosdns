# Cache lifecycle design — review draft

状态：冻结设计已全部实现；S1–S7 已通过同一 C2C 对话复审，S7 已完成，C2C iteration 11 FINAL: PASS / DONE。用户已授权本任务全部切片。

## 1. Components and ownership

- `CacheCatalog` 保存 compile-time CacheId、callsite ExecutableId → CacheId 映射及 named tag → CacheId 管理映射。named 引用同一实例；quick 按 callsite 创建实例，不合成公开 tag。不扩展 sequence External 为任意配置载荷。
- `CacheOwner` 保存唯一 NativeCache、选项、generation、dirty revision、刷新 cancellation root、same-key task registry、256 non-blocking permits、刷新 join/drain 集合以及持久化状态。named/quick 用同一 owner 类型。禁止第二份 shadow cache 或 Go mirror。
- `PublicationFrame` 保存 request-local frame token、CacheId、generation、key、scope identity。不是 CacheId 去重集合。每次 dispatch 建一帧，完成恰好一次；不同 branch 的帧不能消费父帧。
- `RefreshTask` 拥有 dispatch 前状态副本、successor continuation recipe、generation、key、permit、owner-child cancellation 与独立 root control。drop guard 回收 singleflight slot/permit；shutdown 必须 join，不能只有 abort handle。

## 2. Nested publication boundaries

sequence-core 当前 watch 只允许一项。将其扩展为 typed watch registration token + LIFO completion notifications；每一项绑定真实 scope instance，不能只绑定 executable。同 scope 多帧离开时逆登记顺序完成，各自 consume token 后才继续调用者下一条规则。旧单-watch 接口可作为 wrapper 保留，避免重开其它调用者契约。

A miss → B miss → forward：forward 正常完成到 scope boundary 后 B 发布，再 A 发布。不同 enclosing scope 各在自己的离开点捕获 state，不能统一等 root 结束后存同一个最终 wire。A hit 以现有 cache short-circuit 语义终止其 successor，不执行 B。重复 A dispatch 可以建不同 frame；不能静默跳过第二项。fuel 限制递归和循环。

terminal error、cancel、fuel exhaustion、异常 drop 必须使相关 frame 失效；普通 accept/exit 是否为成功按现有 sequence completion classification，不能将 Runtime error 因残存 response 当成功。fallback losers 取消不会发布，正常已完成 branch 的合法 publication 保留其真实完成语义，不将 sibling state 混入。

## 3. Refresh capture, budget and facts

Lazy lookup 后先捕获原 dispatch 状态与 successor recipe，再设置 TTL=5 stale response 并返回。刷新只运行 dispatch 所在 enclosing scope 的余下规则。

现有 `fork_successor` 经过 `fork_child` 继承 client RootFuelHandle，不能直接用于 detached refresh。提供显式 capture/rebind seam：保存可拥有的 continuation recipe，启动时绑定 CacheOwner cancellation、`RootFuelHandle::new(64)`、`now+5s` 绝对 deadline。所有内部 fork 分享这个 root。运行编译 program 的 owner snapshot 必须比 task 活得更久；recipe 不借用 request stack，不使用 lifetime transmute 或 unsafe self-referential machine。可在 task 内持有编译 snapshot 并据 recipe 构建借用它的 machine。

same key 已在 registry 时不执行、不获取 permit；不同 key permit 满时直接跳过。先登记任务与 permit 再执行 I/O；失败/取消/drop 均 exactly-once 清理。owner admission 一旦停止不能再启动任务。client/socket cancellation 不向已 admission 的 task 传播。

refresh 禁止触发其它 detached lazy refresh：它执行途中再遇到 lazy cache 时按 inline miss 运行其 successor，共享当前 refresh root/deadline，不新建 root/permit；旧值保留到合法新值 publish。不能把内层 stale 当 fresh 重新存到外层以延寿。Fresh hit 仍可短路。前台不同 owner 的 lazy admission 仍彼此独立。

刷新使用独立执行 facts sink，不重新调用 client query admission/audit/ranking/duration 路径。configured upstream attempt 如现有 metrics 要求计数则通过 upstream-work sink 统计，不能改原 query supplier/attempts。audit-off 热路径保留 copyable IDs，不能因为刷新总是生成 identity strings。

publish 必须原子验证：successor 正常完成；response admission 有效；owner Running；generation 未变化；token 尚未消费；deadline 未过且 owner cancellation 未触发。验证失败不更改旧值。成功更新 dirty revision。旧值自然到 cache expiration 失效。

## 4. Generation and persistence transactions

generation 是 publication epoch；dirty revision 是内容版本，两者职责不同。miss/refresh capture generation，commit 在 owner gate 下检查。flush/import 的线性化点改变 generation；旧请求即使有新 wire 也不得回填。generation/revision 溢出必须报错或停止 owner，不能 wrap 造成误匹配。

持久化操作每 owner 串行。snapshot 捕获 revision；成功写入只将该 revision 标 clean，较新的修改保持 dirty。周期失败保留 dirty 并重试。

已批准 durable-first flush：准备完整 empty dump → 在事务中替换文件 → 以不可失败的内存操作清 store 并切 generation → 返回成功。事务期间旧 publication 被阻断/延迟并在提交后判失效；读请求可看到旧 snapshot，不允许看到成功清理后又复活。替换前失败解除事务并恢复允许原 generation 发布，内存、旧 generation 与旧文件不变。需避免持有 current-thread 借用跨 await，文件 I/O 放专用阻塞任务；状态 transition 明确，取消 API 请求不撤销已进入最终 commit 的 owner transaction。

无 dump_file 的 flush 在同一 owner gate 内推进 generation、清 store，无文件操作。两条路径都先完成所有可失败的准备（generation/revision checked increment、空 store 状态/替换资源），文件替换之后的内存 commit 不分配、不 await、不返回 recoverable error。正常 safe-code 路径应使“文件成功、内存清理失败”不可达；若 panic/invariant violation 仍发生，owner/host fail-closed，不再服务、不返回成功，不用旧内存执行 shutdown dump；保留已替换空 dump 并报告 fatal failure。

原子 rename 是外部可见 commit 点；不能把 rename 成功后的额外 fallible 操作错误描述成“500 且旧文件不变”。本合同针对运行/重启的原子一致性，不宣称任意断电/文件系统故障下的绝对 durability；若要增加 rename 后目录 fsync，其失败属于 post-commit fatal/indeterminate，不能普通回滚。成功 flush 保证 commit 点 memory 与 persisted snapshot 均为空；返回之后新请求允许正常填充。

import 先完整解析、校验、构建 bounded staging，失败无修改；随后一次 gate commit 切 generation、merge（同 key 替换），标 dirty。管理 import 不自动写 disk，与 save 分开。旧 generation tasks 可被 cancel 加速回收，但 generation check 是不可省略的正确性边界。

## 5. Codec, time and domain_set

统一 key 到 Go 产品布局：flags byte + QTYPE big-endian + 文本 qname 字节长度 + 文本 qname；仅支持 IN，不把其它 class 映射成 IN。qname 保留原查询大小写、尾点，按 miekg/dns 文本 escaping 输出特殊标签字节，普通标签不 lowercase；长度必须能在一个 byte 内精确表达，超过 255 文本字节的名字绕过缓存、不截断。dump key 必须与 response question 的名称/QTYPE 对应，并校验 flags 未知位；AD/CD/DO 分离。以大小写、尾点、转义、边界长度 Go golden fixtures 验证该合同，不将 Go 长度截断的意外行为作为实现目标。

已批准范围：单问题 IN 查询无 additional，或只有一个合法 EDNS0 OPT，可缓存；DO 独立 key，OPT advertised UDP size 不作 key。ECS option 查询绕过 cache，enable_ecs=true 配置错误，含 ECS suffix 的 dump 完整拒绝（manual 400，startup 报错后空缓存启动）。未知/非零 EDNS version、扩展 RCODE、不支持 additional/option shape 绕过缓存而不修改既有转发行为；非 ECS 不改变答案语义的未知 EDNS options 可被验证后忽略，不能把未验证包当基础 EDNS0。

cached response 按旧 cache 产品契约去除 OPT，不把 OPT TTL 当 DNS TTL；客户端 miss 的原 response 不因缓存入库被修改。压缩指针需要结构安全解析/重建，不能简单删除中间字节或截断尾部导致指针错位。仅已验证单问题、opcode=0、非 TC、与查询匹配的 wire 入库。exclude_ip 接受 scalar 空格分隔或字符串列表，非法 CIDR 报警跳过（Go 明确行为），合法网段命中任一 answer A/AAAA 则不发布；domain_set 不作为 key。NXDOMAIN 保留 30 秒、SERVFAIL 5 秒；NOERROR message TTL=minimal TTL，空 answer 上限 300 秒，非正数取 5 秒；启用 lazy 时 NOERROR cache retention 为从存储时刻起的 lazy_cache_ttl（不是 message expiry 之后再加）。其它已支持 RCODE 沿现有 adapter 的 5 秒保留，不扩展 wire 支持集合。

gzip Name=mosdns_cache_v2，protobuf 字段号保持现有 dump.proto；每 block 8 字节 big-endian 长度、上限 1MiB，完整 gzip footer/checksum 验证。总限额冻结：compressed 输入 16MiB、decoded stream/staging 64MiB、entries 100000，任一超限完整拒绝。字节预算包含 key/wire/domain_set 的 owned staging 及 decoded framing，count 另限；流式解码并限制分配，不先无限解压。load_dump 使用 route-specific 16MiB body cap，其它 API 保持 api.rs:248 的 1MiB。每 block 1MiB；坏 payload/超限在 merge 前拒绝，手动 400。导出也必须满足相同可导入限额，超限返回错误不写出不可重新加载的文件，旧文件不变。

运行期时间继续单调；存储保留 wall 存储时间供 dump。restart 一次读取 wall/mono snapshot，按 wall 算 TTL age/remaining，再换算单调 expiry。fully expired 条目跳过；message expired 但 lazy retention 未过可恢复 Lazy。时间策略：stored/msg-exp/cache-exp 必须为非负可表达 Unix 秒，expires >= stored，checked arithmetic 不溢出；stored > 当前 wall snapshot 视为 future/rollback-invalid，完整拒绝该 dump，不能夹成新鲜值或扩 TTL。manual 400 且原缓存不变，startup 报错后空缓存启动。cache-exp <= now 跳过；msg-exp <= now < cache-exp 恢复 Lazy（owner lazy=0 则跳过），其它条目恢复 Fresh。运行中 wall 后退不影响已入库单调 expiry；dump 用条目原始 wall 时间戳，不每次重算 TTL 来延寿。重启时无法辨别合理 wall 校正与陈旧未来条目，选择拒绝而不猜寿命，此为明确更严格的 import 行为。

publication 捕获该 frame successor completion 的 routing.domain_set，存入 core。hit 恢复非空值覆盖当前 routing.domain_set，空值不清除当前值；域匹配/标签其它状态不从 dump 臆造。DNS/HTTP/audit 的 miss-hit-restart 测试验证最终结果。

相对 dump_file 沿用 RawPlugin.base_dir，即声明插件的文件目录（include 亦如此），绝对路径原样。没有隐式 Chdir、mkdir 或 fallback 路径。named/quick size 缺省或 <=0 取 1024；named dump_interval 缺省或 <=0 取 600 秒；lazy_cache_ttl 缺省 0、负值配置错误，超出表示范围配置错误；dump_interval 无 dump_file 时接受但无周期写盘。enable_ecs 缺省/false 可用，true 明确报错；未知选项不静默忽略。不同 named cache 解析到同一 dump 路径则配置报错，避免多个 owner 覆盖同一文件；不引入跨进程文件锁。

## 6. HTTP and Vue contracts

管理 catalog 只列 named cache。新增 `GET /api/v1/cache/inventory` → `{schema_version:1,caches:[{tag}]}`，配置顺序稳定。quick 无 API/dump/public tag metrics。

`GET /metrics` 输出四个既有 cache 指标：query_total/hit_total/lazy_hit_total/size_current；hit_total 包含 lazy hit，query_total 是进入该 cache 的前台 dispatch 数，不是后台 refresh 数。host client 请求统计也不能增加。tag 按 Prometheus 规则转义。size/show/dump 都基于当前未 fully-expired 集合，不能 raw len。

既有管理方法：GET flush/dump/save/show，POST load_dump。save 未配置文件 400，I/O 500；load_dump 完整校验错误 400。show 保留 Cache Entry/Key/DomainSet/三个时间行/DNS Message 文本与 Vue 解析格式；q 搜索和 offset/limit 使用有效条目集合。路由矩阵：未知 tag、未挂载 action、非 cache tag 的 cache-only action 返回 404（先于 method 判定）；有效 named cache action 用错 method 返回 405；show/save 等已存在其它插件 action 按其原 mount dispatch，不被 cache 路由覆盖。show limit 缺省或 <=0 为 100，offset<0 为 0；q 为 case-insensitive key/answer 文本搜索，按快照稳定 key 字节顺序分页，保留旧 body shape，不依赖必有 X-Total-Count。

Vue 成功读 inventory 才使用真实 named cache；404/明确 unsupported 可回退旧 Go hardcoded 列表，500/超时/未知版本显示加载错误。不通过 metrics 猜测配置，不伪造不存在 cache 的零值。批量清理使用 settled results 展示成功/失败标签；各 cache transaction 彼此独立，不声称全局 atomic flush。

## 7. Shutdown state machine

Running → StoppingAdmission → Draining → Persisting → Closed。先停止外部新请求/管理修改，再停止 refresh admission；cancel 所有 owner task，join/drain 后作最终 snapshot。只有在全部 publication 无法再发生时才保存。final save 失败保留旧文件并报告；host 关闭聚合每个 owner 的错误，但继续完成其它 owner drain/save；任一 final save 失败最终 assembly.run 返回 Err，使现有 main.rs 路径退出码 2。不能吞错或假装保存成功。

故障回退只针对本任务新增路径，不引入 Go runtime fallback；保持其它插件和现有 upstream lifetime 合同。
