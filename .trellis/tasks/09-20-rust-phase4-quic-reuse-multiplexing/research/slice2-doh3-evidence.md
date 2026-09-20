# Slice 2 DoH3 reuse evidence

Status: implementation complete; scoped reviewer gate is pending. This record
covers Slice 2 only and does not claim Slice 3.

## Implementation boundary

Changed implementation paths:

- `rust/upstream-core/src/quic_reuse.rs`
- `rust/upstream-core/src/quic.rs`
- `rust/upstream-core/src/lib.rs`
- `rust/upstream-core/tests/quic_reuse_doh3.rs`

The shared DoH3 adapter uses one validated `QuicReuseKey`/generation, one
authenticated Quinn connection, one long-lived H3 driver, and a cloneable H3
sender. Each exchange opens one fresh HTTP/3 request stream, preserves the
endpoint authority and encoded path, applies the existing response validators,
and restores the caller's DNS ID. Teardown force-closes the connection and
endpoint, waits caller-owned connection handles, joins the driver, waits Quinn
endpoint idle, and only then reaches terminal removal. H3 stream errors use the
authoritative R0b classifier; driver/connection terminal evidence deactivates
the exact key+generation without replaying a query.

No Cargo manifest/lockfile, Go/cgo/FFI, config/API/WebUI, TCP pool, or Slice 3
production surface was changed.

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

## Required commands

All commands were run from `/Users/tom/github/mosdns-rust/rust` unless noted.

| Command | Exit |
|---|---:|
| `cargo test -p mosdns-upstream-core --test quic_reuse_doh3 --locked` | 0 |
| `cargo test -p mosdns-upstream-core --test slice2_doh3 --locked` | 0 |
| `cargo test -p mosdns-upstream-core --test slice3_quic --locked` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy -p mosdns-upstream-core --all-targets --locked -- -D warnings` | 0 |

The focused reuse test passed 1/1, the existing one-shot H3 suite passed 22/22,
and the existing Slice 3 QUIC suite passed 23/23. The full crate run and task
validation remain part of the pre-review gate.
