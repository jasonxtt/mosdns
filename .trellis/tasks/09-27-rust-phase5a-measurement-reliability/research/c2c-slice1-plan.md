# C2C Slice 1 plan

Task `c2c_5a9e`, iteration 4, returned `STATE: PLAN` after G0 PASS.
The admitted unit is offline/loopback Phase 5A helper tooling only. It must not
run SSH, pilot/calibration, official Go/Rust traffic, profiling, or produce
hotspot/capacity claims. The missing process-directed profiler remains a sticky
gate for Slice 2 and later.

## Boundary

Keep new behavior in reliability-specific files below
`tests/phase5a-baseline/cmd/phase5a-baseline/`; `main.go` may only receive
narrow dispatch/usage plumbing. Add the public `reliability-run` and
`reliability-assess` commands, an independent versioned reliability schema,
`scripts/run-phase5a-reliability.sh`, bounded README documentation, and task
research evidence. Preserve legacy `run`, exchange helpers, ledger,
aggregator, and baseline script semantics. No Rust or Go product edits.

## Ordered red-to-green slices

1. Define tests for a closed reliability-v1 slot/control schema. Every planned
   slot has an integer ID, monotonic planned offset, optional reached offsets,
   one terminal outcome, byte counts, and full-frame `dns_sent`. Recomputable
   equations are `planned = harness_skipped + harness_rejected + started`,
   `started = failed_before_dns_send + dns_sent`, and `dns_sent = sum` of
   mutually exclusive post-send terminals. Duplicates stay diagnostic. Persist
   worker, in-flight, dispatch-queue, evidence-queue, and record-size limits;
   hard limits stay within the G0 256 in-flight / 512-FD envelope.
2. Implement bounded open-loop `reliability-run`: fixed workers, bounded
   dispatch/evidence queues, planner independent of response completion, and
   explicit records for scheduler lag and rejection. Cover normal completion,
   pause/lag, saturation, slow/no response, pre-connect failure, strict wrong
   or protocol response, and resource limits. Only time/network/filesystem
   boundaries may be mocked; planner, queues, accounting, and classification
   remain real.
3. Use one monotonic stage origin and absolute deadlines:
   `planned_at = stage_start + slot_offset`, `service_deadline = planned_at +
   request_deadline`, `collection_deadline = service_deadline + late_drain`.
   Queue, connect, complete UDP/TCP write, and on-time read consume the same
   remaining budget. Test queue/connect/write/read consumption, expired
   pre-connect/pre-write, partial TCP writes, deadline races, late collection,
   and wall-clock jumps. `dns_sent` requires a complete accepted frame; late
   collection cannot make a result on-time.
4. Decouple evidence persistence. Workers enqueue to a bounded,
   cancellation-aware writer with a separate bounded control path. Test writer
   errors and a sink that blocks forever without error. Exhaustion creates
   explicit rejected/skipped slots and `load_valid=false`; missing journal
   ranges force `evidence_valid=false`; cancellation has a bounded cleanup
   deadline and unreclaimable real writers surface `cleanup_failure`.
5. Build offline `reliability-assess` from raw evidence, never trusted
   summaries. Write only to a fresh derived directory and recompute accounting,
   planned-slot-to-finish, dispatch-to-finish, write-start-to-finish, samples,
   denominators, and goodput. Cover evidence/load/correctness/service axes,
   valid-load degradation, harness saturation, wrong DNS, overload/recovery,
   no-overload indeterminate, restart non-recovery, and fail-closed missing
   evidence. State-machine decisions are real, not mocked.
6. Close public durability boundaries with subprocess/CLI tests and a real
   loopback UDP/TCP peer for success, slow/refused/lost responses, wrong
   answers, process exit, and port rebind. Exercise repo-root markers,
   historical Git object/path, tampered/missing/ambiguous inputs, and fresh
   output refusal. Keep the new shell runner local/offline and run `bash -n`
   for both runners.

## G1 gate

Retain exact red/green commands and evidence, then run focused Go tests,
`go test -race ./tests/phase5a-baseline/cmd/phase5a-baseline`, CLI/loopback
integration tests, shell syntax checks, `task.py validate`, and `git diff
--check`. Existing legacy tests stay green. Hash the Slice 1 schema/CLI/test
evidence and submit only the Slice 1 diff to C2C. A G1 PASS ends this unit but
does not unlock Slice 2 while the G0 profiler blocker remains.
