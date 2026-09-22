# Implementation plan — Rust Phase 5A native cache

Status: Slice 2 directed remediation complete; awaiting scoped re-review PASS.
Planning review: `PLANNING: PASS` at
`d49da845b694ec39ce9ecb09fd51447350f06b97`; see
`research/planning-review.md`. Executor still owns authorize/start/activate.

## Pre-start and review contract

- [ ] Read AGENTS, project-context, config-notes, rust-handover, rewrite plan,
  workflow and applicable specs; load trellis-before-dev before code edits.
  Read the applicable Phase 5A/native-host sections of the long
  `.trellis/spec/backend/rust-migration.md` directly; do not rely on truncated
  context injection.
- [x] Obtain explicit `PLANNING: PASS` from the designated reviewer against
  the exact pushed planning commit and all three planning artifacts.
- [x] Executor is current destination task `01a0c7fa-e0ea-7f52-adb0-f3789e7a7bdb`;
  reviewer is `01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3`. Configure executor-session
  automation with that reviewer and snapshot only Slice 0–3 before task start.
  Use existing host thread transport/evidence; do not invent provider names,
  bypass preflight or create substitute conversations. Then task.py start and
  automation activate in the executor's session. Keep auto-commit disabled.
- [ ] User's explicit “planning then tell the selected conversation to execute”
  authorizes this bounded handoff/run; no additional process approval is needed.
  User scope changes override generic skill defaults. Material changes still
  require review and, if they expand scope, user approval.
- [x] Record initial dirty paths; preserve `.trellis/workflow.md`,
  `.trellis/spec/backend/quality-guidelines.md`, `.trellis/workspace/tom/**`
  and existing `.DS_Store` changes. Never stage unrelated files.
- [x] Hash the entire archived Go baseline and `tests/phase5a-baseline/**`;
  retain a task-local manifest for final immutability comparison. W1 archive is
  read-only. No baseline runner or benchmark is authorized.

For each slice: narrow RED tests first, minimum GREEN implementation, focused
checks, inspect exact diff, explicit scoped commit/push, then one complete
review request. Include repository/branch, full pushed SHA and parent, changed
paths, evidence/commands, active slice and stop boundary. Await an explicit
PASS before advancing; on scoped FAIL fix only that slice, retest and resubmit.
Use the installed automation run/review commands to record progression. Do not
infer PASS from silence, queued messages or a successful push. Final PASS stops.

Common checks per code slice (choose affected packages in addition to the
listed slice checks):

```sh
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
python3 .trellis/scripts/task.py validate rust-phase5a-native-cache
git diff --check
```

Existing unrelated dirty lines causing a check failure must be reported,
not reformatted into this task. Record exact tests/counts and actual omissions.

## Slice 0 — safe owned cache-core API

Allowlist: `rust/cache-core/**`, `rust/runtime/**` only narrowly necessary bridge
adaptation/tests, and this task directory. No new dependency, host wiring,
Go source edit, listener/network or sequence-engine change.

- [x] RED: independent native cache instances, owned lookup/buffer isolation,
  overwrite/expiry, TTL copy-before-age, domain-set data and native/ABI parity
  at identical input times; retain ABI invalid-input/lifecycle tests.
- [x] GREEN: expose an owned safe cache object; share storage implementation
  with existing handle adapters. Native methods have no registry/raw ABI args.
- [x] Check bounded capacity after maintenance, no O(n) query scan and no new
  global serialization. Preserve public ABI layout/status/ownership semantics.
- [x] Run cache-core and runtime tests/clippy with `--all-targets --locked`
  (`clippy ... -- -D warnings`), common checks and dependency/diff inspection.
  Linux bridge integration is also mandatory in final Slice 3 before final PASS.
- [x] Commit/push and obtain `SLICE 0: PASS` before Slice 1.

Slice 0 local evidence before review:

- RED: the new public-API tests failed at compile time with unresolved
  `NativeCache`, before implementation.
- GREEN: `cargo test --manifest-path rust/cache-core/Cargo.toml --all-targets --locked`
  passed 11/11 tests.
- ABI regression: `cargo test --manifest-path rust/runtime/Cargo.toml
  --all-targets --locked` passed 2 unit, 19 ABI, 11 query-ABI, and 6 valued-ABI
  tests (38 total).
