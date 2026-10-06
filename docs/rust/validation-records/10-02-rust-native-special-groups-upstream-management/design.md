# Design: native special groups and upstream management

Baseline79d93ae1; planning only. Scope selected by user; final approval pending.

## 1. Integration and compiled representation

Native-only opt-in top-level YAML `native_management: {special_groups: true}`.
State root is the declaring main config directory, independent of process CWD.
Without opt-in preserve current strict unmanaged config and HTTP refusal behavior.
With opt-in, load managed state and generate `sub_config/special_groups.yaml`,
merge that document exactly once before resolving names. Explicit inclusion of
that same generated file is rejected, not silently deduplicated. Reserved generated
tags must not collide with user tags. Supply an executable sample with empty state.

The base sequence explicitly calls `$special_upstream_matcher` where special
routing belongs. Managed opt-in requires that hook to be reachable from the primary
entry; missing/unreachable hook is a compilation error, not a silently ineffective
main-routing capability. Do not prepend routing globally or reorder user instructions.
The generated matcher/router performs ordered qname matching and direct named
child calls using existing sequence-core program IDs; no generic External payload
extension. One focused native router executable descriptor owns ordered groups,
provider IDs and child SequenceIds. Generated group definitions use native
`domain_set`, `forward`, `cache`, `flow_setter`, sequence and UDP/TCP listener types.
No Go plugin instantiation, bridge, switch selector or aliapi signed fallback.
Generated routing source is a compilation input, not a cosmetic export of some
other hidden graph. Compile/hash the exact staged artifacts before publication.

The router has ascending-slot priority. For each main-enabled group, diversion
local sources precede its manual provider. First matching source/group wins;
disabled/empty sources do not match. A selected group executes its child then
exits the enclosing request path (including failure/no response), rather than
falling through to another group/default upstream. An unmatched query returns
normally to the caller's existing default path. Custom-port entry goes directly
to its group regardless of qname. No A/AAAA-only routing restriction.

Stable names: special_N, special_upstream_N, special_route_N, special_manual_N,
cache_special_N, sequence_special_N, special_{udp,tcp}_server_N. Keep group view
fields/key/path; name is display metadata, slot is identity. slots>=50, allocate
first unused, trim names, case-insensitive duplicate name409; reserved53 and
invalid ports400, duplicate/occupied listen port409. custom_port_only with port0
normalizes false. Deleting then reusing a slot creates a new internal incarnation
so prior query/publication tokens cannot attach to the replacement.

## 2. Managed upstream and rule contract

Group create retains the existing standard UDP default223.5.5.5 and concurrent2;
controlled tests immediately replace it with loopback fixtures before DNS queries.
Upstream inventory includes real named forward definitions plus generated groups,
never quick-forward callsites. Named forward entries can be managed through the
same override compiler; unknown tag404. Override schema/array order/tag/enabled
are preserved. Require unique nonempty entry tags and at least one enabled entry;
all-disabled/empty400 (explicit native validation difference). Disabled advanced
entries can be retained verbatim without activation; mutation cannot activate an
unsupported protocol/option. Do not drop unknown stored fields during read/save.

Normalize dot→tls and doh→https; scheme mismatch400; HTTPS missing path→/dns-query.
Use existing compiler for numeric dial, bootstrap_version0 dual/4 A-only/6 AAAA-only,
TLS service identity, query timeout (milliseconds;0 existing default5s), reuse,
max three distinct racing entries and typed pre-send busy. Do not invent a new
forward retry/timeout implementation. Unsupported nondefault options400 before
persistence: signed aliapi, quic/H3, socks proxy, socket marks/device/max_conns,
positive upstream idle_timeout or enable_pipeline=true. Harmless UI defaults
(false/0/empty/null) normalize to absent compiler fields. No OS resolver/downgrade.

Existing endpoints and bodies:
- GET /api/v1/special-groups: ordered view array; POST same: group view200;
  DELETE /{slot}:204; missing404, malformed400, conflict409, durable failure500.
- GET /api/v1/upstream/tags: actual tag array; config: override map; runtime/{tag}:
  {tag,override_config,runtime_targets}, based on committed snapshot.
  POST config {plugin_tag,upstreams}:200 {message:"Upstream configuration saved."}.
- GET /plugins/special_route_N/config: source array; PUT /config/{name}: source
  object200/201; DELETE /config/{name}:204. URL is not required in native local
  mode. No downloader/update endpoint success: unsupported400/405.
- manual special_manual_N uses existing domain-set show/save/post HTTP and body
  contract. All writes affecting managed rules use this transaction coordinator,
  never mutate a live matcher/file behind an immutable query snapshot.

