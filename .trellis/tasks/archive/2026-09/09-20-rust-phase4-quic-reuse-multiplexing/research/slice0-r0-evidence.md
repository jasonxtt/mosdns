# Slice 0 R0 verification evidence

Recorded: 2026-09-20. Scope is Slice 0 only: the QUIC reuse model, the R0
pre-start gates, and deterministic pure tests with no socket and no QUIC/H3 I/O.
Allowed paths for the cited implementation and tests are
`rust/upstream-core/src/quic_reuse.rs` (+ minimal `src/lib.rs` additions) and
`rust/upstream-core/tests/quic_reuse_model.rs`. Paths below that start with a
crate and version (for example `h3-0.0.8/...`) are files inside the **locked
local registry source** under `~/.cargo/registry/src/<registry>/`, not files
copied into this repository. Registry paths were read directly; no line below
is inferred from memory.

Pinned versions verified unchanged and locked: `quinn 0.11.7`, `h3 0.0.8`,
`h3-quinn 0.0.10`. Manifest: `rust/upstream-core/Cargo.toml:80-82` pins the
three crates with `default-features = false`. Lockfile: `rust/Cargo.lock`
records `h3 0.0.8` (`:377-380`), `h3-quinn 0.0.10` (`:391-394`), and
`quinn 0.11.7` (`:792-795`). No dependency was added, removed, or version-bumped
for this slice.

## R0a — four-phase H3 cancellation decision model (model evidence only)

Decision table (`h3_cancellation_decision`, exercised by
`r0a_four_phase_decision_model_never_selects_the_pinned_recv_stop` and the
stream-local health test in `quic_reuse_model.rs`):

- Before send → `ActiveStop`/send-side stop (`RequestStream::stop_stream`).
- After request FIN → drop-only (receive side untouched).
- During response head → drop-only.
- During body read → drop-only.

The model never selects the pinned `Option::None` `stop_sending` path, and the
logical shared-entry state stays healthy (`h3_cancellation_effect` is
stream-local for every phase, and a stream-local classification changes nothing
on the entry in `r0a_stream_local_failure_leaves_the_shared_entry_healthy_and_leasable`).
This is the Slice 0 decision/state model only; it is not the pinned-stack H3
health proof, which belongs to Slice 2/A5.

Pinned hazard lines, each verified holds:

| # | Assumption | Locked source | Result |
| --- | --- | --- | --- |
| A1 | `h3_quinn::RecvStream::poll_data` moves the inner `Option<quinn::RecvStream>` into the in-flight `read_chunk_fut` and restores it only on completion | `h3-quinn-0.0.10/src/lib.rs:376` (`self.stream.take()`), `:383-384` (`ready!(...)` then `self.stream = Some(stream)`) | holds |
| A2 | `h3_quinn::RecvStream::stop_sending` unwraps that option, so a call after an aborted `poll_data` panics | `h3-quinn-0.0.10/src/lib.rs:391-397` (`.as_mut().unwrap().stop(...)`) | holds |
| A3 | `RequestStream::stop_sending` targets the receive direction; `stop_stream` targets the send direction | `h3-0.0.8/src/client/stream.rs:225` (`stop_sending`), `:251` (`stop_stream`) | holds |
| A4 | Dropping the stream leaves Quinn's `RecvStream::Drop` to stop unread receive data with code zero | `quinn-0.11.7/src/recv_stream.rs:500-515` (`impl Drop`, `stop(0u32.into())` when `!all_data_read`) | holds |
| A5 | `SendRequest::send_request` takes `&mut self` and `SendRequest` is cloneable, so the driver keeps one template and each query clones a sender | `h3-0.0.8/src/client/connection.rs:147` (`pub async fn send_request`, `&mut self`), `:109` (`pub struct SendRequest`), `:231-245` (`impl Clone`) | holds |
| A6 | Each H3 request maps to a fresh Quinn bidirectional stream | `h3-quinn-0.0.10/src/lib.rs:189-252` (`OpenStreams`, `poll_open_bidi` via `conn.open_bi()`, `impl Clone`) | holds |
| A7 | The H3 client driver must be polled to completion; `shutdown` starts graceful shutdown and `wait_idle` waits for closure | `h3-0.0.8/src/client/connection.rs:384` (`shutdown`), `:391` (`wait_idle`), `:397` (`poll_close`) | holds |

## R0b — error classification (authoritative pinned table)

