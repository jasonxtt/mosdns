# M7: simplified bounded real-host W1 regression

User approved 100 QPS, 30 seconds, old / new audit off / new audit on,
three repetitions each, one prospective review and one consolidated result
review. Executor is inline; reviewer stays 002reviewer native task
01a0d43d-d0aa-7401-af0f-2ca3a45ba519. No hardware allocation changes.

## Frozen scope and rule

This authorization replaces repeated identical-baseline calibration and CI
qualification for this bounded low-load W1 TCP regression only. It does not
qualify M2–M6, erase V12's higher-load failure, close full A5, or authorize
W2/W3, capacity claims, production deployment or task closure.

Exactly nine fresh sessions, ordered old/off/on, off/on/old, on/old/off.
Each sends 100 QPS for 30 seconds: 3000 queries, 27000 total. No retries,
excluded runs or supplemental batches. All scheduled/sent/received/correct
counts must equal 3000 in every run, with zero sender shortfall, late,
incorrect, protocol, transport and timeout counts. Existing response, sender
and fixture session-counter oracles must pass; failed startup/evidence is a
failed run. Every attempt and raw failure is retained.

For new-off versus old and new-on versus new-off, compute three within-round
p95 and p99 ratios. Their median must be <=1.10 for each comparison/metric.
This is a predeclared practical regression screen, not statistical equivalence
or a throughput limit. All latency values must be finite and positive.
CPU seconds over the sample bracket and peak sampled RSS are auxiliary only.
Failure ends this batch with a report; no automatic resampling or tuning.

## Fixed apparatus

Client10.0.0.50 Debian1vCPU/2GiB sends queries; server10.0.0.92
mosdns-rust2vCPU/4GiB runs all three versions on the same hardware. Different
physical hosts are not compared as SUTs. SUTCPU0, fixture/samplerCPU1;
clientCPU0. Existing helperv10, frozen W1 corpus, 500ms deadline/100ms drain,
GOMAXPROCS1/GOGCoff/GODEBUGgctrace1/GOMEMLIMITabsent for Go roles remain.
TCP listener10.0.0.92:15354 and fixture127.0.0.1:15454 are test-only.
Original YAML changes only listener bind and, for new-on, enable_audit=true.

Old SHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa;
V12 native SHA8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d;
helper SHA28d5faf5f5129aa990aac51efd8752eba655b852f0bcb27619b216e572c0450e.
No Rust/Go product changes. M7 selects actual old/candidate binaries and audit
configs, records their identities, and checks inputs before/after the batch.

M7 roots are measurement-v7/results-m7-server under existing server benchmark
BASE, /root/mosdns-phase5a-m7-client/{tools,results} on client, run IDs m7-rN-variant.
Owned PID/start/pidfd cleanup and TIME_WAIT-safe availability probe are reused.
Unchanged m5-remote-tools preserves two-host sampling and exact source hash
trees; run-m6-w1 supplies transport/inventory/hash verification only.

Unit1 includes protocol/scripts, focused red-to-green tests and no-query
preflight; committed parent/head must receive prospective PASS before traffic.
Unit2 consumes exactly these nine runs and submits all evidence once for
consolidated result review. Old evidence and verdicts remain intact.
