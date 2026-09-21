# Implement — Rust Phase 4 QUIC reuse and multiplexing

Status: implementation complete; all four slices are accepted. The selected
executor and reviewer were explicitly fixed for this task, and the remaining
finish/archive gate is bookkeeping plus the separately recorded repository
MSRV-clippy baseline issue. No later task or production wiring is authorized
by this record.

The user's task-level approval covers all four planned slices. Execution is
sequential and automatic: dispatch the next slice only after the previous
slice's parent verification and explicit scoped reviewer `PASS`. A reviewer
`FAIL`, a planning/scope change, a pinned-API blocker, or an explicit user
pause stops progression at the current slice; no extra user confirmation is
required between scoped `PASS` results.

## Acceptance record

- **Slice 0 — ACCEPTED (2026-09-20):** parent verification passed, and the
  selected web reviewer returned an explicit scoped `PASS` for commit
  `4fe08c0eebed45efd304f9b197ce1d6e5d2608d5` against base
  `44fe6bac98d604f1a691dc0df295fbb7bdfc45b9`, with `P0: 0` and `P1: 0`.
  The reviewed scope was the four Slice 0 files only; Slice 1 is now the next
  authorized boundary.

- **Slice 1 — ACCEPTED (2026-09-21):** parent verification passed, and the
  selected web reviewer returned an explicit scoped `PASS` for remediation
  commit `399ccd625efc8e95f929eb266329fdd28603f3c9` against
  `a596eb90897fa3fff38c8a4e69cc686fcd9b2a10`, with `P0: 0` and `P1: 0`.
  The review confirmed the two remediation findings were closed and made no
  judgment on Slice 2 or Slice 3; continuous execution may proceed to Slice 2.

- **Slice 2 — ACCEPTED (2026-09-21):** parent verification passed, and the
  selected web reviewer returned an explicit scoped `PASS` for the final
  remediation commit `199892cebb4c634739c5cbd3cca3ebd2901dc8e5` against
  `ed7ffd093e562d3a76c832e09cc8f573f07de0df`, with `P0: 0` and `P1: 0`.
  The reviewed scope was exactly the three latest remediation paths:
  `rust/upstream-core/src/quic_reuse.rs`,
  `rust/upstream-core/tests/quic_reuse_doh3.rs`, and the Slice 2 evidence
  record. The review confirmed the real pinned-stack owner-close/held-handle
  terminal-ordering evidence and made no judgment on Slice 3; Slice 3 is now
  the next authorized boundary.

- **Slice 3 — ACCEPTED (2026-09-21):** parent verification passed, and the
  selected web reviewer returned an explicit scoped `PASS` for remediation
  commit `4e8972cca1e42f9d265725a5592593cfe193969a` against
  `cb07a0425b6d4977b80802abd034649bdc5d75db`, with `P0: 0` and `P1: 0`.
  The reviewed scope was exactly:
  `rust/upstream-core/tests/quic_reuse_doq.rs`,
  `rust/upstream-core/tests/quic_reuse_doh3.rs`, and the Slice 3 evidence
  record. The review confirmed both prior P1 findings were closed: real
  peer-credit pending-open cancellation and per-caller marker association for
  cross-query mixup detection. It made no judgment on later tasks or other
  slices.

## 0. Pre-start gates

- [ ] Confirm `task.py current` points to
      `09-20-rust-phase4-quic-reuse-multiplexing` and status is `planning`.
- [ ] Review `prd.md`, `design.md`, and this file in full; resolve any
      material plan change before starting.
