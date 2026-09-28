# Execution plan (not executed)

This task is in `planning`. The checklist below is authorization-gated: do not run `task.py start`, connect to `mos-test`, build, or launch peers/sidecars until the user explicitly authorizes execution after reviewing the plan.

## Step 0 — freeze inputs and preflight

- [ ] Confirm the candidate is exactly `016103f3c21ed2d659694ce10e64aaf24b5c2767`; the prior composition exact-range review passed. Verify `git ls-tree` reports `rust/Cargo.lock` as blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f`, verify the candidate archive contains it, and verify extracted SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`. If any check differs, STOP before starting peers or sidecars. Do not change or rebuild from current dirty files.
- [ ] Freeze the four proposed execution defaults in `design.md` with the user before any host-side execution: config-package snapshot only; same-config Go comparison best-effort/non-gating/no Go build; controlled peers are the hard oracle; TERM plus complete OS-resource release is sufficient rollback.
- [ ] Recheck the read-only config-package source commit and hashes captured in `research/canary-inputs.md`; these sibling-repository identifiers were not independently verified by the exact-range C2C review. Derive and record the sanitized reduction from the exact execution-time inputs. If any hash differs or the package snapshot is not safely accessible, stop before starting any sidecar and revise/review the plan.
- [ ] Connect only with `ssh mos-test`; verify hostname/kernel/architecture. Record the relevant current MosDNS service active state/MainPID and `ss` listeners. Do not use `mosdns-rust` or a direct IP as a substitute for this host.
- [ ] Confirm the Rust toolchain and required shell/network tools (`python3`, `tar`, `sha256sum`, `ssh`, `scp`) are already available. Do not install or upgrade packages. Confirm all proposed high loopback ports are unoccupied; select and recheck ports immediately before start. If isolation or a dependency cannot be established, record `STOP / environment invalid`.
- [ ] Create a new remote `/tmp/mosdns-rust-canary.<unique>/`; keep all source, binary, config/rules, peer helper, logs, PID receipts, and temporary evidence inside it. Record its path and verify no command writes to `/cus/mosdns`.
- [ ] Freeze derived paths exactly: `config/udp.yaml` and `config/tcp.yaml` include `sub_config/routes.yaml`; that included file uses `files: [rules/local.txt]`; the fixture is `config/sub_config/rules/local.txt`. Derive sanitized configs from the verified inputs and keep them under the temp root.
- [ ] Record the Go comparison availability decision. Only an already available Go binary with a safely isolated identical reduced config may be compared; do not build Go or query a different-config service as a hard oracle.

## Step 1 — transfer exact source and build once

- [ ] Produce a temporary archive from the exact candidate's Rust workspace, including tracked `rust/Cargo.lock`; record local SHA-256 and verify the archive listing contains the lockfile.
- [ ] Transfer through the `mos-test` SSH alias into the new temp root and compare the remote archive SHA-256 before extraction. After extraction, verify the committed lockfile's expected Git blob identity and SHA-256 before Cargo runs.
- [ ] Record remote `rustc --version` and `cargo --version`; build the Rust-native host once with the committed lockfile:

  ```sh
  CARGO_TARGET_DIR="$ROOT/target" cargo build \
    --manifest-path "$ROOT/source/rust/Cargo.toml" \
    -p mosdns-native-host --release --locked
  ```

- [ ] Record lockfile SHA-256, build exit status, binary SHA-256, byte size, and ELF architecture. Do not rebuild between the UDP and TCP runs.

## Step 2 — run the bounded functional canary

