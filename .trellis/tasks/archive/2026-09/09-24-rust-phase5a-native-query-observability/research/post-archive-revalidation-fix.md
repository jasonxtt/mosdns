# Post-archive offline revalidation fix

Closing fix for the archived task. Nothing here reopens Phase 5A measurement:
no Rust or Go product change, no workload or gate change, no new traffic, no
re-measurement. Original measurement results, journals, identities, manifests,
frozen tool copies and error logs are unchanged.

## Root cause

Two independent assumptions broke when this task directory moved to
`.trellis/tasks/archive/2026-09/`:

1. `m10-route-oracle.py::derive` built its `git show` path from `__file__`,
   so it asked commit `daeff167` for
   `.trellis/tasks/archive/2026-09/…/research/m10-preflight/identity.json`.
   That commit recorded the pre-archive path
   `.trellis/tasks/09-24-rust-phase5a-native-query-observability/research/…`,
   so `git show` failed with `path exists on disk, but not in the specified
   commit` and the whole offline proof could not be recomputed.
2. Three tests derived the repository root by fixed depth
   (`Path(__file__).resolve().parents[4]` / `parents[3]`), which after archiving
   resolved to `.trellis/tasks/scripts/…` and
   `.trellis/tasks/tests/…` instead of the repository.

## Changes

| File | Change |
| --- | --- |
| `repo_paths.py` (new) | Single `repo_root()` ancestor search that requires every marker `go.mod`, `rust/Cargo.toml`, `tests/phase5a-baseline`; raises `ValueError` when no ancestor qualifies. |
| `run-m10-w3.py` | Adds `TASK_DIR`/`TASK_PREFIX`, `committed_research_root(head)` and `committed_bytes(head, path)`. `verify_reviewed_tools` now reads the reviewed preflight and tools from the measuring commit's own recorded research path, failing with `exactly one … research directory` when that path is absent or ambiguous. |
| `m10-route-oracle.py` | `derive` resolves the measured research directory from the measuring commit and reads the preflight plus every `local_tools` entry from there, keeping SHA-256 verification against the frozen `identity.json`. Current archived files are used only as the repaired analysis code. |
| `test_m3_measurement.py`, `test_m9_remaining.py`, `test_m10_w3.py` | Replace the fixed-depth repository lookups with `repo_paths.repo_root(...)`. The `test_m10_w3` reviewed-tools fake now answers the new `git ls-tree` tree read. |
| `test_archived_task_paths.py` (new) | Regression: the archived directory, a file inside it, and the frozen `run-m5-w1.py` / `run-m6-w1.py` derivations all resolve the same real repository root; a tree without the markers is rejected instead of silently guessed. |
| `test_archived_m10_derive.py` (new) | Regressions for the whole offline `derive`, run from the archived checkout over a throwaway repository whose single commit is the measuring commit: the recorded path is the pre-archive one, absent/ambiguous/incomplete task directories are rejected, a missing history path fails instead of crashing, an unreported hash is rejected, and a repair to the current copy plus a hostile file at the old path still cannot substitute for measured history. |

No pre-existing test was deleted, weakened or skipped.

## Red → green

New regressions against the pre-fix code (`git show HEAD:` copies of the five
touched files, run from an archived-shaped directory): 8 of 10 error, including
`git show … .trellis/tasks/archive/…/m10-preflight/identity.json` returning exit
128 and `committed_research_root`/`committed_bytes` being absent.

Full existing suite from the real archived directory
(`python3 -m unittest discover -s .trellis/tasks/archive/2026-09/09-24-rust-phase5a-native-query-observability/research -p 'test_*.py'`):

- before: 70 tests, 10 failures, 4 errors, 1 skipped
- after: 80 tests, 0 failures, 3 errors, 1 skipped

The 3 errors are `bind`/`listen` on `127.0.0.1` returning `PermissionError:
[Errno 1] Operation not permitted` — this sandbox denies local port binding.
They are unrelated to the archive layout (no repository path is involved) and
are the only failures of that suite when the same tests run without that
restriction.

## Offline re-derivation

Run locally against the frozen evidence, writing a new directory beside the old
proof and starting no SSH, SUT, fixture or query traffic:

```
m10-route-oracle.py \
  --raw-root   /Users/tom/.codex/artifacts/mosdns-phase5a-m10-w3-daeff167-20260926 \
  --result-root /tmp/claude-501/m10-offline-proof-repaired-20260926T225135 \
  --workload   tests/phase5a-baseline/workloads/routing.jsonl \
  --cleanup-proof    …/postbatch-proof/cleanup-verification.json \
  --postbatch-identity …/postbatch-proof/identity.json
```

Result: 9 sessions, 27,000 verified queries, 45,000 verified route events, 36
cleanup receipts matching the recorded PID/start identity, `new_queries: 0`,
`original_verdict_unchanged: true`, all four paired median gates still pass, and
the legacy `exit=1` routing-oracle line plus `original_runner_exit: 1` are
carried through on every row. The output is labelled an independent offline
route proof under the same numeric gate; it does not claim the original runner
succeeded.

## Comparison with the earlier proof

Identical: the 9 `rows.json` entries, all four paired median ratios and the
`assessment.json` pass/fail verdict, every `route-proofs.json` entry,
`original_driver_assessment_sha256`, `original_rows_sha256`,
`cleanup_proof_sha256`, `postbatch_identity_sha256`, `new_queries` and both
original oracle counts.

Expected to differ: `oracle_sha256`
(`55bff2ec…` → `c35987fe…`) — the analysis tool was repaired, and that field
records the tool, not the measurement. The two directories differ in location,
and the new directory is deliberately new so the old proof is left untouched.

## Frozen evidence check

Re-verified after the fix: 341/341 `manifest.json` entries and 349/349
`bundle-manifest.json` entries still hash as recorded,
`bundle-manifest.json` still matches its `.sha256` sidecar, and no original
result, journal, identity, preflight, manifest, frozen tool copy, FAIL log or
saved proof was written. `git status` on the task directory lists only the
research scripts and tests above.
