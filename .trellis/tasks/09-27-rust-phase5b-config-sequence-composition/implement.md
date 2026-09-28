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
- [x] 在 `mosdns-rust` Linux x86_64 临时工作区补测 file-backed `full:local.only.test` exact 正反分支；UDP/audit-on 与 TCP/audit-off 的 exact 名均到 local peer，子域名均落到 default peer，计数正确（详见下方补充记录）。
- [x] 将 exact-rule 重跑使用的测试源码、身份校验运行器、原始输出与运行 JSON 保存在 `research/a6-exact-rule-*`；本次实测哈希见下方。旧的一次性补测仅留哈希，已由本次可复核重跑取代。
- [x] 在 `mosdns-rust` 运行完整 Rust workspace：60 个测试目标、896 passed / 0 failed / 0 ignored；包括 `w1_tcp`、`w1_udp`、`w2_cache`、`w3_routing` 以及取消/关闭用例。实际命令、两次未运行的失败尝试和清理记录见下方。
- [ ] 专门的远端 fault/cancel/close E2E 变体未运行，保留为 deferred；完整 workspace 中的 Linux cancellation/close/shutdown regressions 已运行，但不替代专门远端故障 E2E。
- [x] 做过轻量短诊断；没有性能验收、性能 PASS 或持续资源结论。
- [x] 覆盖表记录 P02/P26/P34/P44/P15/P12、L01/L02/C01 的实际子项和延期。
- [x] 独立最终 full-scope review 精确比较 `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767`；于 2026-09-28 返回 `FINAL: PASS`，详见下方 verdict 记录。
- [x] Codex `002reviewer` 对 A6 补充证据范围 `bcac20374312d5bf875164f87673224b9da2a796..4fc737aa0dffc8e92ed878577b9c8a4131568077` 返回历史 verdict `FINAL: PASS`。
- [ ] 用户指定的 C2C 对同一范围复审于 2026-09-28 返回 `FINAL: FAIL`，发现 P2-1（remaining scope 摘要漏记专门远端 fault 变体仍未运行）和 P2-2（exact-rule 临时测试/运行器源码未保存）。本次补齐 scope 描述、保存源码并重复实测；修复范围的 C2C re-review 待完成。
- [ ] 本轮不执行 finish/archive/journal；review PASS 本身不改变 Trellis 生命周期。

## Local verification (2026-09-27–28)

~~~text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check                                      OK
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core -p mosdns-native-host --all-targets -- -D warnings   OK (0 warnings)
cargo test --manifest-path rust/Cargo.toml --workspace --no-fail-fast                           exit 0; all workspace tests/doctests passed
~~~

The pre-review full-workspace run recorded 891 passed / 0 failed. After the four review fixes, `cargo test --manifest-path rust/Cargo.toml --workspace --no-fail-fast` exited 0 with 896 passed / 0 failed; `cargo test --manifest-path rust/Cargo.toml --workspace -- --list` independently counted 896 tests. `cargo fmt` and Clippy (`-D warnings`) also exited 0 on this candidate.

## Spec synchronization (2026-09-28, Phase 3.3)

Reviewed `.trellis/spec/backend/rust-migration.md` and `quality-guidelines.md` with `trellis-update-spec`. No additional code-spec change is needed: the existing seven-section “native named sequence calls and cache successor boundaries” scenario already records the direct-call/cache-boundary signatures, include-relative source context, cancellation/publication failures, ownership identities, and required real-listener/regression tests. This pass adds Linux regression evidence, not a new implementation contract.

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
(896 passed) pass on this candidate. The committed Linux retest is recorded
below. A final exact-range re-review returned `FINAL: PASS`; all four prior
findings are closed and no actionable finding remains open.

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

That first remote integration run used the pre-review source. Remote fault/cancellation/close variants, the `local.only.test` exact-rule request, and remote full workspace and legacy W1/W2/W3 suites remain unperformed. At the time, independent full-scope review was still pending; its later PASS is recorded below. This integration-test run did not launch the main CLI; a separate CLI run and short diagnostic are recorded below. No performance PASS or production change is claimed. Its temporary integration-test workspace and generated fixture directories were removed; `mosdns.service` remained active with PID 425, and the listener snapshot hash matched before and after.

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

