# VM validation transcript summary

All commands below ran in `/root/mosdns-rust-querydiag/rust` on the
`mosdns-rust` SSH VM. The browser proof used the fixtures next to this file.
The final implementation candidate for the code changes is
`9f5dd01f9f9b9c2816c5e2a3d10fb0f7a50a9d21`. The later documentation-only
commit records this exact candidate and does not change product code.

## Bounded record/projection checks

- `cargo test -p mosdns-native-host --test slice8_audit_read_http`: `1 passed`.
  This real UDP/HTTP test fills the configured retained ring with `400000`
  real DNS records, then performs concurrent `/api/v2/audit/logs?limit=500`
  and `/api/v2/audit/rank/slowest?limit=300` reads while additional DNS
  requests continue and verifies both projections remain available.
- `time -p cargo test -p mosdns-native-host --test slice8_audit_read_http`:
  `1 passed`, `real 29.96` (the VM image does not provide `/usr/bin/time`).
  The test printed `retained=400000 logs=500 slowest_max=300
  logs_bytes=187987 logs_bytes_per_record=375.97` for the concurrent views.
- `time -p cargo test -p mosdns-dns-core --lib response::tests::answer_projection_keeps_a_large_ordered_answer_set_without_eviction`:
  `1 passed`, `real 0.33`, `user 0.28`, `sys 0.10`; the test projects 1024
  ordered A answers and checks the first/last TTLs, with no hidden answer
  eviction.
- No peak-RSS number was claimed because the VM image does not provide
  `/usr/bin/time`. These are bounded-count, projection-size and progress/timing
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

The exact-candidate run also passed `cargo test --workspace`, focused native
host/DNS/sequence tests, and the real UDP/TCP/HTTP/browser proof described in
`README.md`. The Vue build completed with only the existing Vite large-chunk
warning. No production process, push, or alternate build host is involved;
all product validation used the `mosdns-rust` VM and the disposable
`/root/mosdns-rust-querydiag` checkout.
