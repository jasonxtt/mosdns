# Slice 1 canonical sequence machine evidence

Date: 2026-09-22

## Scope

Initial implementation commit: `cd9e14b`.
Remediation commit: `f17ae97`.

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
- The remediation matrix compares owned external resume against the sync
  adapter for Continue/Return/Accept/Reject/Exit and ordinary errors, covers
  nested try parity, checks fuel/cancellation after a pending dispatch, and
  verifies owned-machine missing-entry rejection.

## Root review remediation

The first root review returned `SLICE 1: FAIL` with P0=0, P1=1, P2=1:

- P1 required the planned owned-machine/sync-adapter outcome and suspension
  parity matrix, nested-try and ordinary-error parity, post-dispatch
  fuel/cancellation checks, and direct missing-entry test.
- P2 required updating the `ExecutableId` documentation from fixture-only to
  the shared executable catalog.

Only `rust/sequence-core/tests/slice5_resumable.rs` and the one documentation
line in `rust/sequence-core/src/program.rs` changed in remediation.

## Required checks

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked  PASS (65 tests)
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
