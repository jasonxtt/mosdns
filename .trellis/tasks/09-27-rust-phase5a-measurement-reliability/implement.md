# Implementation plan — execution record

用户已在 2026-09-27 明确批准按本版规划执行全部已冻结 slice；任务已通过 `task.py start` 进入 `in_progress`。本任务采用 inline 流程；同一 C2C 对话按 slice 审查执行证据，不派发 implement/check agent。所有远端测量继续受环境门禁约束。

## Slice 0 — 实验契约与环境门禁（A1/A3/A5）

- [x] 核对 branch=rust、dirty baseline、三份规划及 backend specs，auto-commit=false，精确 staging。
- [x] 按 research/source-audit.md 核实旧方法、复用边界、新 schema、planned accounting、timeout 起点及分类。
- [x] 实现授权之后才只读 SSH 盘点 mosdns-rust：CPU/quota/affinity/steal、RAM/FD/port/disk、版本和 profiler 权限。不扩容、不动生产。
- [x] execution-plan 先冻结 pilot/official/profile 的数量、时长、资源安全上限与停止/cleanup 预算；实际 offered 点和技术阈值仍须 Go pilot 后冻结。
- [x] 固定 Go-only 来源 `5b1eca69e0668ad1ddb6db88c0f39202557d5b98`、candidate source baseline、workload/tool 输入哈希；新 binary/release 参数哈希留到 Slice 2 manifest，禁止复用历史 binary hash。
- [x] 写发送有效性、技术预算形成公式、客观过载/恢复契约；没有候选数据参与标准制定。
- [x] **G0：人工审查方法/环境/预算**完成；profiler remediation 已安装并由 `002reviewer` `FINAL: PASS` 接受 software-event fallback，尚未产生热点或容量结论。

第一个阶段性小目标是 G0：获得可审查的实验契约，不追求容量数字。

初始 G0 C2C review 结果：iteration 1 的 P2-1（缺少可复放的 exact command/raw
preflight transcript）已在 Slice 0 内修复并于 iteration 2 返回 `FINAL: PASS`。
该 PASS 只放行已授权的离线/loopback Slice 1 helper/state-machine 工作；当时远端
没有 `perf` 等进程级 profiler，不能由 G1 或 helper 绿灯解锁
Slice 2 校准、official Go/Rust 测量、热点或容量结论。详见
`research/c2c-slice0-review.md`、`research/slice0-preflight-transcript.md`。

后续用户已授权仅对 `mosdns-rust` 安装 profiler。已安装
`linux-perf`/`libc6-dbg`，并以 `cpu-clock` 软件事件成功生成有调用链的
process-directed smoke profile；硬件 cycles/instructions 仍为零，且未降低
`perf_event_paranoid`。新增 remediation transcript，且 `002reviewer` 已确认
该软件事件路径满足当前设计的 fallback；正式 profile 仍须独立运行并披露
PMU 限制，不把 smoke profile 当作 MosDNS 热点结论。

## Slice 1 — 有界发送器和离线状态机（A1/A2/A3）

C2C iteration 4 returned `STATE: PLAN` for this unit. The finite plan is
recorded in `research/c2c-slice1-plan.md`; implementation remains bounded to
the helper/CLI, loopback tests, and task evidence. The initial Slice 1 review
was written before the later profiler remediation; that remediation is now
accepted by `002reviewer`, while Slice 2+ remains gated by its own review steps.

