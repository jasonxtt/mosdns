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
