# Design — measurement reliability

## Ownership and data flow

允许修改 helper package `tests/phase5a-baseline/cmd/phase5a-baseline/`、其 README、新增窄范围 `scripts/run-phase5a-reliability.sh`、本任务 research 和新性能报告。沿用 workload parser、严格 DNS oracle、counter/TTL 与 PID/start 采样，必要时在同 package 拆文件。旧 baseline runner 不整体翻修，不写通用压测框架；Rust 产品和 Go 业务代码不在范围。

`reviewed execution plan → preflight → reference/calibration pilot → frozen official manifest → bounded sender/oracle → raw slot/process evidence → offline assessment → hotspot report`。后继工件引用前驱哈希；raw 与 derived 分离。新 reliability 子命令/schema 不使用旧 aggregator 的 all-success 有效性判据；旧入口保留历史行为。

## Sender contract

- stage start 单调时钟 + 整数 slot 编号生成计划，响应不推动发送；跳过时隙也有记录。并发 worker、dispatch 和 evidence queue 有界，队列满/写入失败明确归为 harness 状态。
- 保存 planned/dispatch/DNS-write-start/DNS-write-complete/finish offset。write-start 是客户端开始调用发送 API 的本进程单调时刻，write-complete 是完整 UDP payload / TCP 长度前缀和 DNS frame 已被发送 API 接受的时刻；不声称能观测真实网卡 wire start。部分写入/写入错误与字节数单列，未确认完整 frame 时不可计为 dns_sent。
- 主延迟为 planned-slot-to-finish；另报告 dispatch-to-finish（包含排队后连接成本）及 write-start-to-finish（发送 API 开始到响应完成），注明各样本起点与失败分母。TCP connect 不能从主延迟或 service budget 中扣除，连接前失败不能叫作 DNS 已发送。
- 每 slot 一个互斥终态：`planned = harness_skipped + harness_rejected + started`；`started = failed_before_dns_send + dns_sent`；`dns_sent = 所有互斥发送后终态之和`。额外收到的重复报文单列，不重复进入分母。旧 counters 的重叠语义不能直接套用新守恒规则。
- `goodput = correct_on_time / offered_stage_duration`，drain 不改变分母。成功样本分位数始终附失败比例与样本数，不能从大量失败中挑快样本。慢服务仍保持开环；负载器自身限制产生 invalid-load，不伪装容量。
- limits 以 offered-rate×deadline 为并发估算起点，覆盖连接、drain、日志与 FD，结合实际 RAM/磁盘冻结。两端采用相同限制、GOMAXPROCS、常规 GC、日志和采样条件，不继承旧补救实验的 GOGC=off。

### One absolute service deadline

`planned_at = stage_start_monotonic + slot_offset`；`service_deadline = planned_at + request_deadline`；`collection_deadline = service_deadline + late_drain`。wall-clock/UTC 仅作为诊断标签，不参与期限、排序、latency 或跨 host 因果推导。

排队、TCP connect、完整 DNS write 以及按期 read 都消耗同一个 service_deadline 的剩余预算，不允许阶段间重新给一个完整 timeout。dispatch/connect 之前及 write 之前检查余量；期限已经耗尽则记录 pre-send deadline expiry，不建立新连接/不发新 DNS。写入中跨过期限的竞争单列为 deadline/write race，保留实际字节与完成时刻，不将其当按期成功，不借 late_drain 继续发送。

只有已经确认完整发送的请求，才允许在 service_deadline 后继续收集响应至固定 collection_deadline；结果永远为 correct_late（或其他失败），不延长 service budget。正常 read 先以 service_deadline 为界，到期后显式切到已发送请求的 collection phase；这不是为 connect/write 重置预算。collection_deadline 后终态一次封闭，所有阶段结束后的 drain 也不改变 offered-duration 分母。

旧 exchangeTCP 的 DialTimeout 后重新 SetDeadline 仅保留给 legacy；新入口不能直接复用它的 timeout 计算。Slice 1 使用可控网络/时钟边界验证 queue+connect+write/read 共用余量、禁止过期新发、collection 只收已发送迟到证据，以及 wall-clock 跳变不影响单调结果。

### Blocked evidence sink

worker 不同步等待日志落盘。evidence owner 有界队列、可取消的 write 边界和独立控制状态；sink 持续不消费且不返回错误时，达到队列/等待预算便进入 harness failure，后续时隙按编号区间明确封闭为 rejected/skipped，`load_valid=false`，不能无声压低 offered rate。

已启动/排队/未生成时隙的终态及 planned accounting 必须守恒，control ledger/counters 有独立的有界保存路径，不能将无限未写 payload 留在内存。原始 journal 缺失的记录数和区间显式保存；若最终无法完整持久化，`evidence_valid=false`，绝不借控制计数伪造完整原始 journal 或给正式 PASS。

测试 sink 不失败但永久暂停消费，直到队列满；取消必须在冻结退出预算内收回 worker/evidence owner，无需测试放开 sink 才退出。writer 边界要实际支持 cancel/close 唤醒；不可取消的文件 I/O 不能藏在必须同步 join 的 worker 路径中。真实边界无法在预算内回收时，记录 cleanup failure 并阻塞验收，不假装已退出。

## Assessment contract

每 stage 独立给出 evidence_valid、load_valid、dns_correctness、service_budget、overload、recovery。证据身份/账本/采样完整才可判有效；完整施压且 lag/fixture 校准合格才可评价服务退化。