## Final independent review result (2026-09-28)

- C2C reviewed the exact range `11bd56c40d255d6ae93b0a2eba1c85214300b149..016103f3c21ed2d659694ce10e64aaf24b5c2767` in the conversation **Rust MosDNS测试进度** and returned `FINAL: PASS`.
- The first review's `P1-1`, `P1-2`, `P1-3`, and `P2-1` findings were closed by `abeeb3e3bfb4458588430b83bfbd9280b359d37d`; the final review also closed its evidence-alignment item (`P2-2`). No actionable finding remains open.
- The reviewer confirmed the Rust code/spec in the remotely tested commit `abeeb3e3bfb4458588430b83bfbd9280b359d37d` are byte-identical to the reviewed HEAD `016103f3c21ed2d659694ce10e64aaf24b5c2767`; the latter adds documentation only. This ties the A6 retest above to the reviewed implementation.
- At the time of that review, remote full workspace and legacy W1/W2/W3 suites, remote fault/cancel/close variants, and the `local.only.test` exact-rule request remained unrun. No performance PASS, production deployment, or Trellis lifecycle change is claimed.

## Remote exact-rule A6 supplement (2026-09-28, `mosdns-rust`)

### Source, fixture, and build

- Target was reached only through `ssh mosdns-rust`: Linux x86_64, `rustc 1.95.0`, `cargo 1.95.0`. Baseline: `mosdns.service` active, MainPID `425`; pre-test `ss -lntup | sort` SHA-256 `44d2aad2643cb5c0e27ed90ddbc7d14d1403b0239c0aae9baf8b3bc44f3d4a39`. Existing port 53 remained owned by that process.
- Candidate was reviewed commit `016103f3c21ed2d659694ce10e64aaf24b5c2767` (parent `abeeb3e3bfb4458588430b83bfbd9280b359d37d`); product sources were not edited. The Rust-only archive was SHA-256 `afbc5829a566afa7486ff0ee6eb60e96a7c13e8684ff92521325b04f61784b98` locally and remotely. `rust/Cargo.lock` was blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f`, SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`, 38,673 bytes.
- The release CLI was built on the remote host with Cargo only; no frontend or Go build ran. Build exited 0 in 1m15s. `mosdns` was ELF x86_64, 4,584,440 bytes, SHA-256 `cbcc7f3785330cf3bfd099e2c397f2f012f26858296f8cce16d1212fdfacab2f`. The CLI binary was not launched in this supplement.
- To isolate the exact matcher from the broader `domain:local.test` rule, a temporary test was appended only in the extracted remote source tree. Its rule fixture contained only `full:local.only.test`, plus an invalid root-relative decoy; config was derived from the committed `ROOT_CONFIG`/`ROUTES_CONFIG` include chain. It sent `local.only.test A` and `sub.local.only.test A` through real UDP/audit-on and TCP/audit-off listeners. The exact request must reach the local peer (`192.0.2.21`); the non-exact suffix must reach the default peer (`192.0.2.22`). Both peer fixtures used counted loopback UDP sockets; this supplement does not claim TCP upstream coverage. Base test source hash was `f077b4347d5915ecb5ab02dca5897fb8e037dc4eee2b79274334464cf16c1fa1`; temporary augmented test source hash was `f8cf9bc766300186a6fa3eeee740948fd55efdbb68a7b06947f7fedc44409849`; the temporary process-owner runner hash was `3fd96e21c12a51fd2deac86f0833cc3f7094e04a5f69319df473610d29e52990`.

The one-off test and runner above were removed during that run's cleanup. The exact-rule count/behavior was re-executed below with preserved source and output so the result is independently reviewable; use the reproducible rerun as the current evidence.

### Commands and result

