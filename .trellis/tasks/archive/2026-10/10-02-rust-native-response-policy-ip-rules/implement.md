# Implementation plan — approved execution

一个任务按以下可独立验证切片实现，不拆child/派发agent。用户已确认IP文本范围；完整规划已于2026-10-02获用户批准，task已激活。

| 切片 | 依赖 | 可观察行为 | 公开接口测试 | 模拟边界 |
| --- | --- | --- | --- | --- |
| S1 config/rules | 已批准矩阵 | hosts/redirect/ip_set、quick ttl、resp_ip解析；defaults/path/errors；payload priority | compile_yaml_with_base /真实文本fixtures | 临时文件、缺文件 |
| S2 wire policies | S1 | hosts双栈/空族/SOA、TTL fixed/range/0/OPT | native request driver + DNS response validation | 原始wire fixtures |
| S3 QueryView/redirect | S1/S2 | target发给真上游；original question恢复、CNAME；嵌套和terminal隔离 | public sequence/machine与native DNS | controlled upstream gates |
| S4 IP response matcher | S1 | IPv4/IPv6/CIDR/$tag/&file、OR、只Answer | config + native branch结果 | A/AAAA/CNAME包 |
| S5 composition | S2/S3/S4 | cache内外顺序、fallback/prefer、lazy/restart/取消/audit supplier | native listener + HTTP + dump restart | clock、peer响应/延迟 |
| S6 proof/spec/closure | S5 | 当前Vue详情真实CNAME/TTL/answers；必要回归和范围登记 | DNS/API/browser | 独占loopback测试服务 |

1. 启动前加载before-dev和backend spec；核对343c3810/2c059b0e以来变更及task approvals，保护现有脏改动，Trellis auto-commit=false。
2. 每切片先测试可观察行为，再实现。S1必须覆盖inline/file后者覆盖、payload匹配优先级、非空sets拒绝、文本格式/上限/line errors。
3. S2覆盖hosts的no-op/Continue、FakeSOA全字段、A/AAAA多地址、已有supplier清理；TTL0与inverted range实际行为、OPT不变、malformed无部分修改。
4. S3首先以“真实target上游question + final original question”建立QueryView红测试；覆盖redirect→cache与cache→redirect，不能只mocknetwork或传入人工target结果。测试exit保持ScopeAborted、错误与取消不发布、跨branch父raw/state不变、循环64fuel终止。
5. S4复用matcher-core，测试mapped地址和最长prefix，validate只Answer，不通过stringparse逐query加载文件。
6. S5验证上游attempts真实、Local/Cache/supplier区分、最终HTTP答案与DNS一致；cache lazy与TTL政策的顺序、停机join、原questionkey/targetkey的dump重启等价。原缓存/secure/fallback测试不可删除或弱化。
7. S6实际Vue详情与配置fixture组合：至少一条本地hosts链、一条redirect到controlledupstream链、一条双栈IP条件fallback链。浏览器仅消费本任务服务；不新增本地IP编辑产品流。更新必要spec/覆盖表，只登记实际支持子项。

## Minimum acceptance matrix added after planning review

| 行为 | 最低证明（不替代其它回归） |
| --- | --- |
| hosts A/AAAA/缺族 | 实际UDP请求，正确RR及空族SOA；至少一条TCP组合验证 |
| redirect question/CNAME | 真实peer收到target，client收到original question与a→b→d完整链；negative RCODE/SOA/TTL不被改归属 |
| TTL fixed/range | wire边界、0与inverted、OPT不变，cache内外顺序 |
| resp_ip IPv4/IPv6/CIDR | literal命中、CIDR命中/未命中；只Answer；inline/provider/&file OR |
| 文本文件 | empty/comment有效空集；IP缺文件告警与hosts缺文件失败；坏行/I/O/超限拒绝 |
| cache组合 | original/target两个placement的重复查询、supplier与后台refresh |
| fallback组合 | policy未命中、显式未短路允许覆盖、正常完成与取消/exit区别 |
| 规则变动/restart | 相同dump保留旧值的明确合同，执行flush后重启不复活；不宣称自动policy失效 |
| 后台刷新 | 同immutable规则snapshot，无文件读取/新root；query终态facts不变 |

P1/P2意见处置与来源记录于research/planning-review.md。按这些公开行为验证，不写用于满足意见标题的空测试。

## Validation environment

