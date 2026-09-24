# Execution plan: Phase 5A native query observability

Planning only. Do not call `task.py start` until the user reviews the final planning summary in a later message. Read `prd.md`, `design.md`, `research/source-audit.md`, `AGENTS.md`, and the relevant backend specs before editing runtime code. Preserve unrelated dirty paths and keep Trellis auto-commit disabled.

## Slice 0 — freeze contracts and evidence plan

- [ ] Record the exact W1/W2/W3 YAML/corpus hashes, native source commit, Go audit field discovery, current metric names, and before-state behavior in task research. Freeze the typed audit/metrics snapshot field contract, terminal outcome state machine, and event-retention invariant.
- [ ] Before implementation, freeze the old Rust source/binary identity, Linux VM topology, runner/fixtures, valid offered-rate stages, repetition/order, resource collection, invalid-run rules, and review budgets. Pin the new source/binary hash after building and before official candidate runs. Reuse the earlier harness only after checking its current hash and limitations.
- [ ] Add red focused tests for accepting `enable_audit: true` in existing strict graphs and for unchanged negative-config behavior. Public surface: `compile_yaml`/`HostAssembly`; boundary: no socket on compilation errors.

## Slice 1 — host-owned observer and bounded snapshot

- [ ] Add red tests around a public read-only host snapshot: audit off retains no query/client details; metrics count fixed outcomes; audit on retains terminal entries; test-only small capacity evicts oldest with an exact visible count. Boundary: in-process observer, no HTTP or disk mock.
- [ ] Implement the host-owned typed observer and snapshot, fixed metric dimensions/buckets, bounded retention, and reset-on-new-assembly lifetime. Keep synchronization compatible with a later multi-core host; review allocations/locks on the disabled hot path.
- [ ] Make the existing YAML `enable_audit` value select capture while retaining strict rejection of all other unsupported config shapes. Verify before-I/O errors and W1/W2/W3 audit-off regression.

## Slice 2 — execution provenance and listener terminalization

- [ ] Add red tests at the native execution seam for W1 direct forward, W2 cold/warm, W3 A/B→A/B→C, upstream/local SERVFAIL, timeout, and failed leg. Public surface: native `execute_request` result/snapshot via UDP/TCP integration tests; mock only controlled upstream responses and transport send failure where needed.
- [ ] Carry actual cache/leg/final-response facts out of the existing execution driver without changing its sequence or cache semantics. Finalize once at the listener after framing/send or cancellation; count malformed/partial requests outside admitted-query totals.
- [ ] Exercise deterministic UDP and TCP send failure/cancellation and multiple TCP requests on one connection. Check final response code, transport/client/question identity, elapsed time, and that no canceled query is marked sent.

## Slice 3 — concurrency, lifecycle, Linux evidence, review

- [ ] Run mixed requests with distinct IDs/routes and shutdown barriers. Verify exact audit-to-request correlation, counters, no late send or extra upstream leg, in-flight zero after drain, owner close, and rebind. Keep W1/W2/W3 correctness oracles and cache publication tests intact.
- [ ] From `rust/`, run `cargo fmt --all -- --check`, focused native-host tests, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and applicable existing Go/cgo regression checks from the repo root. Record exact commands, commit, failures and fixes.
- [ ] Build the pinned native binary for Linux amd64 and run W1/W2/W3 audit-on/off E2E on `ssh mosdns-rust`; do not use production `mos`. Run only the frozen valid low/moderate Rust-before/Rust-after probe. Report p50/p95/p99, correct-on-time throughput, CPU, RSS, audit-on overhead, raw hashes, and invalid stages. Treat unrepeatable or sender-limited runs as inconclusive.
- [ ] Ask the designated reviewer for a scoped A1–A6 review and fix findings. Update coverage/handover with bounded evidence, perform `trellis-check`/`trellis-update-spec` only where a lasting rule emerged, audit exact changed paths, and commit/push only task-owned changes. After final PASS, follow the normal finish/archive lifecycle; do not deploy or start the next task automatically.

## Review and rollback points

The most sensitive files are `rust/native-host/src/execution.rs`, `udp.rs`, `tcp.rs`, `assembly.rs`, and `config.rs`. Keep the observer isolated enough that an audit change can be reverted without altering DNS response construction or cache/route logic. A regression in response bytes, upstream counts, cancellation, or unaccounted audit loss blocks the slice. A repeatable p95/p99 or correct-on-time regression beyond the predeclared budget blocks final PASS until repaired or explicitly scoped into a separate corrective task.
