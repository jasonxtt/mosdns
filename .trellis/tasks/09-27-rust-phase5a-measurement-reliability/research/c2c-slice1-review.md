# C2C Slice 1 G1 review and remediation

## Iteration 5 review

The C2C review returned `STATE: PLAN` and `FINAL: FAIL`. All findings were
declared G1-blocking:

- P1-1: real TCP/UDP transport reused a pre-connect relative budget and did
  not count the TCP length prefix in `FrameBytes`; partial TCP writes could be
  marked `dns_sent`.
- P1-2: overload/recovery consumed caller-supplied derived window summaries
  instead of recomputing p95, timeout rate, validity, and health from raw slot
  membership/evidence.
- P1-3: evidence enqueue failure did not stop later offered load; control
  records and record-size enforcement were not fully bounded, and CLI sink
  close could re-enter a blocked write.
- P2-2: latency views omitted failure denominators and assessment omitted
  raw-derived goodput.
- P2-3: the Slice 1 hash index was stale after the reviewed implementation
  changed; it must be regenerated only after the remediation diff is final.
- P2-4: tests called command functions directly and did not exercise `main`
  subprocess exit/dispatch or the public shell runner.

## Remediation applied

`reliability_net.go` now recomputes the service remainder after connect and
before every write iteration, refuses an expired write, and counts the full
TCP length-prefixed frame. Controlled slow-connect and partial-write tests
cover the transport boundary.

Raw `reliabilityWindow` now contains only window ID, raw slot membership, and
process identity. `reliability-assess` derives all window metrics from the
selected control/evidence slots; forged derived JSON fields are ignored and a
tamper test proves they cannot create recovery.

The runner has a run-level evidence-failure latch, explicit rejected slots for
the remaining plan, a bounded control byte budget, enforced record-size
limits, and a non-blocking atomic sink close path. An end-to-end writer-error
test proves no later exchanges occur and slot accounting remains closed.

Latency views now expose eligible samples, failure denominators, and
ineligible counts. Assessment also writes the raw-derived correct-on-time
count, offered stage duration, and goodput. A built-helper subprocess test
exercises both public commands and the local shell runner against a loopback
peer.

Iteration 6 is the compact finding restatement; a new C2C EXECUTED review is
required after final hash regeneration and the complete validation gate.

## Iteration 7 re-review

C2C returned `STATE: PLAN` with the following remaining G1 blockers:

- P1-1: transport evidence had a write-start field but no write-complete
  offset, and the write boundary did not explicitly preserve the complete
  write/deadline race. Slow-connect and full-write-crosses-service-deadline
  coverage were required; a raced complete write must never be on-time.
- P1-2: raw window derivation still lacked a frozen on-time-rate floor,
  consecutive-window enforcement, required PID/start identity, and a raw
  resource-return-to-budget fact. Criteria could not be silently defaulted
  before G2.
- P1-3: the evidence owner needed an internal cancellation context and an
  end-to-end permanently blocked-sink run. Oversize records were being
  rewritten as false `harness_rejected` outcomes, and CLI sink-close failure
  could prevent raw evidence from being written. Control-budget validation
  also needed overflow-safe arithmetic.
- P2-2: latency percentile values included timeout/transport/pre-send
  records instead of only successful correct responses, while failures still
  needed to remain in the denominator.
- P2-5: the shell runner accepted `w1-udp`/`w1-tcp` even though scenario and
  transport are separate public fields.

## Iteration 7 remediation

`reliability_net.go` now returns explicit write-start/write-complete presence
bits and offsets, counts the TCP length-prefixed frame, preserves actual bytes
and terminal state, and classifies a complete write crossing the service
deadline as late/degraded. Tests cover slow connect, partial write, and a
complete write race.

Window criteria now require explicit p95, timeout-rate, on-time-rate,
overload-window, and recovery-window values whenever raw windows are present.
Window membership derives on-time rate and timeout rate from raw slots,
requires non-empty PID/start identity and an explicit resource budget fact,
checks identity continuity through overload/recovery, and refuses recovery
when resources have not returned to budget.

The evidence writer owns a cancellable context and cancels it when the bounded
cleanup wait expires. Oversize control records are compacted without changing
their actual started/DNS-sent/terminal facts; the slot is marked missing from
the evidence journal instead of being rewritten as a harness rejection.
Control-size validation is division-based, the blocked-sink run test completes
with `cleanup_failure` without releasing the sink first, and the CLI writes
`reliability-raw.json` before returning a sink-close error.

