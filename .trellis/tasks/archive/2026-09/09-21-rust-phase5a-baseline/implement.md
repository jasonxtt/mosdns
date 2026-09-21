# Implementation plan — Rust Phase 5A Go-only whole-process baseline

Status: **in progress — Slice 2 report complete, awaiting final root review**.

## 0. Pre-start gates

- [x] Verify branch is `rust`; planning HEAD and implementation activation are recorded in the task history.
- [x] Verify planning source anchor `e70a2408e2dcd2141e48bcc84765c5adfa406fe4` is an ancestor of the task revision.
- [x] Verify `.trellis/tasks/` has no unrelated active task; this task was `planning` before activation.
- [x] Read the required project, Rust migration, performance, and backend quality guidance.
- [x] Review `prd.md`, `design.md`, and `research/baseline-evidence.md` in full.
- [x] Preserve all unrelated dirty files; only task-scoped paths are staged for task commits.
- [x] Validate executor `codex:current` and reviewer `chatgpt:6ab0d3ff-8e70-83e8-af40-3beb029ab52c` routing.
- [x] Root reviewer returned planning `PASS` with P0/P1=0 at `bc08b9d76e9b49822c237567637471cd6d9f3cb0`.
- [x] Run `python3 ./.trellis/scripts/task.py start .trellis/tasks/09-21-rust-phase5a-baseline`; task status is now `in_progress`.

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

- [x] Add `tests/phase5a-baseline/README.md` with the one authoritative runner interface and explicit non-goals.
- [x] Add four YAML fixtures: W1 UDP, W1 TCP, W2 cache, and W3 routing.
- [x] Derive the YAML from current Go `udp_server`, `tcp_server`, `forward`, `cache`, `domain_set`, `resp_ip`, and sequence contracts.
- [x] Add fixed JSONL workloads with stable case IDs, expected rcode/answer/route class, deadline, and weight fields.
- [x] Add the narrow Go helper for deterministic upstreams, correctness-aware replay, fixed-rate stages, latency samples/counters, and Linux `/proc` sampling.
- [x] Add no new module dependency; `go.mod` and `go.sum` are unchanged.
- [x] Controlled upstream supports only the committed UDP/TCP identities, deterministic answer table, fixed delay flag, counters, and signal shutdown.
- [x] Implement response ID/question/rcode/answer validation; wrong/protocol results never count as useful throughput.
- [x] Gate smoke/helper success on wrong-response, protocol, transport, timeout, and sender-shortfall counters; retain a deliberate no-listener failure regression.
- [x] Verify W3 route-class semantics against fixture counter deltas: domain hit `A`, IP-rule hit `B→A`, and IP-rule miss `B→C`; add a tampered-counter failure regression.
- [x] Run W2 as separate measured cold-miss and warm-hot stages, with unmeasured prefill and a counter barrier before warm timing; verify exact cold/prefill deltas and zero warm delta.
- [x] Keep fixture counter updates in memory on the hot path, serialize snapshots, and flush only after clean fixture shutdown.
- [x] Freeze the Slice 0 TCP policy as one fresh TCP connection per request in the run manifest.
- [x] Implement `MOSDNS_BINARY` validation and SHA-256 recording without rebuilding the SUT.
- [x] The same Go SUT copied to a second executable path passed the unchanged W1-UDP smoke.
- [x] Cleanup trap was exercised by interrupting a live smoke; no fixture/helper/SUT process remained.
- [x] Produce `research/run-manifest.json` as a non-official schema/template; QPS and official values remain unfrozen.
- [x] Focused helper tests, `go vet`, shell syntax checks, JSON/JSONL parsing, and all four local startup/correctness smokes passed.
- [x] Run `gofmt`, `git diff --check`, and the applicable helper static checks.
- [x] Record exact changed paths, commands, results, and limitations below.

### Slice 0 execution record

The initial implementation commit was `1f270b3ce05de9d0d29a7eebcd322325ae668578`.
The first scoped re-review remediation was `e361cc2edcd2122fcca009920b546f60e7229856`.
The counter-delta and route-regression follow-up was accepted by the scoped
reviewer in commit `481bb1ecee2aaf8572ee3c1a9f4c347303b77660`.
The task-scoped changed paths are:

