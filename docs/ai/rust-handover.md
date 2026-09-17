# Rust migration handover

Last verified: `2026-09-16`

This is the canonical cross-session entrypoint for Rust migration work. It
records task state and worktree ownership; architecture remains authoritative
in `docs/ai/rust-rewrite-plan.md`, and executable requirements remain in each
Trellis task's `prd.md`, `design.md`, and `implement.md`.

As of `2026-08-18`, the migration target is explicitly a **pure Rust-native
MosDNS host**. The cache/matcher/query cgo bridges, `MOSDNS_*_BACKEND`
selectors, Go mirrors/fallback, paired generations, and FFI handle registries
already built in Phase 1/2/3A are transitional validation scaffolding. Preserve
them safely while they remain in-tree, but do not extend that hybrid pattern
into Phase 3B+ by default. Go is now a behavior-discovery reference; only
reviewed MosDNS product contracts are normative for new Rust modules.

## Resume protocol

A successor agent must:

1. Read `AGENTS.md`, `docs/ai/project-context.md`, and
   `docs/ai/config-notes.md`.
2. Read this file and `docs/ai/rust-rewrite-plan.md`.
3. Run `git branch --show-current`, `git status --short --branch`, and
   `python3 ./.trellis/scripts/task.py list`.
4. Read all three artifacts for the task being resumed.
5. Run the `trellis-before-dev` skill before changing implementation code.
6. Preserve unrelated dirty-worktree changes and keep Trellis auto-commit off.

This migration deliberately runs in the dedicated `/Users/tom/github/mosdns-rust`
worktree on branch `rust`. Do not switch back to `main` merely from a folder-name
convention. The verified branch/base is:

```text
branch: rust
base commit: 3896a4a7e0ce4311b40a7e4c80c93f2c8b3b4f1d
base release: v0.7.1
```

A-E Rust migration work commits are completed and reviewed (`A` governance,
`B` cache, `C` unified runtime, `D` matcher adapters, and `E` CI/build/evidence).
F is the task-archive finish commit and G is a journal-only finish commit; the
overall Rust rewrite remains active. The production/main line remains Go-only;
the `rust` branch now targets a future pure Rust-native replacement and is not
used as an intermediate production runtime.

## Task state

| Task | Trellis state | Implementation state | Successor action |
| --- | --- | --- | --- |
| `08-13-rust-cache-foundation` | **archived/completed** (`2026-08-13`) | All slices 0–6 complete; all three remaining gates (reproducible soak, Miri, extended mos-test verification) closed on `2026-08-13`. Rust stays experimental because the hybrid bridge missed the 10% QPS/latency gate. | Preserve the Rust cache core and its product-contract evidence. Do not spend Phase3B/4 optimizing the cgo bridge; that bridge/fallback is transitional and will be removed after the Rust-native host exists. |
| `08-13-rust-matcher-foundation` | **archived/completed** (`2026-08-13`) | Approved `2026-08-13`; Slices 0–5 complete. Compatibility matrix, Go golden fixtures, real rule-set fixture, KixDNS matcher ledger, single Rust runtime extraction, pure Rust domain/IP matchers (`matcher-core`), transactional FFI with matcher ABI in runtime, C header, Go provider integration (`domain_set`/`ip_set` with `MOSDNS_MATCHER_BACKEND=rust` env-gated Rust backend and Go fallback), Linux+cgo tagged integration/race, fixed-fixture evidence, CI gates, and isolated `mos-test` reload/fallback/restart smoke are verified. | Preserve the pure matcher core and the product-facing rule evidence; do not expand the FFI/provider fallback. Existing bridge artifacts remain historical scaffolding until the post-host retirement gate. |
| `08-13-rust-matcher-phase2-expansion` | **archived/completed** (`2026-08-14`) | Slices 0–5 are implemented, reviewed, and verified. Linux+cgo provider/mapper normal and race gates, fixed-fixture benchmarks, the full embedded-UI experimental binary, and isolated reload/fallback/restart smoke passed on `mos-test`. | Preserve the existing opt-in/fallback scaffolding without expanding it. Its pure matcher core and product-contract evidence remain useful; the Go/Rust bridge is scheduled for retirement only after the Rust-native host is complete. |
| `08-15-rust-phase3-query-execution-core` | **archived/completed** (`2026-08-17`) | Phase 3A query/wire foundation complete: Slices 0–4 root-reviewed, including Rust dns/query core, query ABI, Go opt-in adapter/fallback, Linux+cgo real-staticlib normal/race, and final wire/parity remediation. Overall Phase 3 remains incomplete because sequence control flow, matcher dispatch, no-network executable ownership, and query execution ownership are still Go-owned. | The next task is Phase 3B sequence/execution ownership. Reuse the pure Rust dns/query types, but do not extend the query ABI/Go fallback pattern into sequence-core. Keep the existing adapter untouched until the later retirement gate. |
| `08-17-rust-phase3b-sequence-execution-foundation` | **archived/completed** (`2026-08-18`) | Pure `rust/sequence-core` foundation complete: typed owned execution state, validated program model, explicit continuation stack, product-contract/deviation classification, fuel/cancellation, and no Go adapter/ABI/selector. Implementation commit `0c53c7d`; archive commit `a18fd89`. | Preserve the reviewed sequence contract. Do not reopen Phase3B while planning or implementing Phase4. |
| `08-17-rust-phase4-upstream-foundation` | **archived/completed** (`2026-09-16`) | Slices0–4 accepted; Slice4 `PASS / CLOSED` at `9d43e9f`, Actions `35090514316` success, closure record `cb15361`. Pure numeric UDP, fresh TCP and UDP TC→TCP foundation only. | Preserve reviewed contracts; archived artifacts are under `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/`. |
| `09-16-rust-phase4-secure-upstream-foundation` | **in_progress — Slices 0-3 CLOSED, Slice4 final quality gate active** (`2026-09-17`) | Slices 0-3 are implemented and formally accepted by `rust0916`: Slice1 `PASS` at `25c7c961453e15d7347d65bbc401026f813ff27c`, Slice3 `PASS / CLOSED` after one scoped remediation at `d3566bf105008e23c315536d6560b00d55250e55`. The bounded pure Rust DoT + DoH (HTTP/1.1 and HTTP/2) foundation exists in `rust/upstream-core` with explicit numeric dialing and independent service identity. | Slice4 is the active bounded scope: final quality and isolated Linux evidence. Local macOS evidence is recorded in the task `implement.md`; the Linux GitHub Actions result must still be observed for the revision under review. Do not wire host/production or start a later slice. |