- [ ] Freeze a sanitized config-reduction map from the verified source snapshot. Place UDP/TCP config, included route file, and relative rules under the temp root; verify both configs differ only in listener type/address, `enable_audit`, and TCP `idle_timeout`.
- [ ] Before starting peers or any sidecar, add a temporary integration-test harness only at `$ROOT/source/rust/native-host/tests/canary_preflight.rs` (not in the repository or candidate archive). It must call `HostAssembly::from_config_file` for both configs and drop the assemblies without `run_udp`/`run_tcp`. Run `MOSDNS_CANARY_CONFIG_ROOT="$ROOT/config" cargo test --manifest-path "$ROOT/source/rust/Cargo.toml" -p mosdns-native-host --test canary_preflight --locked`; record the harness source hash and result. This must prove include/provider path resolution without binding listeners or opening upstream connections. If it fails, classify as `STOP / fixture-or-harness invalid`, correct the sanitized fixture, and repeat before sidecar startup.
- [ ] Keep the owning shell alive and start the two owned deterministic peers on free loopback high ports. Record peer PIDs, transport, addresses, log paths, initial counters, `/proc/<pid>/stat` starttime, resolved `/proc/<pid>/exe`, PPID, and process group.
- [ ] Before any Rust sidecar starts, self-test local UDP and default TCP peers directly using one known DNS request each. Verify transport framing, echoed ID/question, `.21`/`.22` response, and exactly one matching counter increment. Reset both counters and record zero baseline. A peer self-test failure is `STOP / harness invalid`, not Rust `FAIL`.
- [ ] Snapshot the existing MosDNS service state/PID/listeners immediately before the first canary process.
- [ ] Start only the Rust UDP/audit-on sidecar with an owned PID. Immediately record its `/proc/<pid>/stat` starttime, resolved `/proc/<pid>/exe`, PPID, process group, and exact command/temp-root identity. Confirm its only listener is its assigned loopback port. Reset peer counters and run Q1–Q6 in the order in `design.md`, recording request ID, response ID, QNAME/QTYPE, RCODE, answers/TTL, and counter deltas after each request.
- [ ] Before TERM or KILL to any peer or Rust process, revalidate PID starttime, executable, PPID, and process group against the launch receipt while the owning shell is alive. If the PID is gone, only record/wait; if any identity differs, do not signal it and stop for investigation. TERM only the matching recorded Rust UDP process; wait up to 10 seconds. If still alive, repeat the full identity check before KILL, record FAIL, and continue cleanup. Confirm PID and listener are absent before proceeding.
- [ ] Reset the two peer counters, start only the Rust TCP/audit-off sidecar on its assigned loopback port, record the same process identity fields, then run the same Q1–Q6 oracle via TCP. Apply the same owned-process stop and cleanup checks.
- [ ] If the pre-approved optional same-config Go comparison is safely available without building Go or affecting the existing service, run the identical corpus in a separate owned process and compare semantic fields only. Otherwise record the concrete unavailable reason and keep it non-gating.
- [ ] When the TCP run and optional Go comparison have finished—or immediately if either Rust run fails or the task enters an early STOP path—shut down every still-running canary-owned process: both DNS peers and any isolated Go server, as well as any remaining Rust sidecar. For each process, keep the owner shell alive, revalidate PID/starttime/executable/PPID/process group against its launch receipt, send TERM only on an exact match, and wait up to 10 seconds. If it remains, repeat the full identity check before KILL, record the run as FAIL, then verify the PID and its listening socket are gone. If identity differs, do not signal or remove files still in use; stop for investigation. An early failure/STOP skips all not-yet-started sidecars and goes directly to this cleanup path.
- [ ] After each run and at final cleanup, compare the existing service MainPID/active state/listeners with baseline. Any unexplained change means STOP investigation; never repair it by altering the service.

## Step 3 — evidence, classification, and cleanup

- [ ] In this `implement.md`, record the remote host identity, source archive/lock/binary hashes, toolchain, config snapshot identity and hashes (explicitly distinguishing locally captured planning values from execution-time rechecks), sanitized reduction map, pre-I/O config harness hash/result, peer self-test result, exact commands, UDP and TCP result/counter tables, Go comparison result or skip reason, failures/retries, each owned process's PID/starttime/executable/PPID/process-group identity and exit status, and before/after service/listener snapshots.
- [ ] Clearly label each result as remote canary measurement or prior local evidence. List remote workspace/legacy suites, fault/cancel/close matrices, full config compatibility, audit extraction, performance, capacity/recovery, and soak as unrun/deferred unless actually performed under separate authorization.
- [ ] Only after the process cleanup step proves all owned PIDs absent and all owned ports released, capture compact sanitized evidence and remove the remote temp root and local transfer archive. If process identity cannot be verified or any process/socket remains, preserve files it may use, record the cleanup failure, and stop for investigation. Recheck the existing service baseline.
- [ ] Classify the result as PASS, FAIL, or STOP using `design.md`. If a product-code defect appears, stop and create a separate remediation plan; do not edit product code here.
- [ ] Do not report performance PASS or production readiness. Do not deploy, change port 53, modify systemd, write `/cus/mosdns`, or archive/close this task without the appropriate separate authorization.

## Planning validation and review

- [x] `python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-28-rust-mos-test-native-sidecar-canary` passed.
- [x] `git diff --check` passed for the planning edits.
- [x] C2C reviewer-only exact-range review returned `FINAL: PASS` for `016103f3c21ed2d659694ce10e64aaf24b5c2767..82953751bdde89fa3fc2244cea2a86be4f6a3d06`.
- [x] Report the exact reviewed base/head and remaining execution gate: this task stays in `planning`; the canary requires separate explicit execution authorization.

No canary, build, remote command, or product test has been run as part of writing this plan.
