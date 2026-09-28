# Execution checklist — revision 2

实现授权已由用户于 2026-09-27 后续明确给出；本文件按小步顺序记录已执行项。auto-commit=false；未部署、未生产切换。

## 1. 跑通代表配置（A1/A2/A3）

- [x] trellis-before-dev 读取规范并核对 source/dirty baseline；保留无关 dirty 文件。
- [x] research/example-compositions.md 落为稳定配置/规则和独立响应预期，保留裁剪来源；未新增压测 runner。
- [x] 定向确认 direct call、cache 后继、文件路径语义；用 characterization 测试固定行为。
- [x] strict 定义/引用编译、顶层 include、provider files/exps、qtype/has_resp；消除插件数/固定图，加载负例在 bind 前定位。
- [x] canonical machine 接 direct child；单 cache 支持 entry/child 后继；常用 reject 0/3 与 >15 未支持范围加载期报错。
- [x] 本地跑通 block、路由 miss/hit、默认分支、child 后父继续（execution 级 in-process 证据；listener 需 Linux）。

## 2. 兼容与故障收敛（A2/A3/A4/A5）

- [x] child accept/reject/exit、try、jump/goto/return、exec list 用小变体验证，未建设第二解释器。
- [x] 证明 child cache 不被父覆盖污染，entry cache 包裹下游正确；重复访问失败；取消无 publication。
- [x] 共用 deadline、fuel=64；deadline 门改由“是否已有前序 attempt”决定，不再按 forward 数选择。
- [x] 最小 named origin 接缝：`last_origin` 来自真实执行的具名 sequence，synthetic inline 不冒充；不再固定回填 entry。
- [x] 旧 W1/W2/W3 回归和第一次完整 workspace 回归：`cargo test --workspace` 891 passed / 0 failed（review remediation 前）。

## 3. Linux 交付及最终审查（A6）

- [x] 用 `ssh mosdns-rust` 核对目标机服务与监听；`mosdns.service` active，PID 425。仅启动临时目录中的自有 Linux 测试进程，不触碰生产服务。
- [x] Linux x86_64 上运行代表链 UDP listener/audit on 与 TCP listener/audit off；确认 block、qtype 65、cache miss/hit、父继续/default 分支及 peer 计数（实测细节见下）。
- [x] 保存自有测试 PID、退出码、观察到的临时 peer 端点与运行前后监听 socket 对照；测试进程已退出回收。
- [x] Linux CLI TCP/audit-off 代表链复核及既有 `dnsperf` 短诊断完成；UDP/audit-on 由上方远端 listener 集成测试覆盖。短诊断包含 SSH 转发开销，只记录观测，不作为性能门槛或 PASS。
- [ ] 远端故障/取消/关闭变体、`local.only.test` exact 分支、完整 workspace 及旧 W1/W2/W3 suites 未执行；这些仍是明确未测项，不影响所选远端 E2E 子项记录。
- [x] 做过轻量短诊断；没有性能验收、性能 PASS 或持续资源结论。
- [x] 覆盖表记录 P02/P26/P34/P44/P15/P12、L01/L02/C01 的实际子项和延期。
- [ ] 一次独立最终 full-scope review 覆盖 A1–A6；仅在精确范围已提交且可由当前 workspace connector 比较后发送，记录明确 PASS/FAIL 后再更新本项。
- [ ] 本轮不执行 finish/archive/journal；review PASS 本身不改变 Trellis 生命周期。

## Local verification (2026-09-27–28)

~~~text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check                                      OK
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core -p mosdns-native-host --all-targets -- -D warnings   OK (0 warnings)
cargo test --manifest-path rust/Cargo.toml --workspace --no-fail-fast                           exit 0; all workspace tests/doctests passed
~~~

