# Source-backed API and diagnostic contracts

Baseline: rust HEAD `9f6dfdb2c718e65d6969c1485c28df695090ba83`.
This document is planning, not evidence of delivered routes. Compatibility
proposals below require explicit final-plan approval before implementation.

## Source anchors and current gaps

| Concern | Current source | Finding |
| --- | --- | --- |
| Go endpoint registration/defaults | coremain/api_audit_v2.go:64-205 | Four key/count ranks, slowest rich logs, log query fields |
| Go full schema/flags/answers | coremain/audit.go:123-156,388-426 | class/trace/response/route fields; Answer only; flags serialize uppercase AA/TC/RA |
| Go labels | coremain/audit.go:451-695 | dedup pipe tags; exact special/direct/proxy/memory priorities; unmatched sentinel |
| Go ranks/slowest | coremain/audit.go:698-730,1077-1117;433-447 | ranks over retained logs; separate top-300 slowest heap persists across ring eviction |
| Go search | coremain/audit.go:847-895,1120-1318 | q OR across fields; family filters AND; repeated client IP OR |
| Native store/projection | rust/native-host/src/observer.rs:290-326; api.rs:718-870,990-1021 | AuditRecord has terminal/cache/attempt facts; public HTTP only five fields, no filtered/rank route |
| Native matcher | rust/native-host/src/config.rs:1101-1137; matchers.rs:13-31 | grouped qname matcher loses provider name/exact rule identity |
| Native routing seam | rust/sequence-core/src/engine.rs:749-796; state.rs:106-128 | matcher mutation precedes later matcher success; not necessarily effective route |
| Native final response seam | rust/native-host/src/execution.rs:143-186, result_from_wire | interrupted path must not publish earlier response |
| Vue query | webui-log/src/components/QueryManager.vue:129-143,612-650,959-1018 | q/exact/list/details; lowercase flags currently differ from Go uppercase |
| Vue ranks | webui-log/src/components/OverviewManager.vue:682-746,962-1000 | effective fallback, q/client_ip/domain_set/effective_tag drill-down |
| Existing contracts | .trellis/spec/backend/native-audit-control.md | terminal capture, retained stats/windows, v1 settings and limit=500 preserved |

## HTTP matrix

All paths are under `/api/v2/audit`. GET success is 200 JSON,
Content-Type `application/json`, arrays are `[]`, never null. Known route wrong
method returns existing 405 `method not allowed\n`; unknown path 404
`404 page not found\n`, errors `text/plain; charset=utf-8`.
Existing native stats/windows and v1 mutation bodies stay unchanged.

| GET suffix | Parameters | Response |
| --- | --- | --- |
| logs | page,limit,q,exact,domain,client_ip (repeatable),answer_ip,cname,domain_set,effective_tag | existing pagination object + rich logs |
| logs/domain | domain (required), page, limit | same pagination/logs schema; exact query_name membership |
| rank/domain | limit | key/count array |
| rank/client | limit | key/count array |
| rank/domain_set | limit | key/count array |
| rank/effective | limit | key/count array |
| rank/slowest | limit | array of rich log objects |

Logs page/limit malformed, missing or nonpositive independently use 1/50;
positive log limit >500 gives existing 400. Rank defaults 20, slowest 100,
malformed/nonpositive defaults; proposed maximum requested limit=500 applies
to new ranks too, with 400 above it (new bounded safety contract versus Go).
Slowest returns at most its independently retained top 300 even if requested
limit is 500. Overflow page offsets saturate safely to out-of-range empty page,
not integer panic or wrapped start. Default rank tie order count descending,
then key UTF-8 lexical ascending; slowest descending elapsed, then newer
admission ID. Deterministic ties replace Go's unspecified map/heap tie order.
Unknown query names return 400 `unsupported audit query parameter\n`.
`logs/domain` missing/empty domain returns 400 `exact domain is required\n`.
Repeated scalar uses the first value;
malformed page/limit and exact parsing follow the rules here, not new errors.

## Rich log schema and provenance

