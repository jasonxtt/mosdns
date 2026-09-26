# Measurement state, 2026-09-26

## Current step: M10 final bounded acceptance PASS

Separately authorized nine W3 sessions completed once:27000 correct queries,
45000 ordered route events verified offline, zero DNS errors/shortfall. Four
unchanged paired median p95/p99 gates pass. Original driver FAIL is preserved:
its route oracle assumes shared clocks/barriers; one cleanup logged ESRCH.
Independent unique-ID route proof and all36 owned-process exit receipts pass.
Offline oracle/controller repairs have70 local regressions passing(oneLinux
skip). Linux/W1/W2 evidence reused without traffic. M10-FINAL-001 explicitly approved
this validation repair and bounded A5/A6; no lifecycle/production/extra traffic authorization.
See m10-w3-assessment.md and m10-review-ledger.md.

## M9 historical state

User authorized unify remaining criteria, one Linux regression, W2/W3 supplement
and one final review. M9 preparation review passed12:52:56UTC. Linux workspace
869 tests/strict Clippy/rustfmt and helper race/vet pass. All9 W2 sessions pass:
22500 warm +18 cold queries correct, zero errors/shortfall; paired medians<=1.10.
W3 controller incorrectly passed route_a/b/c instead of helper route-a/b/c.
Four completed W3 sessions each returned3000 wrong responses; fifth was
interrupted and four remaining slots never started. Major harness issue stops
this single batch without replacement. W3 performance/A5 remain closed;
fixture ID repair and direct helper-contract regression test are green offline,
not staged or measured. Final A1–A6 review M9-FINAL-001 returned FAIL13:12:13UTC
for missing valid W3/A5 evidence; A1–A4/W2 and archive/cleanup supported.
No production or lifecycle
closure. See m9-final-assessment.md and m9-review-ledger.md.

## M8 historical state

User authorized rebuilding and repeating the same simplified nine-run plan
after reviewed source changes. Native/helper rebuilt from18d71c8c with fresh
M8roots and pinned hashes, no-query two-host preflight passed.55harness tests
pass(oneLinux-only skip on macOS). Prospective review pending; noM8traffic.
See measurement-revision-v8.md,m8-build-identity.json,m8-preflight.json.

M8 subsequently received prospectivePASS09:56:21UTC and completed its fixed
9run batch. All27000planned queries sent/received/correct,zero shortfall or
responseerrors/timeouts. Pairedmedian newon/off p951.0588,p991.0579;
newoff/old p951.0178,p990.9074. M8 passed its bounded100QPS W1screen;
first-round audit-on p99ratio1.2715 remains disclosed. Consolidated result
review M8-REPORT-001 passed10:06:16UTC for this limited result/evidence/stop;
no broader acceptance. See m8-regression-assessment.md.

## M7 and remediation history

Post-M7 source-cost remediation is complete: exact-sized audit qname rendering
and64KiB buffered helper ledger writes with flush/error preservation.
POST-M7-CODE-001 review PASS09:37:53UTC confirms code only. See
post-m7-remediation.md/post-m7-review-ledger.md; no new measured evidence,
M7 performance failure unchanged. Revised sources need rebuilt pinned binaries
and a separately frozen performance validation before any new claims.

User approved simplifying to100QPS30s, old/new-off/new-on three times each.
The bounded W1 regression now uses a predeclared paired median10% latency
screen, without another self-control calibration loop. See
measurement-revision-v7.md. M2–M6 and higher-load V12 failures remain unchanged;
full A5/capacity/W2/W3 acceptance stays closed. No-query two-host preflight
passed and51 focused harness tests passed (one Linux-only skip on macOS).
Prospective M7-UNIT1-002 review passed08:28:41UTC; its nine-run batch then
completed.26997actual queries all correct,3sender slots missed in2runs,
zero response errors/timeouts. Audit-on diagnostic paired median p99ratio
1.1101 exceeds1.10. M7 NOT PASSED; no reruns. Complete report and retained
evidence are m7-regression-assessment.md and m7-w1-results. Consolidated
result review M7-REPORT-001 passed08:40:43UTC for evidence/stop only;
M7 remains NOT PASSED and full acceptance remains closed.

## Historical states (authorization at each prior boundary)

The user authorized correction/review, qualified stable controls, then
resumption of acceptance. M2–M4 corrections and prospective reviews are complete;
stable controls have **not** been established. Acceptance must stay closed.