The pre-review full-workspace run recorded 891 passed / 0 failed. After the four review fixes, `cargo test --manifest-path rust/Cargo.toml --workspace --no-fail-fast` exited 0 with 896 passed / 0 failed; `cargo test --manifest-path rust/Cargo.toml --workspace -- --list` independently counted 896 tests. `cargo fmt` and Clippy (`-D warnings`) also exited 0 on this candidate.

用户确认 `tests/slice3_composition.rs` 的代表链用例在本机真实 UDP/TCP listener 和 loopback peers 通过（9 cases）：

- `the_representative_chain_routes_blocks_and_falls_through_to_default`: blocked.test and
  another-blocked.test → NXDOMAIN with zero peer calls; qtype 65 → NOERROR with zero peer
  calls; a.local.test first query → local peer, cache miss, final_sequence=sequence_main;
  unmatched.test → parent continues to the default peer; two repeats of a.local.test →
  child cache hit (local peer still at 1, default still at 1); audit records all 7 queries
  with correct miss/hit status.
- `the_representative_chain_also_serves_tcp_with_audit_off`: same chain behind a TCP
  listener, identical wire and peer counts, and no retained audit record.

W1 UDP/TCP、W2 cache、W3 routing 与完整 Rust workspace 回归均按上一会话结果通过；这不是本轮远端完整回归。

## Independent review attempt 1 (2026-09-28)

The reviewer compared `11bd56c40d255d6ae93b0a2eba1c85214300b149` to
`b0c3a29876b917abfc503dbad2f5706488ee1c91` and returned `FINAL: FAIL`. It
reported four root causes; retain these IDs in the re-review:

- `P1-1`: included plugin definitions had lost their declaring YAML path and
  base directory. `RawPlugin` now keeps both, definition compilers use that
  source path, and included `domain_set.files` resolve from the included YAML
  directory. `included_definitions_keep_their_relative_path_and_source_context`
  checks a real relative file and errors naming the included YAML and rule file.
- `P1-2`: the legacy primary-forward view wrongly rejected a valid graph when
  its only forward was reachable through `goto` or `try`. The view is now
  optional and non-semantic, and traversal includes both edges.
  `goto_and_try_only_forward_paths_compile_and_execute` exercises each path
  over a real UDP listener and counted loopback peer.
- `P1-3`: a request could access the same cache again after a first hit or
  after a miss had published. A request-local access guard now rejects every
  second dispatch before lookup. Unit tests
  `a_second_cache_access_after_a_hit_fails_closed` and
  `a_second_cache_access_after_miss_publication_fails_closed` cover both paths.
- `P2-1`: a forward's effective observer identity could collide between a
  default tag and another forward's explicit upstream tag. Compilation now
  checks the final identity across all forwards;
  `effective_upstream_identities_must_be_unique_across_explicit_and_default_tags`
  covers that mixed case.

Focused local `slice2_config` (12 passed), `slice3_composition` (11 passed),
the two cache-access unit tests, formatting, Clippy, and the full workspace
(896 passed) pass on this candidate. The Linux retest against its committed
source is pending; no second review has been requested yet.

## Remote Linux E2E record (2026-09-28, A6 selected subitems)

### Source, config, build and command

- Branch `rust`, unchanged parent/HEAD at execution start: `11bd56c40d255d6ae93b0a2eba1c85214300b149`. Source archive SHA-256: `3dba45ef74051dd8adeb01352742ea066830a1bd7e0007daf14260c93cd7d515`; remote SHA matched. Representative test source SHA-256: `cd9fedb05b76e005777d8ff7013baf1915fe7324e031199ff68767844100c953`.
- The workstation is macOS arm64 and had no Linux Rust target/linker. To avoid a local VM, the Rust workspace source archive was copied to `/tmp/mosdns-phase5b-a6.euf3Vb/source/` on `mosdns-rust`; the Rust integration-test executable was built there with the workspace Cargo command below. This run did not build or launch the `mosdns start -c` CLI binary.
- Config source is `ROOT_CONFIG`/`ROUTES_CONFIG` and `chain_assembly` in `rust/native-host/tests/slice3_composition.rs`. The fixture wrote `/tmp/phase5b-routing-406778/{config.yaml,sub_config/routes.yaml,rules/local.txt}`, `/tmp/phase5b-tcp-406778/...`, and the loader fixture `/tmp/phase5b-chain-406778/...`; `Fixture::drop` removed them on test completion. `rules/local.txt` was `domain:local.test` plus `full:local.only.test`. Runtime peer addresses were substituted from loopback UDP peers; both listener APIs bound `127.0.0.1:0` for kernel-assigned ports. In particular, the TCP listener was real DNS-over-TCP, while the test's upstream peer fixtures used UDP.

