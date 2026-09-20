# Rust migration handover

Last verified: `2026-09-20`

Concise cross-session handover for the Rust migration on branch `rust`.

## Source of truth (precedence)

1. **Live execution** — `python3 ./.trellis/scripts/task.py current` and
   `python3 ./.trellis/scripts/task.py list`, plus the selected task's `prd.md`,
   `design.md`, and `implement.md`. An empty current pointer does not mean no
   task is in progress.
2. **Evidence** — archived task artifacts under `.trellis/tasks/archive/`.
3. **Architecture** — `docs/ai/rust-rewrite-plan.md`.
4. This file is a **concise handover only**. It summarizes current state and
   constraints; it does not restate slice narratives, test transcripts, or file
   inventories. Where it disagrees with the sources above, they win.

## Resume protocol

1. Read `AGENTS.md`, `docs/ai/project-context.md`, `docs/ai/config-notes.md`.
2. Read this file and `docs/ai/rust-rewrite-plan.md`.
3. Check state: `git branch --show-current`, `git status --short --branch`,
   `python3 ./.trellis/scripts/task.py list`.
4. Read all three artifacts of the task being resumed, if any.
5. Run the `trellis-before-dev` skill before changing implementation code.
6. **Preserve unrelated dirty files.** Keep Trellis auto-commit disabled, never
   use `git add -A`, and stage exact paths only.

## Branch and worktree

- Dedicated worktree `/Users/tom/github/mosdns-rust` on branch `rust`.
- Base commit `3896a4a7e0ce4311b40a7e4c80c93f2c8b3b4f1d`, base release `v0.7.1`.
- Do not switch to `main` merely from the folder name. `main` stays Go-only; the
  `rust` branch is a future pure Rust-native replacement, not an intermediate
  production runtime.

## Status as of 2026-09-20

At this documentation check, `task.py current` has no selected task;
`09-20-rust-phase4-quic-reuse-multiplexing/task.json` is `in_progress`, and
Slice 0 model implementation is present. This is not evidence that the whole
task or an outstanding review gate has passed. Resume from the task's actual
artifacts and evidence; do not treat the earlier “planning only” summary as
current. Existing Go live behavior is unchanged and no Rust path is enabled
by default.

Completed milestones (all archived; each archive holds its own evidence):

| Milestone | Outcome | Archive |
| --- | --- | --- |
| Phase 1 cache | `rust/cache-core`: Moka/`Bytes` cache, raw-wire TTL, versioned ABI, opt-in cgo bridge. Soak/Miri/host gates closed; hybrid bridge missed the 10% QPS gate, so Rust stays experimental. | `.trellis/tasks/archive/2026-08/08-13-rust-cache-foundation/` |
| Phase 2 matcher (+ expansion) | `rust/matcher-core`: pure Rust domain/IP matchers and compiled rule indexes, plus `sd_set`/`si_set`/valued `domain_mapper` opt-in adapters. | `.trellis/tasks/archive/2026-08/08-13-rust-matcher-foundation/`, `.trellis/tasks/archive/2026-08/08-13-rust-matcher-phase2-expansion/`, `.trellis/tasks/archive/2026-08/08-17-rust-phase2-matcher-correctness-remediation/` |
| Phase 3A query/wire | `rust/dns-core`: DNS wire parsing, TTL/EDNS/ECS helpers, opt-in query ABI/Go adapter. | `.trellis/tasks/archive/2026-08/08-15-rust-phase3-query-execution-core/` |
| Phase 3B sequence | `rust/sequence-core`: owned execution state, validated program model, explicit continuation stack, fuel/cancellation. No Go adapter/ABI/selector. | `.trellis/tasks/archive/2026-08/08-17-rust-phase3b-sequence-execution-foundation/` |
| Phase 4 UDP/TCP | `rust/upstream-core`: numeric UDP, fresh plain TCP, and UDP TC→TCP policy with cancellation/deadline/close contracts. | `.trellis/tasks/archive/2026-09/08-17-rust-phase4-upstream-foundation/` |
| Phase 4 secure DoT/DoH | `rust/upstream-core` secure module: one-shot DoT and DoH over HTTP/1.1 and HTTP/2, explicit numeric dial separate from service identity, verified TLS by default. | `.trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/` |
| Phase 4 resolver/bootstrap | `rust/upstream-core` resolver module: single-family numeric-address resolution through a numeric bootstrap peer, TTL/cache/refresh, single-flight publication, close/drain. | `.trellis/tasks/archive/2026-09/09-17-rust-phase4-endpoint-resolution-foundation/` |
| Phase 4 dual-stack selection | Resolver extension: explicit `bootstrap_version=0` A+AAAA candidate collection, A-preferred selection, per-family TTL/state. No connection fallback or Happy Eyeballs. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-dual-stack-endpoint-selection/` |
| Phase 4 connection reuse | `rust/upstream-core` reuse owner: explicit reuse key over numeric dial + transport + secure identity/authority/ALPN, serial-per-connection minimum, bounded idle/pending limits, typed errors. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-connection-reuse-pipeline/` |
| Phase 4 QUIC/HTTP3/DoQ foundation | `rust/upstream-core` fresh one-shot DoQ and DoH3 clients with exact ALPN, identity separation, bounded response validation, lifecycle/commit semantics, and no fallback. | `.trellis/tasks/archive/2026-09/09-18-rust-phase4-quic-http3-doq-foundation/` |