Active migration work is the **secure-upstream foundation** on branch `rust`.
Slices 0-3 are CLOSED. The UDP/TCP foundation is archived after final root
acceptance; its completion is not completion of the whole Phase4 data plane. The
native host, resolver/bootstrap, connection pooling, listeners and hybrid
retirement are not implemented by this foundation.

`.trellis/tasks/09-16-rust-phase4-secure-upstream-foundation/` is the active
task; its Slice4 gate is in progress. The task's `implement.md` holds the exact
verification commands, results and their limitations. Production wiring,
deployment, and any later slice remain unauthorized.
The old foundation's exact commands and results remain in its archived
`implement.md`; no new runtime verification is claimed by administrative closure.

### Historical Phase4 CI/test-boundary remediation — 2026-09-16 (pre-Slice3 authorization)

- The docs-only `05275cd` push exposed a real but pre-existing Slice1 test
  race in `late_datagram_cannot_complete_a_later_exchange`; the unmodified
  test's equal 50 ms server/client sleeps did not establish cancellation before
  the late response.
- The approved test-only repair uses explicit `query-seen`, `allow-late`, and
  `late-sent` handshakes, asserts in-flight registration release, and leaves
  `rust/upstream-core/src/**` unchanged. The parent-worktree focused test
  passed 20 consecutive runs; upstream-core and dns-core regressions, fmt,
  warnings-denied clippy, task validation, and diff checks also pass.
- `.github/workflows/test.yml` now defines ordinary push/PR checks as the Go
  gate plus focused pure-Rust foundation fmt/test/clippy. The existing
  `rust-runtime-experimental` cgo/ABI/selector/fallback/embedded-runtime/smoke
  commands remain intact but are restricted to explicit `workflow_dispatch`.
  Node.js deprecation warnings are not part of this remediation.
- This remediation is preserved as historical scoped commit `9bf73f3`. Its
  statement that Slice3 remained unauthorized described the pre-authorization
  state at that time; the user subsequently authorized Slice3 on 2026-09-16,
  and its implementation is now formally `PASS / CLOSED` at same-thread root
  review of `21bff19212637c47df632a29dd4b3380cac7a4cc` with Actions run
  `35084388223` success. The remediation record itself is unchanged.

### Historical Phase4 Slice4 final quality gate — 2026-09-16

The following records the pre-archive gate. Lifecycle closure is recorded above.

