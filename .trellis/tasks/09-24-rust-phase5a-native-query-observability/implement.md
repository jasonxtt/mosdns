# Execution plan: Phase 5A native query observability

Approved execution scope: Slices 0–3, authorized on 2026-09-25 after the planning review passed. Read `prd.md`, `design.md`, `research/source-audit.md`, `AGENTS.md`, and the relevant backend specs before editing runtime code. Preserve unrelated dirty paths and keep Trellis auto-commit disabled.

## Slice 0 — freeze contracts and evidence plan

- [x] Record the exact W1/W2/W3 YAML/corpus hashes, native source commit, Go audit field discovery, current metric names, and before-state behavior in task research. Freeze the typed audit/metrics snapshot field contract, lifecycle terminal enum, response source/state, per-attempt and failure provenance, sole-listener audit-flag rule, fixed histogram edges, and event-retention invariant as separate contracts.
- [x] Before implementation, freeze the old Rust source/binary identity, Linux VM topology, runner/fixtures, valid offered-rate stages, repetition/order, resource collection, invalid-run rules, and review budgets. Pin the new source/binary hash after building and before official candidate runs. Reuse the earlier harness only after checking its current hash and limitations.
- [x] Add red focused tests for accepting `enable_audit: true` in existing strict graphs and for unchanged negative-config behavior. Public surface: `compile_yaml`/`HostAssembly`; boundary: no socket on compilation errors.
- [x] Add a red config test proving a second listener plugin still rejects before assembly/I/O, including when the two listeners specify different audit values. The current 5A compiler permits exactly one listener; W1 selects UDP or TCP and W2/W3 use UDP. Public surface: compile_yaml; mock boundary: none, compile only.

Slice 0 evidence (2026-09-25): `performance-manifest.md` is pinned by
`performance-manifest.sha256`; the old Linux binary and helper hashes were
verified on `mosdns-rust`. `cargo fmt --all -- --check`, task-context
validation, and `git diff --check` pass. Focused `slice2_config` tests produce
the intended red result: 9 pass, including the mixed-audit duplicate-listener
rejection; the new audit-enabled acceptance test fails because
`compile_listener` still rejects `true`. That behavior changes in the next
approved slice.

Slice 0 review remediation: C2C finding P1-1 identified incomplete evidence
that the frozen Rust-before binary matches the complete pre-change source/build
graph. The manifest now records the identical full `rust/` tree IDs, matching
`Cargo.lock` content hashes, absence of repository build overrides, and the
archived source archive, build command, compiler, and binary identities. The
bounded C2C re-review is pending; Slice 1 remains gated on its explicit PASS.

## Slice 1 — host-owned observer and bounded snapshot

- [x] Add red tests around a public read-only host snapshot: audit off retains no query/client details; metrics count fixed outcomes; audit on retains terminal entries; test-only small capacity evicts oldest with an exact visible count. Boundary: in-process observer, no HTTP or disk mock.
- [x] Add red histogram tests that inject explicit elapsed Duration values through the observer's shared aggregation path and inspect metrics_snapshot(): every frozen inclusive cumulative edge, +infinity, nondecreasing counts, histogram count == completed, and admitted == completed + in-flight. Production supplies monotonic elapsed time; bucket tests use no sleeps. Public surfaces: HostAssembly::metrics_snapshot() and audit_snapshot(); mock boundary: deterministic elapsed Duration only, no clock/network mock.
- [x] Implement the host-owned typed observer and snapshot, fixed metric dimensions/buckets, bounded retention, and reset-on-new-assembly lifetime. Keep synchronization compatible with a later multi-core host; review allocations/locks on the disabled hot path.
- [x] Make the existing YAML `enable_audit` value select capture while retaining strict rejection of all other unsupported config shapes. Verify before-I/O errors and W1/W2/W3 audit-off regression.

Slice 1 evidence (2026-09-25): `HostAssembly` owns an `Arc`-backed observer;
public metrics/audit snapshots are consistent copies; audit-disabled recording
updates metrics without invoking the sensitive-record builder; enabled capture
retains only the newest configured number of records and increments an exact
eviction count. Typed response/lifecycle/cache/upstream/failure fields and all
15 inclusive histogram bounds plus `+infinity` are covered by deterministic
tests. The sole-listener `enable_audit: true` value now assembles for W1 UDP/TCP,
W2, and W3 while mixed/additional listeners remain rejected. Listener request
admission and terminal event wiring remains in Slice 2.

