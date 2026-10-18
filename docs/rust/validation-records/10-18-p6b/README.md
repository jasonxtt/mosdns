# Phase 6b — retire `native_management` (2026-10-18)

BASE = `74f1aa31`. Nothing is committed. Builds and tests ran on the VM `mosdns-rust` (`-j2`), directory `/root/p6b-20261018` (deleted afterwards). Inventory and decisions (every symbol → keep / delete / change): `.trellis/tasks/10-18-rust-native-p6b-retire-native-management/research/design.md`; removed tests by name: `research/removed-tests.md`.

## What changed
- **Removed**: the `native_management` switch and everything only it used — `ManagedProfile`, candidate profiles, `ManagedStore` and its `webinfo/native_management.{transaction,staging,retired,lock}` journal, `compile_managed_candidate` / `apply_candidate` and the managed branches of `apply_config`, the managed special-groups / upstream / diversion / manual-rule HTTP handlers, the managed router in the cache-reuse proof, the start-up store open / recovery / pending-transaction gate, `examples/inspect_managed_config.rs`, `docs/rust/examples/native-managed-empty/`, and the `fs2` dependency.
- **Kept** (the standard mode uses them): the `blocking_io` worker (thread renamed `mosdns-blocking-io`), `CandidateInputSet`, `sha256`, `lexical_path`, `apply_config` + `FileCommit` / `DerivedFile` (used by `apply_settings` and `apply_groups`), standard special groups, typed settings, the rule-file `ManagedDomainSet` management (an unrelated meaning of "managed").
- **Behaviour**: a configuration with a top-level `native_management` key (any value) is refused by load, `check -c` (global finding, exit 1) and `start`: `native_management was removed; use the standard special_groups configuration`. Leftover `webinfo/native_management.*` files are neither read nor deleted. Without a standard profile the special-groups / upstream-config / diversion routes answer as they did for non-managed configurations (GET `[]`, 400 `native special_groups profile is disabled`). `check` no longer prints the `SKIPPED managed-store …` line (its step no longer exists). Two capability reason strings no longer mention the managed configuration.

## Results (VM)
| Step | Result |
| --- | --- |
| `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 / 0 |
| `cargo test --workspace --no-fail-fast --locked` | **1490 passed, 1 failed, 1 ignored** in the first full run, the failure being `f1_system::a_restart_goes_ahead_even_when_the_final_save_fails` (`Text file busy`, a spawn race in the test); rerun alone 2 of 3 passes. The **BASE** tree fails the same test 1 of 6 runs, so it is a pre-existing flake. All other 1491−1 tests pass (1491 in the earlier full run, which had no flake). BASE had 1561 passed / 4 ignored |
| `mosdns check -d config_rust -c config_custom.yaml` | new binary and BASE binary: `SUMMARY gaps=0 cascades=0 globals=0 definitions=128` |
| Reduced E2E-1 (200 names → 956 rows per runtime), Go reference from `tools/go-reference/build.sh` | new vs BASE native binary: identical (`semantic_equal 401`, `semantic_different 555`, `final_sequence 509, final_upstream_group 401, domain_set 46, effective_tag 46, ttl_policy 4`; `mismatches.json` 555 rows equal); both exit 0, original service unchanged, namespaces cleaned |
| Browser smoke of `/` (Chrome pane through an ssh tunnel, host in a private netns with the E2E mocks) | upstream page shows the standard groups; "新增专属分流组" saved a group through the UI (`GET /api/v1/special-groups` shows slot 50 with its group reference); system page saved an ECS override `2001:db8::1` (`/api/v1/settings/effective`: source `settings`); group deleted (204) |

## Test-count change
Rust 1561 → 1491 passed (ignored 4 → 1): −76 test functions (listing in `research/removed-tests.md`: `special_groups_config` 12, `special_groups_whole_chain` 3, `special_groups_supervisor` 22, `runtime_snapshot` 18 (managed apply/journal/crash/cache-closure), `transaction` 18, `assembly` 1, `switch_admission_tests` 1 (renamed, redone), `m1_f1_typed_settings` 1; 3 of them were `#[ignore]` crash probes) and +3 added (2 new rejection / leftover-state tests, 1 renamed). Two generic runtime tests (removed-binding publication, cleanup closes transports) and three switch-admission tests were kept by porting them to the standard apply path / plain YAML.

## Not done / limits
- Coverage that existed only through the managed mode and is not reproduced for the standard mode: cache reuse across a special-group edit (the standard mode refuses reuse for `sequence_special_*` closures, unchanged), the managed crash/recovery matrix (no journal any more), and the managed HTTP CRUD suites (standard CRUD is covered by `m1_e_standard_groups`).
- The `f1_system` test race is not fixed (out of scope; it fails the same way on BASE).

## Planner verdict (2026-10-18)

Accepted. Codex round 1 `FINAL: PASS` with no findings. Checked: the only remaining
`native_management` references in `rust/` are the load-time refusal, its tests and
history comments; the standard-mode `managed.rs` (file-backed `domain_set` providers) is
unrelated and kept; `fs2` was used only by the removed lock; `check` on `config_rust`
and the reduced E2E-1 differential are unchanged; no production addresses in the diff.
