# M7 review ledger

Selected reviewer002reviewer, native task01a0d43d-d0aa-7401-af0f-2ca3a45ba519.
Executor inline. User-authorized simplified scope is measurement-revision-v7.md.

M7-UNIT1-001: parent3260d7ad21afd89483472bd784107b3db260719d,
head d3ba9decd6a86aab87f78ca9d24dbf5572242f20. Sent one atomic request;
turn01a0dccb-14e2-79e1-b8c0-eae256041d56 completed08:25:02UTC.
FINAL: FAIL — M7-UNIT1-001-F1, P1 ambiguous SSH startup skipped cleanup.
No traffic. Scoped remedy moves cleanup guard before attempted start, so
fresh owned PID/start records are consulted even on a lost SSH response.
Transport-error-after-launch regression failed before fix and passes after.
52focused tests pass with1Linux-only skip. Refreshed no-query preflight
matches the remedied driver/protocol hashes, rows[]; no hardware changes.
Re-review covers this same unit only, no expanded scope or traffic yet.

M7-UNIT1-002: parent d3ba9decd6a86aab87f78ca9d24dbf5572242f20,
head ba0f4a890e96adf2210d70576bb1c4c9b82a809d. Atomic re-review turn
01a0dcd2-9281-7381-b4c5-759c0923f31c completed08:28:41UTC.
FINAL: PASS; F1 closed. Reviewer verified attempted-start cleanup,
owned PID/start stop, failed-cleanup evidence, regression and no-query hashes.
PASS authorizes only the single frozen nine-run M7 W1 screen followed by
consolidated evidence review, not fullA5. Execution uses this exact HEAD.
