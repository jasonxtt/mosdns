# Source audit for the next bounded task (2026-09-24)

## Confirmed anchors

| Concern | Current source | Planning implication |
| --- | --- | --- |
| Strict listener config | `rust/native-host/src/config.rs`, `compile_listener` | `enable_audit: true` currently rejects before I/O; preserve all other strict subset checks. |
| Native terminal paths | `rust/native-host/src/udp.rs`, `tcp.rs`, `execution.rs` | Listener knows send/cancel, execution knows cache and final upstream; one layer alone cannot make a correct audit entry. |
| Host resource owner | `rust/native-host/src/assembly.rs` | Attach bounded observer to the one native host owner; avoid a global singleton or Go bridge. |
| Go audit shape | `coremain/audit.go`, `pkg/server_handler/entry_handler.go` | Product fields include final response/routing; current Go audit collection occurs through the listener flag and can silently drop on full channel. Copy the contract, not that loss mode. |
| Coverage gate | `docs/rust/feature-coverage.md`, C08/C16 | Basic 5A evidence cannot close complete audit API or profiler compatibility. |
| Phase scope | `docs/ai/rust-rewrite-plan.md`, Phase 5A–5D | 5A requires basic audit/metrics; 5C owns complete management/API, 5D full-system performance. |
| First native comparison | `docs/rust/phase5a-native-comparison.md` | 12/21 three-pair groups; no objective overload point, indeterminate recovery, and no multi-core conclusion. Measure observability overhead only under valid offered load. |

## Decision inventory

- User intent already established: pure Rust-native final host; DNS correctness, latency, concurrency, stability, and effective throughput first; memory secondary; Linux amd64 primary. The user requested that this planning be made executable and has not authorized implementation in this turn.
- This task's bounded 5A surface is typed Rust snapshots, with the existing YAML flag enabling in-memory capture. Full public audit, Prometheus, and WebUI semantics are explicitly deferred to 5C. No additional product decision blocks writing this proposal; the planning summary must be reviewed before `task.py start`.
- The collector's exact fixed histogram buckets and performance regression budget are technical values to freeze in the execution manifest before official candidate runs, not values to select after seeing measurements.
