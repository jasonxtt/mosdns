# Design — Rust Phase 5A native cache

## Evidence and boundaries

Read `research/source-audit.md` with the PRD. W1 already supplies a canonical
resumable `ExecutionMachine`, a shared UDP/TCP `execute_request`, immutable
compiled configuration and a current-thread Tokio runtime. Reuse these.
`cache-core` owns useful cache/TTL logic behind a global handle surface; expose
that logic as an owned Rust object rather than duplicating its storage engine.
No new third-party dependency or broad trait/Send/Sync refactor is needed.

## Safe cache-core surface

Introduce an owned cache object with safe Rust methods accepting borrowed byte
slices and returning owned lookup data with ordinary Rust errors/results.
Names are implementation choices. Native calls must not construct ABI pointer
containers or consult `HANDLES`. Registry entries may wrap the same owned
object, and existing ABI functions adapt to it. Preserve symbol signatures,
status mapping, pointer validation, panic containment, buffer ownership,
flush/close behavior, timestamps and domain-set payload handling.

Keep immutable stored wire bytes and copy-before-TTL-patch semantics. Preserve
Moka's existing eviction/admission model; do not require a new cache algorithm
or impose exact synchronous eviction ordering in tests. Capacity tests should
settle pending maintenance and assert bounded storage, without a hot-path full
scan. Test two isolated objects, overwrite/expiry, malformed wire, buffer
independence, and parity of native/bridge results at identical explicit times.

The core continues accepting caller-supplied integer times, so existing bridge
Unix-time semantics do not change. The native adapter uses elapsed monotonic
seconds from a host-owned epoch, with a deterministic test clock and checked
expiry arithmetic. Within that cache instance, stored/lookup/expiry times must
all use the same epoch. Use `Instant` only for production elapsed time; do not
introduce sleeps to prove TTL boundaries.

## Host adapter and canonical execution

One cache instance belongs to the assembled plugin/host, shared among requests
on the current runtime. Per-request state owns a pending-store token and all
response bytes. The adapter handles lookup/key/retention policy; sequence-core
must not acquire cache or Tokio dependencies.

The compiled W2 sequence is exactly one flat root `$cache -> $forward`. On
cache dispatch: hit sets `ResponseState::Raw` from an owned aged copy patched
for this query, then resumes with the existing `Accept` outcome; miss retains
one request-local key/token and resumes with `Continue`. The same machine then
dispatches the existing forward adapter. On its terminal completion, the shared
host execution driver consumes the token exactly once and stores only if the
sequence completed successfully with a validated, cacheable upstream response
and its cancellation/deadline scope remains valid. Error, terminal execution
failure or cancellation drops the token. No detached task performs stores.

This deliberately uses existing `Dispatch`/`resume`/`Complete` semantics; it
needs no new general continuation ABI or second interpreter. The token models
the post-`next.ExecNext` cache action for the accepted flat root only. Nested
cache/try/jump/return middleware configurations remain rejected; do not claim
general cache-plugin continuation compatibility. Add tests proving cache hit
skips the exact forward dispatch, miss resumes it, and completion/error paths
consume/drop the token correctly. If inspection proves this bounded approach
cannot express that contract, obtain revised planning review before changing
sequence-core; no speculative general engine extension is authorized.

Keep this logic in a shared host execution component callable from existing
listeners (moving the current driver out of `udp.rs` is a narrow allowed
extraction). Listeners only admit, frame, send and supervise requests. Neither
listener may perform lookup or store. Preserve W1 response mapping and shutdown.
Track successful upstream provenance separately from merely having response
bytes: W1 can synthesize SERVFAIL while resuming with `Continue`, so a completed
machine alone is insufficient evidence to cache. Check cancellation/deadline
again at the publication boundary; no await between final check and insert.

## Query key and response eligibility

Use the existing parsed, self-contained qname wire labels with case preserved,
qtype and AD/CD from the validated query flags. Only IN is cache eligible, so
non-IN queries bypass without sharing entries. ID is excluded; compression
layout must not create separate keys for the same decoded name. Native in-memory
key bytes need not reproduce Go's textual packing: they are not persisted or
exchanged with the hybrid cache. No global domain-name normalization is added.
DO/ECS are deferred because the accepted query parser rejects additional records.

Use `dns-core` validation/TTL observation for the response. Non-OPT additional
and authority RRs contribute to minimum TTL; OPT TTL bits never do. An OPT in
any response makes it bypass this milestone's cache (forwarded unchanged through
W1's response path), avoiding unsafe removal of compressed wire sections. If
existing helpers cannot expose OPT presence/RCODE, add only a narrow shared
metadata observer with tests; do not add a second DNS parser or blindly splice
compressed bytes. Keep the existing response validation limits explicit.

Retention follows PRD requirement 6; on-wire RR TTL aging follows cache-core.
At exact expiry a lookup misses. Zero/minimum-retention policy must not inflate
the TTLs carried in the stored answer. TC or parsing failure bypasses storage.
Locally generated SERVFAIL/REFUSED never gets a token committed; upstream
SERVFAIL can be cached if the exchange and response validation succeeded.

## Strict configuration and dependency boundary

The compiler accepts its existing W1 graph or the frozen four-plugin W2 graph.
W2 requires explicit integer `size: 64` and `lazy_cache_ttl: 0`, only these cache
args, one reference to that cache followed by one forward reference, a UDP
listener/upstream, audit false and existing strict top-level settings. Tags are
not hardcoded and declaration order remains irrelevant. Reject omission,
wrong types, other values, duplicate args, extra cache options, reversed or
repeated steps, missing/wrong references and cache-enabled TCP before assembly.
Update plugin-count diagnostics so existing three-plugin W1 remains supported.

Add only a native-host path dependency on cache-core and its resulting lockfile
package-edge update. Existing pinned external dependency versions/features stay
unchanged. Native-host may pull cache-core's existing dependency closure, but
must never link runtime/cgo. No transport or workspace-membership change.

## Correctness evidence and operational limits

Tests use independent controlled loopback upstreams with counters and barriers.
W2 cold and warm use separate cache lifecycles. Exact two-case input is read from
or explicitly checked against frozen `workloads/cache.jsonl`; frozen YAML must
be parsed as-is in config tests. Network tests may substitute only ephemeral
loopback ports and short test deadlines, recording that distinction. Do not
silently regenerate/change the baseline corpus. Add protocol variants in new
Rust/task-local fixtures, not in historical baseline directories.

Barrier-forced simultaneous cold misses may each reach upstream. Assert correct
responses and subsequent hits, not a universal cold count of one per key.
Warm counter assertions occur after verified prefill and use safe TTL/controlled
clock conditions. Test response buffers by mutation of one returned copy.

Linux correctness uses a fresh temporary directory on `ssh mosdns-rust`, no
system installation/service changes, and cleanup only of owned test artifacts.
Record uname/architecture/toolchains/exact commit and local/remote check output.
Go bridge tests require the existing staticlib/build tags; changes to FFI require
existing Linux+cgo integration and an applicable focused memory-safety check.
No performance verdict may be drawn from this correctness run or historical
QEMU baseline. Rollback is reverting this task's commits while keeping W1 and
existing Go defaults; there is no production cutover in this task.
