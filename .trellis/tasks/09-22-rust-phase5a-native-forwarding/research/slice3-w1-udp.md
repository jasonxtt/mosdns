# Slice 3 W1 UDP evidence

## Scope and commit

Slice 2 root re-review returned `SLICE 2: PASS` with P0/P1/P2 all zero and
authorized Slice 3 only. The UDP implementation was committed as:

```text
0efb9db feat(native-host): add phase5a UDP forwarding path
```

The product diff is limited to:

```text
rust/native-host/src/assembly.rs
rust/native-host/src/lib.rs
rust/native-host/src/main.rs
rust/native-host/src/udp.rs
rust/native-host/tests/w1_udp.rs
```

No Cargo manifest or lockfile change was required. Existing unrelated dirty
workspace/journal files and `.DS_Store` entries were not staged.

## Real local UDP path

`UdpServer` binds one Tokio `UdpSocket` and receives each datagram into an
owned `Vec<u8>`. Each request is a local task with its own parsed query,
canonical `ExecutionMachine`, child cancellation token, deadline, and response
peer. The only external dispatch is the validated forward executable from the
compiled W1 graph. The task awaits the existing `ForwardAdapter`/
`upstream-core` exchange, validates and patches the returned response ID/RA,
resumes the same machine, and sends one response to the original peer.

The loopback integration target uses an independent local UDP mock upstream
and four tests:

```text
udp_answers_positive_nxdomain_and_concurrent_distinct_queries
malformed_datagram_is_dropped_and_listener_stays_available
stalled_upstream_maps_to_servfail_and_shutdown_allows_rebind
cancellation_stops_pending_request_without_a_late_response
```

These cover positive A forwarding, NXDOMAIN preservation, three concurrent
queries with distinct IDs/qnames, malformed-datagram isolation, deadline to
SERVFAIL, cancellation with no late write, shutdown task joining, upstream
close, listener release, and clean rebind. A private unit test additionally
drives the compiled sequence to completion without setting a response and
verifies the final mapping is valid REFUSED.

## Required checks

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
  PASS

cargo test --manifest-path rust/native-host/Cargo.toml --test w1_udp --locked
  PASS: 4 tests

cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked
  PASS: 9 unit/integration tests

cargo clippy --manifest-path rust/native-host/Cargo.toml --all-targets --locked -- -D warnings
  PASS

cargo test --manifest-path rust/dns-core/Cargo.toml --all-targets --locked
  PASS: existing full dns-core suite

python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
  PASS; only the pre-existing rust-migration context-size warning is emitted

git diff --check -- rust/native-host rust/dns-core \
  .trellis/tasks/09-22-rust-phase5a-native-forwarding
  PASS after the task-local evidence update
```

No SSH, VM, browser, official baseline runner, benchmark, deployment, or
production integration was run. The task-local loopback test is correctness
evidence only and is not a Go/Rust performance comparison.

## Stop boundary

This record requests root `SLICE 3: PASS`. Until that review returns PASS,
Slice 4 TCP work, remote Linux evidence, benchmark work, deployment, and
finish/archive remain unauthorized.