~~~sh
tar -czf /tmp/mosdns-rust-phase5b-a6-20260928-src.tar.gz --exclude='rust/target' --exclude='*/.DS_Store' --exclude='.DS_Store' rust
scp /tmp/mosdns-rust-phase5b-a6-20260928-src.tar.gz mosdns-rust:/tmp/mosdns-phase5b-a6.euf3Vb/source.tar.gz
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-a6.euf3Vb && mkdir source && tar -xzf source.tar.gz -C source'
ssh mosdns-rust 'A6_REMOTE_ROOT=/tmp/mosdns-phase5b-a6.euf3Vb CARGO_TARGET_DIR=/tmp/mosdns-phase5b-a6.euf3Vb/target CARGO_BUILD_JOBS=2 cargo test --manifest-path /tmp/mosdns-phase5b-a6.euf3Vb/source/rust/Cargo.toml -p mosdns-native-host --test slice3_composition --no-run'
ssh mosdns-rust '/tmp/mosdns-phase5b-a6.euf3Vb/target/debug/deps/slice3_composition-2f26e2878285f663 the_representative_chain --nocapture'
~~~

The Linux build completed in 39.56s. Artifact: `slice3_composition-2f26e2878285f663`, ELF x86_64; 94,906,936 bytes; SHA-256 `8d0ea44dc523f73d29f72fd6f7dca6df918034254cc3d76093afc4dd2c415539`. The test filter ran three tests: config load/include/rule-file, UDP representative chain, and TCP representative chain. Result: `3 passed; 0 failed; 0 ignored; 6 filtered out` (0.03s).

### Observed behavior

- UDP listener with audit on sent seven fixed queries: `blocked.test A` and `another-blocked.test A` returned NXDOMAIN; `other.test HTTPS` (qtype 65) returned NOERROR; first `a.local.test A` was a cache miss answered by local peer `192.0.2.21`; `other.test A` fell through to default peer `192.0.2.22`; two repeated `a.local.test A` queries were child-cache hits returning `.21`. Each peer's total was one request, so block/qtype-65 and both hits added zero peer requests. Audit retained seven records; the first local query was `Miss`, the repeated query was `Hit`, and the executing sequence was `sequence_main`.
- TCP listener with audit off sent three fixed queries: block returned NXDOMAIN, local suffix returned `.21`, and `unmatched.test A` reached default peer `.22`. Local/default peer counts were one each; audit retained zero records.
- Initial `ss -Hltnup` inventory: `mosdns.service` PID 425 held TCP ports `53,2222,3077,3099,3111,3333,4444,7777,8888,9099` and UDP ports `53,2222,3077,3099,3111,3333,4444,7777,8888`; other observed system listeners were SSH 22, Exim 25 and systemd-resolve 5355. Test listeners and peers requested kernel-selected ephemeral ports with `127.0.0.1:0`, avoiding these fixed ports. `ss` sampled four process-owned UDP fixture endpoints: `127.0.0.1:{37458,45401,53212,55169}`. The short run's sampling did not capture the assigned listener ports. Full listener snapshots before/after matched byte-for-byte (both SHA-256 `17a694daa8a6cfdeff606df95c3b89b5b4babb9a3373aca45d6b0e43ecb19d18`, diff 0 bytes), proving no listener remained. The production service stayed active with PID 425 before and after.
- Owned test PID `406778` (started `2026-09-27 16:28:29Z`) exited 0 and was reaped; the generated fixture directories were absent after the run. No application or production configuration was changed.
- Scoped failures: none. GNU tar emitted ignored macOS extended-attribute warnings while unpacking; the source archive SHA-256 matched on the remote and build/test completed successfully.

