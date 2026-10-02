# S1 evidence — 2026-10-02

Scope: typed hosts/redirect/ttl/IP config, immutable bounded startup loaders and explicit incomplete-runtime assembly guard. Runtime wire policies/redirect/IP evaluation remain S2–S4.

Remote test root: `/root/mosdns-rust-response-policy-20261002`, SSH alias `mosdns-rust`. No local Rust build/test, production traffic, deployment or push. Reused the completed cache task's target directory within these owned test roots to avoid duplicate disk usage. Root tests fixtures copied for include_str paths. Sources touched after rsync to force correct Cargo freshness.

Authoritative RED (`s1-red.log`): baseline HEAD 2c059b0ea1ae19b52fbe92a079d43bee863ccc4a source, two new grammar tests fail because hosts/ip_set are unsupported; identical valid baseline config control passes. Earlier invalid-fixture red attempts and one stale-artifact baseline attempt were discarded, not counted as proof.

GREEN (`s1-validation.log`): workspace all-target clippy -D warnings PASS; full native-host suite **241 passed, 0 failed** (including 9 new public config tests); cargo fmt --all --check PASS. Commands use CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0, offline, -j1.

Intermediate failures: initial fixture required log/listener/enable_audit; corrected all. Clippy caught helper placement and local imports; corrected. Full regression caught a historic negative test rejecting valid IPv6 resp_ip; narrowed it to ::1/129, with new positive IPv6 coverage. Earlier library-only count was provisional; final authoritative counts are in this log.

Review route: dedicated C2C binding explicitly set to new task conversation ID 6abf42c0-0b24-83e8-a56e-cdd6427a976a. CLI requires canonical /c/ URL; it is the same project conversation. Doctor PASS. No automation run was synthesized; inline execution remains the authorized controller mode.