~~~sh
git archive --format=tar.gz 016103f3c21ed2d659694ce10e64aaf24b5c2767 rust > /tmp/mosdns-phase5b-a6-src.EiavCY/source.tar.gz
ssh mosdns-rust 'mkdir -p /tmp/mosdns-phase5b-a6.XMycgT/source /tmp/mosdns-phase5b-a6.XMycgT/run'
scp /tmp/mosdns-phase5b-a6-src.EiavCY/source.tar.gz mosdns-rust:/tmp/mosdns-phase5b-a6.XMycgT/source.tar.gz
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-a6.XMycgT && tar -xzf source.tar.gz -C source'
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-a6.XMycgT && sha256sum source.tar.gz source/rust/Cargo.lock'
ssh mosdns-rust 'CARGO_TARGET_DIR=/tmp/mosdns-phase5b-a6.XMycgT/target CARGO_BUILD_JOBS=2 cargo build --manifest-path /tmp/mosdns-phase5b-a6.XMycgT/source/rust/Cargo.toml -p mosdns-native-host --release --locked'
scp /tmp/mosdns-phase5b-a6-src.EiavCY/exact-rule-test.rs mosdns-rust:/tmp/mosdns-phase5b-a6.XMycgT/exact-rule-test.rs
ssh mosdns-rust 'cat /tmp/mosdns-phase5b-a6.XMycgT/exact-rule-test.rs >> /tmp/mosdns-phase5b-a6.XMycgT/source/rust/native-host/tests/slice3_composition.rs'
ssh mosdns-rust 'CARGO_TARGET_DIR=/tmp/mosdns-phase5b-a6.XMycgT/target CARGO_BUILD_JOBS=2 cargo test --manifest-path /tmp/mosdns-phase5b-a6.XMycgT/source/rust/Cargo.toml -p mosdns-native-host --test slice3_composition --no-run --locked'
scp /tmp/mosdns-phase5b-a6-src.EiavCY/run-owned.py mosdns-rust:/tmp/mosdns-phase5b-a6.XMycgT/run/run-owned.py
ssh mosdns-rust 'python3 /tmp/mosdns-phase5b-a6.XMycgT/run/run-owned.py /tmp/mosdns-phase5b-a6.XMycgT /tmp/mosdns-phase5b-a6.XMycgT/target/debug/deps/slice3_composition-2f26e2878285f663'
~~~

The temporary test binary was 96,458,904 bytes, SHA-256 `7741852ea28fce22b394dd59974fa0cde09c0ea47883cab0af59d0b790257c60`. The complete focused `slice3_composition` binary ran sequentially: **12 passed, 0 failed, 0 ignored**. Existing cases rechecked include the block/qtype-65 no-peer responses, local cache miss/hit and peer deltas, parent continuation/default route, UDP/audit-on records, TCP/audit-off no-record behavior, and config include/source-path checks.

The added exact-rule case observed UDP listener `127.0.0.1:57111`, local peer `127.0.0.1:57110`, default peer `127.0.0.1:58273`; exact name → .21, suffix → .22, counters local/default `1/1`, audit records `2`. TCP listener `127.0.0.1:34329` used the same peers; the same two results increased cumulative counters to `2/2`, and audit records remained `0`.

- Test executable PID `426290`, starttime ticks `37749070`, exe `/tmp/mosdns-phase5b-a6.XMycgT/target/debug/deps/slice3_composition-2f26e2878285f663`, PPID `426289`, process group `426289`; exit `0`, reaped and absent. No TERM/KILL was sent. Test fixture directories and listener/peer ports were absent after the run.
- Post-run service remained active with MainPID `425`; listener snapshot SHA-256 remained exactly `44d2aad2643cb5c0e27ed90ddbc7d14d1403b0239c0aae9baf8b3bc44f3d4a39`. The verified remote temp root was removed. No production configuration or process was changed; no performance PASS is claimed.
- Operational corrections: `rg` was absent on the remote host, so process inventory used `ps`, `systemctl`, and `ss`; a first cleanup assertion had a shell quoting error before deletion, then the corrected identity/port/baseline checks passed. Build/test failures in this supplement: none.

### Reproducible exact-rule rerun and owned-process record (2026-09-28)

