# Canary planning inputs

Status: planning research only. No `mos-test` connection or canary execution occurred in this task.

## Verified repository facts

- Candidate: `016103f3c21ed2d659694ce10e64aaf24b5c2767` is the current Rust branch `HEAD` when this plan was written; its parent is `abeeb3e3bfb4458588430b83bfbd9280b359d37d`.
- Exact-candidate lockfile verification run locally on 2026-09-28: `git ls-tree -l 016103f3c21ed2d659694ce10e64aaf24b5c2767 -- rust/Cargo.lock` returned blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f` (38,673 bytes); `git rev-parse 016103f3c21ed2d659694ce10e64aaf24b5c2767:rust/Cargo.lock` returned the same blob; `git archive 016103f3c21ed2d659694ce10e64aaf24b5c2767 rust/Cargo.lock | tar -tf -` listed both `rust/` and `rust/Cargo.lock`; hashing `git show 016103f3c21ed2d659694ce10e64aaf24b5c2767:rust/Cargo.lock` returned SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. The plan uses this committed lockfile with Cargo `--locked`; it must not generate a new lockfile. Repeat archive membership and extracted file hash checks during execution.
- `rust/native-host/src/cli.rs` accepts only `mosdns start -c <config>` (or `--config`). `rust/native-host/src/main.rs` prepares the assembly and calls `assembly.run()`; there is no CLI audit snapshot/dump command.
- TCP listener config requires positive `idle_timeout`; UDP does not accept that field. The plan uses separate sequential configs and includes the field only for TCP.
- `rust/native-host/src/assembly.rs` exposes an in-process audit snapshot API, and `rust/native-host/tests/slice3_composition.rs` covers audit-on/off behavior in process. Neither fact proves that a standalone CLI exposes those records.
- The repository has no standalone native-host build script. The existing `scripts/build-rust-experimental.sh` builds the transitional Go/cgo binary and is not the build path for this pure Rust-native canary. The native binary is built from `rust/Cargo.toml` with Cargo.
- `.trellis/tasks/09-27-rust-phase5b-config-sequence-composition/research/example-compositions.md` records the reduced composition and independent oracle: include/provider files, qtype 65 reject 0, block reject 3, named child calls, cache/local forward, default route, local exact rule, and deterministic `.21`/`.22` answers. It explicitly notes that its sample ports/config are illustrative, not executed as written.
- The 5B task's current evidence records selected Linux E2E on `mosdns-rust` and a C2C exact-range `FINAL: PASS`. That is not remote evidence from `mos-test`; the new task must independently verify the target host and canary.

## Frozen configuration-source snapshot

The sibling config-package repository was available read-only during planning. Its observed commit was `28c64936a0a1889a02dec4617e258e01d5501866`; the relevant source files below were clean in that package worktree when inspected. These values are planning-captured external snapshot identifiers and were not independently verified by the subsequent exact-range C2C review, which covered only the `mosdns-rust` repository paths listed in its request. They are not execution evidence. Recheck the package commit and each file hash from the safely accessible read-only snapshot before canary execution; if they differ or cannot be re-read, stop and revise the plan. Hashes are recorded without copying raw/private config into Trellis.

| Source file (relative to `config/config_lite_all/`) | SHA-256 |
| --- | --- |
| `config_custom.yaml` | `2af449999863776c634067445593f67e33d1d2fa133c0df518f1f5e255a4e52a` |
| `sub_config/process_main.yaml` | `c7baeda711ee0248c238db57692982f2c397c2daf16a5a4a67e8d98b20cbb14a` |
| `sub_config/process_ot.yaml` | `b6e12854398d8c19eea92b0ad0466eae401cda0bd977dadf0653df9c5322b3ae` |
| `sub_config/forward_nocn.yaml` | `4d77e256083300e3822e9e943fdb26bd881fe951838e9bb6765aa63ff2019161` |
| `sub_config/forward_1.yaml` | `8b92fe3933130f74cbf189ea1e3c1b1e70b828687c9da73d9a9c83586c580812` |
| `sub_config/cache.yaml` | `051b8694027d174b5520cfc587d9072707c01aeab9dfe4301299be1f8417094b` |
| `sub_config/rule_set.yaml` | `174949fc2ffe01d34008d225863fe0d1a091a003164b09b92545620bbacebc7e` |

The root config declares relative top-level includes. `rule_set.yaml` uses relative file paths for provider files; those provider types include unsupported `domain_set_light`. The planned derived layout is fixed as `config/udp.yaml` (or `config/tcp.yaml`) including `sub_config/routes.yaml`; that included file declares `files: [rules/local.txt]`; the sanitized fixture is `config/sub_config/rules/local.txt`, resolved relative to the included YAML's directory. The fixture uses the supported `domain_set` syntax already recorded in the 5B composition oracle. This tests the relative path and multi-rule mechanics only; it does not claim source provider compatibility. Before listener startup, a temporary integration-test harness will call `HostAssembly::from_config_file` on both derived configs and drop the pre-I/O assemblies; its hash and `cargo test --locked` result will be recorded. Only selected source fragments are to be transferred/used, not the entire config package.

Before starting either Rust sidecar, the temporary peer harness must also pass independent direct-query self-tests against its local UDP and default TCP peer. The test checks transport framing, ID/question echo, deterministic `.21`/`.22` answers, and one counter increment per request, then resets both counters. A failure is `STOP / harness invalid`, not evidence against Rust.

## Inputs not measured yet

- Hostname, OS/kernel, architecture, MosDNS service unit/process/PID/listeners, available ports, preinstalled toolchain versions, helper availability, source archive checksum on both ends, build output, query results, peer counters, PIDs, and cleanup are all future `mos-test` measurements. Do not fill them from the prior `mosdns-rust` host evidence.
- Do not read or use `/cus/mosdns` live state as a substitute for the frozen read-only package snapshot.
- The optional same-config Go comparator, if any, must be decided before execution. Do not build Go; unavailable or unsafe comparison is non-gating by default and must be recorded with a reason.

## Proposed execution defaults to approve before start

| Decision | Proposed default |
| --- | --- |
| Config source | Read-only config-package snapshot; never live managed state |
| Go comparator | Best-effort only when the same reduced config can run safely and no Go build is needed; otherwise skip non-gating |
| Upstream oracle | Canary-owned deterministic local peers are hard gate; no public resolver dependency |
| Stop/rollback | TERM owned process, require PID/socket release; no claim of graceful signal-aware shutdown |

These are planning defaults, not permission to execute. The user may approve or adjust them before the task is started.
