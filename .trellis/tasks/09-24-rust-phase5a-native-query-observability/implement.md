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
and raw manifest are preserved. The V6 matrix completed all 27 attempts with
zero runner exits, 63/63 valid primary rows, and 56 paired assessments; its
901-file raw tree passed remote SHA-256 verification (manifest digest
`1ab82e212c6e2651993a0c2219abf29ddc58b95581e8119acfbfb7f046b2844f`). There
were no wrong responses, protocol/transport errors, timeouts, or sender
shortfalls. Peak paired RSS increases were +168 KiB audit-off versus
Rust-before and +2,228 KiB audit-on versus off; CPU comparisons remained
inconclusive at 100-Hz sampling resolution. V6 crossed four latency guards
repeatably: W1 TCP 400 QPS audit-off versus Rust-before p95/p99 and W2 warm
400 QPS audit-off versus Rust-before p95/p99. All other paired latency guards,
including the V5 W2 cold 200 QPS crossings, cleared. See
`research/slice3-v6-pilot-assessment.md` and its adjacent identity, source
manifest, run audit, attempt-order, derived TSV, and raw-hash evidence.

Source inspection makes the restored completed-event checkpoint lock pair a
plausible contributor to V6's high-rate audit-off crossings, but the matrix
does not prove causality. V7 will move the completed `TerminalObservation`
into a boxed listener-owned slot. This preserves lock-free successful
finalization while reducing the listener guard's inline state that was a
possible W2 cold 200 QPS factor in V5. V7 implements that boxed slot. The
normal finish path moves boxed facts directly into terminal accounting and
avoids both completed-event checkpoint locks; interrupted execution still
reads partial facts from the shared checkpoint. The native-host package suite,
strict workspace clippy, rustfmt, and complete workspace test suite all
passed, including the 224.78-second QUIC case. Its 27-attempt matrix completed
with all runner exits zero, 63/63 valid primary rows, and 56 paired
assessments; the 901-file raw tree passed remote SHA-256 verification
(manifest digest
`30d461d2c102075fbe0c18e9f64c6a0a2d45412c15b0ef0f48806ef30b6c0a43`). Wrong
responses, protocol/transport errors, timeouts, and sender shortfalls were
zero. Paired RSS stayed under budget (+196 KiB audit-off versus Rust-before;
+2,276 KiB audit-on versus off), while CPU comparisons remained inconclusive
at 100-Hz sampling resolution. Ten latency guards crossed repeatably: W1 TCP
200 QPS audit-off versus Rust-before p95/p99; W2 cold 200 QPS audit-on versus
audit-off p95/p99; W2 warm 200 QPS audit-on versus audit-off p95/p99; and W2
warm 400 QPS for both audit-off versus Rust-before and audit-on versus
audit-off p95/p99. W1 TCP 400 QPS crossings from V6 did not repeat. See
`research/slice3-v7-pilot-assessment.md` and its adjacent identity, source
manifest, run audit, attempt-order, derived TSV, and raw-hash evidence.

Source review found two observer mutex acquisitions on the audit-off request
path: one for admission counters and one for terminal metrics. V8 replaces the
admission lock with a sequentially consistent atomic in-flight counter and
derives admitted totals as completed plus in-flight when taking a metrics
snapshot. The terminal metrics transition remains under the existing mutex,
preserving a consistent completion/in-flight snapshot boundary. A focused
assertion verifies the admitted/completed/in-flight values while a request is
active. The native-host package suite, strict workspace clippy, rustfmt, and
full Rust workspace suite passed after this change, including the 224.78-second
QUIC test and doctests. One earlier full-suite attempt had an unrelated
`reuse_doh` idle-half-close H2 test fail with `MaybeSent`; the exact test passed
alone and the complete rerun passed. V8's matrix completed all 27 attempts with
25 zero runner exits, 62/63 valid primary rows, and 56 paired assessments; its
901-file raw tree passed remote SHA-256 verification (manifest digest
`fa14246231c100634173dbd94be208d7b4960b45087322e97b4bc188e56f3821`). The two
exit-1 runs were retained without replacements: W2 warm r3 Rust-before missed
one 400 QPS overload slot, and W3 audit-on r1 missed one 350 QPS supporting
slot while both primary points remained valid. Wrong responses,
protocol/transport errors, and timeouts were zero. Peak paired RSS deltas were
+172 KiB audit-off versus Rust-before and +2,324 KiB audit-on versus off; CPU
comparisons remained inconclusive. Eight p95/p99 comparisons crossed frozen
guards repeatedly, so V8 is not reviewable. See
`research/slice3-v8-pilot-assessment.md` and its adjacent identity, source
manifest, audit, attempt-order, derived TSV, and raw-hash evidence.