Validation: 6 observer unit tests, the 100,000-record default-capacity test,
2 public snapshot tests, and 10 focused config tests passed. `cargo fmt --all --
--check` and `cargo clippy -p mosdns-native-host --all-targets --locked -- -D
warnings` passed.

## Slice 2 — execution provenance and listener terminalization

- [x] Add red tests at the native execution seam for W1 direct forward, W2 cold/warm, W3 A/B→A/B→C, upstream/local SERVFAIL, timeout, and failed leg. Public surface: native `execute_request` result/snapshot via UDP/TCP integration tests; mock only controlled upstream responses and transport send failure where needed.
- [x] Assert lifecycle outcome independently from response state/source and per-attempt/failure provenance. A local SERVFAIL after timeout has a local response source plus upstream-timeout provenance; a valid upstream SERVFAIL has the upstream source/identity; a failed leg followed by a successful fallback records both ordered attempts but names only the accepted final upstream. Public surface: retained audit snapshot and transport result; mock boundaries: controlled upstream responses/errors, injected send failure, and cancellation token; never infer delivery or response source from nonempty wire bytes.
- [x] Carry actual cache/leg/final-response facts out of the existing execution driver without changing its sequence or cache semantics. Finalize once at the listener after framing/send or cancellation; count malformed/partial requests outside admitted-query totals.
- [x] Exercise deterministic UDP and TCP send failure/cancellation and multiple TCP requests on one connection. Check final response code, transport/client/question identity, elapsed time, and that no canceled query is marked sent.

Slice 2 evidence (2026-09-25): the first focused execution test compiled red because
the execution-observation result surface did not exist. The native request driver
now returns response source/code, cache disposition, executed sequence, accepted
final upstream, ordered configured-upstream attempts, and typed failure provenance
alongside the unchanged wire response. Tests cover W1 direct forwarding; W2 cold
and warm paths; W3 A, B→A, B→C, and B SERVFAIL→C; local timeout SERVFAIL versus a
valid upstream SERVFAIL; and failed first/second legs without changing the
existing stop-on-transport-error policy.

Both listeners now share the host-owned observer. A request is admitted only after
DNS query parsing; a guard records exactly one terminal result after send, send
failure, cancellation, or an unfinished-task drop. UDP malformed datagrams and
TCP invalid/truncated frames increment `malformed_total` without admission. The
UDP/TCP tests inject send errors and cancellation at their I/O boundary; real
listener tests verify client/question identity, response facts, successful-send
counts, multiple TCP queries on one connection, partial-frame accounting, and
zero in-flight after each completed query. Dropped requests consult their
cancellation scope, so task shutdown is not mislabeled as a send success. The
C2C Slice 2 review found that unfinished-query Drop finalization discarded
partial execution facts. The guard now shares an execution checkpoint with the
request future, preserving established cache/sequence/response/attempt/failure
facts and marking unresolved cache disposition explicitly. An upstream leg
dropped while pending is counted as canceled or interrupted according to the
cancellation scope. A W2 cancellation test aborts the request while its cold
cache miss is awaiting the forward and verifies the retained cache miss,
sequence, and interrupted attempt.

Validation: `cargo fmt --manifest-path rust/Cargo.toml --all -- --check`,
`cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked`
(41 unit tests plus all native-host integration suites),
`cargo clippy --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets
--locked -- -D warnings`, and `git diff --check` pass. W1/W2/W3 response and
cache regression suites remain green. Transport-error execution continues to
terminate as local SERVFAIL under the existing sequence contract; the added
fallback case is an upstream B SERVFAIL response followed by the existing C
route, with only C marked final.

## Slice 3 — concurrency, lifecycle, Linux evidence, review

- [x] Run mixed requests with distinct IDs/routes and shutdown barriers. Verify exact audit-to-request correlation, counters, no late send or extra upstream leg, in-flight zero after drain, owner close, and rebind. Keep W1/W2/W3 correctness oracles and cache publication tests intact.
- [x] From `rust/`, run `cargo fmt --all -- --check`, focused native-host tests, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and applicable existing Go/cgo regression checks from the repo root. Record exact commands, commit, failures and fixes.
- [ ] Build the pinned native binary for Linux amd64 and run W1/W2/W3 audit-on/off E2E on `ssh mosdns-rust`; do not use production `mos`. Run only the frozen valid low/moderate Rust-before/Rust-after probe. Report p50/p95/p99, correct-on-time throughput, CPU, RSS, audit-on overhead, raw hashes, and invalid stages. Treat unrepeatable or sender-limited runs as inconclusive.
- [ ] Ask the designated reviewer for a scoped A1–A6 review and fix findings. Update coverage/handover with bounded evidence, perform `trellis-check`/`trellis-update-spec` only where a lasting rule emerged, audit exact changed paths, and commit/push only task-owned changes. After final PASS, follow the normal finish/archive lifecycle; do not deploy or start the next task automatically.

