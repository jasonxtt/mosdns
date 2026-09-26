# M2 fixed controls: unqualified

The reviewed fixed experiment ran once, 2026-09-26 02:12:09–02:32:54 UTC.
All 54 runner exits were zero; 126/126 primary measurements were valid.
All 108,000 scheduled requests were sent, received and correct on time;
late/wrong/protocol/transport/timeout/shortfall counters were zero.

Both batches nevertheless reported six repeated latency guard crossings.
Batch 1 had 15 comparisons with an individual latency crossing; batch 2
had 13. Only 1/28 six-pair latency equivalence intervals qualified. The
qualification output lists 55 failure reasons (28 individual-guard comparison
failures and 27 equivalence failures). M2 is unqualified; V12's prior failed
acceptance remains unchanged. No candidate acceptance is resumed.

Evidence: `m2-control-qualification.json` and `m2-calibration-results/`.
The complete 117 MiB raw tree remains at
`mosdns-rust:/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m2-calibration`.
Its 1,724-file manifest verified remotely, SHA-256
`341cc3f1a782bc5aa5f26566861285de1a63373dc8097984665bb20054cc7ddf`.
284 selected files were verified locally against that manifest; its sidecar
also verified locally. No rejected attempts were replaced.

The correction removed avoidable sampler subprocesses and fixed helper
parallelism, but did not establish stable controls. It does not prove the
remaining variability's cause. At 200 QPS over three seconds, p99 depends on
roughly six tail observations. A prospective longer-window revision can
increase tail observations without widening the practical 10% margin.
Any new revision must be reviewed before fresh data and must preserve M2's
failure. Its controls, not these historical results, gate later acceptance.
