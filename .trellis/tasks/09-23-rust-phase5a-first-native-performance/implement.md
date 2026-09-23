# Implementation plan — first native whole-process comparison

Status: **in progress — Slice 0 pilot complete; scoped changes awaiting reviewer PASS**. Planning review passed and execution was authorized in this task. No official samples have run and no official manifest is frozen.

## Before start

- [x] Read `AGENTS.md`, project-context, config-notes, rust-handover, rust-rewrite-plan, `.trellis/workflow.md`, `docs/rust/performance-validation.md`, task PRD/design and affected specs. Inspect dirty worktree and preserve unrelated changes; keep Trellis auto-commit disabled.
- [x] Obtain independent planning review for the exact planning commit. Planning review PASS was reported by the user for `605c30577b79d397b5695618dbd2980e550ca6f3`; execution was activated with `task.py start` after the user's authorization.
- [x] Freeze source identities and compare current hashes of seven baseline config/workload inputs with the archived report; all seven match. Exact source, binary, runner, helper, and corpus hashes are recorded in `research/slice0-pilot.md`.
- [x] Follow the design's test-first behavior slices: add rejecting tests for strict W1 DNS responses; W2 cold/warm counters and per-key TTL eligibility; W3 per-request event correlation, exact leg counts/order, duplicates, reversed/missing/unmatched events and stage barriers; alternating schedule and paired completeness; per-input hash mismatch; and recovery requiring one PID/session plus frozen sample/latency criteria. Actual candidate binaries and VM CPU/resource paths remain outside mocks; only controlled loopback DNS fixtures substitute upstreams.

## Slice 0 — tooling and VM preflight (no official results)

Allowed edits: `scripts/run-phase5a-baseline.sh` and `tests/phase5a-baseline/cmd/phase5a-baseline/**` only for narrowly required paired-measurement support/tests; this task directory. Original configs/workloads and product paths are read-only.

- [x] Audit current native-host CLI, runner, metric coverage, cleanup, immutable-input hash checks and Go/Rust-neutral oracle. Add only focused measurement-tool support for gaps that prevented paired evidence; keep historical runner mode usable.
- [x] Strengthen the shared `responseMatches` oracle to enforce the design's full DNS response contract (including QR, ID/opcode, question class, truncation and exact answer set) before running either candidate's smoke matrix; add table-driven rejection tests first. `go test ./tests/phase5a-baseline/cmd/phase5a-baseline -run 'TestResponseMatches' -count=1` passes after the new negative cases first exposed the missing checks.
- [x] Replace W3 aggregate-only route acceptance with a test-first per-request event oracle: shared-sequence fixture journal, client request ledger, stage sequence barriers, one in-flight request per canonical question tuple, and exact ordered path checks. Fixture-side DNS IDs remain diagnostic because the live Go-only smoke rewrites them. Freeze/hash the event schema in the Slice 1 manifest.
- [x] Enforce hash checks for every fixed config/workload/helper input before official stages. Pin the runner/helper/fixture process tree externally and verify its observed affinity separately from the pinned SUT.
- [x] Add a narrow continuous-stage run mode: keep SUT and fixtures alive across normal reference, common, near-saturation, overload, and recovery; preserve same PID/session evidence. Keep W2 cold isolated and prefill W2 warm before its continuous sequence. The VM pilot confirms same-process stage identity and TTL eligibility.
- [x] On `ssh mosdns-rust`, use task-owned disk-backed paths, inspect CPU/FD/disk/load/port availability, and record exact host/toolchains. Build Go-only from the archived source commit and native-host from the reviewed Rust commit; record commands and hashes. No build/deployment on production `mos`.
- [x] Run unchanged W1/W2/W3 smoke for both. Verify W2 cold/prefill/warm deltas, W3 route legs/order and wrong-answer rejection, plus process cleanup and port rebind.
- [x] Run a labeled pilot to choose a fixed-rate ladder, establish three stable normal-reference samples and p95/p99 ceilings/sample minimum; test same-process continuity, W2 per-key TTL eligibility, event-journal overhead, trace capacity, sender/upstream headroom, and CPU affinity on the 2-CPU host. The pilot did not establish stable overload; capacity and service-recovery claims remain indeterminate unless official data demonstrates overload.
- [ ] Focused helper tests/vet, shell syntax, task/JSON validation and `git diff --check`; commit/push scoped changes and obtain Slice 0 reviewer PASS before freezing the official manifest.

