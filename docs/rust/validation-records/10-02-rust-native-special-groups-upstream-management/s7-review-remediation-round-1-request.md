# S7 P2-1 remediation review request — round 1

**State:** sent on 2026-10-04; explicit `FINAL: PASS` received.

This re-review keeps the same finding ID and exact unit. Original reviewed head
`59b1612ad3629150cee68a68c8e0d19a7ed5968d` is the remediation parent; new
candidate is `3eaeabdd75f74c4eb86db96b46f466afbe4aa3be`, tree
`91f40762e7836b16e564f7292488d18a7b3fc622`. The change removes the duplicated
digest block in `s7-status.md`, retains the identity matching the committed
`s7-evidence-manifest.json`, and records the original request/result.

Validation is record-scoped: 527 source inputs and 38 evidence artifacts still
match the candidate tree byte-for-byte; the corrected status names the evidence
map SHA-256 `d1e65b0e06e2fbcef282aca9c805f7693e82e90eee261df9f43de1ed9572437f`
and manifest-file SHA-256
`113a4d870fa4b111b06f894a2ca6cbe60986ee17e29fe0e6c61ef144c89e2274` exactly
once. Changed files pass `git diff --check`. Runtime/test source is unchanged
from the S7 candidate that passed the listed isolated validation.

## Atomic C2C request

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: 10-02-rust-native-special-groups-upstream-management
UNIT: Slice 7 P2-1 rereview
BASE_SHA: 59b1612ad3629150cee68a68c8e0d19a7ed5968d
HEAD_SHA: 3eaeabdd75f74c4eb86db96b46f466afbe4aa3be
TREE_SHA: 91f40762e7836b16e564f7292488d18a7b3fc622
PATHS: exact remediation range; corrected status and round-1 records.
PRIOR: P2-1 open: conflicting digests for one evidence manifest.
FIX: Removed duplicate; kept map SHA256 d1e65b0e06e2fbcef282aca9c805f7693e82e90eee261df9f43de1ed9572437f and file SHA256 113a4d870fa4b111b06f894a2ca6cbe60986ee17e29fe0e6c61ef144c89e2274.
CHECK: 527/527 source and 38/38 evidence hashes match the candidate; diff check passes. Runtime/test source unchanged from S7.
REVIEW: git_compare this exact range; resolve the same P2-1 only. No edits, executions, or Trellis writes. Return exactly one final line: FINAL: PASS or FINAL: FAIL.
```