The authoritative table is `PINNED_ERROR_CLASSIFICATION_TABLE` in
`src/quic_reuse.rs`, consumed by `classify_*` and asserted by
`r0b_connection_level_and_stream_level_classification`,
`r0b_side_effect_states_are_recorded_independently_of_the_class`, and
`r0b_table_covers_the_pinned_vocabulary`. `SideEffectState`
(`NotSent`/`MaybeSent`/`Sent`) is recorded independently of the class.

Connection/entry-terminal: every `quinn::ConnectionError` variant
(`quinn-proto-0.11.18/src/connection/mod.rs:3912-3939` —
`VersionMismatch`, `TransportError`, `ConnectionClosed`, `ApplicationClosed`,
`Reset`, `TimedOut`, `LocallyClosed`, `CidsExhausted` — `EntryTerminal`),
`read`/`write` `ConnectionLost`, every
`h3::quic::ConnectionErrorIncoming` variant, any `h3::error::ConnectionError`
(driver `poll_close` outcome), and the request-stream cases whose explicit
observation says backend connection error, peer GOAWAY closing, or driver
connection error.

Stream-local: per-request stream outcomes and framing/validation bounds
(`quinn` `Stopped`/`Reset`/`ClosedStream`/`IllegalOrderedRead`/`TooLong`,
`h3-quinn` `StreamTerminated`/`Unknown`, this crate's framing/validation
errors), plus the `h3 0.0.8` request-stream struct-shaped variants and an
unobserved `#[non_exhaustive]` remainder with no connection evidence. The
`quinn::ReadToEndError::Read` wrapper is expanded into exact nested rows, so
`Read(ConnectionLost)` remains entry-terminal while `Read(Reset)` remains
stream-local.

BQ2 conclusion (not a blocker): `h3 0.0.8` marks its whole `StreamError` enum
**and** every variant `#[non_exhaustive]` while the locked dependency
configuration leaves h3's opt-out feature disabled (`h3-0.0.8/src/lib.rs:20-21`),
so a downstream crate can only pattern-match the struct-shaped variants and can never
construct or match the tuple/unit variants `ConnectionError(_)`,
`RemoteClosing`, or `Undefined(_)` at all. The current
`classify_h3_stream_error` therefore cannot structurally route those three. This
is **not** a silent weakening: Slice 0 records the required discriminator model
in `H3ErrorObservation` and `classify_h3_request_outcome` instead of guessing
from a string or a bare enum shape.

- A backend `ConnectionErrorIncoming` is an explicit connection discriminator
  and is entry-terminal.
- A normal peer GOAWAY sets the shared `is_closing` flag through
  `process_goaway` (`connection.rs:663-701`) and returns `Ok`; it can therefore
  produce `RemoteClosing` while `poll_close` remains `Pending`. The explicit
  `H3ErrorObservation::PeerClosing` provenance is entry-terminal in that case.
- A driver-reported connection error is entry-terminal; otherwise a
  struct-shaped stream error or an `H3ErrorObservation::Unobserved` remainder
  with no connection observation is stream-local under the pinned producer paths
  (`connection_error_creators.rs:191-209` and `h3-quinn-0.0.10/src/lib.rs:407-435,
  486, 505`).

No nameable variant is folded into the wildcard, no future `#[non_exhaustive]`
variant can slip through silently (new matchable variants fail exhaustiveness
at compile time), and no dependency change was made to work around it. Slice 2
must bind the observation fields at call sites that own the real driver and
backend state.

## R0c — pinned API verification (holds/does-not-hold)

Every assumption in `research/quic-reuse-evidence.md` re-verified against the
locked local registry source:

| # | Assumption | Locked source | Result |
| --- | --- | --- | --- |
| C1 | `client::Builder::build` returns driver + cloneable `SendRequest`; `send_request` takes `&mut self`; `SendRequest: Clone` | `h3-0.0.8/src/client/connection.rs:109` (struct), `:147` (`pub async fn send_request`, `&mut self`), `:231-245` (`impl Clone`) | holds |
| C2 | `poll_close` must be polled continuously; `shutdown` starts graceful shutdown; `wait_idle` waits for closure | `h3-0.0.8/src/client/connection.rs:384` (`shutdown`), `:391` (`wait_idle`), `:397` (`poll_close`) | holds |
| C3 | `OpenStreams` cloneable; each H3 request maps to a fresh Quinn bidi stream | `h3-quinn-0.0.10/src/lib.rs:189-252` (struct, `poll_open_bidi` via `conn.open_bi()`, `impl Clone` at `:252-258`) | holds |
| C4 | `stop_stream` (send) vs `stop_sending` (receive); interrupted read leaves `Option::None`, then `stop_sending` unwraps | `h3-0.0.8/src/client/stream.rs:225`, `:251`; `h3-quinn-0.0.10/src/lib.rs:375-387` (take/restore), `:391-397` (unwrap) | holds |
| C5 | `quinn::Endpoint` cloneable with refcounted `EndpointRef`; keep handle alive; `close`/`wait_idle` for teardown | `quinn-0.11.7/src/endpoint.rs:46-47` (`#[derive(Debug, Clone)]`, struct), `:292` (`close`), `:316` (`wait_idle`), `:669`, `:702-707` (`EndpointRef`, refcount bump on clone) | holds |
| C6 | `quinn::Connection` cloneable; per-stream `open_bi`; `closed`/`close` for health/teardown | `quinn-0.11.7/src/connection.rs:290-291` (derive + struct), `:316` (`open_bi`), `:361` (`closed`), `:420` (`close`) | holds |
| C7 | `RecvStream::Drop` stops unread data with code zero when not fully read | `quinn-0.11.7/src/recv_stream.rs:500-515` | holds |