Preserve existing query_time RFC3339Nano, query_name no trailing dot except
root '.', known query_type mnemonic/unknown empty string, client_ip without
port and numeric duration_ms. Add query_class known mnemonic/unknown empty,
trace_id (native admission identity), response_code mnemonic or NO_RESPONSE,
response_flags object, answers array, and optional domain_set/effective_tag/
matched_group/final_sequence/final_upstream/upstream_targets/selected_upstream/
matched_rule_source. No fabricated blocked boolean or transport success.
AA/TC/RA uppercase keys follow Go JSON. The existing Vue flag reader must
accept canonical uppercase and retain lowercase fallback for older payloads.
No-response has empty answers and all flags false, matching Go default shape.
Formed response with send failure still has its formed response code/flags;
only native terminal outcome says delivery was unsuccessful.

Answers preserve final-wire Answer-section order and TTL after cache aging.
A/AAAA contain normalized IP string, CNAME/NS/PTR target DNS presentation with
trailing dot, MX exchange only (Go behavior), TXT chunks joined with a space.
Implement these six type families without adding a query plugin. Remaining
RR types require a truthful presentation, not disappearance: propose `TYPE<n>`
and exact raw RDATA hex (`\\# <byte-length> <hex>`) for types outside those
six families. This is a raw diagnostic view, not a portable reconstructed
RR (known types can contain packet-relative compression). Add log-level
`answer_details_status`: `complete` for fully decoded supported families,
`raw_rdata` if any Answer uses raw representation, `decode_error` if audit
projection cannot safely decode. On decode_error expose empty answers and optional `answer_decode_error`
with closed values `invalid_response`, `bad_name`, `truncated_message`,
`invalid_rdata`; map existing DNS errors explicitly (TooShort/TruncatedQuestion/
TruncatedRecord -> truncated_message, NotResponse -> invalid_response, BadName
-> bad_name, known RDATA shape -> invalid_rdata). Retain response code/flags
and native terminal facts; never fabricated partial answers. No-response has status `complete` and empty answers. This differs from Go's generic
RR.String presentation; approve the choice or extend formatting before start.
Do not omit SOA/SRV/HTTPS/SVCB records from answer count while claiming full
answer display. Name compression loops/out-of-bounds and illegal lengths must
not crash, publish partial fabricated records or change DNS execution behavior.
If an audit-only decoder cannot represent a delivered response, preserve
wire transport behavior and explicit diagnostic status, do not turn it into
an artificial DNS failure. The additive status above is frozen as the proposed public contract.

Native ID is allocated once per real admitted query, formatted
`n-<32 lowercase hex process nonce>-<16 lowercase hex u64 counter>` (length
51, counter starts at 1). Use direct native-host `getrandom::fill` at host creation with the same
`getrandom = { version = "0.4.3", default-features = false, features = ["std"] }`
facility already reviewed in upstream-core (no resolver-internal dependency);
fail host creation visibly if randomness is unavailable.
Never reuse packet DNS ID, client port or wall time alone. On counter exhaustion
fail new admission visibly rather than wrap; existing admitted queries still
finish and supervisor shutdown remains intact. Deterministic nonce/counter
injection tests are allowed at the allocation boundary.
No persistence or cross-restart correlation is claimed. It supports q exact
search; capture tracing remains unsupported.

Provider evidence comes from the immutable generation consulted at evaluation.
Named provider tag alone is sufficient domain_set identity; exact rule/source
must be captured at match time where emitted, never re-evaluate against later
rules. The execution seam must not promote a negated hit, an executable-less
rule or a rule whose later matcher failed into an effective routing choice.
Collect necessary candidates separately from final label so preserved raw
match information is not advertised as final route. Inline source is a stable
YAML rule descriptor; provider source includes its configured tag and source
kind. For this batch source precision is provider-generation or inline-YAML rule
identity, not the exact winning literal in an overlapping provider list. Keep
current provider reference order, then current combined inline matcher order;
the first consulted matching group supplies the candidate. Do not rerun or
retie overlapping rules to invent a literal winner. Exact per-line provenance
remains deferred and must not appear in UI as if it were captured.

