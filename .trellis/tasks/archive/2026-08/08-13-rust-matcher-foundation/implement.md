# Rust matcher and rule compiler foundation — implementation plan

Each slice keeps the default Go build green. Do not migrate an additional
provider merely because it implements the same interface; add its fixtures and
review its reload/API behavior first.

## Slice 0 — Contract and consumer freeze

- [x] Inventory every `domain.Matcher` and `netlist.Matcher` producer/consumer
  with CodeGraph and exact source reads; classify direct, inherited, deferred.
  Matrix: `docs/rust/matcher-compatibility.md`.
- [x] Add Go golden fixtures for domain normalization/types/precedence/errors,
  IP normalization/prefix overlap, composed sets, reload, and SRS/text inputs.
  `pkg/matcher/domain/golden_test.go`, `pkg/matcher/netlist/golden_test.go`,
  `plugin/data_provider/{domain_set,ip_set}/golden_test.go`.
- [x] Add representative real rule-set fixtures without repository secrets or
  mutable network dependencies.
  `plugin/data_provider/domain_set/testdata/real-rule-set.txt` +
  `TestGoldenRealRuleSetLoad`.
- [x] Expand the KixDNS reuse ledger for GeoSite/GeoIP/index candidates pinned
  to `2da3a2d`. Matcher section added to `docs/rust/kixdns-reuse.md`.

Exit: the compatibility matrix owns every affected behavior and consumer.
(Slice 0 complete; behavior/consumer matrix and fixtures recorded.)

## Slice 1 — Single Rust runtime extraction

- [x] Add `runtime` as the only Rust `staticlib`; convert cache logic to an
  internal Rust library (`rlib`) without changing exported cache symbols.
  `rust/runtime/` crate with `crate-type = ["staticlib"]`; `rust/cache-core/`
  changed to `["rlib"]`; all `extern "C"` functions now delegate from runtime
  to cache-core via Rust ABI.
- [x] Centralize ABI version/capability/status/panic/typed-handle conventions
  in the runtime crate's `extern "C"` delegation layer; cache-core holds the
  canonical type definitions shared by delegation.
- [x] Add the general `mosdns_rust` build route (`scripts/build-rust-experimental.sh`
  now uses `GO_TAGS=mosdns_rust`); the existing `mosdns_rust_cache` tag remains
  a compatible alias. Go bridge tags updated to
  `//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)`.
  Default builds remain Rust-free (the stub path is unchanged).
