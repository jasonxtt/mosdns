# Native response policy compilation

## 1. Scope / Trigger

S1 of `rust-native-response-policy-ip-rules` compiles immutable startup policy descriptors. Runtime hosts/TTL, scoped redirect and expanded IP execution follow in S2–S4. Do not serve descriptors that the request driver cannot execute: assembly currently rejects incomplete policies before listener I/O.

## 2. Signatures

`compile_yaml` / `compile_yaml_with_base` produce `CompiledConfig.response_policies`, `ip_sets` and `response_ip_rules`. `DomainPayload<T>::lookup(&str)` selects one immutable payload; `TtlPolicy` is `Fixed(u32)` or `Range { min: u32, max: u32 }`. Hosts preserve separate IPv4 and IPv6 ordered address vectors.

## 3. Contracts

Named `hosts` accepts entries/files; `redirect` rules/files; `ip_set` ips/files and empty sets only. Missing/null lists are empty. Quick `ttl` accepts exactly one unsigned decimal uint32 or min-max pair; zero and inverted bounds compile. Expanded `resp_ip` combines literal/CIDR, `$ip_set` and `&text-file` by OR. Relative files resolve against the declaring YAML/include directory. Domain payload defaults to full match; existing core full/suffix/regexp/keyword precedence applies. Duplicate patterns replace payload without changing first registration order. Files follow inline rules and may replace their payloads. IPv4-mapped hosts literals remain in the IPv6 family.

## 4. Validation & Error Matrix

| Condition | Result |
|---|---|
| Missing IP file | warning, skip |
| Missing hosts/redirect file | startup error |
| Bad UTF-8, I/O, unknown fields or rule syntax | error with declaration/line location |
| Nonempty provider sets, SRS/binary/compressed text | unsupported error |
| Line over 64 KiB, aggregate source over 64 MiB, over 1,000,000 noncomment rules | startup error |
| Blank/comment-only file | valid empty snapshot |
| Bad overwritten rule | error; replacement cannot hide malformed input |

Source byte budgets include comments and repeated rules count toward the rule budget. Anonymous IP sources share the owning sequence's budget. Inline ip_set items must contain exactly one IP/CIDR; text IP files may include trailing fields. No request-time file reads or hot reload.

## 5. Good/Base/Bad Cases

Good: `resp_ip $networks 2001:db8::/32` compiles immutable OR snapshots. Base: empty hosts rules produce an empty payload matcher. Bad: `::1/129`, `ttl +1`, invalid redirect target, or nonempty sets fail at startup.

## 6. Tests Required

`native-host/tests/policy_config.rs` verifies public YAML, payload precedence/replacement, declaration paths, immutable snapshots, empty/missing/bad/binary files, all resource limits, numeric bounds and fail-closed assembly. Existing config negative tests must reject invalid IPv6 widths rather than valid newly supported IPv6 syntax. Run remote workspace clippy, full native-host tests and fmt. After rsync preserving source mtimes, touch changed sources before Cargo validation to avoid stale cached artifacts.

## 7. Wrong vs Correct

Wrong: compile a policy then silently execute the ordinary forward path, or reload files on each request. Correct: snapshot and validate at startup; refuse assembly until its driver supports the descriptor, then pass the immutable snapshot into the real native execution path.

# Hosts and TTL execution (S2)

## 1. Scope / Trigger

Native root, branch sequence and direct fallback target dispatch execute hosts and quick TTL policies. Redirect and expanded IP runtime remain gated.

## 2. Signatures

`apply_wire_policy(&ResponsePolicy, &mut ExecutionState, &[u8]) -> Result<bool, ExecutorError>` returns true only when hosts replaces the response. `clamp_response_ttls(&[u8], min: u32, max: u32) -> Result<Vec<u8>, ResponseError>` shares dns-core's bounded record walker.

## 3. Contracts

Hosts only answers IN A/AAAA, preserving configured order with TTL 10. Empty selected family emits NOERROR and one SOA at the current question: TTL 300, ns `fake-ns.mosdns.fake.root.`, mbox `fake-mbox.mosdns.fake.root.`, serial 2021110400, refresh 1800, retry 900, expire 604800, minimum 86400. No match, both families empty, non-IN or non-address types leave the entire response/generation unchanged. Successful hosts sets Local supplier and clears selected endpoint, preserving historical attempts. Policies Continue; only subsequent sequence rules may accept/reject or overwrite them.

TTL fixed zero is a validated no-op. Positive fixed TTL replaces all non-OPT record TTLs in Answer/Authority/Additional. Ranges apply nonzero minimum then nonzero maximum, including inverted ranges. OPT bytes remain unchanged; TTL preserves supplier identity. Wire edits are prepared on a copy and committed only on success.

## 4. Validation & Error Matrix

