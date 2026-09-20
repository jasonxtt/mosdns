# Implement — Rust Phase 4 QUIC reuse and multiplexing

Status: planning only. Do not run `task.py start`, dispatch implementation, or edit
runtime code until the final planning summary is approved in a later user
message. No executor is selected; the selected reviewer is the user-provided
ChatGPT web conversation. The route is chosen and validated at dispatch time:
the executor must be an explicitly validated external-executor target (for
example `dsh-web`, Codex, or Herdr) and the reviewer must remain the selected
ChatGPT conversation. Before execution, explicitly select and validate the
executor target.

## 0. Pre-start gates

- [ ] Confirm `task.py current` points to
      `09-20-rust-phase4-quic-reuse-multiplexing` and status is `planning`.
- [ ] Review `prd.md`, `design.md`, and this file in full; resolve any
      material plan change before starting.
- [ ] Validate routing with `python3 ./.trellis/scripts/codex_routing.py validate`;
      the executor must be an explicitly validated external-executor target
      (`dsh-web`, Codex, or Herdr) and the reviewer must remain the selected
      ChatGPT conversation.
- [ ] Preserve all pre-existing dirty files. Record exact task-scoped paths before
      each external-executor apply; never use `git add -A`, reset, checkout,
      rebase, or broad cleanup.
- [ ] The existing locked QUIC/H3 dependency graph passes the Slice 0 audit. Any
      dependency change pauses the task for a revised planning/review gate.
- [ ] **R0 gates are closed before any Slice 1 network work.** R0a: the per-phase
      H3 request-stream cancellation contract (section 0.1 of `design.md`) is
      implemented and exercised by the Slice 0 **decision/state model only**,
      including the explicit drop-only phases; the real pinned-stack H3 loopback
      health proof is deferred to Slice 2/A5. R0b: the connection-level versus
      stream-level error classification table (section 0.2) is complete over the
      pinned `h3`/`h3-quinn`/`quinn` vocabulary and is the only source later
      slices use for logical deactivation. R0c: every pinned API assumption in
      `research/quic-reuse-evidence.md` is verified against the locked local
      registry source with an exact citation and a holds/does-not-hold result.
      R0 is **not** optional follow-up; Slice 1 does not start until R0 is
      reviewed PASS.
- [ ] If any pinned API cannot satisfy R0a/R0b/R0c, **stop and return to
      planning review**. Do not add, remove, or version-bump a dependency to work
      around it.
- [ ] The atomic multi-key admission contract (`design.md` §4.1), the single
      `Initializing` crossing/handoff protocol including publication
      (`design.md` §3), and the entry-owned supervised
      `Initializing/Active -> Closing -> Drained | Failed` teardown that alone
      performs terminal removal (`design.md` §7) are implemented in Slice 0 with
      their deterministic model tests.
- [ ] After the user approves this plan, run `python3 ./.trellis/scripts/task.py start
      rust-phase4-quic-reuse-multiplexing` (or the repository-equivalent start
      command) and only then dispatch Slice 0.

## 1. Slice 0 — QUIC reuse model, R0 gates, no network I/O

Allowed implementation surface:

- `rust/upstream-core/src/quic_reuse.rs` (new sibling module);
- minimal `src/lib.rs` module/re-export/error additions;
- focused pure tests and task-local evidence only.

Checklist:

- [ ] **R0a.** Record the per-phase H3 request-stream cancellation contract against
      the pinned `h3 0.0.8` / `h3-quinn 0.0.10` sources: before send, after
      request FIN, during response head, during body read. Mark each phase
      active-stop or drop-only, cite the pinned hazard
      (`RecvStream::poll_data` takes the `Option`; `stop_sending` unwraps it), and
      add the **decision/state-model** four-phase test. Slice 0 has no socket or
      QUIC/H3 I/O, so this is model evidence only: it proves the model never
      selects the panic path and keeps the logical shared-entry state healthy. Do
      not claim real H3 health here; Slice 2/A5 owns that loopback proof. No
      dependency change is allowed.
