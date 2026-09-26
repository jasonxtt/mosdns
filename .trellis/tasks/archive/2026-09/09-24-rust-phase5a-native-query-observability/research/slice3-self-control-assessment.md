# Slice 3 identical-binary diagnostic assessment

## Scope and identity

Protocol pinned in commit `dec901e` before assessment. Nine W1 TCP slots ran
once from 2026-09-26 01:31:06 UTC through 01:33:52 UTC. Every slot used the
same Rust-before binary, SHA-256
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`, and the
same audit-disabled W1 TCP configuration, SHA-256
`1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1`.
The nine `sut.json` records match; all nine complete input-hash manifests are
byte-identical. Rates, stage duration, order, CPU sets, deadline, runner,
helper, and analyzer matched the V12 diagnostic protocol.

The labels `before_off`, `after_off`, and `after_on` are pairing slots only.
All are the same baseline with audit off. This diagnostic is neither a V13
candidate nor an enabled-audit comparison and cannot replace official runs.

## Result

All nine runner exits were zero. All 18/18 primary rows were valid, and all
16,200 scheduled primary requests were sent, received, and correct on time.
Late or wrong responses, protocol or transport errors, timeouts, and sender
shortfalls were zero. No attempt was replaced.

Despite identical inputs, the frozen analyzer called two comparisons
`repeatable regression`:

| W1 TCP 400 metric | Control values, us | Same-binary slot values, us | Paired deltas | Frozen guard |
|---|---:|---:|---:|---:|
| p95 | 447, 523, 559 | 569, 412, 641 | +27.29%, -21.22%, +14.67% | 13.77% |
| p99 | 793, 834, 1,134 | 1,002, 583, 1,480 | +26.36%, -30.10%, +30.51% | 10.00% |

Each comparison exceeded its guard in two of three pairs. No CPU or RSS
comparison crossed a guard; two of four CPU comparisons were inconclusive.

The full 244-file raw manifest verified remotely with SHA-256
`b3137bf71d819bf7db492126f9b15082740bfc47ff4661c08a9894a7628a6694`.
Raw results remain at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-self-control-w1`.
Top-level derived evidence and each slot's binary/input/affinity/environment
metadata are captured in `slice3-self-control-w1/`. All 55 copied files listed
in the raw manifest verified locally, as did the manifest sidecar.

## Gate and next boundary

This demonstrates that the current short same-host measurement can classify
identical executable/configuration variation as repeatable latency regression.
It does not establish the source of variation (VM scheduling, harness,
network, or another environmental effect), the false-positive frequency, or
that V12's actual overhead is acceptable. The official V12 five-crossing
verdict remains unchanged and A5 remains unmet.

This is a major evidence issue under the active workflow's stop rule. Stop
speculative source corrections and acceptance retries. A separate, explicitly
authorized measurement correction must first localize the variance, establish
a stable identical-binary control, and obtain review of a new frozen protocol
before running candidate acceptance. Do not relax thresholds retrospectively,
select successful attempts, claim PASS, archive this task, or deploy.
