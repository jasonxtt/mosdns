# S6 public proof — 2026-10-02

Product source HEAD: `3ffa1b273f8262bbae19fe6d2500eb9aa58a54df`. No product source changes in S6. Native binary SHA-256 is in evidence.json.

Dedicated Linux root `/root/mosdns-rust-response-policy-20261002`; all DNS peers, native API, and Vite bind loopback. Source and root test fixtures were synchronized to this root, using the previous completed task's build/dependency cache to avoid disk duplication. No local Rust build/test, production service, push or deployment.

## Reproduction

Place the checked-in proof harness in `<root>/proof/`, matching its explicit paths; build native `mosdns` from this source in `<root>/rust`. Copy `hosts.seed.txt` to `hosts.txt`, then run `python3 services.py start`, `python3 oracle.py`, and `python3 services.py cleanup`. The oracle flushes only the disposable `stored` cache; use a fresh isolated root. UDP and TCP configurations run sequentially because the native host supports exactly one listener per process. `services.py` verifies owned PID, start-time and cwd before signals, waits for SIGTERM shutdown, and refuses identity mismatches.

Initial fixture incorrectly included both listeners; startup correctly rejected `exactly one listener plugin is supported`. This was a harness correction, not a product fix. The initial public run passed UDP/TCP/restart/Vue; the final run additionally asserts real IPv6 Answer-IP fallback and API supplier/attempt invariants. Both use identical product source.

## Observed behavior

- Real UDP A/AAAA hosts answers retain TTL 10; empty requested family returns full FakeSOA TTL 300.
- Real upstream sees `target.example`, never the admitted `original.example`; final client question is restored, CNAME chain TTLs are 1/30/30. NXDOMAIN preserves target SOA fields and response flags; the explicitly configured TTL policy sets its TTL 30.
- Warm cache produces no new peer query, no selected upstream and no attempts in public audit. Hosts also has no network supplier. IPv4 and IPv6 bad-answer predicates both select backup with two actual attempts.
- A running owner keeps its immutable hosts snapshot after file change. Save + actual SIGTERM/process restart with retained v2 dump still answers the old address. **Dump import is not policy-versioned**: rule changes alone do not invalidate retained entries. Quiescing producers → API Flush while owner is open → SIGTERM/final save → restart returns the new address without resurrection. API inventories and distinct actual process PIDs are retained in evidence.json.
- TCP length-framed redirect and fallback pass under the same product logic.
- Unmodified maintained Vue consumes the real API through isolated Vite. Three DOM files and JPG screenshots show local hosts without network supplier; original query with CNAME/data/TTL/actual primary; IP fallback with actual backup and both attempts. Screenshots were taken during the first successful run, whose final four audit rows are preserved in the DOM; the final run recreates equivalent rows with new trace IDs. No frontend source changes or new IP editing flow.

## Quality and cleanup

`CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build --offline -p mosdns-native-host --bin mosdns -j1`; `cargo fmt --all --check`; `cargo clippy --offline --workspace --all-targets -j1 -- -D warnings`; `cargo test --offline --workspace -j1`: **1,070 passed, zero ignored**, 71 result blocks including doctests. Full output: workspace-validation.log. Final process assertions and owned cleanup: process-validation.log. Browser forwarding session stopped; temporary Vue tab closed. The dedicated C2C review conversation is retained.

This proves the approved bounded response-policy/IP chain, not whole Phase 5/6 acceptance or production cutover. SRS/binary/compressed/provider sets, management reload, static UI embedding and full performance remain outside this task.
