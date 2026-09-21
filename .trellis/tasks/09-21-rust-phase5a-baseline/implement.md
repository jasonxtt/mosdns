# Implementation plan — Rust Phase 5A Go-only whole-process baseline

Status: **planning only**. Do not run `task.py start`, modify implementation paths, execute official benchmarks, or create a later Phase 5A host task until this plan receives an explicit root-review `PASS`.

## 0. Pre-start gates

- [ ] Verify branch is `rust` and record current HEAD.
- [ ] Verify planning source anchor `e70a2408e2dcd2141e48bcc84765c5adfa406fe4` is an ancestor of the task revision.
- [ ] Verify `.trellis/tasks/` has no unrelated active task and this task is `planning`.
- [ ] Read `AGENTS.md`, `docs/ai/rust-rewrite-plan.md`, `docs/ai/rust-handover.md`, `docs/rust/feature-coverage.md`, `docs/rust/performance-validation.md`, `.trellis/spec/backend/rust-migration.md`, and repository quality/error guidance.
- [ ] Review `prd.md`, `design.md`, and `research/baseline-evidence.md` in full.
- [ ] Preserve all unrelated dirty files; stage exact paths only. Never use `git add -A`, broad checkout/reset, rebase, or cleanup.
- [ ] Validate the selected executor/reviewer routing under the repository's current Trellis process.
- [ ] Root reviewer returns explicit planning `PASS` with P0/P1=0.
- [ ] Only after that `PASS`, run the repository-equivalent `task.py start rust-phase5a-baseline`.

## 1. Slice 0 — freeze fixture/harness contract and correctness smoke

### Allowed implementation surface

```text
tests/phase5a-baseline/**
scripts/run-phase5a-baseline.sh
.trellis/tasks/09-21-rust-phase5a-baseline/research/**
.trellis/tasks/09-21-rust-phase5a-baseline/implement.md
```

No existing product source or dependency manifest is allowed.

### Checklist

- [ ] Add `tests/phase5a-baseline/README.md` documenting the one authoritative runner interface and explicit non-goals.
- [ ] Add four committed YAML fixtures: W1 UDP, W1 TCP, W2 cache, W3 routing.
- [ ] Derive YAML semantics from current Go source; do not invent future Rust behavior.
- [ ] Add fixed line-oriented workload files with stable case IDs and expected outcome/route fields.
- [ ] Add the narrow Go helper command inside the existing module, with only deterministic upstream fixture, correctness-aware request replay, fixed-rate stage machinery, histogram/counters, and Linux process sampling primitives needed by this task.
- [ ] Add no new module dependency; prove `go.mod` and `go.sum` unchanged.
- [ ] Controlled upstream supports only committed UDP/TCP fixture behavior, deterministic answer maps, optional manifest-frozen delay, counters, and clean shutdown.
- [ ] Implement response association and correctness checks; wrong answers are never counted as useful throughput.
- [ ] Implement the replaceable `MOSDNS_BINARY`/explicit binary interface and record binary SHA-256.
- [ ] Prove the same Go SUT copied to a second executable path passes the same correctness smoke without code/config changes.
- [ ] Implement cleanup traps; after an injected failure no SUT/upstream/listener process remains.
- [ ] Produce `research/run-manifest.json` schema/template, but do not yet freeze official QPS values until the Linux pilot in Slice 1.
- [ ] Run focused Go tests for helper/parser/histogram/accounting logic plus YAML startup/correctness smoke with the Go-only SUT.
- [ ] Run repository formatting/static checks applicable to added Go/shell files.
- [ ] Record exact changed paths, commands, results, and limitations in this file.

### Slice 0 exit gate

Stop and request scoped root review of the exact Slice 0 commit. Review must confirm:

- product paths and dependency manifests untouched;
- all three workload groups exist with deterministic correctness semantics;
- replaceable-binary contract does not rely on Go internals;
- no generic benchmark/platform scope creep;
- cleanup and wrong-response accounting are real, not documentation-only.

`PASS` authorizes Slice 1 only.

## 2. Slice 1 — isolated Linux amd64 Go baseline execution

### Allowed change surface

Prefer no code changes. Only narrow fixes in the already-authorized baseline tooling/fixtures are allowed when required to obtain valid evidence; any semantic workload change or dependency/product change reopens planning review.

Evidence paths:

