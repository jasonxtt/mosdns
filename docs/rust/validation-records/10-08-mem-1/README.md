# 10-08 Mem-1：原生内存测量、分配器与重建残留的验证记录

任务 `10-08-rust-native-mem-1-allocator-rebuild`（基线 `7d973d2b`）。构建、测试、测量全部在 VM `mosdns-rust`
（Linux 7.0.9 xanmod x86_64，2 vCPU，7.9 GB，glibc 2.41，THP `always`、`khugepaged/max_ptes_none` 511）上进行，
release 构建。生产快照（`prod-config-20261006`）只在仓库外使用。

## 测量方法

`scripts/measure-rule-memory.py`：由快照的 `sub_config/rule_set.yaml`（去掉 `rewrite`）与 `adguard.yaml` 生成配置，
加一个执行 `unified_matcher1` 并按 mark 返回不同 rcode 的 sequence、一个 UDP listener 与 API（`unified_matcher1`
148,412 条唯一规则）。流程：启动 → DNS 可答 → 5 s 后读 `VmRSS`（启动稳态）与 `VmHWM`（启动峰值）→ 6 次：
`clear_refs=5` 重置 HWM，`POST /plugins/blocklist/post`（生产列表 + 1 条新 `full:` 探针），等探针被答为 NXDOMAIN
（新一代已服务），5 s 后读数 → 再等 30 s（或 300 s）读数 → 对 `top_domains.txt` 全部 10,486 个名字各查一次，
取 `name rcode` 的 sha256。`--qps 200` 时另有一个单并发查询客户端全程测延迟。单位 kB。

## 结果（每格为 3 次运行的范围）

| | 启动稳态 | 启动峰值 | 每次重建峰值 | 6 次重建后稳态 | 空闲 300 s 后 | 重建耗时 |
| --- | --- | --- | --- | --- | --- | --- |
| 基线 `7d973d2b`（glibc 默认） | 114,212–116,276 | 201,064–204,292 | 310,504–374,884 | 363,972–374,884 | 374,980 | 0.7–0.9 s |
| **Mem-1** | **36,596–37,088** | **95,040–95,204** | **120,584–132,816** | **39,452–39,868** | **39,932** | **0.30–0.46 s** |
| Go v0.7.3（同快照、同脚本） | 121,412–143,940 | 同左 | 157,012–193,960 | 170,828–174,288 | 113,480–116,524 | 1.4–1.8 s |

R4：6 次重建后稳态 / 启动稳态 = 1.07–1.08（目标 ≤ 1.2）。启动稳态 −68%，启动峰值 −53%，重建峰值约 −60%
（基线的“重建峰值”含已滞留的上一轮内存）。重建期间新旧两代共存不可避免：峰值 ≈ 稳态进程（含仍在服务的旧代
matcher ≈ 16 MiB）+ 规则文本 ≈ 8 MiB + `ValuedRule` 批 33 MiB + 构建临时数据与新 matcher ≈ 44 MiB。

DNS 结果：基线与 Mem-1 所有运行的答案摘要相同（`8521e609…1670`）。Go 与原生只差 1 个名字：大小写混合的查询名，
其小写形式是 `full:` 规则——即 2026-10-07 已批准的偏差（`full:` 大小写不敏感），基线相同，与本任务无关。

## 改动与逐项前后对比

| 改动 | 单独效果（同脚本） |
| --- | --- |
| 启动后与每次重建发布后在构建线程 `malloc_trim(0)` | 启动稳态 114→34–36 MB，重建后 365→37–46 MB；峰值不变 |
| `ValuedDomainMatcher::build` 临时数据借用规则串、结果原地继承（输出不变） | 构建临时数据 130→44 MiB，启动峰值 201→109 MB，重建峰值 ~235→140–162 MB，重建 0.85→0.3 s；但 glibc 动态 mmap 阈值下重建后有 10–30 MB 碎片波动 |
| 固定 glibc mmap 阈值 128 KiB（`mallopt`） | 消除上项碎片：重建后稳定 39–40 MB；启动峰值 109→96 MB |
| 进程关闭 THP（`PR_SET_THP_DISABLE`） | THP `always` 时 `khugepaged` 会把已释放页所在的 2 MiB 区重新填满：空闲 300 s 后 41→89.5 MB；关闭后保持 39.9 MB（生产机同为 `always`/511） |
| 被替换的 mapper 与 AdGuard 直接匹配器在 DNS 线程外（blocking 线程）释放 | DNS 线程上释放旧代耗时 11–15 ms，曾使最大延迟升到 15 ms；移出后最大延迟 7.2–7.8 ms（基线 6.8–9.1 ms） |

