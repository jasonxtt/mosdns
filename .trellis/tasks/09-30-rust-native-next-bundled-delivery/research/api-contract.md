# Frozen audit API contract for this task

This file is part of the reviewed planning range. It records the wire shape
that implementation and tests must use; it is not a claim that the native host
already serves these routes. Source characterization is from the `rust` branch
at the planning base, primarily `coremain/api_audit.go:15-90`,
`coremain/api_audit_v2.go:17-205`, `coremain/audit.go:129-166,937-1016,1120-1318`,
`webui-log/src/services/dashboard.ts:25-88`, and
`webui-log/src/components/SystemControlManager.vue:455-521`.

## Header and error rules

- Go JSON handlers use `encoding/json.Encoder`, so successful v1 JSON bodies
  are UTF-8 JSON followed by `\n`; their content type is exactly
  `application/json`.
- Go mutation handlers call `WriteHeader(200)` before `fmt.Fprint`, so they do
  not set an explicit content type. The native implementation preserves the
  observed plain-text body/status behavior and tests the header as absent for
  successful mutation responses. Vue only depends on `response.ok` and does
  not parse these bodies.
- Native error responses use `text/plain; charset=utf-8` and one trailing
  newline, matching the existing `Response::error` convention.
- A known path with an unsupported method returns `405` and body
  `method not allowed\n`. An unknown path returns `404` and body
  `404 page not found\n`. No unsupported route returns synthetic `200`.

## v1 control routes

| Method/path | Success | Success body | Success content type |
| --- | ---: | --- | --- |
| `GET /api/v1/audit/status` | 200 | `{"capturing":true}\n` (boolean varies) | `application/json` |
| `POST /api/v1/audit/start` | 200 | `Audit log collection started.` | absent |
| `POST /api/v1/audit/stop` | 200 | `Audit log collection stopped.` | absent |
| `POST /api/v1/audit/clear` | 200 | `In-memory audit logs cleared.` | absent |
| `GET /api/v1/audit/capacity` | 200 | `{"capacity":100000}\n` (integer varies) | `application/json` |
| `POST /api/v1/audit/capacity` | 200 | `Audit log capacity set to N. Existing logs have been cleared.` | absent |

The Rust capacity POST intentionally differs from the permissive Go decoder:
the body must be exactly one JSON object field, `capacity`, whose value is an
integer in `0..=400000`. Missing, null, floating-point, string, extra-field and
out-of-range bodies return `400`, content type `text/plain; charset=utf-8`,
body `invalid audit capacity request\n`. A host without an explicit state root
returns `500` with body `audit settings state root is unavailable\n`. A temp
write or final replacement failure returns `500` with body
`audit settings persistence failed\n`. These failures leave the old file
bytes, in-memory capacity and retained records unchanged.

## v2 read routes

All three successful responses use `200`, content type `application/json`, and
one internally coherent retained-ring snapshot. The Rust body is compact JSON
without relying on a trailing newline.

`GET /api/v2/audit/stats`:

```json
{"total_queries":2,"average_duration_ms":1.5}
```

`total_queries` counts retained records only. `average_duration_ms` is zero
when there is no retained record. It is not derived from lifetime host
metrics.

`GET /api/v2/audit/stats/windows`:

```json
{"generated_at":"2026-09-30T12:00:00Z","items":[
  {"key":"1h","label":"1小时内","window_seconds":3600,"request_count":2,"average_duration_ms":1.5,"complete":true,"coverage_start":"2026-09-29T00:00:00Z"},
  {"key":"6h","label":"最近6小时","window_seconds":21600,"request_count":2,"average_duration_ms":1.5,"complete":true,"coverage_start":"2026-09-29T00:00:00Z"},
  {"key":"24h","label":"24小时内","window_seconds":86400,"request_count":2,"average_duration_ms":1.5,"complete":true,"coverage_start":"2026-09-29T00:00:00Z"},
  {"key":"3d","label":"最近3天","window_seconds":259200,"request_count":2,"average_duration_ms":1.5,"complete":true,"coverage_start":"2026-09-29T00:00:00Z"},
  {"key":"7d","label":"最近7天","window_seconds":604800,"request_count":2,"average_duration_ms":1.5,"complete":true,"coverage_start":"2026-09-29T00:00:00Z"}
]}
```

The five items and labels are fixed by `defaultAuditStatWindows`. Timestamps
in `generated_at` and `coverage_start` use RFC3339 seconds formatting. An empty
ring omits `coverage_start` from every item. `coverage_start` is the oldest
retained record's admission wall time; `complete` is true exactly when that
time is no later than the window cutoff. It does not claim continuous capture.

`GET /api/v2/audit/logs?page=&limit=`:

```json
{"pagination":{"total_items":2,"total_pages":1,"current_page":1,"items_per_page":50},"logs":[
  {"query_time":"2026-09-30T12:00:00.123456789Z","query_name":"example.com","query_type":"A","client_ip":"192.0.2.10","duration_ms":1.5}
]}
```

Each log object contains exactly the five fields above. `query_time` uses
RFC3339Nano, `query_name` removes one DNS trailing dot except root `.`,
`query_type` is the mnemonic or an empty string for an unknown numeric type,
`client_ip` has its port removed without IPv4-mapped-IPv6 unmapping, and
`duration_ms` is numeric. The implementation must not fabricate trace IDs,
answers, response flags, effective tags or rule-source fields.

Page and limit default independently to page `1` and limit `50` when malformed,
missing or nonpositive. Positive limits `1..=500` are accepted, including the
Vue card's `160`; `501` and above return `400` with body
`audit log limit must be between 1 and 500\n`. A page beyond the end returns
the same pagination object with an empty `logs` array. The pagination object is
always present, including an empty ring (`total_items=0`, `total_pages=0`).
Only `page` and `limit` are accepted query parameters. Any filter/search key,
including Go's `q`, `exact`, `domain`, `answer_ip`, `cname`, `client_ip`,
`domain_set`, and `effective_tag`, returns `400` with body
`unsupported audit log parameter: <name>\n`.