Diversion source catalog remains `srs/special_N.json` for path compatibility;
this JSON is a catalog, not permission to read SRS binaries. Supported sources:
nonempty name, matching group type/key, one local UTF-8 text file, enabled;
url empty, auto_update false, enable_regexp false. Use the existing native domain
rule grammar (including supported regexp expressions in manual text), parser and
path resolution; malformed/missing enabled source rejects whole candidate.
Empty text is valid no-match. Resolve local paths against config base; mutation
may reference existing readable files but may not delete arbitrary source files.
Deleting a group removes its owned catalog/manual/override/generated objects;
external referenced files are retained. Group paths are fixed, URL path segments
are validated/decoded once, arbitrary traversal/symlink escape writes refused.
Existing unsupported enabled catalog data rejects managed startup/apply explicitly;
disabled unsupported records remain visible read-only, byte content preserved.
No background file watcher: API edit/apply or restart is the visibility boundary.

Generated per-group cache defaults preserve size20000000, lazy259200000,
dump interval36000, dump path cache/cache_special_N.dump; capacity is a limit,
not eager allocation. Cache execution surrounds the group's upstream and final
response normalization. Implement the generated CNAME-removal product behavior
as a focused native policy: only A/AAAA queries with A/AAAA answers and no DNAME;
remove CNAME and rename retained answer owners to original qname; pure CNAME,
DNAME and other query types stay intact. Preserve RCODE/authority/additional,
DNS wire validity and current supplier/ECS ownership. Cache stores the normalized
successor result; add compression/multi-record safety proofs. No fake local supplier.
No dependency on switch4: native profile group cache is enabled explicitly by its
generated definition; Go switch toggles are visibly unavailable in native UI.

## 3. Runtime snapshots and shared ownership

`RuntimeSnapshot` owns compiled program, immutable rules, listener entry table,
forward invocation/catalog handles, cache catalog handles, policy dependency IDs
and a monotonically checked generation. IDs are snapshot-local; no cross-version
lookup of an old ExecutableId in the new catalog. One host QueryObserver survives
publication, retaining audit capture/capacity/metrics settings.

`ManagedControlPlane` serializes mutations and exposes current committed metadata.
`HostSupervisor` owns DNS/API admission, sockets, request registries, retirement
and final resource close. Listener code MUST stop closing shared forward/cache
catalogs on individual listener exit. One listener failure still cancels host and
joins all work. Do not widen this current-thread LocalSet design to Send/Sync.

At each UDP datagram or complete TCP DNS frame admission, capture one snapshot
and one listener binding/entry atomically. TCP connections do not pin the initial
config across later frames; an in-flight query retains its initial snapshot,
audit origin and deadline. All facts/refresh captures resolve against that snapshot.
Concurrent overlapping generations are bounded: serialized mutation joins its
retired requests/resources before another apply; admission is briefly paused at
commit, not while parsing/compiling/staging files. Existing request deadlines and
refresh5s bound drain; no unbounded retirement queue.

Keep primary listener settings unchanged. Multi-listener compilation validates
unique (protocol,address), entry/type and audits independently; UDP/TCP may share
port. Reuse unchanged sockets, prebind new UDP+TCP before commit, no candidate
accept loop before publication. Changed/removed group port stops frame admission
and closes idle TCP connections; cancel+join owned requests for that port without
closing shared owners. New or reused listener tasks read committed binding table.
Avoid reuse_port tricks. API bind stays stable through applies.

## 4. Transaction and crash recovery

Use one bounded local transaction journal under webinfo with schema_version1,
transaction ID, old/new generation, scoped file paths/digests, staged artifacts,
backups and a durable commit marker. No assumed multi-file atomic rename.
Path validation must reject journal-controlled writes outside approved managed
files. One OS-level state-root writer lock prevents two native processes writing
this state. A startup recovery completes before compile, sockets or management.

Prepare: validate body/profile → derive complete candidate and dependency closure
→ parse files/compile exact staged YAML → assemble new resources without requests
→ prebind new sockets → acquire impacted cache publication/persistence gates
→ stage and fsync artifacts/backups plus prepared journal. File I/O/codec on a
bounded blocking worker; never hold RefCell borrow across await. Shutdown before
commit aborts and restores old state.

Commit: pause new request and management admissions; replace scoped canonical
JSON/YAML/rule files and impacted empty cache dumps, fsync directories; write and
fsync commit marker. All allocation, compilation, binding and fallible candidate
startup must have completed. Then synchronously swap runtime+metadata bindings,
advance invalidated cache generations/revoke publications and release admission.
This final in-memory step contains no await or fallible memory-clear operation.
No successful response before durable marker AND runtime swap. Retire old work
and close exclusively retired owners exactly once; clean journal only after
retirement. Preserve unrelated files/config/dirty workspace changes.