- Clippy passed for cache-core and runtime with `--all-targets --locked --
  -D warnings`; workspace fmt check, task context validation and `git diff
  --check` passed.
- `cargo tree --manifest-path rust/Cargo.toml -p mosdns-cache-core --edges
  normal --locked` shows the existing bytes/dashmap/moka closure; no dependency
  manifest or lockfile changed.
- Implementation paths are limited to `rust/cache-core/src/lib.rs`; task
  evidence is in `research/execution-state.md`. No runtime/ABI source was
  changed.

Reviewer remediation round 0 (`0d4a4355cc0447e6bef6b82e89b0245233c8f1c0`)
returned a scoped FAIL with P1-1, P2-1 and P2-2. The remediation keeps the
legacy ABI validation-before-handle-lookup precedence and adds the closed-handle
regression test, writes five distinct entries into the capacity-four native
cache test, and checks both sides of the message-expiry and cache-expiry
boundaries. The selected reviewer then returned `SLICE 0: PASS`; the slice
closed at pushed head `245404663f2039d1782fa3e611c9f0512f6fa9c4`.

## Slice 1 — native cache adapter and request completion

Allowlist: `rust/native-host/**`, `rust/Cargo.lock` path-edge changes only,
`rust/dns-core/**` only narrowly required response metadata helpers/tests, and
this task directory. No sequence-core production change; no accepted W2 config
or live cache listener required yet. No new external dependency/version/feature.

- [x] RED: deterministic clock/key/retention tests, exact expiry, no cumulative
  TTL mutation, case/qtype/AD/CD separation and non-IN bypass.
- [x] GREEN: host-owned native cache adapter and request-owned pending-store
  token integrated with the existing canonical sequence machine as designed.
  Test with an in-memory compiled W2 program/controlled exchange seam.
- [x] RED: hit skips forward; miss forwards and stores after completion only;
  upstream SERVFAIL versus local SERVFAIL; error, no response, malformed, TC,
  OPT, cancellation/deadline at publication, wrong/terminal machine paths.
  Add mismatched name/type/class, missing question and non-QUERY response
  cases: all must prevent storage while preserving W1 forwarding results.
- [x] GREEN: shared execution driver preserves W1 mapping, uses valid upstream
  provenance, drops uncommitted tokens, and adds no listener cache shortcut.
- [x] Tests cover negative, empty, zero-TTL and positive sub-5-second retention;
  authority/additional RR minimum TTL; compressed question key equivalence.
  Test ARCOUNT=1/EDNS bypass before lookup and store, including a pre-existing
  plain-query cache hit and an initially empty cache; W1 parsing is unchanged.
- [x] Run native-host, dns-core, cache-core and sequence-core tests with
  `--all-targets --locked`, clippy for changed crates, common checks and
  `cargo tree --manifest-path rust/Cargo.toml -p mosdns-native-host --edges normal --locked`.
  Verify no runtime/cgo edge and no native handle calls by source inspection.
- [x] Commit/push and obtain `SLICE 1: PASS` before Slice 2.

Slice 1 local evidence before review:

- RED: the adapter contract test failed before implementation with unresolved
  `CacheTestClock` and `NativeCacheAdapter` imports.
- GREEN: native-host all-targets locked passed 15 unit tests, 8 adapter tests,
  2 strict-config tests, 5 W1 TCP tests and 4 W1 UDP tests.
- The controlled execution seam proved a W2-shaped in-memory program skips the
  exact forward dispatch on a hit, publishes only after a successful miss,
  caches valid upstream SERVFAIL for five seconds, and does not cache local
  synthesized SERVFAIL.
- Adapter tests cover deterministic expiry, immutable TTL aging and buffer
  copies, qname/qtype/class/AD/CD key dimensions, non-IN and EDNS bypass,
  compressed query-name equivalence, NXDOMAIN/SERVFAIL/empty/zero-TTL
  retention (including long positive TTL versus the empty-answer cap), and
  TC/OPT/malformed/mismatched-name/type/class/missing-question/non-QUERY
  admission gates.
- Locked checks passed for native-host, cache-core, dns-core and
  sequence-core; native-host clippy with `--all-targets -- -D warnings`,
  workspace format check and `git diff --check` passed. The only lockfile
  change is the reviewed `mosdns-native-host -> mosdns-cache-core` path edge.
