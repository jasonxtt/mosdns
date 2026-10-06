# S6 exact-source C2C review — round 0

Range: `63b836b7ba0d0046cbb28763950f206b320cf201` →
`e5d129feaf0adc1b75c23ddeeb650a94618903d2` (tree
`b730e97b77f61e53e87c417d43d6c2948aecbca3`). Reviewer: [Review Rust Native
Groups](https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6ac058cd-6138-83e8-af54-758358f73006).

The reviewer returned this actionable finding and terminal result:

```text
P1-1:  webui-log/src/components/RulesManager.vue:231,262  replaces the legacy diversion-catalog  Promise.allSettled  behavior with  Promise.all  for both native and legacy runtimes. After capability discovery falls back to Go on the newly-correct 404, any absent optional diversion plugin now returns 404, rejects the whole batch, clears diversionRules, and reports a page-wide load failure, discarding rules from catalogs that did succeed. This regresses the existing Go workflow precisely because S6 changed unmatched optional routes from false-success 200 to truthful 404. The committed Go compatibility fixture contains no diversion providers, while the S6 browser proof does not exercise the legacy Rules page, and the 12 UI tests have no partial-catalog legacy case. Preserve fail-fast behavior for native if required, but restore per-catalog tolerance for the legacy path (or otherwise treat missing optional legacy catalogs independently); add a regression where capability discovery returns legacy, one diversion catalog is 404, and another succeeds without losing its rules. [open] FINAL: FAIL
```

Because the finding and final token shared one line, the strict workflow parser
initially classified the raw response as pending. After the reviewer completed
a formatting-only restatement, the formal `record-review` operation accepted:

```text
P1-1: Legacy RulesManager catalog loading uses Promise.all, so an optional 404 discards successful catalogs. [open]
FINAL: FAIL
```

The formal run records S6 as `remediating`, with the same `P1-1` open. The
finding is valid: `RulesManager.vue` uses `Promise.all` for legacy catalogs, so
an optional route's truthful 404 rejects the combined request and the catch
clears every catalog, including successful ones. S6 remediation will preserve
successful Go catalogs, keep each missing catalog visible as a partial-load
error, and add a regression for capability 404 plus one successful and one
missing legacy catalog. This FAIL remains part of the public validation record.