- [ ] **R0b.** Complete the connection-level versus stream-level error
      classification table over the pinned `h3`/`h3-quinn`/`quinn` vocabulary,
      independent of the `SideEffectState` decision. Entry-terminal errors
      logically deactivate by exact key+generation `Active -> Closing`; physical
      map removal stays a terminal `Drained`/`Failed` concern. Later slices must
      consume this table rather than classifying errors at call sites.
- [ ] **R0c.** Verify every pinned API assumption in
      `research/quic-reuse-evidence.md` against the locked local registry
      source with an exact file/line citation and an explicit
      holds/does-not-hold result.
      Record `quinn 0.11.7`, `h3 0.0.8`, `h3-quinn 0.0.10` unchanged. A mismatch
      stops the task for re-review.
- [ ] Define the closed protocol/ALPN discriminator and validated
      `QuicReuseKey` constructors for DoQ and DoH3.
- [ ] Include numeric dial, canonical identity, DoH3 authority where applicable,
      TLS mode, and roots revision; prove key equality/isolation deterministically.
- [ ] Define owner/entry state transitions with the explicit
      `Initializing -> Active -> Closing -> Drained | Failed` lifecycle,
      generation identity, stream-slot reservation, health states, idle
      timestamps, and typed backpressure/closed errors without opening a socket.
- [ ] Implement the single initialization crossing protocol (`design.md`
      §3.1/§3.2): owner close marks every `Initializing`/`Active` entry
      `Closing`/`TeardownRequested` at the shared lock linearization point without
      removing the reservation; entering `Closing` starts the supervised teardown
      exactly once; the initializer builds resources outside the lock and hands
      one result back under the same lock/handoff protocol, publishing `Active`
      only when the owner is still `Open` and the generation is still
      `Initializing`, otherwise handing the whole result (including a
      late-acquired resource) to the supervised teardown; no resource means the
      task records terminal `Failed` and removes exactly once.
- [ ] Implement cancellation-safe initializer execution ownership (`design.md`
      §3/§3.1/§3.2): installing the `Initializing` reservation starts and holds one
      entry-owned initializer task with a `JoinHandle` (or an equivalent
      entry-owned shared future plus guard) under the same lock — never a
      caller-owned future and never spawn-and-forget. Same-key callers only await
      the shared completion; a caller's `ExchangeControl`/deadline/cancellation
      ends only its own wait, and dropping the last waiter or every caller cannot
      stop the initializer. Its guard is released only at terminal, so exactly one
      completion is always delivered to the supervised teardown and an
      `Initializing`/`Closing` entry can never be stranded.
- [ ] Implement the atomic multi-key admission section (`design.md` §4.1): one
      no-await map critical section performing lookup, transition of
      dead/idle-expired entries to `Closing` (without removal), the capacity
      check counting `Initializing`/`Closing`/`Active` entries as occupied until
      the terminal `Drained`/`Failed`, and reservation/join/reuse.
- [ ] Implement the same-key lookup behavior (`design.md` §4.3): `Active` leased,
      `Initializing` joined by awaiting the **entry-owned** initializer under the
      caller deadline (cancelling only that wait, returning `Closed(NotSent)` if
      close wins), `Closing` returns `Closed(NotSent)` with no wait on drain, no
      second generation, and no lease of the `Closing` slot, and the same key may
      retry to admit a fresh generation only after the old entry reaches terminal
      removal.
- [ ] Implement the entry-owned supervised teardown task (`design.md` §7.2):
      started exactly once at the `Closing` transition; it holds the entry-owned
      initializer task/`JoinHandle`/guard and its completion/handoff, plus the
      driver/`JoinHandle`, shutdown signal, liveness guard, shared completion, and
      terminal-only map removal with slot/liveness release. Close callers only
      await the completion, and the admission path never awaits a drain.
- [ ] Freeze task-local bounds as finite non-configurable constants. Keep the
      proposed values (32 streams, 8 entries, 30 seconds lazy idle) explicitly
      implementation-only.
- [ ] Add pure tests for close/admission races, stale-generation deactivation,
      permit release, idle expiry, and no queue growth.
