# Final review and archive readiness

Date: 2026-09-24. Reviewer: the selected independent Codex conversation `01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3` (“成为001号 reviewer”). The user requested a final check and archive if the task passed.

## Original result review

The reviewer returned **FINAL: PASS** for exact pushed range `aa7e0bb834060d479876dfd2f5f70ab2c572af64..1abf1e20c954e8d25157f19b31d26189a8323ff1`. They independently matched the 12 fully paired groups' latency/resource summaries with the reconciled aggregate, verified W1 forwarding counters, W2 miss and TTL eligibility, W3 route-leg counts and invalid-attempt treatment, and checked all 718 raw files against the committed index and remote artifact hashes. The PASS covers only the report/evidence and bounded W1/W2/W3 conclusions. It is not a whole-product performance verdict, recovery/capacity proof or deployment approval.

## Archive-path remediation

Before archive, a separate local check found that the report's task-relative evidence links would break after Trellis moved the task, and that the common runner's default `MANIFEST_PATH` pointed at a nonexistent current-task file. Commit `7a26521939a97f2c14d6578f0a13022c74f23ec4` changed report/preflight links to their future archive targets but attempted to restore the older Go baseline manifest as a default. The reviewer returned **CLOSEOUT FIX: FAIL** for range `1abf1e20c954e8d25157f19b31d26189a8323ff1..7a26521939a97f2c14d6578f0a13022c74f23ec4`: the archived 2026-09-21 manifest exists but its schema is incompatible with the current v8 official helper. The reviewer accepted the link corrections.

Commit `c04210c88841cdc09909ebf81e7f7278273e2221` removed the incompatible implicit default. Current `official` mode now rejects an absent `MANIFEST_PATH` with exit 2 before creating results or starting helper/SUT; the README documents the explicit compatible manifest and SHA requirement and how to reproduce the older baseline from its frozen tool revision. The same reviewer returned **CLOSEOUT FIX: PASS** for exact range `7a26521939a97f2c14d6578f0a13022c74f23ec4..c04210c88841cdc09909ebf81e7f7278273e2221`. The frozen v2 manifest/driver hashes, 718 raw files, and original report's measured values were not changed by either closeout commit. A future measurement with the modified runner requires a newly reviewed manifest/tool hash; it cannot be backfilled into this matrix.

## Independent local checks before archive

- `go test ./tests/phase5a-baseline/...` and `go vet ./tests/phase5a-baseline/...` passed.
- `bash -n scripts/run-phase5a-baseline.sh`, `task.py validate`, and `git diff --check` passed.
- A subprocess check of `RUN_MODE=official` without `MANIFEST_PATH` returned exit 2 with the explicit-manifest error and created no result directory.
- Manifest SHA-256 remained `3d8b76e8799bf709ef7e19df9edd6245936f05f9c4e09230dd49ecdaaf000f13`; matrix driver SHA-256 remained `d7cd9cc786afcf343a0546a1ffe97883a064a39dd3686769128fc1077dcb8c18`.
- The future archived paths of all report evidence links and the preflight-to-report link resolved to present files. On `ssh mosdns-rust`, `sha256sum --status -c` passed for all 718 entries in the official raw-result index. No benchmark was rerun or production service touched during this closeout.

Only task-local review records and the report's status line are updated after the reviewer PASS. Normal Trellis finish/archive is authorized by the user's request; the task remains non-production and all invalid/partial points remain visible.
