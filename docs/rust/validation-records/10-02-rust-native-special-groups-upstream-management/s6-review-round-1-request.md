# S6 exact-source C2C review request

**Task:** `10-02-rust-native-special-groups-upstream-management`  
**Formal context:** `codex_01a0fc3e-79ef-7e23-9943-34062de8ded0`  
**Unit:** Slice 6 — maintained Vue workflow  
**Reviewer:** dedicated `Review Rust Native Groups` conversation  
**Conversation:** <https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6ac058cd-6138-83e8-af54-758358f73006>

The exact independent audit range is parent `63b836b7ba0d0046cbb28763950f206b320cf201` to candidate `e5d129feaf0adc1b75c23ddeeb650a94618903d2` (tree `b730e97b77f61e53e87c417d43d6c2948aecbca3`). Review every path in that committed range with `git_compare`; do not substitute the working tree, index, or branch HEAD. The real branch remains at `79d93ae1b3b3253a2d09563444b251aad18eb5df` and its index tree remains `65b21ecc1a3e3fe14f50da3d49e0cb153c1d53ac`.

The 526-path tested-source manifest is `evidence/s6-source-manifest.json`, SHA-256 `a37b9f2f530e89db3527755efb7b47dcf11a6a1451949a522e48161ac2dbeb06`. Local and isolated-host inputs match with zero differences; all 140 accepted S5 source inputs also match. The principal code scope is the unmatched-Go-route 404 behavior and regression, maintained Vue native-management capability discovery/workflows, their embedded bundles, and the UI tests. The range also publishes the S5 exact-source PASS record and the S6 validation record/evidence.

Validation evidence: maintained and compatibility Vite builds passed (619 and 612 modules); UI tests passed (12/12); focused and full `coremain` Go tests passed; final Go binary build passed; the native-host build passed, with Rust unchanged from accepted S5. Controlled browser/DNS/HTTP evidence and limitations are summarized in `s6-status.md`. All runtime and build checks used only the isolated `mosdns-rust` host and high ports; no public DNS or port 53 was used.

Check that native capability fallback occurs only on HTTP 404, runtime-supported controls and disabled unsupported records remain truthful, source-file validation and rename preserve the native workflow contract, and the Go unmatched-route change retains its help body and existing method-mismatch behavior. Include any actionable issue with a stable finding ID. Return exactly one explicit terminal result: `FINAL: PASS` only when the whole exact range has no actionable findings; otherwise list stable findings and `FINAL: FAIL`.

## Message sent to the dedicated reviewer

Exactly one request was sent after creating the audit object and recording the
range, target and evidence above:

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK: .trellis/tasks/10-02-rust-native-special-groups-upstream-management / Slice 6
REQUEST_KIND: review
REPO: mosdns-rust@rust
BASE_SHA: 63b836b7ba0d0046cbb28763950f206b320cf201
HEAD_SHA: e5d129feaf0adc1b75c23ddeeb650a94618903d2
TREE_SHA: b730e97b77f61e53e87c417d43d6c2948aecbca3
PATHS: inspect every path in the exact committed BASE_SHA..HEAD_SHA range; focus on Go unmatched-route status/regression, maintained Vue native-management flow, embedded bundles, and UI tests. The range also publishes the accepted S5 result and S6 records.
VALIDATION: 526-path manifest SHA-256 a37b9f2f530e89db3527755efb7b47dcf11a6a1451949a522e48161ac2dbeb06; local/isolated-host and audit-tree hashes match, including 140 accepted S5 inputs. Vite builds 619/612 modules pass; UI 12/12 pass; focused/full Go tests and final Go build pass; native-host build passes (Rust unchanged since S5). Isolated browser/HTTP/DNS evidence is in the task validation record. No public DNS or port 53.
ACCEPTANCE: review Slice 6 only from exact committed git_compare(BASE_SHA, HEAD_SHA), not worktree/index/branch HEAD. Check 404-only native fallback, truthful supported/disabled controls, local-source validation/rename behavior, and Go unmatched-route 404 while preserving method-mismatch behavior. Review the source and evidence; do not execute, edit, or change Trellis state. Do not infer S7 or cumulative completion.
EVIDENCE: docs/rust/validation-records/10-02-rust-native-special-groups-upstream-management/s6-status.md and evidence/s6-source-manifest.json plus linked s6-* proof.
OUTPUT: stable finding IDs if actionable; end exactly FINAL: PASS or FINAL: FAIL.
```
