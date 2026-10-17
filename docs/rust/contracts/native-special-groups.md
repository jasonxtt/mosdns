# Native special groups

## M1-e standard file configuration (2026-10-09)

Implementation is under external review; validation is recorded in
[M1-e evidence](../validation-records/10-09-m1-e/README.md). A file configuration
that includes `sub_config/special_groups.yaml` enables standard group management
without `native_management`. The sections below this M1-e section document the
older managed opt-in, which is frozen until the Phase 6 retirement assessment.

`webinfo/special_upstream_groups.json` is authoritative. It retains the v0.7.3
array fields `slot`, `name`, `listen_port`, `custom_port_only`,
`upstream_sources` and `owned_upstreams`. Source records contain
`kind: group|upstream`, `plugin_tag` and optionally `upstream_tag`. Private entry
input uses `upstreams`; persisted/output entries use `owned_upstreams` and
`use_socks_proxy`. Settings input also accepts `use_proxy`.

Every start, check and candidate compile resolves enabled private entries first,
then enabled source entries from YAML plus typed settings. Whole-group references
supersede individual references to that group; repeated references are deduplicated.
Missing/disabled references produce visible warnings and are excluded. A group
with no effective entries has no diversion rule or dedicated listener. Its
generated placeholder upstream is unreachable through the group dispatcher.
Referenced entries inherit resolved source proxy and bootstrap defaults.

Private `use_proxy=true` requires exactly one configured `socks5_proxy` object.
Zero objects reject with a missing-object reason; multiple objects reject with
an ambiguity reason. No silent direct fallback is allowed. A sole object's empty
address retains the explicit M1-f1 no-proxy state. UDP retains the ordinary
transport rule that ignores SOCKS. References inherit the source's proxy rather
than selecting an object again.

The derived YAML has Go's schema-4 structure and bytes, except that each
`special_upstream_N.upstreams` block carries actual native entries and unique
runtime identities. These identities are not persisted into the group JSON.
Generated tags are reserved; user collisions fail compilation. Active main-flow
groups are ordered by slot and run at the existing matcher/dispatcher hooks;
custom-port-only groups are excluded from that dispatcher. Dedicated UDP/TCP
listeners enter the group's child sequence directly. This preserves the existing
configuration's priority before DDNS.

`GET/POST /api/v1/special-groups` and
`DELETE /api/v1/special-groups/{slot}` expose the v0.7.3 derived view. Slot zero
allocates the first free slot from 50. POST returns 200; DELETE returns 204.
Invalid fields return 400; duplicate name/port or occupied sockets return 409;
missing delete targets return 404. Omitted source/private arrays retain existing
values; explicit empty arrays clear them. `GET /api/v1/upstream/sources` returns
ordinary source groups sorted by plugin tag. Ordinary upstream POST refuses
`special_upstream_N` with 400; private entries belong to the group editor.
Capabilities enable `groups.manage` and `groups.upstreams` in this standard
profile. Effective settings/check reports include group entries and sources.

Writes compile and prebind the whole candidate before committing files. Under
the existing management drain/input fence, derived YAML is durably replaced
first, then authoritative group JSON (or `settings.json` for a source/settings
change) is renamed as the commit point. A failed authority write restores old
derived bytes; failed restoration stops the host with recovery required.
Startup compiles from authority and regenerates YAML after listener binds, before
serving. Check never writes. This repairs a crash between the two renames.
Input digests refuse externally changed inputs before commit.

