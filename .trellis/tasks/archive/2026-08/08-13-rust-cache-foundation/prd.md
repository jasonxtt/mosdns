# Rust cache foundation

## Goal

Deliver the first reviewable Rust migration slice: freeze the current Go cache contract, introduce a memory-safe and opt-in Rust cache core/bridge, and prove that it can run beside the existing Go implementation without changing default behavior.

## Background and confirmed decisions

- The current `rust` branch starts from MosDNS-T v0.7.1 and keeps Go as the control plane and default data plane.
- `/Users/tom/github/mosdns-rust-cache` contains a useful end-to-end prototype, but its global lock, linear L1 scan, unsafe buffer release, panic exposure, stale metrics, and incomplete tests prohibit wholesale import.
- KixDNS commit `2da3a2d` is the preferred Rust design/source reference. Reuse is module-by-module with attribution and parity review; its JSON pipeline and application runtime are not adopted.
- The first slice is cache only. WebUI, matcher, sequence, upstream, and server rewrites are outside this task.

## Requirements

### R1 — Freeze the compatibility contract

Record executable fixtures/tests for the current cache key, positive and negative answers, TTL aging, ECS, `exclude_ip`, lazy cache behavior, `domain_set`, dump version `mosdns_cache_v2`, show/load/flush API behavior, metrics, close/reload, and raw response paths.

### R2 — Establish a minimal Rust cache core

Create the smallest Rust workspace/crate structure needed for cache. Use a concurrent cache design based on established crates such as Moka and immutable byte storage; do not reproduce the prototype's global `Mutex` or O(n) L1 lookup.

### R3 — Define a versioned, memory-safe ABI

Provide ABI version/capability negotiation, opaque handles, fixed-width values, explicit buffer ownership/release, stable status codes, and panic containment. Invalid pointers/lengths, duplicate or concurrent close, and poisoned/internal failures must have tested behavior.

### R4 — Preserve the Go plugin contract

Keep `cache` plugin type, YAML arguments, lifecycle, HTTP endpoints, Prometheus names/labels, dump files, and default Go backend unchanged. Rust is selected only by an explicit experimental build/runtime mechanism and must be reported in logs/metrics.

### R5 — Integrate audited reuse

For every KixDNS-derived component, record whether it is a direct dependency, extracted/adapted code, design-only reference, or rejected. Pin the reviewed upstream commit and preserve copyright/license attribution. Prefer upstream crates over copying thin wrappers.

### R6 — Provide deterministic fallback

Startup ABI/capability failure must leave the Go implementation available. Runtime Rust failures must follow a documented policy distinguishing request-local fallback from disabling the Rust backend; no failure may silently corrupt or discard an existing dump.

### R7 — Make safety and parity enforceable

Add repeatable Go, Rust, FFI, race, dump round-trip, and malformed-input tests. Add Rust checks to CI/build flow without replacing the existing Vue prebuild, Go build, update manifest, or non-Rust release artifacts.

### R8 — Measure before defaulting

Provide reproducible Go-versus-Rust benchmarks for throughput, p50/p95/p99, CPU, RSS, and allocations under the same workload. This task does not make Rust the production default.

## Acceptance criteria

- [ ] A checked-in compatibility matrix maps every R1 behavior to a fixture/test and its Go/Rust result.
- [ ] The default build and configuration continue to use the Go backend and pass existing tests without requiring a Rust toolchain.
- [ ] An explicit experimental build produces a binary that negotiates the Rust ABI and can exercise lookup, store, flush, length/metrics, dump export/import, and close.
- [ ] Rust tests cover TTL, negative answers, ECS key isolation, exclusion, lazy behavior, dump round-trip, malformed wire/dump input, concurrency, panic conversion, and handle/buffer lifecycle.
- [ ] Go parity tests run the same cases against Go and Rust backends and report structured differences.
- [ ] `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, focused Go tests, `go test ./...`, and relevant race/FFI checks pass.
- [ ] No FFI path relies on `Vec` length equaling capacity, and no `unwrap`/panic can cross the ABI boundary.
- [ ] Rust cache lookup is not implemented as an O(n) scan and the whole cache is not serialized by one global mutex.
- [ ] Existing cache API paths, metric names/labels, `mosdns_cache_v2` data, YAML arguments, and Go fallback behavior remain compatible.
- [ ] CI builds Rust experimental artifacts in addition to—never in place of—the existing release workflow, including freshly built Vue assets.
- [ ] Benchmark results are checked in with environment and commands; any p99, CPU, or RSS regression over 10% keeps Rust experimental and records the blocking result.
- [ ] The experimental binary passes smoke/parity verification on `mos-test` before any production deployment is proposed.

## Out of scope

- Making Rust cache the normal or production default.
- Removing the Go cache implementation or fallback.
- Rewriting matchers, sequence execution, upstream transports, DNS servers, `coremain`, HTTP API, or Vue UIs.
- Adopting KixDNS JSON configuration, application entrypoint, or pipeline semantics.
- Changing config schema/package IDs, cache API schemas, metric names, audit fields, or dump version.

## Open questions

None required to begin after this plan is approved. Backend selection and fallback details are fixed in `design.md`; any later compatibility conflict must return to planning rather than being resolved silently during implementation.