### Still unperformed

That first remote integration run used the pre-review source. Remote fault/cancellation/close variants, the `local.only.test` exact-rule request, full workspace and legacy W1/W2/W3 suites, and independent full-scope A1–A6 review remain unperformed. This integration-test run did not launch the main CLI; a separate CLI run and short diagnostic are recorded below. No performance PASS or production change is claimed. Its temporary integration-test workspace and generated fixture directories were removed; `mosdns.service` remained active with PID 425, and the listener snapshot hash matched before and after.

## Remote Linux A6 retest after review remediation (2026-09-28)

### Exact source and configuration

- Branch `rust`; candidate commit `abeeb3e3bfb4458588430b83bfbd9280b359d37d`, parent `b0c3a29876b917abfc503dbad2f5706488ee1c91` (the reviewed range still starts at `11bd56c40d255d6ae93b0a2eba1c85214300b149`). The source for this retest is committed; unrelated worktree changes were excluded.
- This workstation is macOS arm64 without a Linux linker/target. There is no standalone build script for `mosdns-native-host`; `scripts/build-rust-experimental.sh` builds the transitional Go/cgo executable, so it was not used. The Rust workspace was built on `mosdns-rust` with Cargo only. No frontend or Go build ran.
- The first source archive contained only `rust/` (SHA-256 `f8ec469d99198222d772fbd462bbdc9111b48efb6e6b2bdb33b444613a6b5eb0`). The initial remote `--no-run` command (same Cargo command shown below) failed with exit 101 because `slice2_config.rs` has compile-time `include_str!` references to `tests/phase5a-baseline/configs/{forward-udp,forward-tcp,cache,routing}.yaml`. No test executable ran and no test socket was opened in that attempt. The corrected archive included `rust/` and only those four config fixtures; local and remote SHA-256 both matched `07ca5b94df64eda2c8a551860b7d58e17f0d6123e2ea20230876828d973b00f3`.
- Composition source/config: `rust/native-host/tests/slice3_composition.rs` (SHA-256 `f077b4347d5915ecb5ab02dca5897fb8e037dc4eee2b79274334464cf16c1fa1`), constants `ROOT_CONFIG` and `ROUTES_CONFIG`, and `chain_assembly`. The fixture wrote root `config.yaml`, included `sub_config/routes.yaml`, and `sub_config/rules/local.txt` containing `domain:local.test` and `full:local.only.test`; it also placed an invalid root-level `rules/local.txt` decoy. Runtime upstream peers and listener sockets bind loopback port `0` for kernel-assigned ephemeral ports.

### Build and exact test commands

