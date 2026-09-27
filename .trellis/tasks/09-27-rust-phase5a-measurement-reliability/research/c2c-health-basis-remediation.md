# C2C remediation — explicit recovery health latency basis

The first official-r2 attempt was preserved as invalid evidence after the
terminal health check failed. The raw stage evidence showed complete
200/300/350/400/200-QPS W1-TCP traffic, correct counters, unchanged process
identity, and resources below the frozen caps. The failure was a measurement
contract mismatch: the runner applied the pilot-derived W1 p95/p99 band to the
planned-slot-to-finish summary, which includes sender scheduling lag, while the
pilot band was based on dispatch-to-finish latency.

The remediation keeps the attempt unchanged and makes the distinction
executable:

- `stageResult` retains `planned-slot-to-finish` as the primary latency view and
  adds separately declared dispatch health samples and percentiles.
- `verify-continuous` accepts an explicit latency view and fails closed when a
  requested dispatch health view is missing or mismatched.
- The official execution contract pins `recovery_latency_view` to
  `dispatch-to-finish`; the official runner hard-sets that value and records it
  in run metadata. Non-official compatibility calls retain the planned view as
  their default.
- Unit coverage proves that a primary planned latency above the frozen band can
  still pass only when the declared dispatch health view is within the band,
  and that missing dispatch metadata fails.

A controlled remote W1-TCP smoke after synchronization is archived under
`c2c-health-basis-smoke/` (raw-file-hashes sidecar:
`02d05a4c5ad964cf737beafc378dd5414e5c53620256f73c6f51fe73186c1267`). Its
stage projection contains primary p95/p99 `2164/2164` µs and independently
declared dispatch health p95/p99 `1004/1004` µs. This is schema/transport
evidence only: it is a smoke run, not an official candidate result or a
threshold decision.

This changes no frozen p95/p99 ceiling and does not reinterpret the failed
official-r2 attempt. It authorizes no rerun, profiling, or capacity conclusion;
those remain gated on a fresh C2C `FINAL: PASS` for the committed remediation.