- [x] 一行为一 slice 红→绿：完整 slot、调度停顿、慢服务、限额/队列满、writer 失败、各 latency 起点、连接前失败、互斥 outcomes 与守恒。mock 只限时间/网络/文件等系统边界。
- [x] 统一期限红→绿：queue、connect、write/read 逐段消耗同一个 planned-at 绝对 service deadline；connect 消耗后 read 只获得剩余预算；过期不能新发；late-drain 只收已完整发送响应；wall-clock 跳变不改结果；partial write / deadline-write race 可见。
- [x] blocked-sink 红→绿：sink 停止消费但不报错，直至队列满；发送/worker 不无限等待，后续 slot 明确 rejected/skipped，control accounting 守恒，缺原始 journal 时 evidence_valid=false，load_valid=false；取消在冻结预算内完成，不依赖 sink 恢复消费。
- [x] 新 reliability 子命令与 schema，复用严格 DNS oracle，保留旧入口；不扩大为通用负载平台。
- [x] CLI/离线测试：有效负载 timeout 是有效退化，harness 错误是 invalid-load，wrong DNS 是 correctness failure；过载→同进程恢复、无过载→indeterminate、restart→不能判恢复。
- [x] 实际 loopback 慢/拒绝/失联测试，验证退出/端口回收；新增完整复验及 archive/path/hash 负例。
- [x] **G1：helper 全套、必要 race、脚本检查绿**；C2C iteration 10 返回 `FINAL: PASS`，关闭 Slice 1。G0 profiler capability gate 已由 remediation review 关闭；Slice 2+ 仍须按本任务的 G2/G3 门禁推进。

Iteration 9 C2C remediation is implemented locally and awaits re-review:
explicit write-start/write-complete offsets and write-deadline races; frozen
window on-time/resource/identity criteria; an internally cancellable evidence
owner with factual oversize compaction and raw-before-close-error CLI output;
successful-response-only percentile samples; explicit rejection of
transport-encoded shell scenario aliases; raw contiguous/equal window sequence
facts with frozen reference QPS; complete missing-slot reconciliation for
non-error queue overflow; an independent wall-clock jump case; and strict
reference-before-overload/recovery-phase gating. Focused tests are green, but
G1 stays unchecked until C2C returns a final verdict. The missing
process-directed profiler was a sticky G0 blocker at that historical point. The
later `slice0-profiler-remediation` review removed that blocker by accepting
the software-event fallback with its PMU limitation.

## Slice 2 — 校准与官方冻结（A2/A3/A4/A5）

- [ ] 在 G0/G1 后执行有限 fixture-direct 校准；sender 至少覆盖最高 official 点 1.25×，sender/fixture 余量、lag 和限制均有数据。
- [ ] W1 校准覆盖整条 fresh-TCP ladder 的累计时长/连接数及峰值，盘点 ephemeral range、TIME_WAIT/reuse、errno、重复冷却及余量；直接全时长或 reviewed 定量等价证明进入 frozen manifest。不能仅凭峰值 QPS 通过 G2。
- [ ] Go-only reference pilot 建立预算/误差带；核实计时、deadline、W1 fresh TCP、W2 per-key TTL 与零增量。
- [ ] W1 UDP/W3 correctness 回归；W3 不以跨 host 墙钟生成新容量证明，旧冻结文件不改。
- [ ] 正式 manifest 冻结 exact 身份、共同负载点、三次交错顺序、时长/窗口/阈值、warm 生命周期、限制、环境、停止/重跑规则；pilot 独立存档。
- [ ] **G2：人工审查 manifest**。发送器校准失败退回，不先跑候选再放宽标准。

## Slice 3 — 对照与热点报告（A4/A5）

- [ ] approved isolated 目录/端口执行一次官方矩阵；全部 attempts 保留，不选最好的三次。
- [ ] W1 增载/回落同 PID/start；W2 每点独立预热，不声称恢复。每官方点三次有效 pair，否则 A4 未通过。
- [ ] 保存 DNS outcomes/goodput、planned-slot-to-finish 主延迟、dispatch-to-finish、write-start-to-finish 三个 latency 视图与 lag，以及 thread CPU/RSS/FD/连接、counter 和预算；服务失败保留。schema/raw/report 中各视图的起点、样本数、失败分母和数值均能从 slot offsets 离线复算；未到 dispatch/write 的请求不伪造该视图样本，但仍进入 planned/失败分母。
- [ ] 独立 profile runs 保存栈/符号/丢样/原始命令，正式结果不混入采样扰动。
- [ ] 有必要只允许一次 reviewed revision 的 official 重跑，旧失败完整保留，不混合 revision。错包/资源上限立刻停。
- [ ] `docs/rust/phase5a-measurement-reliability.md` 写有效/无效点、下界/过载/恢复、热点及证据强度，给 runtime/Send 或 5B 的下一任务建议。
- [ ] **G3：结果范围审查**。没找到过载可诚实收口有限报告，但缺有效 pair/profile 不能整体 PASS。

