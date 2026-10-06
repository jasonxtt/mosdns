# Cumulative P1-1 persistence exact-source re-review

Parent is the failed remediation object; BASE→HEAD is the entire task.
Request/result records stay outside the object to avoid self-reference.

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: 10-02-rust-native-special-groups-upstream-management
UNIT: Final cumulative re-review, same P1-1 round2
BASE_SHA: 79d93ae1b3b3253a2d09563444b251aad18eb5df
REMEDIATION_PARENT: ccc147bb35aab2301633325c563368ac8457dbba
HEAD_SHA: ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc
TREE_SHA: 1575272a271e02ca9b7bd5051b651e9d63f471cf
FIX: Origin survives native snapshots/v2 optional field7 save/restart; legacy origin stays unknown. Existing atomic dump/gates own it.
PROOF: 527 source hashes match tree/local/isolated; 115 artifacts. Workspace1184/0/3 parent probes; native381/libs410; fmt/strict Clippy/build pass. Restart RED->GREEN; real save/exit/new process3 hits/zero new attempts or peer requests. Go reader3/3 pass.
RECORD: public cumulative-status.md, cumulative-p1-1-persistence-remediation.md.
CHECK: P1-1 and whole BASE->HEAD frozen S1-S7, ECS/lazy/durable/owners. git_compare exact; no edits/execution/Trellis writes. End FINAL: PASS or FINAL: FAIL.
```