- The user explicitly authorized Slice4 on 2026-09-16 as an evidence-only final
  quality gate. No implementation, production wiring, protocol, listener,
  pooling, retry, runtime-ownership, selector, or Go/cgo/ABI/fallback work was
  started. The external same-thread root review has now formally returned Slice4
  `PASS / CLOSED` at review commit
  `9d43e9fca09a24ad35399838c00299f7cc898301` (Actions run `35090514316`
  success: Ubuntu `rust-foundation` success, Go build success, historical
  `rust-runtime-experimental` skipped), so the Phase4 foundation final quality
  gate is CLOSED and the task remains `in_progress`.
- The exact verification commands and their results are recorded in
  `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/implement.md` under
  "Slice 4 execution and evidence record — 2026-09-16". All
  repository-executable Rust, Go, dependency, Trellis, and diff checks passed
  at revision `db374dfff5c1c286de86c646e098ed78dd6e3e8b`.
- Dependency inspection confirms `mosdns-upstream-core` depends only on
  `mosdns-dns-core`, `tokio`, and `tokio-util`, keeps
  `#![forbid(unsafe_code)]`, and has no `mosdns-sequence-core`/`mosdns-runtime`
  edge and no FFI/selector/pool boundary.
- The final compatibility/deviation matrix and the KixDNS transport ledger were
  re-inspected; no classification changed, and every research-unresolved and
  deferred row remains explicit.
- Local verification ran on macOS (Darwin 25.5.0 arm64) and must not be cited as
  Linux evidence. The repository's Ubuntu GitHub Actions `rust-foundation` job
  for review commit `9d43e9fca09a24ad35399838c00299f7cc898301` has completed
  successfully as Actions run `35090514316`, providing the Linux loopback
  network evidence.
- Current execution is STOP at the closed Phase4 foundation final quality gate;
  nothing here automatically authorizes Rust-native host wiring,
  production/default selection, hybrid retirement, release, or any
  deferred/research-unresolved scope.

## Implemented cache foundation

The current experimental path provides:

- `rust/cache-core`: Moka/Bytes cache, raw-wire TTL processing, versioned ABI,
  capability negotiation, panic containment, typed integer lifecycle handles,
  caller-owned lookup output, and ownership contract tests;
- an opt-in Linux+cgo Go bridge under build tag `mosdns_rust_cache` and runtime
  selection `MOSDNS_CACHE_BACKEND=rust`;
- unchanged default Go cache and deterministic startup/runtime fallback;
- positive/negative/lazy/ECS/exclusion/domain-set parity, raw UDP/TCP/HTTP
  response support, transactional `mosdns_cache_v2` import, API/metrics/close,
  concurrency, and malformed-input tests;
- experimental Linux CI and Vue-aware build entrypoints;
- audited KixDNS reuse pinned to commit `2da3a2d`.

Authoritative evidence:

- `docs/rust/cache-compatibility.md`
- `docs/rust/kixdns-reuse.md`
- `docs/rust/benchmarks/cache-foundation.md`
- `docs/rust/test-host-cache-foundation.md`
- `.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/implement.md` (archived)

Profiling reduced the preliminary Rust facade median gap from about 85.7% to a
noisy 39% range and allocations from 704 to 688 B/op. The remaining fixed cost
is mainly the temporary Go↔Rust request boundary. Do not hide it with a Go L1
or spend the next task on small cache tweaks; it disappears from the native
hot path only after Rust owns query/sequence execution.

## Cache gates closed on 2026-08-13

All three explicit cache gates are now addressed (evidence in
`docs/rust/benchmarks/cache-foundation.md` and
`docs/rust/test-host-cache-foundation.md`):

- **Reproducible replay/soak**: `TestCacheReplaySoak` (gated by
  `MOSDNS_CACHE_SOAK=1`) measures QPS, p50/p95/p99, process CPU, steady/peak
  RSS, allocations, and cgo calls. On `mos-test`, Go median QPS 4.27M vs Rust
  3.11M (−27%), p50 370 vs 622 ns, p99 1,760 vs 6,742 ns, both backends ~370%
  of 4 cores, Rust steady RSS lower (~80 vs ~90 MiB), allocations 7.91 vs 8.64
  allocs/op, and cgo calls exactly 1 per query on the Rust path.
- **Miri**: nightly `1.99.0-nightly` runs; all 16 `mosdns-cache-core` tests
  pass under `-Zmiri-tree-borrows -Zmiri-ignore-leaks`. The default Stacked
  Borrows model reports a known `crossbeam-epoch 0.9.20` (moka dependency)
  incompatibility, not a defect in the FFI boundary.
