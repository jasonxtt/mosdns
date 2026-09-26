# M5 prospective review ledger

Report review parent `49ac145ca1345860a92f60682b46c3a127cf7273`,
head `07d93598cd44dfd4bd3412dbd455a7e2cb501b0c`, same002reviewer.
Scope evidence/report/stop only; no readiness or next-run authorization.
FINAL: PASS — M5-REPORT-001 at2026-09-26T07:26:03Z. Reviewer independently
verified full175-entry durable manifest,176 selected files, source manifests,
18-slot accounting and qualified TIME_WAIT diagnosis. No further run.

Attempt2 parent `b93822b8532a4c0392f64a7ad6c804aab7aabb7b`,
head `49ac145ca1345860a92f60682b46c3a127cf7273`, reviewer same002reviewer.
Scoped F1/F2 remediation only;44 local Python tests/7 Linux M5 tests and
fresh zero-attempt preflight PASS. Submitted for explicit final re-review.
FINAL: PASS — M5-UNIT1-002 at2026-09-26T07:20:18Z; F1/F2 closed.
Only the single fixed M5 W1 calibration is authorized. W2/W3, candidate/A5
acceptance remain closed. Unit2 uses reviewedHEAD49ac145 and retains all data.

Unit2 finished18 slots once:1 success,17 startup failures,2 valid windows,
15000 correct on time. Invalid matrix; no control stability verdict. Complete
175-entry local manifest verified against durable copy;176 selected files
retained. No W2/W3/candidate or rerun. See m5-calibration-assessment.md.

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
