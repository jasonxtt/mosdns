# Rust cache foundation — implementation plan

Each slice is test-first and leaves the default Go build working. Stop and return to planning if a compatibility fixture reveals an unresolved semantic choice.

## Slice 0 — Baseline and contract freeze

- [x] Record the exact branch/base commit, toolchain versions, build commands, and benchmark environment.
- [x] Inventory YAML fields, key construction, hit classes, TTL behavior, API paths/shapes, metrics, dump blocks, lifecycle, and raw transport behavior from current source.
- [x] Add Go golden/parity fixtures and missing focused tests before Rust runtime code.
- [x] Create the KixDNS reuse ledger pinned to `2da3a2d`, with origin/license/decision per candidate module.
- [x] Run and record focused cache tests plus `go test ./...` as the pre-change baseline.

Exit: every compatibility item in PRD R1 has a test/fixture owner; baseline is green or pre-existing failures are documented.

## Slice 1 — Rust crate and safe ABI skeleton

- [x] Add the minimal Cargo workspace and `cache-core` crate with pinned dependency versions.
- [x] Implement version/capability negotiation, status codes, opaque lifecycle, and explicit buffer ownership without functional cache behavior.
- [x] Contain panic and validate null pointer/length/duplicate-close/concurrent-close cases.
- [x] Add Rust unit tests, ABI/header consistency test, formatting, clippy, and a focused memory-safety check.
- [x] Add a deterministic Rust build script without changing default Go builds.

Exit: the static library can be created, queried for ABI capabilities, and destroyed safely; all negative lifecycle tests pass.

## Slice 2 — Concurrent cache semantics

- [x] Implement bounded Moka/`Bytes` storage that treats the exact Go-generated MosDNS key bytes as opaque canonical keys.
- [x] Implement lookup/store/flush/len and positive, negative, ECS, exclusion, expiration, and lazy hit classifications.
- [x] Keep lazy refresh execution in Go and expose only the minimum metadata needed across ABI.
- [x] Add concurrency and eviction tests proving there is no linear L1 scan or single global cache mutex.
- [x] Run the Go/Rust parity harness for semantic cases before raw fast paths, including the real Linux+cgo bridge on `mos-test`.

Exit: non-dump cache semantics match all frozen fixtures and Rust reports its true size.

## Slice 3 — Raw DNS response path

- [x] Adapt the audited KixDNS DNS wire walker/TTL patching; retain canonical ECS key generation and truncation in the Go facade to avoid duplicating MosDNS contracts.
- [x] Preserve current UDP/TCP/HTTP behavior, audit fields, and query context ownership.
- [x] Add malformed wire, compression, EDNS, truncation, TTL boundary, NXDOMAIN, SERVFAIL, and empty-answer tests.
- [x] Verify TTL work happens inside the single lookup ABI call and each returned response has independent Go-owned storage.

Exit: raw paths match Go fixtures byte-for-byte where required and semantically where transaction ID/TTL are expected to vary.

## Slice 4 — Persistence, API, metrics, and fallback

- [x] Make `mosdns_cache_v2` file replacement and import validation transactional, then mirror validated entries through the Rust store ABI; exercise the real Go→Rust path on Linux+cgo.
- [x] Connect existing show/load/flush API handlers through the mirrored Go facade without schema changes.
- [x] Connect the existing counters/gauge to the active backend and retain bounded structured backend/fallback logs without adding metric cardinality.
- [x] Implement startup fallback and the runtime one-way circuit breaker, with tests for status/import failure and Rust panic containment at the ABI boundary.
- [x] Confirm malformed imports do not partially mutate either backend and Rust never writes the persisted dump directly.

Exit: dump, API, metrics, close/reload, and fallback acceptance criteria pass against the actual bridge.

## Slice 5 — Build, CI, and performance evidence

- [x] Add Linux+cgo experimental CI alongside current Go/UI/release checks.
- [x] Run Rust fmt/test/clippy, Go full/focused/race tests, FFI integration, and available sanitizer/Miri checks. (All available stable checks pass; Miri passes under the Tree Borrows model; the Stacked Borrows finding is a known crossbeam-epoch/moka dependency limitation, recorded in the benchmark evidence.)
- [x] Run reproducible Go/Rust benchmarks and record QPS, p50/p95/p99, CPU, RSS, and allocations.
- [x] Keep Rust experimental after hot-path work reduced the preliminary median regression from about 85.7% to a noisy 39% range; p99/CPU/RSS replay measurements remain required.
- [x] Verify the non-Rust release path still builds fresh Vue assets and does not require Rust, using an isolated source copy to preserve the dirty worktree.

Exit: all automated gates pass and evidence supports a test-host rollout; no default has changed.

## Slice 6 — Test-host verification

- [x] Build the experimental artifact through the repository build path, including fresh Vue assets and the locked Rust static library.
- [x] Deploy only to `mos-test` and verify startup/ABI logs, DNS parity, metrics, API, dump restart, fallback injection, and sustained concurrency.
- [x] Record exact isolated artifact, config override, commands, smoke results, and rollback procedure.
- [x] Restore/leave the test host in an explicitly documented state; no service, port-53 listener, installed binary, or `/cus` config was changed.

Exit: test-host evidence is complete. Production deployment or making Rust default requires a new explicit approval/task.

## Required verification command families

```text
go test <focused cache/core packages>
go test -race <concurrency-sensitive packages>
go test ./...
cargo fmt --manifest-path rust/Cargo.toml --check
cargo test --manifest-path rust/Cargo.toml --all-targets
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
scripts/build-local.sh                    # default Go path, with fresh UI assets
scripts/build-rust-cache.sh               # experimental path, exact arguments documented
```

Commands may be refined after Slice 0 discovers the exact package/test boundaries, but gates may not be weakened silently.