有效负载下的 late/timeout/可归因于 SUT 的 transport failure 是有效退化数据，降低 goodput 与 service_budget；sender 临时端口耗尽、连接创建不足或 fixture 饱和另归因，无法区分就不给 SUT 过载结论。wrong-response/串包是 correctness failure，立即停止验收。

默认方法：连续两个等长窗口违反冻结按期率或尾延迟预算才称过载，CPU 高只辅助解释。回落到同参考 QPS 后，原 PID/start 连续三个窗口恢复预算且资源回到冻结范围，记录首次及持续恢复时间；重启、超时未恢复、资源未回收分别报告。没有前序客观过载就保持 indeterminate。窗口长度与采样条件在候选前冻结。

## Pilot and official freeze

Slice 0 先冻结 execution-plan 的 pilot QPS/次数/时长上限、official 次数、安全预算、停止条件和 cleanup 方法。技术起点为 W1 单点 30 秒、W2 单点 10 秒，deadline 500ms、drain 100ms、W1 窗口 5 秒；这些默认值须在环境盘点后正式写入计划并审查，不能直接执行模板。

W2 每点独立预热；prefill→最后响应须小于原 30 秒 TTL 减 deadline/安全余量，不通过修改 TTL 适配长扫描。不给 W2 同进程恢复结论。

正式每场景二至四个共同负载点，至少包含低负载参考与最重有效/受限点，每点三次 Go/Rust 交错；pilot 不计入配对。W1 每个候选重复是一条同进程 ladder，不能为各负载点重启后再拼成恢复序列；W2 的独立预热按点运行。发送器需要在至少 1.25× 最高正式 offered rate 的 fixture-direct 校准中满足冻结 lag/发送预算，并证明 fixture 余量；不足则在 Rust official 之前降低目标/重新冻结，或报告环境不足。

W1 资格另要求使用同一 fresh-TCP 策略、相同目的端点布局与实际 client 网络栈，在 fixture-direct 校准中覆盖不短于一条正式 ladder（含回落窗口）的总持续时间、不少于该 ladder 的累计 fresh connections，同时覆盖上面的峰值速率余量。记录 ephemeral-port 范围、开始/期间/结束的 TIME_WAIT 和连接计数、reuse 设置、dial errno、端口/FD/连接余量、重复之间冷却/基线恢复方式。

完整 calibration envelope 及安全 margin 写入官方 manifest，G2 不能仅看一个瞬时 1.25× QPS 点。若使用定量等价证明代替直接全时长校准，必须展示端口范围/reuse/TIME_WAIT 生命周期与峰值/总量/重复压力的推导，并经 G2 人工审查；更改 source IP/目的端点、连接复用或 kernel reuse 设置都使资格失效，不能靠扩大端口池掩盖正式连接策略变化。

只使用 Go reference pilot 与采样误差确定正常延迟/按期率/资源带及恢复期限，数值/公式进入 official manifest，避免微秒量级噪声让判据随机失败。官方 hash 覆盖 exact sources/binaries/tools/config/corpus、环境/affinity/GC、QPS/limits/window、audit、warm 生命周期、三次顺序、停止和 retry。修订必须保留旧 manifest 与全部 attempt，不跨 revision 混合。正式重跑最多一次。

## Environment and profiling

先盘点 `mosdns-rust` 的实际 Linux amd64、cpuset/cgroup quota/steal、CPU、RAM、FD、端口、磁盘、工具和权限。优先 SUT、sender、fixture/采样隔离；2-vCPU 只有全部校准通过才给受限单核对照。拓扑不足就停止；新增机器/扩容另行确认，不启动本机 VM。

正式矩阵不夹 profiler。另对每 scenario/candidate 的参考点与最重有效点做一次 profile，保存 exact binary hash、符号、事件/频率、丢样、原始栈和导出，报告扰动。

沿用 /proc 并补必要 per-thread CPU/调度、FD/连接、采样缺口；100Hz tick 的粗分辨率不能当零 CPU。优先进程定向 Linux perf；PMU 不可用先审查软件事件与栈能否满足归因，不自动降低全局权限，不引入 nft/eBPF。仅总 CPU/RSS 不足以完成 hotspot 验收。CPU top stack 无法单独证明 I/O 等待，结论明确区分实证、假设和未知。

报告至少区分 sender 调度、日志、分配/锁、SUT 执行、TCP connect/I/O、cache 与 fixture/network。若支持 current-thread 主瓶颈，另规划 Send/所有权/取消/快照迁移及收益验证；没有该证据则优先推进 5B，不能机械把 Rc 换 Arc。

## Durability and rollback

维护入口放稳定 tests/scripts 路径，research 保存 frozen recipe/identity。仓库根由明确 markers 定位；历史工具用指定 commit 和记录的 repo-relative path 获取 Git 对象，不从当前 archive 路径猜历史目录，不固定 parents[N]、不静默 fallback。

raw/manifest/sidecar 不覆盖；derived 输出到全新目录。最终测试覆盖任务归档形态及历史路径不同、缺失/歧义/篡改拒绝。大文件可放 durable artifact，仓库保留完整索引/哈希/可获得路径及存在证明，不能只指 /tmp。

cleanup 核对本次 owned PID/start，包括 SUT、fixture、sender、sampler、profiler；ESRCH 只在确有已退出身份回执时接受。rollback 是停测量、自身进程退出、保留证据，不改产品/生产。预算耗尽、错包或安全上限触发就停，不通过重跑改写结论。