Source inspection found that each query still allocates and clones an
`Arc<Mutex<ExecutionCheckpoint>>` used only when execution is interrupted.
V9 will store the checkpoint in a box owned by the listener guard and lend a
mutable reference to execution. ExecutionFacts Drop can then publish partial
facts directly without the shared Arc, mutex, or clone; normal completion
stores its terminal observation in the same box. This preserves the
interrupted-query fallback and keeps large state off the listener future's
inline frame. V9 needs a new pinned identity and full matrix before review.

V9 implementation is now in the worktree. `AdmittedQueryGuard` owns the boxed
checkpoint from admission through terminalization; execution borrows it
mutably, and its cancellation Drop hook writes partial facts directly. A
successful terminal observation occupies the checkpoint's existing allocation,
so the completed path no longer allocates a separate boxed observation. The
guard retains the completed facts through send finalization, while an
interrupted execution still records the known response/cache/route/attempt
facts and the in-flight attempt outcome. TCP, UDP, cancellation, and dropped
W2/W3 execution tests cover both paths. The frozen code change is limited to
the native observer, request execution seam, and its two listeners; response,
cache, route, or configuration behavior was not changed.

V9 local validation on the current source passed:

- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed (44 unit tests and all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` — passed.
- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed.
- `cargo test --workspace` from `rust/` — passed across all workspace tests and doctests; the final 23-case QUIC test group took 224.79 seconds.

The pinned Linux build passed for source commit `f22c558365f1fc929752ecf214b29d510118d619`; its 2,303,968-byte release binary passed helper v8 validation. The first generated matrix driver expected helper v9 and exited at preflight before any benchmark attempt. The corrected pinned driver checks the frozen helper v8 and uses the fresh `results-v9-run` directory. Its helper/binary, CPU-set, and audit-overlay preflight checks passed before the matrix began.

V9 then completed its 27-attempt matrix: all runner exits were zero, all 63/63 primary rows were valid, all 54,000 requests were correct on time, and the 901-file raw tree passed remote SHA-256 verification (manifest digest `60b7058a9a51d8da38f1cd13720fdae8ac5619fe136c17fb085adced6ea0fa71`). There were no wrong responses, late responses, protocol/transport errors, timeouts, or sender shortfalls. Peak paired RSS deltas were +208 KiB audit-off versus Rust-before and +2,324 KiB audit-on versus audit-off. Thirteen of 14 CPU comparisons remained inconclusive at 100-Hz sampling resolution; W3 400 QPS audit-off versus Rust-before crossed its CPU guard in 2/3 pairs. Twelve frozen guard comparisons repeated: 11 p95/p99 and one CPU comparison. V9 is not reviewable; see `research/slice3-v9-pilot-assessment.md` and its adjacent identity, source manifest, audit, attempt order, derived TSV, and raw hash evidence.

V9 source inspection found avoidable audit-only data work in the audit-disabled path: it still creates sequence/failure provenance and clones the upstream identity into a response-source record even though basic metrics consume response code, cache disposition, lifecycle, and upstream-attempt outcomes. V10 now gates these audit details on the listener's existing audit flag, while retaining the configured upstream identity/outcome required for per-upstream metrics. W3 reserves its bounded attempt vector from the configured forward count before dispatch instead of growing during a multi-leg request. This is a new performance hypothesis, not a changed threshold; enabled-audit field semantics and cancellation checkpoints remain covered by existing tests. V10 requires its own pinned identity and complete frozen matrix before review.

V10 implementation is complete in `rust/native-host/src/observer.rs` and `rust/native-host/src/execution.rs`. The execution checkpoint carries the admitted host's audit flag; execution omits audit-only final sequence, response-source identity, and failure-provenance allocations when disabled while continuing to emit attempt identities/outcomes for metrics. A new regression test executes an audit-disabled W1 request through the real observer guard, checks terminal/response/cache/forward metrics, and confirms that no audit record is retained. Existing enabled-audit W1/W2/W3 and cancellation tests continue to cover detailed fields.

The backend Rust migration spec now records the Phase 5A observer hot-path contract: gate audit-only materialization at execution, preserve the per-upstream attempt facts needed by metrics, reserve bounded W3 attempts, and keep enabled-audit provenance covered.

V10 Linux amd64 release build passed on Rust 1.95.0 from source commit `65a31ff57046e0047006ea401aa023519c658c16`; the 2,304,904-byte executable passed helper v8 validation with SHA-256 `bb13371306d6a26228bcca7e496e8f1354e1d9ac0ecfa2ef36bb5c7b01dba2bd`. The 107-file Rust source manifest, 27-attempt order, driver, frozen-input hashes, and candidate identity are recorded in `research/slice3-candidate-v10-identity.md` and its adjacent files. No V10 benchmark attempt had started when this identity was captured.

V10 preflight passed before any attempt. The official matrix then completed all 27 attempts without replacement; 26 runner exits were zero and the only nonzero exit came from a dropped slot at the 350 QPS supporting stage plus its recovery check. All 63 primary rows were valid, and all 54,000 scheduled primary requests were sent, received, and correct on time. The 902-file raw result manifest verified remotely (digest `c862ae80466ee8255166a325486e20dec5c592614f6308abe1486894cd15b309`). Nine paired p95/p99 comparisons crossed frozen guards; no CPU comparison crossed, 13/14 CPU comparisons were inconclusive at 100-Hz resolution, and paired RSS increases stayed within budget. V10 is not reviewable; see `research/slice3-v10-pilot-assessment.md` and the adjacent raw-hash, audit, attempt-order, derived measurement, binary, and preflight evidence.

V11 implements the source-backed hot-path hypothesis in `observer.rs` and `execution.rs`: the zero-or-one upstream attempt remains inline and promotes to a vector on a second W3 leg, reserving from the configured forward count. The exchange reuses the already materialized in-flight upstream identity for its terminal attempt record instead of resolving and allocating the same name twice; cancellation keeps that identity in its checkpoint until the exchange completes. The enabled-audit snapshot still exposes an ordered `Vec`, and the disabled path feeds the same attempt facts to metrics without allocating a vector. A regression test proves inline storage and ordered promotion; W1/W2/W3 and cancellation suites pass. Workspace tests, strict Clippy, rustfmt, task-context validation, and `git diff --check` all pass; the final QUIC group took 224.77 seconds. Thresholds are unchanged, and V11 still needs its separately pinned Linux matrix.

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
- `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` — passed on the Linux benchmark host with Rust 1.95.0 for source commit `cd96b0adf76767cfede5f20a929ad7ad36e0ce44`; the resulting candidate binary passed helper v8 validation.
- `sha256sum --quiet -c slice3-v6-rust-source-files.sha256` — passed on the Linux candidate source before the V6 release build (107 tracked Rust files).
- V6 frozen 27-attempt matrix — completed without replacements; all 27 runner exits were zero, 63/63 primary rows were valid, and the 901-file raw tree hash manifest verified remotely. The frozen analyzer reported four repeatable p95/p99 guard crossings; Slice 3 remains blocked from review pending a new candidate.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed after the V7 boxed terminal observation change (44 unit tests plus all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings`, `cargo fmt --manifest-path rust/Cargo.toml --all -- --check`, and `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — all passed after V7; the QUIC suite completed in 224.78 seconds.
- `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` — passed on the Linux benchmark host with Rust 1.95.0 for source commit `ad0097ea73374425652586b6fe6d06748309c362`; the resulting candidate binary passed helper v8 validation.
- `sha256sum --quiet -c slice3-v7-rust-source-files.sha256` — passed on the Linux candidate source before the V7 release build (107 tracked Rust files).
- V7 frozen 27-attempt matrix — completed without replacements; all 27 runner exits were zero, 63/63 primary rows were valid, and the 901-file raw tree hash manifest verified remotely. The frozen analyzer reported ten repeatable p95/p99 guard crossings; Slice 3 remains blocked from review pending another candidate.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed after the V8 atomic admission counter (44 unit tests plus all native-host integration suites).
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed on the final V8 source, including all integration tests and doctests; QUIC completed in 224.78 seconds. The preceding attempt's isolated `reuse_doh` failure passed when run alone and did not recur in the full rerun.
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed on the final V8 source.
- `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host --bin mosdns --release --locked` — passed on the Linux benchmark host with Rust 1.95.0 for source commit `b7c4935e176809bdd3fc46fbea0cca13887a89f4`; the resulting candidate binary passed helper v8 validation.
- `sha256sum --quiet -c slice3-v8-rust-source-files.sha256` — passed on the Linux candidate source before the V8 release build (107 tracked Rust files).
- V8 frozen 27-attempt matrix — completed without replacements; all 27 attempts ran, 25 runner exits were zero, 62/63 primary rows were valid, and the 901-file raw tree hash manifest verified remotely. The frozen analyzer reported eight repeatable p95/p99 guard crossings; Slice 3 remains blocked from review pending another candidate.
- `go test ./...`, `go build ./...`, and `go vet ./...` — passed from the repository root on macOS. No Go/cgo source is changed in this native-host task; Linux-tagged hybrid bridge suites remain outside this task's affected surface.
- `python3 .trellis/scripts/task.py validate .trellis/tasks/09-24-rust-phase5a-native-query-observability` and `git diff --check` — passed.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked audit_disabled_execution_keeps_metric_facts_without_audit_details` — passed on V10.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed on V10 (45 unit tests and all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed on V10.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed on V10 across workspace tests and doctests; the final 23-case QUIC group completed in 224.78 seconds.
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` — passed on V11 (46 unit tests and all native-host integration suites).
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` and `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed on V11.
- `cargo test --manifest-path rust/Cargo.toml --workspace --locked` — passed on V11 across workspace tests and doctests; the final 23-case QUIC group completed in 224.77 seconds.
- `python3 .trellis/scripts/task.py validate .trellis/tasks/09-24-rust-phase5a-native-query-observability` and `git diff --check` — passed on V11.



## V12 source correction and local checks

V12 responds to V11's repeated guards without changing frozen inputs or budgets. While an upstream exchange is pending, execution now checkpoints the copyable `ExecutableId` rather than resolving and allocating its identity before the network await. A normal completion resolves the identity once after the exchange; the unfinished-execution drop hook resolves it only when it must publish a canceled/interrupted attempt. The duplicate execution-level `final_upstream` string was removed; enabled audit materialization derives the public field from the existing upstream response source, preserving the `AuditRecord` value while avoiding a second owned copy in request facts.

On the V12 source, `cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --locked` passed (46 unit tests and all native-host integration suites), `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` passed, `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` passed, and `cargo test --manifest-path rust/Cargo.toml --workspace --locked` passed across workspace integration tests and doctests; the 23-case QUIC group completed in 224.78 seconds. The pinned Linux build passed on Rust 1.95.0 from source commit `eddcb48057096f1f8562d55b0bbc6290bff35756`; its 2,306,464-byte executable passed helper v8 validation with SHA-256 `8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d`. The 107-file source manifest, frozen 27-attempt plan, driver, and candidate identity are recorded in `research/slice3-candidate-v12-identity.md` and adjacent V12 files. The V12 preflight passed on 2026-09-25 22:28:34 UTC using the frozen runner,
helper v8, source and binary identities, fixtures, overlays, CPU sets, and ext4
result storage. Its attempt-order output contains only the header, so no
benchmark attempt ran during preflight; captured evidence is under
`research/slice3-v12-preflight/`. The one official 27-attempt matrix subsequently
ran in the fresh `results-v12-run` directory with no replacements.

## V11 pinned benchmark preflight

The pinned Linux amd64 release candidate for source commit
`7b7a0822b3820b4b7d56a5270893ed3eb6f13c69` built successfully with Rust
1.95.0. Its 2,307,104-byte executable passed helper v8 validation with SHA-256
`a50ac020785f9e55f42c2455690d5ceaedf0edd37e5d33e39fde649352e3f75c`. The
107-file source manifest matched before build. Candidate identity, frozen
27-attempt order, and driver are pinned in `research/slice3-candidate-v11-identity.md`
and adjacent V11 files.

The V11 preflight passed on 2026-09-25 21:45:53 UTC using the frozen runner,
helper, fixtures, overlays, CPU sets, and ext4 result storage. Both binary
identities and the source manifest validated; the attempt-order output contains
only its header, so no benchmark attempt ran during preflight. Captured evidence
is in `research/slice3-v11-preflight/`. The single official 27-attempt matrix
is pending and will use a fresh `results-v11-run` directory with no replacement
attempts.


## V11 official matrix result

The single frozen V11 matrix completed with all 27 runner exits at zero and no replacement attempts or invalid stages. All 63/63 primary rows were valid; all 54,000 scheduled primary requests were sent, received, and correct on time, with no late/wrong responses, protocol or transport errors, timeouts, or sender shortfalls. The 901-entry raw hash manifest (SHA-256 `533356cafd58da4684f428fb3812206beb8105793313d5d6cfd895b03c1e1372`) and its sidecar verified remotely; the full ~61 MiB raw run remains on the Linux host, and the derived evidence is captured under `research/slice3-v11-run/`.

Seven p95/p99 paired guards repeated: W1 TCP 200 audit-off versus Rust-before (p95 and p99), W1 TCP 400 audit-off versus Rust-before (p99), W2 cold 200 audit-off versus Rust-before (p99), W3 200 audit-on versus audit-off (p95), and W3 400 audit-off versus Rust-before (p99) plus audit-on versus audit-off (p95). No CPU guard repeated, 13/14 CPU comparisons were inconclusive at 100-Hz resolution, and paired sampled RSS remained within budget (+132 KiB maximum audit-off versus Rust-before; +1,964 KiB maximum audit-on versus audit-off). V11 is not reviewable; see `research/slice3-v11-pilot-assessment.md`. Source inspection suggests testing the upstream identity allocation's move onto the pre-exchange critical path and duplicate audit-on identity storage as bounded follow-up hypotheses; neither is yet confirmed as causal.

## V12 official matrix result

The single frozen V12 matrix completed without replacement attempts: all 27
runner exits were zero, all 63/63 primary rows were valid, and all 54,000
scheduled requests were sent, received, and correct on time. No late or wrong
responses, protocol or transport errors, timeouts, or sender shortfalls were
recorded. The 901-file raw hash manifest verified remotely (SHA-256
`52b9b1be2dc59ada84beb304b317f00e58b1b0ecbade1cd544d3c4cb51e26c34`);
derived evidence is under `research/slice3-v12-run/` and the full ~61 MiB raw
tree remains on the Linux benchmark host.

Five frozen p95/p99 paired guards repeated: W1 TCP 200 audit-off versus
Rust-before p95/p99, W1 TCP 400 audit-on versus audit-off p95/p99, and W2 warm
200 audit-off versus Rust-before p95. No CPU guard repeated, though all 14 CPU
comparisons were inconclusive at 100-Hz resolution. Sampled RSS remained within
budget (+192 KiB maximum audit-off versus Rust-before; +1,884 KiB maximum
audit-on versus audit-off). V12 remains blocked from review; see
`research/slice3-v12-pilot-assessment.md`.

## Measurement diagnostic and major evidence stop (2026-09-26)

The diagnostic protocol pinned in `dec901e` ran nine W1 TCP slots using only
the exact Rust-before binary and audit-disabled configuration. All binary
identities and complete input-hash manifests were identical, all nine runner
exits were zero, and all 18/18 primary rows and 16,200 requests were valid and
correct on time. Nevertheless, the frozen analyzer reported repeated p95 and
p99 regressions at 400 QPS (two of three pairs exceeded the guards).

See `research/slice3-self-control-assessment.md` and adjacent evidence. This
confirms a measurement limitation without proving V12 free of real overhead.
A5 remains unmet and the V12 verdict is unchanged. The workflow's major-issue
stop applies: further speculative implementation or acceptance retries stop
pending an explicitly authorized measurement correction and reviewed frozen
protocol. No acceptance review, finish/archive, new task, or deployment is
authorized by this diagnostic.

## Review and rollback points

The most sensitive files are `rust/native-host/src/execution.rs`, `udp.rs`, `tcp.rs`, `assembly.rs`, and `config.rs`. Keep the observer isolated enough that an audit change can be reverted without altering DNS response construction or cache/route logic. A regression in response bytes, upstream counts, cancellation, or unaccounted audit loss blocks the slice. A repeatable p95/p99 or correct-on-time regression beyond the predeclared budget blocks final PASS until repaired or explicitly scoped into a separate corrective task.
