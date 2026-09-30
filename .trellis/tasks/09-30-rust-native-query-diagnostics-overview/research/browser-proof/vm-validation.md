# VM validation transcript summary

All commands below ran in `/root/mosdns-rust-querydiag/rust` on the
`mosdns-rust` SSH VM. The browser proof used the fixtures next to this file.
The final implementation candidate is `df83609c` (full SHA recorded by Git).
It includes the initial implementation and the corrective review iteration.

## Bounded record/projection checks

- `cargo test -p mosdns-native-host --test slice8_audit_read_http`: `1 passed`.
  This real UDP/HTTP test fills the configured retained ring with `400000`
  real DNS records, then performs stats/windows/log reads while additional DNS
  requests continue and verifies responses remain available.
- `time -p cargo test -p mosdns-native-host --test slice8_audit_read_http`:
  `1 passed`, `real 30.20`, `user 28.79`, `sys 4.28`.
- `time -p cargo test -p mosdns-dns-core --lib response::tests::answer_projection_keeps_a_large_ordered_answer_set_without_eviction`:
  `1 passed`, `real 0.33`, `user 0.28`, `sys 0.10`; the test projects 1024
  ordered A answers and checks the first/last TTLs, with no hidden answer
  eviction.
- The VM image does not provide `/usr/bin/time`, so a peak-RSS number was not
  available. These are bounded-count, projection-size and progress/timing
  observations only; they are not a worst-case memory or production-capacity
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

The task uses the existing real-listener regression plus the focused
`mosdns-native-host` library, DNS projection, and `slice8_audit_read_http`
tests. No production process, push, or alternate build host is involved.
