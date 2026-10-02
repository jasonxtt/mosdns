# S3 evidence

S2 commit 8ce75e1257bc8989207a720f97a29af86d745064 independently passed before S3 began. Scope: current owned QueryView, scoped redirect, preference probe query/state alignment, supplier-kind inheritance, exit completion and attempt detail preservation across branch wrappers.

RED s3-red.log: 2 valid real-UDP redirect fixtures fail S2 pending guard. Additional s3-exit-red.log: named fallback exit incorrectly executes parent TTL (42 instead of CNAME TTL1) before the wrapper fix. These are behavior failures, not compilation errors. Intermediate fixture corrections include goto $main spelling and expected existing NoResponse terminal on fuel exhaustion.

GREEN s3-validation.log: remote full native-host **258 tests PASS**, workspace all-target clippy -D warnings PASS, fmt PASS. 14 public UDP policy tests now include target-peer wire, nested NXDOMAIN/SOA, compressed CNAME preservation, original/target cache keys and warm hits, correct Cache supplier, OPT/flags/QTYPE, direct fallback, non-IN no-op, exit/ScopeAborted publication and cycles. Unit test proves runtime error, cancellation and shared root fuel return typed failure, original query state and inherited response without decoration.

Named sequence/direct fixture targets and fallback/preference wrappers retain Exited rather than manufacturing Completed. Root resume Exit invalidates outer cache watch; inner cache valid own boundary can still store target wire. Audit branch collector retains actual attempt details and source selection distinguishes Upstream/Cache/Local. No schema additions.

Test root /root/mosdns-rust-response-policy-20261002 on SSH mosdns-rust. No local Rust test/build, production/deploy/push. Expanded IP evaluator remains explicitly gated for S4. S5/S6 composition proof still pending; no claim of full task completion.