## Slice 4 — 完整审查与可复验收口（A6）

- [ ] 全部 raw/index/sidecar hash 验证；fresh derived 复算，结果一致。完整 CLI 和归档形态验证，不只调内部 verify。
- [ ] owned PID/start 回执确认 SUT/fixture/sender/sampler/profiler 退出、端口可 rebind，失败日志不删除。
- [ ] trellis-check full-scope review、A1–A6 和未决项明确；reviewer 未指定，不能预写 review PASS。
- [ ] 同步主线/交接/有限报告，不将完整 feature coverage 升级为已完成；必要新知识再更新 spec，不回写历史证据。
- [ ] 验收通过后按届时授权执行工作提交 → finish/archive → journal；归档后再离线 sanity。未通过不能 completed。

## Existing validation commands

拟定公共边界为 helper `reliability-run`（新 sender/账本）和 `reliability-assess`（离线判定）子命令，以及 `scripts/run-phase5a-reliability.sh`；更名须保持行为并同步规划。具体 flag/schema 在 G0 冻结，不能只写私有函数单测。

| 行为 slice | 主要公共验证边界 | 可 mock 的系统边界 |
| --- | --- | --- |
| slot/accounting/慢服务/限额 | reliability-run 的 stage/slot 输出及退出状态，实际 loopback 集成 | 单调/墙钟、网络 Dial/Write/Read、受控 DNS peer |
| 单一期限/late collection | reliability-run 完整 CLI + 系统边界注入的 segment 延迟测试 | 时钟与网络；不得 mock outcome 或 dispatcher 内部顺序 |
| blocked sink/writer error | reliability-run 的有限退出、control ledger、queue 预算/缺证据状态 | 持久化 writer sink，可取消阻塞；不 mock queue/accounting 结果 |
| overload/recovery/restart | reliability-assess 对完整 synthetic raw bundle 的 verdict/理由 | 文件系统、PID/start/时钟采样输入；不 mock状态机 |
| identity/hash/archive | reliability-assess 完整入口，对归档布局/指定 Git object 的正负例 | 临时文件系统/throwaway Git repository |

每个行为先一个正确原因的 red case，再最小实现、green/refactor；CLI 命令与真实 loopback 不被内部函数断言替代。

```sh
go test ./tests/phase5a-baseline/cmd/phase5a-baseline
go test -race ./tests/phase5a-baseline/cmd/phase5a-baseline
bash -n scripts/run-phase5a-baseline.sh
# 新 runner 存在后
bash -n scripts/run-phase5a-reliability.sh
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host
# G3 后完整 workspace regression
cargo test --manifest-path rust/Cargo.toml --workspace
python3 .trellis/scripts/task.py validate .trellis/tasks/09-27-rust-phase5a-measurement-reliability
```

新 CLI 与 Linux 执行命令在 G0–G2 补全，不能把以上回归当容量证据。Go race 如受开发环境限制应在 Linux 跑绿，不删除测试。G3 后跑一次完整 Rust workspace regression，未改产品时无需每个小 slice 重跑全套。新增 Python 测试从实际任务目录全跑并禁 pycache；旧观测 benchmark 不重跑。context validation 对两个大型 spec 提示自动注入会截断，本任务 inline 执行须直接完整读取相关 spec，不依赖截断注入，不修改全局上限。

保存每 slice red/green、G0–G3 审查、schema/manifest/identity/commands、pilot/official/profile 索引、raw hash、attempt 状态、cleanup 及 fresh derivation 对照。大原始文件使用 durable artifact，不能只保留临时路径。

## Stop / rollback

G0 不足仅完成方法/盘点；G1 失败停在 helper；G2 校准失败不产生候选结论；G3 错包/预算耗尽退出自身进程保留数据，A4/A5 未过；归档复验失败修离线工具，不重跑压测掩盖问题。
