# S4 evidence

S3 supplemental exact commit aea396532e99b6209b1c680b978413f03653910e received iteration 4 FINAL PASS before S4. S4 implements real read-only Answer A/AAAA OR matching over all compiled Rc prefix snapshots, removes PendingIpMatcher and runtime_ready/assembly gate, and exports ResponseIpRuleConfig as a typed public descriptor.

RED s4-red.log: real named/CIDR/dual-stack sequence test fails S3 pending IP guard. GREEN s4-validation.log: full native-host + matcher-core 314 tests PASS; workspace all-target clippy -D warnings PASS; fmt PASS remotely. Named/inline/empty OR, IPv4/IPv6 negatives, mapped IPv6 and zero prefix proved via UDP. Text file mutation/removal leaves current result unchanged. Unit coverage proves Authority/Additional/CNAME-only non-match, malformed error and no state mutation; matcher-core goldens cover normalized prefix boundaries.

Same dedicated SSH mosdns-rust root /root/mosdns-rust-response-policy-20261002; no local Rust test/build, production/deployment/push. No IP management/hot reload/provider sets/binary formats added. S5/S6 pending.
