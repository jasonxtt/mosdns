# Rust Phase 5A native cache

Status: planning; implementation waits for explicit planning PASS.
Source anchor: `e4dcc71398c2412a4d42a5be6bad3ddcca1f0bc0` on `rust`.

## Goal and value

Make the existing Rust-native host execute the frozen W2 cache workload through
`cache -> forward`, with correct cache-hit and cache-miss behavior. This is the
next bounded integration milestone after archived W1 UDP/TCP forwarding. It
establishes a real native cache path before broader Phase 5B feature coverage.
Correctness and stability are prerequisites; latency, useful throughput and
concurrency are primary eventual performance goals, with memory secondary.
This task produces correctness evidence, not performance or release approval.

## Confirmed scope and authorization

The user requested planning followed by execution in **处理执行者001任务**
(`codex://threads/01a0c7fa-e0ea-7f52-adb0-f3789e7a7bdb`), with
**成为001号 reviewer**
(`codex://threads/01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3`) as reviewer.
This is authorization to hand off the previously agreed W2 milestone and its
four bounded slices after planning review. Do not ask again merely to select
these conversations or begin that reviewed scope. Material scope changes still
require the user. Planning and every slice require explicit reviewer PASS.
Final PASS stops before finish/archive, a new task or deployment.

## Requirements

1. Accept `tests/phase5a-baseline/configs/cache.yaml` unchanged: one UDP listener,
   one UDP numeric upstream, one root sequence containing `$cache` then
   `$forward`, cache `size: 64`, `lazy_cache_ttl: 0`, audit disabled. Retain both
   accepted W1 configurations and all prior strict rejection behavior. Reject
   unsupported cache/config combinations before resource creation or I/O.
2. Reuse cache-core through a safe, owned Rust API. Native requests must not use
   C ABI wrappers, integer handles, the global handle registry, Go bridges,
   environment backend selectors, or Go fallback. Existing bridge behavior and
   ABI remain compatible for transitional users.
3. Cache lookup is a sequence executable. A hit skips forward. A miss resumes
   the same canonical sequence machine and stores only after successful
   completion with a validated upstream response. No listener shortcut or
   separate sequence interpreter. Request failure/cancellation never publishes
   a cache entry; a legitimate upstream SERVFAIL is distinct from a locally
   synthesized failure.
4. The initial cacheable query subset is existing native-host supported standard
   single-question, no-additional-record IN queries. Non-IN queries keep W1
   forwarding behavior but bypass cache. Preserve qname case distinctions and
   isolate qtype and AD/CD bits; transaction ID is not part of the key. EDNS
   queries remain rejected as in W1. Responses containing OPT bypass caching
   intact in this milestone; full EDNS handling is deferred, not approximated.
5. Cache responses are immutable owned snapshots. Hits use request-private
   response bytes, current request ID, and TTL aging without cumulative mutation.
   Entries expire when age reaches their retention time; lazy serving/refresh is
   disabled. Capacity remains bounded under the existing cache policy.
6. For cacheable non-truncated responses, use the existing documented retention
   policy: NXDOMAIN 30 s, upstream SERVFAIL 5 s; NOERROR uses minimum ordinary RR
   TTL, capped at 300 s for empty answers; a computed zero/no-record retention
   and other RCODEs use 5 s. Positive TTLs below 5 remain unchanged. Retention is
   distinct from on-wire RR TTLs. TC, malformed/unvalidated, missing and local
   synthetic responses never enter cache. Preserve the existing cache-core
   TTL-aging behavior; do not silently rewrite the ABI contract.
7. Parallel cold misses remain independent; this milestone does not add
   singleflight. Warm concurrent hits isolate IDs, qnames and buffers. Request
   cancellation, shutdown and rebind retain W1 ownership/lifecycle guarantees.
8. Preserve all frozen Go baseline artifacts, inputs and provenance. Linux
   correctness validation may use only the previously authorized
   `ssh mosdns-rust` test host in a fresh temporary directory. No baseline
   runner, performance campaign, local VM, production service or deployment.

## Acceptance criteria

- [ ] A1: Frozen W1 UDP, W1 TCP and W2 YAML compile unchanged; negative config
  matrix rejects duplicate/unknown/missing fields, unsupported values, cache
  options, plugin counts, refs and order before I/O.
- [ ] A2: Native API isolation and bridge regression tests pass; native-host's
  call graph uses no handle/ABI API. No new external dependencies or features.
- [ ] A3: Each frozen W2 hot case, in a fresh cold lifecycle, adds exactly one
  controlled-upstream query. A separate warm lifecycle performs exact prefill,
  captures a counter barrier, then repeated warm queries add zero.
- [ ] A4: Deterministic clock tests cover TTL aging, repeated-hit immutability,
  exact expiry, negative/empty/zero-TTL retention, and post-expiry forwarding;
  ID, qtype, AD/CD, qname case, cache-instance isolation and non-IN bypass pass.
- [ ] A5: Controlled cold concurrency asserts all forced pre-publication misses
  complete correctly without imposing singleflight; concurrent warm hits have
  zero upstream delta and no cross-request mutations.
- [ ] A6: Upstream timeout/error, malformed response, TC, OPT, no response,
  cancellation before publication and shutdown prevent cache publication as
  specified. A valid upstream SERVFAIL follows its separate 5 s retention.
- [ ] A7: W1 UDP/TCP and W2 integration tests pass on Linux amd64 with exact
  source identity, command/output and cleanup evidence. Rust checks and Go
  bridge regression checks pass. Historical corpus/evidence hashes unchanged.
- [ ] A8: Reviewer explicitly returns final PASS; handover/coverage describe W2
  as a bounded correctness milestone, not full cache migration, a performance
  improvement, Phase 5A completion or production readiness.

## Exclusions and deferred work

No lazy refresh, ECS, exclusion lists, dump/import/API, metrics/WebUI migration,
W3 routing, new transport, cache-enabled TCP configuration, broad DNS parser or
runtime redesign, thread-model changes, dependency upgrades, singleflight,
performance comparison, cutover, scaffolding retirement, or production wiring.
Existing core/ABI behavior for those features must remain safe. Full cache
compatibility remains a Phase 5B gate. No blocking product decision remains
within this narrow scope; technical implementation naming is executor-owned.
