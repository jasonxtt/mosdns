# Execution plan

The user explicitly authorized execution on 2026-09-28 using all four defaults listed in `design.md`, and then selected `mosdns-rust` as the VM for this and all project build/test verification. The original C2C review approved the earlier `mos-test` plan only; this host-target change must pass the bootstrap reviewer before any remote build or test. After that review passes, proceed under the existing execution authorization. Send the final host-level canary evidence to the same C2C project conversation for acceptance.

Run all project build/test verification on the `mosdns-rust` SSH alias. Local source inspection and task-document validation are planning work; no alternate VM or direct IP may be used. The checklist below remains ordered and gated by its technical preflight results. If a preflight requires STOP, do not start any sidecar.

## Step 0 — freeze inputs and preflight

- [x] Confirmed the candidate is exactly `016103f3c21ed2d659694ce10e64aaf24b5c2767`; the prior composition exact-range review passed. The archive contained `rust/Cargo.lock`; its Git blob and extracted SHA-256 matched `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f` and `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. The canary used this exact source, not the dirty checkout.
- [x] Freeze the four execution defaults in `design.md`: read-only config-package snapshot; same-config Go comparison best-effort/non-gating/no Go build; controlled peers as the hard oracle; TERM plus verified release of every owned OS resource as rollback, without claiming graceful shutdown. User authorized these defaults on 2026-09-28.
- [x] Recheck the read-only config-package source commit and the seven frozen file hashes captured in `research/canary-inputs.md`; on 2026-09-28 they matched commit `28c64936a0a1889a02dec4617e258e01d5501866` and the recorded SHA-256 values in the clean sibling `file` repository. These local read-only hashes are planning evidence, not VM execution evidence.
- [x] Rechecked the read-only snapshot commit and seven source hashes before deriving the sanitized reduction; all matched the recorded inputs in `research/canary-inputs.md`. Attempt 3 records hashes for both derived configs, the included route file, and relative rules.
- [x] A read-only pre-review probe through `ssh mosdns-rust` identified hostname `mosdns-rust`, Linux `7.0.9-x64v3-xanmod1` x86_64, and `mosdns.service` active/running with MainPID `425`, starttime `373`, executable `/usr/local/bin/mosdns`. The observed service listeners were recorded in `design.md`; its wildcard port-53 listeners are baseline-only.
- [x] Refreshed host identity and the service/listener baseline through the `mosdns-rust` alias for the canary run. Attempt 3 records identical before/after service state, MainPID/starttime/executable, and stable listener identity; no other VM or direct IP was used.
- [x] The read-only pre-review probe found rustc `1.95.0 (59807616e 2026-04-14)`, cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, and `python3`, `tar`, `sha256sum`, `ssh`, `scp` already present.
- [x] The pre-review probe recorded the preinstalled toolchain and helpers. Attempt 3 built with `--locked` and recorded binary identity; a read-only follow-up on `mosdns-rust` reported the same rustc/cargo versions. No packages were installed or upgraded. The run used rechecked, free loopback ports. The controller result does not contain a separate execution-time toolchain-version field, so the follow-up is supplemental rather than a contemporaneous version receipt.
- [x] Created a unique remote `/tmp/mosdns-rust-canary.<unique>/` and kept canary files and receipts inside it. Attempt 3 records the isolated root and confirms it was removed after evidence capture; no `/cus/mosdns` write occurred.
- [x] Used the frozen derived paths: `config/udp.yaml` and `config/tcp.yaml` include `sub_config/routes.yaml`; that included file uses `files: [rules/local.txt]`; the fixture is `config/sub_config/rules/local.txt`. Both config hashes and the included route/rule hashes are recorded.
- [x] Recorded the optional Go comparison as skipped because no safe same-config comparator was validated; it remained non-gating and no Go build ran.

## Step 1 — transfer exact source and build once

- [x] Produced a temporary archive from the exact candidate's Rust workspace, including tracked `rust/Cargo.lock`; archive SHA-256 and lockfile listing were verified.
- [x] Transferred through the `mosdns-rust` SSH alias, verified the archive hash before extraction, and verified the committed lockfile's expected Git blob identity and SHA-256 before Cargo ran.
- [x] Built the Rust-native host once with the committed lockfile:

  ```sh
  CARGO_TARGET_DIR="$ROOT/target" cargo build \
    --manifest-path "$ROOT/source/rust/Cargo.toml" \
    -p mosdns-native-host --release --locked
  ```

- [x] Recorded the lockfile SHA-256, successful build status, binary SHA-256, byte size, and x86_64 ELF architecture. The UDP and TCP runs used that same build.

## Step 2 — run the bounded functional canary

- [x] Derived sanitized configs from the verified inputs and kept UDP/TCP config, included route file, and relative rules under the temporary root. The two configs differed only in the planned listener/audit/TCP timeout fields.
- [x] Before starting peers or sidecars, ran the temporary, non-listening `HostAssembly::from_config_file` harness for both configs; it passed and its source hash is recorded.
- [x] Kept the owning shell alive and started only the two controlled peers on free loopback high ports. Attempt 3 records each peer's PID, starttime, executable, parent/process group, transport, address, and counter evidence.
- [x] Self-tested the UDP and TCP peers directly before Rust startup; framing, ID/question, answer, and one-request counter checks passed, then counters reset to zero.
- [x] Captured service state and listeners before the first canary process and after cleanup. Comparison used stable protocol/state/endpoint/owner identity; raw listener hash was supporting evidence.
- [x] Started the Rust UDP/audit-on sidecar on its owned loopback port and recorded its process identity. All six query results and peer deltas are recorded.
- [x] Revalidated owned process identity before TERM. Both sidecars and peers exited after TERM, no forced KILL or identity mismatch occurred, and their listeners were released.
- [x] Reset peer counters and ran the Rust TCP/audit-off sidecar on its owned loopback port; all six query results and peer deltas are recorded.
- [x] Skipped the optional Go comparison for the recorded non-gating reason; no Go build or production-service oracle was used.
- [x] Shut down all canary-owned peers and sidecars with identity-checked TERM; every PID and owned port was released.
- [x] The existing MosDNS service state/MainPID/starttime/executable and stable listener identity matched before and after; no service repair or modification was performed.

## Step 3 — evidence, classification, and cleanup

- [x] Recorded the remote host, source archive/lock/binary identity, config hashes, sanitized reduction, non-listening harness hash/result, peer self-tests, both query/counter runs, optional-Go skip, process identities/exits, cleanup, and before/after service/listener snapshot in this file. The recorded pre-review and post-run toolchain versions and the lack of a separate attempt-time version field are stated explicitly.
- [x] Distinguished remote measurements from earlier local inputs and listed workspace/legacy suites, fault/cancel/close matrices, full config compatibility, audit extraction, performance, capacity/recovery, and soak as unrun.
- [x] Captured compact evidence after proving all owned PIDs/ports released, removed the remote temporary root and local transfer archive, and rechecked the service baseline.
- [x] Classified attempt 3 as `PASS` under `design.md`; the earlier invalid-controller attempt remains `STOP / harness invalid`, not a Rust product failure.
- [x] Made no performance or production-readiness claim and made no deployment, port-53, systemd, `/cus/mosdns`, or task-archive change.

## Planning validation and review

- [x] The earlier C2C reviewer-only exact-range review returned `FINAL: PASS` for `016103f3c21ed2d659694ce10e64aaf24b5c2767..82953751bdde89fa3fc2244cea2a86be4f6a3d06`. It approved the original host plan and does not cover the current `mosdns-rust` target change.
- [x] `python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-28-rust-mos-test-native-sidecar-canary` and `git diff --check` passed for the updated planning artifacts.
- [x] Spec-sync review found no reusable implementation convention to add under `.trellis/spec/`: the `2,2` counter mismatch was specific to this temporary controller's per-transport log reset and is documented with its correction in this task's evidence.
- [x] Bootstrap plan-change review returned `FINAL: PASS` from `确认002reviewer` (`01a0d43d-d0aa-7401-af0f-2ca3a45ba519`, host `local`). Snapshot: branch `rust`, HEAD `c4a785fea6e66531921396def362d0c44d4a1666`, working-tree patch SHA-256 `9ea51551398c804dbf76c1a7d4dd58ded1a5b9468244b4de52c0bdc78f1a88e6`; reviewed only `design.md`, `implement.md`, `prd.md`, `research/canary-inputs.md`, `task.json`, and `docs/rust/next-stage-plan.md`. The reviewer ran no tests or remote commands.
- [x] After bootstrap reviewer `PASS`, completed the already-authorized canary and submitted the evidence for same-chat worktree review; iteration 9 returned `FINAL: PASS`.
- [ ] Final acceptance remains pending: commit this scoped task, resolve and verify the dedicated `c2c-web` reviewer binding, then review the exact committed canary range and compact evidence. The worktree review does not substitute for that committed-range review.

The VM target-change preflight used the Codex reviewer named above, rather than the user's requested same C2C Project conversation. Iteration 9 occurred after execution and cannot be described as pre-execution approval for that change. The task also entered `in_progress` without `automation.py authorize`; no original pre-start snapshot exists, and none will be backdated. A separate review-only task will seek prospective acceptance of the preserved result. The original task stays historically incomplete until that review passes and a supported supersession transition is recorded.

The bootstrap review completed before the first execution attempt. The first attempt built the exact candidate, passed the non-listening config preflight and peer self-tests, then stopped before any Rust sidecar because the baseline comparator treated full raw `ss` rows as identity. See the attempt record below; no Rust DNS query was sent.

## Superseded `mos-test` execution attempt — 2026-09-28

- User authorized execution with the four proposed defaults unchanged; the task was started with `python3 ./.trellis/scripts/task.py start .trellis/tasks/09-28-rust-mos-test-native-sidecar-canary` after plan validation.
- Local exact-candidate checks still resolve `016103f3c21ed2d659694ce10e64aaf24b5c2767:rust/Cargo.lock` to blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f`, 38,673 bytes, SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`; the candidate archive listing includes `rust/Cargo.lock`.
- C2C local connection diagnostics passed before the attempt. The read-only `ssh mos-test` identity probe did not establish a connection. The first socket remained in `SYN_SENT`; a bounded retry using `ssh -o ConnectTimeout=8 -o BatchMode=yes mos-test 'hostname -f; uname -srm'` exited 255 with `Operation timed out`.
- **Classification: STOP / environment invalid.** No SSH session was established, so no remote command ran. Host identity, service/PID/listener baseline, config snapshot hashes, toolchain, and free ports could not be verified. No local transfer archive, remote temp root, build, peer, sidecar, or canary-owned process/socket was created. No service state was changed.
- This historical STOP applied only to the original `mos-test` target and is superseded by the user-selected `mosdns-rust` target. It is not a Rust product failure or PASS. The current canary target requires the plan-change review and a fresh preflight.

## `mosdns-rust` target selection and plan-change preflight — 2026-09-28

- The user directed that all project tests use the VM configured by the `mosdns-rust` alias. The plan now uses only that alias; the previous `mos-test` timeout is retained above as historical evidence.
- Read-only probes through `ssh mosdns-rust` succeeded. The hostname, Linux kernel/architecture, service state/MainPID/starttime/executable, wildcard listener baseline, toolchain versions, and helper availability are recorded in `design.md`. They are refreshed before each attempt.
- All seven frozen config-package file hashes matched at commit `28c64936a0a1889a02dec4617e258e01d5501866`; the sibling repository was clean. No config payload was copied.
- The exact candidate remains `016103f3c21ed2d659694ce10e64aaf24b5c2767`, and all original functional cases and process/service isolation boundaries remain unchanged.
- The bootstrap plan-change review returned `FINAL: PASS`. First execution attempt stopped before Rust sidecar startup because its raw `ss` row comparator was too strict; owned peers exited cleanly and the original service remained active with PID 425. A stable listener identity comparator is now specified before retry.


## Canary attempt 1 — pre-listener harness stop — 2026-09-28

- Transfer archive SHA-256 `6d976f7abbf577e07d9c8e16cbf991912bf280ccfce9658bb25976938b1cbfc4`; exact candidate lockfile Git blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f`, SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. One release build on the exact source succeeded: x86_64 ELF, 4,584,440 bytes, SHA-256 `cbcc7f3785330cf3bfd099e2c397f2f012f26858296f8cce16d1212fdfacab2f`.
- Sanitized UDP/TCP configs both passed `HostAssembly::from_config_file` via temporary `cargo test --locked`: 1 passed, 0 failed. Harness SHA-256 `b5d4ce85d5ee095310bc40879a63e6b0d740e9e8cf78b0de3dec6d71fa08c35a`.
- Independent controlled peer self-tests passed: UDP echoed ID/question and `.21` answer; TCP framing echoed ID/question and `.22`; one request per peer was observed, then both logs were reset to zero.
- Before any Rust listener started, the controller detected a difference in raw service-listener rows and stopped as `STOP / harness invalid`. No Rust sidecar or canary DNS query was launched. Both owned peers received TERM after PID/starttime/executable/PPID/process-group identity checks, exited with status 0, and their listeners were released. Temp root was removed.
- Controller's before/after snapshots both showed `mosdns.service` active/running, PID `425`, starttime `373`, executable `/usr/local/bin/mosdns`, and 19 service-owned listeners with SHA-256 `6fdb074ec5f0e4c3d5be196c753efa290bf77ec8a04da08fb11ad2352967a716`. Three subsequent read-only samples showed the same stable protocol/state/endpoint/owner identity. This points to an over-strict raw-row comparison; no persistent service change was observed. This attempt is not a Rust PASS or FAIL.
- The controller has been corrected to compare stable listener identity fields. On STOP, capture compact evidence first, then remove the temporary root once every owned PID and port is confirmed gone and the service baseline is verified. Retain the root beyond evidence capture only if process identity or resource cleanup cannot be established. Retry uses a fresh unique root and re-runs the exact-source build and preflight before any bounded sidecar.