- Source is the previously reviewed/tested product commit `016103f3c21ed2d659694ce10e64aaf24b5c2767`; archive hash `afbc5829a566afa7486ff0ee6eb60e96a7c13e8684ff92521325b04f61784b98`, `Cargo.lock` hash `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` (38,673 bytes), and unmodified `slice3_composition.rs` hash `f077b4347d5915ecb5ab02dca5897fb8e037dc4eee2b79274334464cf16c1fa1`.
- The appended test is preserved at [`research/a6-exact-rule-test.rs`](research/a6-exact-rule-test.rs), SHA-256 `8b82fa7004bd55059b9f581049dfd842417a5dd280dbdb28d881a58ace341bd2`. It builds the file-backed include chain with only `full:local.only.test` in `config/sub_config/rules/local.txt` and an invalid root-relative decoy at `config/rules/local.txt`; `local.only.test A` must route to `.21`, and `sub.local.only.test A` to `.22`. It asserts one local and one default peer call after UDP/audit-on, cumulative two each after TCP/audit-off, two UDP audit records, and zero TCP records. The test uses the existing counted UDP loopback peers; this is not TCP-upstream coverage.
- The identity/cleanup runner is preserved at [`research/a6-run-owned.py`](research/a6-run-owned.py), SHA-256 `383f43bd5ac7cb1600d6df1382490f8014618c52118be9430977361067451923`. It launches the test binary in its own process group, records PID/starttime/executable/PPID/group, only sends TERM/KILL after rechecking that identity on timeout, stores stdout and JSON, and checks all printed listener/peer ports plus fixture directories and before/after listener snapshot hashes.

~~~sh
git archive --format=tar.gz 016103f3c21ed2d659694ce10e64aaf24b5c2767 rust > /tmp/mosdns-phase5b-a6-repro-src.p7vyyA/source.tar.gz
scp /tmp/mosdns-phase5b-a6-repro-src.p7vyyA/source.tar.gz .trellis/tasks/09-27-rust-phase5b-config-sequence-composition/research/a6-exact-rule-test.rs .trellis/tasks/09-27-rust-phase5b-config-sequence-composition/research/a6-run-owned.py mosdns-rust:/tmp/mosdns-phase5b-a6-repro.XIHMEh/
ssh mosdns-rust 'tar -xzf /tmp/mosdns-phase5b-a6-repro.XIHMEh/source.tar.gz -C /tmp/mosdns-phase5b-a6-repro.XIHMEh/source'
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-a6-repro.XIHMEh && sha256sum source.tar.gz source/rust/Cargo.lock source/rust/native-host/tests/slice3_composition.rs a6-exact-rule-test.rs a6-run-owned.py'
ssh mosdns-rust 'cat /tmp/mosdns-phase5b-a6-repro.XIHMEh/a6-exact-rule-test.rs >> /tmp/mosdns-phase5b-a6-repro.XIHMEh/source/rust/native-host/tests/slice3_composition.rs'
ssh mosdns-rust 'CARGO_TARGET_DIR=/tmp/mosdns-phase5b-a6-repro.XIHMEh/target CARGO_BUILD_JOBS=2 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test --manifest-path /tmp/mosdns-phase5b-a6-repro.XIHMEh/source/rust/Cargo.toml -p mosdns-native-host --test slice3_composition --no-run --locked'
ssh mosdns-rust 'python3 /tmp/mosdns-phase5b-a6-repro.XIHMEh/a6-run-owned.py /tmp/mosdns-phase5b-a6-repro.XIHMEh /tmp/mosdns-phase5b-a6-repro.XIHMEh/target/debug/deps/slice3_composition-33af4ebe833654d9'
scp mosdns-rust:/tmp/mosdns-phase5b-a6-repro.XIHMEh/a6-exact-rule-test.log mosdns-rust:/tmp/mosdns-phase5b-a6-repro.XIHMEh/a6-exact-rule-run.json .trellis/tasks/09-27-rust-phase5b-config-sequence-composition/research/
ssh mosdns-rust 'rm -rf /tmp/mosdns-phase5b-a6-repro.XIHMEh'
~~~