Slice 3 local evidence (2026-09-25): W3's concurrent frozen-corpus test now
matches each distinct DNS ID and qname to its audit route, final response,
ordered upstream attempts, and metric deltas. The barrier-driven shutdown test
matches both canceled audit records to the B→A and B→C fixture events, confirms
there was no successful send or late upstream leg, and checks zero in-flight,
closed primary owner, and listener rebind. The audit-off shutdown case still
retains no detailed records. An actual aborted B→A execution now proves that B's
intermediate response cannot become the final response or final upstream; a
cache-free interrupted request is classified `NotApplicable` rather than
`Undetermined`.

The first Linux v1 matrix completed 27 attempts but crossed four predeclared
latency regression guards; its assessment and per-repetition data are in
`research/slice3-v1-pilot-assessment.md` and the adjacent TSVs. W3 repetition 3
Rust-before also missed one sender slot at each of 350/400 QPS, so its 400 QPS
pair is inconclusive with no replacement attempt. The repeatable v1 signals
blocked review. The per-query histogram now increments only its one internal
bucket and materializes the same cumulative public histogram on snapshot. A
second full matrix was run for this candidate, but did not clear the guards;
see the V2 assessment below. A further pinned candidate and full matrix are
required before Slice 3 is eligible for review. V1 raw files remain on the test
VM with a checked hash manifest.

The V2 matrix completed all 27 attempts with exit code 0 and 63/63 valid primary
stages. It crossed 13 frozen p95/p99 guards and one CPU guard, so it was not
submitted for review. Its 901-file raw result tree passed remote SHA-256
verification; the manifest digest is recorded in
`research/slice3-v2-pilot-assessment.md`. The first summary invocation rejected
the V2 attempt-order header; the original table and analyzer were retained, and
a header-aware analyzer produced the final TSVs without changing or rerunning
measurements. V2 shows RSS under the frozen budgets and host load remained low.

The next candidate removes normal-path clones of completed execution facts:
the listener transfers those facts into its cancellation checkpoint before
send, then terminalization moves them into metrics and, when enabled, audit
retention. The interrupted-future checkpoint path still preserves partial
facts. The focused native-host suite passed (42 unit tests plus all package
integration suites), strict workspace clippy passed, and the complete Rust
workspace test suite passed, including the 224.77-second QUIC case. A new
Linux release identity was pinned before its matrix as V3. The V3 matrix
completed all 27 scheduled attempts with no replacements; 25 runner exits were
zero, 62/63 derived primary rows were valid, and the 901-file raw tree passed
remote SHA-256 verification (manifest digest
`b2236d6cf3a16c5abf27b84ab23d57b3b2a57e8a9e80aa3248a588bdcdb8cead`). Two W1
attempts had invalid overload/recovery stages, including one sender shortfall;
wrong responses, protocol errors, transport errors, and timeouts were zero.
V3 remained over the latency budget in six paired assessments: W1 TCP 400 QPS
after-off p99, W2 warm 200 QPS after-off p99, W2 warm 400 QPS audit-on p95 and
p99, and W3 400 QPS audit-on p95 and p99. RSS deltas stayed below the frozen
budgets (maximum +188 KiB after-off versus before-off and +496 KiB audit-on
versus off); CPU comparisons were inconclusive at the 100-Hz sampling
resolution. See `research/slice3-v3-pilot-assessment.md` and its adjacent
identity, TSV, run-audit, and hash-manifest evidence.

Source inspection localized a likely audit-on tail-latency cost in V3:
`AuditRecord` construction happened while holding the shared observer mutex.
V4 constructs the record before acquiring that mutex; metrics and bounded-ring
insertion remain atomic inside one critical section. Its full matrix completed
27 attempts with no replacements, 26 runner exits of zero, and 62/63 valid
primary rows. V4 eliminated V3's repeated W3 audit-on 400 QPS latency
crossings, but retained two repeated guards: W1 TCP 200 QPS after-off p95 and
W2 cold 200 QPS audit-on p99. One W1 after-off overload row was invalid because
the sender missed a slot. The 901-file raw tree passed remote SHA-256
verification; the manifest digest is recorded in
`research/slice3-v4-pilot-assessment.md`.

