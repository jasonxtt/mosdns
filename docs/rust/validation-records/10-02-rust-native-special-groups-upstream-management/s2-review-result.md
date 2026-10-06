# S2 exact-source review and remediation

Human-selected replacement reviewer: https://chatgpt.com/c/6ac058cd-6138-83e8-af54-758358f73006.
Original range 706ab902ceb5d3096c9b18513ae6c651ffc4abce → c6abbe42ba12e5bda86188b81547a8d86edd10c8.
The original review transport failed; the human opened a new conversation and
resent the unchanged request. Original authorization and failed submission were
preserved. The actual replacement reviewer is recorded with the result.

The committed evidence is internally consistent: s2-native-host-reviewed.log totals 323 passed / 0 failed / 0 ignored across 29 suites; reviewed Clippy/fmt evidence is present; s2-source-manifest.json records differences: {}. Three source-level S2 blockers remain.

P1-1: Removed listener bindings can still admit post-publication DNS requests on the old port. RuntimeControl::install publishes the new graph at runtime_snapshot.rs:322, while old listener cancellation is only processed asynchronously by the supervisor at assembly.rs:646-653. During that window, RuntimeControl::capture at runtime_snapshot.rs:372-381 identifies a listener only by (tag, kind), not the full binding address. A port change preserves tag/kind, so an old UDP socket or old TCP connection can receive a new datagram/frame after publication and incorrectly capture the new snapshot even though its address no longer exists in that snapshot. Gate admission by the full BindingKey or synchronously mark removed bindings non-admitting at publication; add a race regression proving pre-publication in-flight work finishes but a post-publication datagram/next TCP frame on the removed port cannot execute. [open]

P1-2: Retirement/shutdown cleanup short-circuits before required owner closure when an earlier cleanup operation fails. RuntimeControl::close uses self.retire().await? at runtime_snapshot.rs:424, and later stop_refreshes().await?, so either error skips subsequent persistence cleanup and current.forwards.close_all() at line 437. Likewise, retire() returns immediately on cache retirement failure before old.forwards.close_exclusive_to(...). This violates the S2 contract that retirement closes exclusive owners and supervisor shutdown closes current owners; an observable refresh/persistence failure can therefore leave upstream transports unclosed. Make cleanup best-effort/aggregating so every required close/drain step executes while preserving the first/combined error, and add failure-injection coverage asserting old and current transport owners close even when cache retirement fails. [open]

P1-3: The cache-reuse fingerprint is not actually the documented “full supported policy” fingerprint. config.rs:1046-1048 excludes entire udp_server/tcp_server definitions from policy_sha256, which also excludes their execution-affecting entry selection. Consequently a snapshot can switch an unchanged listener from sequence A to sequence B while keeping the same fingerprint; RuntimeControl::prepare then reuses matching cache owners and can serve responses cached under A while B is now active. Include listener execution semantics such as entry in the fingerprint while still excluding socket/display-only fields needed for port-only warm-cache reuse, and regress with two predeclared sequences sharing a cache where changing the active entry must not hit A’s cached answer. [open]

FINAL: FAIL

## Remediation evidence (re-review pending)

P1-1: full listener address now participates in snapshot admission. A held
supervisor update mailbox deterministically exposes the publication/cancel gap;
removed bindings cannot capture the new graph. Real old-port UDP/TCP messages
do not execute: UDP gets no response, TCP closes, admitted_total stays zero.
Old leases retain generation zero; existing paused old-frame proof still passes.

P1-2: cache retirement and host shutdown continue all required cleanup steps,
retaining the first error. Injected retired refresh-join failure plus current
persistence-rename failure still closes old/current transport lifecycle states.
The isolated RED restores the exact original retire/close functions in a separate
source/target directory and fails with Open versus required Closed. Initial RED
compilation lacked baseline fixtures and is retained; repaired RED is genuine.
A test first expected Closed exchange error, while ForwardAdapter reports Connect
after taking its owner; that invalid expectation was replaced with direct
upstream lifecycle-state assertions. All failed logs are retained.

P1-3: full policy fingerprint includes listener entry semantics while excluding
socket/audit-only fields. Controlled DNS reproduces stale A response after entry B
publication. Repaired test returns B response; port-only warmed-cache proof still
passes. Initial hookless fixture was invalid RED, and local-reject-only fixture
passed without reproducing the bug; only s2-p1-3-wire-red.log is valid RED.

Repaired final validation: 326 native-host checks across29 suites pass; fmt and
strict all-targets Clippy pass;137 Rust source hashes match the tested remote
source with zero differences. See s2-p1-source-manifest.json. Re-review pending.

## Exact-source re-review PASS

Range c6abbe42ba12e5bda86188b81547a8d86edd10c8 →
a9fe9f0eb5ceac81f7b43407acb2f0e81cdfaedf.

Exact committed re-review confirms all three previously reported root causes are remediated. The committed REDs reproduce the original defects; repaired coverage passes, including the old-binding wire race, cleanup-error transport closure, and listener-entry cache invalidation. Final evidence totals 326 passed / 0 failed / 0 ignored across 29 suites, with 137 tested Rust source hashes matching and differences: {}; fmt and strict Clippy evidence are present. S3–S7 remain pending.
FINAL: PASS

Formal controller closed P1-1/P1-2/P1-3 and advanced to Slice 3.
No branch commit/push, deployment or task completion was performed.