## Canary attempt 2 — controller aggregate-accounting stop — 2026-09-28

- The exact candidate archive SHA-256 remained `6d976f7abbf577e07d9c8e16cbf991912bf280ccfce9658bb25976938b1cbfc4`; extracted lockfile SHA-256 was `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. The `--locked` release build succeeded with the same x86_64 binary hash `cbcc7f3785330cf3bfd099e2c397f2f012f26858296f8cce16d1212fdfacab2f` (4,584,440 bytes). Both config preflight cases passed (1 passed, 0 failed); harness SHA-256 `b5d4ce85d5ee095310bc40879a63e6b0d740e9e8cf78b0de3dec6d71fa08c35a`.
- The direct UDP/TCP peer self-tests passed and counters reset to zero. Both Rust sidecars started on `127.0.0.1:48155` (UDP, audit enabled) and `127.0.0.1:48156` (TCP, audit disabled). All six response and per-query peer-delta checks passed in each run: block/NXDOMAIN and HTTPS/no-answer made no peer calls; local miss returned `.21` with one local request; the new-ID repeat returned cached `.21` with no request; the exact rule returned `.21` with one local request; default routing returned `.22` with one default request.
- Raw controller result: `FAIL`, `RuntimeError: unexpected peer request totals after the two ordered runs`. The final aggregate assertion expected `local_udp=2, default_tcp=2`, but logs are reset before each transport run, so the post-cleanup files contain only the final TCP run's correct totals: `local_udp=2, default_tcp=1`. This contradicts the controller assertion, not the per-query records. Current assessment: `STOP / harness invalid` pending same-chat review; no Rust behavior failure was observed, and this attempt is not a canary PASS. Proposed correction: keep per-query checks unchanged and make the final assertion reflect the last-run counters (`2,1`) or explicitly aggregate the two recorded runs.
- All owned processes were identity-checked before TERM and released their ports: UDP peer PID `456165` and TCP peer PID `456168` exited `0`; UDP sidecar PID `456173` and TCP sidecar PID `456181` exited `-15` after TERM. No forced KILL, identity mismatch, or cleanup error was recorded. `mosdns.service` remained active/running with PID `425`, starttime `373`, executable `/usr/local/bin/mosdns`, and stable service-listener identity hash `d3d964bd539308b66da3d6861347865618128e4d3f807f092535fe2156d00a9e`. The owned ports were released and the temporary root was removed after the compact result was emitted.
- Same-chat C2C iteration 8 returned `FINAL: PASS` for the execution record and correction. The approved retry changes only the final aggregate expectation to `local_udp=2, default_tcp=1`; all per-query checks remain unchanged. No product code, managed config, service, firewall, or production state was changed.

## Canary attempt 3 — corrected aggregate PASS — 2026-09-28

- Ran the corrected retry on the `mosdns-rust` alias from a fresh `/tmp` root. Exact candidate archive SHA-256 `6d976f7abbf577e07d9c8e16cbf991912bf280ccfce9658bb25976938b1cbfc4`; extracted lockfile SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`; release build exited 0 with x86_64 binary hash `cbcc7f3785330cf3bfd099e2c397f2f012f26858296f8cce16d1212fdfacab2f` (4,584,440 bytes). The non-listening config preflight passed (1 passed, 0 failed; harness SHA-256 `b5d4ce85d5ee095310bc40879a63e6b0d740e9e8cf78b0de3dec6d71fa08c35a`), and both config/include/relative-rules checks passed.
- The UDP and TCP controlled-peer self-tests passed and reset counters to zero. The UDP/audit-on sidecar at `127.0.0.1:48155` and TCP/audit-off sidecar at `127.0.0.1:48156` each passed all six cases. Across both runs: `blocked.test A` returned NXDOMAIN without peer calls; `other.test HTTPS` returned NOERROR/no answer without peer calls; `a.local.test A` miss returned `.21` with one local request; a new-ID repeat returned cached `.21` with no peer request; `local.only.test A` returned `.21` with one local request; `other.test A` returned `.22` with one default request. Per-run totals were `local_udp=2, default_tcp=1`; the final aggregate assertion now matches the counter reset between transports.
- Raw controller classification `PASS`, exit 0, no error, and no cleanup errors. Peer PID `461609` (starttime `40706231`) and PID `461612` (`40706242`) exited 0 after identity-checked TERM; UDP sidecar PID `461617` (`40706255`) and TCP sidecar PID `461625` (`40706260`) exited `-15` after identity-checked TERM. No forced KILL or identity mismatch occurred; all owned ports were released. `mosdns.service` remained active/running with PID `425`, starttime `373`, executable `/usr/local/bin/mosdns`, and service-listener identity hash `d3d964bd539308b66da3d6861347865618128e4d3f807f092535fe2156d00a9e` before and after. The temporary root was removed after compact evidence capture.
- The optional same-config Go comparison was skipped as non-gating; no Go build ran. Full workspace/legacy suites, fault/cancel/close matrices, full config compatibility, audit extraction, performance/capacity/recovery/soak remain unrun. No product code, managed config, service, firewall, or production state changed.
- Same-chat C2C iteration 8 approved the aggregate correction. Iteration 9 returned `FINAL: PASS` for the current worktree documents and evidence. The canary is not complete until the exact committed range receives final acceptance through the verified reviewer binding.
- The sanitized [attempt-3 controller evidence](research/attempt3-controller-evidence.json) was independently checked against the local raw output (source SHA-256 `5c519eea2974c28a452c1cf718e810d3d75d8e1e28579663d1f85e6ed7880d48`): both six-case response/counter runs, unchanged service fields, four owned-process exits, and cleanup matched this summary. The sanitized file SHA-256 is `34a0743bd5ab5cb360e0489dacff88bb716418facf0dc9aea54c0da614d1a58f`. It is a projection of the controller output, not an independent remote rerun or proof of deferred suites.
