# KixDNS reuse ledger

Audited upstream: `https://github.com/olicesx/kixdns`

Pinned commit: `2da3a2d59466e996a0f846c3e7e504970b878b06` (`2026-08-12`, `fix(geoip): preserve overlapping dat tags (#37)`).

KixDNS and this project are GPL-3.0, but every extracted/adapted source file still needs an origin URL, pinned commit, copyright/license notice, and local modification note. An upstream update is reviewed as a new change; this ledger does not authorize following its default branch automatically.

| Capability | Upstream evidence | Decision for cache foundation | Reason / gate |
|---|---|---|---|
| Concurrent cache | `src/cache.rs`, `src/engine/core.rs`, Moka usage in engine | direct `moka` dependency; adapt architecture | Avoid copying a thin wrapper; prove MosDNS capacity/expiry semantics independently |
| Immutable wire bytes | `bytes::Bytes` throughout engine/upstream | direct `bytes` dependency | Cheap immutable sharing fits raw cached responses |
| Cache hit flow | `src/engine/phases.rs::check_cache` | design reference, then selective adaptation | KixDNS pipeline/stale semantics differ; MosDNS fixtures control behavior |
| Query/response fast parse | `src/proto_utils.rs` | adapted in `rust/cache-core/src/wire.rs` | Local walker validates headers, labels/compression pointers, RR bounds, and response shape before returning copied bytes |
| TTL patch | `src/proto_utils.rs`, calls from `engine/phases.rs` | adapted in `rust/cache-core/src/wire.rs` | Fresh hits use saturating subtraction; lazy hits set TTL 5; OPT pseudo-RR flags are preserved |
| ECS | `src/ecs.rs`, hash use in `engine/execution.rs` | facade/design reference only for this slice | Go already produces the frozen canonical key including ECS text; parsing or masking it again in Rust would duplicate the contract and risk divergence |
| UDP truncation/EDNS size | `src/proto_utils.rs`, `src/engine/phases.rs` | retained in Go facade | `EntryHandler` uses the existing `dns.Msg.Truncate`/response-OPT path only when needed; no second truncation implementation is introduced |
| Refresh deduplication | `src/engine/utils.rs`, `src/engine/refresh.rs` | design reference only in this task | Lazy refresh remains in Go to avoid crossing sequence/query context through ABI |
| Rule/pipeline cache | `src/engine/pipeline.rs` | reject for cache foundation | KixDNS JSON pipeline and rule semantics are not MosDNS sequence semantics |
| Upstream transports | `src/engine/transport.rs`, `upstream.rs` | defer to later migration phase | Outside cache boundary and deeply coupled to KixDNS engine |
| Full application/config | `src/main.rs`, `src/config.rs` | reject | Would replace stable YAML/plugin/coremain/WebUI contracts |

## Matcher / rule-index candidates (Slice 0 audit, same pinned `2da3a2d`)

Audited for the `08-13-rust-matcher-foundation` task. MosDNS semantics are
authoritative; KixDNS matcher code is a design/parsing reference, not a
behavioral contract. Match behavior must be re-validated against the Go golden
fixtures in `docs/rust/matcher-compatibility.md`.

| Capability | Upstream evidence | Decision for matcher foundation | Reason / gate |
|---|---|---|---|
| GeoSite `.dat` parser + hot reload | `src/matcher/geosite.rs` (1438) | adapted (parsing/reload); matcher semantics NOT copied | V2Ray-style dat decode and the build-new-snapshot + swap reload are useful. The `DomainMatcher::Suffix` rule uses `domain.ends_with(suffix)` with no dot boundary, so `badexample.com` matches `example.com`; MosDNS uses a label trie with strict label boundaries. The `Full/Suffix/Keyword/Regex` enum maps to MosDNS `full/domain/regexp/keyword` but only as a shape reference. |
| GeoIP `.dat`/MaxMind/MMDB | `src/matcher/geoip.rs` (1175), `geoip_converter.rs` (684), `geoip_proto.rs` (33) | reference / defer | MosDNS `ip_set` and `netlist` use IPv4-mapped prefix binary search, not MMDB. V2Ray GeoIP `.dat` decode may inform a later GeoIP source, but prefix semantics remain `netlist`; MMDB conversion is out of scope. |
| Compiled rule index / candidate fast path | `src/engine/pipeline.rs` (559), `src/matcher/advanced_rule.rs` (361) | design reference | `RuleIndex::get_candidates` + exact O(1) map + suffix index built by stripping labels + `query_type` index + candidate merge/sort + first-rule-priority and safe-fast-path exit are the right shape for a MosDNS compiled index. The JSON pipeline rule/action model is not adopted. |
| Matcher evaluation context | `src/engine/matcher_adapter.rs` (56), `MatcherContext` | design reference | Bundling qname/qclass/client_ip/qtype/edns into one matcher-evaluation context matches how MosDNS will hand a query-context slice across FFI; ownership stays Go-side. |
| Runtime rule model | `src/engine/rules.rs` (1092), `RuntimeMatcher`/`RuntimeRule` | reject | KixDNS JSON pipeline rule/operator semantics do not map to MosDNS `sequence`/`domain_set`/`ip_set` behavior; fixtures own the contract. |

Reuse principle for the matcher slice: extract dat decode, snapshot-swap reload,
and candidate-index ideas; never adopt KixDNS suffix/keyword matching semantics,
JSON pipeline rules, or its regex dialect as MosDNS behavior.

## Prior MosDNS Rust cache prototype

Reference workspace: `/Users/tom/github/mosdns-rust-cache`, commits through `44e1c88` plus its explicitly dirty files.

Reuse selectively:

- Go bridge/environment opt-in shape;
- `mosdns_cache_v2` protobuf dump mapping;
- raw query-context/server-handler test cases;
- Linux static library build mechanics;
- parity/benchmark command ideas.

Do not copy:

- the single `Mutex<CacheCore>` runtime;
- the `HashMap` plus linear `l1_find_slot`/`l1_remove` scan;
- buffer release that reconstructs `Vec` from pointer and length without the original capacity;
- FFI-reachable `unwrap()`/panic behavior;
- stale release workflows or unrelated dirty worktree changes.

The prototype is evidence that the integration boundary is viable, not evidence that the implementation is production-safe.
