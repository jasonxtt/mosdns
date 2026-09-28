# Rust-native isolated mosdns-rust sidecar canary

## Goal

After the user's explicit execution authorization, verify that the reviewed Rust-native composition candidate at `016103f3c21ed2d659694ce10e64aaf24b5c2767` can serve a small, configuration-derived DNS chain as an isolated Linux sidecar on the `mosdns-rust` SSH alias, while the existing MosDNS service and its managed state remain untouched. The user selected this VM for all project build/test verification; use this alias for later project test tasks as well.

The canary is a bounded operational and functional check. It is not a production cutover, full config-package compatibility claim, or performance gate.

## Requirements

- Use only the `mosdns-rust` SSH alias and verify the remote host identity before any sidecar starts. Record existing MosDNS service state, MainPID, and listener baseline; verify them again after each run and at final cleanup. Never substitute a direct IP or another VM.
- Build only the Rust workspace/native host from a clean source archive of the exact candidate SHA. The candidate tree must contain `rust/Cargo.lock` with Git blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f` (SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`); verify the archive listing and extracted file hash on both ends before building with `--locked`. If either check differs, stop before sidecar startup. Record toolchain and binary provenance. Do not build the frontend or Go.
- Derive a minimal config from the read-only `config_lite_all` snapshot identified in `research/canary-inputs.md`. Freeze the derived paths as `config/udp.yaml` (and `config/tcp.yaml`) including `sub_config/routes.yaml`, with that included file declaring `files: [rules/local.txt]` and the fixture located at `config/sub_config/rules/local.txt`. Before any listener starts, load both files through a temporary, non-listening `HostAssembly::from_config_file` preflight and require successful include/provider resolution. Preserve the package's top-level include and relative file-path layout, using a sanitized temporary multi-rule fixture for the supported `domain_set` matcher; do not claim compatibility with the source package's unsupported `domain_set_light` provider. Preserve ordered sequence/direct-child calls, reject behavior, child cache, local route, and default route. Recheck the frozen source hashes before deriving. Do not copy or write `/cus/mosdns` state or store credentials/private upstreams in this repository.
- Use only canary-owned processes, a new `/tmp` root, and free high ports bound to `127.0.0.1`. No canary-owned process may bind port 53. An existing service listener on port 53, if present, must remain unchanged as part of the service baseline. Do not change a service/unit/config, redirect traffic, or install/upgrade packages. If these isolation conditions cannot be proved, stop as `STOP / environment invalid` without starting a sidecar.
- Run two sequential sidecars with the same sequence, providers, cache, forward behavior, controlled peers, and query corpus: UDP listener with `enable_audit: true`, then TCP listener with `enable_audit: false`. The TCP config may add only its required positive `idle_timeout` and its listener transport/address.
- In both runs, check six deterministic cases: `blocked.test A` → NXDOMAIN with zero peer calls; `other.test HTTPS` (qtype 65) → NOERROR/no answer with zero peer calls; `a.local.test A` → local peer answer on miss; repeat with a new ID → same answer and no peer-count change; `local.only.test A` → exact local rule and local peer; `other.test A` → parent continuation and default peer answer.
- Use canary-owned deterministic peers as the hard routing/cache oracle. Before starting Rust, self-test each UDP/TCP peer directly with a known request and verify transport framing, echoed ID/question, `.21`/`.22` answer, and exactly one counter increment; reset counters and only then continue. A failed peer self-test is `STOP / harness invalid`, not Rust `FAIL`. If a post-run aggregate controller assertion is internally inconsistent with the recorded per-query deltas because of reset/accounting logic, classify it as `STOP / harness invalid`, preserve the raw result, and review/correct before retry; do not label a product failure from an invalid assertion alone. A same-config isolated Go comparison is optional and non-gating unless explicitly made a prerequisite before execution; do not build Go. A different-config production service or public resolver is not a hard oracle.
- Prove rollback for every owned peer and Rust process: keep the owning shell alive and record PID, `/proc/<pid>/stat` starttime, resolved `/proc/<pid>/exe`, PPID, and process group at launch. Before TERM or KILL, recheck starttime, executable, PPID, and process group; if the PID vanished, only record/wait, and if its identity changed, never signal it and stop for investigation. Confirm exit and owned ports released, preserve the original service baseline, collect compact evidence, then remove the canary temporary root. Do not claim internal graceful shutdown behavior.
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

- [x] Execution is separately authorized; exact candidate, config source snapshot, reduction, and host isolation inputs are frozen before starting a sidecar.
- [x] Clean exact-SHA source archive contains the expected committed lockfile blob/hash and matches on `mosdns-rust`; both configs pass the non-listening include/provider preflight; one Rust-native binary is built successfully with `--locked`, and its hash/architecture are recorded.
- [x] UDP/TCP peer helpers pass their independent framing/ID/question/answer/counter self-tests before Rust starts.
- [x] UDP/audit-on and TCP/audit-off each pass all six DNS and peer-counter oracles without unexpected timeout, SERVFAIL, crash, or unexplained semantic difference.
- [x] Every owned process is signaled only after PID/starttime/executable identity revalidation; all owned PIDs exit, every owned port is released, canary files are removed after evidence capture, and the existing service state/PID/listeners match the baseline.
- [x] Results distinguish remote measurements from prior local evidence and enumerate all deferred/unrun work. No performance PASS is claimed.
