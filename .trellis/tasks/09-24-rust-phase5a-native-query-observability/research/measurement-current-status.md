# Measurement state, 2026-09-26

## Current authorized step: M7

User approved simplifying to100QPS30s, old/new-off/new-on three times each.
The bounded W1 regression now uses a predeclared paired median10% latency
screen, without another self-control calibration loop. See
measurement-revision-v7.md. M2–M6 and higher-load V12 failures remain unchanged;
full A5/capacity/W2/W3 acceptance stays closed. No-query two-host preflight
passed and51 focused harness tests passed (one Linux-only skip on macOS).
Prospective review is required before the fixed nine actual runs.

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