The workspace also holds `rust/runtime`, the transitional single `staticlib`
from the earlier hybrid work. New Phase 3B+ modules compose as plain Rust
libraries for the future host and must not add ABI symbols to extend the hybrid
shape or create a second runtime.

### Secure upstream status and caveats

Secure upstream Slices 0–4 were root-reviewed and closed (Slice4 `PASS / CLOSED`,
P0=0/P1=0), then archived at
`.trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/`.
Its exact commands, results, and limitations are in that archive's
`implement.md`.

Two evidence caveats carried from that review:

- **Rust 1.85 was never actually installed or run.** MSRV is evidenced only
  indirectly, via resolver-3 selection plus a `cargo metadata` audit showing no
  resolved package above 1.85, and the stable-toolchain Linux CI build.
- The task's `research/secure-upstream-evidence.md` is **evidence-only**, not
  normative; the reviewed code and `implement.md` are authoritative.

Scope of what exists: the DoT/DoH and QUIC/HTTP3 foundations are **bounded
one-shot, fresh-connection** transport work. The current task adds QUIC-specific
connection reuse/multiplexing; it is not host wiring or completion of the Phase 4
data plane.

## Product priorities and next frontier

The user confirmed Linux amd64 as the primary platform. Full backend feature
compatibility and correctness are prerequisites; query latency (especially
p95/p99), sustainable useful throughput, overload recovery, and long-running
stability are the primary improvements. Memory is secondary with no required
reduction percentage; bounded, reclaimable extra memory is acceptable when
measurements justify the performance benefit.

The architecture plan now brings **Phase 5A minimal native host** forward:
after the current QUIC reuse task closes within its existing scope, prioritize
YAML subset -> UDP/TCP listener -> async sequence -> matcher/cache/real upstream
-> response and basic audit/metrics. Do not wait for every remaining Phase 4
protocol or tuning task before validating this composition. Reject unsupported
configuration explicitly; this early host is isolated and experimental.

Remaining Phase 4 foundations compose with **5B full query features**; **5C full
control plane** covers APIs, existing UI, persistent state and updates; **5D**
validates full-system performance and stability; **Phase 6** retires hybrid
scaffolding before production replacement. Current sequence foundation and
serial transport reuse are not evidence of completed async host wiring or
final concurrency performance.

- Architecture, dependencies, and gates: `docs/ai/rust-rewrite-plan.md`.
- Feature ownership and native acceptance inventory:
  `docs/rust/feature-coverage.md`.
- Reproducible performance/stability workloads and threshold-freeze rules:
  `docs/rust/performance-validation.md`.

This roadmap update does not change the existing QUIC task, close its gates,
create/start another task, or authorize runtime changes or deployment. Later
implementation needs its own scoped task and review.

## Non-negotiable constraints

- Preserve the MosDNS **product contract**: YAML/config and sequence/plugin
  semantics, final DNS/routing/audit behavior, API/WebUI workflows, metrics and
  persistent formats. Go internals and accidental quirks are not contract.
- The final target is a **pure Rust-native host**. Existing Phase 1/2/3A hybrid
  adapters, selectors, Go mirrors/fallback, and cgo scaffolding stay safe but
  frozen: do not extend that pattern into Phase 3B+.
- Do not make the incomplete `rust` branch a production or default release until
  the Rust-native E2E gate **and** the later hybrid-retirement gate pass.
- KixDNS `2da3a2d` is a selective GPL-3.0 design/source reference, not a subtree
  merge. `/Users/tom/github/mosdns-rust-cache` is a read-only prototype
  reference, not a drop-in.
- Validate locally, then on isolated `mos-test`, and promote to production only
  with explicit user approval.