~~~sh
git archive --format=tar.gz HEAD rust tests/phase5a-baseline/configs > /tmp/mosdns-phase5b-candidate.dkVK8k/rust-plus-test-fixtures.tar.gz
scp /tmp/mosdns-phase5b-candidate.dkVK8k/rust-plus-test-fixtures.tar.gz mosdns-rust:/tmp/mosdns-phase5b-a6.5Oi0hx/source.tar.gz
ssh mosdns-rust 'tar -xzf /tmp/mosdns-phase5b-a6.5Oi0hx/source.tar.gz -C /tmp/mosdns-phase5b-a6.5Oi0hx/source'
ssh mosdns-rust 'A6_REMOTE_ROOT=/tmp/mosdns-phase5b-a6.5Oi0hx CARGO_TARGET_DIR=/tmp/mosdns-phase5b-a6.5Oi0hx/target CARGO_BUILD_JOBS=2 cargo test --manifest-path /tmp/mosdns-phase5b-a6.5Oi0hx/source/rust/Cargo.toml -p mosdns-native-host --test slice2_config --test slice3_composition --no-run'
ssh mosdns-rust 'set +e; BIN=/tmp/mosdns-phase5b-a6.5Oi0hx/target/debug/deps/slice2_config-cc5529512b9fe56b; A6_REMOTE_ROOT=/tmp/mosdns-phase5b-a6.5Oi0hx "$BIN" --nocapture & PID=$!; printf "OWNED_PID=%s\n" "$PID"; wait "$PID"; RC=$?; printf "EXIT=%s\n" "$RC"; printf "%s\n" "$RC" > /tmp/mosdns-phase5b-a6.5Oi0hx/slice2.exit; exit "$RC"'
ssh mosdns-rust 'set +e; BIN=/tmp/mosdns-phase5b-a6.5Oi0hx/target/debug/deps/slice3_composition-2f26e2878285f663; A6_REMOTE_ROOT=/tmp/mosdns-phase5b-a6.5Oi0hx "$BIN" --nocapture & PID=$!; printf "OWNED_PID=%s\n" "$PID"; wait "$PID"; RC=$?; printf "EXIT=%s\n" "$RC"; printf "%s\n" "$RC" > /tmp/mosdns-phase5b-a6.5Oi0hx/slice3.exit; exit "$RC"'
~~~

The corrected Linux build exited 0 in 1.78s. Test executable SHA-256 values were `c77b1b629c76a2ee466e33928a3b0afa0424f2c258b2e4490c10376973f3f5a4` (`slice2_config`) and `9057c26ed07d8aa2e4e246e9f31bb5cc6d79a5ae8076aad9ebb4906ce3b9b89a` (`slice3_composition`). Results: `slice2_config` 12 passed / 0 failed; `slice3_composition` 11 passed / 0 failed. The latter exercised the included relative path/source-context correction, forward reachability only through `goto` and `try`, and both real-listener representative-chain variants.

### Observed behavior and cleanup

- UDP listener / audit on: `blocked.test A` and `another-blocked.test A` returned NXDOMAIN; `other.test HTTPS` (qtype 65) returned NOERROR; all three made zero peer calls. The first `a.local.test A` was a child-cache miss and reached the local peer once; `other.test A` continued from the parent to the default peer once; two repeats of `a.local.test A` were child-cache hits. Final counters were local `1`, default `1`. Audit retained all seven requests, marked the initial local request Miss and a repeat Hit, and reported the actual `sequence_main` position.
- TCP listener / audit off: a real DNS-over-TCP request to a blocked name returned NXDOMAIN; local and unmatched names returned local/default answers with each peer at one request. Audit retained no per-query record. Both listeners and their counted loopback UDP peers bound port `0`; the test output does not log the assigned ephemeral numbers.
- Added retest process PIDs `419216` (`slice2_config`) and `419242` (`slice3_composition`) both exited 0 and were absent afterward. `Fixture::drop` removed all test directories; zero `/tmp/phase5b-*-419242` directories remained. The before/after `ss -Hltnup` snapshots were byte-identical, SHA-256 `17a694daa8a6cfdeff606df95c3b89b5b4babb9a3373aca45d6b0e43ecb19d18`; no test listener remained. `mosdns.service` stayed active with PID 425.
- The corrected test archive, target directory, executables and logs were removed from `/tmp/mosdns-phase5b-a6.5Oi0hx`; the local archive directory was removed. The only remote failure was the first incomplete source package described above; all functional tests passed. No production process/configuration was changed and no performance PASS is claimed.

## Delivered in this pass