全部构建/测试在SSH alias mosdns-rust的本任务独占目录执行，不本机构建。先确认磁盘容量、部署自有源码快照；不清理其它任务目录。Cargo workspace根rust/，最少：

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mosdns-dns-core
cargo test -p mosdns-matcher-core
cargo test -p mosdns-sequence-core
cargo test -p mosdns-native-host
cargo test --workspace --exclude mosdns-native-host -j1
cargo build -p mosdns-native-host
```

前端无产品源码变更时复用现有bundle，仅browser证明，不为演示修改UI；若确有必要UI更改则在批准范围内跑现有Node tests/build/build:log1。真实DNS/API采用loopback高端口，无公共DNS/53端口/生产改动。测试失败、资源不足、未执行项均准确记录。

最终提供准确commit或working-tree范围与证据；review方式按当时用户授权，不自动新建C2C。禁止push/部署、无关reset/clean。如QueryView/token技术事实要求改变已批准语义，先更新规划说明差异再审批，不用临时兼容fallback掩盖。

## Execution/review orchestration authorized 2026-10-02

User requested this executor to complete every slice inline, using a new C2C conversation and requiring each slice FINAL: PASS before continuing. No implementation/check sub-agents. The new conversation verified workspace mosdns-rust through its connector and independently read the full current planning artifacts and relevant source; planning iteration 0 returned FINAL: PASS and a scoped S1 plan.

Review chat: https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abf42c0-0b24-83e8-a56e-cdd6427a976a

S1→S2→S3→S4→S5→S6 dependencies remain as above. For each slice: public behavior RED → implementation GREEN → appropriate checks → record success/failure and command output → same-chat independent review → fix/retest/re-review until PASS → next slice. Final review covers all scope and actual DNS/API/browser/restart proof. Review scope is explicitly identified per iteration; preserve unrelated changes, no implicit production/push/auto-commit authorization. Initial source baseline: 2c059b0ea1ae19b52fbe92a079d43bee863ccc4a.

Latest overall planning summary still requires subsequent human approval under the active planning workflow. No product source edits, builds or task.py start in this planning review session. After approval, retain this conversation for all S1–S6 reviews; do not ask again for the already-authorized review loop.

## Final approval — 2026-10-02

Human explicitly approved the latest complete planning summary with “批准”. Approval covers S1–S6, the frozen IP/text-only scope and retained-dump limitation, inline execution and same-chat per-slice C2C PASS before continuing. No repeated approval is needed for this unchanged scope. Planning/review gates are satisfied; task activation has not run because this turn still carries an explicit stay-in-planning workflow directive. This is a recorded approval, not implementation evidence.

## Current execution status — 2026-10-02

The final plan was explicitly approved by the human. Task activation succeeded after correcting the conditional planning breadcrumb; historical planning-only notes above describe earlier sessions. S1/S2/S3 (including jump-continuation supplemental correction) have independently passed exact-commit C2C review. S4 real response-IP matcher is undergoing final validation/review; S5/S6 remain pending. Every next slice stays gated by the preceding PASS. No push or deployment.

## S1 → S2 execution checkpoint

S1 exact commit e7e8a651ef3e5734d5bb3529775a96e3fc4f13b9 received FINAL: PASS in dedicated C2C iteration 1. S2 implements hosts/TTL in root/branch/direct fallback target paths and validates actual UDP behavior. Final S2 quality/review pending; S3-S6 not started.

## S2 → S3 execution checkpoint

S2 commit 8ce75e1257bc8989207a720f97a29af86d745064 received FINAL: PASS in iteration 2. S3 introduces scoped QueryView/redirect with safe wire rebuilding and supplier inheritance. A new RED test exposed exit conversion through named fallback targets; correction retains Exited across wrappers and prevents outer cache publication. S4–S6 remain pending.

## S4 → S5 execution checkpoint

S4 commit 7937a8f6879fbad7b2a15ebbe092335a739d5ab9 received FINAL: PASS in iteration 5. S5 adds seven real composition tests; no product source change was required by their passing behavior. Bound-file owner reconstruction/flush is tested here; actual process restart and public API/Vue remain S6.

## S6 final evidence candidate

S1–S5 independently passed C2C before the next slice. S6 has actual UDP/TCP, public API, maintained Vue details, real SIGTERM/process restart and retained-dump/Flush proof; Linux workspace 1,070 tests pass with no ignored tests, fmt/clippy pass. See research/public-proof/README.md. Whole-task exact-commit final review remains pending. No new product behavior, production promotion or push.

## Final accepted closure

S1–S6 and cumulative exact range 2c059b0e..13d60d3e received independent C2C FINAL: PASS (iteration 7). All approved task gates are complete; see research/s6-final-review.md. Earlier pending/planning notes are chronological evidence, superseded by this closure. No production cutover or push.
