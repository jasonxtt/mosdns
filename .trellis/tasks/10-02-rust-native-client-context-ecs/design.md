# Client context and ECS design — PLAN READY

Planning only; public behavior proposals require final approval. One task, S1–S6; no new lifecycle roots except existing cache refresh.

## Trusted client and query state

Add immutable ClientContext {peer_ip: Option<IpAddr>, transport} to execution input and refresh capture, supplied by actual UDP recv/TCP accepted socket. Unknown direct-embedding identity is None; no loopback guess. Unmap IPv4-mapped peer; never source client_ip from ECS or proxy headers. Reuse native startup IP matcher catalog, literal/CIDR/$ip_set/&plain_file OR and existing negation. client_ip is matcher quick grammar, no named type; existing IP format deferrals hold.

QueryView carries original immutable client EDNS snapshot, current outbound options, ECS provenance (ClientAdmission or Policy), response-OPT state. Admission IDs/qname/client_addr remain original. All scoped branches/redirect/prefer/ref refresh copy client/context view; no global mutable ECS state. cache dispatch key and network exchange use the same current outbound view. ECS handler typed invocation descriptor remains External ID-only; no generic argument payload expansion.

## Handler matrix and ownership

Named ecs_handler fields: forward/send false, preset empty, mask4/6 default0→24/48. integer validbounds0..32/128, negative/out-of-range/type errors; preset parse/unmap. Legacy quick ecs: empty no-op, first token IP before slash (deprecated mask ignored with warning), trailing tokens ignored warning; not CIDR mask configuration. No added alias.

The first explicit handler evaluates ClientAdmission ECS through its immutable original snapshot; it removes only that admitted ECS from its *local* outbound view before selecting policy. It does not alter handler-free direct forwarding. Existing Policy-origin current ECS (set by enclosing/preceding handler) wins unchanged. Otherwise valid incoming when forward=true wins over preset/send; then preset; then send with knownpeer; finally no ECS. Non-IN leaves no new insertion, and does not silently rewrite existing raw query outside profile.

Insertion/replace uses structurally validated OPT; preserve DO, UDP payload and existing supported non-ECS options, ID/qtype/question. Without OPT and nonempty selection create valid EDNS0 OPT with UDP size1232 and DO=false; preserve original lack of client OPT for response. Masks generate scope0 and masked prefix bytes. Malformed/duplicate/family0/nonzero queryscope ECS: cache bypass when merely forwarded, typed local failure when explicit handler tries to interpret it; no parser-only acceptance, no partial wire mutation. Cancel/runtime errors propagate, no new CNAME/supplier or success.

Handler runs own successor to enclosing boundary with current root fuel/deadline/cancellation, then selects response ECS from the true supplying response only if forwarded client-origin ECS and client originally had OPT. Response must have one supported matching family/source prefix/address ECS with legal scope <=source mask; noscope reuse. Generated ECS doesn't echo. Strip inherited upstream ECS from final response unless this invocation authorizes echo; keep other permittedOPT/DO fields. Cachehit has no upstreamOPT, return an ECS-free EDNS response appropriate to original client, not invented scope. Nested handlers propagate explicit echo authorization; only the one supplyingresponse is consumed, no loserOPT. Zero/empty/no-op handler does not change supplier, routing or terminal completion.

## Cache key and dump compatibility

False/quick: same old ECS bypass, even client_ip now available; no clientIP added to ordinary key. True: base AD/CD/DO/type/textname + nonempty ECS suffix lengthbyte/string in Go v2 layout. No ECS is unchangedbase. Identity proposal: canonical masked network family/source/scope0; same prefixnetwork shares, differentnetwork/family/mask separates. Use exact Go IPv4 and bracketed IPv6 formatting. Family0/unsupported/duplicate/queryscope!=0 never inserted.

Go logical String can carry hostbits before pack. Import validates original suffix then canonicalizes maskednetwork, explaining this as native wire-semantic normalization; fullinput key stays same schema but byte identity isn't promised for noncanonical legacysuffix. Merge duplicate normalizedkeys deterministically in dump order (last wins like ordinary collision), enforcecap. Export canonical Go-compatible keys; actual Go reader fixture must demonstrate roundtrip parsing/matching to canonical query. Document generated Go prepackhostbits may require its own cache refill after native export. Do not normalize address acrossIPv4/IPv6 family.

