# Execution evidence

## S1 — trusted context / client_ip

Baseline aa32270aacf054af5b6cf668b77c9cdc990c6532; local branch rust. No push,
deployment, port53 or public DNS. Reviewer workspace/scope verified in new chat
https://chatgpt.com/c/6abf8d5c-9fb8-83e8-9fa8-dc220e52f750.

Remote source root `/dev/shm/mosdns-rust-client-ecs-20261002/src`, target and
logs in sibling `target` / `evidence`. Initial `df -h` showed root739MiB,
/tmp744MiB and /dev/shm2GiB free. Build uses one job, debug0/incremental0.
No unrelated paths removed. After final test OOM (confirmed kernel OOM receipt),
421342480 bytes of this task's executable targets moved into
`/root/mosdns-rust-client-ecs-20261002/linked-tests` with symlinks retained.
The repaired run passed; resource failure remains in s1-final-tests.log.

Commands: source copied via rsync excluding target; baseline fixture directory
`tests/phase5a-baseline` also copied. From remote source rust directory:

```
cargo fmt --all
cargo fmt --all --check
CARGO_TARGET_DIR=/dev/shm/mosdns-rust-client-ecs-20261002/target CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p mosdns-native-host --test client_context -j 1
CARGO_TARGET_DIR=/dev/shm/mosdns-rust-client-ecs-20261002/target CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p mosdns-native-host -p mosdns-sequence-core -j 1
CARGO_TARGET_DIR=/dev/shm/mosdns-rust-client-ecs-20261002/target CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo clippy -p mosdns-native-host -p mosdns-sequence-core --all-targets -j 1 -- -D warnings
```

Final tests: 352 passed / 0 failed, 30 result targets including doc tests;
s1-final-tests2.log exit0. s1-green7.log: 3 new public behavior tests pass.
Fmt s1-fmt2.log exit0; clippy s1-clippy3.log exit0. Source manifest
s1-source-sha256.json verifies 123 local/remote Rust sources/manifests equal.

Retained failed attempts: initial test used nonexistent assembly constructor;
missing log; then correct RED unsupported client_ip (s1-red3.log). Green
iterations include destructuring-edit compile failure, absent required forward,
missing TCP idle_timeout / listener-kind mismatch / enable_audit, an invalid
ExecutionControl test constructor; regression packaging initially omitted
baseline fixtures. Fixed fixture mistakes before claiming behavior evidence.
Clippy reports were fixed without suppression (explicit IPv4 match, checked
length conversion and raw-string delimiters). No failed/unrun check counts as PASS.

Implemented: trusted UDP/TCP peer input; mapped Unmap; explicit unknown default;
client_ip via existing immutable prefix catalog and negation; sequence-owned
query identity preserved by scope/snapshot/refresh recipe copying. Actual forged
ECS does not select client branches. Real UDP multiple loopback origins, IPv6
UDP, IPv6 and mapped IPv4 TCP, audit on/off tested. ECS policy, cache partition,
dump and S6 proof remain pending. S1 C2C verdict pending.
