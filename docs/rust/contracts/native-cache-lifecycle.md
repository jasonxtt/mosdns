# Native cache lifecycle and management

## 1. Scope / trigger

Changes to native cache config, sequence publication, background refresh,
persistence, management HTTP or the existing Vue cache page must preserve this
contract. Source: `rust/native-host/src/cache.rs`, `cache_dump.rs`, `execution.rs`,
`api.rs`, `assembly.rs`; approved task `10-01-rust-native-cache-lifecycle-management`.
ECS key isolation and whole-runtime cutover remain outside this delivered scope.

## 2. Signatures

`CompiledConfig.caches` indexes named and private quick stores by `CacheId`.
`PendingStore` captures owner generation; tokenized `WatchToken` publication
belongs to its own successor's natural completion. `SuccessorRecipe` owns a
state/scope copy and binds an independent `ExecutionControl`.

`NativeCacheAdapter::{dump,import_dump,save,flush}` use the one native core store.
`NativeCache::{snapshot_bounded,merge_prepared}` avoid a shadow cache or ABI change.
GET `/api/v1/cache/inventory` returns `{schema_version:1,caches:[{tag}]}` in named
config order. GET `/metrics` exposes only the four `mosdns_cache_*` cache families.
GET `/plugins/{tag}/{show,dump,save,flush}`; POST `/plugins/{tag}/load_dump`.
Existing domain_set show/save/post stays mounted independently.

## 3. Contracts

Key: AD/CD/DO flags byte, BE QTYPE, one-byte case-preserved escaped textual qname
length and trailing-dot name; only IN, basic EDNS0/DO, no ECS suffix. Invalid or
ECS queries bypass without changing forwarding. Cache response OPT removal must
rebuild compression offsets. Store original Unix wall times and use monotonic
runtime expiration. Nonempty hit domain_set restores successor metadata.

Detached Lazy refresh: owner singleflight key, 256 nonblocking slots/no queue,
5 seconds, new shared root fuel64. Capture before stale response assignment.
Nested background Lazy is inline miss under that same root. No client audit,
admission or foreground cache counts; real upstream attempts still count.

Management is owner-owned and serialized, not cancelled with the HTTP caller.
Import fully stages/validates before one generation-changing merge. Dirty revision
is separate from generation; save cleans only its captured revision. Durable-first
flush commits an **empty** dump first, then no-await/no-recoverable-error memory
clear and generation transition. Precommit failure leaves old file/memory/version;
postcommit panic stops host and must never save old memory over the empty file.

V2 gzip Name=mosdns_cache_v2, existing dump.proto fields; block8-byte BE/1MiB,
compressed16MiB, cumulative decoded+owned key/wire/domain64MiB, entries100000.
Check full footer/CRC/trailing bytes, all keys/wire/times/UTF8 before merge,
including expired entries. Negative/inverted/future-stored/overflow rejects entire
dump. Expired skip, Lazy restore only if owner lazy>0. Export has the same limits.

Relative file paths use declaration/include base; duplicate normalized targets
reject, no mkdir/fallback path. SIGTERM/SIGINT cancel native supervisor, drain all
listeners/owner work, then save every owner. Any final save failure returns exit2.

## 4. Validation / error matrix

| Trigger | Result |
| --- | --- |
| Unknown tag, private quick, unmounted/cache-only action on noncache | 404 before method |
| Known mounted action wrong method | 405 |
| save without target / invalid manual import | 400, old state retained |
| write/rename failure | 500, precommit file/memory unchanged |
| Bad/missing startup dump | diagnostic, start empty |
| Periodic write failure | keep dirty, retry |
| Postcommit invariant failure | fatal cancellation, preserve empty disk |
| Final saves fail | finish other owners, aggregate errors, process exit2 |
| inventory404 or supported:false | existing Go UI fallback |
| inventory500/timeout/unknown schema | local cache-panel error, no fabricated rows |

load_dump alone permits16MiB request body; other routes retain1MiB. URI encoded
tag bytes must reach the same named owner. Prometheus labels escape slash/quote/
newline appropriately. Live gauge/show excludes fully expired entries. Show is
sorted by key, q matches key/answer case-insensitively, offset<0→0, limit<=0→100.
Keep Go DNS section markers and metadata lines for the existing Vue parser.

## 5. Good / base / bad cases

Good: a gated upstream refresh completes after clients leave; one follower task;
source state stays independent; restart preserves domain_set and original age.
Base: named stores isolate, quick callsites remain private, nested publication
returns to the enclosing successor and direct fallback exit never publishes.
Bad: old-generation response after flush/import; corrupted last entry/CRC partially
merges; management disconnect cancels rename; successful flush resurrects on restart.

## 6. Required behavior tests

`cache_catalog` covers real nested DNS, direct fallback exit, owner drain and
refresh fuel. `cache_lifecycle` covers escaped keys/DO/OPT, wall rollback, full
import/transaction races and real Go v2 fixture. `cache_http` drives actual DNS,
HTTP routing/metrics/body cap/restart, encoded labels, and SIGTERM/exit2.
`cache.rs` persistence tests cover periodic/final/error aggregation and postcommit
fatal preservation; `cache_dump.rs` tests cover budget/framing/footer limits.
Existing `slice6_management_http`/`slice7_management_publication` must still pass.
Vue `tests/cacheInventory.test.mjs` covers fallback/schema/missing metric/partial
failure labels; actual browser proof must use live owners and actual DNS text.

## 7. Wrong vs correct

Wrong: "durable-first flush saves the old snapshot, then clears memory."
Correct: atomic replacement with the **empty** snapshot is the external commit;
old-snapshot persistence would restore cleared entries after restart.

Wrong: inventory errors silently select hardcoded caches with zero metrics.
Correct: fallback only when explicitly unsupported; show the cache-panel error;
missing metrics are unavailable, real zeros remain zeros. Batch flush reports
successful count and each failed tag without claiming a global atomic operation.
