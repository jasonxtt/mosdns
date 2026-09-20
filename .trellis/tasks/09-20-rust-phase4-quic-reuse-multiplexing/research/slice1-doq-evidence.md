# Slice 1 DoQ reuse evidence

Status: implementation complete in the shared worktree; no commit or push was
performed. This record covers Slice 1 only.

## Implementation boundary

Changed implementation paths:

- `rust/upstream-core/src/quic_reuse.rs`
- `rust/upstream-core/src/quic.rs`
- `rust/upstream-core/src/lib.rs`
- `rust/upstream-core/tests/quic_reuse_doq.rs`

The shared DoQ adapter uses the existing `QuicReuseOwner` admission/lifecycle
model, `tcp::write_frame`, the existing DoQ response validator, and one
entry-owned QUIC endpoint/connection per validated DoQ key. It opens one fresh
bidirectional stream per leased query. No Cargo manifest/lockfile, Go/cgo/FFI,
config/API/WebUI, TCP pool, H3, driver, or production wiring was changed.

## RED-to-green evidence

The first focused run was intentionally RED with exit status `101`: the new
test target failed to compile because `DoqReuseUpstream` did not yet exist.
After implementation, the same target passed with exit status `0`:

```text
cargo test -p mosdns-upstream-core --test quic_reuse_doq --locked
5 passed; 0 failed; exit 0
```

The loopback suite proves:

- concurrent queries use one QUIC accept and independent bidirectional stream
  IDs;
- outbound DNS IDs are zeroed while response IDs are restored to the caller;
- peer request/response FIN behavior is required;
- cancellation of one stream leaves a second stream on the healthy connection;
- reset and malformed responses remain stream-local;
- connection failure deactivates the exact generation, keeps it discoverable
  until terminal removal, and permits one later replacement connection;
- no query is replayed by the adapter.

## Required commands

All commands were run from `/Users/tom/github/mosdns-rust/rust` unless noted.

| Command | Exit |
|---|---:|
| `cargo test -p mosdns-upstream-core --test quic_reuse_doq --locked` | 0 |
| `cargo test -p mosdns-upstream-core --test slice1_doq --locked` | 0 |
| `cargo test -p mosdns-upstream-core --test slice3_quic --locked` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy -p mosdns-upstream-core --all-targets --locked -- -D warnings` | 0 |
| `cargo test -p mosdns-upstream-core --locked` | 0 |
| `git diff --check` | 0 |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing` | 0 |

The full crate run included 104 unit tests, all upstream-core integration
targets, the five new DoQ reuse tests, and doc tests; all passed. The complete
`slice3_quic` run passed 23/23; its existing exhaustive peer-code case takes
approximately 225 seconds.

Pre-existing dirty files outside the Slice 1 boundary were preserved and are
not part of this evidence or implementation change.