- [ ] Validate routing with `python3 ./.trellis/scripts/codex_routing.py validate`;
      both the executor (`dsh-web`, Codex, or Herdr) and reviewer (ChatGPT or
      Codex) must be explicitly selected and validated for this conversation.
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
- [ ] Implement the atomic multi-key admission section with its model-only
      `accepting` (Open) gate (`design.md` §4.1): one no-await map critical
      section that **checks `accepting` first** (reject with `Closed(NotSent)`,
      installing nothing and starting no initializer), then performs lookup,
      transition of dead/idle-expired entries to `Closing` (without removal), the
      capacity check counting `Initializing`/`Closing`/`Active` entries as
      occupied until the terminal `Drained`/`Failed`, and reservation/join/reuse.
      This is the sole map-side admission-vs-close linearization and stage two of
      the two-stage owner close (`design.md` §7.3): stage one is
      `Lifecycle::begin_close` turning the real `Lifecycle` `Open -> Closing` and
      rejecting `register`, never executed inside the map lock. An exchange already
      registered but not yet admitted when close sets `accepting=false` must
      release that registration plus any local liveness guard before returning
      `Closed(NotSent)`, leaving zero liveness/slot residue and no new generation.
      `Initializing -> Active` publication requires `Lifecycle == Open`,
      `accepting == true`, and the exact generation `Initializing` under the one
      map/state lock; the real `Lifecycle` state and the map gate are independent,
      so neither check alone is sufficient.
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
- [ ] Add the **post-close admission race test** (`design.md` §4.1/§7.3/§11.2):
      an exchange that completed `Lifecycle::register` parks before map admission;
      close linearizes `accepting=false`; on release the admission returns
      `Closed(NotSent)`, installs no reservation/initializer/second generation,
      leaks no `Lifecycle` liveness or slot, and the entries captured by close
      still finish through the existing `Closing -> Drained`/`Failed` protocol.
- [ ] Add the **begin_close-to-accepting=false publication race test**
      (`design.md` §3.1/§7.3/§11.3): with an installed `Initializing` entry, run
      only stage one (`Lifecycle::begin_close`) while the map critical section is
      parked at a barrier so `accepting` is still `true`; the initializer's
      publication attempt must fail the three-way condition and never publish
      `Active`, taking the late-resource/supervised-teardown path instead. Then
      release the map barrier and assert the same non-`Active` outcome plus exactly
      one teardown to terminal; the inverse order must fail at the gate too.
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
- [ ] Stop the current slice at reviewer `PASS`, then automatically begin Slice
      1 using the same bounded executor/reviewer routing. Do not dispatch Slice
      1 before the scoped Slice 0 `PASS`; no additional user authorization is
      required unless scope changes or the user pauses.

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
- [ ] Stop the current slice at reviewer `PASS`, then automatically begin Slice
      2 using the same bounded executor/reviewer routing. Do not dispatch Slice
      2 before the scoped Slice 1 `PASS`; no additional user authorization is
      required unless scope changes or the user pauses.

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
- [ ] Stop the current slice at reviewer `PASS`, then automatically begin Slice
      3 using the same bounded executor/reviewer routing. Do not dispatch Slice
      3 before the scoped Slice 2 `PASS`; no additional user authorization is
      required unless scope changes or the user pauses.

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
- If an admission can install a reservation after owner close sets
  `accepting=false`, if a rejected post-close admission leaks a `Lifecycle`
  registration/liveness/slot, if close's scan can miss an entry that installed
  before `accepting=false`, if `Lifecycle::begin_close` runs inside the map lock,
  or if publication checks only `accepting` or only `Lifecycle` (so a stage-one
  close could still be followed by `Active`), stop Slice 0 and fix the
  admission-vs-close linearization and publication gate before anything else.
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

## Finish gate closure record

Recorded 2026-09-21 after the final Slice 3 review and the corrected Linux/MSRV
run on `ssh mosdns-rust`:

- A1-A13 are covered by the Slice 0–3 evidence records, the parent diff
  inspections, and the four accepted slice records above.
- The selected web reviewer returned scoped `PASS` for every slice, with
  `P0: 0` and `P1: 0`; Slice 3 remediation commit `4e8972c` is the final
  implementation commit and is pushed at `origin/rust`.
- Debian 13 with Rust 1.85.1 passed locked metadata, formatting, and the full
  workspace test suite, including the bounded QUIC stress coverage.
- The repository's current-toolchain workspace clippy gate passed on Rust 1.95.
  The separate Rust 1.85 `-D warnings` clippy probe reports only pre-existing
  baseline findings outside this task's allowed paths; it is recorded without
  overclaiming and is not folded into the Slice 3 diff.
- `task.py validate` and `git diff --check` pass. No dependency, production,
  configuration, Go/cgo/FFI, API/WebUI, or later-task changes were introduced.
