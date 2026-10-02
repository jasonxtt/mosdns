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

C2C S1 verdict: explicit FINAL: PASS, no P0/P1/P2/P3 findings for exact
 aa32270aacf054af5b6cf668b77c9cdc990c6532..fad847ec73639a619b1b64502ba52a1e9055c4ff.
Reviewer separately confirmed trusted propagation, unknown, mapped normalization,
forged ECS independence, audit-off and grammar reuse. S2–S6 excluded.

## S2 — handler / outgoing wire

Exact source manifest s2-source-sha256.json: 125 local/remote Rust sources and
manifests equal. Named ecs_handler + quick ecs compile to scoped External
policies; immutable admission QueryView, current policy provenance, strict
native ECS profile and local outbound OPT reconstruction. No supplier echo or
cache/dump acceptance is claimed yet. Real UDP upstream captures IPv4/IPv6,
forward/preset/send/default/legacy/masks, original other OPT/DO, noOPT creation,
non-IN preservation, later handler retention and malformed-local SERVFAIL.

Remote cargo fmt --all --check and clippy native-host --all-targets -D warnings
passed (s2-clippy2.log). Full native-host regression passed (s2-tests3.log).
Retained failures: s2-red unsupported handler; unused field warning removed;
renamed private runner unit fixture fixed; full test compile OOM s2-tests.log;
task-owned binaries moved off memory disk before successful rebuild. The extra
retention test initially placed upstream outside its exec-list boundary and
expected policy leakage; corrected to consecutive rules within one successor.
s2-tests2.log retains this failed test. No unrelated resource cleanup.

Automation helper has a multi-unit defect: it chooses global rereview after
S1 bootstrap but requires a previous head for the new S2 unit. First submission
of each later unit is pinned as a first unit review with the same verified
reviewer; only the bootstrap flag is locally reset during helper validation,
then restored. No approval/PASS is inferred or skipped; actual C2C response is
required before advancing, and follow-up findings use normal remediation.