评估后未采用：`MALLOC_ARENA_MAX=1`（重建后 281 MB；与固定阈值合用无额外收益，且构建线程会与 DNS 线程争同一 arena 锁）、
jemalloc 5.3（`LD_PRELOAD`，重建后 130–151 MB，30 s 后 132 MB）、mimalloc 0.1.52（全局分配器实验构建，启动 219 MB）。
未引入新依赖：`rustix`（已有、版本不变）增加 `thread` feature；`Cargo.lock` 不变。glibc 调用只在 `linux`+`gnu` 下
编译，是 `mosdns-native-host` 唯一的 `unsafe`（crate 级 `forbid` 改为 `deny`，仅 `memory.rs` 放行）。

延迟（200 qps，单并发 Python 客户端，诊断数据）：p99 基线 454–605 µs、Mem-1 582–629 µs，在客户端的运行间波动内；
最大值见上表；无超时。

## 内存去向（计数分配器，请求字节）

编译后存活堆 22.4 MiB（前后相同），几乎全是 `unified_matcher1`：domain trie 21.1 MiB（140,134 条 `domain:`，
约 150 B/条：56 B 节点 + 每个标签一个堆 `String` + 子表；其中约 6.4 MiB 为未触及的 `Vec` 容量，不驻留），`full:`
1.0 MiB。编译峰值 169.5 → 86.3 MiB。基线启动 RSS 114 MB 而存活堆只有 22 MiB：约 80 MB 是 glibc 未归还的编译临时内存。
明细与 Mem-2 候选见任务 `research/memory-breakdown.md`。

**Mem-2 建议范围**：紧凑 domain trie（标签共享字符串池、子节点紧凑表、结果用池索引代替每节点 `Arc`；matcher
21 → 约 5–7 MiB，稳态约 −13 MB）+ 按 provider 分组的构建输入（代替每条规则 4 次分配的 `ValuedRule`；峰值约
−30 MiB），顺带在重建中移交 provider 规则串（−7 MiB）。

## 测试与 check

| 命令（`rust/`，VM） | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --workspace --no-fail-fast` | 1364 passed，0 failed，4 ignored |

matcher 重写的等价性：用生产 provider 构建 `unified_matcher1`，对 934,198 个名字（每条规则模式及其变体、父级、
`top_domains.txt`）的匹配结果取摘要，基线与新实现相同（`dff3d4e6…0463`）。

`mosdns check -d <root> -c <root>/config_custom.yaml`（`file` 仓库 `config_all` `28c6493` 与生产快照，
`config_custom.yaml` 相同）：基线与 Mem-1 的 stdout/stderr 逐字节相同，`SUMMARY gaps=16 cascades=66 globals=1
definitions=126`，与 M1-a2 记录的 `prod-snapshot-check.txt` 相同；运行前后输入文件 sha256 不变。

## 已知限制

- `native_management` 的配置 apply 之后不 trim（`config_all` 不用）。
- `malloc_trim` 依次持有各 arena 锁（本机 1.5–9 ms），DNS 线程此间的分配会等待。
- 非 glibc（如 musl）构建不做上述 glibc 调优，只关闭 THP。

## 审查

自查（`/code-review high`）10 条：修复 3 条（AdGuard 直接匹配器旧代也移出 DNS 线程释放；旧代释放不再排在构建线程队列后面；
测量脚本探针每次使用新的 DNS id），其余为已记录的限制或性能项。Codex（`gpt-6.1-sol`/medium，只读）第 1 轮：无 finding，
`FINAL: PASS`。

## 用户决定（2026-10-08）

- 接受 `mosdns-native-host` 由 `forbid(unsafe_code)` 改为 crate 级 `deny`，仅 `memory.rs` 放行
  `mallopt`/`malloc_trim` 两个 glibc 调用（仅 linux+gnu 编译）。
- 接受进程级关闭 THP（仅影响本进程）。
- `native_management` apply 后不 trim：记为待办，用到该模式时再补。
- Mem-2（domain trie 紧凑化等）不立即做，挪到 5D 阶段按 RSS 预算需要再定；下一步 M1-b1。