- `rust/sequence-core`：新增 `ExecutableSpec::Call`/`ValidatedExecutable::Call`（direct named call，独立 child scope，退出传播遇 try 才捕获）；`ScopeStack` 稳定 scope 身份；`watch_enclosing_scope`/`resume_scope_completion` + `MachineStep::ScopeComplete`，在 cache 所在后继真正结束的边界通知一次；`last_origin` 记录真实具名执行位置；`ValidatedSequence::synthetic` 标记 multi-exec lowering。
- `rust/native-host/src/config.rs`：重写为收集后解析的通用编译器。顶层 include 按声明文件目录解析、嵌套 include 明确拒绝、domain_set exps/files、qname/qtype/has_resp/resp_ip/_true/_false、direct `$sequence` → Call、exec scalar/list、reject 0..15 加载期校验、cache size 可配置、单 cache/单 listener/重复 tag/跨类型引用在 bind 前定位。
- `rust/native-host/src/execution.rs`：cache token 只在“所在后继自然完成”边界发布；hit 结束后继链但父继续；miss 在 child 完成点保存，父后续改写不污染；重复动态访问受控失败；forward 校验与终态策略不再按 forward 数分叉。
- 证据：`rust/native-host/tests/slice3_composition.rs`（loader/include/规则路径/负例/重排）与 execution 级代表链测试（block/qtype-0/routed miss/hit/父继续/child cache 不被父污染/entry cache/重复访问/取消）。

## Appropriate checks

~~~sh
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core -p mosdns-native-host --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace
python3 .trellis/scripts/task.py validate .trellis/tasks/09-27-rust-phase5b-config-sequence-composition
git diff --check
~~~

本地完整 workspace、fmt、Clippy 在 2026-09-28 均重新执行并通过。远端所选功能用例、CLI 复核和短诊断不替代远端完整 workspace/旧链回归。

## Remote Linux CLI and short diagnostic (2026-09-28, A6 selected subitems)

### Source, build, configuration, and command

- The source was the same uncommitted Rust workspace on branch `rust`, parent `11bd56c40d255d6ae93b0a2eba1c85214300b149`. A Rust-only source archive was copied through the `mosdns-rust` SSH alias to `/tmp/mosdns-phase5b-cli.KhdZLu/source/`; local and remote SHA-256 both equaled `b880172bb8d43a48c43cacddf289ca536be1144c13e00e9cd34ff22f40dfefd1`.
- The repository has no build script for `mosdns-native-host` alone. `scripts/build-rust-experimental.sh` builds the transitional Go/cgo runtime and is not appropriate for this pure Rust-native host check. The Linux native-host executable was therefore built with the Rust workspace Cargo command (no frontend or Go build was run):

~~~sh
CARGO_TARGET_DIR=/tmp/mosdns-phase5b-cli.KhdZLu/target CARGO_BUILD_JOBS=2 cargo build \
  --manifest-path /tmp/mosdns-phase5b-cli.KhdZLu/source/rust/Cargo.toml \
  -p mosdns-native-host --release --locked
~~~

  Build exited 0 in 1m18s. The resulting `mosdns` was an x86_64 ELF, 4,578,160 bytes, SHA-256 `e0167f0b52397a584bf3a4a0233a758059a7d097ebc04abdbde0cfb042ae84b9`.
- The CLI config was derived from `ROOT_CONFIG`/`ROUTES_CONFIG` in `rust/native-host/tests/slice3_composition.rs`, switched to `tcp_server`, audit disabled, `idle_timeout: 5`; `rules/local.txt` contained `domain:local.test` and `full:local.only.test`. Recorded file hashes: `config.yaml` `3d56c19f6d1c943ab359004ae69ed1c8724b700c9326147e56ca1676977a98db`, `routes.yaml` `3e9e6b28392303f831671cf4bf606c5a3cf575582c12e87c4875b03388f2a69a`, `local.txt` `d513818101e5317609119e8bef2fa389f5aa5d83cc3a4905d200b9be71e4bc95`.
- The CLI command was `mosdns start -c /tmp/mosdns-phase5b-cli.KhdZLu/config.yaml`. Its TCP listener bound `127.0.0.1:45643`; counted UDP fixtures bound local peer `127.0.0.1:57355` and default peer `127.0.0.1:60727`. No fixed production/service port was used.