Malformed TTL wire returns executor error with unchanged state; root converts executor failure to the existing local SERVFAIL path, branch propagates typed failure. Too many hosts answers or a response exceeding 65535 bytes fails before committing. Assembly permits hosts/TTL but still rejects pending redirect/new IP evaluation.

## 5. Good/Base/Bad Cases

Good: hosts → ttl 42 → has_resp accept yields a Local TTL-42 answer. Base: empty hosts match preserves an earlier response. Bad: ttl applied to malformed response cannot publish a partially changed packet.

## 6. Tests Required

`policy_wire.rs` drives real UDP for dual-stack order, FakeSOA fields, no-op/Continue behavior, numeric TTL semantics, upstream→hosts supplier replacement, and direct fallback hosts→TTL. `policy::wire_tests` asserts response/generation preservation and malformed atomic failure. dns-core tests assert all-section range edits, unchanged OPT and malformed late-record failure. Regress full native-host and dns-core suites plus workspace all-target clippy/fmt remotely.

## 7. Wrong vs Correct

Wrong: treating hosts as implicit Accept or erasing all upstream attempts when it replaces a response. Correct: Continue through the canonical sequence machine; clear current selection and retain completed attempt history.

# Scoped QueryView and redirect execution (S3)

## 1. Scope / Trigger

Redirect consumes only its enclosing successor under the same root fuel, absolute deadline and cancellation tree. Immutable admission remains owned by the request/checkpoint. Native branch contexts use one `QueryView` containing current owned/Rc wire, header and question; matchers use the corresponding machine query state.

## 2. Signatures

`QueryView::redirect(&str) -> Result<QueryView, ExecutorError>` safely decodes/re-encodes the actual outgoing query. `run_redirect` drives a captured successor and returns its typed `BranchOutcome`; restoration is local to that frame. Branch supplier state carries `Option<ResponseSource>`, including Cache rather than conflating every non-network response with Local.

## 3. Contracts

IN matching redirect changes qname while preserving ID/qtype/class/flags/allowed OPT. Forward, cache lookup/store, hosts, query matchers and preference probes consult the current view. Preference's alternate-QTYPE wire and machine question must agree. On successful response (including successful exit), rebuild Question to the original and prepend original→target IN CNAME TTL1; preserve target chain, SOA, rcode, flags and TTLs. Every nested frame restores one layer. Missing response/error/cancel/drop cannot synthesize a CNAME success.

Outer cache captures the restored original wire/key; inner cache captures target wire/key before restoration. Completion remains `Exited` through named targets, fallback/preference wrappers and redirect; resume Exit so enclosing watches get ScopeAborted. No conversion to natural completion/publication. Natural completion resumes Accept to retire the consumed enclosing scope. Its captured jump continuations have already run; Return would pop and re-execute an original continuation. The caller's outside scope may continue normally.

Supplier identity and actual historical attempts survive redirect; local/cache replacement clears current network selection. Branch collector transfers detail records only when audit capture is enabled, and transfers numeric metrics regardless. Trace selection must reject stale candidates when the selected outcome's source is Local/Cache.

## 4. Validation & Error Matrix

No match or non-IN: ordinary Continue, unchanged view. Malformed query/response or oversized restored wire: executor error, no partially installed wire. Error/cancel/fuel failure restores the frame's original machine question and propagates the typed error. Self/cross-plugin cycles exhaust shared root fuel; no new budget, no successful response manufacture. Assembly now gates only pending expanded IP execution.

## 5. Good/Base/Bad Cases

Good: a→b, peer b→d CNAME + d A yields original Question and a→b→d chain. Base: target NXDOMAIN retains its target-owned SOA and RCODE with a→b CNAME. Bad: converting exit through a named fallback target to Completed runs parent TTL and publishes outer cache; preserve Exited instead.

## 6. Tests Required

`policy_wire.rs`: actual target peer question, nested negative restoration, flags/QTYPE/OPT, cache before/inside redirect and warm hits, direct fallback target and non-IN no-op, exit/outer cache abort, named fallback exit skipping parent tail, bounded self-cycle. Native execution unit test verifies error/cancel/shared fuel typed results and unchanged inherited response without CNAME decoration. Regress full native-host suite and workspace clippy/fmt remotely.

## 7. Wrong vs Correct

Wrong: changing display qname while sending admission wire, patching compressed DNS question bytes in place, or reconstructing successful completion after exit. Correct: current owned QueryView; safe full message decode/encode; frame-local restoration and the original typed completion.

# Immutable response-IP matcher (S4)

## 1. Scope / Trigger

All compiled response-IP expressions now execute natively. The S1 placeholder and runtime_ready assembly restriction are removed; no legacy/special case path remains for single IPv4.

## 2. Signatures

`ResponseIpMatcher::new(Vec<Rc<IpPrefixList>>)` receives ready immutable provider/anonymous snapshots; `Matcher::evaluate(&ExecutionState)` returns a read-only `MatchOutcome`.

