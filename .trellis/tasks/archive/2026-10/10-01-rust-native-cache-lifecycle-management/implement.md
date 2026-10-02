# Implementation plan — in progress

状态：用户已于 2026-10-01 授权实施（task.py start 已执行，status=completed）；**S1/S2 已实现并通过独立 review，S3 已完成，S4 已完成，S5 已完成，S6 已通过 review，S7 已完成，C2C iteration 11 FINAL: PASS / DONE**，见下方 Progress。R2 已选择，durable-first flush、基础 EDNS/DO 与 ECS 延期已确认。一个任务按可独立验证切片顺序执行，不新建 child tasks、不派发实现 agent。Trellis auto-commit 保持关闭。

## Progress — 2026-10-01 round

| 切片 | 状态 | 说明 |
| --- | --- | --- |
| S1 catalog/config | **完成** | `CompiledConfig.caches: Vec<CachePluginConfig>` + `CacheId`/`CacheKind`；多个 named cache 并存；`exec: cache [size]` 每 callsite 私有实例（lazy 恒 0、无 tag/dump/metrics）；size/dump_interval 默认值与显式 null；enable_ecs=true、负值/越界 lazy、未知键拒绝；首轮曾 fail-closed 拒绝后续功能；当前 dump_file/exclude_ip/正值 lazy 已在 S3–S5 交付 |
| S2 publication | **完成** | sequence-core typed `WatchToken` + 多 watch + LIFO `pending_scope_completions`；`ScopeComplete`/`ScopeAborted` 区分自然完成与被 exit 解开的 scope，按 token 而非 executable 配对；request/branch 改为 `Vec<PendingFrame>`；删除 duplicate-cache fail-closed 与 `cache_accessed` 记账 |
| S3 key/time/metadata | **完成，C2C iteration 7 PASS** | 产品文本 key/AD-CD-DO、单调与墙上时间、基础 EDNS、OPT 重建、CIDR 排除及 domain_set；225 项缓存/host 回归通过，fmt/clippy 与 30 项重点复测通过 |
| S4 refresh | **完成，C2C iteration 8 PASS** | owner 单飞、256 上限、5 秒截止、独立 64 fuel successor、嵌套 Lazy inline miss、审计隔离、关闭 cancel/join；299 项 host/sequence 回归、fmt/clippy 通过 |
| S5 codec/transaction | **完成，C2C iteration 9 PASS** | bounded v2 codec、原子 prepared merge、generation/dirty revision、owner 事务、durable-first flush、startup/periodic/final save；239 项回归与后续 3 项终态验证、fmt/clippy、真实 Go 双向互通通过 |
| S6 HTTP/metrics | **完成，C2C iteration 10 PASS** | named inventory、四项前台 metrics/live size、show/dump/save/load_dump/flush 与 route-specific body cap；真实 DNS/HTTP/restart 及既有 domain_set 37 项回归、fmt/clippy 通过 |
| S7 Vue/proof | **完成，C2C iteration 11 PASS / DONE** | 真实 inventory/指标缺值、详情搜索、部分失败、持久加载错误；真实 DNS/API/browser/restart、SIGTERM 收敛、编码 tag；见 research/s7-final-evidence.md |
| 独立 review | 已复核 | 首轮 FAIL（F1–F11）：F1/F2 改为 typed watch token + 全 scope 退场检测，F3/F5/F9 已修，F4/F6/F8 随 F3 消失。次轮 FAIL（N1–N7，无行为阻塞）：N1 改为错误路径清空 armed watch 并修正文档，N2/N3 补齐此前误报的 F10/F7，N4 重跑并改正证伪记录，N5 修正 PRD 披露，N7 清理残留注释；N6 为既有不对称，已在证据中披露。每轮修完重跑受影响检查。第三轮 FAIL 仅 N8（记录早于最后两次编辑）：在同一冻结 revision（rust/**/*.rs 的 md5 列表指纹 cc5483573e614d546927e63359c7e316）上重跑 fmt/clippy/test 与证伪，记录 1005 passed / 0 failed、引擎 lib 37 passed、证伪 32 passed / 5 failed，并修掉三条 nit。第四轮 FAIL 仅 P1-1（fallback 的 direct cache target 绕过 tokenized publication，exit 后仍写缓存）+ P3-1（task 文档仍写 planning-only）：`BranchOutcome` 改为携带终止 `ExecutionCompletion`，direct cache target 仅在 `completed_naturally()` 时发布，新增两个真实 fallback+cache 的回归测试；P3-1 三处文档已统一为「S1/S2 已授权并实现，S3–S7 未实施」 |

差异范围与资源证据见 `research/implementation-evidence.md`。构建/测试全部在
SSH `mosdns-rust` 的本任务独占目录执行，未在本机构建、未 push、未部署。

## Slice dependency and public tests