Effective label is the pure product normalization in computeEffectiveTag with
existing exact-name helpers. Test unmatched_rule, special_n/special_upstream_n,
ban/adblock labels, dedup pipe tags, direct/proxy and memory conversion cases.
These pure display cases do not claim the corresponding missing plugins work.
Configured final_upstream keeps flow_setter precedence; actual numeric endpoint
supplier is selected_upstream. Parent rewrite supersedes child response facts;
cache/local has no selected network supplier. Missing facts remain omitted.

## Filter semantics

q searches query_name, normalized client_ip, trace_id, domain_set,
effective_tag, matched_rule_source, selected_upstream and each answers.data.
Fuzzy is case-insensitive substring. Exact is case-sensitive equality except
IP normalization. `exact` follows Go strconv.ParseBool spellings
1/t/T/TRUE/true/True and 0/f/F/FALSE/false/False; missing or invalid is false.
q match is OR over those fields, then AND with all other supplied families.
client_ip repeated is OR with normalized mapped IPv4/IPv6 and host:port input;
empty repeated value behavior must follow the characterized Go predicate.
domain uses case-sensitive query_name substring; answer_ip exact equality
against A/AAAA data; cname case-sensitive substring against CNAME data.
domain_set/effective_tag use exact full string equality. Empty values disable
single-value filters. Repeated scalar fields use the first value like Go
URL.Values.Get; only client_ip is repeatable OR. Page totals count matches,
not whole ring. Out-of-range page returns empty logs with requested page and
correct filtered totals; empty totals/pages are zero. Cross-page capture can
change membership; no cursor/stable multi-request snapshot is promised.

Domain-rank drill-down must NOT use cross-field q for the native host. Add
GET `/api/v2/audit/logs/domain?domain=<key>&page=1&limit=50` with exact equality
to the projected query_name, sharing the same predicate/key normalization as
rank/domain. Do not change existing domain substring or q semantics. Vue uses
this distinct route and falls back to its existing q exact request ONLY if
that route returns 404 (legacy Go); 400/405/503/network errors remain errors.
This preserves existing Go UI behavior without modifying Go or creating a
general capability framework. Strict rank-membership equality is native scope;
legacy Go fallback retains its documented cross-field limitation. Test the
collision where answers/tags equal another query name, and suffix domains.
Other drill-downs use exact client/domain_set/effective field predicates.

## Slowest retention choice

Preserve the Go product history: top 300 eligible committed terminal records
since last clear/capacity-change, independent of ordinary ring eviction.
Stop freezes both; start resumes; clear/resize clears both; capacity zero
records nothing. This is an intentional distinct view, not retained-ring
ranking. Explain this in the UI/evidence so an evicted log in slowest isn't
misreported as inconsistent rank totals. Use immutable shared records so this
extra 300 doesn't duplicate large answers. No synthetic deletion on normal
ring eviction. If the user instead wants ring-only slowest, approve that
product change explicitly before implementation.

## Approved execution decisions — 2026-09-30

1. New rank limit 500, deterministic ties, two-expensive-job admission with
   visible503 (no queue), and exact logs/domain drill-down
   with 404-only legacy fallback; existing q/domain/logs limits unchanged.
2. Native admission ID and flags case compatibility repair as described.
3. Generic uncommon RR presentation and the exact additive answer_details_status/answer_decode_error schema defined above.
4. Preserve top-300 slowest history rather than silently convert it to ring-only.
5. Strict form-query encoding errors and omission of unavailable provenance
   from categorical ranks (genuine unmatched_rule remains counted).
6. No new hidden byte eviction/truncation; preserve the existing 400000-record
   query-diagnostics cap and complete answer payloads. The user explicitly
   approved this resource behavior on 2026-09-30: if retaining a detailed
   projection reaches a recoverable fallible allocation boundary, keep
   terminalization/lifetime metrics and omit only the detailed retained
   record; if an expensive diagnostic read reaches a recoverable allocation or
   encoding failure, return HTTP 500. This does not claim that a hard
   process-level allocator OOM is catchable. These failure paths must not
   affect DNS service behavior or statistics.

Items 1–5 were approved by the execution prompt. Item 6 was explicitly
confirmed by the user on 2026-09-30 after the measured resource screen. The
18.00 GiB large-answer projection estimate is retained as a diagnostic
worst-case observation, not a reason to silently lower capacity or truncate
answers.

