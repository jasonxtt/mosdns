# Repair canary review and Trellis start gate

## Goal

Make the completed `mosdns-rust` sidecar canary independently reviewable, resolve its incomplete review lifecycle without rewriting history, and prevent automation tasks from starting without genuine pre-start authorization.

## Background

- The original canary task entered `in_progress` without `automation.py authorize`; later activation correctly rejected the absent snapshot. The historical snapshot must not be fabricated.
- Its VM target-change preflight used Codex reviewer `01a0d43d-d0aa-7401-af0f-2ca3a45ba519`, while the user requested one C2C Project conversation. That conversation later reviewed worktree documents, but the committed range `c4a785fea6e66531921396def362d0c44d4a1666..879f53d283d2ef19cc87bcec59ba80d7f1e25cc2` still lacks final review.
- Attempt-3 raw output remains in a local temporary file, SHA-256 `5c519eea2974c28a452c1cf718e810d3d75d8e1e28579663d1f85e6ed7880d48`. The existing replacement review task is still planning. Unrelated dirty files must be preserved.

## Requirements

1. Independently validate and durably preserve sanitized attempt-3 controller evidence: source identity, twelve query and counter cases, unchanged service baseline, process receipts, and cleanup. Keep evidence limits and unrun work explicit. No canary rerun solely for review repair.
2. Record the actual reviewer sequence and missing historical snapshot in the original and replacement tasks. A later PASS must not be called pre-execution approval.
3. Revise the existing replacement task to obtain a genuine review-only pre-start snapshot, then seek same-conversation C2C review of one exact committed range from before the original canary commit through the later evidence/record correction head. Name the original commit as an internal boundary. The submitted parent/head and path list must be exact. Keep its authorization separate from the original execution.
4. Add a supported terminal `superseded` task transition after replacement review PASS. The old task remains inspectable with successor and reason, without being archived or marked completed.
5. For explicitly automation-required tasks, make `task.py start` reject absent, wrong-task, invalid, or reviewer-mismatched authorization before it changes task status or active pointer. Ordinary tasks retain their start behavior. Cover both paths with CLI regressions.
6. Keep the user's selected C2C conversation as reviewer for the replacement task; do not carry the historical Codex reviewer forward.
7. Preserve unrelated dirty worktree changes and `session_auto_commit: false`. No product-code edit, deployment, production change, or push is part of this repair.

## Acceptance Criteria

- [x] Durable sanitized evidence matches the raw attempt-3 record and has a documented source hash.
- [x] The historical authorization gap and reviewer deviation are explicit, with no retroactive approval claim.
- [x] The replacement task records a genuine pre-start review authorization, an initial C2C `FINAL: FAIL`, and a scoped re-review `FINAL: PASS` for exact committed ranges.
- [x] After PASS, the old task is `superseded` with retained evidence, successor, and reason; it is not reported completed.
- [x] Missing or mismatched authorization leaves both status and active pointer unchanged; matching authorization succeeds; focused Trellis tests pass.
- [x] No VM canary rerun, unrelated overwrite, product runtime change, or production action occurs.

## Scope

This task owns evidence and workflow repair. The existing replacement task owns the final C2C review request and verdict. A PASS here does not itself start 5B or 5C.
