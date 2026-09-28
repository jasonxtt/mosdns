# Rust-native isolated mos-test sidecar canary

## Goal

After explicit execution approval, verify that the reviewed Rust-native composition candidate at `016103f3c21ed2d659694ce10e64aaf24b5c2767` can serve a small, configuration-derived DNS chain as an isolated Linux sidecar on `mos-test`, while the existing MosDNS service and its managed state remain untouched.

The canary is a bounded operational and functional check. It is not a production cutover, full config-package compatibility claim, or performance gate.

## Requirements

- Use the `mos-test` SSH alias and verify the remote host identity before any sidecar starts. Record existing MosDNS service state, MainPID, and listener baseline; verify them again after each run and at final cleanup.
- Build only the Rust workspace/native host from a clean source archive of the exact candidate SHA. Verify the archive hash on both ends, use the committed `rust/Cargo.lock` with `--locked`, and record toolchain and binary provenance. Do not build the frontend or Go.
- Derive a minimal config from the read-only `config_lite_all` snapshot identified in `research/canary-inputs.md`. Preserve the package's top-level include and relative file-path layout, using a sanitized temporary multi-rule fixture for the supported `domain_set` matcher; do not claim compatibility with the source package's unsupported `domain_set_light` provider. Preserve ordered sequence/direct-child calls, reject behavior, child cache, local route, and default route. Recheck the frozen source hashes before deriving. Do not copy or write `/cus/mosdns` state or store credentials/private upstreams in this repository.
- Use only canary-owned processes, a new `/tmp` root, and free high ports bound to `127.0.0.1`. Never bind port 53, change a service/unit/config, redirect traffic, or install/upgrade packages. If these isolation conditions cannot be proved, stop as `STOP / environment invalid` without starting a sidecar.
- Run two sequential sidecars with the same sequence, providers, cache, forward behavior, controlled peers, and query corpus: UDP listener with `enable_audit: true`, then TCP listener with `enable_audit: false`. The TCP config may add only its required positive `idle_timeout` and its listener transport/address.
- In both runs, check six deterministic cases: `blocked.test A` → NXDOMAIN with zero peer calls; `other.test HTTPS` (qtype 65) → NOERROR/no answer with zero peer calls; `a.local.test A` → local peer answer on miss; repeat with a new ID → same answer and no peer-count change; `local.only.test A` → exact local rule and local peer; `other.test A` → parent continuation and default peer answer.
- Use canary-owned deterministic peers as the hard routing/cache oracle. A same-config isolated Go comparison is optional and non-gating unless explicitly made a prerequisite before execution; do not build Go. A different-config production service or public resolver is not a hard oracle.
- Prove rollback for every owned peer and Rust process: terminate only recorded PIDs, confirm exit, confirm owned ports are released, preserve the original service baseline, collect compact evidence, then remove the canary temporary root. Do not claim internal graceful shutdown behavior.
- Keep audit claims within the CLI surface: prove the audit-enabled and audit-disabled configurations start and that their wire results/peer counts match the expected behavior. The canary does not expose or claim external access to in-process audit snapshots.
- Do not modify product code in this task. If the canary reveals a product defect, record evidence, stop this task, and create a separate remediation task.
- Do not claim latency, throughput, capacity, recovery, soak, or any other performance PASS.

## Non-goals

- Full configuration-package compatibility or remaining 5B plugin/provider/protocol coverage.
- Remote workspace-wide, W1/W2/W3, fault, cancellation, close-variant, or soak matrices.
- New audit/metrics/API/WebUI surfaces, graceful signal-aware shutdown, runtime/`Send` redesign, or product-code fixes.
- Public upstream dependence, production service changes, port 53, systemd edits, or production deployment.
- Performance measurements or PASS claims.

## Acceptance Criteria

- [ ] Execution is separately authorized; exact candidate, config source snapshot, reduction, and host isolation inputs are frozen before starting a sidecar.
- [ ] Clean exact-SHA source archive and committed lockfile provenance match on `mos-test`; one Rust-native binary is built successfully with `--locked`, and its hash/architecture are recorded.
- [ ] UDP/audit-on and TCP/audit-off each pass all six DNS and peer-counter oracles without unexpected timeout, SERVFAIL, crash, or unexplained semantic difference.
- [ ] Every owned PID exits, every owned port is released, canary files are removed after evidence capture, and the existing service state/PID/listeners match the baseline.
- [ ] Results distinguish remote measurements from prior local evidence and enumerate all deferred/unrun work. No performance PASS is claimed.
