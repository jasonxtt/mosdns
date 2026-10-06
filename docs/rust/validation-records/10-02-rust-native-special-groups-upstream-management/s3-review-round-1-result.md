# S3 exact-source review — round 1 FAIL

Exact review range: `a9fe9f0eb5ceac81f7b43407acb2f0e81cdfaedf` →
`a5b3a936fde0cd5ea0ec0f856ad324c8a0c1665e` (tree
`3b435e0c5ba1620b5e0ac794a974b50f60535add`). The reviewer used the dedicated
user-selected C2C conversation. The formal Trellis controller recorded this
result against Slice 3; both findings remain open until a passing re-review.

## Findings

**P1-1 — startup lock decision and compiled configuration can come from different reads.**
`HostAssembly::from_config_file` made the managed writer-lock/recovery decision
from its first YAML read, then called `load_and_compile(path)`, which read the
root file again. A false→true opt-in change between reads could return a managed
runtime without the S3 writer lock or recovery owner. The reviewer requested
compiling from the exact preflight YAML snapshot or failing closed on ownership
mismatch, with a deterministic race regression.

**P1-2 — absence of an optional generated manual provider was not a candidate input.**
Generated profile compilation checked `rule/special_<slot>.txt` with
`try_exists()`, but did not record the absent path in `CandidateInputSet`. An
external create between compile and prepare/replace therefore escaped both
dependency checks. The reviewer requested recording and rechecking the missing
path, with an absent→created conflict regression.

## Remediation validation

Both findings have RED/GREEN regressions and the exact remediation source passes
the isolated native-host suite, fmt and strict Clippy checks. See
[`s3-status.md`](s3-status.md), `evidence/s3-p1-1-{red,green}.log`,
`evidence/s3-p1-2-{red,green}.log`, and
`evidence/s3-p1-remediation-source-manifest.json`. Same-ID exact-source
re-review is pending; this FAIL has not been converted into a PASS.

```text
FINAL: FAIL
```