The first wrapper attempt omitted `--nocapture`, so the test passed 12/12 but its wrapper could not parse the endpoint markers. After enabling capture, a second parser correction was needed because Rust's test progress prefix shares the UDP marker's line. The final captured run passed **12 passed, 0 failed, 0 ignored**. Compiled test binary: 12,694,392 bytes, SHA-256 `fca43645c333ad60428b0f9cc55f1a6d776c17bba50b1de1cb245702b7cebfc1`. Preserved output [`research/a6-exact-rule-test.log`](research/a6-exact-rule-test.log) has SHA-256 `9257b0bb2bd5a2c1c7ee8e03d039453a32eb98e18fd7ce762a785b4ca0a208b2`; [`research/a6-exact-rule-run.json`](research/a6-exact-rule-run.json) has SHA-256 `8423643d268684ac14225e6cbca0af249bbf398305770b465a78092020755cb4`.

The final run used PID `439475`, starttime `38296003`, PPID `439473`, process group `439475`, and executable `/tmp/mosdns-phase5b-a6-repro.XIHMEh/target/debug/deps/slice3_composition-33af4ebe833654d9`; exit code was 0, final identity was absent, and no TERM/KILL was sent. UDP listener/local peer/default peer were `127.0.0.1:59688`, `127.0.0.1:41697`, `127.0.0.1:38815`; TCP listener was `127.0.0.1:40027` with the same peers. Runner found zero leaked ports and zero fixture directories. Its sorted `ss -Hlntup` snapshot hash stayed `17a694daa8a6cfdeff606df95c3b89b5b4babb9a3373aca45d6b0e43ecb19d18`; an independent `ss -lntup | sort` before/after hash stayed `44d2aad2643cb5c0e27ed90ddbc7d14d1403b0239c0aae9baf8b3bc44f3d4a39`. `mosdns.service` remained active at PID `425`. The remote temporary root was removed and `/tmp` returned to 208 KiB used. No production process/configuration changed; no performance PASS is claimed.

### User-selected C2C review of the previous A6 evidence range (2026-09-28)

- In **Rust MosDNS测试进度**, C2C reviewed `bcac20374312d5bf875164f87673224b9da2a796..4fc737aa0dffc8e92ed878577b9c8a4131568077` and returned `FINAL: FAIL`.
- `P2-1`: the `task.json` summary called scoped review the sole remaining item while omitting the dedicated remote fault E2E variant. Corrections above now explicitly defer that variant and distinguish it from workspace cancellation/close regressions.
- `P2-2`: exact-rule source and runner were absent, leaving only hashes. The reproducible source, runner, test output, and run JSON are now preserved under `research/a6-exact-rule-*`, and the test has been rerun against the recorded product source.
- The exact committed follow-up range review is pending; task status remains `in_progress`.

## Full Rust workspace on Linux (2026-09-28, `mosdns-rust`)

### Source, target and command

- Candidate branch/head at run time: `rust`, `bcac20374312d5bf875164f87673224b9da2a796` (parent `82953751bdde89fa3fc2244cea2a86be4f6a3d06`). Rust sources are unchanged from the previously reviewed/tested `016103f3c21ed2d659694ce10e64aaf24b5c2767`; `git diff 016103..bcac203 -- rust` is empty. The workspace archive contained `rust/`, `tests/phase5a-baseline/configs/`, and `tests/phase5a-baseline/workloads/`. Corrected archive SHA-256: `ac16bccb2ddcb0b3d573a83d56bc760f0e2296d6e5226e0c0707f2285ad33a3a`; remote `Cargo.lock` SHA-256: `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`; `slice3_composition.rs` SHA-256: `f077b4347d5915ecb5ab02dca5897fb8e037dc4eee2b79274334464cf16c1fa1`.
- Target was reached only via `ssh mosdns-rust` (Linux x86_64, rustc/cargo 1.95.0). No frontend or Go build ran. The full-suite run used a unique temporary source/target root and loopback test fixtures. Existing production listeners remained owned by `mosdns.service`; the workspace tests use their own test fixtures and did not bind production ports.

~~~sh
git archive --format=tar.gz HEAD rust tests/phase5a-baseline/configs tests/phase5a-baseline/workloads > source-with-workloads.tar.gz
scp source-with-workloads.tar.gz mosdns-rust:/tmp/mosdns-phase5b-workspace.1iYapG/source-with-workloads.tar.gz
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-workspace.1iYapG && tar -xzf source-with-workloads.tar.gz -C .'
ssh mosdns-rust 'cd /tmp/mosdns-phase5b-workspace.1iYapG && CARGO_TARGET_DIR=/tmp/mosdns-phase5b-workspace.1iYapG/target CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test --manifest-path /tmp/mosdns-phase5b-workspace.1iYapG/rust/Cargo.toml --workspace --no-fail-fast --locked'
~~~