Latency views now admit only correct on-time/late responses while retaining all
planned records in the failure denominator. The shell runner accepts only
`w1|w2|w3` and documents the separate `TRANSPORT` field; its alias rejection
is covered through the built-helper shell subprocess test.

The final local gate is green for the focused package and race detector. A
fresh Slice 1 hash index and a new C2C `EXECUTED` review are required before
marking G1 complete. The Slice 0 profiler blocker remains unchanged and still
freezes Slice 2+.

## Iteration 8 re-review

C2C returned `STATE: PLAN` with four remaining blockers:

- P1-1: classification changed `ResponseAt` to a synthetic
  `service_deadline+1ns` value for a write race, violating factual finish
  offsets. The race must affect the terminal classification only.
- P1-2: raw windows lacked enough sequence facts to prove contiguous,
  equal-length windows and return to the frozen reference QPS. PID/start,
  rate, resource, and on-time criteria were present, but arbitrary slot groups
  could still be labeled overload/recovery.
- P1-3: after a non-error queue overflow, later closed slots were not all
  listed in `MissingSlotIDs`; reconciliation ran only for writer errors.
- P2-6: the approved wall-clock-jump case was still absent because the fake
  clock tied wall time directly to monotonic time.

## Iteration 8 remediation

`classifyReliabilityExchange` now preserves the observed response timestamp and
uses `WriteDeadlineRace` only to select the late terminal; offline service
degradation also re-derives a race from actual write-complete and planned/
deadline offsets. A transport-level partial TCP writer test checks full frame
bytes, write offsets, DNS-sent, and raced completion.

Raw windows now carry start/end offsets, phase, and offered QPS. Assessment
requires contiguous equal-length boundaries covering every control slot,
monotonic reference→overload→recovery phase ordering, overload QPS above the
explicit frozen reference QPS, and recovery QPS exactly at that reference.
The recovery state machine uses those raw sequence facts rather than caller
labels alone.

Missing-slot reconciliation now runs for every persistence outcome, including
non-error queue overflow, and the blocked complete-run test asserts the missing
journal set is nonempty while later exchanges stop. The fake clock now has
independent wall and monotonic components; a wall-clock jump test proves
deadline and late classification remain monotonic.

The focused package and race gates are green after these changes. The Slice 1
hash indexes must be regenerated again, then a new C2C `EXECUTED` review must
decide G1. The G0 profiler blocker remains sticky.

## Iteration 9 re-review

C2C returned one remaining G1 blocker:

- P1-2: the raw sequence validator required overload and recovery ordering and
  recovery QPS, but allowed the sequence to start directly at overload. After
  overload, `reliabilityRecoveryState` also counted any healthy same-process
  window, including a healthy window still labelled `overload`, toward the
  recovery-window count.

This could declare recovery before the workload returned to the frozen
reference stage. The G0 process-directed-profiler blocker remains sticky and
independently prevents Slice 2+.

## Iteration 9 remediation

`validateReliabilityWindowSequence` now requires the first window to be a
`reference` phase at the explicit frozen reference QPS, and every reference
window is checked against that rate. Once overload has been observed,
`reliabilityRecoveryState` only counts healthy windows whose raw phase is
`recovery` and whose offered QPS equals the frozen reference QPS; healthy
overload windows reset the recovery streak and cannot produce `recovered`.

Added negative coverage for a missing or wrong-QPS reference phase and for
three healthy overload-phase windows after objective overload. Focused tests
are green; regenerate the evidence index and submit the next C2C `EXECUTED`
review before checking G1. Slice 2+ remains frozen by G0.

## Iteration 10 final review

C2C returned `STATE: DONE` and `FINAL: PASS`. It independently accepted the
current Slice 1 boundary and closed P1-1, P1-2, P1-3, P2-2, P2-3, P2-4,
P2-5, and P2-6. The Slice 1 evidence index matches the changed artifacts;
the public subprocess/shell boundaries, factual deadline/write-race timing,
raw-derived window sequence, bounded persistence failure, latency denominators,
scenario/transport contract, and wall-clock independence were all accepted.

This closes G1/Slice 1 only. The G0 absence of an acceptable process-directed
profiler remains a sticky blocker and takes precedence over generic later-stage
wording: Slice 2 calibration, pilot, official Go/Rust measurements, profiling,
hotspot attribution, and capacity claims remain unauthorized.