```text
.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json
.trellis/tasks/09-21-rust-phase5a-baseline/research/results/**
.trellis/tasks/09-21-rust-phase5a-baseline/implement.md
```

### Checklist

- [ ] Use a Linux amd64 isolated environment, not macOS and not production `mosdns`.
- [ ] Build the Go-only SUT with `SKIP_UI_BUILD=1`, `CGO_ENABLED=0`, `GOOS=linux`, `GOARCH=amd64`, empty `GO_TAGS`, and no Rust backend selector.
- [ ] Record task SHA, source anchor, product-path diff check, toolchain/build command, `go.mod`/`go.sum` hashes, binary SHA-256, GOMAXPROCS/GOGC/GOMEMLIMIT, kernel/CPU/memory/FD environment.
- [ ] Run preflight and correctness smoke for W1-UDP, W1-TCP, W2, W3.
- [ ] Run the bounded pilot only to select a useful fixed-rate range; do not report pilot as official capacity evidence.
- [ ] Freeze `run-manifest.json` with official offered-QPS ladder, stage durations, request deadline, warm-up/prefill procedure, fixture delay, CPU affinity/isolation, scenario order, and file hashes **before** official repetitions.
- [ ] Compute and record manifest SHA-256; official execution refuses a hash mismatch.
- [ ] Run at least three complete official repetitions per measured variant with the fixed order and ladder.
- [ ] Retain every valid/invalid run. Never overwrite or delete a poor result.
- [ ] Store per-stage counters/histograms/percentiles, sender shortfall, upstream counter deltas, and resource samples.
- [ ] Verify W2 cold/warm upstream counters and W3 route counters match the frozen correctness model.
- [ ] Verify no wrong response/mixup is silently counted as useful throughput.
- [ ] Verify CPU and RSS samples cover each official stage.
- [ ] Verify harness/upstream headroom and document any sender-limited point as invalid/limited rather than SUT capacity.
- [ ] Verify cleanup leaves no benchmark/SUT process or listener.
- [ ] Record exact commands and evidence paths in this file.

### Slice 1 exit gate

Stop and request scoped root review of the exact evidence/tooling diff. `PASS` must confirm the manifest was frozen before official samples, Linux amd64 isolation is credible, all repeats are retained, correctness/failures are included, and CPU/RSS evidence is tied to each run.

`PASS` authorizes Slice 2 only.

## 3. Slice 2 — baseline report and acceptance closure

### Allowed surface

```text
docs/rust/phase5a-go-baseline.md
.trellis/tasks/09-21-rust-phase5a-baseline/research/**
.trellis/tasks/09-21-rust-phase5a-baseline/implement.md
```

No benchmark semantic changes in this slice. If the report reveals invalid methodology requiring a workload/tool change, return to the earlier slice and re-review rather than editing evidence into compliance.

### Checklist

- [ ] Write `docs/rust/phase5a-go-baseline.md` with source/binary identity, environment, manifest/config/workload hashes, exact commands, scenario descriptions, and raw evidence locations.
- [ ] Summarize every official repetition, not just the best one.
- [ ] Report p50/p95/p99, offered/sent/correct-on-time effective throughput, wrong/error/timeout/sender-shortfall counts, CPU/query, stable/peak RSS for every stage.
- [ ] Report spread/variation and any invalid runs with reason.
- [ ] State explicitly that the result is Go-only, controlled-local, Linux amd64 baseline evidence; no Rust performance or full Phase 5A compatibility claim is made.
- [ ] Document the exact future rerun command using `MOSDNS_BINARY` and state that later Go/Rust comparison must rerun Go in the same environment/session.
- [ ] Confirm no public DNS dependency, production deployment, API/WebUI work, Rust host code, transport feature work, or feature-coverage ownership change occurred.
- [ ] Map final evidence to PRD A1–A13.
- [ ] Run final diff/path audit proving only authorized paths changed and `go.mod`/`go.sum` remain untouched.

### Final exit gate

Request final root review of the exact task completion diff and evidence. The reviewer must return `FINAL: PASS` or `FINAL: FAIL` for this task only.

On `FINAL: PASS`:

1. finish/archive this task using normal Trellis workflow;
2. stop;
3. do **not** create/start or implement the Phase 5A native-host task under this authorization;
4. do **not** deploy or replace production `mosdns`.
