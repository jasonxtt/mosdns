# Implementation plan — Rust Phase 4 upstream connection reuse and pipeline foundation

Implementation status: implemented; controller independent verification and
reviewer PASS remain.

## Routing and worktree rules

- Executor after activation: user-selected Claude, Herdr pane `w6:p2`.
- Reviewer after the verified commit: the user-selected ChatGPT web project
  conversation for MosDNS Phase 4 upstream review.
- Current branch: `rust`; base branch: `rust`. Preserve the unrelated dirty
  documents and `.DS_Store` files already present in the worktree.
- Stage exact task/code paths only. Never `git add -A`, reset/rebase, force-push,
  or switch branches. Trellis auto-commit stays disabled.
- Forbidden: Go/cgo/FFI, YAML/config loader, API/WebUI, host/plugin/sequence
  wiring, production/default selection, installed-service mutation, port 53,
  QUIC/HTTP3/DoQ implementation, SOCKS/local bind/socket policy, server
  listeners, Go mirror/fallback, `MOSDNS_*_BACKEND` selectors, and deployment.
- Every slice is RED first: write the failing public contract test, record the
  real failure, then the minimum GREEN change, then a bounded refactor.

## Slice 0 — pure reuse key and owner model

- RED public tests for `ReuseKey` equivalence and discrimination: equal keys
  collide; differing numeric dial, transport, `ServerIdentity`, DoH authority,
  TLS policy discriminant, or negotiated ALPN do **not** collide.
- Add `ReuseKey`/`SecureKey` as pure values with no I/O and no socket.
- Test that a hostname never appears in a key and that the resolver snapshot is
  not an input.
- No connection is opened in this slice; no existing type changes.

## Slice 1 — serial reuse for plain TCP

- RED loopback tests: a second exchange for the same key reuses one accepted
  connection (the fixture counts accepts); a different key opens a new one; a
  half-closed idle connection is discarded and retried exactly once at
  `NotSent`; a `Sent`-state failure is terminal and discards the connection.
- Implement the owner with the settled serial policy: one short-mutex connection
  map, a lease guard whose drop returns-to-idle or discards, and checkout/insert
  with lazy idle expiry from an injected clock. One connection carries at most one
  outstanding query (`MAX_PENDING_PER_CONNECTION = 1`); no pending map and no
  reordering buffer are introduced.
- Reuse `tcp.rs` framing and `race_*` helpers verbatim; do not add a second
  framing implementation.
- Register leases with the existing `Lifecycle` so `drain()` covers them.

## Slice 2 — deadline, cancellation, close, and bounds

- RED tests: a reuse hit does not extend the caller's absolute deadline; caller
  cancellation and owner close stay typed and keep their precedence; `close()`
  drains leased connections, drops idle connections, refuses later checkouts,
  and is idempotent; a guard returning after `Closing` discards instead of
  inserting.
- RED tests for the confirmed task-local bounds — `MAX_IDLE_PER_KEY = 1`,
  `MAX_IDLE_TOTAL = 8`, `IDLE_TIMEOUT = 10s`, `MAX_PENDING_PER_CONNECTION = 1`;
  exceeding a bound yields a typed error (or discards an idle entry), never a
  panic or a silent drop.
- RED test for the settled serial decision: with a connection leased, a second
  concurrent request for the same key is typed-rejected or served by a separate
  connection, and the test asserts no ID rewrite and no second outstanding query
  on the same connection.
- Implement the bounds as non-configurable task-local constants with no wait
  queue. Assert bound *behavior* only; do not assert parity with Go's values and
  do not expose the constants as configuration.

## Slice 3 — secure reuse with identity preservation

- RED tests: a DoT connection authenticated as identity X is never used for
  identity Y; HTTP/1.1-negotiated and HTTP/2-negotiated DoH connections are never
  interchanged; DoH authority/path and DoT SNI remain the configured identity
  across a reuse hit (assert against `dot.identity()`, `doh.host()`,
  `doh.path()`).
- Implement secure checkout/insert on top of `DotUpstream`/`DohUpstream`
  prepare/validate paths, recording ALPN after the handshake and building pooled
  HTTP/2 on the existing `H2Children`/`H2ChildGuard` tracking.
- Prove the request bytes and `:authority` are byte-identical to the
  fresh-connection path for the same input.

