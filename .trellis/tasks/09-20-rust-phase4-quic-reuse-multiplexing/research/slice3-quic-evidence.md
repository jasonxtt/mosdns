# Slice 3 QUIC reuse evidence

Status: implementation and focused verification complete; the scoped web
review gate returned PASS. Linux/MSRV runtime verification is also complete;
the separate Rust 1.85 warnings-denied clippy run is blocked by pre-existing
workspace lint baseline findings recorded below. This record covers Slice 3
only and makes no claim about later task work.

## Implementation boundary

Slice 3 changes only focused real-stack fixtures/tests and this task evidence:

- `rust/upstream-core/tests/quic_reuse_doq.rs`
- `rust/upstream-core/tests/quic_reuse_doh3.rs`
- `.trellis/tasks/09-20-rust-phase4-quic-reuse-multiplexing/research/slice3-quic-evidence.md`

No production Rust implementation, Cargo manifest/lockfile, Go/cgo/FFI,
config/API/WebUI, TCP pool, or socket-policy surface is changed by Slice 3.

## Bounds, lifecycle, and typed backpressure

The existing model suite provides deterministic evidence for the non-I/O
contracts:

- `stream_slot_backpressure_and_permit_release` proves the local stream bound,
  typed pre-send backpressure, permit release, and no unbounded internal queue.
- `concurrent_distinct_key_admissions_never_exceed_the_owner_cap` proves the
  atomic `MAX_CONNECTIONS_PER_OWNER` bound (`8`) across distinct keys.
- `initializing_and_closing_entries_hold_their_slot_until_terminal`,
  `idle_expiry_marks_closing_without_removal`, and
  `same_key_closing_lookup_is_closed_not_sent_without_a_drain_wait` prove that
  an initializing/closing entry remains discoverable and occupies capacity,
  while a same-key Closing lookup returns `Closed(NotSent)` without waiting for
  teardown or opening a replacement.
- `init_vs_owner_close_hands_the_late_resource_to_teardown`,
  `owner_close_cancels_streams_and_waits_for_exchange_registrations`,
  `post_close_admission_race_leaves_no_residue`,
  `aborted_at_barrier_with_no_surviving_caller_reaches_terminal`, and
  `stale_generation_callbacks_never_touch_a_newer_generation` cover late
  initialization, concurrent close, no surviving caller registration, aborted
  exchange teardown, and generation isolation.

The real DoQ and DoH3 fixtures additionally set the peer-advertised concurrent
bidirectional stream limit to `1`. Two concurrent exchanges complete on the
same physical connection, retain one owner entry and one liveness registration,
and close cleanly. The tests are:

- `doq_peer_advertised_stream_limit_serializes_without_replacement`
- `doh3_peer_advertised_stream_limit_serializes_without_replacement`

The companion cancellation tests hold the first stream open while the peer
advertises only one bidirectional stream, start a second real exchange, and
cancel that exchange after the server has accepted the first stream. They
assert the typed cancellation result and one accepted connection/generation:

- `doq_peer_stream_limit_honors_pending_open_cancellation_without_replacement`
- `doh3_peer_stream_limit_honors_pending_open_cancellation_without_replacement`

These prove that pending peer stream credit is bounded by the original caller
cancellation path and does not create a duplicate connection. The DoH3 path
retains its pinned `MaybeSent` side-effect classification at the request-stream
boundary; the DoQ path remains `NotSent` before its bidirectional stream opens.

## Bounded concurrent stress

The real loopback stress fixtures run concurrent exchanges through one shared
generation:

- `doq_bounded_concurrent_stress_keeps_one_generation` runs up to
  `MAX_STREAMS_PER_CONNECTION` (capped at eight for bounded test cost), checks
  one accepted QUIC connection, one independent stream per query, one active
  owner entry/liveness registration, and the marker encoded in each caller's
  query is returned to that same caller.
- `doh3_bounded_concurrent_stress_keeps_one_generation` runs eight concurrent
  HTTP/3 request streams, checks one active entry/liveness registration, unique
  response IDs, and that each request's marker decoded from its `dns=` GET
  target returns to that exact caller, plus one authority across all requests.

The stress query itself carries a unique `markerN.example.org` question that
the loopback server decodes and reflects. The assertions compare each caller's
expected marker with its own response body, so a marker permutation across
concurrent streams fails even when the aggregate marker set and response-ID
set remain valid.

## Resolver composition boundary

Existing resolver tests exercise the authoritative `PublishedTarget` boundary
used by the QUIC constructors:

- `a_dual_a_selection_composes_a_doq_endpoint_without_rewriting_identity`
- `a_dual_aaaa_selection_composes_a_doq_endpoint_without_rewriting_identity`
- `the_quic_composition_boundary_is_a_numeric_selection_plus_caller_identity`
- `doh_composition_keeps_the_original_url_authority_and_path`

Together they verify that the selected A/AAAA numeric dial changes the composed
endpoint/key input while the caller's secure identity, DoH authority, and path
remain unchanged.

## Verification commands

All commands are run from `/Users/tom/github/mosdns-rust` with the locked
dependency graph:

| Command | Result |
|---|---:|
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_doq --locked -- --test-threads=1` | 0; 10 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_doh3 --locked -- --test-threads=1` | 0; 8 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_model --locked -- --test-threads=1` | 0; 21 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test resolver_dual_stack --test resolver_slice4 --locked -- --test-threads=1` | 0; 34 passed |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice3_quic --locked -- --test-threads=1` | 0; 23 passed (400.59s) |
| `cargo metadata --manifest-path rust/Cargo.toml --locked --format-version 1` | 0; locked metadata resolved |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` | 0 |
| `cargo test --manifest-path rust/Cargo.toml --workspace --locked` | 0; all workspace targets and doctests passed; `slice3_quic` 23/23 (224.79s) |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing` | 0; all validations passed |
| `git diff --check` | 0 |

The isolated Linux/MSRV run on `mosdns-rust` completed on Debian 13 with
`rustc/cargo 1.85.1`:

| Command | Result |
|---|---:|
| `rustup run 1.85.1 cargo metadata --locked --format-version 1` | 0 |
| `rustup run 1.85.1 cargo fmt --all -- --check` | 0 |
| `rustup run 1.85.1 cargo test --workspace --locked` | 0; all workspace targets and doctests passed; `slice3_quic` 23/23, DoQ 10/10, DoH3 8/8, model 21/21 |
| `rustup run 1.85.1 cargo clippy --workspace --all-targets --locked -- -D warnings` | non-zero; existing `dns-core` `clippy::precedence` findings (4) |
| `rustup run 1.85.1 cargo clippy -p mosdns-upstream-core --all-targets --locked -- -D warnings` | non-zero; the same existing `dns-core` findings (4) |

The MSRV clippy failures are outside the Slice 3 changed paths and were not
modified. The successful MSRV test run is valid Linux/runtime evidence; the
warnings-denied clippy baseline remains a separate repository quality issue.

The previously attempted `mos-test` SSH probe is superseded for this task:
the Rust branch test environment is `mosdns-rust`, which provided the Debian
13 and Rust 1.85.1 evidence above. The workspace declares MSRV 1.85 and the
locked metadata resolved successfully.

Before review, the parent also inspected the full diff, exact changed-path
boundary, and dirty-worktree preservation. No unrelated dirty file is staged.
