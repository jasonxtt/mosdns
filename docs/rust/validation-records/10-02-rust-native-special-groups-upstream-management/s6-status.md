# Slice 6 — maintained Vue workflow

**Status:** S6 is accepted after the dedicated exact-source remediation review
returned `FINAL: PASS` for `P1-1`. The initial FAIL, finding, and parser
restatement remain preserved in
[`s6-review-round-0-result.md`](s6-review-round-0-result.md); the passing
re-review is recorded in
[`s6-review-remediation-round-1-result.md`](s6-review-remediation-round-1-result.md).
S7 and the cumulative review from the original branch baseline remain open.

## Source and authority

The S6 candidate is based on accepted S5 audit object
`63b836b7ba0d0046cbb28763950f206b320cf201`. The real `rust` branch remained at
`79d93ae1b3b3253a2d09563444b251aad18eb5df`, and its index remained unchanged.
Formal task context `codex_01a0fc3e-79ef-7e23-9943-34062de8ded0` records Slice 6
as the active unit under the original frozen authorization at
`2026-10-02T16:11:43Z`; no task or authorization state was reconstructed.

The exact tested-source manifest covers **526 inputs**: the accepted 140-file
Rust/config closure, all tracked Go sources, all tracked WebUI files, embedded
entry/assets, build manifests/scripts, and the new UI and Go tests. Local and
isolated-host SHA-256 values match for all 526 paths. The 140 S5 inputs also
still match the accepted S5 manifest. See
[`s6-source-manifest.json`](evidence/s6-source-manifest.json), SHA-256
`a37b9f2f530e89db3527755efb7b47dcf11a6a1451949a522e48161ac2dbeb06`.

## Implemented behavior

- The maintained Vue workflow discovers native capabilities per page session.
  Only capability-route HTTP 404 selects the existing Go workflow; network,
  server, and malformed-response failures remain visible errors.
- Native upstream controls are limited to runtime-advertised protocols.
  Unsupported stored upstreams remain visible and read-only. Native diversion
  sources accept local `.txt` files, preserve disabled unsupported records, and
  reject unsupported enabled records before mutation. Rename uses one catalog
  transaction; it does not delete and recreate the source.
- The existing Go host's invalid-route helper previously returned HTTP 200 for
  unknown paths, preventing the frozen 404-only discovery rule from identifying
  the legacy runtime, cache-inventory fallback, and absent optional plugins. Go
  now keeps its route-help body and returns 404 for unmatched routes. The
  method-mismatch path retains its previous status and route-help response.
  Regression coverage includes capability discovery, cache inventory, an
  optional plugin route, an arbitrary missing route, and a method mismatch.
- Maintained and compatibility Vue bundles were built through the repository
  Vite configs on the isolated host. The six assets and their two generated
  embedded HTML entry pages were copied back and hash-checked against that
  build.

## Validation

All builds, tests, controlled HTTP/DNS work, and source comparisons ran through
SSH alias `mosdns-rust` in
`/root/mosdns-rust-special-groups-20261003/src`, with Rust output under
`target-candidate` and evidence under the task-owned `evidence` directory.
Nothing used public DNS or port 53.

- Maintained Vue production build: Vite transformed 619 modules and passed;
  compatibility UI build: 612 modules and passed. Both logs show Vite's
  advisory that the main minified chunk exceeds 500 kB.
- `node --test tests/*.test.mjs`: **12 passed, 0 failed**. Coverage includes
  native capability validation, 404-only legacy fallback, surfaced network/
  server errors, local-source payload/ownership rules, and unsupported upstream
  visibility. See the [final test log](evidence/s6-ui-tests-final.log).
- `cargo build -p mosdns-native-host`: passed. Rust source inputs are unchanged
  from S5; the accepted S5 serial suite remains **376 passed, 0 failed,
  3 ignored** (the ignored process probes are called by their parent tests).
  S6 did not rerun that full Rust suite; its S5 result is linked from
  [S5 status](s5-status.md).
