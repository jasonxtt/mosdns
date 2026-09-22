# Slice 1 canonical sequence machine evidence

Date: 2026-09-22

## Scope

Implementation commit: `cd9e14b`.

Changed paths are limited to:

- `rust/sequence-core/src/engine.rs`
- `rust/sequence-core/src/program.rs`
- `rust/sequence-core/src/lib.rs`
- `rust/sequence-core/tests/slice5_resumable.rs`

The existing dirty `.trellis/workspace/**` files and `.DS_Store` files were
not staged or modified by this slice.

## Behavior covered

- `ExecutionMachine` owns state/control for the future async host and has a
  borrowed constructor used by the existing synchronous `execute` adapter.
- Both constructors drive the same scope/frame/continuation engine.
- Validated external executables receive stable `ExecutableId` values and
  yield one identity-only `ExternalDispatch` at a time.
- `resume` accepts only the matching pending ID and preserves state, frames,
  fuel, and cancellation across the boundary.
- Wrong pending IDs, a second pending dispatch, post-terminal resume, fuel
  exhaustion, cancellation, and sync fixture parity are tested.

## Required checks

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked  PASS (61 tests)
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked -- -D warnings  PASS
cargo tree --manifest-path rust/Cargo.toml -p mosdns-sequence-core --edges normal --locked  PASS (dns-core only)
git diff --check -- rust/sequence-core .trellis/tasks/09-22-rust-phase5a-native-forwarding  PASS
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding  PASS (non-fatal context-size warning)
```

No Tokio/upstream/native-host dependency, listener, network, VM, SSH, or
benchmark command was run. The historical baseline archive and frozen
evidence were not touched.

## Gate

Stopped after Slice 1 implementation and local checks. Waiting for root
`SLICE 1: PASS`; Slice 2 has not started.
