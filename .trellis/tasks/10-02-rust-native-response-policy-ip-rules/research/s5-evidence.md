# S5 composition evidence

S4 commit 7937a8f6879fbad7b2a15ebbe092335a739d5ab9 passed independent C2C iteration 5 before S5 began. This slice adds seven real public composition tests and shared fixture helpers only; no product source fix was required.

First-run behavior: four cases passed immediately. Retained-dump fixture attempted Save after listener shutdown and correctly received Closed; corrected fixture saves/flushes while owner remains open and sole query producer is quiescent, then stops/drains. This was a fixture lifecycle error, not a product RED failure. Two later tests also passed initially; no synthetic RED claim. Clippy caught helper let-and-return and was corrected.

Final s5-validation.log: remote full native-host **269 tests PASS**, workspace all-target clippy -D warnings PASS, fmt PASS. Seven cases prove (1) cache inner/outer TTL and aging; (2) bound v2 save/new owner import retained old hosts rules, durable no-refill flush and empty reload; (3) lazy refresh after disk hosts rule mutation keeps immutable snapshot; (4) IP condition selects real backup forward and preserves both attempts; (5) redirected prefer alternate QTYPE matcher sees current query; (6) held primary/secondary redirect siblings see distinct targets, canceled primary never contributes final CNAME/supplier; (7) IP predicates observe target/restored cached Answers at own boundaries.

Owner reconstruction/import is not an OS process restart. S6 explicitly supplies actual start/SIGTERM/restart/API/Vue proof. No new policy cache identity/generation or automatic invalidation. Same dedicated SSH mosdns-rust task root, no local Rust build/test, production/deploy/push.
