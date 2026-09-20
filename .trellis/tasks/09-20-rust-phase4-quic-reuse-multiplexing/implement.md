# Implement — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. Do not run `task.py start`, dispatch implementation, or edit
runtime code until the final planning summary is approved in a later user
message. The selected executor is MCP DSH Web; the selected reviewer is the
user-provided ChatGPT web conversation.

## 0. Pre-start gates

- [ ] Confirm `task.py current` points to
      `09-20-rust-phase4-quic-reuse-multiplexing` and status is `planning`.
- [ ] Review `prd.md`, `design.md`, and this file in full; resolve any
      material plan change before starting.
- [ ] Validate routing with `python3 ./.trellis/scripts/codex_routing.py validate`;
      executor must be `dsh:provider-managed` and reviewer must remain the
      selected ChatGPT conversation.
- [ ] Preserve all pre-existing dirty files. Record exact task-scoped paths before
      each DSH apply; never use `git add -A`, reset, checkout, rebase, or broad
      cleanup.
- [ ] The existing locked QUIC/H3 dependency graph passes the Slice 0 audit. Any
      dependency change pauses the task for a revised planning/review gate.
- [ ] After the user approves this plan, run `python3 ./.trellis/scripts/task.py start
      rust-phase4-quic-reuse-multiplexing` (or the repository-equivalent start
      command) and only then dispatch Slice 0.

## 1. Slice 0 — QUIC reuse model, no network I/O

Allowed implementation surface:

- `rust/upstream-core/src/quic_reuse.rs` (new sibling module);
- minimal `src/lib.rs` module/re-export/error additions;
- focused pure tests and task-local evidence only.

Checklist:

- [ ] Define the closed protocol/ALPN discriminator and validated
      `QuicReuseKey` constructors for DoQ and DoH3.
- [ ] Include numeric dial, canonical identity, DoH3 authority where applicable,
      TLS mode, and roots revision; prove key equality/isolation deterministically.
- [ ] Define owner/entry state transitions, generation identity, stream-slot
      reservation, health/dead/closing states, idle timestamps, and typed
      backpressure/closed errors without opening a socket.
- [ ] Freeze task-local bounds as finite non-configurable constants. Keep the
      proposed values (32 streams, 8 entries, 30 seconds lazy idle) explicitly
      implementation-only.
- [ ] Add pure tests for close/admission races, stale-generation eviction,
      permit release, idle expiry, and no queue growth.
- [ ] Run the focused model tests, `cargo fmt --check`, and
      `cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings`.
- [ ] Parent inspects the complete DSH diff and exact changed paths, reruns the
      focused checks, and sends the scoped Slice 0 evidence to the selected web
      reviewer.
- [ ] Stop after reviewer PASS. Slice 1 requires a new explicit user
      authorization; an internal Slice 0 PASS does not authorize it.

Suggested focused commands:

`bash
cargo test -p mosdns-upstream-core --test quic_reuse_model --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing
`

## 2. Slice 1 — shared DoQ connection

Allowed implementation surface:

- the Slice 0 QUIC reuse module and its minimal exports/errors;
- existing DoQ helpers in `rust/upstream-core/src/quic.rs` only where a
  shared-connection adapter is required;
- new focused DoQ reuse fixtures/tests and task evidence.

Checklist:

- [ ] Write RED loopback tests first: N concurrent queries, one QUIC accept,
      N independent bidirectional stream IDs, zeroed wire IDs, peer FIN, and
      original-ID/marker preservation.
- [ ] Implement one physical connection per DoQ key with single-flight connect,
      per-stream permits, caller-owned control/deadline races, and no replay.
- [ ] Prove one stream cancellation/timeout does not kill a healthy connection or
      another query; use the existing DoQ stream cancellation semantics.
- [ ] Prove a connection-level failure evicts only the exact key generation and
      the next independent query establishes one replacement.
- [ ] Prove stream-local reset/malformed response remains stream-local when the
      shared connection is healthy.
- [ ] Rerun focused DoQ reuse tests plus all archived one-shot DoQ/QUIC tests.
- [ ] Parent inspects and applies only the reviewed DSH patch, reruns tests in the
      parent worktree, and requests the same web reviewer’s scoped PASS.
- [ ] Stop after reviewer PASS. Slice 2 is separately authorized.

Suggested focused commands:

`bash
cargo test -p mosdns-upstream-core --test quic_reuse_doq --locked
cargo test -p mosdns-upstream-core --test slice1_doq --locked
cargo test -p mosdns-upstream-core --test slice3_quic --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
`

## 3. Slice 2 — shared DoH3 connection and owned driver

Allowed implementation surface:

- the QUIC reuse module and H3-specific helper types;
- existing H3 request/response validation helpers in `quic.rs`;
- new focused H3 reuse fixtures/tests and task evidence.

Checklist:

- [ ] Write RED tests first: one H3 connection, multiple concurrent request
      stream IDs, unchanged authority/path/headers, matching response markers,
      and original-ID restoration.
- [ ] Implement one-time H3 build per key, cloneable request sender, and a
      long-lived driver task with explicit health, shutdown command, join, and
      drain ownership.
- [ ] Keep the H3 driver alive between requests; do not reuse the one-shot
      `H2ScopeLease` as the owner abstraction.
