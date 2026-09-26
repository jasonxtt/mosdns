# M6 prospective review ledger

Report review parent `b664bade4e0b78b11305b2b70c9b86c76ea216ff`,
head `99798c129abf6884ed985859f0d9e74ec8c87f22`, same002reviewer.
Scope results/evidence/stop only; no extra run or source changes. Atomic
M6-REPORT-001 requested; pending explicit verdict.
FINAL: FAIL — M6-REPORT-001 at2026-09-26T08:02:16Z. F1: report count unit
should distinguish affected windows from missed requests. Corrected to3
windows/8 missed requests at200QPS and9 windows/32 missed requests at400QPS.
All other counts/manifests/stop decision independently verified by reviewer.
Findings1, failed remediation rounds0; no raw data, threshold or traffic change.

Attempt1 exact parent `b1ab386184f8f8c6c5f864370aad08817d1292e2`,
head `b664bade4e0b78b11305b2b70c9b86c76ea216ff`, pushed origin/rust;
same002reviewer. Atomic M6-UNIT1-001 sent for readiness only.
FINAL: PASS — M6-UNIT1-001 at2026-09-26T07:37:41Z, readiness only. The
single fixed M6 W1 run is authorized. Nonblocking note: raw ip route inventory
contains trailing spaces, preserved as captured data. No W2/W3/candidate.

Unit2 finished07:54:50UTC:18 attempts,8 exit0/10 exit1,36 windows/16valid,
269960 correct actual queries,40 sender shortfall. Startup fix effective in
all18 sessions; controls UNQUALIFIED. All764 full manifest entries and36
remote session source manifests reverified;748 selected files retained.
No exclusions/reruns/W2/W3/candidate. Report and stop review pending.

User authorized next corrective step after M5 report. Executor=current inline,
reviewer002reviewer01a0d43d-d0aa-7401-af0f-2ca3a45ba519; native transport
verified. Unit1 only, initial findings0. Parent
`b1ab386184f8f8c6c5f864370aad08817d1292e2`; head recorded before atomic send.
Validation: realLinux TIME_WAIT RED errno98 then GREEN after reuse correction;
active listener still refuses; early failure records persist; remote stderr
and disjoint generation test. Local49 tests (one Linux skip), Linux5 M6 tests,
task context/diff check, zero-traffic fresh preflight PASS. Review pending.
