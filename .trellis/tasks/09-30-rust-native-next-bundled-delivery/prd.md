# Rust-native audit control, DNS overview card and audit controls

## Goal

Deliver one bounded, runnable 5C workflow after the file-backed `domain_set`
management closure: real UDP/TCP DNS requests feed a Rust-native audit store;
  the host-owned HTTP API controls and reads that store; the maintained Vue `/`
  DNS overview card and System audit controls show and change real behavior.
  The surrounding Overview and System pages remain only partially supported.
Keep the four related slices in one PRD so the next execution agent can finish
one substantial user-visible workflow and submit one complete implementation
review.

## Confirmed context

- The checkout is the dedicated `rust` branch. The previous bounded 5C task is
  archived at `.trellis/tasks/archive/2026-09/09-28-rust-native-domain-set-management/`
  after its supplied exact-range review returned `FINAL: PASS`; it is not full
  5C or production readiness.
- The parent `09-28-rust-next-step-roadmap` remains a planning task. Its
  canary, bounded 5B `fast_mark`/`flow_setter`, and first 5C child are complete;
  the broader feature-coverage rows remain open.
- `rust/native-host/src/observer.rs` already records real lifetime metrics and
  a bounded audit ring, but its audit switch and nonzero capacity are fixed at
  host assembly. The new native `ApiServer` is host-owned and supervised with
  DNS but currently receives only `CompiledConfig`; its only implemented
  routes are the bounded domain-set API and empty real special-groups list.
- The maintained Vue DNS card requests v2 audit stats, recent logs, v1
  status/capacity, and v2 window stats when its popover opens. System audit
  controls call v1 start/stop/clear/capacity. The full OverviewManager and
  SystemControlManager also call many unrelated endpoints that remain absent.
- `docs/rust/feature-coverage.md` keeps C08/C10/C11 as incomplete broad rows.
  The existing dirty `docs/rust/next-stage-plan.md` and parent roadmap contain
  stale first-5C status; this task records the accurate status here and leaves
  those unrelated dirty files to their owner.
- All project builds, Cargo tests, integration tests and E2E verification use
  the `mosdns-rust` SSH alias. The final 5D, Phase 6 and production replacement
  gates remain separate.

## Requirements

### R1. Dynamic audit ownership and control

- Keep listener `enable_audit` as a static eligibility gate. Runtime
  `capturing` starts true as in Go and is independent of lifetime host metrics.
  For an eligible request, sample `capturing` once when the request reaches its
  terminal outcome: a request begun while stopped but completed after start is
  audited; one begun while running but completed after stop is not. Do not
  reproduce Go's asynchronous worker race. An ineligible listener never
  records audit, even after start.
- Runtime start/stop changes only `capturing`. Clear removes all already
  committed retained audit records and audit-derived statistics at one
  linearization point, without changing `capturing`, capacity, or lifetime
  metrics. An in-flight request completing after clear may be recorded anew.
- Capacity is the maximum retained record count, including legal value zero;
  default 100000, allowed direct API range 0..400000. Changing it clears the
  retained ring and audit-derived statistics, without changing capturing or
  lifetime metrics. The maintained Vue form continues to accept 1..400000.

### R2. Host-owned audit settings and v1 HTTP control

- Serve `GET /api/v1/audit/status`, `POST /start`, `POST /stop`, `POST /clear`,
  `GET /capacity`, and `POST /capacity` on the existing host-owned HTTP
  listener. Freeze method, status, JSON or plain-text body, and content type
  against current Go and Vue callers. Unsupported method/path combinations
  fail explicitly; no synthetic success from unrelated API routes.
- Persist capacity to `<config-base>/webinfo/audit_settings.json` and recover
  it on fresh host start. Choose the source path before parsing: an existing
  canonical file wins even if malformed; only when absent, try legacy
  `state/` then root-level settings. If only a legacy file exists, retain its
  value and migrate safely without deleting the source before the canonical
  copy succeeds. The settings-file parser follows Go startup semantics,
  separately from strict POST: a JSON object may contain unknown fields;
  missing or null `capacity` means zero; saved negative values clamp to zero
  and values above 400000 clamp to 400000. Missing file or malformed JSON/
  field type uses the 100000 default with a visible diagnostic; malformed
  canonical does not fall back to a valid legacy file. Do not infer the state
  root from a plugin's included-YAML directory.
- POST capacity requires exactly a JSON object with one `capacity` integer
  field in 0..400000; missing, null, noninteger, extra fields and out-of-range
  values return 400. This strict body and range rule is an intentional safety
  deviation from Go. Persist a complete
  candidate before publishing the new capacity and clearing the ring. File
  write or final replace failure returns 5xx, retains old file bytes and old
  in-memory capacity/ring, and removes the temporary file. This is an
  intentional safety deviation from Go's success response after a failed
  settings write. Claim process-restart retention, not power-loss durability.
  Keep blocking file I/O off the DNS runtime thread. An in-memory host without
  an explicit state root must reject capacity mutation visibly; it must not
  write a settings file into the working directory.

### R3. Bounded v2 read projection

- Serve `GET /api/v2/audit/stats`, `/stats/windows`, and
  `/logs?page=&limit=`. Each endpoint response uses one internally coherent
  audit-store snapshot; separate HTTP requests need not share a generation.
  v2 stats count only retained audit records and their average duration;
  stop, clear,
  capacity change and eviction affect those values. Lifetime host metrics
  continue independently. Windows expose the Go/Vue keys, labels, window
  seconds, request counts, average duration, coverage start and completeness
  derived from retained records. `coverage_start` is the oldest retained
  record's admission timestamp; `complete` means only that this timestamp is
  no later than the window cutoff, not that capture was continuous. With no
  retained record, omit `coverage_start` from JSON (`omitempty`), rather than
  returning an empty string.
