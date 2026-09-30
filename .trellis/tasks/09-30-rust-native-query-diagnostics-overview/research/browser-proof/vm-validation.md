# VM validation transcript summary

All commands below ran in `/root/mosdns-rust-querydiag/rust` on the
`mosdns-rust` SSH VM. The browser proof used the fixtures next to this file.
The final implementation candidate for the code changes is
`be8f8ce1` (`fix(native): preserve query provenance and projection bounds`).
The later documentation-only commit records this exact candidate and does not
change product code.

## Bounded record/projection checks

- `cargo test -p mosdns-native-host --test slice8_audit_read_http`: `2 passed`.
  The real UDP/HTTP test fills the configured retained ring with `400000`
  real DNS records, performs concurrent `/api/v2/audit/logs?limit=500` and
  `/api/v2/audit/rank/slowest?limit=300` reads, asserts a bounded read remains
  active while DNS probes complete, and verifies both projections remain
  available. The same test also proves every real HTTP filter family,
  combinations, repeated mapped client IP candidates, malformed/overflow
  queries, all four rank routes, slowest membership, and exact domain logs.
  A sibling case uses a real TCP listener for a positive AAAA+CNAME response.
- `cargo test -p mosdns-native-host --test slice8_audit_read_http` with a
  `/proc`/`ps` polling wrapper: `2 passed`, `slice8_peak_rss_kib=317400`.
  The full-ring test printed `retained=400000 concurrent_reads=2
  dns_progress_during_reads=4 logs=500 slowest_max=300 logs_bytes=211469
  logs_bytes_per_record=422.93` for the concurrent views; it also holds both
  expensive reads open and proves a third simultaneous read receives 503
  `audit read capacity exhausted`. The test checks that at least one HTTP
  worker remains active before and during the DNS probes. The VM image does not provide `/usr/bin/time`, so the recorded
  peak is the test-process resident-set observation from `/proc`/`ps`, not a
  production capacity claim.
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

The `be8f8ce1` candidate also passed the focused provenance, effective-tag,
projection, native-host, DNS-core and real UDP/TCP/HTTP tests used by the
browser proof. The final workspace and disposable Vue builds are recorded
after this candidate. The Vue build completed with only the existing Vite
large-chunk warning. No production process, push, or alternate build host is
involved; all product validation used the `mosdns-rust` VM and the disposable
`/root/mosdns-rust-querydiag` checkout.
