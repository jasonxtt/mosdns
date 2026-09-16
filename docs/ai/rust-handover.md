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
| `08-17-rust-phase4-upstream-foundation` | **in_progress — Slice1 UDP implemented, awaiting root review** (`2026-09-16`, planning PASS `16192ea`, Slice0 PASS `3108305`) | Phase3B prerequisite, the Phase4 planning package, and Slice0 are root-reviewed. The user explicitly authorized Slice1; the one-exchange/one-socket UDP primitive and deterministic loopback contract tests are implemented. No TCP, fallback, C ABI, Go ownership, selector, or production wiring was added. | Review Slice1 UDP evidence; do not start Slice2/TCP or later work until a new explicit root-review authorization. |

Active migration state is **Phase 4 Slice1 UDP implemented; awaiting root
review** on branch `rust`: Phase3B is archived/completed, the Phase4 planning
package passed root review at `16192ea`, the Slice0 remediation passed at
`3108305`, and the user explicitly authorized Slice1. Slice0 provides the pure
Rust contract skeleton, dns-core header helper, separate caller/owner
cancellation control, and guarded close completion; Slice1 adds only the
one-exchange/one-socket UDP primitive and its focused tests. The task remains
`in_progress` and must stop for another root review. It must not start Slice2,
TCP/fallback, add a transport C ABI, Go pool-buffer ownership, selector, or
production wiring. The existing `main` release remains Go-only and this
incomplete `rust` branch is not a production target.
The matcher tasks (`08-13-rust-matcher-foundation`,
`08-13-rust-matcher-phase2-expansion`) are archived/completed and are no
longer active work; their historical records are unchanged.

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

The worktree is intentionally dirty. Do not stage all files or infer ownership
from modification time.

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