## Slice 4 — resolver consumer boundary and deferred protocol contract

- Keep `ResolverComposition` and `ResolvedUpstream` numeric-only; verify a
  `ResolutionSnapshot::selected_target(now)` feeds the reuse owner's dial address
  without the snapshot entering the key, and that a resolver refresh does not
  disturb established connections.
- Document the QUIC/HTTP3 consumer boundary and the deferral list (including the
  separately-task-settled concurrent pipeline / ID demux work) as assertions or
  doc contracts only; add no QUIC code and no demux scaffolding.

## Slice 5 — quality, Linux evidence, and review gate

Run the local gates:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo tree --manifest-path rust/Cargo.toml --workspace -e features --locked
python3 ./.trellis/scripts/task.py validate rust-phase4-connection-reuse-pipeline
git diff --check
```

- Obtain Linux/Rust 1.85.1 loopback evidence through the isolated Debian VM via
  `ssh mosdns-rust`, into a dedicated staging path, with sha256-verified transfer
  and a task-scoped `CARGO_TARGET_DIR`. Do not use Mac Docker/Colima; do not
  mutate the installed service or `/root/mosdns-rust-build`; remove the staging
  directory afterwards.
- Independently inspect exact changed paths, the complete diff, status, branch,
  and commit identity; record explicit P0/P1 and PASS/FAIL. Send only the
  verified commit to the selected reviewer and do not infer PASS.

## Implementation evidence — Slices 0-4 (2026-09-18)

Evidence record for the **uncommitted** working tree on branch `rust` at
`cccadcd`. Final acceptance still requires the controller's exact-diff review,
commit/push, and the selected reviewer's explicit scoped PASS.

### Changed paths

- `rust/upstream-core/src/reuse.rs` (new): the reuse key, plain-TCP owner, the
  `DoT` secure owner, typed `PoolError`, the four confirmed bound constants, and
  the two serial leases.
- `rust/upstream-core/src/secure/dot.rs`: adds `PooledDotSession`
  (`connect`/`is_peer_closed`/`exchange`). It lives beside the protocol code that
  owns the handshake and framing so reuse composes the existing
  `write_frame`/`flush_bytes`/`read_frame` helpers and the same control race
  rather than duplicating a second DoT state machine. No existing item changed.
- `rust/upstream-core/src/secure/doh.rs`: adds `PooledDohSession` (HTTP/1.1 and
  HTTP/2 variants), `PooledDohOutcome`, the `DohSender` adapter, and
  `run_pooled_exchange`, and makes `H2ScopeLease::pooled` obtainable so a pooled
  session owns its own child-tracking scope with no owner registration and no
  caller token. Existing fresh-path items are unchanged; `H2ChildState._liveness`
  became optional to allow a pooled scope that holds no registration.
- `rust/upstream-core/src/secure/tls.rs`: `TlsPolicy` gains a private
  `roots_revision` (minted per `verified()`, sentinel `0` for
  `insecure_skip_verify`) plus a crate-private `roots_revision()` accessor, so the
  reuse key can separate two verified policies with different root stores. The
  public API is unchanged and no root material is stored or hashed.
- `rust/upstream-core/src/secure/mod.rs`: crate-internal `pub(crate) use` of
  `PooledDohSession` and `PooledDotSession`, so the reuse layer can reach the
  pooled-session types that live beside their protocol code. Nothing new is
  re-exported from the crate root.
- `rust/upstream-core/tests/reuse_doh.rs` (new, 7 tests): `DoH` session reuse over
  a real TLS loopback peer for **both** HTTP/1.1 and HTTP/2 (reuse proven by
  accepted-connection counts, plus h2 stream counts), authority isolation, the
  expired-deadline and close contracts, and the regression that cancelling the
  opening request's caller token must not kill a retained HTTP/2 session.
- `rust/upstream-core/tests/reuse_slice0_key.rs` (new, 24 tests): key contracts,
  plain-TCP reuse, deadline/cancel/close, bounds, and the resolver consumer
  boundary.
- `rust/upstream-core/tests/reuse_secure.rs` (new, 7 tests): `DoT` reuse with
  identity isolation, and `DoH` key/authority/ALPN discrimination.
- `rust/upstream-core/src/lib.rs` (**narrow additive export only**, flagged for
  the reviewer): `mod reuse;`, one `pub use reuse::{...}` block, and
  `TlsPolicy` added to the existing `secure` re-export so the secure reuse owner
  is constructible by callers. No existing item changed.

No other path changed. `rust/Cargo.toml` and `rust/Cargo.lock` are untouched, so
no dependency was added.

### What each slice delivered

- **Slice 0 (key).** `ReuseKey` is built only from a validated numeric endpoint
  plus secure material. `SecureKey` carries the kind, the normalized service
  identity text, the optional `DoH` authority, the TLS-policy discriminant, and
  the **roots revision** (see design decision 12); the negotiated protocol is a
  separate field refined after a handshake. Covered by discrimination tests for
  numeric dial, transport, identity, `DoH` authority, TLS policy mode, roots
  revision, and negotiated protocol — the cross-identity case (A2) is the
  highest-risk one. UDP endpoints are refused.
- **Slice 1 (serial plain-TCP reuse).** `ReuseOwner` checks out a retained
  connection for the endpoint's key, or dials one. Reuse is proven by counting
  *accepted* connections, not by timing: two and three sequential exchanges on one
  owner produce exactly one accept, while a different dial or a different owner
  produces its own.
- **Slice 2 (deadline, cancel, close, bounds).** An expired or cancelled caller
  context is refused before any socket work (0 accepts). `close()` drops idle
  entries, drains registrations, converges on `AlreadyClosed`, and refuses later
  exchanges; a connection returning after `Closing` is discarded rather than
  re-pooled. A stale idle entry past `IDLE_TIMEOUT` is evicted and the next
  exchange dials. A second concurrent request for a busy key is refused (no
  queue) rather than parked. The `MAX_IDLE_TOTAL` relationship is asserted as a
  compile-time invariant (`const { assert!(…) }`), and one owner — which holds
  exactly one key — is proven never to exceed `MAX_IDLE_PER_KEY`.
- **Slice 3 (secure reuse).** `SecureReuseOwner` dials and authenticates one
  `DoT` session through `PooledDotSession`, then serves later exchanges on that
  session. Verified end to end against a real TLS server: a second exchange for
  the same identity reuses one accepted connection; a **different identity on the
  same dial address dials and fails its own handshake rather than borrowing the
  authenticated session** (the security-critical case); a different dial address
  opens its own session; an expired deadline does not dial; `close()` is
  idempotent and drops idle sessions.
- **Slice 3b (`DoH` session reuse).** `DohReuseOwner` retains the negotiated
  session — HTTP/1.1 **and** HTTP/2 — and serves later exchanges on it. Both are
  proven against real TLS loopback servers, and reuse is asserted by counting
  accepted connections, not by timing: a second exchange of the same service
  produces exactly one accept. The HTTP/2 case additionally counts *streams*, so
  a single accepted connection carrying two h2 streams is distinguished from two
  connections carrying one each. A different authority on the same dial address
  is a different key and cannot be served by the retained session; an expired
  deadline does not dial; `close()` is idempotent and drops the session.
- **Slice 4 (resolver consumer boundary).** A `resolve_numeric` publication feeds
  the numeric dial address directly, and the key built from it carries no
  resolver state at all (no hostname, snapshot, or generation). A later resolver
  generation selecting the same numeric address yields the same key, so a refresh
  does not invalidate an established connection — asserted by a second exchange
  that still reuses the first connection.

### Design decisions worth reviewer attention

1. **Serial is structural, not a default.** `MAX_PENDING_PER_CONNECTION = 1` is
   enforced by a single lease in each pool state; there is no pending map and no
   reordering buffer anywhere in the module.
2. **A busy key uses the existing transport error set.** `PoolError::Busy` maps
   to `UpstreamError::Runtime(SideEffectState::NotSent)` rather than adding a new
   transport variant, keeping `UpstreamError` closed as reviewed. `PoolError`
   itself is exported for callers that want the pool-specific cause.
3. **Dead idle connections are detected before any write.** A retained connection
   the peer closed cannot be detected by writing (TCP accepts the bytes locally
   and only the read then fails, which would look like a sent-then-failed
   exchange). `is_peer_closed` therefore probes readiness non-destructively and
   takes no timeout; only then may a replacement dial happen, and only once.
4. **The clock is the resolver's existing trait.** `reuse` re-exports
   `crate::resolver::{Clock, SystemClock}` rather than defining a second time
   abstraction, so idle expiry uses the same injected-clock pattern.
5. **`SecureKey` stores identity text, not `ServerIdentity`.** `ServerIdentity`
   is deliberately not `Hash` in the reviewed endpoint contract, and changing it
   would be an unrelated API change; hashing the canonical text is equivalent for
   keying. `Transport` is mirrored by a local `TransportDiscriminant` for the
   same reason.
6. **The `DoT` pooled session lives in `secure/dot.rs`.** It reuses that module's
   private framing, control-race, and handshake-classification helpers, so
   placing it there avoids widening `secure::tls`/`secure::doh` visibility merely
   to reach them from a sibling module. Only `PooledDotSession` is shared, and
   it is exposed `pub(crate)`, not at the crate root.
7. **Secure identity is bound at construction, not at checkout.**
   `SecureReuseOwner` owns exactly one `DotEndpoint`, so its reuse key is fixed
   for its lifetime and a session authenticated for one identity can never be
   selected for another. The cross-identity test proves the refusal empirically.
8. **A pooled HTTP/2 session keeps its scope and performs no settle step.**
   `PooledDohSession::Http2` owns the `H2ScopeLease` whose executor Hyper
   dispatches its children to. The connection driver is itself one of those
   children and stays alive exactly as long as the session is usable, so waiting
   for the tracked child count to reach zero before reuse would wait for the
   connection to die — an actual deadlock, observed in testing and removed. The
   scope is instead held for the session's whole pooled lifetime and sealed (with
   children aborted) only on discard, owner close, or drop. Reuse safety comes
   from serial owner admission (`MAX_PENDING_PER_CONNECTION`) plus the rule that
   a session is handed back for retention **only** after a fully completed
   exchange; every terminal failure returns it as a discard.
9. **Pooled `DoH` validation keeps the negotiated version.** `run_pooled_exchange`
   takes the `SecureHttpVersion` its variant actually negotiated and threads it
   into `ValidatedDohResponse`, rather than assuming HTTP/1.1 as an earlier draft
   did. The lookup key deliberately omits the negotiated protocol: the session
   itself is the authority on which protocol it speaks, so a mismatch can only
   miss (the safe direction).
10. **A pooled HTTP/2 scope never captures a caller token.** `H2ScopeLease::pooled`
    takes only the owner token and installs a fresh, never-cancelled
    `TransportCancellation` as the scope's caller token. `PooledDohSession::connect`
    therefore no longer accepts a caller token at all. This was a review-caught
    defect: the pooled scope's executor races every tracked child against the
    scope's caller token, and the connection driver is one of those children, so
    binding the scope to the *first* request's caller token meant that cancelling
    that token after the first exchange succeeded silently killed the retained
    driver and broke every later reuse. Regression test:
    `a_cancelled_first_caller_token_does_not_kill_the_retained_http2_session`
    cancels the first token, then asserts the second exchange still succeeds on
    the same connection (`accepts = 1`, `streams = 2`). It was verified to
    *discriminate*: with the old wiring restored it fails with
    `Transport(Receive(MaybeSent))` on the second exchange. Cancellation for the
    exchange actually in progress is unaffected — it is enforced by that
    exchange's own `ExchangeControl` inside `run_pooled_exchange`, and a failure
    there hands the session back as a discard, which seals the scope and aborts
    its children. The fresh path (`H2ScopeLease::new`) still takes a real caller
    token and is unchanged.
11. **The secure pooled owners perform the final commit themselves.** A pooled
    session validates its response but owns no owner `Lifecycle`, so neither
    `PooledDotSession::exchange` nor `PooledDohSession::exchange` can run
    `commit_final_response`. This was a second review-caught defect of the same
    class: both owners called `release`/returned the response straight after the
    session reported `Ok`, leaving the whole window between response validation
    and the owner's return uncovered. An owner close, caller cancellation, or
    deadline landing in that window would let a pooled exchange report success
    where the fresh path would fail — the exact window the final-commit
    linearization point exists to close.
    `SecureReuseOwner::exchange` and `DohReuseOwner::exchange` now each call a
    private `commit_pooled_response` (`commit_final_response(
    &control.caller_cancellation(), deadline, SideEffectState::Sent)`) **after**
    the session returns `Ok` and **before** `release`, on both the reused-session
    and first-dial branches. On commit failure the session is dropped — never
    re-pooled — and the typed `SecureError::Transport(...)` is returned.
    (`Sent` is correct: the request had already been transmitted.)
    The plain-TCP `ReuseOwner` already committed in `run_framed` before returning
    `Attempt::Success`, so it needed no change; the fresh paths are untouched.

    Regression coverage (in-crate, in `reuse.rs`'s `cfg(test)` module, reusing
    the existing `CommitPause` seam, now `pub(crate)` so the owners can park on
    it):

    | Test | Owner | Window | Expected |
    | --- | --- | --- | --- |
    | `a_pooled_dot_response_loses_the_commit_gate_to_owner_close` | DoT | before commit | `Closed(Sent)`, idle stays 0 |
    | `a_pooled_dot_response_loses_the_commit_gate_to_caller_cancellation` | DoT | before commit | `Cancelled(Sent)`, idle stays 0 |
    | `a_pooled_doh_response_loses_the_commit_gate_to_owner_close` | DoH (H1) | before commit | `Closed(Sent)`, idle stays 0 |

    Each runs a real loopback TLS peer so the owner genuinely reaches its gate
    with a validated response, parks there, makes the commit lose, and then
    asserts both the typed error and that the session was **not** returned to the
    idle pool. All three were verified to **discriminate**: neutralising the
    respective `commit_pooled_response` (returning `Ok(())` immediately) turns
    them red with `owner close must fail the response` / `caller cancellation must
    fail`. Every wait in these tests is bounded — an earlier draft hung because
    the peer's drain loop had a second unbounded `read` and because the test-side
    `pause.arrived()` was unbounded; both are now wrapped in `POOL_TEST_TIMEOUT`
    (10s) so a broken path fails loudly instead of hanging.
12. **The TLS policy discriminator carries a roots revision, not just a mode
    bit.** This was a review-raised contract gap, and it was a **real deviation
    from the approved design**, not a false alarm: `design.md` rule 5 specifies
    that `tls_policy` discriminates on "verification mode **and** a roots
    revision identifier", but the implementation had reduced it to the single
    boolean `TlsPolicy::is_insecure_skip_verify()`. That boolean separates
    verified from insecure and nothing more, so two policies verified against
    **different** root stores produced byte-identical `SecureKey` values and were
    interchangeable at checkout.

    The fix follows the design as written:
    - `TlsPolicy` gains a private `roots_revision: u64`. `TlsPolicy::verified`
      mints a fresh value from a process-global monotonic `AtomicU64` starting at
      `1`; `insecure_skip_verify` (still `const`) uses the fixed sentinel `0`.
      `Clone` is preserved, and a clone keeps the same revision because it is the
      same configuration.
    - `SecureKey` gains the matching `roots_revision` field, so it participates
      in `Hash`/`Eq`.
    - `SecureReuseOwner` and `DohReuseOwner` build keys through the new
      crate-private `SecureKey::from_policy`, which reads both the mode and the
      revision from the owner's own policy.
    - The existing public `SecureKey::new` / `try_new` keep their exact
      signatures and record the sentinel `0`, so the reviewed public surface is
      unchanged. They identify a mode but no particular root store, which is
      exactly what "policy-agnostic" should mean.

    No trust material enters the key: the revision is an opaque ordinal, not the
    anchors and not a digest of them, which is what design rule 5 requires
    ("not on the root material, so rotating roots invalidates entries without
    leaking trust material into the key").

    Regression tests (in-crate, `reuse.rs`):
    - `two_separately_constructed_verified_policies_do_not_share_a_key` — builds
      two verified policies over **byte-identical** roots (the strictest form),
      asserts their keys differ, that one policy is self-consistent across calls,
      that a **clone** of it still matches, and that insecure never equals
      verified. Verified to **discriminate**: forcing the revision back to `0`
      (the pre-fix behaviour) makes it fail with both sides printing
      `roots_revision: 0`.
    - `the_policy_agnostic_key_stays_compatible_and_distinct` — pins the
      compatibility path: `new(.., false)` is stable, is **distinct** from a key
      built from a specific verified policy, and still agrees with the insecure
      policy key.

### Local gates (macOS)

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | clean, exit 0 |
| `cargo test --manifest-path rust/Cargo.toml --workspace --locked` | exit 0 — **644 tests passed, 0 failures**. The controller independently re-ran the workspace suite as `--workspace --all-targets --all-features --locked` and it also passed. |
| `cargo test … -p mosdns-upstream-core --lib --locked` | exit 0 — lib **90** (80 + 5 pool tests + 3 pooled final-commit tests + **2 policy-revision tests**) |
| `cargo test … -p mosdns-upstream-core --lib --locked reuse::tests` | exit 0 — **10 passed** |
| `cargo test … -p mosdns-upstream-core --test reuse_doh --locked` | exit 0 — **7 passed** (incl. the h2 reuse test and the cancellation regression test), 0 failed |
| `cargo test … -p mosdns-upstream-core --test reuse_secure --test reuse_slice0_key --locked` | exit 0 — reuse_secure **7**, reuse_slice0_key **24**, 0 failed |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings` | exit 0, clean — run by the executor **and** independently re-run by the controller |
| `cargo tree -e features --locked` (`rust/`) | exit 0 — no new dependency or feature |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-connection-reuse-pipeline` | All validations passed |
| `git diff --check` | exit 0, clean |

The reuse tests are deterministic: no wall-clock sleeps and no elapsed-time
polling. Reuse is asserted by connection-accept counts (and h2 stream counts for
the HTTP/2 case), expiry by an injected clock, and every await is wrapped in a
bounded guard. The pool-level tests are in-crate, so they use `#[cfg(test)]`
checkout/release seams on `ReuseOwner` rather than widening the public surface.

