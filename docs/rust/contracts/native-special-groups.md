# Native managed special groups

## 1. Scope / trigger

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
snapshot and compiles that same snapshot against the declaring directory. It
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