- Remote `go test ./coremain -count=1`: passed, including 404 responses for
  missing capability/cache/requery routes and preservation of the prior
  method-mismatch response. The targeted test was also run independently. The
  final Go binary was built with `scripts/build-local.sh`
  after both Vue bundle builds; its command skipped only a redundant second UI
  build.
- Native browser workflow through the isolated Rust host exercised group
  create/edit/delete, upstream save/enable/disable and hide/show-disabled,
  manual-rule save, local diversion-source create/rename/toggle/delete, and an
  invalid `.srs` save. The invalid source produced the visible local `.txt`
  validation error and sent no mutation request. The HTTP trace, controlled DNS
  peer, audit record, cache detail, disk files, and screenshots are retained as
  `s6-browser-*` evidence.
- A controlled query returned `198.51.100.42` from loopback peer
  `127.0.0.1:25455` through special group 50 and reported its sequence, source
  upstream, peer, UDP transport, and response attempt in native audit details.
  A high-port main-listener query also demonstrated custom-port-only exclusion.
  A query sent directly to the group's custom port bypasses the diversion match;
  it is not claimed as proof of source-rule matching. Group deletion left empty
  group/upstream state and an empty valid cache dump, while the external `.txt`
  source file remained.
- A separate isolated Go browser session confirmed the legacy upstream modal
  remains available with UDP, TCP, DoT, DoH, DoQ, and AliAPI choices; query
  logs, cache management, and `/log` render. The config loader warned that the
  fixture package schema 4 did not match the binary's required schema 3, then
  continued with the fixture configuration. These results apply to this
  isolated test fixture, not a production Go configuration. A controlled query
  through the Go `cache_all` listener reached loopback peer `127.0.0.1:25455`,
  returned `198.51.100.42`, and produced two audit rows. The legacy Vue cache
  page showed 2 queries, 1 hit, 1 item, and opened the `s6-go-cache.example`
  detail. See
  [query proof](evidence/s6-go-cache-query.json),
  [cache detail](evidence/s6-go-cache-detail.log),
  [metrics](evidence/s6-go-cache-metrics.txt),
  [fixture startup log](evidence/s6-go-cache-run.log), and
  [audit](evidence/s6-go-cache-audit.json). The fixture intentionally omitted
  legacy rule-list and requery plugins; those controls surfaced missing-list
  404s and marked requery unavailable rather than parsing a false success.

## Preserved failures and limits

- The initial browser capability probe is recorded in
  [Go RED evidence](evidence/s6-go-capability-red.log): both GET and POST
  unknown requests incorrectly returned 200. The repaired live run records
  capability/cache/requery 404s and the retained method-mismatch response in
  [final Go evidence](evidence/s6-go-unmatched-route-live-final.log). Two
  intermediate approaches are preserved as `s6-go-broad-status-*` and
  `s6-go-capability-only-*`; only `s6-go-unmatched-route-*` binds the final
  source.
- The initial source-file HTTP 400 used a path outside the fixture's actual
  config directory; after moving the test fixture to that directory, the UI
  create returned 201. The trace retains both attempts. The first maintained
  build ran before Vite was installed on the isolated host; `npm ci` then
  installed the lockfile dependencies and the final builds passed. npm reported
  five dependency advisories (1 low, 1 moderate, 3 high); they were not changed
  as part of S6.
- One early UI test assertion expected an English helper message while the
  implementation correctly returned the Chinese message; the assertion was
  corrected and the final 12-test suite passed. Its intermediate failure log
  was overwritten during reruns and is not presented as retained evidence.
- Browser checks also encountered expected Go-only endpoint 404s on the native
  host for optional overview aliases/requery modules. The UI displayed those
  unavailable states; these endpoints are outside the S6 native management
  capability matrix.

## Evidence index

- UI build/test: [maintained build](evidence/s6-maintained-ui-build-final.log),
  [compatibility build](evidence/s6-compatibility-ui-build-final.log),
  [npm install/audit](evidence/s6-ui-npm-ci.log),
  [UI suite](evidence/s6-ui-tests-final.log).
