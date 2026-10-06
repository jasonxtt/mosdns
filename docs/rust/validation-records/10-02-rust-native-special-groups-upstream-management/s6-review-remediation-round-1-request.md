# S6 exact-source remediation review — round 1

**Status:** sent once to the dedicated reviewer; it returned an accepted
`FINAL: PASS`. See
[`s6-review-remediation-round-1-result.md`](s6-review-remediation-round-1-result.md).
This is the
re-review of the same `P1-1` from
[`s6-review-round-0-result.md`](s6-review-round-0-result.md).

**Exact audit range:** parent
`e5d129feaf0adc1b75c23ddeeb650a94618903d2` → head
`eec6c4447e221ea304893dd8ddaf5b761a35cdb3`, tree
`44a41971f7729270c8d171a9c58612d2734243a7`. The first parent of the audit
object is the previously reviewed S6 candidate. The exact tested source set
contains 526 inputs; the local/isolated-host comparison and candidate-tree
comparison each report 526/526 matches and zero differences. The source-map
SHA-256 is `8d519ce6d9fea8867bbf438fc094edb63664b5239d29544b96b6c0ae0c0ef9cc`;
the manifest-file SHA-256 is
`65b533ebbf03f11b35132793bf451d91c4dd2253a07b8ac1a80e5bf5ae1051b7`. All
140 accepted S5 inputs still match.

The implementation keeps successful legacy diversion catalogs when another
optional endpoint returns 404, displays failed plugin tags and errors, and
preserves native fail-fast catalog loading. The RED regression first failed
with `HTTP 404 Not Found`; after the change, the full UI suite passed 13/13.
Maintained and compatibility UI builds passed with 619 and 612 modules. The
full `coremain` Go tests and embedded Go binary build passed. Both Vite builds
retain the >500 kB chunk advisory. Rust sources are unchanged; prior native-host
evidence is retained and was not rerun for this Go/Vue-only remediation. This
remediation did not add a legacy Rules page browser run; the regression is a
unit test of the helper used by `RulesManager`.

Evidence is in `evidence/s6-p1-1-*`, and the exact source manifest is
[`s6-p1-1-source-manifest.json`](evidence/s6-p1-1-source-manifest.json). The
initial FAIL and its strict-parser restatement are retained in the round-0
result. The real branch remains at
`79d93ae1b3b3253a2d09563444b251aad18eb5df`; the real index tree remains
`65b21ecc1a3e3fe14f50da3d49e0cb153c1d53ac`.

## Atomic message for the dedicated reviewer

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: 10-02-rust-native-special-groups-upstream-management
UNIT: Slice 6 P1-1 rereview
BASE_SHA: e5d129feaf0adc1b75c23ddeeb650a94618903d2
HEAD_SHA: eec6c4447e221ea304893dd8ddaf5b761a35cdb3
TREE_SHA: 44a41971f7729270c8d171a9c58612d2734243a7
PATHS: exact range; RulesManager/helper/test, bundles, S6 records/evidence.
PRIOR: P1-1: legacy optional 404 must not erase successful catalogs.
VALIDATION: 526/526 source hashes match host/tree; manifest SHA256 65b533ebbf03f11b35132793bf451d91c4dd2253a07b8ac1a80e5bf5ae1051b7; RED retained; UI 13/13; Vite 619/612; Go tests/build pass. Rust unchanged.
ACCEPTANCE: Keep fulfilled legacy catalogs and show failed tags/errors; native remains fail-fast. Resolve P1-1 from exact diff.
REVIEW: git_compare exact range only; no execution, edits, Trellis changes, or S7/cumulative verdict.
OUTPUT: Stable findings; final exactly FINAL: PASS or FINAL: FAIL.
```

The response was recorded with the formal Trellis `record-review` operation.
