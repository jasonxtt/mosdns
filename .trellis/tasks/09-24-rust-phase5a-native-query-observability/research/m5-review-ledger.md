# M5 prospective review ledger

Executor=current inline. Reviewer=Codex002reviewer,
01a0d43d-d0aa-7401-af0f-2ca3a45ba519. Unit1 only, initial finding count0.

Attempt1 parent `0d4a172945f1127f6725461f6c8fa12a3f8a841a`,
head `b93822b8532a4c0392f64a7ad6c804aab7aabb7b`, pushed origin/rust.
Validation:43 Python tests; macOS/Linux Go helper tests; Go vet; Linux6 M5
tests; diff check; task context validation; read-only two-host preflight with
zero measured attempts. FINAL: FAIL at2026-09-26T07:17:28Z. No official traffic.

F1 P1: first sampler PID identity not tied to owned.json. Remediation passes
owned starts to initial/every sample and independently compares both merged
windows to owned.json during qualification. Fake preinitial PID-reuse test
rejects before ready. F2 P2: copies lacked remote source hash comparison.
Remediation computes source-manifest.json on each endpoint after process stop,
verifies exact transferred file set/hashes and host, and reruns verification
during qualification. Tampered-copy test fails. Merged oracle input is kept
outside immutable raw server tree. Both regressions RED before fixes, GREEN
afterwards. Finding count2, failed remediation rounds0. Re-review pending.
