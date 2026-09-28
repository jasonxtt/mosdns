# Canary planning inputs

Status: the plan-change bootstrap review passed on 2026-09-28. Canary attempt 1 stopped before any Rust listener because the raw listener-row comparator was too strict. Retry attempt 2 passed every per-query response and counter oracle, then exposed an invalid `2,2` aggregate check after logs were reset between transports; same-chat C2C iteration 8 approved `STOP / harness invalid` and correction to `2,1`. Retry attempt 3 with that approved correction returned `PASS`: both transports passed all six queries, owned resources were released, and the MosDNS service baseline was unchanged. Same-chat C2C iteration 9 returned `FINAL: PASS` on the worktree evidence; final acceptance of the exact committed range remains pending.

## Verified repository facts

- Candidate: `016103f3c21ed2d659694ce10e64aaf24b5c2767` is the current Rust branch `HEAD` when this plan was written; its parent is `abeeb3e3bfb4458588430b83bfbd9280b359d37d`.
- Exact-candidate lockfile verification run locally on 2026-09-28: `git ls-tree -l 016103f3c21ed2d659694ce10e64aaf24b5c2767 -- rust/Cargo.lock` returned blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f` (38,673 bytes); `git rev-parse 016103f3c21ed2d659694ce10e64aaf24b5c2767:rust/Cargo.lock` returned the same blob; `git archive 016103f3c21ed2d659694ce10e64aaf24b5c2767 rust/Cargo.lock | tar -tf -` listed both `rust/` and `rust/Cargo.lock`; hashing `git show 016103f3c21ed2d659694ce10e64aaf24b5c2767:rust/Cargo.lock` returned SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. The plan uses this committed lockfile with Cargo `--locked`; it must not generate a new lockfile. Repeat archive membership and extracted file hash checks during execution.
- `rust/native-host/src/cli.rs` accepts only `mosdns start -c <config>` (or `--config`). `rust/native-host/src/main.rs` prepares the assembly and calls `assembly.run()`; there is no CLI audit snapshot/dump command.
- TCP listener config requires positive `idle_timeout`; UDP does not accept that field. The plan uses separate sequential configs and includes the field only for TCP.
- `rust/native-host/src/assembly.rs` exposes an in-process audit snapshot API, and `rust/native-host/tests/slice3_composition.rs` covers audit-on/off behavior in process. Neither fact proves that a standalone CLI exposes those records.
- The repository has no standalone native-host build script. The existing `scripts/build-rust-experimental.sh` builds the transitional Go/cgo binary and is not the build path for this pure Rust-native canary. The native binary is built from `rust/Cargo.toml` with Cargo.
- `.trellis/tasks/archive/2026-09/09-27-rust-phase5b-config-sequence-composition/research/example-compositions.md` records the reduced composition and independent oracle: include/provider files, qtype 65 reject 0, block reject 3, named child calls, cache/local forward, default route, local exact rule, and deterministic `.21`/`.22` answers. It explicitly notes that its sample ports/config are illustrative, not executed as written.
- The 5B task's current evidence records selected Linux E2E on `mosdns-rust` and a C2C exact-range `FINAL: PASS`. That earlier E2E does not replace the independent host preflight and canary specified here.

## Frozen configuration-source snapshot

The sibling config-package repository at `/Users/tom/github/file` was available read-only. Its observed commit is `28c64936a0a1889a02dec4617e258e01d5501866`, and its worktree was clean. On 2026-09-28, the seven relevant source-file hashes below were rechecked under `mosdns/config/config_lite_all/` and matched these values. The checks disclosed only paths, commit/status, and hashes; no raw config payload was copied. This remains input-provenance evidence only, not a remote test result. Recheck the package commit and each hash immediately before deriving the canary fixture; if they differ or cannot be re-read, stop and revise/review the plan.

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

## Read-only target preflight (2026-09-28)

- The `mosdns-rust` SSH alias connected and returned hostname `mosdns-rust`, Linux `7.0.9-x64v3-xanmod1` x86_64, user `root`.
- `mosdns.service` was active/running with MainPID `425`, process starttime `373`, executable `/usr/local/bin/mosdns`. Service-owned UDP and TCP wildcard ports observed: `53`, `2222`, `3077`, `3099`, `3111`, `3333`, `4444`, `7777`, `8888`; TCP also had `9099`. Port 53 is existing service state and is forbidden to the canary.
- rustc `1.95.0 (59807616e 2026-04-14)` and cargo `1.95.0 (f2d3ce0bd 2026-03-21)` plus `python3`, `tar`, `sha256sum`, `ssh`, and `scp` were present.
- These are review-time observations only. Refresh hostname, service identity/state/listeners, tool versions, helpers, and proposed free loopback ports before any remote build/test or canary-owned process. Always use the SSH alias, never a direct IP.

## Measured and pending inputs

- Attempt 1 measured the exact source archive and extracted lockfile hashes, a successful release build, both non-listening config loads, and independent UDP/TCP peer self-tests. It stopped before any Rust listener or DNS query; all owned peers and ports were confirmed gone, the service baseline remained stable, and its temporary root was removed.
- Retry attempt 2 reverified the exact source, built the same binary, passed both non-listening config loads and peer self-tests, then completed both UDP and TCP six-query runs. All 12 wire responses and per-query peer deltas matched the expected cases. Final peer logs contained `local_udp=2, default_tcp=1`, consistent with the TCP run after its required reset; the controller incorrectly expected `2,2` and returned `FAIL`. Both sidecars and peers were terminated with verified identities; no cleanup error or service-baseline change occurred, and the temporary root was removed.
- Retry attempt 3 rebuilt the exact candidate, passed the non-listening config harness, repeated the peer self-tests, and returned controller `PASS` with the approved final totals `local_udp=2, default_tcp=1`. Both six-query runs passed; all owned processes and ports were released, the service listener identity remained stable, and the temp root was removed after evidence capture.
- Pending: final review of attempt 3's exact committed host-level evidence. Full workspace/legacy suites, fault/cancel/close matrices, full config compatibility, audit extraction, and performance/capacity/recovery/soak remain unrun.
- Do not read or use `/cus/mosdns` live state as a substitute for the frozen read-only package snapshot. All project build/test verification uses the `mosdns-rust` SSH alias.
- The optional same-config Go comparator, if any, must be decided before execution. Do not build Go; unavailable or unsafe comparison is non-gating by default and must be recorded with a reason.

## User-approved execution defaults (2026-09-28)

| Decision | Proposed default |
| --- | --- |
| Config source | Read-only config-package snapshot; never live managed state |
| Go comparator | Best-effort only when the same reduced config can run safely and no Go build is needed; otherwise skip non-gating |
| Upstream oracle | Canary-owned deterministic local peers are hard gate; no public resolver dependency |
| Stop/rollback | TERM owned process, require PID/socket release; no claim of graceful signal-aware shutdown |

The user explicitly approved all four defaults and separately authorized execution. The `mosdns-rust` target change passed bootstrap review; same-chat C2C iteration 7 approved the target/evidence correction and iteration 8 approved the retry-2 STOP classification and aggregate correction. Retry attempt 3 returned `PASS` with the corrected `2,1` totals. Final same-chat host-level acceptance of this execution evidence remains pending.
