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

## Go field and metric discovery

`coremain/audit.go:AuditLog` currently exposes `client_ip`, `query_type`,
`query_name`, `query_class`, `query_time`, `duration_ms`, `trace_id`,
`response_code`, response flags/answers, `domain_set`, `effective_tag`,
`matched_group`, `final_sequence`, `final_upstream`, `upstream_targets`,
`selected_upstream`, and `matched_rule_source`. These are discovery fields;
the typed 5A Rust snapshot deliberately does not promise the 5C JSON/API shape.
The current Go record does not provide the new lifecycle/provenance split in
the reviewed form, and its collector may drop records on a full channel.

The Go metrics registry adds the `mosdns_` prefix. Current relevant plugin
families are:

- `metrics_collector`: `mosdns_metrics_collector_query_total`,
  `mosdns_metrics_collector_err_total`, `mosdns_metrics_collector_thread`,
  and `mosdns_metrics_collector_response_latency_millisecond` (constant label
  `name`).
- `cache`: `mosdns_cache_query_total`, `mosdns_cache_hit_total`,
  `mosdns_cache_lazy_hit_total`, and `mosdns_cache_size_current` (constant
  label `tag`).
- `forward`: `mosdns_forward_query_total`, `mosdns_forward_err_total`,
  `mosdns_forward_thread`, `mosdns_forward_response_latency_millisecond`,
  `mosdns_forward_conn_opened_total`, and `mosdns_forward_conn_closed_total`
  (constant labels `upstream` and `tag`).

These Go names are not reused as a Rust export schema: the active Rust work
adds read-only typed snapshots only. Full Prometheus name/label parity remains
in Phase 5C.

## Frozen pre-change native state

- Pre-change runtime source commit: `605c30577b79d397b5695618dbd2980e550ca6f3`;
  native-host runtime source and tests are unchanged through the reviewed
  planning commit `64d9cf51468ced016f813bcde84d08337a1c1872`.
- The strict compiler accepts the existing `enable_audit: false` graphs and
  rejects boolean `true` in `compile_listener` before assembly or socket bind.
  Unknown fields, wrong YAML types, unsupported graphs, and extra listeners
  stay rejected.
- No native observer/snapshot API or terminal query event exists before this
  task. The exact W1/W2/W3 inputs and probe constraints are in
  `performance-manifest.md` and its SHA-256 sidecar.

## Verified listener constraint

The strict compiler rejects a second listener plugin and requires exactly one listener per host. W1 selects UDP or TCP; W2 and W3 select UDP. The supported 5A host therefore cannot mix listener audit flags. Treat the sole listener's flag as the capture choice for all admitted queries, and keep the duplicate-listener rejection as a regression check.

## Decision inventory

- User intent already established: pure Rust-native final host; DNS correctness, latency, concurrency, stability, and effective throughput first; memory secondary; Linux amd64 primary. The user authorized the reviewed Slices 0–3 on 2026-09-25.
- This task's bounded 5A surface is typed Rust snapshots, with the existing YAML flag enabling in-memory capture. Full public audit, Prometheus, and WebUI semantics are explicitly deferred to 5C. The approved PRD/design/implement set is active; it does not authorize deployment or a capacity claim.
- The fixed histogram bucket edges are frozen in the PRD and design before runtime implementation. The performance manifest will repeat those edges and freeze the performance regression budget before official candidate runs; neither may be selected after seeing measurements.
