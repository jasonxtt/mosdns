# Native audit control and read API

## 1. Scope / Trigger

Use this contract when changing the Rust-native host's audit capture, retained
records, settings file, HTTP audit routes, or the maintained Vue audit controls.
The bounded 5C delivery implements the DNS overview card and audit panel; it
does not complete C08, C11, the full Overview/System pages, or `/metrics`.

## 2. Signatures

- `QueryObserver` owns runtime `capturing`, capacity, retained records, and
  lifetime `MetricsSnapshot` as separate state.
- Host API routes: v1 `GET /api/v1/audit/status`, `POST /start|stop|clear`,
  `GET|POST /capacity`; v2 `GET /api/v2/audit/stats`, `/stats/windows`,
  `/logs?page=&limit=`.
- The file-backed host owns `<config-base>/webinfo/audit_settings.json`.
  Included plugin source directories cannot supply this root.

## 3. Contracts

- Listener `enable_audit` is a static eligibility gate. For eligible requests,
  read runtime `capturing` once at terminalization. An in-flight request can
  therefore be recorded after start or omitted after stop. Lifetime metrics
  update independently. Clear removes retained records and derived audit
  statistics, but leaves capacity, capturing, and lifetime metrics intact.
- Capacity defaults to 100000 and may be 0..400000. Changing it clears the
  retained ring. Persist a complete same-directory candidate before publishing
  the new capacity; a process restart reads the committed value. This claims
  process-restart retention, not power-loss durability.
- Startup settings select canonical before legacy `state/` then root path.
  An existing malformed canonical file does not fall back to legacy. The file
  parser accepts unknown fields, treats missing/null capacity as zero, clamps
  saved out-of-range integers, and uses default 100000 for malformed JSON/type.
  The HTTP POST parser is intentionally stricter.
- v2 stats and windows derive only from the retained ring. Each endpoint
  response is internally coherent; separate requests need not share a
  generation. Window `coverage_start` is the oldest retained admission time,
  omitted for an empty ring. `complete` means only that time is at/before the
  cutoff, not uninterrupted capture. Logs are newest first and expose only
  real `query_time`, `query_name`, `query_type`, `client_ip`, `duration_ms`.
- The detailed ring remains capped at 400000 records and retains complete
  answer payloads; do not introduce byte eviction or silently lower capacity.
  A detailed-retention allocation failure may omit only that record after
  terminalization and lifetime metrics are recorded. Expensive diagnostic
  reads own one of two bounded worker permits through worker exit, including
  after a client disconnect; cancellation returns 499, while recoverable
  snapshot, projection, or encoding allocation failures return 500. Neither
  retention nor read failures may affect DNS service behavior or statistics.

## 4. Validation and error matrix

| Input or state | Required behavior |
| --- | --- |
| `POST /capacity` body exactly `{"capacity":7}` | 200; persist, clear ring, publish 7 |
| Missing/null/fraction/string/extra field or outside 0..400000 | 400; old file, capacity and ring unchanged |
| No host state root | 500; no write to working directory |
| Temp-write or final-replace failure | 500; old file and runtime state intact, temp removed |
| Logs page/limit malformed or nonpositive | Default page 1 / limit 50 independently |
| Logs limit above 500 or unsupported filter | 400, with no silent clamp/filter omission |
| Unknown path / wrong method | 404 / 405 respectively |

The exact response bodies and content types are frozen in the task's
`research/api-contract.md`; update that contract and review a new range before
changing wire behavior.

## 5. Good, base, and bad cases

- Good: a real DNS query commits one eligible terminal record; the v2 card
  shows its retained count, time window, and recent row; System capacity 7
  survives a fresh host process.
- Base: capture off leaves DNS and lifetime metrics active while retained
  audit counts stay unchanged. Empty ring omits `coverage_start`.
- Bad: a failed capacity replace returns 5xx and leaves file, capacity and
  retained records unchanged; an unsupported log filter returns 400.

## 6. Tests required

- Real UDP/TCP in-flight barriers prove static eligibility and terminal-time
  start/stop sampling, plus lifetime-metrics independence.
- Real HTTP/DNS tests prove capacity zero, canonical/legacy precedence,
  strict POST versus permissive file parsing, restart, temp/final-replace
  failures, listener shutdown and rebind.
- Real DNS with an injectable admission/window clock proves cutoff equality,
  complete/incomplete, eviction, empty coverage omission, pagination and
  bounded reads while DNS continues at a near-full ring.
- Browser proof must use a disposable Vite source copy and native binary on
  `mosdns-rust` loopback, with a task-owned SSH tunnel. Click the actual Vue
  clear/capacity confirmations; direct HTTP calls are supplementary evidence.
  Record the exact source revision, process/tunnel identity, observed API/file
  state, restart and cleanup. Generated assets must stay out of the checkout.

## 7. Wrong vs correct

Wrong: map `MetricsSnapshot.completed_total` to v2 `total_queries`, report a
successful capacity POST before the settings file is replaced, or call the
entire Overview page complete because the DNS card works.

Correct: derive v2 data from retained records, publish capacity after file
replacement, and report the DNS card and audit panel as bounded C08/C11
subitems while rank, QueryManager, other System APIs and static serving remain
separate work.