### Live smoke finding

The first Go-only W3 smoke produced correct DNS responses and route events, but confirmed upstream fixtures see proxy-assigned DNS IDs (`0`/`1`) rather than client ledger IDs (`41645`–`41647`). No frozen config or workload was changed. The W3 evidence join now uses canonical question tuples and occurrence order, with a test/runner guarantee that identical tuples never overlap; the event DNS ID remains diagnostic. The original failed attempt is retained under the VM task results.

The first continuous W2 pilot exposed a counter-oracle error: a 30-request stage for each cache key correctly caused one fixture miss per key, but the oracle expected 30. New failing tests reproduced this for both stage deltas and final session totals; the oracle now expects one cold miss per unique key and still requires zero warm delta. The failed v2-helper pilot is retained. The v3 helper was built on the VM, then bilateral smoke and a three-pair pilot completed with it.

Reviewing the v3 pilot showed that raw resource samples covered only SUT CPU/RSS, so they could not substantiate generator/fixture headroom or FD usage. The v4 helper samples SUT, load-generator, and every fixture process by role at each stage, including cumulative user/system CPU, RSS, and FD count; runner validation rejects any missing role. The v4 bilateral smoke and three-pair pilot passed at 20/40/60/100 QPS; the W2 warm sequence stayed within its 30-second TTL with the frozen 500 ms margin. At 100 QPS, process CPU remained low (under 3% for the measured SUTs), so that ladder did not approach saturation.

A v4 exploratory ladder at 1,000/2,000/4,000/8,000 QPS found no sender shortfall at 1,000 QPS, but shortfall began at 2,000 QPS and rose at higher rates while SUT CPU remained below saturation. Those high-rate rows are retained as invalid load probes, not capacity results. Helper v5 adds a sender-validity gate: every accepted stage must have zero dropped schedule slots and `sent == scheduled`. The first v5 final-pilot Go W1-UDP attempt at 1,000 QPS dropped two slots in three seconds and is retained as invalid; no Rust run had yet been made at that pair position. Keep the planned 200/400/800/1,000 QPS ladder fixed for both candidates and preserve every shortfall attempt. If no stable valid overload point exists, the report must leave saturation, capacity, and service-recovery claims indeterminate; do not relabel a low-load probe as overload.

## Slice 1 — freeze and official paired runs

Allowed edits: this task's `research/**` manifest/results and small benchmark-tool remediation reviewed before fresh official runs. No product change or archived evidence rewrite.

- [ ] Freeze a new task-owned manifest **before** official samples: exact Go/Rust source/binary/helper and corpus hashes, VM/environment, config parity, CPU placement, scenario/order/repetition/QPS/duration/deadline, TCP policy, per-key W2 prefill times, 30-second fixture TTL and safety margin, W3 event schema, continuous stage sequence, recovery reference rate/duration/sample minimum/p95-p99 ceilings, logs/audit and criteria for invalid stage. Record SHA-256 and review it before official execution.
- [ ] Run frozen open-loop matrix at least three times per valid point; alternate candidate order. Preserve the same SUT PID/fixture session across each staged sequence, while W2 cold and W2 warm retain their separately declared lifecycle. Keep all valid/invalid attempts with separate directories and reasons. Verify manifest/input hashes, correct-on-time counters, exact W3 event path per sent request, stage sequence barriers, recovery criterion, resource samples and headroom after each run.
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
