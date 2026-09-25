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

- [ ] Add red tests around a public read-only host snapshot: audit off retains no query/client details; metrics count fixed outcomes; audit on retains terminal entries; test-only small capacity evicts oldest with an exact visible count. Boundary: in-process observer, no HTTP or disk mock.
- [ ] Add red histogram tests that inject explicit elapsed Duration values through the observer's shared aggregation path and inspect metrics_snapshot(): every frozen inclusive cumulative edge, +infinity, nondecreasing counts, histogram count == completed, and admitted == completed + in-flight. Production supplies monotonic elapsed time; bucket tests use no sleeps. Public surfaces: HostAssembly::metrics_snapshot() and audit_snapshot(); mock boundary: deterministic elapsed Duration only, no clock/network mock.
- [ ] Implement the host-owned typed observer and snapshot, fixed metric dimensions/buckets, bounded retention, and reset-on-new-assembly lifetime. Keep synchronization compatible with a later multi-core host; review allocations/locks on the disabled hot path.
- [ ] Make the existing YAML `enable_audit` value select capture while retaining strict rejection of all other unsupported config shapes. Verify before-I/O errors and W1/W2/W3 audit-off regression.

## Slice 2 — execution provenance and listener terminalization

- [ ] Add red tests at the native execution seam for W1 direct forward, W2 cold/warm, W3 A/B→A/B→C, upstream/local SERVFAIL, timeout, and failed leg. Public surface: native `execute_request` result/snapshot via UDP/TCP integration tests; mock only controlled upstream responses and transport send failure where needed.
- [ ] Assert lifecycle outcome independently from response state/source and per-attempt/failure provenance. A local SERVFAIL after timeout has a local response source plus upstream-timeout provenance; a valid upstream SERVFAIL has the upstream source/identity; a failed leg followed by a successful fallback records both ordered attempts but names only the accepted final upstream. Public surface: retained audit snapshot and transport result; mock boundaries: controlled upstream responses/errors, injected send failure, and cancellation token; never infer delivery or response source from nonempty wire bytes.
- [ ] Carry actual cache/leg/final-response facts out of the existing execution driver without changing its sequence or cache semantics. Finalize once at the listener after framing/send or cancellation; count malformed/partial requests outside admitted-query totals.
- [ ] Exercise deterministic UDP and TCP send failure/cancellation and multiple TCP requests on one connection. Check final response code, transport/client/question identity, elapsed time, and that no canceled query is marked sent.

## Slice 3 — concurrency, lifecycle, Linux evidence, review

- [ ] Run mixed requests with distinct IDs/routes and shutdown barriers. Verify exact audit-to-request correlation, counters, no late send or extra upstream leg, in-flight zero after drain, owner close, and rebind. Keep W1/W2/W3 correctness oracles and cache publication tests intact.
- [ ] From `rust/`, run `cargo fmt --all -- --check`, focused native-host tests, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and applicable existing Go/cgo regression checks from the repo root. Record exact commands, commit, failures and fixes.
- [ ] Build the pinned native binary for Linux amd64 and run W1/W2/W3 audit-on/off E2E on `ssh mosdns-rust`; do not use production `mos`. Run only the frozen valid low/moderate Rust-before/Rust-after probe. Report p50/p95/p99, correct-on-time throughput, CPU, RSS, audit-on overhead, raw hashes, and invalid stages. Treat unrepeatable or sender-limited runs as inconclusive.
- [ ] Ask the designated reviewer for a scoped A1–A6 review and fix findings. Update coverage/handover with bounded evidence, perform `trellis-check`/`trellis-update-spec` only where a lasting rule emerged, audit exact changed paths, and commit/push only task-owned changes. After final PASS, follow the normal finish/archive lifecycle; do not deploy or start the next task automatically.

## Review and rollback points

The most sensitive files are `rust/native-host/src/execution.rs`, `udp.rs`, `tcp.rs`, `assembly.rs`, and `config.rs`. Keep the observer isolated enough that an audit change can be reverted without altering DNS response construction or cache/route logic. A regression in response bytes, upstream counts, cancellation, or unaccounted audit loss blocks the slice. A repeatable p95/p99 or correct-on-time regression beyond the predeclared budget blocks final PASS until repaired or explicitly scoped into a separate corrective task.