Before marker, ordinary validation/bind/I/O failures restore every changed file
from backups while gates/admission remain blocked;500/409 then resume old generation.
Rollback failure or ambiguous fsync/commit outcome is recovery_required: stop host
admission, cancel/join, retain journal, return503 if possible. Do not claim unchanged
state. After durable marker, never roll back to old runtime: restart recovery rolls
forward the entire new generation. A panic/invariant failure during memory swap
stops host for recovery. HTTP client disconnect does not abort an already committed
mutation. After commit marker shutdown must complete swap/recovery bookkeeping,
then drain; delete/cache cleanup failure is reported, not disguised as rollback.

Recovery: absent marker restores all old scoped files; marker completes all new
files from digest-validated staged artifacts; corrupt/incomplete journal fails
startup, never guesses. Cleanup is idempotent; inject process death after each
write/rename/fsync/marker/swap/cleanup boundary. Startup unsupported config remains
an error, never starts partial group state. Ordinary no-journal external edits are
validated on restart; concurrent external writes detected by precommit digests
cause409. No broad external package managed_files adoption.

## 5. Cache dependency and retirement

Group upstream/rules/enabled/main-routing/custom-only/order changes invalidate the
cache policy closure, including ancestor caches wrapping the managed router and
other named forwards whose overridden policy changed. Do not merely flush
cache_special_N: a cache_all ancestor could otherwise serve stale routing.
Compute conservative compiled reachability through calls/fallback/preference/
goto/try/recursion; unknown dependency invalidates potentially affected owners.
Stable name-only changes reuse wire caches but update display metadata through
current group view, with audit labels resolved from query snapshot. Port-only
changes do not change wire policy, unless custom-only membership changes.
Unrelated provably disjoint owners retain values and lazy tasks; prove it.

Impacted persistence operations and publication are gated through transaction
commit. Candidate caches start empty and impacted canonical dumps are replaced
with valid empty v2 dumps as journaled changes. Old owner generation is revoked
on commit; old front miss and refresh cannot publish; cancel/join old refreshes,
stop old periodic persistence and retire WITHOUT a final old dump overwriting the
committed empty/new dump. Failed precommit restores old dump, releases gates and
retains original memory/generation. API flush/import/save participates in the
same serialization order. Existing durable-first flush, opt-in ECS key/placement,
lazy singleflight256/no queue/fuel64/deadline5s contracts remain intact.
Snapshots with reused caches must preserve stable IDs via explicit remapping or
immutable owner keys, never tag-only accidental numeric ExecutableId collisions.
Deleted slot incarnation/cache is not reimported on slot reuse. No deletion of
unrelated cache files. Shutdown: stop mutations/admission → finish or abort active
transaction by marker → cancel/join requests and refresh → final dumps for current
owners only → close forwards once → all sockets/tasks released.

## 6. Capability/UI and acceptance differences

Add GET /api/v1/capabilities native response with schema_version1, runtime:"rust",
special_groups {enabled,profile:"local_text_dns_v1"}, upstream_protocols,
rule_formats, unsupported_features and existing endpoint support flags. True only
when actual compiled profile enabled. UI caches it per page session; Go404 means
legacy flow; other failures show discovery error, never silently assume Go.
Do not amend old response shapes with mandatory generation fields.

Native RulesManager uses local-file validation, disables downloader/update/advanced
formats, preserves unavailable records visibly; UpstreamManager skips unavailable
/overrides and switch queries, disables unsupported options, and renders only actual
inventory. Keep Go form validation/save flows unchanged. Native rename/move rule
operations use one PUT transaction; do not copy Vue's delete-then-put partial flow.
Changing group ownership across catalogs is refused in native local mode; create
new destination then explicit removal if needed. Explicit truthful loading errors
replace swallowed Promise.allSettled rejections. Active UI/compatibility UI retained.

Intentional differences for final plan approval: native explicit integration/profile,
transactional hot apply instead of scheduled self-restart, reject all-disabled
upstreams, local-file source without mandatory URL, enabled caches without legacy
switch UI, strict unsupported startup/mutation, no partial persisted-success reload
failure. These are not a claim of full Go config/UI compatibility or final cutover.

Acceptance is the PRD plus implement.md matrix. C2C planning READY cannot authorize
implementation or substitute per-slice/final exact-source review PASS.