- [ ] Implement phase-aware request-stream cancellation. Avoid the pinned
      h3-quinn missing-stream panic path and prove canceled requests do not
      close the shared connection.
- [ ] Implement owner close ordering: stop admission, cancel request streams,
      signal driver shutdown, close/force-close the QUIC connection if needed,
      await driver/stream cleanup, then allow `Lifecycle` to finish.
- [ ] Add deterministic close barriers for driver and stream drain, concurrent
      close/idempotence tests, and zero-residue assertions.
- [ ] Rerun focused DoH3 reuse tests plus one-shot DoH3 and secure lifecycle tests.
- [ ] Parent inspects/applies the exact DSH patch, reruns focused tests, and sends
      the scoped evidence to the selected web reviewer.
- [ ] Stop after reviewer PASS. Slice 3 is separately authorized.

Suggested focused commands:

`bash
cargo test -p mosdns-upstream-core --test quic_reuse_doh3 --locked
cargo test -p mosdns-upstream-core --test slice2_doh3 --locked
cargo test -p mosdns-upstream-core --test slice3_quic --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
`

## 4. Slice 3 — bounds, teardown, resolver composition, and stress

Allowed implementation surface:

- the completed QUIC reuse implementation;
- minimal resolver composition tests or helper additions if the existing
  `PublishedTarget` boundary cannot be exercised otherwise;
- focused stress/fixture tests and task evidence.

Checklist:

- [ ] Add local stream-slot and owner-entry bound tests with typed,
      pre-send backpressure and no unbounded queue.
- [ ] Add peer advertised stream-limit tests. The pending open path must obey the
      original deadline/cancellation and must not create duplicate connections.
- [ ] Add idle expiry using the injected clock/maintenance path, dead-entry
      replacement, close-vs-return race, concurrent close, and aborted exchange
      tests.
- [ ] Add A/AAAA `PublishedTarget` composition tests proving the selected
      numeric dial changes the key while identity/authority remain unchanged.
- [ ] Add bounded concurrent stress for DoQ and DoH3, checking connection count,
      stream count, lifecycle registrations, and no cross-query response mixups.
- [ ] Run the complete Rust quality gate, Linux/MSRV evidence, and final diff
      boundary review.
- [ ] Parent independently inspects the full task diff and exact staged paths,
      then sends one final scoped review request to the selected web reviewer.
- [ ] Stop for explicit reviewer PASS; do not archive or start later socket
      policy/listener tasks in this task.

Suggested final commands:

`bash
cargo metadata --locked --format-version 1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
git diff --check
python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing
`

The isolated Linux/MSRV run must use the repository's established Debian/Rust
1.85 evidence path. Do not claim an MSRV runtime test that was not actually
installed and executed; record indirect evidence separately if the environment
cannot provide it.

## 5. Review and handoff protocol

For each DSH slice:

1. Send a prompt beginning with the exact active task path and naming only the
   current slice, allowed files, required red-to-green checks, and forbidden
   scope.
2. Use bounded asynchronous DSH waits; do not duplicate a still-running job.
3. After completion, inspect the complete DSH diff, changed-path list, base, and
   task scope before applying anything.
4. Rerun focused tests in the parent worktree after applying the exact reviewed
   patch. A DSH summary is not evidence by itself.
5. Send the full slice diff, commands, exit statuses, and explicit forbidden-scope
   statement to the selected web reviewer URL.
6. Treat only an explicit scoped `PASS` as closure. Active, pending,
   unchanged, timed-out, or local-green review is not PASS.
7. A scoped FAIL permits only the requested in-scope remediation. A scope change
   returns to planning and requires user direction.
8. Do not commit/archive automatically. Finish and archive remain separate gates
   after the user confirms the completed task and reviewer evidence.

## 6. Risk and rollback points

- If the key cannot identify TLS/ALPN/authority isolation without exposing trust
  material, stop Slice 0 and revise the model.
- If concurrent first users can create two connections for one key, stop before
  DoQ/H3 integration and fix single-flight ownership.
- If a query cancellation closes a healthy shared connection, stop that slice;
  do not weaken the test.
- If the H3 driver can outlive owner close or a request can commit after close,
  stop Slice 2 and return to the lifecycle design.
- If peer stream limits cause an unbounded wait queue or duplicate connections,
  stop Slice 3 and revise backpressure.
- If any dependency, Go/cgo, socket-policy, listener, config, or production
  wiring appears in the diff, reject the patch as out of scope.
- Rollback is a task-scoped revert of new QUIC reuse paths and tests only.
  Preserve unrelated pre-existing changes and `.DS_Store` files.

## 7. Finish gate

Before any future archive/finish action, all of the following must be present:

- [ ] A1-A11 in `prd.md` mapped to focused/full evidence.
- [ ] Slice-by-slice DSH reports and parent diff inspection.
- [ ] Explicit scoped PASS from the selected web reviewer.
- [ ] Linux/MSRV and bounded stress evidence recorded without overclaiming.
- [ ] `task.py validate`, final quality checks, and `git diff --check` pass.
- [ ] Any durable new convention is handled through the separate spec-update
      workflow; do not silently overwrite unrelated spec changes.
- [ ] User authorizes finish/archive as a separate action.
