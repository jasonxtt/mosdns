# Execution plan (not executed)

This task is in `planning`. The checklist below is authorization-gated: do not run `task.py start`, connect to `mos-test`, build, or launch peers/sidecars until the user explicitly authorizes execution after reviewing the plan.

## Step 0 — freeze inputs and preflight

- [ ] Confirm the candidate is exactly `016103f3c21ed2d659694ce10e64aaf24b5c2767`; the prior composition exact-range review passed. Do not change or rebuild from current dirty files.
- [ ] Freeze the four proposed execution defaults in `design.md` with the user before any host-side execution: config-package snapshot only; same-config Go comparison best-effort/non-gating/no Go build; controlled peers are the hard oracle; TERM plus complete OS-resource release is sufficient rollback.
- [ ] Recheck the read-only config-package source commit and hashes frozen in `research/canary-inputs.md`; derive and record the sanitized reduction from those exact inputs. If any hash differs or the package snapshot is not safely accessible, stop before starting any sidecar and revise/review the plan.
- [ ] Connect only with `ssh mos-test`; verify hostname/kernel/architecture. Record the relevant current MosDNS service active state/MainPID and `ss` listeners. Do not use `mosdns-rust` or a direct IP as a substitute for this host.
- [ ] Confirm the Rust toolchain and required shell/network tools (`python3`, `tar`, `sha256sum`, `ssh`, `scp`) are already available. Do not install or upgrade packages. Confirm all proposed high loopback ports are unoccupied; select and recheck ports immediately before start. If isolation or a dependency cannot be established, record `STOP / environment invalid`.
- [ ] Create a new remote `/tmp/mosdns-rust-canary.<unique>/`; keep all source, binary, config/rules, peer helper, logs, PID receipts, and temporary evidence inside it. Record its path and verify no command writes to `/cus/mosdns`.
- [ ] Record the Go comparison availability decision. Only an already available Go binary with a safely isolated identical reduced config may be compared; do not build Go or query a different-config service as a hard oracle.

## Step 1 — transfer exact source and build once

- [ ] Produce a temporary archive from the exact candidate's Rust workspace, including tracked `rust/Cargo.lock`; record local SHA-256.
- [ ] Transfer through the `mos-test` SSH alias into the new temp root and compare the remote archive SHA-256 before extraction.
- [ ] Record remote `rustc --version` and `cargo --version`; build the Rust-native host once with the committed lockfile:

  ```sh
  CARGO_TARGET_DIR="$ROOT/target" cargo build \
    --manifest-path "$ROOT/source/rust/Cargo.toml" \
    -p mosdns-native-host --release --locked
  ```

- [ ] Record lockfile SHA-256, build exit status, binary SHA-256, byte size, and ELF architecture. Do not rebuild between the UDP and TCP runs.

## Step 2 — run the bounded functional canary

- [ ] Freeze a sanitized config-reduction map from the verified source snapshot. Place UDP/TCP config, included route file, and relative rules under the temp root; verify both configs differ only in listener type/address, `enable_audit`, and TCP `idle_timeout`.
- [ ] Start the two owned deterministic peers on free loopback high ports. Record peer PIDs, transport, addresses, log paths, and initial counters.
- [ ] Snapshot the existing MosDNS service state/PID/listeners immediately before the first canary process.
- [ ] Start only the Rust UDP/audit-on sidecar with an owned PID. Confirm its only listener is its assigned loopback port. Reset peer counters and run Q1–Q6 in the order in `design.md`, recording request ID, response ID, QNAME/QTYPE, RCODE, answers/TTL, and counter deltas after each request.
- [ ] TERM only the recorded Rust UDP PID; wait up to 10 seconds. If still alive, KILL that PID, record FAIL, and continue cleanup. Confirm PID and listener are absent before proceeding.
- [ ] Reset the two peer counters, start only the Rust TCP/audit-off sidecar on its assigned loopback port, and run the same Q1–Q6 oracle via TCP. Apply the same owned-PID stop and cleanup checks.
- [ ] If the pre-approved optional same-config Go comparison is safely available without building Go or affecting the existing service, run the identical corpus in a separate owned process and compare semantic fields only. Otherwise record the concrete unavailable reason and keep it non-gating.
- [ ] After each run and at final cleanup, compare the existing service MainPID/active state/listeners with baseline. Any unexplained change means STOP investigation; never repair it by altering the service.

## Step 3 — evidence, classification, and cleanup

- [ ] In this `implement.md`, record the remote host identity, source archive/lock/binary hashes, toolchain, config snapshot identity and hashes, sanitized reduction map, exact commands, UDP and TCP result/counter tables, Go comparison result or skip reason, failures/retries, each owned PID's exit status, and before/after service/listener snapshots.
- [ ] Clearly label each result as remote canary measurement or prior local evidence. List remote workspace/legacy suites, fault/cancel/close matrices, full config compatibility, audit extraction, performance, capacity/recovery, and soak as unrun/deferred unless actually performed under separate authorization.
- [ ] Verify all owned PIDs are absent and owned ports are released. Capture compact sanitized evidence, then remove the remote temp root and local transfer archive. Recheck the existing service baseline.
- [ ] Classify the result as PASS, FAIL, or STOP using `design.md`. If a product-code defect appears, stop and create a separate remediation plan; do not edit product code here.
- [ ] Do not report performance PASS or production readiness. Do not deploy, change port 53, modify systemd, write `/cus/mosdns`, or archive/close this task without the appropriate separate authorization.

## Planning validation and review

- [ ] `python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-28-rust-mos-test-native-sidecar-canary`
- [ ] `git diff --check`
- [ ] Request C2C reviewer-only review against one fixed committed range covering only this planning task and `docs/rust/next-stage-plan.md`.
- [ ] After review, report the exact base/head SHA and any remaining user decision. Keep this task in planning until explicit execution authorization.

No canary, build, remote command, or product test has been run as part of writing this plan.
