# Slice 1 preflight and manifest v2

Status: replacement manifest v2 is frozen locally and on `mosdns-rust`; the digest-mutation regression and all 24 tuple validations pass. Independent Slice 1 review is pending. No official sample has run.

## Superseded v1 review

Manifest v1 at commit `31e86c90dab7ee8b6073dfec26b5942043b9558d` (SHA-256 `2ff4e0804b26af77db41c1357e107b00ea9539127665784087b29add3e43698f`) received reviewer **FAIL**. Its driver computed the expected manifest hash from the file being checked and forwarded that same value to the helper, so a changed manifest could validate itself. The review found no other issue in the evidence it inspected. No official sample ran; v1 files remain unchanged as the rejected review snapshot.

## Replacement freeze

- Manifest: [official-manifest-v2.json](official-manifest-v2.json), SHA-256 `3d8b76e8799bf709ef7e19df9edd6245936f05f9c4e09230dd49ecdaaf000f13`, bound to [official-manifest-v2.sha256](official-manifest-v2.sha256).
- Matrix driver: [run-official-matrix-v2.sh](run-official-matrix-v2.sh), SHA-256 `d7cd9cc786afcf343a0546a1ffe97883a064a39dd3686769128fc1077dcb8c18`. The manifest digest is pinned as a driver constant, and the driver checks both the file and sidecar against that constant before `--validate-only`, `--dry-run`, or `--execute`. The driver prints its own observed SHA for the execution transcript; this avoids a circular driver-hash/manifest-hash dependency.
- Regression test: [test-run-official-matrix-manifest-pin.sh](test-run-official-matrix-manifest-pin.sh), SHA-256 `c3ea0f117b2fbaec7d41b4af4d4e926da0444513c976d04a55387526ff6c4ded`. It first validates all 24 tuples with a stub helper, edits one manifest QPS field, then requires rejection before any further helper invocation. Running this against v1 reproduced the accepted-tampered-manifest bug ([red result](slice1-manifest-v1-mutation-repro.txt), SHA-256 `278f7b38f56f674f183c73ecb7190b1c6f278756d3ac644500e670c64212479f`); running against v2 passes ([green result](slice1-manifest-v2-pin-test.txt), SHA-256 `b9cee8ba46e61208bafe02ae67fbfb964a48388dedc934dcbda0be23cf8150b6`). The test starts no SUT and makes no official result directory.
- Candidate binaries and helper reuse the already built and smoke-tested bytes under `bin/official-v1/`; identities remain in `slice1-build-identities-v1.txt`. The seven frozen config/workload hashes remain unchanged from v1 and the archived baseline.
- The manifest's execution command and results root are versioned to `evidence-official-v2/` and `results/official-v2/`. The test VM received the v2 manifest, sidecar, and driver; VM-side sidecar validation passed. The remote driver SHA equals the local SHA above.

## VM validation, no-load state

Commands, each pinned to harness CPU 1:

```sh
taskset --cpu-list 1 bash evidence-official-v2/run-official-matrix-v2.sh --validate-only
taskset --cpu-list 1 bash evidence-official-v2/run-official-matrix-v2.sh --dry-run
```

Both commands validated the frozen 24 candidate/scenario/repetition tuples. The dry-run printed 24 unique scheduled rows in the frozen alternating Go/Rust order; combined output is [slice1-manifest-v2-dry-run.txt](slice1-manifest-v2-dry-run.txt), SHA-256 `4766a167392a63defd1a61306f093d47b50751674e5c644d31c1693881526e21`. The separate VM mutation test output is [slice1-manifest-v2-pin-test.txt](slice1-manifest-v2-pin-test.txt). After validation, `results/official-v2/` was absent and no task candidate or helper process remained. `--validate-only` and `--dry-run` do not launch a SUT.

The previous preflight's environment, build provenance, bilateral W1/W2/W3 smoke, fixed input hashes, and existing-service isolation findings remain applicable; see `slice1-preflight-v1.md`, `slice1-environment-v1.txt`, and `slice1-smoke-v1-status.tsv`. Production `mos` was not accessed.

Official samples remain blocked until an independent reviewer returns PASS for the exact pushed v2 manifest and driver hashes.
