# Slice 1 red/green evidence

Execution timestamp: `2026-09-26T17:40:18Z` (local task date 2026-09-27).
The implementation stayed within the C2C iteration-4 plan: helper/CLI files,
README/script documentation, and task evidence only. No Rust source, Go
product code, SSH, SUT launch, pilot/official traffic, profiling, or capacity
claim was added.

## Red then green

The first TDD command after adding the contract tests was:

```text
go test ./tests/phase5a-baseline/cmd/phase5a-baseline
```

It failed at compile time on the intentionally absent reliability schema,
accounting, deadline, runner, and assessor symbols. The smallest subsequent
changes introduced the versioned schema and accounting equations, then the
absolute-deadline classifier, bounded runner/evidence writer, raw assessor,
loopback transport, CLI dispatch, and shell entry.

The focused green command:

```text
go test ./tests/phase5a-baseline/cmd/phase5a-baseline -run 'TestReliability' -count=1 -v
```

passed all reliability tests, including slot conservation, complete-frame
`dns_sent`, monotonic absolute deadlines, open-loop in-flight rejection,
   explicit scheduler skips, slow/absent response, writer error, permanently blocked sink and bounded
cleanup failure, raw-evidence fail-closed assessment, same-PID recovery versus
restart, UDP/TCP loopback, strict wrong-answer rejection, fresh derived output,
and historical Git archive/hash positive and negative cases.

## Regression gates

The following completed successfully:

```text
go test ./tests/phase5a-baseline/cmd/phase5a-baseline -count=1
go test -race ./tests/phase5a-baseline/cmd/phase5a-baseline -count=1
bash -n scripts/run-phase5a-baseline.sh
bash -n scripts/run-phase5a-reliability.sh
git diff --check
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-27-rust-phase5a-measurement-reliability
```

The existing helper package remains green under the race detector. Rust
performance/runtime tests were not substituted for this unit because Slice 1
does not modify Rust source and C2C explicitly bounded G1 to helper/CLI gates.

## G1 boundary

The new runner enforces loopback-only addresses, uses a fixed worker pool and
bounded dispatch/evidence queues, preserves control records when evidence
persistence fails, and writes `load_valid=false`/`evidence_valid=false` through
raw evidence facts. The assessor recomputes all three latency views and does
not trust stored summaries. The public shell runner is local-only and does not
launch MosDNS or perform remote measurements.

G1 remains subject to C2C review. A green helper unit does not clear the Slice
0 process-profiler blocker, so Slice 2 calibration/pilot/official/profile work
must remain stopped unless a later review explicitly changes that gate.

## C2C remediation gate

The first G1 review found blocking P1/P2 issues. They were closed in the
current working diff with new controlled slow-connect/partial-write tests,
raw-window derivation and tamper coverage, run-level evidence-failure stop
and finite record budgets, latency denominators/goodput, and public helper plus
shell subprocess coverage. The final hash index is regenerated only after the
post-remediation validation and the next C2C review.

## Iteration 7 remediation validation

The follow-up fixes add explicit write-complete offsets and deadline-race
classification, raw-derived on-time-rate and resource/identity gates,
cancellation-aware evidence ownership, factual compact control records for
oversize evidence, raw persistence before CLI sink-close errors,
successful-response-only percentile samples, and shell alias rejection. New
focused tests cover a complete write crossing the service deadline, frozen
window criteria/resource/sequence facts, failure denominators, complete
missing-slot reporting after non-error queue overflow, independent wall-clock
jumps, and a complete blocked-sink `runReliability` invocation that returns
cleanup failure without releasing the sink.

The final local package and race commands pass; the next C2C re-review decides
whether these fixes close G1. Slice 2+ remains frozen by the Slice 0 profiler
gate regardless of the helper result.

## Iteration 9 remediation validation

C2C identified one remaining recovery-state defect: the assessor could count
healthy overload-phase windows as recovery after two violating overload
windows, and the raw sequence could omit its initial reference phase. The
validator now requires a reference window at the explicit reference QPS
before overload, while the recovery counter accepts only `phase=recovery`
windows at that same QPS. Negative tests cover missing/wrong-QPS reference
stages and healthy overload windows that must remain
`overload-without-recovery`. The focused package test passes after the change;
the full/race/index gates and the next C2C review are the remaining G1 steps.

## G1 final review

C2C iteration 10 returned `STATE: DONE` and `FINAL: PASS`, accepting all
current Slice 1 findings and the regenerated evidence identity. G1 is closed
for the bounded helper/CLI unit. This does not clear the G0 process-directed
profiler blocker, so no calibration, pilot, official measurement, profiling,
hotspot, or capacity slice may start.
