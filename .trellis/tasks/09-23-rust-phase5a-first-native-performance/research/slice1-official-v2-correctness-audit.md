# Official v2 raw-result correctness audit

Date: 2026-09-24. Read-only validation of the completed official matrix; no SUT was launched and no file under results/official-v2/ was changed.

## Pinned verifier and input identity

- Helper: phase5a-baseline-helper/v8, SHA-256 df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065.
- Manifest: SHA-256 3d8b76e8799bf709ef7e19df9edd6245936f05f9c4e09230dd49ecdaaf000f13.
- W2 frozen TTL: 30 s; per-key safety margin: 500 ms; eligibility ceiling: 29.5 s.
- Workloads and all seven fixed input hashes are listed in official-manifest-v2.json.

## W2

For all six candidate/repetition runs, the pinned v8 helper revalidated:

1. Cold stage exact upstream miss delta from the pre-stage fixture counter.
2. Warm prefill exact miss delta for each unique cache key.
3. All five warm stages against per-key prefill time and the 29.5 s eligibility ceiling.
4. Zero warm-stage upstream counter delta after prefill.

Each run prefetched both keys. The largest observed prefill-to-response age was 18.373472 s (r1 Go, recovery stage), leaving about 11.127 s before the safety ceiling. No W2 counter or TTL reason appears in the invalid-stage records.

## W3

For all six candidate/repetition runs, the pinned v8 helper revalidated each of five stage windows: 30/30 verify-routing-events invocations passed. It also verified complete journal sequence and tail barriers for 6/6 runs. These checks correlate event occurred_at with exactly one matching request interval, enforce fixture sequence barriers, and compare exact per-request route legs/order. The frozen paths were:

- DOMAIN_HIT: [route-a]
- IP_RULE_HIT: [route-b, route-a]
- IP_RULE_MISS: [route-b, route-c]

The two W3 1000 QPS sender-shortfall attempts each omitted one scheduled slot. Event-oracle validation still passed for all requests actually sent; those two repetitions remain invalid for paired latency coverage.

## Upstream counter and leg cross-check

The read-only `summarize-official-v2-upstreams.py` export reverified all 718 frozen raw files against `official-results-index.sha256` before producing the 132-row `slice1-official-v2-upstream-observations.tsv`. Its SHA-256 is `6e04b326f3237e468178a86c9ad2528df97e1670adcffa8880a7fbd508896aeb`; script SHA-256 is `35d5d96109f99ef463f7a386e4fd979cbdac12a94c725468c78f05d0c66ed6d4`.

- W1: all 60 measured stage rows reconcile the sum of positive and negative forwarding-counter deltas to the actual sent count. The two UDP sender-shortfall rows have 14,399 sent at 800 QPS and 17,999 sent at 1,000 QPS; all other W1 stages sent their full schedule.
- W2: in each of six candidate attempts, cold and warm-prefill each incremented both unique-key counters exactly once. Every one of the 30 warm measured-stage rows had zero counter delta, including rows invalidated only by sender shortfall or terminal health-check criteria.
- W3: for all 30 stage rows, sent-request counts by case predict the observed route-a/b/c events by qname and upstream exactly. The 17,998 sent overload requests produced 29,996 observed legs (`route-a=11,999`, `route-b=11,998`, `route-c=5,999`); the two shortfall attempts remain invalid for paired latency coverage. Exact per-request order is established by the pinned helper audit above, not inferred from these aggregates.

The export and script were stored under the task-owned `evidence-official-v2` analysis directory on the test VM; neither the raw `results/official-v2/` tree nor any SUT/service was modified or rerun.

## Raw-tree integrity and cleanup

- sha256sum -c verified all 718 entries in official-results-index.sha256.
- Raw result tree: 718 files, 83 MiB.
- Reconciled stage table: 126 rows; resource table: 438 rows.
- After audit: zero process argv matched the exact task Go/Rust/helper binaries; all ten frozen benchmark ports were free.
- Existing VM mosdns service remained present and untouched. Production mos was not used.

The paired aggregator requires a per-attempt attempt-exit-status.txt; the official driver stores authoritative status in the root attempt-status.tsv. The final aggregate therefore uses a read-only analysis overlay and a hash-bound 24-row status map. Only overlay sidecars were created; raw attempt files remain index-verified.