- **Extended `mos-test` verification**: isolated-process dump write → restart →
  restore (hit served from the restored Rust-backed entry), explicit runtime
  fault injection via `MOSDNS_CACHE_FAULT_INJECT=1` tripping the one-way
  circuit breaker to Go fallback with no panic, and 60 s sustained concurrency
  at ~95K QPS with 100% success, zero log errors, and plateauing RSS.

These gates prevent making cache/Rust the default: the QPS/latency regression
far exceeds the 10% rollout gate, so the Rust backend remains experimental.
They do not block the planned matcher/runtime foundation. Production
deployment, a port-53 service replacement, or default selection requires a
separate explicit approval.

## Approved architecture direction

The migration is a strangler sequence that ends in a pure Rust-native host; the
existing Go/Rust bridge is temporary:

```text
Phase 0-3A: behavior discovery + hybrid validation scaffolding
Phase 3B-4: pure Rust execution/transport/server foundations
Phase 5: complete Rust-native host and end-to-end cutover evidence
Phase 6: remove hybrid selectors/cgo adapters/Go mirrors/fallback and pass purity gate
```

The implementation sequence is:

```text
cache foundation
    -> domain/IP matcher and compiled rule indexes
    -> Rust DNS/query context and sequence control flow
    -> upstream transports and server data plane
    -> Rust-native host/control-plane replacement
```

The second module formed the single transitional Rust `staticlib` runtime and
retained existing cache ABI symbols while linking cache and matcher cores. That
was appropriate for Phase 1/2/3A validation. New Phase 3B+ modules should not
add ABI symbols merely to preserve the hybrid shape; they should compose as
Rust libraries for the future native host. Do not create a second independent
Rust runtime.

The key performance conclusion is architectural: while Go owns each request,
per-plugin cgo calls remain measurable. When Rust owns server → query context →
sequence → matcher/cache → upstream → response, this cross-language hot-path
cost disappears. Rust code still requires normal algorithm/allocation/locking
discipline; native ownership is not an automatic performance guarantee.

## Matcher foundation and continuation gate

The full next-task plan is located at:

- `.trellis/tasks/08-13-rust-matcher-foundation/prd.md`
- `.trellis/tasks/08-13-rust-matcher-foundation/design.md`
- `.trellis/tasks/08-13-rust-matcher-foundation/implement.md`

Slices 0–5 froze domain/IP behavior, extracted the single Rust runtime, added
the pure Rust indexes and transactional FFI, integrated only direct
`domain_set`, `ip_set`, `base_domain`, and `base_ip` adapters, and closed the
foundation CI/evidence/isolated-smoke gates. That archived task remains
Go-only and its Rust path remains opt-in; its continuation boundary was the
separate Phase 2 expansion task described below.

## Matcher Phase 2 expansion (completed)

The completed Trellis task was:

- `.trellis/tasks/08-13-rust-matcher-phase2-expansion/prd.md`
- `.trellis/tasks/08-13-rust-matcher-phase2-expansion/design.md`
- `.trellis/tasks/08-13-rust-matcher-phase2-expansion/implement.md`

Slices 0–4 preserve Go control-plane parsing and fallback while publishing
paired Rust/Go generations for `sd_set`, `si_set`, and valued `domain_mapper`.
Slice 5 artifacts are:

- CI default/focused/race, Rust ABI/header, Linux+cgo normal/race, benchmark,
  and experimental-binary gates in `.github/workflows/test.yml`;
- fixed-fixture provider and valued-mapper benchmarks in
  `scripts/benchmark-rust-matchers.sh` and
  `plugin/data_provider/domain_mapper/rust_benchmark_linux_test.go`;
- isolated reload/fallback/restart smoke in
  `scripts/smoke-rust-matcher-mos-test.sh`;
- `docs/rust/benchmarks/matcher-phase2-expansion.md` and
  `docs/rust/test-host-matcher-phase2-expansion.md`.

Linux+cgo and host smoke evidence was collected on `mos-test` using an
isolated `/tmp` source copy. The full embedded-UI experimental binary was
also built and used for the final smoke. The task was archived after the root
review and selective work commit; at that time it did not authorize Phase 3 or
a Rust default-backend switch. Phase 3 authorization is now recorded by the
active task row above; the default backend remains Go-only.

## Worktree ownership boundary

The following ownership list is historical migration context. Inspect current
`git status` instead of assuming these paths are dirty. Do not stage all files
or infer ownership from modification time.