M5 used10.0.0.50 as client and mosdns-rust as server after prospective review
PASS. Its one fixed18-slot run ended with1 successful attempt/2 valid windows/
15000 correct queries;17 slots failed before startup. The availability probe
mistook fixture-port TIME_WAIT for an occupied listener. The matrix is invalid,
not qualified; no latency stability conclusion. See m5-calibration-assessment.md.
No extra hardware/resource allocation, resampling, W2/W3 or candidate traffic.
Evidence and stop review M5-REPORT-001 passed at07:26:03UTC; it does not
qualify controls or authorize a further run. Startup-check correction and a
separately frozen/reviewed next protocol are still required.

User authorized the next step; M6 startup correction and disjoint generation
were prepared. Five Linux M6 tests and local49-test measurement suite passed
(Linux-only TIME_WAIT case is skipped on macOS and passes on Linux). Fresh
two-host read-only preflight passes with zero attempt ledgers. M6 prospective
review was required before its one fixed18-attempt W1 run. This paragraph
records its zero-traffic preflight state, before the run reported below.

M6 subsequently received prospective PASS and completed its one fixed18-slot
run07:54:50UTC: all sessions start correctly, but8/18 attempts pass and16/36
windows are valid.269960 actual queries are correct on time;40 planned slots
are dropped by the sender. Controls remain UNQUALIFIED; the mechanically
computed8 intervals include invalid load and are diagnostic only (0/8 inside
margin). See m6-calibration-assessment.md; acceptance stays closed.
M6-REPORT-002 passed08:03:31UTC for evidence and stop only, after clarifying
affected-window vs missed-request count units. No further run authorized.

| Revision | Fixed attempts | Valid primary | Correct on time | Qualified latency intervals | Control result |
|---|---:|---:|---:|---:|---|
| M2: v9 no sampler subprocess, pinned scheduler |54|126|108000|1/28|Unqualified|
| M3:25-second primary windows, TTL-checked warm points |18|36|270000|1/8|Unqualified; W1 stop|
| M4:bounded helper GC-off experiment |18|36|270000|1/8|Unqualified; W1 stop|
| M5:separate client/server |18 slots|2|15000|None: incomplete matrix|Invalid; W1 stop|
| M6:TCP startup correction |18|16/36|269960|No accepted six-valid-pair estimates|Unqualified; sender shortfall|

Each protocol received designated002reviewer PASS **before** fresh traffic.
Each ran once in a separate root; all attempts and unexecuted plans remain.
Practical equivalence margin stays10%; all individual old guards retained.
M4 observed zero GC traces and49.36MiB maximum sampled Go-role RSS, so this
intervention alone did not resolve the problem. Cause remains unisolated.

V12 source `eddcb48057096f1f8562d55b0bbc6290bff35756`, Linux executable SHA
`8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d`,
remains at its original failed performance verdict. No Rust runtime source
was changed by M2–M4; no new candidate acceptance data were collected.

See `m2-calibration-assessment.md`, `m3-calibration-assessment.md`,
`m4-calibration-assessment.md`, their adjacent selected raw/derived evidence,
and each review ledger. Complete raw remains on mosdns-rust and is hashed.
Final evidence/stop review `M4-REPORT-002` returned PASS at03:34:33 UTC,
verified all120 selected M4 files and the manifest sidecar, and confirmed
the unqualified control result and closed acceptance. This is report PASS,
never calibration/A5 PASS; it authorizes no resampling or acceptance.
Unit tests and shell checks cover profile enforcement, stage selection,
qualification failure paths and GC/resource evidence on macOS/Linux.

Next prerequisite: reachable exclusive Linux measurement host (or host-side
interference isolation with frozen evidence). mos-test connection checks
timed out; production aliases are excluded. User input requested for a host.
Inventory and pin any new host/tools/plan before a separately reviewed fresh
calibration. Only complete qualified controls allow a separately reviewed
V12 acceptance supplement with audit-on and contemporaneous controls.
No archive, deployment or A5 approval. Trellis status stays in_progress.

## Final bounded acceptance

002reviewer returned **FINAL: PASS — M10-FINAL-001** at 2026-09-26 14:03:57 UTC,
turn01a0de01-65a0-7132-8c7f-2a395d7e7cd5, exact reviewed parent
daeff167f16b4b4e816329de3e12c706cd03ccf5..head
909bb3fd56812045206b75e46482a01f9e6ee649. Reviewer independently
verified all9 unique-ID/question/ordered-path proofs,27000 requests/45000
events, all36 matching exited-owner receipts,349 bundle entries and341 raw
manifest entries. The separate proof and exit receipt support bounded A5/A6;
original driver FAIL and all9 runner_exit=1 remain unchanged. A1–A4 and
M8 W1/M9 W2/latest Linux evidence remain as previously reviewed. No reviewer
tests/traffic. All six criteria for this basic-observability subset are now
accepted; full Phase5A/C08/capacity/production are outside this verdict.
Task status remains in_progress; no archive, deployment or new task authorized.