- Logs are newest first, default page 1 and limit 50, with malformed or
  nonpositive input falling back to defaults as characterized. Positive limits
  above 500 return 400, an intentional bounded-API deviation from Go; limit
  500 is accepted. Preserve the pagination object (`total_items`,
  `total_pages`, `current_page`, `items_per_page`) and
  return empty logs for an out-of-range page. Support the Vue card's limit 160.
  Project real native data into its needed Go-visible JSON fields:
  `query_time` in RFC3339Nano, `query_name` without a DNS trailing dot except
  root `.` remains `.`, `query_type` mnemonic (unknown type is empty string),
  `client_ip` without port, and numeric `duration_ms`. Do not fabricate a
  trace ID, answer data, effective tag, response flags or rule source.
- This endpoint is a Dashboard-compatible **subset** of v2 logs. Explicitly
  reject unsupported filter/search parameters rather than ignoring them and
  returning apparently complete results. Do not claim full QueryManager or
  complete audit v2 compatibility.

### R4. Maintained Vue and real-process closure

- Through a disposable Vite source copy targeting an isolated native host,
  drive real UDP and TCP queries and show their retained count, latency and
  recent-log changes on the existing DNS overview card, including its time
  window popover. Drive the existing System audit controls: stop still lets
  DNS answer but prevents new audit records; start resumes; clear resets the
  card; capacity change shows the reread value and clears retained data;
  fresh-host restart retains capacity. Preserve failed-action errors. The DNS
  card itself must show no audit API warning. Known unrelated Overview rank
  404s may still make the containing page report load failure; record that
  separately and do not call the whole page complete.
- The broader OverviewManager ranks/upstream data and other System settings
  remain visibly unsupported. A narrowly scoped Vue change may isolate audit
  panel failures from unrelated 404s when necessary for this workflow; it may
  not synthesize other API results or imply that entire pages work.
- Keep DNS+HTTP bound to the existing single supervisor. At a full or near-full
  400000-record ring, repeated stats/windows/logs reads must not starve real
  DNS progress. No JSON encoding under the observer lock and no full-ring
  deep copy to return a small log page. Formal latency/QPS targets remain 5D.
  Normal close and failure paths release both listeners and allow rebind.

## Observable behavior slices

| Slice | Public result | Verification boundary |
| --- | --- | --- |
| 0: audit runtime | UDP/TCP requests under controlled start/stop and static listener eligibility produce correct retained records and independent lifetime metrics | Real host/listeners and a deterministic in-flight barrier; no mocked observer |
| 1: v1 control | Real HTTP changes capture/clear/capacity; settings persist and restart; injected write/final-replace failures preserve the old file and runtime state | Isolated config root and loopback HTTP/DNS; narrow persistence fault seam only |
| 2: v2 reads | Stats/windows/logs reflect retained ring, eviction, clear and pagination with truthful JSON projection | Real DNS requests and HTTP responses; injectable test clock proves complete/incomplete, exact cutoff and eviction coverage |
| 3: Vue closure | DNS card and System audit panel operate over actual native API and DNS, including refresh/restart and visible failure | Native binary and disposable Vite copy on `mosdns-rust` loopback; local browser over SSH tunnel; no mocked API or DNS answers |

## Acceptance criteria

- [ ] R1 is proven by real UDP/TCP and in-flight boundary tests; audit-derived
  stats and lifetime metrics stay independent through start/stop/clear/eviction.
- [ ] R2 v1 methods, status/body/type, canonical/legacy settings precedence,
  capacity zero, restart, temp-write/final-replace failure and cleanup pass
  through real HTTP/DNS tests; DNS/HTTP shutdown and rebind still pass.
- [ ] R3 exact retained-ring statistics, windows (including a cutoff-equal
  record, complete/incomplete and eviction changes) and newest-first paginated
  logs match the bounded API contract; unsupported filters fail visibly.
- [ ] R4 browser proof uses maintained Vue, real native HTTP/DNS and isolated
  files, showing the DNS card and audit controls without implying other
  Overview/System/Query features work.
- [ ] Focused and full Rust tests, fmt, clippy, disposable Vue build, and a
  bounded `mosdns-rust` Linux real-process E2E run are recorded with candidate
  revision, failures/corrections, SSH tunnel topology and resource cleanup.
  Native binary and disposable Vite copy run on VM loopback; a local browser
  may reach Vite only through a task-owned SSH tunnel. Generated assets and
  unrelated dirty paths are excluded from the exact task range.
- [ ] Only proven C08/C11 coverage subitems are updated; C10 receives no
  evidence from this task. The exact
  committed implementation range receives explicit final review PASS before
  task completion; no deployment or production/default replacement occurs.

## Out of scope

Full OverviewManager rankings and upstream data; v1 `GET /api/v1/audit/logs`;
QueryManager search/filter, answer-IP/CNAME/effective-tag fields, trace IDs
and complete Go AuditLog;
`/metrics`/Prometheus and Go runtime metrics; audit capture files; other
System settings; cache flush/dump; `special_groups` mutation; upstream/config
generation; Rust static UI serving and `/log`; all remaining 5B query plugins,
5D capacity/soak/performance acceptance, Phase 6 hybrid retirement and
production cutover. Do not touch live `/cus/mosdns`.

## Proposed decisions for final planning approval

This is the recommended package while the user's direction choice is pending.
It proposes direct API capacity 0, strict JSON body, explicit 400 on
out-of-range values and log limit above 500, persist-before-publish failure
atomicity, terminal-time capture, and the `stats/windows` endpoint. Each is
written above as the execution contract; none authorizes implementation until
the user approves the final planning
summary after C2C planning review.