- [ ] Add the **multi-key cap concurrency test** (`design.md` §4.1): concurrent
      distinct-key admissions never exceed `MAX_CONNECTIONS_PER_OWNER`.
- [ ] Add the **same-key `Closing` lookup test** (`design.md` §4.3):
      `Closed(NotSent)` with no drain wait, no second generation, no slot reuse,
      and a successful fresh-generation admission only after terminal removal.
- [ ] Add the **init-vs-owner-close barrier test** (`design.md` §3.1/§11.1):
      abort/cancel the first initializer caller and drop every exchange waiter at
      the initializer barrier, then assert the entry-owned initializer task is
      still alive and yields exactly one completion; close then wins the
      linearization point first, the reservation is not removed, and the
      initializer completes and acquires its resource; assert `Active` is never
      published, the generation does not disappear, `Lifecycle` does not drain
      early, the resource is taken over and closed by the supervised teardown with
      no orphan or second generation, and removal plus slot/liveness release
      happen only at terminal `Drained`/`Failed`.
- [ ] Add the **aborted-at-barrier/no-surviving-caller supervised-teardown test**
      (`design.md` §11.1): abort the first close waiter, then drop all close
      waiter futures, and assert exactly one teardown runs to `Drained`/`Failed`
      with the driver supervised and terminal-only removal.
- [ ] Run the focused model tests, `cargo fmt --check`, and
      `cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings`.
- [ ] Parent inspects the complete external-executor diff and exact changed
      paths, reruns the focused checks, and sends the scoped Slice 0 evidence to
      the selected web reviewer.
- [ ] Stop after reviewer PASS. Slice 1 requires a new explicit user
      authorization; an internal Slice 0 PASS does not authorize it.

Suggested focused commands:

```bash
cargo test -p mosdns-upstream-core --test quic_reuse_model --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing
```

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
- [ ] Prove a connection-level failure only logically deactivates the exact key
      generation (`Active -> Closing`, immediately unleasable; map removal only
      at `Drained`/`Failed`) and the next independent query establishes one
      replacement after that terminal.
- [ ] Prove stream-local reset/malformed response remains stream-local when the
      shared connection is healthy.
- [ ] Rerun focused DoQ reuse tests plus all archived one-shot DoQ/QUIC tests.
- [ ] Parent inspects and applies only the reviewed patch, reruns tests in the
      parent worktree, and requests the same web reviewer’s scoped PASS.
- [ ] Stop after reviewer PASS. Slice 2 is separately authorized.

Suggested focused commands:

```bash
cargo test -p mosdns-upstream-core --test quic_reuse_doq --locked
cargo test -p mosdns-upstream-core --test slice1_doq --locked
cargo test -p mosdns-upstream-core --test slice3_quic --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
```

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
      long-lived driver task whose `JoinHandle` is handed to the entry-owned
      supervised teardown task before the entry publishes `Active`.
- [ ] Keep the H3 driver alive between requests; do not reuse the one-shot
      `H2ScopeLease` as the owner abstraction.
- [ ] Implement phase-aware request-stream cancellation against the R0a decision
      contract. Avoid the pinned h3-quinn missing-stream panic path and prove,
      with real pinned-stack loopback, that canceled requests do not close the
      shared connection, the driver, or another concurrent request (this is the
      Slice 2/A5 real-H3 evidence, not the Slice 0 model).
- [ ] Implement the entry-owned supervised teardown task (`design.md` §7.2) and
      owner close ordering: stop admission, mark `Initializing`/`Active` as
      `Closing`/`TeardownRequested` without removing reservations, have the
      supervised task await the initializer handoff (closing any late-acquired
      resource), cancel request streams, signal driver shutdown, close/force-close
      the QUIC connection if needed, await driver/stream cleanup, and only then
      remove the entry, release slot/liveness, and let `Lifecycle` finish; close
      callers only await the shared completion.
- [ ] Add deterministic close barriers for the supervised teardown, driver and
      stream drain, no-surviving-caller progress, concurrent close/idempotence
      tests, and zero-residue assertions.
- [ ] Rerun focused DoH3 reuse tests plus one-shot DoH3 and secure lifecycle tests.
- [ ] Parent inspects/applies the exact reviewed patch, reruns focused tests, and sends
      the scoped evidence to the selected web reviewer.
