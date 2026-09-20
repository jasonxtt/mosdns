# Implementation and acceptance record — Rust foundation lint / CI path filter

## Scope

The implementation was completed in `b75bd7f` (`fix(ci): box pooled DoT
session in outcome and ignore doc-only CI triggers`). The retained pooled DoT
session is boxed inside `PooledDotOutcome`, preserving explicit session
ownership and the existing side-effect/rebuildability semantics while keeping
the `exchange` error below Clippy's large-error threshold. The `push` and
`pull_request` triggers in `.github/workflows/test.yml` ignore `**.md`,
`docs/**`, and `.trellis/**`; `workflow_dispatch` remains present.

No QUIC implementation, resolver/reuse behavior, product configuration,
deployment state, or production wiring was changed by this task.

## Acceptance evidence

- `rustc 1.95.0` / `cargo 1.95.0` were used for the local gates.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core -p mosdns-upstream-core --all-targets --all-features --locked` passed; every reported dns-core, upstream-core, reuse, secure-transport, resolver, and QUIC foundation test binary completed with `0 failed`.
- `cargo clippy --manifest-path rust/Cargo.toml -p mosdns-dns-core -p mosdns-upstream-core --all-targets --all-features --locked -- -D warnings` passed.
- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` passed.
- `git diff --check` passed.
- `python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-19-ci-rust-foundation-lint-doc-path-filter` passed for both context manifests.
- A YAML structure assertion verified the exact three `paths-ignore` entries under both `push` and `pull_request`, plus the unchanged `workflow_dispatch` trigger.
- The implementation diff for `b75bd7f` is limited to `.github/workflows/test.yml`, `rust/upstream-core/src/secure/dot.rs`, and this task's context manifests. No broad lint suppression was added.

## Completion decision

All acceptance criteria A1–A5 pass. The task is independent of the active
QUIC/HTTP3/DoQ planning work and is ready for Trellis archival.
