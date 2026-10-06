# Cumulative review result — round 1

Recorded 2026-10-04 from the dedicated reviewer. Exact base
`79d93ae1b3b3253a2d09563444b251aad18eb5df`; head
`1495c508fe5ead3dc2ba9fa6c7c435a0a220cf1e`; tree
`b06ced93248124644b89a5a52bc420b15ee6377b`.

## Reviewer response

P1-1: rust/native-host/tests/special_groups_config.rs:413-493 and the S7 live-proof.json expose a cumulative contract gap on cache hits. The frozen contract requires cached normalized answers to retain the actual supplying entry (native-special-groups.md:59,95), but the regression only asserts final_upstream == actual_supplier on the miss and merely checks CacheStatus::Hit on the second query. The S7 live proof confirms the missing assertion reflects runtime behavior: responses served from the group caches have the correct group/sequence and no new peer request, but omit final_upstream/selected supplier provenance, while corresponding misses retain it. Preserve the cached answer’s original supplier identity and restore it into audit provenance on hits without fabricating a new upstream attempt; add miss→hit coverage that asserts the same real supplier survives the hit. [open] FINAL: FAIL

The displayed final token was on the finding line. Recording it separately does
not alter the explicit FAIL. P1-1 is open; whole-task PASS is not claimed.
