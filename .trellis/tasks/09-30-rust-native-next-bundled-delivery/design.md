# Design — Rust-native audit control, DNS card and audit panel

## Scope and ownership

This task extends the existing native host; it does not add a second HTTP
server, DNS runtime, query executor or observer. The current owner chain is
`HostAssembly` → shared DNS/HTTP supervisor → `QueryObserver` for request
facts, while `ApiServer` currently receives `Rc<CompiledConfig>` only
(`assembly.rs`, `observer.rs`, `api.rs`). Add a narrow host API state containing
the compiled-config view, an observer control/read handle, and an optional
host-level state root. Keep upstream catalog and DNS listener lifetime with
the existing supervisor. Existing `domain_set` routes remain on the same API
server and must pass their prior tests unchanged.

The host must retain the directory of the top-level config file for
`webinfo/audit_settings.json`. Included-YAML plugin directories are not a
substitute. The file-backed `from_config_file` path supplies this root; tests
that assemble from YAML text may supply an explicit temporary root through
host options. Without a state root, status/start/stop/clear/read endpoints may
work, but capacity mutation must reject visibly instead of writing in the
repository/current directory.

## Audit state and request lifecycle

Keep the static listener `enable_audit` gate. For an eligible request, retain
the minimal admission facts needed to build an audit record. At terminalization
under one observer-state synchronization point, read runtime `capturing` once:
when true, append a complete record and evict the oldest if capacity is full;
when false, skip the audit ring. A zero-capacity ring stores no records even
when capturing is true. In all cases, update the existing lifetime metrics
exactly once. This is a deliberate deterministic boundary in place of Go's
multiple asynchronous capture checks. It also corrects the current fixed
`audit_enabled` branch in `record_terminal` so a later start cannot demand
an audit context that admission did not create.

`start` and `stop` only flip runtime capturing. `clear` takes the same state
synchronization point as record commit and drops retained records/derived
aggregates; it leaves capacity and capturing untouched. `set_capacity` must
serialize with other capacity updates. After persistence succeeds, it takes
the observer-state point once to replace capacity and clear retained records.
An in-flight request that terminalizes later can become the first record of
the new ring. Audit stats and windows read a coherent view of this retained
state. Existing `MetricsSnapshot` is a separate lifetime view and is never
mapped into v2 audit statistics.

Keep DNS hot-path locks short. HTTP request tasks must not hold observer locks
while awaiting, formatting JSON, or writing files. Choose a bounded-cost
snapshot/aggregation strategy for 400000 retained records during Slice 0 and
verify under concurrent real DNS queries. `stats/windows` may derive from a
bounded snapshot outside the lock; maintain minimal incremental aggregates
or a compact projection when a full-record clone would stall readers.
Never JSON-encode under the observer lock or deep-copy all 400000 complete
records to serve a small log page. Continuous read traffic at a near-full
ring must still allow real DNS requests to complete; formal latency/QPS
targets remain for 5D. Avoid a general metrics framework in this task.

## Settings file and failure transaction

Canonical path: `<top-level-config-base>/webinfo/audit_settings.json` with
JSON `{ "capacity": N }`. Select the source path before JSON parsing: an
existing canonical file wins even if malformed; only when it is absent, read
`state/audit_settings.json`, then root `audit_settings.json` as Go's migration
helper does (`coremain/state_files.go`). File parsing is intentionally
separate from strict POST parsing. A JSON object may contain extra fields;
missing or null `capacity` reads as zero, and saved negative/above-400000
values clamp to 0/400000. Missing file or malformed JSON/field type uses
default 100000 with a diagnostic; a malformed canonical file never falls
back to a valid legacy file. Retain a valid legacy value and migrate it to
canonical only after a complete safe write.
Never remove a legacy source before that write succeeds. File/path failures
must be diagnosable, and no live `/cus/mosdns` path is used by task fixtures.

For capacity POST: require a JSON object containing only `capacity` as an
integer in 0..400000. Missing, null, floating-point, string, extra-field and
out-of-range bodies return 400. Serialize capacity updates,
prepare the candidate JSON, write a temporary file in the canonical directory
on the blocking pool, then replace the target. Only after successful replace
does the observer publish the new capacity and clear its ring. Inject faults
at temp write and final replace for tests. A failure retains old canonical
bytes, old runtime capacity and old audit records and cleans the temporary
file. This reuses the 5C `domain_set` safe-publication principle, but keep
audit settings ownership separate from domain rule-file ownership. Successful
process restart reads the committed capacity; no fsync/power-loss guarantee is
claimed.

