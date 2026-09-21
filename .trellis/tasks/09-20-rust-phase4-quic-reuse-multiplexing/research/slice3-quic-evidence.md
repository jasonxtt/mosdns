# Slice 3 QUIC reuse evidence

Status: implementation and focused verification in progress; the scoped web
review gate is pending. This record covers Slice 3 only and makes no claim
about later task work.

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

These prove that pending peer stream credit is bounded by the original
exchange deadline/cancellation path and does not create a duplicate
connection.

## Bounded concurrent stress

The real loopback stress fixtures run concurrent exchanges through one shared
generation:

- `doq_bounded_concurrent_stress_keeps_one_generation` runs up to
  `MAX_STREAMS_PER_CONNECTION` (capped at eight for bounded test cost), checks
  one accepted QUIC connection, one independent stream per query, one active
  owner entry/liveness registration, and unique response markers.
- `doh3_bounded_concurrent_stress_keeps_one_generation` runs eight concurrent
  HTTP/3 request streams, checks one active entry/liveness registration, unique
  response IDs and markers, and one authority across all requests.

The server response marker is independent per accepted stream; a complete
marker set and the caller response-ID checks therefore detect cross-query
response mixups rather than merely checking aggregate success.

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
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_doq --locked -- --test-threads=1` | 0; 9 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_doh3 --locked -- --test-threads=1` | 0; 7 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test quic_reuse_model --locked -- --test-threads=1` | 0; 21 passed |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test resolver_dual_stack --test resolver_slice4 --locked -- --test-threads=1` | 0; 34 passed |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 |
| `cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --test slice3_quic --locked -- --test-threads=1` | 0; 23 passed (400.59s) |
| `cargo metadata --manifest-path rust/Cargo.toml --locked --format-version 1` | 0; locked metadata resolved |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings` | 0 |
| `cargo test --manifest-path rust/Cargo.toml --workspace --locked` | 0; all workspace targets and doctests passed; `slice3_quic` 23/23 (224.79s) |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-quic-reuse-multiplexing` | 0; all validations passed |
| `git diff --check` | 0 |

The required Linux/MSRV SSH probe to the repository's `mos-test` Debian VM
(`ssh -o BatchMode=yes -o ConnectTimeout=5 mos-test ...`) timed out before any
remote command ran. This is recorded as unavailable evidence, not as a passing
Linux/MSRV runtime test. Local toolchain is `rustc/cargo 1.95.0`; the workspace
declares MSRV 1.85 and the locked metadata was resolved successfully.

Before review, the parent also inspected the full diff, exact changed-path
boundary, and dirty-worktree preservation. No unrelated dirty file is staged.
