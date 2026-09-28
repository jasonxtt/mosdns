# Restart committed canary review

## Goal

Obtain the missing exact-commit acceptance review for the already completed `mosdns-rust` sidecar canary, using the user's selected C2C conversation and the existing evidence. The original task was started without a real pre-start authorization snapshot, and Trellis has no supported recovery command for that state. Do not fabricate or backdate one.

## Requirements

- Submit one exact committed range from `c4a785fea6e66531921396def362d0c44d4a1666` through the new correction head. Identify `879f53d283d2ef19cc87bcec59ba80d7f1e25cc2` as the original canary boundary within it; verify and list every changed path before submission.
- Reuse canary attempt 3 and the existing validation records. Do not rerun the VM canary or project tests.
- Bind to and verify the user's selected C2C Project conversation before creating a real pre-start authorization snapshot for `Slice 1`.
- Send one atomic `[C2C] MODE: REVIEW_ONLY` request with the complete SHAs, paths, validation summary, acceptance criteria, and forbidden scope. Include no diff or file body. The reviewer must inspect the exact range through read-only `git_compare`.
- Persist each submission's exact parent/head SHAs and reviewer identity before sending. Only an explicit final `FINAL: PASS` is acceptance; pending, partial, or malformed replies do not pass.
- If a scoped documentation finding is returned, make only that correction and submit an exact-parent re-review. Stop and report findings that require product-code changes, rerunning the canary, deployment, or broader authorization.
- Disclose the Codex reviewer used for the VM target-change preflight and the missing original pre-start snapshot. A later C2C PASS is not retroactive pre-execution approval.
- After replacement review PASS, use the supported terminal supersession transition on the original task. Do not archive it or mark it completed.

## Acceptance Criteria

- [ ] A genuine pre-start snapshot for only `Slice 1` records the verified reviewer and transport evidence before `task.py start`.
- [ ] The user-selected C2C reviewer returns explicit `FINAL: PASS` for the exact submitted range, scoped paths, and durable evidence.
- [ ] Submission and result are recorded in Trellis with the exact SHAs and target; any scoped findings are closed by an approved re-review.
- [ ] The old task is terminal `superseded` only after review PASS, retaining its evidence and reason without being archived as complete.
- [ ] No canary rerun, project test run, product-code edit, deployment, or production change occurs in this task.

## Notes

- Original task: `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary`.
- Original task remains `in_progress` until the reviewed replacement task passes and the supersession transition runs.
- Existing dirty worktree changes are unrelated and must remain untouched.
