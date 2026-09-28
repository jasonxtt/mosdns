# Execution plan

This is a review-only recovery task. Its one authorized unit covers the exact committed canary review and any documentation-only correction needed to address an in-scope reviewer finding.

## Slice 1 — exact committed-range acceptance review

- [ ] Recheck that this task is planning and has no run or authorization snapshot. Resolve the dedicated C2C reviewer binding against the configured workspace Project/connector identity; verify the platform-native transport for the bound chat. If verification is unavailable or mismatched, stop before `task.py start` and do not send a review request.
- [ ] Create and inspect the pre-start authorization snapshot for exactly `Slice 1`. Then run `task.py start` for this replacement task and activate from that snapshot. Preserve the original task status until final review PASS.
- [ ] Confirm branch `rust`, original base `c4a785fea6e66531921396def362d0c44d4a1666`, original canary boundary `879f53d283d2ef19cc87bcec59ba80d7f1e25cc2`, and the new committed correction head. Freeze one exact base-to-correction-head range and all changed paths.
- [ ] Record the submission SHAs, request kind, and verified reviewer identity in the run before sending. Submit one atomic `[C2C] MODE: REVIEW_ONLY` request through the verified transport. The request must include the exact SHAs, paths, validation summary, acceptance criteria, and forbidden scope, and must contain no diff or file body.
- [ ] Wait on the same conversation until a complete response arrives. Parse with the C2C review-only contract. Do not treat `DONE`, `PLAN`, pending output, or an iteration limit as acceptance.
- [ ] On `FINAL: PASS`, persist the result, complete only this authorized unit, then use the supported supersession transition for the original task. On a documentation-scoped FAIL, fix only that root cause, record the exact re-review range, and send a bounded re-review to the same reviewer. Stop on out-of-scope or product/execution findings.
- [ ] Confirm there was no VM command, project test, product-code edit, deployment, or production change. Preserve the original canary evidence and record its terminal `superseded` status only after PASS.

## Completion gate

This task's scope is accepted only when Slice 1 records `FINAL: PASS` for the exact submitted range (or an exact remediation range after a scoped finding), with no open findings. Do not finish/archive the original canary task or start follow-on roadmap work as part of this review task.