prepare_import fullvalidates key base+suffix+response question and allentries (including expired), time/UTF8/budgets before singlemerge. false owners reject ECS dumpall, true acceptprofile; no silent suffixdrop. Existing16MiB compressed/64MiB cumulativedecoded+owned/100000entries/1MiB block, timestamp and generation unchanged. Store DNS without OPT as priorv2, so no upstreamECS echo metadata on hit/import; no protobuf/UI schema additions.

## Refresh/facts and management

Configuration assembly computes conservative may-transform-ECS and may-branch-on-client summaries for each cache invocation's successor up to its publication boundary. Traverse nested calls, jump/goto/try, fallback/preference and inherited continuations; recursive summaries use a monotone fixed point. Conditions are potentially reachable unless control flow itself makes them unreachable. Reject all named/quick, true/false cache placements with either effect, including unknown dynamic reachability. Report both callsite and offending policy. Empty/no-op handlers have no transform effect; conservatively reject potentially active handlers even when an earlier Policy ECS might make them no-op. No runtime reorder or operator-only warning. Policy must precede cache; distinct client routing paths that vary answers beyond ECS require separate cache owners. This does not claim automatic partitioning for all unrelated routing dimensions.

The first refresh admission captures an immutable owned dispatch peer/ECS/state snapshot. Followers do not overwrite it; unknown remains None. A captured real peer needs no live socket and preserves successor semantics. A blanket None would change those semantics. The placement gate excludes downstream client_ip/send effects absent from the key. This is never a global last-client variable.

Supplier ECS validation failure (duplicate, malformed, mismatched family/source/network, invalid scope) removes ECS only, preserving an otherwise valid response; malformed DNS retains the existing validation failure path. Only the winning response may authorize echo.

Refresh captures dispatch ClientContext/current ECS/provenance and original clientOPT to replay selectedsuccessor; client disconnect doesn't clearidentity. samekey singleflight uses fullcanonicalkey; oldgeneration afterflush/import cannot publish. Existing64fuel/5s/256/noqueue, no nestednewrefreshroot. Background trueupstreamattemptmetrics only; clientquery/audit/ranking/admission unchanged. Peer metadata is executiondata irrespective auditoff.

Management existinginventory/metrics/show/save/load/flush; show format base + readable ECSsuffix withoutbreakingVueDNS parsing. No newECS controls/detailsfields. Native v2 false-owner ECSimport400 surfaced; newtrue ownerrestart preservespartition/age/domain_set. Captureon/off finalsupplier remains actualnetwork/local/cache source.

## Slices and operational scope

## State ownership and transitions

| Event | Incoming ECS and peer | Current outbound ECS | Supplier and final echo |
| --- | --- | --- | --- |
| Admission | Immutable original snapshot and trusted peer | Raw ECS tagged ClientAdmission | No supplier/token |
| Explicit IN handler | Read-only original | Child view preserves Policy; otherwise strips admission ECS and selects forward, preset, send, none | Only forwarded incoming ECS grants eligibility |
| Non-IN handler | Unchanged | No insertion or policy rewrite | No new eligibility |
| Nested handler | Unchanged | Policy remains scoped | Inherit eligibility for same outbound ECS; never promote generated ECS |
| Redirect/reference | Original client OPT/peer unchanged | Clone current view; redirect changes question | Token remains tied to selected outbound ECS and returned supplier wire |
| Fallback/preference | Independent child copies | Only accepted winner commits; siblings cannot mutate | Winner supplier/token only; losers drained/discarded |
| Cache dispatch/hit | Unchanged | Key reads current view; hit preserves view | Stored wire has no upstream OPT; no invented echo; reconstruct base client OPT |
| Refresh | First admission owns snapshot | Independent owner copy; full-key singleflight | No client audit mutation; background supplier only for publication |
| Error/cancel/exit | Immutable originals preserved | Existing terminal scope rollback/commit | No fabricated supplier or echo; preserve valid existing terminal response only |

Echo eligibility is a token bound to the selected outbound ECS and supplying response, not inferred from response bytes. Changing outbound ECS invalidates its token. QueryView restore/commit must retain the association and cannot attach a parent's token to an unrelated child's wire.

Dump normalization is the explicit R9 deviation in PRD and must have semantic, not byte-identical, acceptance proof.

UDP/TCP socket identity only, no DoH/PROXY/XFF or multi-listener. Text IP snapshot only. Strict profile validation belongs native-host helpers, not a retroactive rewrite of the approved foundational EDNS parser. No scope-covering cache or production claims. Evidence uses controlled upstream plus DNS/API/browser and Go generator/reader on isolated mosdns-rust. C2C iteration 1 returned PLAN READY; see research/planning-review.md.
