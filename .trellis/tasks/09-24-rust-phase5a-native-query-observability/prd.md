# Rust Phase 5A native query observability

Status: planning. This task is ready for review, not authorized for implementation. Base branch: `rust`.

## Goal and value

Give the isolated Rust-native W1/W2/W3 host trustworthy, bounded evidence of what happened to each admitted DNS query. The query path must report the final response, effective route, cache outcome, and request lifecycle without changing DNS behavior. Basic metrics remain available when detailed audit capture is off. This closes the *basic observability* part of the Phase 5A host only; C08's complete audit/API contract remains Phase 5C.

## Confirmed baseline

- `rust/native-host/src/config.rs` rejects `enable_audit: true` for the strict W1/W2/W3 YAML subset. The current listener and execution paths have no terminal query event or metric snapshot.
- `rust/native-host/src/execution.rs` can perform multiple external legs for W3 and a cache hit for W2. A final upstream cannot be inferred from the first dispatch, the last configured plugin, or response bytes alone.
- Go's `AuditLog` contains query/response and final routing fields. Its current `AuditCollector.Collect` can discard an event on a full channel without reporting that loss; this is discovery evidence, not a Rust behavior to reproduce.
- The reviewed first native performance comparison is limited: 12/21 scenario-stage groups had three valid Go/Rust pairs, no objective overload point was established, and the 2-vCPU VM cannot prove multi-core capacity. The task must not claim a performance win or release readiness.

## Requirements

R1. Preserve the existing strict W1 UDP/TCP, W2 UDP cache, and W3 UDP routing YAML subset. Accept `enable_audit: true` as well as `false` on those listeners; do not silently accept any other new field, plugin, or graph. The flag controls detailed per-query audit retention; basic counters do not depend on it. Do not add a new YAML or CLI configuration surface for this task.

R2. For every parsed, admitted query, account for exactly one terminal outcome: response sent, transport send failure, cancellation before send, or internal no-response failure. Malformed/unadmitted input retains its current DNS behavior and must be separately countable without being misreported as a completed query. Audit entries, when enabled, contain a timestamp, client address, transport, question name/type/class, elapsed time, final response code or explicit no-response outcome, cache hit/miss/not-applicable, final sequence, and effective final upstream when one exists. A canceled or failed send must never be labeled delivered.

R3. Derive route/cache fields from the executed request, not YAML guesses. W2 warm hits report a cache hit and no upstream for that query. W3 `A`, `B→A`, and `B→C` paths report A, A, and C respectively as final upstream, while retaining the actual ordered leg path for diagnostics. A failed B/A/C leg, timeout, local SERVFAIL, and valid upstream SERVFAIL have distinct terminal classifications where the present execution path can distinguish them. No intermediate B response is treated as final.

R4. Provide host-owned, read-only Rust snapshots for basic metrics and retained audit records so native-host integration tests and the later 5C management layer can inspect them. At minimum metrics cover admitted/completed/canceled/send-failed/malformed counts, current in-flight count, response-code totals, cache hit/miss totals, forward-attempt outcomes by configured upstream, and a fixed-bucket end-to-end latency distribution. Labels are bounded by enums or configured upstream identities; question names, clients, and trace IDs are never metric labels. Snapshot reads must not mutate query state.

R5. Keep audit retention bounded with the Go default capacity of 100,000 records for the supported 5A subset. Retention eviction is explicit through an `evicted_total` count in the snapshot; there is no hidden full-channel drop or unbounded queue. Enabled audit must preserve one terminal record per admitted query until the documented retention boundary; disabling audit must avoid retaining per-query names and client addresses. Query processing must not wait on disk, network, or management work to record an event.

R6. Preserve W1/W2/W3 response bytes, upstream leg counts/order, deadlines, cancellation, shutdown/rebind, and cache publication behavior. Concurrent requests must not mix identities or routes. Observation state is owned by the native host and drains with its existing lifecycle; no Go/cgo bridge or second execution engine may be introduced. The observer interface must not force future multi-core runtime work to retain the current `Rc`/current-thread design.

R7. Validate the new enabled/disabled paths on Linux amd64 with a frozen source/input manifest and correctness oracles. Capture a small same-host Rust-before/Rust-after probe for W1 TCP, W2 cold/warm, and W3 under valid offered load; report p50/p95/p99, correct-on-time throughput, CPU, RSS, and audit-on overhead with invalid-run reasons. Freeze thresholds before official candidate runs. Treat this as a regression check, not a new Go/Rust capacity conclusion or an optimization task.

## Acceptance criteria

- [ ] A1 (R1/R5): unchanged supported YAML with audit false still compiles; audit true compiles for W1 UDP/TCP, W2, and W3; unknown fields and unsupported graphs still reject before I/O. Audit-off snapshots contain no retained query/client data.
- [ ] A2 (R2/R3): deterministic tests prove exactly one terminal record/count for successful send, failed send, cancellation, and internal no-response paths. Malformed input does not create a false completed event. W1/W2/W3 entries match the actual final response, cache status, and per-request ordered upstream legs, including W3 A/B→A/B→C and upstream versus local SERVFAIL.
- [ ] A3 (R4/R5): metric counts reconcile with sent/received fixtures and audit records; snapshots are stable and bounded; a small test capacity demonstrates eviction with an exact `evicted_total` and no unexplained missing event. No sensitive value becomes a metric label.
- [ ] A4 (R6): mixed concurrent queries, deadline expiry, shutdown while an upstream is pending, and rebind preserve DNS and upstream-count oracles; in-flight returns to zero and all owners close. W1/W2/W3 audit-off regression suites remain green.
- [ ] A5 (R7): focused Rust/workspace checks and Linux amd64 E2E pass on a pinned commit. The frozen probe and report disclose offered-load validity, on/off overhead, CPU/RSS, and uncertainty. Any repeatable regression beyond the predeclared budget blocks review or receives an explicit corrective task; no unsupported capacity claim appears.
- [ ] A6: reviewer checks PRD/design/implementation scope, event semantics, retention behavior, performance evidence, and exact changed paths. Coverage/handover is updated only for the proven bounded 5A observability subset; task finishes and archives only after a final PASS.

## Out of scope and deferred gates

Complete Go audit v1/v2 JSON/API schema, audit start/stop/capacity/clear endpoints, persistent audit settings, Vue/compatibility UI, Prometheus `/metrics`, `metrics_collector`/`query_summary` plugin parity, other transports/listeners, arbitrary plugin graphs, production deployment, Go/Rust rerun, full overload/soak, multi-core runtime migration, hybrid retirement, and default release remain separate tasks. The task may add a test-only small retention capacity through host options; the product YAML shape stays fixed. Phase 5A is not fully closed by this one task or by the earlier limited performance comparison.
