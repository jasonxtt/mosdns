# Slice 4 W1 TCP evidence

## Scope

This slice implements only the frozen W1 TCP listener path under:

```text
rust/native-host/**
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

No cache, routing, API/WebUI, QUIC, secure transport, pooling, production
integration, deployment, benchmark campaign, or historical baseline artifact
was changed.

## Runtime path

`TcpServer` owns one Tokio `TcpListener` and one local `JoinSet`. Accepted
connections run as independent local tasks with child
`TransportCancellation` scopes. A connection reads exactly a two-byte
big-endian length prefix and the declared body, reassembling fragmented reads.
It processes frames sequentially on that connection while the supervisor
continues accepting other connections concurrently.

Every valid DNS frame enters the same canonical `execute_request` machine used
by W1 UDP. The existing `ForwardAdapter`/`upstream-core` exchange remains the
only upstream path. Responses are framed by `dns-core::frame_response` in
`FrameMode::Stream`, so native host code does not introduce a second DNS
framing format.

The per-frame idle deadline is the compiled positive TCP `idle_timeout`; the
frozen W1 YAML value is two seconds. EOF, partial frames, zero-length frames,
malformed DNS, and client disconnect terminate only the affected connection.
Supervisor cancellation stops accepts, cancels and joins all connection tasks,
closes the upstream owner after the join drain, and releases the listener for
rebinding.

## Focused loopback coverage

`rust/native-host/tests/w1_tcp.rs` contains an independent local TCP upstream
and five tests:

```text
tcp_answers_fragmented_positive_nxdomain_sequential_and_concurrent_requests
partial_frame_eof_closes_only_that_connection_and_listener_stays_available
idle_timeout_closes_inactive_connection
stalled_upstream_maps_to_servfail_and_disconnect_shutdown_allows_rebind
client_disconnect_isolated_and_shutdown_releases_connection_tasks
```

The tests cover positive A response, NXDOMAIN preservation, request/response
ID association, fragmented request bytes, fragmented upstream response bytes,
sequential frames per connection, concurrent connections, effective two-second
idle timeout, stalled upstream to associated SERVFAIL, partial-frame EOF
isolation, client disconnect, shutdown task joining, upstream close, and clean
listener rebind.

## Checks

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check                 PASS
cargo test --manifest-path rust/native-host/Cargo.toml --test w1_tcp --locked PASS (5 tests)
cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked PASS
cargo clippy --manifest-path rust/native-host/Cargo.toml --all-targets --locked -- -D warnings PASS
```

The native-host manifest and `Cargo.lock` were unchanged. No remote Linux
evidence, VM, browser, benchmark, deployment, or official Go baseline runner
was used for the focused local checks; any separately authorized Linux
correctness evidence must be recorded independently and is not performance
comparison evidence.