- Source inspection found no runtime/cgo/ABI-handle/cache-ABI references under
  `rust/native-host`; both listener modules call the shared execution driver.

## Slice 2 — strict W2 config and UDP correctness

Allowlist: `rust/native-host/**` and this task directory. Shared helper defects
outside this scope go back through a scoped reviewer decision, not quiet scope
expansion. Frozen baseline directories and sequence-core remain read-only.

- [x] RED: unchanged W1 UDP/TCP and W2 config fixtures; all PRD/design config
  rejection categories including wrong order/refs/counts/options/types/values.
- [x] GREEN: strict compiler accepts exactly the reviewed W2 graph and wires
  one cache per host; errors precede all listener/upstream I/O.
- [x] RED/GREEN: add `rust/native-host/tests/w2_cache.rs` using independent
  loopback upstream counters and frozen hot-case expectations. Cold lifecycle
  gives one upstream query per case; separate warm lifecycle verifies prefill,
  counter barrier and zero repeated-query upstream delta.
- [x] Test cold concurrency using barriers (no singleflight promise), warm
  concurrency/IDs/buffer isolation, expiry/reforward, non-IN bypass, negative
  and invalid/mismatched response handling, EDNS-query bypass with unchanged
  W1 forwarding, cancellation/no-publication, shutdown/rebind.
  Use deterministic clock injection for TTL assertions.
- [x] Run all native-host targets, cache-core/dns-core affected targets, changed
  crate clippy and common checks. Existing W1 UDP/TCP integration stays green.
- [x] Commit/push and obtain `SLICE 2: PASS` before remote/final Slice 3.

Slice 2 local evidence before review:

- RED: `cargo test -p mosdns-native-host --test slice2_config --locked` first
  failed only because the unchanged compiler rejected the four-plugin W2 YAML
  with the expected W1-only plugin-count diagnostic.
- GREEN: frozen W2 YAML now compiles only with integer `size: 64`,
  `lazy_cache_ttl: 0`, one UDP listener/upstream, and the exact `$cache` then
  `$forward` program. The config test covers declaration order, duplicate and
  unknown fields, wrong types/values, missing roles, counts, TCP, audit and
  wrong/repeated/missing references before assembly.
- UDP correctness: `cargo test -p mosdns-native-host --all-targets --locked`
  passed 15 unit, 8 Slice 1 adapter, 5 config, 5 W1 TCP, 4 W1 UDP and 6 W2
  tests. W2 loopback tests cover the two immutable `workloads/cache.jsonl`
  hot cases in separate cold and warm lifecycles, independent cold misses,
  concurrent warm hits, ID/buffer isolation, expiry/reforward, EDNS and
  non-IN bypass, upstream
  SERVFAIL caching, TC/OPT/malformed/mismatched responses, cancellation,
  shutdown and listener rebind.
- Affected crate checks passed: cache-core 11 tests; dns-core 55 unit plus 47
  integration tests; sequence-core 65 tests; native-host clippy with
  `--all-targets --locked -- -D warnings`; workspace format check and
  `git diff --check`. The native-host dependency tree remains path-only for
  the reviewed cache-core edge, with no runtime/cgo/ABI-handle source use.
- No Linux remote regression, baseline runner, benchmark, VM, deployment or
  production cutover was run in this slice.

Reviewer remediation round 0 (`5075d12adeab809bb34928a53af5503218375f35`)
returned a scoped FAIL with P1-1 and P2-1. The directed remediation adds
immutable corpus checks for both frozen W2 rows, independent cold and warm
lifecycles with a warm counter barrier, and an upstream-receipt signal before
the shutdown cancellation assertion. No implementation scope was expanded.
The selected reviewer re-reviewed exact head
`b558d153cad9ad8e3ffaf18a6e2dde82329e32e0` and returned `SLICE 2: PASS` /
`FINAL: PASS`.

## Slice 3 — Linux regression evidence and final review

Allowlist: this task directory, `docs/ai/rust-handover.md`, and
`docs/rust/feature-coverage.md` for accurate W2 status/evidence links only.
No new implementation in this slice. Product failure returns to its owning
slice and requires retest/re-review. Do not rewrite roadmap gates, archived
history or full-cache coverage status.