## 3. Contracts

Missing/Synthesized response is false. Raw response scans only Answer A/AAAA and applies OR across observed addresses and referenced prefix lists. Authority/Additional addresses and CNAME alone do not match. IPv4, IPv6 and mapped IPv6 share matcher-core's normalized 16-byte prefix contract; no string matching. Literal/CIDR, `$ip_set` and `&text-file` all use the same evaluator and startup resource/path/error contracts.

## 4. Validation & Error Matrix

Malformed raw wire returns MatcherError, with no state/routing/response mutation. Empty snapshots cannot match but may be ORed with other lists. No request-time file reads: file modification/removal after assembly does not alter results.

## 5. Good/Base/Bad Cases

Good: AAAA ::ffff:192.0.2.1 matches 192.0.2.0/24. Base: valid response with CNAME and only additional/authority addresses returns false. Bad: truncated Answer returns error rather than publishing a partial match.

## 6. Tests Required

Real UDP sequence cases in policy_wire verify dual-stack subnet inclusion/exclusion, named/anonymous OR, mapped/zero-prefix behavior and text-file immutability. Matcher unit test proves Answer-only semantics, malformed error and unchanged state. Existing matcher-core prefix golden tests cover exact and boundary/mapped contracts. Regress full native-host and matcher-core suites plus workspace clippy/fmt remotely.

## 7. Wrong vs Correct

Wrong: accepting expanded syntax but serving a pending matcher or reading rules during requests. Correct: execute the already-built immutable prefix snapshots through one matcher for every expression shape.

# Policy composition and retained dumps (S5)

## 1. Scope / Trigger

Sequence order controls policy/cache/fallback/preference composition. Existing cache admission, v2 dump/key format and supplier schema remain unchanged.

## 2. Signatures

Existing `NativeCacheAdapter::{lookup_entry,dump,import_dump,save,flush}` and `HostOptions::with_cache_clock` provide lifecycle verification. No new management endpoints or policy generations.

## 3. Contracts

TTL inside a cache's successor is stored; TTL outside its named child scope modifies only the returned client wire. `resp_ip` can match target Answers before restoration or restored CNAME+A after cache/redirect; it remains read-only. Lazy DNS refresh reuses the owner-bound immutable startup hosts/rule snapshot. Prefer probes must consult the current redirected question and corresponding alternate QTYPE state.

Fallback siblings hold independent QueryViews. A canceled branch cannot leak its target CNAME or selected supplier; actual attempt history retains both branches, including cancellation. Final source may be Cache/Local/Upstream according to the selected wire.

Restart with changed policies and retained same-key v2 dump can hit old data. There is no policy identity/generation in key/dump. For immediate replacement, quiesce query producers while management remains available, complete durable Flush, then switch/restart without refill; alternatively stop/drain/final-save and remove the owned dump before restart. Closing the cache owner/listener before management makes Save/Flush return Closed and is not an executable flush sequence.

## 4. Validation & Error Matrix

Closed cache owner: Save/Flush/Import reject Closed. Preserved dump with changed rules: valid old hit within retention, not automatic invalidation. Flush commit followed by reload of its bound file: no old snapshot resurrection. Background rule-file changes/removal: no effect on current snapshot.

## 5. Good/Base/Bad Cases

Good: cache child hosts TTL10, caller ttl30 yields client TTL30 and stored TTL10. Base: retained dump still supplies old hosts address after owner reconstruction. Bad: assuming deleting a dump during a live owner's lifecycle prevents final-save resurrection, or promising automatic policy invalidation.

## 6. Tests Required

Seven composition cases in policy_wire prove TTL placement/aging, actual v2 bound-file Save/new-owner Import/Flush/re-import, lazy refresh with mutated hosts file, IP-selected fallback supplier/attempts, redirected prefer probe QTYPE, canceled sibling redirect isolation, and target/restored IP matching across cache boundaries. Unit-level owner reconstruction is not process restart proof; S6 supplies actual process/API/Vue/restart evidence.

## 7. Wrong vs Correct

Wrong: silently adding policy keys or excluding every Local response from caching. Correct: keep existing keys/admission and explicitly disclose retained-dump compatibility; verify quiescent durable flush and scoped publication.

## Public proof and rule-change cache contract

Public process proof belongs in the owning task's research/public-proof: real DNS wire, API supplier/attempt projection, maintained Vue, and actual SIGTERM/restart. Separate UDP/TCP runs honor the single-listener host contract. Query-cache v2 dump is not policy-versioned: immutable policy reload alone does not invalidate a retained old answer. To apply changed rules immediately, quiesce producers, Flush through the still-open owner API, then stop/drain/final-save and restart; do not stop owner admission before Flush, or permit refill between Flush and stop. Keep this limitation visible rather than adding unapproved hybrid generations or schema changes.