Additional verified findings (consistent with, not contradicting, the planning
evidence):

- The planning evidence's `connection.rs:109` was an approximate range: the exact
  locations are struct at `:109`, `&mut self` send at `:147-148`, and `Clone` at
  `:231-245` (holds).
- `h3::error::ConnectionError` and `h3::error::StreamError` mark the enum and
  every variant `#[non_exhaustive]` (`error.rs:16-19` and the per-variant
  `cfg_attr` blocks at `:24-27`, `:33-36`, `:84-87`, `:94-99`, `:105-110`,
  `:114-121`, `:126-131`, `:134-139`); downstream construction of
  `ConnectionError::Timeout` is still possible, matching of the tuple/unit
  variants is not. The opt-out feature exists and is deliberately disabled by
  the locked current configuration (`h3-0.0.8/src/lib.rs:20-21`; no dependency
  change). Recorded as the BQ2 conclusion above.
- A clean `H3_NO_ERROR` close is still a closed connection, hence terminal for
  reuse.

## R0d / A13 — deterministic Slice 0 model tests (all in `quic_reuse_model.rs`)

Same-run command and result:

```text
cargo test -p mosdns-upstream-core --test quic_reuse_model --locked

Result: `21 passed; 0 failed`.
```

- Multi-key cap concurrency: many simultaneous distinct-key admissions never
  exceed `MAX_CONNECTIONS_PER_OWNER`, the admitted-key count equals the cap
  exactly, and every excess gets the typed capacity error (A6).
- Same-key `Closing` lookup: `Closed(NotSent)` with no drain wait, no second
  generation, no slot lease; a fresh generation only after terminal removal (A6).
- Post-close admission race (§11.2): the registered-but-not-admitted exchange
  returns `Closed(NotSent)` with no reservation/initializer/second generation
  and zero liveness/slot residue; captured entries finish `Closing -> terminal`
  (A6/A13).
- Init-vs-owner-close barrier (§11.1): the first initializer caller is aborted
  and every exchange waiter dropped, yet the entry-owned initializer yields
  exactly one completion; lifecycle stage one wins before publication and the
  real close path then performs stage two; no `Active` is published; the
  late-acquired resource is closed by the supervised teardown; removal plus
  slot/liveness release happen only at terminal (A8).
- begin_close-to-accepting=false race (§11.3): both orders fail the three-way
  publication condition, with exactly one teardown to terminal (A8/A13).
- H3 non-exhaustive error observation model: known stream-scoped, unobserved
  remainder, backend connection, peer GOAWAY closing, and driver connection
  evidence each route through an explicit provenance discriminator with no
  string match.
- Aborted-at-barrier/no-surviving-caller (§11.1): first and all close waiters
  aborted, exactly one teardown reaches terminal autonomously with
  terminal-only removal (A5).
- Supporting model coverage: validated key isolation per dimension (A1),
  connection-vs-stream deactivation by exact key+generation, injected-clock idle
  expiry (mark `Closing` under the map lock, no removal, no lease, terminal
  removal), stale-generation deactivation as a no-op, stream-slot backpressure
  with permit release and no queue growth, and close-wait plus stream-cancel
  accounting.

R0 exit: R0a/R0b/R0c/R0d hold at model level with the dependency graph locked;
the Slice 2/A5 pinned-stack H3 proof is explicitly not claimed here.

## Out-of-scope confirmation for this slice

A2/A3/A4 (DoQ/DoH3 loopback), A5's real pinned-stack proof, and A9 resolver
composition are Slice 1-3 work and are not claimed in the tests or code above.