The exact controller-requested HTTP/2 command was run twice and passed both
times:

```
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core \
  --test reuse_doh --locked -- \
  --exact a_second_doh_exchange_reuses_one_http2_session --nocapture --test-threads=1
# run 1: test result: ok. 1 passed; 0 failed ... finished in 0.04s
# run 2: test result: ok. 1 passed; 0 failed ... finished in 0.03s
```

The caller-token regression test was verified to **discriminate**, not merely to
pass. Temporarily restoring the old wiring (`H2ScopeLease::with_liveness(None,
owner_cancellation, control.caller_cancellation())` in `connect`'s HTTP/2 branch)
turns it red:

```
# old wiring:
test a_cancelled_first_caller_token_does_not_kill_the_retained_http2_session ... FAILED
  panicked at tests/reuse_doh.rs:610:
  second h2 exchange after the first caller cancelled: Transport(Receive(MaybeSent))
# with the fix (H2ScopeLease::pooled(owner_cancellation)):
test a_cancelled_first_caller_token_does_not_kill_the_retained_http2_session ... ok
```

That failure mode is exactly the predicted defect: cancelling the first caller's
token aborted the retained connection driver, so the second exchange had to fail
on a session the pool still believed was live.

The three pooled final-commit tests were likewise verified to **discriminate**.
Neutralising the respective owner commit (making `commit_pooled_response` return
`Ok(())` immediately, i.e. the pre-fix behaviour) turns each red:

```
# DoT, commit neutralised:
test a_pooled_dot_response_loses_the_commit_gate_to_owner_close ... FAILED
  panicked at src/reuse.rs:2392: owner close must fail the response
# DoH, commit neutralised:
test a_pooled_doh_response_loses_the_commit_gate_to_owner_close ... FAILED
  panicked at src/reuse.rs:2323: owner close must fail the response
# both restored: ok
```

That is precisely the reported defect: without the owner-side commit the pooled
exchange returned success even though the owner had already begun closing.

### Isolated Debian VM — Rust 1.85.1

Transfer is `rsync -az --checksum` of only the local `rust/` workspace into
`/root/mosdns-rust-phase4-reuse`, excluding `.git/`, `target/`, `.cargo-target/`,
`.DS_Store`, and build artifacts, with a task-scoped `CARGO_TARGET_DIR`
(`.cargo-target`), unmodified manifest, and unchanged lock (`--locked`).
Toolchain: `rustc 1.85.1 (4eb161250 2025-03-15)`,
`cargo 1.85.1 (d73d2caf9 2024-12-31)`.

Rerun against the **current** tree (after the pooled-caller-token fix, the pooled
final-commit fix, and the roots-revision fix). These VM runs are executed by the
controller on the isolated host and reported here; they are not run from this
session:

| Remote command | Result |
| --- | --- |
| `cargo +1.85.1 test -j 2 --locked -p mosdns-upstream-core --lib --test reuse_doh --test reuse_secure --test reuse_slice0_key` | exit 0 — lib **90** (incl. all 10 `reuse::tests`), reuse_doh **7**, reuse_secure **7**, reuse_slice0_key **24**, 0 failed |
| `cargo +1.85.1 fmt --all -- --check` | clean, exit 0 |
| `cargo +1.85.1 clippy -p mosdns-upstream-core --all-targets --all-features --locked -- -D warnings -A clippy::precedence -A clippy::needless_lifetimes -A clippy::similar_names` | clean, exit 0 — nothing in this task's code |

The three `-A` allowances cover pre-existing findings in files this task does not
touch, all of which also appear on a pristine clippy-1.85 tree: `precedence` in
`dns-core/src/{edns,query,resolver,response}.rs`, `needless_lifetimes` in
`upstream-core/src/composite.rs:109`, `upstream-core/src/resolver/owner.rs:834`,
and `upstream-core/src/lib.rs:1109`, plus `similar_names` in old code. A full
`cargo +1.85.1 clippy --workspace` without those allowances therefore fails on
unmodified code, not on this task's; clippy 1.95 (the local macOS toolchain) no
longer reports `needless_lifetimes` at all, which is why the macOS workspace gate
is clean. The one `needless_lifetimes` finding that *was* in this task's new code
(`reuse.rs:624`) has been fixed.

<details>
<summary>Historical runs superseded by the rerun above (kept for the record)</summary>

**Pre-roots-revision run** (after the caller-token and final-commit fixes but
before the roots-revision fix, so it counts lib **88** = 80 + 5 pool + 3
final-commit tests and 8 `reuse::tests`):

| Remote command | Result |
| --- | --- |
| `cargo +1.85.1 test -j 2 --locked -p mosdns-upstream-core --lib --test reuse_doh --test reuse_secure --test reuse_slice0_key` | exit 0 — lib **88** (incl. all 8 `reuse::tests`), reuse_doh **7**, reuse_secure **7**, reuse_slice0_key **24**, 0 failed |
| `cargo +1.85.1 fmt --all -- --check` | clean, exit 0 |
| `cargo +1.85.1 clippy -p mosdns-upstream-core --all-targets --all-features --locked -- -D warnings -A clippy::precedence` | no finding in this task's code |

**Earliest run** (before both review-caught fixes), retained only as evidence
that HTTP/2 pooled reuse and multi-key eviction already worked on Rust 1.85.1 at
that revision. All eight changed files matched the local copies by sha256 at that
revision: `src/lib.rs` `dcf83c3d…`, `src/reuse.rs` `5b6915d0…`,
`src/secure/doh.rs` `2eeaf242…`, `src/secure/dot.rs` `21676f7f…`,
`src/secure/mod.rs` `3c8e56de…`, `tests/reuse_doh.rs` `bf2888d8…`,
`tests/reuse_secure.rs` `1c5ae5e4…`, `tests/reuse_slice0_key.rs` `18f6276f…`.

| Remote command | Result |
| --- | --- |
| `cargo +1.85.1 test -j 2 --locked --offline -p mosdns-upstream-core --lib --test reuse_doh --test reuse_secure --test reuse_slice0_key` | exit 0 — lib **85**, reuse_doh **6**, reuse_secure **7**, reuse_slice0_key **24**, 0 failed |
| `cargo +1.85.1 fmt --all -- --check` | clean, exit 0 |

</details>

Cleanup: the staging directory was removed and confirmed gone;
`/root/mosdns-rust-build` was left intact; the installed service was untouched
(`systemctl is-active mosdns` → `active`, PID 454 unchanged before and after,
and the build directory was never written to). No Mac Docker/Colima was used.

### Limitations and deferred work

- **Pooled `DoH` reuse is serial per session, and a reused HTTP/2 session has no
  between-exchange child settle.** `MAX_PENDING_PER_CONNECTION = 1` means one
  outstanding exchange per session, so no reordering or ID demux exists. For
  HTTP/2 specifically, the connection driver is itself a tracked child that lives
  as long as the session, so there is no point at which the child count reaches
  zero while the session is still reusable; the scope is therefore sealed only on
  discard/close/drop. This is correct for serial reuse, but a future
  concurrent-stream design must not reuse this scope shape unchanged.
- **`MAX_IDLE_TOTAL` eviction is exercised for the plain-TCP multi-key pool, not
  per-owner for the secure owners.** `ReuseOwner` is multi-key (per dial
  address), and
  `the_global_idle_bound_evicts_the_oldest_entry_across_keys` inserts
  `MAX_IDLE_TOTAL + 1 = 9` distinct keys and asserts exactly `MAX_IDLE_TOTAL = 8`
  are retained with the oldest evicted. `SecureReuseOwner` and `DohReuseOwner`
  each bind exactly one key for their lifetime, so with `MAX_IDLE_PER_KEY = 1`
  they structurally cannot exceed the global bound; they are covered by the
  per-key and closed-owner tests instead.
- **Cross-platform clippy.** macOS clippy 1.95 is clean workspace-wide. Linux
  clippy 1.85 is clean for `mosdns-upstream-core` once the pre-existing
  `precedence` / `needless_lifetimes` / `similar_names` findings in unmodified
  files are allowed; a bare `--workspace -D warnings` run fails on those old
  findings, not on this task's code. Details in the VM section above.
- **Process deviation, disclosed.** While appending the in-crate `mod tests`
  block to `reuse.rs`, a `cat >> … << 'RSEOF'` heredoc was used once, which
  violates the instruction to edit only with editor/apply-patch and not with
  shell writes. Every other write in this task used the editor. The block itself
  is ordinary reviewed Rust; only the write mechanism was wrong.
- Deferred and unchanged from the plan: same-connection multi-outstanding
  requests / reordering / ID demux, queue-based backpressure, QUIC/HTTP3, socket
  policy, protocol fallback, listeners, host/config/API wiring, and Go/cgo/FFI.

## Allowed paths

- `rust/upstream-core/src/**` for the new reuse owner module, its re-exports, and
  narrowly scoped changes to `tcp.rs`/`secure/**` only if a seam is genuinely
  required and is called out in the diff.
- `rust/upstream-core/tests/**` for new focused reuse/pipeline contract tests.
- this task's `prd.md`, `design.md`, `implement.md`, `research/**`,
  `implement.jsonl`, and `check.jsonl`.

## Forbidden paths

- `rust/upstream-core/src/resolver/**` — the resolver is complete; this task only
  consumes its read-only snapshot.
- `rust/dns-core/**`, Go runtime/config/API/WebUI paths, production config,
  service files, deployment artifacts, unrelated docs/specs.
- `rust/upstream-core/src/lib.rs` unless a narrowly justified additive export is
  required and explicitly reviewed in the diff.
- QUIC/HTTP3/DoQ, same-connection multi-outstanding requests / response
  reordering / ID demux (settled to a separate task), proxy/socket policy,
  listeners, host wiring, selectors, and fallback code.
- Any change that makes the task-local bound constants configurable
  (YAML/config/API), and any assertion of numeric parity with Go's
  64 / 10s / 30s values.

## Rollback points

- Before Slice 1: revert only the key/model commit; all existing transports are
  unchanged.
- Before Slice 3: revert the plain-TCP owner while retaining the key tests that
  document the contract.
- Before review: if a check fails, stop at the failing slice and do not broaden
  scope to repair unrelated pre-existing warnings.

## Non-goals（写入以约束范围）

This task is **not** "finish every upstream protocol", and it is **not** the
concurrent-pipeline work. It delivers controlled **serial** connection reuse for
plain TCP/DoT/DoH plus the key, lifecycle, bounds, and error contracts around it.

Out of scope, each requiring its own reviewed task:

- **Same-connection multi-outstanding requests, response reordering (RFC 7766
  §7), and original-ID demux** — settled to a separate task; it must first
  establish an unambiguous no-ID-rewrite correlation policy.
- QUIC/HTTP3/DoQ, UDP retransmission, SOCKS/local bind/socket policy, protocol
  fallback, connection-failure cross-family racing, server listeners,
  host/config/API/WebUI wiring, production selection, and Go/cgo/FFI.
- Making the task-local bound constants configurable, and any Go-parity claim.
