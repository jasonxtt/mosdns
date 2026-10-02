# S4 execution evidence — 2026-10-02

SuccessorRecipe owns a cloned scope/state and binds a new root control. CacheOwner owns singleflight refresh tasks, independent transport cancellation, five-second deadline and 256-key no-queue cap. Background nested Lazy lookups continue inline with the same 64-fuel root. Foreground retains stale response while detached publication captures its own domain_set. Background upstream attempts count; client audit/admission do not. UDP/TCP stop admission and cancel/join owners before closing transports.

Remote exclusive workspace: /root/mosdns-rust-cache-lifecycle-20261001/rust on SSH alias mosdns-rust. No local build, commit, push or deployment.

Red test: positive lazy config was rejected before implementation (/tmp/cache-s4-red.log). Intermediate compile/clippy failures are retained in /tmp/cache-s4-check.log and final rerun output /tmp/cache-s4-final.log. Final cargo fmt --all --check, cargo clippy --offline -p mosdns-native-host -p mosdns-sequence-core --all-targets -- -D warnings and cargo test --offline -p mosdns-native-host -p mosdns-sequence-core -j1 all succeeded. Total test count follows execution output.

Behavior proof includes gated real UDP nested A/B cache refresh outliving client, follower merging, unchanged audit/client counters, exact upstream attempts; owner 256 permits, stop/drain and five-second timeout retaining old Lazy value; captured successor state isolation and 64 dispatches exhausting new fuel.
