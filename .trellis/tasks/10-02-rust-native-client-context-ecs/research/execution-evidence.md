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

C2C S2 explicit FINAL: PASS on fad847ec..90e75e25, no findings. Automation
record-review advanced to Slice3; an unnecessary subsequent complete --unit
command was rejected as unsupported, with no state mutation.

## S3 — scoped supplier response

QueryView explicit forwarded ECS token, inherited only with existing policy.
Returned network supplier is the only echo input; generated/cache/local has no
invented scope. Strict response validation removes unsupported/mismatching or
duplicate ECS without rejecting valid DNS. Compression offsets are retained
while temporarily renaming option codes, then rebuilt by decoding/re-encoding.
Original client OPT size/DO is reconstructed, absent original OPT suppresses it.

Remote s3-tests.log passed full native-host regression (12 ECS tests at that
point); after adding shutdown and legal IPv6 cases, s3-final-wire.log passed all
14 ECS wire tests on the final test source. Product source unchanged between
those runs. s3-lib.log passed96; s3-clippy3.log strict all-targets passed.
Fmt applied remotely; s3-source-sha256.json confirms all125 source/manifests equal.
Real UDP: IPv4/IPv6 legal echo and invalid family/mask/scope/length/hostbits,
duplicate stripping, generated/no-clientOPT, nested handler+redirect restored
question, fallback winning scope12 versus losing scope20, preference local
suppression, cache-hit OPT/no fakeECS, exit/reject terminal and active upstream
shutdown with NoResponse. Existing fallback/secure/cache/cancellation regressions
passed. Retained RED generated echo; earlier testcompile missing getter and
wrong Cookie enum expectation; clippy test helper argument count/borrow fixed.
S4–S6 and whole-task PASS remain pending.

C2C S3 FINAL: PASS on 90e75e25..5f86b382, no findings; S4 authorized next.

## S4 — full ECS key / placement gate

Boolean named enable_ecs opt-in, quick false, current scoped wire base+ECS key;
canonical masked Go string (including family2 mapped To4 formatting with original
>=96 prefix). Ordinary noECS keys unchanged; strict invalid-profile bypass.
Full-key refresh singleflight: separate ECS networks build separate futures,
same-key follower never constructs one. No broader response-scope reuse.

Monotone validated-program suffix effect/cache summaries reject unsafe named,
quick, true and false placements through calls/jump/goto/try/fallback/preference,
recursion and inherited successors. Client matcher origins are gathered from
compiled configuration sources; invalid targets already fail program validation.
Actual child/inline boundaries differ from jump continuation. Diagnostics name
cache and offending policy/rule. Safe terminal and empty quick ECS tested.

Final native regression s4-final-tests.log passed; native lib96 passed,
all-target strict clippy s4-clippy4.log passed; fmt applied remotely;
s4-source-sha256.json confirms all127 local/remote source/manifests equal.
Public ECS tests: ecs_cache6 and ecs_wire16 pass; ordinary slice1_cache8 pass.
Real UDP forward->truecache uses one upstream request for cold/hit and hit has
no fabricated ECS. Existing config test now accepts true instead of asserting
obsolete unsupported behavior. Full native regressions retain secure/fallback/
audit/cache/TCP/UDP/management behavior.

Retained failures: initial new fixture omitted required enable_audit and had
wrong expected suffix length; executable target enum is Fixture, not External;
clippy redundant trim fixed. First full s4-tests.log found compressed query
regression in strict noOPT handling: noOPT can point into trailing name storage.
Removed that invalid rejection; added real UDP generated-OPT re-encode proof.
s4-tests2.log passed, followed by focused and final full runs after mapped-string
coverage. Only task-owned binaries relocated as earlier for memory headroom.
Dump/management normalization and final proof remain S5/S6, not claimed here.

C2C S4 FINAL: PASS on 5f86b382..6ac4570d, no findings; S5 authorized next.

## S5 — canonical v2 semantic interoperability

Actual Go cache-package task-only generator uses getMsgKeyBytes and writeDump
for family1, family2, mapped family2 and ordered noncanonical host-bit collisions.
Go gzip block payloads are concatenated in controlled order using the unchanged
v2 header/schema. Native import canonicalizes to four keys; last IPv4 collision
retains answer192.0.2.12, domain_set and original wall timestamps. Disabled owner
rejects all ECS entries. Native dump reimport proves restart semantics; actual
Go readDump/Get verifies exported canonical keys and explicit legacy-host-bit
refill boundary. Source for the task-only Go proof is research/go-proof/.

Strict suffix parsing validates full base/suffix lengths, UTF8, supported IP
family/masks, scope0 and no trailing components; hostbits are masked only after
validation. Entire decoded dump validates including expired entries before
existing atomic owner merge. Public HTTP test loads real Go fixture, rejects
malformed final suffix with400 and observes unchanged show output. API renders
name and ECS separately, preserving DNS/flags syntax. Generation and expired
suffix rejection covered through actual public management entrypoints.

Retained failures: isolated Go source copy initially omitted existing embedded
UI assets (copied existing bundle; no frontend build); initial Rust test imported
private module (fixed public reexports); genuine behavior RED shows old native
adapter rejecting real ECS fixture. VM rustc/linker OOM required relocation of
only task-owned linked test executables; no unrelated cleanup. New HTTP fixture
needed required args and a forward plugin. Enhanced Go semantic assertion first
compared randomized DNS IDs; corrected to parsed answer/timestamps/domain_set.
Final check summaries and exact source manifest follow after completion.

Final S5: native297 passes across26 targets (s5-native5.log); lib97, strict all-target clippy passed (s5-clippy.log), fmt remote. All128 Rust source/manifests equal in s5-source-sha256.json. Actual enhanced Go reader passed (s5-go-read-native-semantic.log). Go fixture SHA256 aef1657db5082cb51565637e86e80d1aaf5db7984c8b110e717df9771df9de61.
