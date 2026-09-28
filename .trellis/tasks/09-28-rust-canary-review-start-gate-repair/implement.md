# Execution plan

## Step 1 — preserve evidence and history

- [x] Recheck attempt-3 source hash and independently validate its fields and oracles.
- [x] Save sanitized durable evidence; update original and replacement task artifacts with exact reviewer chronology, missing snapshot, evidence link, and limits.

## Step 2 — guard automation starts

- [x] Write failing CLI tests for missing, wrong-task, reviewer-drift, and valid snapshots; assert failed start changes neither status nor active pointer. Cover ordinary-task start.
- [x] Implement the shared pre-start check, set `automation_required=true` on the replacement task, and update workflow/spec guidance.
- [x] Run focused tests and inspect the diff.

## Step 3 — add honest supersession

- [x] Write failing CLI tests for accepted and unaccepted replacement, missing successor, repeated transition, and pointer cleanup.
- [x] Implement terminal `superseded` status and accurate list/context reporting. Do not transition the original task before review PASS.
- [x] Run the Trellis test suite, task validation, and `git diff --check`.

## Step 4 — review and closeout

- [ ] Create a genuine review-only snapshot while the existing replacement task is planning; start and activate it, then send an atomic exact-range request to the user's selected C2C conversation.
- [ ] Record explicit `FINAL: PASS` or findings. Correct and re-review only in-scope documentation findings.
- [ ] After PASS, supersede the original task and verify the non-success terminal state. Leave 5B/5C decisions in their own tasks.

## Limits

Do not rerun canary, contact the VM, rebuild binaries, edit product code, deploy, or push. Preserve unrelated dirty files and keep auto-commit disabled.