| 切片 | 前置 | 可观察行为 | 测试公开接口 | 可模拟边界 |
| --- | --- | --- | --- | --- |
| S1 catalog/config | design §5 配置矩阵 | named 独立；quick callsite 独立无 API；默认和路径确定 | compile_yaml_with_base / compiled catalog | 文件 fixture，配置 base |
| S2 publication | S1 | A/B 双 miss；A hit；重复 dispatch；scope/branch 隔离 | sequence machine watch token + native request execution | 上游响应，有限 control |
| S3 key/time/metadata | design §5 支持矩阵、S1/S2 | key 隔离、fresh/lazy/expired、domain_set hit 恢复 | cache adapter/core native APIs + HTTP audit | 注入 wall/mono clock、合法/坏 wire |
| S4 refresh | S2/S3 | client 结束仍刷新；64 fuel/5s/256/singleflight/no queue；终态无泄漏 | host request + owner stop/drain | 可控 upstream gates、时钟 |
| S5 codec/transaction | S3/S4 | 双向 eligible dump；原子 merge；flush/import 拦旧 publish；失败不部分更新 | dump reader/writer + owner management | 临时文件、I/O fault injector |
| S6 HTTP/metrics | S1/S5 | 实际 inventory、指标及 show/save/load/flush | 真实 loopback HTTP routes | fixtures；不能 mock catalog/transaction |
| S7 Vue/proof | S6 | 真缓存列表、分页搜索、部分失败、重启命中 | 现有 Vue 管理页 + DNS/API/browser | 受控 loopback upstream、测试 dump |

## Execution checklist after approval

1. 读取 before-dev/spec 与当前工作区差异；记载确切 HEAD、已批准 PRD/design，保护 unrelated dirty。确认远端隔离空间足够。若最终契约有变化重新审批。
2. 为各切片先添加能证明行为的红测试，再实现；不写仅镜像私有结构的测试。记录 watch token API、detached successor recipe/new-root seam；不得通过 unsafe 延长 request lifetime。
3. S1/S2：改 config/cache catalog/assembly，替换单 PendingStore 及全分支 bool；保留现有 fallback/preference completion/facts 回归。
4. S3/S4：实现 key、dual clocks、domain_set 与 owner refresh；测试源 query terminal 后不变、permit/drop 清理、owner cancel 和深层 shared fuel exhaustion。审查 refresh 中禁止再生成 detached refresh。
5. S5：实现 native snapshot/import seam，不扩 hybrid ABI、不维护 shadow store。dump 写到独占临时目录；检验 malformed footer、oversize、wall/time 异常、merge 碰撞、dirty revision race 和 flush/import 与前台/后台 publication race。
6. S6/S7：plugin routing 与 inventory/metrics、现有 Vue 列表/详情/批量清理；保护 Go fallback 页行为。show 使用真正 DNS 文本 fixture，拒绝仅截图或模拟 JSON 作为端到端证明。
7. 审查必要 specs 和 feature roadmap，只更新本任务实现的范围；不标记全量 cache/Prometheus/纯原生迁移门禁完成。
8. 执行下列验证，记录全部失败和重跑结果；2026-10-02 用户已明确选择当前内置浏览器 C2C 对话，每个 slice 完成后提交该对话复审，FAIL 自动修复再复审，PASS 后继续已批准的下一 slice。

## Validation environment and commands

所有构建/测试在 SSH alias `mosdns-rust` 的本任务独占工作目录执行，不在本机构建。使用 workspace 根实际 Cargo manifest（当前 rust/Cargo.toml），逐步命令：

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mosdns-sequence-core
cargo test -p mosdns-cache-core
cargo test -p mosdns-native-host
cargo test --workspace --exclude mosdns-native-host -j1
cargo build -p mosdns-native-host
```

前端按 webui-log/package.json 现有脚本运行必要 lint/test/build，先读取脚本名不猜命令。若资源不足把测试合法拆组，不能将一次 linker 失败改写为通过。远端源码快照应包含必要批准规划/实现文件但不复制凭据，不覆盖其它任务目录。

真实 proof 使用独占 loopback 高端口受控上游、native DNS/API 和实际 Vue；验证 miss → fresh → lazy → 后台更新，same-key合并，两个 named cache，关闭 drain、save/restart 命中、domain_set、metrics 和部分清理失败。禁用公共 DNS、53 端口、生产部署和无关目录清理。Go dump fixture 验证在隔离测试中完成，不延伸成产品 Go fallback。

## Review gates and rollback

- 按 design §5/§6/§7 已冻结的 EDNS/ECS、异常时间、限额、route、exit matrix 写行为测试；若发现技术上不可实现或产品范围需要变化，先更新规划再审批，不临场改合同。
- durable-first flush 已单独批准；验证 pre-commit failure 保原状态，post-commit invariant failure fail-closed，禁止用旧内存 shutdown dump。
- 核心风险是嵌套 scope completion、borrowed program lifetime、事务跨 await、后台任务 audit 隔离；各自需要行为与 terminal/drop 测试。
- 出问题仅回退本任务拥有的变更；不 reset/clean unrelated work，不自动 commit/push。
- 最终报告分别列出自动检查、真实 DNS/API/browser 证明、未通过项和延期范围。

## Continuation — 2026-10-02

用户授权接续本任务全部 S3–S7，并逐 slice 在同一 C2C 对话 review。S1/S2 修复复审 iteration 6 明确 FINAL: PASS，P1-1/P3-1 closed；remote fmt clean/cache_catalog 10 passed 的原始输出已通过 c2c record 发布。审查对话：`https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abefd59-a3c0-83e8-ad5f-e599acf02c06`。不会自动提交、push 或部署。

S5 reviewer 初次 P1-S5-1 建议 flush 保存旧 snapshot，与冻结的 empty dump 合同相反；同一对话读取 design §4/PRD 后撤回为 invalid finding，并更正 FINAL: PASS。未按错误建议修改合同或实现。