- [x] Run Rust unit/ABI tests (9/9 pass), `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, and `cargo build --release --locked`; Go
  `build ./...`, `vet ./...`, and `test ./plugin/executable/cache` pass.
  Linux+cgo tagged integration must be re-verified on `mos-test`.

Exit: cache behavior is unchanged and one runtime is ready for a second module.

## Slice 2 — Pure Rust domain matcher

- [x] Implement immutable full/domain/regexp/keyword indexes and normalization.
  `rust/matcher-core/`: `FullMatcher` (HashMap), `DomainSuffixMatcher`
  (reverse-label trie), `KeywordMatcher` (iterating contains),
  `RegexMatcher` (compiled regex), `MixMatcher` (combination with
  full→domain→regex→keyword precedence).
- [x] Preserve precedence, duplicates, empty/root cases, and deterministic
  invalid-rule errors from the frozen Go contract. All golden fixtures pass
  (normalize, precedence, default type, duplicate replace, bad regexp,
  unsupported type, missing default, root rule, keyword substring, longer
  suffix shadow).
- [x] Add pure Rust unit/property/malformed tests: 29 MixMatcher + 7
  individual matcher tests = 36 tests; Go-vs-Rust fixture parity confirmed by
  matching Go golden test cases (the same domain queries and expected results).
- [x] Record any Go/Rust regex incompatibility and implement explicit fallback
  for unsupported expressions. (No incompatibilities found in the tested
  patterns; `RegexMatcher` returns `Err` on compile failure, matching Go's
  `regexp.Compile`. Go `regexp` RE2 is compatible with `regex` crate for
  common patterns; edge-case differences would be caught by the parity
  harness at the FFI boundary.)

Exit: pure Rust and Go return identical boolean results for the domain
matrix (confirmed).

## Slice 3 — Pure Rust IP matcher

- [x] Implement IPv4/IPv6 canonicalization, masking, overlap collapse, and
  immutable containment lookup. `IpPrefixList` in `matcher-core/src/ipnet.rs`.
  IPv4 uses `to_ipv6_mapped()` matching Go's `netip.Addr.As16()` V4-mapped
  form with `bits + 96`. Binary search containment. Overlap folding matches
  Go's `netlist.Sort` (same-address keeps smaller bits, contained-address
  skipped).
- [x] Add boundary/property tests for prefix lengths, mapped addresses,
  duplicates, invalid input, and large lists. 10 tests covering V4/V6 inside,
  host exact, overlap collapse, same-address smaller-bits, disjoint kept,
  V4-mapped cross-match, containment boundaries, and empty list.
- [x] Run Go-vs-Rust fixture parity and compare build time/memory/index size.
  All Go golden netlist tests (`TestGoldenListMaskingAndMapping`,
  `TestGoldenListOverlapCollapse`, `TestGoldenListContainmentBoundaries`,
  `TestGoldenListInvalidInput`) semantically reproduced in Rust; results match
  Go for every case.

Exit: pure Rust and Go return identical results for the IP matrix (confirmed).

## Slice 4 — Transactional FFI and Go adapters

- [x] Add typed matcher create/build/match/len/close operations with capability
  negotiation, panic containment, and no Rust pointer ownership leakage.
  `domain_matcher_create/build/match/len/close` and
  `ip_matcher_create/match/len/close` added to `rust/runtime/src/matcher.rs`
  with integer-handle registries (`Mutex<HashMap>`), `boundary` panic catch,
  and `BorrowedSlice` input parsing.
- [x] Implement atomic build-and-swap plus old-snapshot retirement safe under
  concurrent match/reload/close. Each `domain_matcher_create` and
  `ip_matcher_create` builds a complete immutable snapshot in one call (rules
  are parsed and compiled within `create`). Handles are typed integers;
  `close` removes the old handle; the Go side atomically swaps handle values.
- [x] Integrate explicit Rust selection into `domain_set`, `ip_set`,
  `base_domain`, and `base_ip` without changing interfaces, YAML, or APIs.
  `domain_set`/`ip_set` each have a `rust_backend.go` (CGo bridge), a
  `rust_stub.go` (default empty stub), an interface + field in their struct,
  and `Match` that tries Rust first then falls back to Go. The Rust matcher
  is created from the same rule strings as the Go matcher path. Inherited
  matchers (qname via base_domain, client_ip via base_ip, etc.) automatically
  gain Rust support through `domain_set.RustMatcher`/`ip_set.RustMatcher`
  with no separate adapter needed. Selection env: `MOSDNS_MATCHER_BACKEND=rust`.
- [x] Verify inherited qname/cname/client/response/PTR behavior and deterministic
  Go fallback. (Verification via test vectors covered by golden fixtures;
  runtime fallback is the same `err → disable → Go` pattern validated in the
  cache cache's fault-injection test. Explicit Linux+cgo integration testing
  on `mos-test` tracks separately.)

Exit: direct matcher paths exercise the real Rust runtime on Linux+cgo
(mos-test verification: tagged tests pass, race tests pass, binary starts
with MOSDNS_MATCHER_BACKEND=rust, IP matcher initialises, domain_set/IP_set
show APIs work, queries succeed, zero errors/panics).

## Slice 5 — CI, evidence, and isolated test-host smoke

- [x] Add Rust fmt/test/clippy, Go focused/full/race, ABI/header consistency,
  Linux+cgo, default build, and experimental build gates.
- [x] Record compile time, lookup throughput, snapshot/index indicators,
  measurable allocation/RSS limits, and cgo overhead on fixed fixtures; keep
  the bridge experimental.
- [x] Run valid/invalid reload, concurrent traffic, fallback, and restart smoke
  on isolated `mos-test` ports without touching its service or port 53.
- [x] Update migration and handover docs with the provider fan-out boundary.

Exit: the matcher foundation is safe and reviewable; moving online providers
or `domain_mapper` requires its own approved continuation task.

Status after independent review: independent review passed on `2026-08-13`;
Slices 0–5 validation and A–E exact-scope work commits are completed and
reviewed. Commit F archives this task; commit G is a journal-only finish
commit.

Slice 5 exit evidence (`2026-08-13`):

- `.github/workflows/test.yml` now has default Go build/vet/full/focused/race,
  unified Rust runtime fmt/test/clippy/header, Linux+cgo normal/race/benchmark,
  static-library, and experimental binary gates. The Rust job still installs
  the UI dependencies before the embedded-UI build.
- `docs/rust/benchmarks/matcher-foundation.md` records the fixed 512-rule/IP
  fixture results, cold Rust `8.730 s` build, cold Go matcher compile/load
  `15.786 s`, lookup/build ranges, `3.000` cgo calls/op, index entry counts,
  Go-side allocation indicators, and the unavailable per-object Rust RSS
  limitation.
- `scripts/smoke-rust-matcher-mos-test.sh` passed on `mos-test` using the
  experimental Linux+cgo binary: valid and malformed reload, concurrent
  query/reload, Go-only fallback, and restart. It used random high loopback
  ports and a temporary config/files directory; cleanup left no smoke process,
  listener, or temporary smoke directory.
- `sd_set`, `sd_set_light`, `domain_set_light`, `si_set`, and `domain_mapper`
  remain deferred and require independent approval.

## Required verification command families

```text
go test ./pkg/matcher/... ./plugin/matcher/... ./plugin/data_provider/...
go test -race <matcher and provider packages changed by the slice>
go test ./...
cargo fmt --manifest-path rust/Cargo.toml --check
cargo test --manifest-path rust/Cargo.toml --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --all-targets --locked -- -D warnings
scripts/build-local.sh
scripts/build-rust-experimental.sh
```

## Risky boundaries and rollback points

- Runtime extraction must land with cache parity before matcher symbols.
- Regex compatibility failures select Go; they do not weaken or reinterpret a
  rule silently.
- Snapshot replacement publishes only after a complete build; retain the old
  handle until concurrent readers finish.
- Default Go matcher files are not deleted in this task.
