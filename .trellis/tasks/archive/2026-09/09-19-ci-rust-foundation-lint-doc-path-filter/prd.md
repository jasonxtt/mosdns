# Fix rust-foundation lint and ignore documentation-only CI

## Goal

Resolve the pre-existing rust-foundation Clippy failure exposed by GitHub Actions run #309 and prevent documentation-only pushes or pull requests from triggering test.yml.

## Background and confirmed facts

- GitHub Actions run [#309](https://github.com/jasonxtt/mosdns/actions/runs/35367216186) was run for planning commit `af8a4cbb`; the `build` job passed and the `rust-foundation` tests passed.
- The failing step was the same job's Clippy invocation, not an existing `reuse.rs` test. It reports `clippy::result-large-err` at `rust/upstream-core/src/secure/dot.rs:746`, where `PooledDotSession::exchange` returns `Result<(SecureResponse, Self), PooledDotOutcome>` and the error variant is at least 1120 bytes.
- The same failure occurred in the preceding run #308 for code commit `4eeab2d`, so it predates and is independent of the QUIC planning push. Local Rust 1.95 does not emit the newer stable-Clippy lint; the CI stable toolchain did.
- `.github/workflows/test.yml` currently runs `build` and `rust-foundation` for every push and pull request. Its manual `workflow_dispatch` path must remain available.

## Requirements

R1. Make the internal pooled DoT exchange error path pass `clippy -D warnings` on the current stable toolchain without weakening the lint policy or changing the existing `NotSent`/`MaybeSent`/`Sent`, session-discard, replacement, or response semantics. Prefer a representation change that keeps the retained session/outcome ownership explicit; do not change `reuse.rs` tests merely to hide this lint.

R2. Add equivalent `paths-ignore` filters to the workflow's `push` and `pull_request` triggers for documentation-only changes: all Markdown files (`**.md`), the `docs/**` tree, and the `.trellis/**` task/spec/documentation tree. Keep `workflow_dispatch` unchanged.

R3. Keep this maintenance change independent of the active QUIC/HTTP3/DoQ planning task. Do not add QUIC code, alter resolver/reuse behavior, change product configuration, or modify deployment/service state.

## Acceptance Criteria

- [x] A1. The existing Rust foundation test suite, including the existing `reuse.rs` tests, still passes under the focused CI command with `--all-targets --all-features --locked`.
- [x] A2. `cargo clippy --manifest-path rust/Cargo.toml -p mosdns-dns-core -p mosdns-upstream-core --all-targets --all-features --locked -- -D warnings` passes on the available toolchain, and the fix does not add a broad lint suppression.
- [x] A3. `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` and `git diff --check` pass.
- [x] A4. `test.yml` structurally ignores changes matching each of `**.md`, `docs/**`, and `.trellis/**` for both `push` and `pull_request`, while a source/workflow change can still trigger the workflow and manual dispatch remains available.
- [x] A5. The final diff is limited to the pooled DoT lint fix, `.github/workflows/test.yml`, and this task's planning/record files; no QUIC planning or production wiring is included.

## Out of scope

- Reworking or adding `reuse.rs` tests when the #309 evidence shows they already pass.
- Pinning CI to an older Rust toolchain solely to avoid the lint.
- Any QUIC/HTTP3/DoQ implementation, resolver/reuse behavior change, configuration/API change, deployment, or service restart.
