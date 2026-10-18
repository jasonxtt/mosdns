# CI test robustness fixes (2026-10-18)

BASE `11c9ada1`. Tests only; no product code changed. Reason and fix per test: `.trellis/tasks/10-18-rust-native-ci-test-fixes/research/design.md`.

- `switch_state::tests::externally_changed_state_files_conflict_without_replacement`: the product compares `(st_dev, st_ino)` plus the sha256 of the content (not mtime or size). The test's "delete then recreate with the same content" can get the freed inode back (seen on the GitHub runner), which is indistinguishable from no change. The old file is now renamed aside and kept until the end of the test, so the new file always has another inode. Not a product gap: a real replacement keeps the old inode alive while the new file is created.
- `tests/f1_system.rs`: a freshly copied executable can be "Text file busy" when another test thread forks at that moment. New `spawn_retrying_busy` retries only `ExecutableFileBusy`, 50 × 20 ms, then returns the real error. (`f2_diagnostics.rs` already had an equivalent retry.)

## VM results (`mosdns-rust`, `-j2`)
| Step | Result |
| --- | --- |
| `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 / 0 |
| `cargo test --workspace --no-fail-fast --locked` | 1491 passed, 0 failed, 1 ignored (unchanged counts) |
| the switch-state test, 20 consecutive runs | 20 / 20 pass |
| the whole `f1_system` file (6 tests, concurrent), 20 consecutive runs | 20 / 20 pass (before the fix the same test failed 1 of 6 runs on this VM) |