```text
tests/phase5a-baseline/README.md
tests/phase5a-baseline/configs/{forward-udp,forward-tcp,cache,routing}.yaml
tests/phase5a-baseline/workloads/{forward,cache,routing}.jsonl
tests/phase5a-baseline/cmd/phase5a-baseline/{main.go,main_test.go}
scripts/run-phase5a-baseline.sh
.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json
```

Validation completed on the local macOS arm64 host (not authoritative Linux
baseline evidence):

```text
go test ./tests/phase5a-baseline/...                         PASS
go vet ./tests/phase5a-baseline/...                          PASS
bash -n scripts/run-phase5a-baseline.sh                      PASS
git diff --check                                              PASS
JSON/JSONL fixture parsing                                      PASS
Go-only SUT build (SKIP_UI_BUILD=1, CGO_ENABLED=0)              PASS
W1-UDP/W1-TCP/W2-cold/W2-warm/W3 local correctness smoke        PASS
same Go binary copied to second path, W1-UDP smoke               PASS
deliberate no-listener failure gate                             PASS
W2 cold/prefill exact counter deltas and zero warm delta           PASS
tampered W3 route-counter verifier regression                     PASS
interrupted smoke cleanup: no fixture/helper/SUT processes      PASS
```

The remediation smoke completed with zero wrong/protocol/transport/timeout or
sender-shortfall counters. W2 emitted distinct cold and warm stages; the
upstream counter delta was exactly one per case for both cold and unmeasured
prefill, then zero during the measured warm stage. W3 counter verification showed the
three route identities: domain hit on `route-a`, IP-rule hit on `route-b`
followed by `route-a`, and IP-rule miss on `route-b` followed by `route-c`.
The tampered-counter unit regression rejected a missing route leg. The
copied-binary-path smoke used the same unchanged SUT from a second path.
The local host cannot provide the required Linux amd64 `/proc` resource
evidence; Slice 1 must run in the isolated Linux environment and will not
treat this smoke as the official baseline.

### Slice 0 exit gate

Stop and request scoped root review of the exact Slice 0 commit. Review must confirm:

- product paths and dependency manifests untouched;
- all three workload groups exist with deterministic correctness semantics;
- replaceable-binary contract does not rely on Go internals;
- no generic benchmark/platform scope creep;
- cleanup, route-counter accounting, cold/warm lifecycle, and failure gating
  are real, not documentation-only.

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

- [x] Use a Linux amd64 isolated environment, not macOS and not production `mosdns`.
- [x] Build the Go-only SUT with `SKIP_UI_BUILD=1`, `CGO_ENABLED=0`, `GOOS=linux`, `GOARCH=amd64`, empty `GO_TAGS`, and no Rust backend selector.
- [x] Record task SHA, source anchor, product-path diff check, toolchain/build command, `go.mod`/`go.sum` hashes, binary SHA-256, GOMAXPROCS/GOGC/GOMEMLIMIT, kernel/CPU/memory/FD environment.
- [x] Run preflight and correctness smoke for W1-UDP, W1-TCP, W2, W3.
- [x] Run the bounded pilot only to select a useful fixed-rate range; do not report pilot as official capacity evidence.
- [x] Freeze `run-manifest.json` with official offered-QPS ladder, stage durations, request deadline, warm-up/prefill procedure, fixture delay, CPU affinity/isolation, scenario order, offline SUT startup environment, and file hashes before official repetitions.
- [x] Compute and record manifest SHA-256; official execution refuses a hash mismatch.
- [x] Run at least three complete official repetitions per measured variant with the fixed order and ladder.
- [x] Retain every valid/invalid run. Never overwrite or delete a poor result.
- [x] Store per-stage counters/histograms/percentiles, sender shortfall, upstream counter deltas, and resource samples.
- [x] Verify W2 cold/warm upstream counters and W3 route counters match the frozen correctness model.
- [x] Verify no wrong response/mixup is silently counted as useful throughput.
- [x] Verify CPU and RSS samples cover each official stage.
- [x] Verify harness/upstream headroom and document any sender-limited point as invalid/limited rather than SUT capacity.
- [x] Verify cleanup leaves no benchmark/SUT process or listener.
- [x] Record exact commands and evidence paths in this file.