## Expensive read admission and resource screen

Expensive filtered-log and rank reads share a host-owned limit of two running
jobs, no unbounded queue. If both slots are occupied return 503
`audit read capacity exhausted\n` with existing text error Content-Type.
The blocking job owns its permit through completion, including after HTTP
disconnect; a canceled request cannot release capacity while its worker still
runs. Check a cancellation flag at bounded scan intervals and before encoding.
Snapshot immutable Arc-like record handles under the mutex on the worker, then
filter/top-k/encode off-thread and outside the lock. No work item retains raw
packet bytes. Tiny stats/v1 control paths keep their existing direct behavior.
Normal unfiltered log page may retain the existing bounded page operation.
Do not add chunk/version-retry infrastructure speculatively; if snapshot lock
time fails the progress screen, repair with measured evidence and record the
new read-boundary design before proceeding.

Screen three things separately: prove protocol/record-count/job-count bounds;
measure ordinary and deliberately large multi-answer projection bytes/record
and project 400000 records plus two read-handle views/top300; run near-full
400000 rich-record DNS/HTTP progress with a VM-sized fixture. Do not attempt
a 400000 maximal-wire-response allocation or call tiny-record success a worst-
case memory proof. Resource estimates/observations are diagnostics. The
approved policy is to keep the 400000 cap and complete answers, avoid hidden
byte eviction/truncation, return diagnostic-read HTTP 500 on recoverable
allocation or encoding failure, and keep DNS behavior/statistics running if a
recoverable detailed-retention allocation boundary fails. Hard process-level
allocator OOM is outside the catchable failure contract.

## URL query decoding and missing-category contract

Query parsing is bounded by the existing HTTP request/header byte limit. Use
form-query decoding (`+` -> space, percent octets -> UTF-8 text); invalid
percent escape/invalid UTF-8/raw semicolon returns 400
`invalid audit query encoding\n`. This strict failure is a proposed safety
deviation from Go net/url's partial-query behavior; do not silently discard
a malformed filter. Unicode rule labels round-trip. Scalar duplicate keys
use first value even if empty. Repeated client_ip values including empty
values remain candidates: empty cannot match a real nonempty address, while
other valid repeated candidates can match. Integer page/limit parse follows
positive signed 64-bit Go/Linux input, missing/bad/nonpositive/out-of-range
integer uses defaults; checked valid page offsets saturate to empty page on
arithmetic overflow. Malformed exact is false as documented.

Domain/client ranks count all retained real query/client keys. domain_set and
effective ranks omit unknown/unavailable provenance instead of creating an
instrumentation-error bucket. Genuine default/unmatched routing establishes
`unmatched_rule` and is counted. Log rows with unavailable provenance remain
visible with optional route/source fields absent; total_queries still counts
those records. Thus categorical-rank sums need not equal total_queries. This
is an explicit accuracy deviation from Go's unconditional empty-label sentinel.

Response flags come from safe final DNS header observation. If complete
response metadata validates, use its extended RCODE including OPT high bits;
unknown mnemonic is empty as in Go. If safe header exists but full walk fails,
retain only safe flags/base RCODE and set answer_details_status=decode_error
and the closed error reason. Do not claim the fallback is a fully decoded
response. No-response uses NO_RESPONSE and zero flags; projection failure never
changes wire execution/send outcome. Add tests for extended RCODE with OPT,
missing/invalid header and malformed later RR all-or-none behavior.

Source format in this batch is `domain_set:<configured-tag>` for named
provider generation or `inline:<configured-sequence>#<zero-based-YAML-rule>`
for an inline qname rule. Generation ID is an owned internal identity paired
with evidence, not re-derived from current rules or presented as exact file
line/literal. Unknown metadata is omitted. Exact provider-line output is deferred.

Overview audit rank/slowest requests have per-panel errors so a failed rank
does not erase already-loaded DNS-card or other rank data. Unsupported upstream/
metrics/alias/switch sections must show unavailable/error state in native usage
rather than successful empty data, without introducing a native-only UI mode
or changing successful Go-backed operations. Query details show raw/decode-error
status when present; never present a decode-error empty answer as a DNS NODATA.