Rust migration / Trellis scope:

Project-level governance files are in scope as exact paths or the explicitly
bounded patterns below. The active task's commit manifest assigns each
current path to one ordered commit; these entries are not permission to stage
a whole directory blindly:

```text
.gitattributes
.gitignore
AGENTS.md
.agents/skills/trellis-*/** (project Trellis skill source)
.codex/agents/trellis-check.toml
.codex/agents/trellis-implement.toml
.codex/agents/trellis-research.toml
.codex/config.toml
.codex/hooks.json
.codex/hooks/inject-subagent-context.py
.codex/hooks/inject-workflow-state.py
.codex/hooks/session-start.py
.trellis/.gitignore
.trellis/.template-hashes.json
.trellis/.version
.trellis/agents/
.trellis/config.yaml
.trellis/scripts/**/*.py (excluding __pycache__ and *.pyc)
.trellis/spec/backend/
.trellis/spec/guides/
.trellis/tasks/archive/2026-08/00-bootstrap-guidelines/
.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/
.trellis/tasks/08-13-rust-matcher-foundation/
.trellis/workflow.md
.trellis/workspace/index.md
.trellis/workspace/tom/index.md
.trellis/workspace/tom/journal-1.md
```

After the A–E work commits, the finish sequence has two separate exact-scope
commits: F, the archive finish commit, moves
`.trellis/tasks/08-13-rust-matcher-foundation/` to
`.trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation/` and records
the task lifecycle transition; G updates only
`.trellis/workspace/tom/index.md` and
`.trellis/workspace/tom/journal-1.md`. Both operations use explicit
`--no-commit` flags before manual path review. The journal records A–E hashes,
not F; G is journal-only.

The real `scripts/build-rust-cache.sh` plus tagged cgo matcher gate is
Linux-only (CI or isolated `mos-test`); macOS cargo success is not cgo evidence.

The workspace index and journal are Trellis cross-session memory and are
governance inputs, not local state to discard. The bootstrap task, referenced
backend specs, thinking guides, archived cache history, and current matcher
artifacts were checked as project-generated text without real credentials or
local databases. `.codex` is intentionally not a blanket inclusion: only the
eight project agent/config/hook files listed above are in scope.

Rust implementation and evidence scope:

```text
.github/workflows/test.yml
docs/ai/rust-rewrite-plan.md
docs/ai/rust-handover.md
docs/rust/
rust/
scripts/build-rust-cache.sh
scripts/build-rust-experimental.sh
scripts/build-local.sh (GO_TAGS support only)
plugin/executable/cache/
plugin/data_provider/domain_set/domain_set.go (matcher task Slice 4/5 changes)
plugin/data_provider/domain_set/golden_test.go
plugin/data_provider/domain_set/rust_backend.go
plugin/data_provider/domain_set/rust_benchmark_linux_test.go
plugin/data_provider/domain_set/rust_integration_linux_test.go
plugin/data_provider/domain_set/rust_stub.go
plugin/data_provider/domain_set/testdata/real-rule-set.txt
plugin/data_provider/ip_set/ip_set.go (matcher task Slice 4/5 changes)
plugin/data_provider/ip_set/golden_test.go
plugin/data_provider/ip_set/rust_backend.go
plugin/data_provider/ip_set/rust_benchmark_linux_test.go
plugin/data_provider/ip_set/rust_integration_linux_test.go
plugin/data_provider/ip_set/rust_stub.go
plugin/matcher/base_domain/domain_matcher.go (matcher task Slice 4 change)
plugin/matcher/base_domain/rust_integration_linux_test.go
plugin/matcher/base_ip/ip_matcher.go (matcher task Slice 4 change)
plugin/matcher/base_ip/rust_integration_linux_test.go
pkg/matcher/domain/golden_test.go
pkg/matcher/netlist/golden_test.go
pkg/query_context/context.go and context_raw_test.go
pkg/server_handler/entry_handler.go and entry_handler_raw_test.go
coremain/audit_raw_test.go
scripts/benchmark-rust-matchers.sh
scripts/smoke-rust-matcher-mos-test.sh
plugin/data_provider/domain_mapper/slice5_benchmark_fixture_test.go
plugin/data_provider/domain_mapper/rust_benchmark_linux_test.go
plugin/data_provider/matcher_adapter/adapter_linux.go
docs/rust/benchmarks/matcher-phase2-expansion.md
docs/rust/test-host-matcher-phase2-expansion.md
```