- [ ] Stop after reviewer PASS. Slice 3 is separately authorized.

Suggested focused commands:

```bash
cargo test -p mosdns-upstream-core --test quic_reuse_doh3 --locked
cargo test -p mosdns-upstream-core --test slice2_doh3 --locked
cargo test -p mosdns-upstream-core --test slice3_quic --locked
cargo fmt --all -- --check
cargo clippy -p mosdns-upstream-core --all-targets -- -D warnings
```

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
- [ ] Add idle expiry using the injected clock/maintenance path: the expired
      entry is marked `Closing` under the owner-map lock (never removed), stops
      being leasable, stays discoverable until terminal `Drained`/`Failed`, and
      its slot is not reusable early. Add connection-level logical deactivation,
      same-key-`Closing` retry-after-terminal, init-vs-close with late resource
      acquisition, close-vs-return race, concurrent close, no-surviving-caller,
      and aborted exchange tests.
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

```bash
cargo metadata --locked --format-version 1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
git diff --check
python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing
```

The isolated Linux/MSRV run must use the repository's established Debian/Rust
1.85 evidence path. Do not claim an MSRV runtime test that was not actually
installed and executed; record indirect evidence separately if the environment
cannot provide it.

## 5. Review and handoff protocol

For each external-executor slice:

1. Send a prompt beginning with the exact active task path and naming only the
   current slice, allowed files, required red-to-green checks, and forbidden
   scope.
2. Use bounded asynchronous external-executor waits; do not duplicate a
   still-running job.
3. After completion, inspect the complete external-executor diff, changed-path
   list, base, and task scope before applying anything.
4. Rerun focused tests in the parent worktree after applying the exact reviewed
   patch. An external-executor summary is not evidence by itself.
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
- If R0a cannot express a safe per-phase cancellation for a phase, that phase is
  drop-only and is recorded as such; do not work around it with a pin change.
  If R0b cannot classify an error, that is a Slice 0 gap that blocks Slice 1.
- If a pinned API assumption from `research/quic-reuse-evidence.md` does not hold,
  stop and return to planning review instead of changing `Cargo.toml`/`Cargo.lock`.
- If concurrent first users can create two connections for one key, if an
  `Initializing` entry can publish `Active` after close wins the shared lock, if a
  late-acquired resource is orphaned or a `Closing` reservation is removed before
  terminal, or if concurrent distinct-key admissions can exceed
  `MAX_CONNECTIONS_PER_OWNER`, stop before DoQ/H3 integration and fix the
  admission/crossing-handoff section.
- If a same-key lookup finding `Closing` blocks on drain, opens a second
  generation, or leases the `Closing` slot, stop Slice 0 and fix the lookup
  contract.
- If initializer execution can be stopped by aborting its first caller or by
  dropping the last/every exchange waiter, if its task/`JoinHandle`/guard is
  caller-owned or spawn-and-forget, or if a stranded `Initializing`/`Closing`
  entry can block its own completion, stop Slice 0 and fix the initializer
  ownership before anything else.
- If the entry-owned supervised teardown can be stopped by aborting the first
  close waiter or all close waiters, if it detaches the H3 driver/`JoinHandle`,
  strands an entry in `Closing`, or makes `Lifecycle` report drained early, stop
  Slice 0 and fix the teardown ownership.
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

- [ ] A1-A13 in `prd.md` mapped to focused/full evidence, including the R0
      pre-start gates (A12), the Slice 0 decision/state-model boundary, and the
      deterministic Slice 0 model tests (A13).
- [ ] Slice-by-slice external-executor reports and parent diff inspection.
- [ ] Explicit scoped PASS from the selected web reviewer.
- [ ] Linux/MSRV and bounded stress evidence recorded without overclaiming.
- [ ] `task.py validate`, final quality checks, and `git diff --check` pass.
- [ ] Any durable new convention is handled through the separate spec-update
      workflow; do not silently overwrite unrelated spec changes.
- [ ] User authorizes finish/archive as a separate action.
