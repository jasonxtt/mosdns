# S2 evidence

Base S1 commit e7e8a651ef3e5734d5bb3529775a96e3fc4f13b9 passed independent review before S2 began. Scope: hosts/TTL native root, branch, direct fallback target dispatch; dns-core atomic range TTL helper. Redirect/IP runtime pending.

Authoritative RED: s2-red.log, 4 valid public tests fail against S1 assembly's pending-runtime guard. Earlier compile fixture mistakes (constructor spelling/missing required forward declaration) were discarded.

GREEN: s2-validation.log, remote mosdns-rust test root /root/mosdns-rust-response-policy-20261002. Native-host + dns-core 356 tests PASS, workspace all-target clippy -D warnings PASS, fmt check PASS. Includes 6 actual UDP policy tests, 2 state-preservation tests and all-section TTL/OPT/malformed test. No local Rust build/test; no production/deploy/push.

Intermediate failures: long numeric test literal lint fixed; direct fallback test plugin references corrected to $local/$upstream. All final checks rerun after correction. Audit validates upstream attempt retained and Local supplier/no final selected endpoint after hosts override. Fail-closed assembly narrowed to pending redirect/new IP only.