Known unrelated/user-owned changes that must not be overwritten, reverted, or
automatically included in a Rust commit:

```text
coremain/api_system.go
coremain/config_manager.go
coremain/config_manager_api_test.go
coremain/mosdns.go
coremain/openwrt.go
coremain/openwrt_test.go
webui-log/src/components/SystemControlManager.vue
webui-log/src/components/system/SystemConfigManagePanel.vue
coremain/www/assets/vue-log/
coremain/www/assets/vue-log1/
coremain/www/log.html
coremain/www/log1.html
```

The embedded Vue assets may have been regenerated while verifying a default
build and therefore contain the unrelated WebUI changes. Treat them as
user-owned unless a later explicit release task requires a fresh asset commit.
`.DS_Store` and `.superpowers/` are local/tool state. The listed `.codex` files
are project integration and are included; no other
`.codex` path is implicitly included. `.trellis/.developer`,
`.trellis/.runtime/`, Python `__pycache__/` and `*.pyc`, personal credentials,
and local databases remain excluded.

Before staging any future Rust commit, enumerate exact files and review every
overlap. Never use `git add -A` in this worktree.

## Last verified checks

Re-verified on `2026-08-13` after the soak harness and fault-injection toggle:

```text
go test ./...                                        # PASS (local)
go vet ./...                                         # PASS (local)
go test -race ./plugin/executable/cache              # PASS (local, stub path)
cargo fmt --manifest-path rust/Cargo.toml --check    # PASS
cargo test --manifest-path rust/Cargo.toml --all-targets   # PASS
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings  # PASS
rustup +nightly cargo miri test --all-targets \
  with MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"    # PASS (16 tests)
```

On `mos-test` (`10.0.0.91`, Linux x86_64), the real Linux+cgo tagged package
passed the full suite and `-race`; `TestCacheReplaySoak` produced the
reproducible soak evidence; and the isolated-process extended verification
(dump restart, runtime fault injection, sustained concurrency) completed. The
`MOSDNS_CACHE_FAULT_INJECT=1` toggle is experimental-only and disabled by
default. The isolated smoke and verification did not modify systemd, `/cus`,
the installed binary, or port 53. Re-run checks appropriate to the files
changed; do not treat this historical result as a substitute for a new
verification run.

Matcher foundation evidence also closed on `2026-08-13`: the tagged Linux+cgo
matcher suites and race suites passed on `mos-test`; the fixed-fixture build,
lookup, index-entry, allocation, and cgo measurements are in
`docs/rust/benchmarks/matcher-foundation.md`; and
`scripts/smoke-rust-matcher-mos-test.sh` passed valid/malformed reload,
concurrent query/reload, Go-only fallback, and restart using only temporary
files and random high loopback ports. The smoke process and temporary
directory were cleaned up, with no port-53 or production-host change. Exact
RSS for an individual Rust snapshot remains unmeasured at the current ABI
boundary.

Phase 2 expansion evidence was rerun on `2026-08-14` in an isolated source
copy on the same host. The expanded provider/mapper normal and race suites,
Rust fmt/test/clippy/header checks, fixed-fixture benchmarks, full embedded-UI
Linux amd64 experimental binary build, and mapper-aware
reload/fallback/restart smoke all passed. Measurements and exact commands are
recorded in `docs/rust/benchmarks/matcher-phase2-expansion.md` and
`docs/rust/test-host-matcher-phase2-expansion.md`. Rust remains opt-in and
default builds remain Go-only.

## Non-negotiable constraints

- The current Go/main release remains unchanged until the complete Rust-native
  replacement passes its final gates; the incomplete `rust` branch is not used
  as a production runtime.
- Preserve the MosDNS product contract: YAML/config and sequence/plugin
  semantics, final DNS/routing/audit behavior, API/WebUI workflows, metrics and
  persistent formats. Go internal structures and accidental quirks are not
  automatically part of that contract.
- Existing Phase 1/2/3A hybrid adapters/fallback remain safe but frozen; do not
  expand them into Phase 3B+. Remove them only in the post-host retirement gate.
- KixDNS `2da3a2d` is a selective GPL-3.0 source/design reference, not a subtree
  merge or JSON-pipeline replacement.
- `/Users/tom/github/mosdns-rust-cache` is a read-only prototype reference, not
  a drop-in implementation.
- Use one coarse Rust boundary and, after the query-core phase, keep the request
  hot path natively in Rust.
- Validate locally, then on isolated `mos-test`; never promote to production
  without explicit user approval.
