# G2 remediation evidence — official reliability path

The first G2 review returned `FINAL: FAIL` with two P1 findings: the frozen
official runner still called legacy `run`, and the manifest's safety contract
was not executable. This remediation is prepared for the next atomic C2C
review. A preserved pre-record-budget official-r1 attempt did start the Go
candidate, but it is invalid evidence and was stopped before the
near-saturation stage could run. No Rust candidate or profile was started.

## P1 closure

1. `scripts/run-phase5a-baseline.sh` now invokes `reliability-run` for every
   measured stage and immediately invokes `reliability-assess` with evidence,
   load, and strict-correctness requirements. The raw bundle and assessment
   remain in fresh per-stage directories. The helper emits compatibility
   `stages.jsonl` and request-ledger projections only after the reliability
   raw bundle is complete; the raw reliability records remain authoritative.

2. The official manifest's `execution_contract` is decoded and validated,
   including the W1/W2 scope, pair/retry rules, profiler separation, recovery
   restriction, source-tree identity, and resource caps. Official execution
   fixes the reviewed worker/queue/in-flight/record/cleanup values, applies a
   512-FD process limit, checks per-role and combined RSS/FD samples, checks
   free disk and attempt size, and stops/preserves the attempt on any failed
   evidence, sender, correctness, resource, or process check. A failed stage
   cannot advance the ladder.

## Verification and preserved invalid attempt

- Local package, race, vet, shell syntax, and `git diff --check` gates pass.
- Remote Linux smoke of the new stage path produced raw reliability evidence,
  a separate assessment, resource samples, stage projection, and request
  ledger under the task-owned `results-g2-smoke-*` roots.
- Remote `verify-manifest` passed for Go/Rust × W1-TCP/W2 using the current
  runner, helper, binaries, environment, and manifest. This command only
  hashes and validates artifacts; it does not start a candidate.
- Current manifest SHA-256:
  `de0836c1618ec3acd016e7f7702f2b762a2ea3dccfd3fd616e9856d92bdb2618`.
- Current runner SHA-256:
  `c884a0912496406381aaf7d7d173b375d82baf7ff2a680e386e4f8d3345b6f3b`.
- Current Linux helper SHA-256 (record-budget revision):
  `b5bba12ab6407c93371359ecc711a58ece54cb4557619d44574d93174ab179f2`.
- The preserved `slice3-official/official-r1-20260927-w1-tcp-go` attempt used
  the pre-record-budget identities: manifest
  `5878de4fcf33b5e737719974e36450fba7eba48f25f9d20058b57fd119084b0a`,
  runner
  `5e08f69cccd1aa517df3e1b1c42512097af1a1c891ebdf10bff385d468711554`,
  helper source
  `7ca51c4a7584bf20f027ebf9013a6dc28ba2f9d3063f4eca7bf2f0f9d8a219d8`,
  and Linux helper binary
  `40c95259d3059c1b3f8fcba2ba6b5289cb85aada2ba5a8432debbe9afff99f4c`.

The invalid Go r1 attempt completed the 200-QPS normal-reference and 300-QPS
common-load stages (600 and 900 planned/sent/correct slots respectively).
At the 350-QPS near-saturation stage, the old 65536-byte record setting
exceeded the helper's 64-MiB control budget, so the helper rejected that stage
before any further candidate traffic. Its raw bundle, old identities, and
`invalid-stages.tsv` are retained as invalid evidence; they are not included
in any pilot, official pair, profile, or capacity conclusion. The current
record-budget revision is the distinct identity set listed above and below.

The follow-up review also requires W2 cold-session and independent-prefilled
correctness failures to stop immediately; the runner now records the invalid
stage and returns before advancing to warm or the next warm point. A valid
official matrix remains gated on `FINAL: PASS`; the preserved r1 attempt is
not a valid matrix result. The official runner now fixes
`record_bytes=32768` and the manifest validator checks that value, preventing
the 350/400-QPS stages from exceeding the helper's 64-MiB control budget.