The reference restarts after saves; native publishes the whole generation hot.
Deleted group owners retire before removing that group's manifest, manual file
and cache dump. Startup removes numeric orphan group artifacts. External source
files are retained. Cache reuse follows the existing dependency closure, so
unrelated cache owners remain; changed source dependencies invalidate affected
group/main-flow caches.
Failure before the authority commit keeps the old authority, derived YAML and
generation (invalidated disk caches may be empty). After publication, a failed
artifact cleanup of a deleted group's files is **not fatal** (5D-2a): the
committed generation is correct without it, so the save succeeds, the host stays
ready, a warning is logged, and the file stays listed as `pending_cleanup`
(paths relative to the configuration root) in `GET /api/v1/system/health`. The
cleanup is a sweep of numeric orphan artifacts (slot >= 50 and not an active
group's); every later group save and every startup runs it again, so a pending
file is removed once the fault is gone, and a file of a slot that a new group
has since taken is never touched. The same applies to startup: an orphan that
cannot be removed is a warning plus a pending entry, not a startup failure.
Only this file removal was relaxed; failure to retire the old generation or to
finish the managed store's retirement remains a committed-cleanup failure that
stops the host. It must not report that an already committed deletion was rolled
back.

The standard path does not read/write Go's `upstream_overrides.json`; old state
that relied on that file to supply missing `owned_upstreams` must be migrated
explicitly. The compatibility claim is for v0.7.3 state with its private entries
present, not that older implicit migration format.

## 1. Scope / trigger

The remaining sections preserve the frozen `native_management` contract;
the M1-e standard profile and proxy rule are described above.

Compiler and native runtime contract for explicit `native_management: {special_groups: true}`.
This document describes the compiled runtime and management HTTP behavior.
S2 exact-source review passed at `a9fe9f0eb5ceac81f7b43407acb2f0e81cdfaedf`
(see the S2 review record). S3 exact-source re-review passed at
`6b50220d1fe137cd2722c44da7c0e095df9095e1`; S4 passed at
`d43da11ababb0104c84b827f96ffd6262e00ac7e`. S5 exact-source review passed at
`63b836b7ba0d0046cbb28763950f206b320cf201`; S6 Vue workflow review passed at
`eec6c4447e221ea304893dd8ddaf5b761a35cdb3`. S7 isolated whole-chain validation
passed, including the workspace test suite, controlled DNS/HTTP/browser proof,
and exact source closure. S7 and the separate cumulative review now both passed;
final tested source is ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc. This task
PASS does not imply complete migration or production release acceptance. See the
[S7 validation record](../validation-records/10-02-rust-native-special-groups-upstream-management/s7-status.md).

## 2. Signatures

`load_and_compile(path: &Path) -> Result<CompiledConfig, ConfigError>` resolves
managed state relative to the canonical main declaring directory.
`ManagedProfile::generate() -> Result<String, ConfigError>` supplies exactly the
YAML parsed by compilation; `generated_sha256` hashes those bytes with SHA-256.
`normalize_groups(&mut [SpecialGroup])` validates identities and sorts by slot.
`forward_entries(&[Value], path)` translates enabled standard DNS entries into
the existing strict native forward compiler. It never performs network I/O.

## 3. Contracts

Groups live in `webinfo/special_upstream_groups.json`; overrides are the plugin
tag to entry-array map in `webinfo/upstream_overrides.json`. Catalogs live in
`srs/special_N.json`, keyed by source name. Disabled unsupported entries remain
in the immutable profile as original JSON. The generated namespace and paths
are defined in the [approved plan](../plans/special-groups-upstream-management.md)
and [frozen design](../validation-records/10-02-rust-native-special-groups-upstream-management/design.md).

The primary sequence must reach the explicit `$special_upstream_matcher` hook.
Ascending slots select the first matching group, with diversion before manual
providers. Selected child completion exits the request, including failure or
absence of an answer. Unmatched requests resume the ordinary default sequence.
All domain providers under the managed opt-in are immutable; generic managed-domain-set mutation
must not bypass the later transaction coordinator.

The native management API exposes runtime-derived capabilities, ordered group
inventory and mutation, upstream tag/config/runtime views, local diversion-source
catalog operations and the existing manual-rule endpoints. A group create with
`slot: 0` or no slot allocates the first unused slot at or above 50; a nonzero
slot updates or creates that stable identity. Group, upstream, diversion and
manual-rule writes compile a complete candidate and publish through the managed
transaction coordinator. Group deletion removes only group-owned catalog,
manual-rule and override state; external source files remain. Unsupported
disabled upstream/source records remain visible and preserved, while unsupported
activation is rejected. The exact method/status/body matrix is in the
[frozen design](../validation-records/10-02-rust-native-special-groups-upstream-management/design.md).

For A/AAAA answers containing addresses and no DNAME, `cname_remover` removes
CNAME records and renames retained answer owners to the original question name.
Pure CNAME, DNAME, other qtypes and negative/control responses retain their wire.
Authority/additional records, ECS and actual supplier remain intact. Rewriting
wire generation must preserve response-owned group and sequence provenance
before cache publication (`ExecutionFacts::note_cname_normalization`).

## 4. Validation / error matrix

| Input | Result |
|---|---|
| No native opt-in | Existing unmanaged compiler behavior |
| Missing/unreachable explicit hook | Compile error |
| Slot below 50, duplicate slot/name/port, port 53 | Compile error |
| Port zero with custom-only true | Normalize custom-only to false |
| Explicit generated include or user reserved tag | Compile error |
| Enabled source missing, malformed or unsupported | Entire compile fails |
| Unknown field in an enabled source, even false/null | Entire compile fails; only name/type/enabled/files/url/auto_update/enable_regexp are supported |
| Enabled empty text source | Valid empty matcher |
| Disabled unsupported source/upstream | Preserve without executing |
| Unsupported enabled upstream or no enabled entries | Compile error |
| Legacy aliapi wrapper with ordinary DNS | Native forward under opt-in |
| Signed AliDNS fields/protocol | Reject, no fallback |
| Multiple managed listeners | Supervisor prebinds all sockets before any serving |
| Occupied candidate UDP or TCP port | Entire preparation fails; old generation continues |
| Managed opt-in/root or API bind change | Snapshot preparation refuses publication |

## 5. Good / base / bad cases

Base: [empty managed sample](../examples/native-managed-empty/config.yaml)
compiles without adding groups. Good: two overlapping groups select the lower
slot and cache its normalized answer with the actual supplying entry. Bad:
enabling a retained signed upstream or malformed source aborts compilation.

## 6. Required tests

`special_groups_config` asserts opt-in/hook reachability, declaring-directory
resolution, collision/identity validation, disabled data retention, protocol
conversion, generated listener inventory and immutable provider IDs. Its real
UDP peer asserts normalized miss/hit wire and actual supplier/group audit.
The native policy unit matrix covers compressed A/AAAA, pure CNAME, DNAME,
non-address/negative responses and OPT preservation. Existing client/ECS/cache
and policy tests remain regression gates. Exact-source logs and limitations are
in [S1 status](../validation-records/10-02-rust-native-special-groups-upstream-management/s1-status.md).

## 7. Wrong vs correct

Wrong: change response wire and allow generation mismatch to clear the selected
group; or mutate generated manual providers through generic rule management.
Correct: advance the facts generation only after successful CNAME normalization;
compile generated providers as immutable snapshots and route later writes
through the managed transaction coordinator.

## Save pause

Creating, changing or deleting a group compiles the candidate on the DNS thread,
so DNS pauses for about 0.4–0.65 s (production rule set; 5D-2b/5D-2c). UDP queries
that arrive meanwhile queue in the listener's enlarged kernel receive buffer and
are not dropped within its capacity; see "UDP receive buffer and apply pause" in
`native-webui-hosting.md`. Compiling off the DNS thread is the M2-3 backlog item.

## S2 runtime snapshot boundaries

`HostAssembly::control()` exposes prepare/install/retire for immutable graph
publication. `prepare_host` stages changed sockets; unchanged listeners remain
under the same supervisor owner. Publication moves prepared handles, retirement
sets and one update slot without awaiting or spawning; prepared periodic tasks
activate only after publication. Startup or preparation failure cannot start a
partial pair. The API listener and observer remain stable.

UDP captures at datagram admission; TCP captures after each complete DNS frame.
HTTP reads capture after the complete bounded request. In-flight DNS owns the old
snapshot through terminalization; snapshot-qualified metric keys never reuse a
new graph's numeric executable IDs. Shared transport owners are reused by stable
definition/entry/target identity. Retirement closes only exclusive owners, while
supervisor shutdown closes current owners. Canceled refresh joins remain owned.

Current cache reuse requires equal full supported policy fingerprints and equal
cache options; otherwise it allocates fresh owners. This conservative behavior
is not the S4 disjoint dependency-closure or durable-dump transaction contract.

## S3 startup and candidate input boundaries

Startup makes the managed opt-in and writer/recovery decision from one root YAML
snapshot and compiles that same snapshot against the main configuration's directory (include paths and managed state; plugin-argument paths use the `-d` root, which defaults to that directory). It
must not reread the root file after deciding whether to own the managed writer
lock. Candidate compilation records every file that affects the compiled graph,
including an optional `rule/special_N.txt` when it is absent and therefore
omitted from generated YAML. If that file appears before durable preparation,
the candidate conflicts and preserves the external write; existing or staged
manual bytes are compiled as the immutable candidate snapshot.

For a file-backed managed host, the immutable profile also carries the
canonical root YAML path that produced it. Each HTTP mutation compiles its
candidate from that path, and each committed candidate carries the same path
into the next runtime snapshot. The path must remain directly under the managed
state root. Management handlers never reconstruct it from the state directory
or assume a filename; an in-memory profile without a root path cannot perform a
persistent API write.

The durable coordinator and candidate apply path have an S3 implementation and
exact-source review PASS, including journal recovery and runtime publication.
The review found a startup config snapshot/lock mismatch and an untracked absent
optional manual provider. The startup compiler now uses the exact preflight YAML
snapshot, and the candidate input set records absent manual paths for conflict
checking. Both RED/GREEN regressions pass; see the
[S3 round 2 result](../validation-records/10-02-rust-native-special-groups-upstream-management/s3-review-round-2-result.md).
S5 wires management HTTP mutations through the coordinator. Its public API,
capability and concurrency tests, exact-source manifest and complete validation
record are in [S5 status](../validation-records/10-02-rust-native-special-groups-upstream-management/s5-status.md).
The first exact-source review found a root-config-path defect. The remediation
has an HTTP regression, matching isolated validation and same-ID exact-source
PASS; the formal controller advanced to Slice 6. S6 then passed its maintained
Vue browser and compatibility regression gates. S7 exercises the integrated
DNS/cache/HTTP/audit chain and Vue audit detail on controlled loopback peers;
its exact-source review and the cumulative review remain separate gates. See
the [S6 status](../validation-records/10-02-rust-native-special-groups-upstream-management/s6-status.md)
and [S7 status](../validation-records/10-02-rust-native-special-groups-upstream-management/s7-status.md).

S2 re-review corrections: admission requires tag, protocol and declared socket
address, so a retired socket cannot use a newly published binding with the same
tag. Listener entry selection participates in wire-policy fingerprints. Cleanup
continues all cache and upstream close steps despite errors, returning the first
error after required closure. The exact-source re-review passed at
`a9fe9f0eb5ceac81f7b43407acb2f0e81cdfaedf`; S3 is reviewed at
`6b50220d1fe137cd2722c44da7c0e095df9095e1`; S4 passed; S5 and S6 exact-source
reviews passed; S7 and the separate cumulative exact-source review also passed.

## Cache supplier persistence

Normalized answers retain their actual supplier entry, selected peer and transport
through memory hits, owner reuse, native save, shutdown and restart. Hits restore
response provenance independently of current network attempts and ECS echo
permission. Explicit response replacement clears origin; transparent CNAME/TTL
and scoped wire decoration preserve it.

Native `mosdns_cache_v2` dumps keep the existing gzip name, block framing and
protobuf entry fields 1–6. Optional length-delimited entry field 7 carries a
version-1 JSON object (`version`, `entry`, `peer`, `transport`) for known native
origin. Its encoded payload is bounded to 64 KiB, with entry identity bounded to
8 KiB before serialization; it participates in existing block/owned decode limits.
Duplicate fields, unsupported version/transport, invalid endpoints and malformed
metadata reject import before any merge. Existing Go protobuf readers skip the
extension and still read the original fields. Existing wire-only dumps remain
readable and their unknown historical supplier is not invented. A legacy reader
that discards the extension on resave yields wire-only data with unknown origin.
The native runtime preserves the extension on ordinary persistence and import.
