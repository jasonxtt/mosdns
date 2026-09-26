# Measurement state, 2026-09-26

The user authorized correction/review, qualified stable controls, then
resumption of acceptance. Corrections and prospective reviews are complete;
stable controls have **not** been established. Acceptance must stay closed.

| Revision | Fixed attempts | Valid primary | Correct on time | Qualified latency intervals | Control result |
|---|---:|---:|---:|---:|---|
| M2: v9 no sampler subprocess, pinned scheduler |54|126|108000|1/28|Unqualified|
| M3:25-second primary windows, TTL-checked warm points |18|36|270000|1/8|Unqualified; W1 stop|
| M4:bounded helper GC-off experiment |18|36|270000|1/8|Unqualified; W1 stop|

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
