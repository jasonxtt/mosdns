# M2 review finding ledger

## Attempt 1

Target: `codex://threads/01a0d43d-d0aa-7401-af0f-2ca3a45ba519`.
Range: `4f30cc7fe5a1e09e1ee7a4cbd92bb93e0cb3a124` →
`27cfc20d3bd27ba90ec67454ab343978d0cb7fed`.
Verdict: **FINAL: FAIL**. Initial finding count: 0; first scoped remediation round.

Native `read_thread`/`wait_threads` reported a completed turn without visible
items. The same platform's persisted rollout contains the explicit final
AgentMessage and task_complete for turn
`01a0db6c-cf48-7d11-9f37-898a02bb3d02` at 2026-09-26 01:59:29 UTC. This is a
retrieval limitation, not a missing verdict or permission to retry transport.

- **P1-1 [open pending re-review]:** protocol permits individual old-guard
  crossings although request says no old crossings. Remediation requires
  `pairs_above_guard == 0` for every p95/p99 assessment in both batches;
  equivalence intervals are an additional gate, never a substitute. The
  qualifier has a regression case where a single crossing blocks even when
  all intervals are exact zero.
- **P2-1 [open pending re-review]:** GOMAXPROCS=1 is neither enforced by the
  runner nor recorded in standard per-run environment evidence. Remediation
  adds `PHASE5A_MEASUREMENT_PROFILE=m2`, rejects missing/wrong GOMAXPROCS,
  non-Rust/non-pilot use, and v8 helper under M2; records measurement_profile
  and gomaxprocs_environment in every environment.txt. The calibration driver
  pins this profile, and qualification checks it for every attempt. Red tests
  against the submitted parent runner reproduced missing enforcement/evidence;
  green helper tests pass with the corrected runner.

No control data were collected before re-review. Findings may close only
after the explicit reviewer result. Existing V12 verdict remains unchanged.

## Attempt 2 — scoped remediation round 1

Range: `27cfc20d3bd27ba90ec67454ab343978d0cb7fed` →
`9932a76aa0c5b3a55bdda71780b97f5d77e84866` (pushed).
Same target. Native final for turn `01a0db75-9c65-70d1-8540-7b4b7d1dd1d1`
completed at 2026-09-26 02:10:30 UTC with **FINAL: PASS**.

- **P1-1 [closed]:** both batches require zero individual p95/p99 guard
  crossings; reviewer verified the exact-zero-interval/single-crossing test.
- **P2-1 [closed]:** reviewer verified profile enforcement before traffic,
  standard per-run environment recording, and per-attempt qualifier validation.

Only unit 2 fixed M2 calibration may advance. Candidate/A5 acceptance remains
gated on qualified controls and a separately reviewed prospective supplement.