- [x] Create a fresh temporary checkout/artifact directory on `ssh mosdns-rust`
  for the exact reviewed source SHA; record Linux amd64/toolchains and commands.
  Run native-host W1 UDP/TCP/W2 and all-targets correctness tests with `--locked`.
- [x] Run Rust workspace fmt, tests and clippy against the reviewed source:
  `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked`
  and corresponding `cargo clippy ... -- -D warnings`.
- [x] Run `go test ./...` (build required UI assets serially if needed by the
  checkout's build contract; do not change generated tracked output). On Linux
  build the existing staticlib with `scripts/build-rust-cache.sh`, then run
  `go test -tags mosdns_rust ./plugin/executable/cache ./pkg/cache ./pkg/query_context ./pkg/server_handler`
  and the corresponding `-race` focused suite. Follow current `.github/workflows/test.yml`
  environment/linker settings rather than inventing a cgo invocation. If ABI
  internals changed, run/document applicable focused sanitizer/Miri or equivalent
  memory-safety validation; do not label ordinary Rust tests as such a check.
- [x] Hash comparison proves frozen baseline and corpus unchanged; record
  upstream count assertions, no late writes, shutdown/rebind and owned remote
  artifact cleanup. Record explicitly: no benchmark, VM or deployment ran.
- [x] Update handover/coverage with bounded W2 support and remaining Phase 5A/5B
  gates; full cache/lazy/EDNS/dump/API remain incomplete.
- [x] Inspect exact docs/evidence diff, validate task/common checks, commit/push
  and obtain `FINAL: PASS` from the selected reviewer for A1–A8.
- [x] Report tested SHA, final evidence SHA, reviewer result and remaining
  finish/archive lifecycle step, then stop. Do not finish/archive, create next
  task, run W3, performance tests or production work automatically.

Slice 3 remote and final evidence:

- The fresh artifact was `/tmp/mosdns-phase5a-slice3.qP7kFM/repo` on
  `mosdns-rust`, Linux amd64 (`Linux mosdns-rust 7.0.9-x64v3-xanmod1
  x86_64`). The exact tested source was `b558d153cad9ad8e3ffaf18a6e2dde82329e32e0`.
  Remote toolchains were `rustc/cargo 1.95.0`, `go1.26.4 linux/amd64`, and
  Python 3.13.5; `rustfmt` and `clippy` components were installed on the
  isolated host before their checks.
- Remote `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` passed.
  `cargo test -p mosdns-native-host --all-targets --locked` passed 15 unit,
  8 adapter, 5 config, 5 W1 TCP, 4 W1 UDP, and 6 W2 tests. The first full
  workspace test attempt hit a linker `SIGBUS` in the 2 GiB `/tmp` tmpfs; the
  scoped retry with `CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-target`
  and `CARGO_BUILD_JOBS=1` passed all workspace targets. The same root-disk
  target passed workspace clippy with `-D warnings`.
- Remote `go test ./...` passed. `CARGO_TARGET_DIR=... scripts/build-rust-cache.sh`
  built the existing `libmosdns_runtime.a`; after placing that artifact at the
  script's expected checkout path, both the normal and `-race` focused suites
  passed for `plugin/executable/cache`, `pkg/cache`, `pkg/query_context`, and
  `pkg/server_handler` with `-tags mosdns_rust`.
- The W2 loopback tests passed both frozen workload rows, cold upstream-count
  assertions, warm zero-delta assertions, expiry/reforward, ID and buffer
  isolation, EDNS/non-IN bypass, invalid/TC/OPT/mismatched no-publication,
  cancellation after upstream receipt, and shutdown/rebind. No late cache
  write was observed after cancellation. The frozen archive digest remained
  `538733b18dd516df21c27b998830c97ceae760c85702bda07391d2984a82634e`, and
  `tests/phase5a-baseline` remained
  `34678ca9acd6072ad2a01d429fd513d70e7dd48c899fbfc7cb7e4df160cc6b2d` in
  both local and remote exact-tree comparisons.
- The owned remote checkout and target directory were removed after
  validation. No benchmark, VM, deployment, production/default cutover, W3,
  or memory-safety sanitizer/Miri campaign was run. The final reviewer result
  is recorded separately as `FINAL: PASS`; only normal Trellis finish/archive
  remains, and it was intentionally not run here.