### Functional and diagnostic results

- Over the real TCP listener, `dig +tcp +noedns` verified block → NXDOMAIN with peer counts `0/0`; `other.test HTTPS` (qtype 65) → NOERROR with counts still `0/0`; `a.local.test A` → local `.21`, counts `1/0`; `other.test A` → default `.22`, counts `1/1`; two repeated `a.local.test A` requests were child-cache hits and left counts `1/1`. The audit-off config retained no per-query audit records. The UDP listener/audit-on path and seven-query audit assertions were exercised in the separate integration-test run above.
- Before the short diagnosis, the Linux CLI RSS sample was 3,976 KiB and `ps` CPU was 0.0% at that instant. Existing `dnsperf` 2.15.1 ran 50 TCP requests through a local SSH forwarding endpoint to the remote TCP listener:

~~~sh
/opt/homebrew/bin/dnsperf -m tcp -s 127.0.0.1 -p 52561 \
  -d /tmp/mosdns-rust-phase5b-cli-input/hot-cache.txt \
  -c 1 -T 1 -n 50 -Q 10 -q 1 -t 2
~~~

  Result: 50/50 completed, 0 lost, 10.0 QPS over 5.000016s; average 0.846 ms, min 0.624 ms, max 2.003 ms, standard deviation 0.307 ms. These timings include SSH tunnel/client overhead and are only a short diagnostic, not a performance acceptance result. Peer counters ended at local `2`, default `1`; therefore exactly one extra local peer call was observed during the 50-request interval, and it would be inaccurate to claim all 50 were cache hits. Post-run RSS sample remained 3,976 KiB; instantaneous CPU snapshots are not a sustained resource measurement.

### Failed attempts, process ownership, and cleanup

- First diagnostic attempt used `dig`'s default EDNS OPT. The cache adapter excludes queries with additional records, so local queries missed cache; the temporary `verify.sh` expected hits and exited 1 (peer totals local `3`, default `1`). Owned CLI/fixture PIDs `410225`/`410218` were signaled and confirmed absent, and listener inventory returned to baseline; their `wait` exit statuses were not captured, so they are not reported as successful exits.
- Second attempt reused a stale peer-readiness JSON file, so config pointed to old peer ports and the local upstream timed out; the verifier exited 9 with counters `0/0`. Corrected run cleared readiness/counter files before launch. Held cleanup recorded CLI PID `410474` exit `143` after SIGTERM and peer PID `410468` exit `0`; all ephemeral ports were released and service PID `425` remained active.
- Corrected functional/diagnostic run used CLI PID `410631`, fixture PID `410624`, and local SSH forwarding PID `527`. Cleanup recorded CLI exit `143` after SIGTERM, fixture exit `0`, tunnel exit `0`, and listener/local-peer/default-peer ports all released. `mosdns.service` remained active with PID `425`. Final listener inventory hash was `ec4730116588800fc184712866f51c9d5cc27be1ee994866abc8f001bde60286`; no test-owned socket remained. The remote temporary root and local archive/input files were removed.

No production configuration/service was changed. This evidence covers a Linux CLI TCP/audit-off pass plus the previously recorded UDP/audit-on integration-test pass; remote fault/cancellation/close variants, exact `local.only.test`, remote full workspace and legacy W1/W2/W3 suites remain unrun. No performance PASS is claimed.

## Scope control

模块可为本链 direct call/cache continuation/常用 reject 做必要小改动，不因 revision 1 文件限制退回 planning。新 runtime/动态插件框架、多 cache 嵌套、完整 EDNS/新协议/API/生产仍延期；重大新增需求更新规划。保留已停止 5A 结论；正式实验冻结阈值/重跑规则不套到普通功能修复。