The successful run exited 0: **60 test targets, 896 passed, 0 failed, 0 ignored**. The focused targets in the complete run were `slice2_config` 12/12, `slice3_composition` 11/11, `w1_tcp` 5/5, `w1_udp` 4/4, `w2_cache` 6/6 and `w3_routing` 6/6. Workspace tests also exercised cancellation, close, shutdown, deadlines and rebind behavior; the 23-test `slice3_quic` target took 400.22 seconds in the serial run. This is Rust-workspace regression evidence, not a claim that every feature has a standalone remote E2E. Successful log SHA-256: `8ea273ebb7b0d12cdfb590960fff29db337bc6aab4b4dad9407b51c554eed495`.

### Failed attempts, process ownership and cleanup

- Initial reduced source archive SHA-256 `efb8290f11ff2cfa007860af333558100078f206be96e033a905299de4c9087a` omitted `tests/phase5a-baseline/workloads/routing.jsonl`. `cargo test --workspace --no-fail-fast --locked` exited 101 during compile; zero test targets ran and no listener was opened. Log SHA-256: `e897eb81a0c054a2e83b9a70e0a5c3b9644db6a9d3dd25475e642be0b5923dec`.
- After adding the config and workload fixtures, the default debug build exhausted the remote `/tmp` 2 GiB tmpfs while linking (`No space left on device`, linker bus error); Cargo exited 101 before any test target ran. The target occupied 1.9 GiB and was removed before retry. Log SHA-256: `d00f6febf472b8c07a1302c2a9f5271aa0befb6ae2ef35a18ca25ea2264f5de2`. The remote root filesystem also reported 0 bytes available during preflight; the retry kept its target in `/tmp`, used test debug info off, disabled incremental compilation and limited Cargo to one build job.
- Successful Cargo PID `431231` (started 2026-09-28 01:58:57 server local time) exited 0 and was absent afterward. Its long-running `slice3_quic` test process PID `435815` also ended and was absent. Final `mosdns.service` state stayed active with MainPID `425`; the sorted `ss -lntup` before/after SHA-256 stayed `44d2aad2643cb5c0e27ed90ddbc7d14d1403b0239c0aae9baf8b3bc44f3d4a39`. The test workspace/target and logs were removed from the remote temporary root; `/tmp` returned to 208 KiB used. No production configuration or process was changed; no performance PASS is claimed.
- Setup/collection helper errors were corrected and separated from test results: the first local compound transfer command had unmatched quoting; a post-extract `du` checked for a nonexistent `source/` directory; the optional `awk` start-time print failed after Cargo had printed its PID; and the first log-copy `scp` used an invalid remote-source form. They did not change product sources or invalidate a test result. The two actual Cargo failures above are retained separately from the successful run.

### Prior Codex review result for the A6 evidence supplement

- Codex thread `002reviewer` reviewed exactly `bcac20374312d5bf875164f87673224b9da2a796..4fc737aa0dffc8e92ed878577b9c8a4131568077` and returned `FINAL: PASS` on 2026-09-28.
- This is a historical bootstrap-review result only. The user-selected C2C reviewer later reviewed that exact A6 range and returned `FINAL: FAIL` with P2-1/P2-2; that newer scoped verdict controls until a C2C re-review passes.
- Trellis task status remains `in_progress`; no finish/archive/journal action was run.

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

No production configuration/service was changed. This evidence covers Linux CLI TCP/audit-off and real-listener UDP/audit-on checks, including the exact file-backed full-rule branch. The subsequent complete Linux Rust workspace run is recorded above. No performance PASS is claimed.

## Scope control

模块可为本链 direct call/cache continuation/常用 reject 做必要小改动，不因 revision 1 文件限制退回 planning。新 runtime/动态插件框架、多 cache 嵌套、完整 EDNS/新协议/API/生产仍延期；重大新增需求更新规划。保留已停止 5A 结论；正式实验冻结阈值/重跑规则不套到普通功能修复。