The current Go direct API clamps out-of-range values and may report success
after a failed settings write. The proposed Rust API instead returns 400 for
out-of-range values and 5xx for persistence failure. These and the strict
body contract are explicit safety deviations proposed for final user approval.
Saved zero remains legal, even
though the Vue form intentionally accepts only 1..400000.

## HTTP projections

Use the existing route/method/error dispatch conventions in `api.rs`. Freeze
only the six scoped v1 responses from `coremain/api_audit.go` at the handler boundary:
JSON status/capacity, plain-text success for start/stop/clear/capacity POST,
correct method errors, and no false 200 from absent routes. Keep unsupported
native API endpoints absent or explicitly unsupported.

For v2, define a small serializer from internal `AuditRecord` to the fields
needed by the DNS card. The internal record has `SystemTime`, `SocketAddr`,
qname with DNS trailing dot, numeric qtype and `Duration`; the projection
must produce Go/Vue-shaped `query_time` in RFC3339Nano, port-free `client_ip`
by host/port separation without unmapping IPv4-mapped IPv6, `query_name`
with one trailing dot removed except root `.` remains `.`, mnemonic
`query_type` with unknown type as an empty string, and numeric `duration_ms`.
Pin these public representations in tests before implementing the handler.
Do not emit invented `trace_id`/answer/response/effective-tag fields.

Each v2 endpoint response uses one internally consistent retained-state
snapshot; separate requests can observe different generations.
`/api/v2/audit/stats` is derived from current retained records, so clear,
resize and eviction alter it; stop does not remove existing data. Implement
`/stats/windows` from the same retained view with the current Vue keys:
`generated_at` and each window's `key`, `label`, `window_seconds`,
`request_count`, `average_duration_ms`, `complete`, `coverage_start`. Format
  generated/coverage timestamps as RFC3339, distinct from query_time. Omit
  `coverage_start` when the ring has no retained record (`omitempty`).
`coverage_start` is the oldest retained record's admission wall time;
`complete` means that timestamp is no later than the window cutoff and does
not imply uninterrupted capture. Inject a narrow test clock governing both
admission wall time and window `now`; real DNS requests before, exactly on,
and after a cutoff must prove complete/incomplete and an eviction-driven
coverage change. Use
`coremain/api_audit_v2.go` only to characterize the named windows and JSON
shape. No Go one-second stale-stat throttle is required.

`/api/v2/audit/logs` returns newest first, default page 1/limit 50,
pagination metadata, and an empty page when beyond the end. Malformed or
nonpositive page/limit uses the Go default. Direct positive limit up to 500
is accepted; above 500 returns 400 rather than silent truncation. This is a
proposed bounded-API safety deviation from Go direct-request parity. The
existing Vue limit 160 must work. Reject all unimplemented filter/search
parameters with a clear 400. This is a bounded Dashboard projection; do not
route QueryManager's filtered queries to it and pretend they worked.

## Vue integration and failure visibility

The maintained `/` frontend already has `services/dashboard.ts`,
`useRealtimeMetrics.ts`, `dashboard/DnsOverviewCard.vue`, and
`SystemControlManager.vue`. Prefer keeping their request formats unchanged.
The DNS card's main polling uses stats/status/capacity/recent logs and its
popover calls `/stats/windows`. `OverviewManager.reloadOverview()` also awaits
rank endpoints not in this task and can display `加载概览失败` while the DNS card
itself works. The card must have no audit API warning; capture the page rank
failure separately as an explicit deferred limitation. Do not claim the full
OverviewManager works.
If the System page's unrelated reload calls prevent its audit panel from being
used, isolate only the audit section's load/error state and show unsupported
other sections honestly. Do not add placeholder API responses for ranks,
system settings, upstream or Go process metrics.

Browser proof runs the reviewed native binary and a disposable exact-source
Vite/WebUI copy on `mosdns-rust` VM loopback. VM Vite proxies to VM native API;
the local browser reaches Vite only through a task-owned SSH port-forward.
Record VM source revision, ports and tunnel PID, and close all processes.
The Vite build and dev
server can stamp `coremain/www` files; assert main-worktree assets stay
byte-identical and do not stage generated output. A failed API action leaves
visible error and no optimistic control-state change. Confirm each successful
action by re-reading the native status/capacity/stats/logs endpoints.

## Rollback and remaining gates

New audit routes are opt-in through the existing native API listener. On any
startup/bind/run failure, reuse the shared supervisor cancellation and wait
for DNS and HTTP cleanup before returning. If this slice fails validation,
restore only this task's changes; keep the archived domain-set closure and
unrelated dirty paths. Record bounded C08/C11 subitems only after proof; this
task adds no C10 evidence.
Full audit search/rank/capture, Prometheus, complete Vue/System pages, static
serving, 5D and Phase 6 remain separately gated.
