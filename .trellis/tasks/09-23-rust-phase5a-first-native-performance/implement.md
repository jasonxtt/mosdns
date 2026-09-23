# Implementation plan — first native whole-process comparison

Status: **planning only**. Execution is for a user-selected separate conversation. Do not mark checklist items done based on this planning review.

## Before start

- [ ] Read `AGENTS.md`, project-context, config-notes, rust-handover, rust-rewrite-plan, `.trellis/workflow.md`, `docs/rust/performance-validation.md`, task PRD/design and affected specs. Inspect dirty worktree and preserve unrelated changes; keep Trellis auto-commit disabled.
- [ ] Obtain independent planning review for the exact planning commit. Only after review PASS, activate with `task.py start` and record the reviewed head. The user chooses the executor/reviewer conversations.
- [ ] Freeze source identities and compare current hashes of seven baseline config/workload inputs with the archived report. If any differ, stop and resolve provenance before implementation.

## Slice 0 — tooling and VM preflight (no official results)

Allowed edits: `scripts/run-phase5a-baseline.sh` and `tests/phase5a-baseline/cmd/phase5a-baseline/**` only for narrowly required paired-measurement support/tests; this task directory. Original configs/workloads and product paths are read-only.

- [ ] Audit current native-host CLI, runner, metric coverage, cleanup, immutable-input hash checks and Go/Rust-neutral oracle. Add focused test-only fixes only where a real gap prevents valid paired evidence; keep historical runner mode usable.
- [ ] On `ssh mosdns-rust`, use task-owned disk-backed paths, inspect CPU/FD/disk/load/port availability, and record exact host/toolchains. Build Go-only from the archived source commit and native-host from reviewed `rust` commit; record commands and hashes. No build/deployment on production `mos`.
- [ ] Run unchanged W1/W2/W3 smoke for both. Verify W2 cold/prefill/warm deltas, W3 route legs/order and wrong-answer rejection, plus stop/rebind cleanup. Any product failure becomes a separate reviewed defect; do not change corpus or loosen oracle.
- [ ] Run a brief labeled pilot to choose a fixed-rate ladder and test sender/upstream headroom on the 2-CPU host. Verify CPU affinity/cgroup and environment parity. If 2 CPU cannot sustain valid measurement, stop with an infrastructure-limited report rather than invent results.
- [ ] Focused helper tests/vet, shell syntax, JSON validation, `git diff --check`; commit/push scoped changes and obtain Slice 0 reviewer PASS before freezing the official manifest.

## Slice 1 — freeze and official paired runs

Allowed edits: this task's `research/**` manifest/results and small benchmark-tool remediation reviewed before fresh official runs. No product change or archived evidence rewrite.

- [ ] Freeze a new task-owned manifest **before** official samples: exact Go/Rust source/binary/helper and corpus hashes, VM/environment, config parity, CPU placement, scenario/order/repetition/QPS/duration/deadline, TCP policy, cache prefill, logs/audit and criteria for invalid stage. Record SHA-256 and review it before official execution.
- [ ] Run frozen open-loop matrix at least three times per valid point; alternate candidate order. Keep all valid/invalid attempts with separate directories and reasons. Verify manifest SHA, same input hashes, correct-on-time counters, W2/W3 fixture deltas, resource samples and headroom after each run.
- [ ] Inspect overloading/recovery behavior separately and ensure missing responses, timeouts and sender shortfall are not hidden. If a method/manifest change is required, issue a new manifest and rerun both candidates at all affected points.
- [ ] Check cleanup on VM (task-owned processes/listeners only), retain hashes and exact commands; commit/push evidence index and obtain Slice 1 reviewer PASS.

## Slice 2 — comparison report and closure

Allowed edits: task evidence and `docs/rust/phase5a-native-comparison.md` (or equivalent report). Handover/coverage status may be updated narrowly after review. No Rust/Go product optimization in this slice.

- [ ] Aggregate every valid paired point: per-run and median/range p50/p95/p99 with sample counts, effective throughput, errors, CPU/query, RSS/FD, W2 hits and W3 routes. Show invalid point reasons and environmental noise. Distinguish single-core comparison from unproven multi-core capacity.
- [ ] If one repeatable hotspot appears, capture lightweight profile or explain why unavailable; label inference vs proof. State the next bounded optimization or feature task, including its correctness and performance regression gate, without claiming it is implemented.
- [ ] Report full scope limits: W1/W2/W3 subset, controlled local upstream, 2-CPU VM, no production config, no full-feature/soak/cutover conclusion. Update handover only to say what measured and what remains open.
- [ ] Run applicable documentation/manifest consistency checks, `task.py validate`, `git diff --check`; obtain independent final review before `finish/archive`. Record actual review result and commit SHA; do not pre-fill PASS.

## Stop rules

Stop official performance claims for any scenario with differing DNS/routing/cache semantics, wrong-answer acceptance, unstable sender/upstream, altered frozen inputs, missing binary/source identity, unresolved benchmark interference, or incomplete evidence. Preserve the failure as a concrete follow-up rather than adjusting the workload after seeing results. Neither this task nor its archive authorizes production rollout.