V5 removes the successful-request checkpoint write/read pair. The listener
guard owns completed execution facts through the send await, and its Drop path
uses those facts if send is interrupted; the shared execution checkpoint stays
for cancellations during execution. The audit ring reserves at most 1,024
entries at assembly when capture is enabled and allocates no ring storage when
audit is disabled. The focused native-host suite passed (44 unit tests plus
package integration suites), strict workspace clippy and rustfmt passed, and
the full Rust workspace test suite passed, including the 224.79-second QUIC
case. The V5 matrix completed 27 attempts with no replacements; 26 runner exits
were zero and 62/63 primary rows were valid. Its 901-file raw tree passed
remote SHA-256 verification (manifest digest
`3ae43b69531bfdf01dd8280e6c0dc50ae9beb5bc016bc9a0aa7cdcd08d28e51d`). One W2
cold after-off overload row was invalid because the sender missed a slot. The
matrix crossed three latency guards repeatedly: W2 cold 200 QPS after-off
p95/p99 versus Rust-before and W3 200 QPS audit-on p99 versus after-off. RSS
stayed under budget; CPU comparisons remained inconclusive. The W2 cold
audit-on versus off p99 crossing from V4 did not repeat, but the after-off
baseline comparison regressed. Full results are in
`research/slice3-v5-pilot-assessment.md`.

Source inspection suggests that storing the completed event inline in
`AdmittedQueryGuard`, which lives across the send await, may enlarge the
listener future and contribute to the W2 cold audit-off regression. This is a
hypothesis rather than a measured cause. V6 restores completed facts to the
heap-backed execution checkpoint while retaining V4's lock-outside audit
record construction and V5's bounded initial ring reservation. A new pinned
Linux identity and full matrix are required before review.

V6 passed the focused native-host package suite (44 unit tests plus all package
integration suites), strict workspace clippy, rustfmt check, and the full Rust
workspace test suite including all integration tests and doctests. The full
workspace run completed its QUIC suite in 224.79 seconds. The V5 assessment
and its verified raw manifest are preserved before starting a fresh V6 matrix;
V6 remains ineligible for review until that matrix clears every frozen guard.

Validation commands and outcomes:

- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed (42 unit tests and all package integration suites before assertion extraction).
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked --test w3_routing` — passed (6 W3 integration tests after extraction).
- `cargo fmt --manifest-path rust/Cargo.toml --all` — passed; final `-- --check` is repeated after the remaining evidence edits.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed, including all workspace integration tests and doctests.
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` — passed after extracting W3 assertion helpers. The first run failed only on `clippy::similar_names` and `clippy::too_many_lines` in the new integration assertions; those were resolved by indexing the bounded upstream map directly and extracting the assertions.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked duration_histogram` — passed after the histogram hot-path change (1 matching test); the first compile caught an ambiguous integer type in snapshot accumulation, fixed with an explicit `u64` accumulator.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` and `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` — passed again after the v1 performance correction.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed after V5 listener-owned finalization and bounded audit-ring reservation (44 unit tests and all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed after V5 changes.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed after V5 changes; the QUIC suite completed in 224.79 seconds.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed after V6 restored heap-backed terminal checkpoints (44 unit tests and all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed after V6 changes.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed after V6 changes, including all integration tests/doctests and the 224.79-second QUIC suite.
- `go test ./...`, `go build ./...`, and `go vet ./...` — passed from the repository root on macOS. No Go/cgo source is changed in this native-host task; Linux-tagged hybrid bridge suites remain outside this task's affected surface.
- `python3 .trellis/scripts/task.py validate .trellis/tasks/09-24-rust-phase5a-native-query-observability` and `git diff --check` — passed.

## Review and rollback points

The most sensitive files are `rust/native-host/src/execution.rs`, `udp.rs`, `tcp.rs`, `assembly.rs`, and `config.rs`. Keep the observer isolated enough that an audit change can be reverted without altering DNS response construction or cache/route logic. A regression in response bytes, upstream counts, cancellation, or unaccounted audit loss blocks the slice. A repeatable p95/p99 or correct-on-time regression beyond the predeclared budget blocks final PASS until repaired or explicitly scoped into a separate corrective task.
