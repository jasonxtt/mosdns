# VM validation transcript summary

All commands below ran in `/root/mosdns-rust-querydiag/rust` on the
`mosdns-rust` SSH VM. The browser proof used the fixtures next to this file.
The final implementation candidate for the code changes is
`19557022` (`fix(native): align query drilldown and allocation contract`),
following `47868d8e` (`fix(native): harden query diagnostic read boundaries`),
`1cb9e00e` (`test(native): tighten audit rank membership proof`) and
`9ce9947e` (`fix(native): close final query diagnostics gaps`). The latest
focused evidence below was collected from this candidate; documentation-only
updates do not change product code.

## Bounded record/projection checks

- `cargo test -p mosdns-native-host --test slice8_audit_read_http -- --nocapture`:
  `5 passed`.
  The real UDP/HTTP test fills the configured retained ring with `400000`
  real DNS records, performs concurrent `/api/v2/audit/logs?limit=500` and
  `/api/v2/audit/rank/slowest?limit=300` reads, asserts a bounded read remains
  active while DNS probes complete, and verifies both projections remain
  available. The same test also proves every real HTTP filter family,
  combinations, repeated mapped and empty client IP candidates, percent/form
  and Unicode decoding, case/exact handling, cross-field exact-domain
  isolation, malformed/overflow queries, all four rank routes, slowest
  ordering/membership, clear/stop/start/resize/eviction, and exact domain
  logs. The real UDP fixture additionally proves raw uncommon RDATA,
  malformed-wire-to-SERVFAIL, and upstream-timeout-to-SERVFAIL audit
  agreement; a sibling case uses a real TCP listener for a positive
  AAAA+CNAME response, and a separate real UDP case proves cache miss/hit and
  TTL aging.
- The latest full-ring test printed `retained=400000 concurrent_reads=2
  dns_progress_during_reads=4 logs=500 slowest_max=300 logs_bytes=211616
  logs_bytes_per_record=423.23 large_answer_projection_bytes_per_record=49152
  estimated_400000_large_projection_bytes=19660800000
  two_snapshot_arc_handle_bytes=6404800`. It also holds both expensive reads
  open and proves a third simultaneous read receives 503 `audit read capacity
  exhausted`. The test checks that at least one HTTP worker remains active
  before and during the DNS probes. The same real HTTP fixture sends a request
  on a socket that is closed before the response and proves that the
  disconnecting worker still occupies a slot until its canceled worker exits.
  These are projection-size and progress observations, not a production
  capacity claim.
- `cargo test -p mosdns-native-host --lib
  canceled_audit_read_releases_its_slot_only_after_worker_exit`: `1 passed`.
  A canceled worker returns 499, and a following read cannot acquire the
  single test permit until that worker has exited; the permit is available
  again afterwards. Rank allocation/serialization failures use the same
  worker-owned path and return 500 rather than being reported as cancellation.
- `time -p cargo test -p mosdns-dns-core --lib response::tests::answer_projection_keeps_a_large_ordered_answer_set_without_eviction`:
  `1 passed`, `real 0.33`, `user 0.28`, `sys 0.10`; the test projects 1024
  ordered A answers and checks the first/last TTLs, with no hidden answer
  eviction. Its VM output was `records=1024 projection_bytes=49152
  bytes_per_record=48.00`.
- The deliberately-large answer projection records `records=1024
  projection_bytes=49152 bytes_per_record=48.00`; the ordinary full-ring
  projection records `422.93` bytes/record. These are bounded-count,
  projection-size and progress/timing observations, not a production-capacity
  claim, and no byte truncation or capacity reduction was introduced.

## Final exact-candidate commands

The final corrective candidate is re-run with:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p mosdns-native-host
npm ci
npm run build
```

On `19557022`, all six commands completed successfully on the VM: workspace
clippy had no warnings, `cargo test --workspace` passed all unit, integration,
and doctest groups, including the five real slice8 listener tests, and the
native-host build completed. `npm ci` installed
the lockfile set and `npm run build` completed; Vite emitted only its existing
large-chunk warning (npm also reported the disposable dependency audit's
5-vulnerability summary, without applying `npm audit fix`). The focused
provenance, effective-tag, projection, native-host, DNS-core and real
UDP/TCP/HTTP tests used by the browser proof passed as well. No production
process, push, or alternate build host is involved; all product validation
used the `mosdns-rust` VM and the disposable `/root/mosdns-rust-querydiag`
checkout.
