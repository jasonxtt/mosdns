# Planning source audit

Source anchor: `e4dcc71398c2412a4d42a5be6bad3ddcca1f0bc0`, branch `rust`.
Planning reads only; no Rust/product code, benchmark, VM or deployment run.

| Source | Finding / planning consequence |
|---|---|
| `rust/cache-core/src/lib.rs` | `CacheState`, immutable `CacheEntry`, Moka storage/TTL data exist; public entrypoints use integer handles and ABI pointer containers. Extract/share owned safe API, preserve adapters. |
| `rust/cache-core/src/wire.rs` | Lookup TTL patch copies; OPT TTL excluded. Preserve established core semantics and explicitly test native/ABI parity. |
| `rust/native-host/src/config.rs` | Strict duplicate-aware three-plugin W1 graph, one forward sequence. W2 needs explicit four-plugin/two-step alternative; preserve W1 rejection gates. |
| `rust/native-host/src/assembly.rs` | Host ownership/current-thread runtime pattern is retained; no multithread conversion needed. |
| `rust/native-host/src/udp.rs::execute_request` and `tcp.rs` | Shared canonical machine dispatch already exists. Forward failure sets synthesized SERVFAIL and resumes Continue: completion is not sufficient proof of cacheability. Track upstream provenance. |
| `rust/sequence-core/src/engine.rs` | Dispatch/resume and Accept/Continue/Complete suffice for the restricted flat W2 root. A scoped request token models store-after-continuation without a second interpreter or a speculative general callback ABI. |
| `rust/dns-core/src/query.rs` | Exactly one standard question, zero answer/authority and ARCOUNT <= 1; self-contained question labels. W1 can forward EDNS. W2 must explicitly bypass lookup/store for ARCOUNT=1 and non-IN, preserving W1. |
| `rust/dns-core/src/response.rs` | Existing response validation, minimum TTL observation, TTL aging helpers. Prefer a narrow metadata extension if OPT/RCODE observation needs it. |
| `plugin/executable/cache/cache.go::Exec` | Hit stops continuation, miss executes remainder before store; lazy path alone has singleflight. No new cold singleflight guarantee. |
| `plugin/executable/cache/cache.go::getMsgKeyBytes` | Key distinguishes AD/CD/DO, qtype and case-preserving qname; ID excluded. Old omission of qclass is not a new native collision contract; initial native cache eligibility is IN only. |
| `plugin/executable/cache/cache.go::saveRespToCache` / `pkg/dnsutils/msg.go` | TC skipped; negative/empty/zero TTL policies documented in PRD. Go strips OPT structurally; initial native W2 instead bypasses OPT caching, with full EDNS parity deferred. |
| `tests/phase5a-baseline/configs/cache.yaml` | Exact `size:64`, `lazy_cache_ttl:0`, UDP/UDP, cache then forward. Immutable acceptance input. |
| `tests/phase5a-baseline/workloads/cache.jsonl` | Two IN A hot cases: cache-a.test. → 198.51.100.20 and cache-b.test. → 198.51.100.21. Immutable expectation corpus. |
| `docs/rust/phase5a-go-baseline.md` | Separate cold and warm lifecycles, prefill verification and counter barrier; historical low-load QEMU runs are not capacity evidence. |
| `.trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-forwarding/research/linux-w1-correctness.md` | Authorized Linux host is `ssh mosdns-rust`, isolated temporary correctness run and cleanup; retain this environment boundary. |
| `.github/workflows/test.yml`, `scripts/build-rust-cache.sh` | Existing tagged cgo/staticlib regression path; use its current environment settings for ABI safety verification. |
| `docs/rust/cache-compatibility.md` | Historical semantic/ABI reference. Full plugin parity remains future work; old memory threshold does not override current performance priorities. |

## Deliberate bounded choices

- New owned cache API is necessary; new general sequence continuation machinery
  is not necessary for the frozen flat W2 graph. Review this boundary before code.
- Keep W1 supported inputs/results. Cache bypasses non-IN, ARCOUNT=1 queries
  (including EDNS), and OPT responses; no implicit full-cache claim.
- Existing upstream source/ID/structure checks do not prove response-question
  identity. Before cache publication, explicitly match decoded name/type/class
  and require one QUERY response question; mismatch bypasses storage.
- Cache retention and on-wire TTL are separate; floor applies only to computed
  zero retention. Test core aging behavior rather than copying Go internals.
- No product decision remains unresolved within agreed W2. Broader policy changes,
  such as singleflight, EDNS or arbitrary sequence cache middleware, are deferred.
