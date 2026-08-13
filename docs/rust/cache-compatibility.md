# Rust cache compatibility contract

Last updated: `2026-08-13`

This matrix freezes the current Go cache behavior before the Rust backend is introduced. MosDNS behavior is authoritative; KixDNS and the prior Rust prototype are implementation references only.

## Baseline

| Field | Value |
|---|---|
| Branch / commit | `rust` / `3896a4a7e0ce4311b40a7e4c80c93f2c8b3b4f1d` (`v0.7.1`) |
| Host | Apple M4, macOS Darwin 25.5.0, arm64 |
| Go | `go1.26.4 darwin/arm64` |
| Rust | `rustc 1.95.0`, `cargo 1.95.0` |
| Node / npm | `v22.22.2` / `10.9.7` |
| Focused baseline | `go test ./plugin/executable/cache ./pkg/cache ./pkg/query_context ./pkg/server_handler` — pass |
| Full baseline | `go test ./...` — pass |

The worktree was already dirty when the Rust task started, so `git describe` reports `v0.7.1-dirty`. The commit above, not the dirty suffix, is the code baseline.

## Behavior matrix

| Contract | Current Go behavior | Executable owner | Rust status |
|---|---|---|---|
| Plugin/YAML | Plugin type `cache`; fields `size`, `lazy_cache_ttl`, `enable_ecs`, `exclude_ip`, `dump_file`, `dump_interval`; scalar and list `exclude_ip` accepted | `cache.go`, existing config paths; Rust selection is an additive environment override | facade |
| Key | `[AD/CD/DO flags][QTYPE BE][qname length][qname][optional ECS text length+text]`; only standard query opcode with one question | `TestCacheContractKeyEncoding`, `TestCacheContractRejectsNonStandardQueries`; Rust receives these bytes as an opaque key | exact |
| Positive response | Minimum RR TTL controls message expiry; lazy TTL controls cache retention when enabled | `TestCacheContractResponseTTLsAndMetadata`, `TestRustCGOBackendMatchesGoCacheSemantics` | semantic |
| NXDOMAIN | Message/cache TTL is 30 seconds | same parity tests | semantic |
| SERVFAIL | Message/cache TTL is 5 seconds | same parity tests | semantic |
| Empty NOERROR | Minimum fallback is 5 seconds; normal calculated TTL is capped at 300 seconds | same parity tests | semantic |
| Truncated response | Not cached | `TestCacheContractSkipsTruncatedAndExcludedResponses`; Go facade filters before mirroring to Rust | facade |
| EDNS response OPT | Removed from stored response; query context owns response OPT separately | `TestCacheContractResponseTTLsAndMetadata`, `TestRawResponseEDNSAndUDPTruncationUseMessagePath` | semantic |
| ECS isolation | ECS text is included only when enabled and present | `TestCacheContractKeyEncoding`, `TestRustCGOFacadeHonorsECSAndExcludeIP` | exact key bytes |
| `exclude_ip` | Any matching A/AAAA answer prevents storage/lazy refresh | `TestCacheContractSkipsTruncatedAndExcludedResponses`, `TestRustCGOFacadeHonorsECSAndExcludeIP` | facade |
| Lazy hit | Expired message retained by cache is returned with TTL 5 and marked lazy | `TestCacheContractLazyHit`, Linux bridge lifecycle/semantic tests | semantic |
| `domain_set` | Stored with entry and restored on hit/dump | Go contract and Linux bridge parity tests | exact metadata |
| Dump | gzip name/header `mosdns_cache_v2`; length-prefixed protobuf blocks of up to 128 entries; block limit 1 MiB; disk replacement and import validation complete before mutation | dump contract/transaction tests plus `TestRustCGODumpFacadeImportsOnlyValidatedEntries` | exact format, facade persistence |
| API | GET `/flush`, `/dump`, `/save`, `/show`; POST `/load_dump`; current status/body behavior preserved | `TestCacheContractAPIPaths`; validated imports mirror into Rust | facade |
| Metrics | `cache_query_total`, `cache_hit_total`, `cache_lazy_hit_total`, `cache_size_current`, label `tag` | metric contract tests; Rust active size comes from ABI `cache_len` | facade |
| L1 | 256 shards, 200 entries/shard, CLOCK-style replacement | current `Cache`; Rust must not reproduce a separate linear slot scan | replace with Moka design |
| Close/reload | close triggers an atomic Go-format dump, closes notification once, and idempotently closes both backends | Go lifecycle plus Rust ABI lifecycle/circuit-breaker tests | semantic |
| Raw UDP/TCP/HTTP | Rust ages TTLs once per lookup; query context owns returned bytes and decodes only for plugin/audit consumers; server patches TXID/RA, preserves HTTP/TCP framing, and uses the existing Go message path for EDNS/UDP truncation | Rust wire tests, `context_raw_test.go`, `entry_handler_raw_test.go`, `audit_raw_test.go`, Linux+cgo tests | semantic |
| Backend default | Go, with no Rust toolchain/cgo requirement | default build/full Go test and `TestRustBackendSelectionIsExplicitAndFallsBack` | exact |

## Parity result notation

As Rust cases land, replace `pending` with one of:

- `exact`: byte-for-byte identical output is required and achieved;
- `semantic`: differences are limited to expected TXID/TTL/time fields;
- `facade`: behavior stays wholly owned by Go and is exercised with Rust active;
- `blocked`: mismatch is documented and Rust cannot become default.

No row may be removed to make a mismatch disappear. A deliberate contract change requires a separately approved config/API migration.

## Benchmark protocol

The first comparable benchmark must record:

- exact binary commit and Rust/KixDNS dependency revisions;
- OS, CPU, logical cores, power mode, Go/Rust toolchains;
- query corpus, cache size, warm-up, concurrency, run duration, repetitions;
- Go/Rust QPS, p50/p95/p99, process CPU, peak/steady RSS, Go allocations and cgo calls;
- raw results plus the aggregation command.

Rust remains experimental when p99, CPU, or RSS is more than 10% worse under the same workload.