### Slice 1 execution record

The final frozen manifest is `afa071f1cb2fd05bf2f3727ffaa706715019526a`
with SHA-256 `a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7`.
The final official evidence is under:

```text
.trellis/tasks/09-21-rust-phase5a-baseline/research/results/official-20260921/frozen-*
```

The matrix contains 36 retained final run directories: 3 repetitions × 3
offered QPS values (`5`, `10`, `20`) × W1-UDP/W1-TCP/W2/W3. W2 has separate
10-second fixed-rate cold and warm measured stages plus an unmeasured one-pass
prefill. There are 45 final measured stage rows in total. Every final stage
has zero wrong/protocol/transport/timeout/sender-shortfall counters, all
scheduled requests are correct-on-time, and every stage has `/proc` resource
samples. W2 counter deltas and W3 route counters were verified by the runner
after graceful fixture shutdown.

Pilot and remediation history is retained beside the final evidence. The
100-QPS and 50-QPS pilot points were not selected: 100 QPS produced W1-UDP
transport errors, while 50 QPS produced a W2 cold counter delta inconsistent
with the frozen cache model. Earlier official attempts are retained as
invalid/non-final evidence with their reasons: missing `rg` in the minimal
container, W2 warm one-pass methodology before its fix, and public config
package startup attempts before `/dev/null` was frozen. No invalid attempt is
included in the final matrix.

Validation and audit commands included `task.py validate`, Go test/vet,
`bash -n`, JSON parsing, exact manifest/input hash checks, fixed-rate/counter
assertions over all 36 final runs, resource-sample coverage checks, and a
post-run process/listener cleanup check. The authoritative scoped root reviewer
accepted this evidence as `SLICE 1: PASS`; the immutable evidence commit was
`e3338226be28ad99b5d621dfd5ccf972d13e32b2`.

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

- [x] Write `docs/rust/phase5a-go-baseline.md` with source/binary identity, environment, manifest/config/workload hashes, exact commands, scenario descriptions, and raw evidence locations.
- [x] Summarize every official repetition, not just the best one.
- [x] Report p50/p95/p99, offered/sent/correct-on-time effective throughput, wrong/error/timeout/sender-shortfall counts, CPU/query, stable/peak RSS for every stage.
- [x] Report spread/variation and every retained invalid run with its reason.
- [x] State explicitly that the result is Go-only, controlled-local, Linux amd64 baseline evidence; no Rust performance or full Phase 5A compatibility claim is made.
- [x] Document the exact future rerun command using `MOSDNS_BINARY` and state that later Go/Rust comparison must rerun Go in the same environment/session.
- [x] Confirm no public DNS dependency, production deployment, API/WebUI work, Rust host code, transport feature work, or feature-coverage ownership change occurred.
- [x] Map final evidence to PRD A1–A13.
- [x] Run final diff/path audit proving only authorized paths changed and `go.mod`/`go.sum` remain untouched.

### Slice 2 execution record

The report is `docs/rust/phase5a-go-baseline.md`. It records all 45 final
measured rows, the 15 cross-repetition spread rows, the frozen input/build
identity, the exact rerun entry point, raw evidence locations, invalid-run
history, and PRD A1–A13 mapping. The report explicitly treats
`environment-frozen.json` as authoritative, identifies `environment-final.json`
as superseded metadata, and records that future execution must use the
approved SSH Linux workflow rather than starting a local VM.

The final quality pass before review includes task validation, focused Go
test/vet, shell syntax, JSON/JSONL parsing, exact frozen-evidence assertions,
`git diff --check`, and an authorized-path/product-path audit. The Trellis
spec-update review found no reusable production coding convention to add: the
baseline contracts are task-local and already captured by `prd.md`,
`design.md`, and the report, so `.trellis/spec/` remains unchanged.

### Final exit gate

Request final root review of the exact task completion diff and evidence. The reviewer must return `FINAL: PASS` or `FINAL: FAIL` for this task only.

On `FINAL: PASS`:

1. finish/archive this task using normal Trellis workflow;
2. stop;
3. do **not** create/start or implement the Phase 5A native-host task under this authorization;
4. do **not** deploy or replace production `mosdns`.
