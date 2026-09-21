# Slice 2 DoH3 reuse evidence

Status: remediation implementation complete; scoped reviewer gate is pending.
This record covers Slice 2 only and does not claim Slice 3.

## Implementation boundary

The parent Slice 2 implementation established the following baseline paths:

- `rust/upstream-core/src/quic_reuse.rs`
- `rust/upstream-core/src/quic.rs`
- `rust/upstream-core/src/lib.rs`
- `rust/upstream-core/tests/quic_reuse_doh3.rs`

The current remediation commit changes exactly these four paths:

- `rust/upstream-core/Cargo.toml` (user-authorized opt-in h3 API feature only)
- `rust/upstream-core/src/quic_reuse.rs`
- `rust/upstream-core/tests/quic_reuse_doh3.rs`
- `.trellis/tasks/09-20-rust-phase4-quic-reuse-multiplexing/research/slice2-doh3-evidence.md`

The locked `h3 =0.0.8` dependency now enables
`i-implement-a-third-party-backend-and-opt-into-breaking-changes`; the version,
lockfile, default features, tracing, and datagram features remain unchanged.
This is required to bind the pinned `RemoteClosing` and `ConnectionError(_)`
provenance at the actual `SendRequest`/request-stream boundary. The shared DoH3
adapter uses one validated `QuicReuseKey`/generation, one
authenticated Quinn connection, one long-lived H3 driver, and a cloneable H3
sender. Each exchange opens one fresh HTTP/3 request stream, preserves the
endpoint authority and encoded path, applies the existing response validators,
and restores the caller's DNS ID. Teardown force-closes the connection and
endpoint, waits caller-owned connection handles, joins the driver, waits Quinn
endpoint idle, and only then reaches terminal removal. H3 stream errors use the
authoritative R0b classifier; driver/connection terminal evidence deactivates
the exact key+generation without replaying a query.

No Cargo.lock, Go/cgo/FFI, config/API/WebUI, TCP pool, or Slice 3 production
surface was changed.

## Loopback proof

`quic_reuse_doh3.rs` runs a real verified TLS/H3 loopback fixture. The server
accepts exactly one QUIC connection, accepts two request streams, waits until
both request FINs arrive, and only then sends both responses. The test proves:

- two concurrent queries multiplex over one physical connection;
- each request has an independent response stream and the caller's response ID
  is restored;
- `:authority` and `:path` are preserved and the GET carries no body;
- the active entry retains one owner liveness registration until explicit close;
- owner close drains the shared entry and leaves no lifecycle registration.

The remediation tests add real pinned-stack negative paths on the same physical
connection:

- cancellation while the response body is pending returns the typed
  `Cancelled(Sent)` result, drops only that request stream, and allows another
  request to complete on the same connection; the client path contains no
  receive-side `stop_sending` call after a pending read is canceled;
- a peer stream reset and a malformed declared body remain stream-local, leave
  the generation active, and are followed by a successful third request;
- a server GOAWAY makes the next `send_request` return the pinned
  `RemoteClosing` path, deactivates the exact key+generation, and reaches map
  removal only after supervised teardown.
- a debug-only real-stack hold keeps a caller-owned DoH3 connection handle and
  stream lease alive while `owner.close()` runs; close remains pending and the
  generation remains discoverable with the held caller registration plus the
  entry liveness registration, then the
  handle release allows driver/endpoint drain and exact map removal.

## Required commands

All commands were run from `/Users/tom/github/mosdns-rust/rust` unless noted.

| Command | Exit |
|---|---:|
| `cargo test -p mosdns-upstream-core --test quic_reuse_doh3 --locked -- --test-threads=1` | 0 |
| `cargo test -p mosdns-upstream-core --test slice2_doh3 --locked` | 0 |
| `cargo test -p mosdns-upstream-core --test slice3_quic --locked` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy -p mosdns-upstream-core --all-targets --locked -- -D warnings` | 0 |

The focused reuse test passed 5/5, the existing one-shot H3 suite passed 22/22,
and the existing Slice 3 QUIC suite passed 23/23. The full crate run and task
validation remain part of the pre-review gate.