- Go compatibility: [pre-fix RED](evidence/s6-go-capability-red.log),
  [route-status test](evidence/s6-go-unmatched-route-test-final.log),
  [full coremain suite](evidence/s6-go-coremain-suite-404.log),
  [final Go build](evidence/s6-go-build-unmatched-route.log),
  [live endpoint checks](evidence/s6-go-unmatched-route-live-final.log),
  [controlled Go cache query](evidence/s6-go-controlled-query.py).
- Native browser: [HTTP trace](evidence/s6-browser-http-trace.jsonl),
  [DNS peer](evidence/s6-browser-dns-peer.jsonl),
  [final audit](evidence/s6-browser-audit-final.json),
  [query detail](evidence/s6-browser-query-detail.jpg),
  [cache detail](evidence/s6-browser-cache-detail.jpg),
  [source rename](evidence/s6-browser-diversion-rename.jpg),
  [post-delete state](evidence/s6-browser-group-delete.jpg).

The initial S6 validation describes the reviewed candidate and does not cover
the newly identified legacy partial-catalog behavior. That behavior must be
fixed and validated on the isolated host before a new exact-source audit object
is submitted. No production switch, deployment, push, ordinary commit, or
change to the actual branch/index was made.

## P1-1 remediation validation

The exact initial review finding is preserved in
[`s6-review-round-0-result.md`](s6-review-round-0-result.md). Its regression
sets capability discovery to legacy by returning HTTP 404, then requests one
optional diversion catalog that returns 404 and a second catalog with a valid
rule. The retained [RED log](evidence/s6-p1-1-red-ui-test.log) shows the test
failed under `Promise.all` with `HTTP 404 Not Found`.

Legacy catalog loading now uses `Promise.allSettled`, merges fulfilled
catalogs, and reports each failed tag and error through the existing error
notice. Native catalog loading remains fail-fast. The same regression now
passes; the complete isolated UI suite is **13 passed, 0 failed** in the
[GREEN log](evidence/s6-p1-1-green-ui-test.log). This is a unit-level exercise
of the helper used by `RulesManager`; the legacy Rules page was not separately
re-run in a browser for this remediation.

Both prescribed UI builds were rerun remotely after the source change:
maintained UI **619 modules passed**, compatibility UI **612 modules passed**.
Both retain Vite's existing >500 kB chunk advisory. Remote
`go test ./coremain -count=1` and the embedded binary build using
`SKIP_UI_BUILD=1 scripts/build-local.sh` passed after the UI bundles were
built. The six results are retained as `s6-p1-1-*.log` in `evidence/`.
Rust sources and the embedded native host are unchanged from the reviewed S6
candidate; the accepted native-host build and suite remain linked above and
were not rerun for this Go/Vue-only remediation.

The new [remediation source manifest](evidence/s6-p1-1-source-manifest.json)
covers the same 526 tested inputs. Its source-map SHA-256 is
`8d519ce6d9fea8867bbf438fc094edb63664b5239d29544b96b6c0ae0c0ef9cc`; its
manifest-file SHA-256 is
`65b533ebbf03f11b35132793bf451d91c4dd2253a07b8ac1a80e5bf5ae1051b7`. The
isolated host reports all **526/526** local input hashes matching with zero
differences in
[`s6-p1-1-source-hash-check.json`](evidence/s6-p1-1-source-hash-check.json).
The original `s6-source-manifest.json` remains unchanged for the first reviewed
candidate. The accepted remediation audit range is parent
`e5d129feaf0adc1b75c23ddeeb650a94618903d2` to head
`eec6c4447e221ea304893dd8ddaf5b761a35cdb3` (tree
`44a41971f7729270c8d171a9c58612d2734243a7`). Its exact request and formal
submission metadata are retained in
[`s6-review-remediation-round-1-request.md`](s6-review-remediation-round-1-request.md)
and
[`s6-review-remediation-round-1-submission.json`](s6-review-remediation-round-1-submission.json).
