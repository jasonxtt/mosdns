# Rust-native fast_mark and flow_setter — design

## Boundaries

The native host owns YAML decoding, compile-time reference validation,
sequence composition, query state initialization, and final observer record
construction. `sequence-core` owns reusable per-query execution state and
validated program semantics. A configured flag or route value must not be
stored in process-global state or inferred from a plugin name.

Relevant current contracts:

- `plugin/matcher/fast_mark/fast_mark.go` registers matcher and executable
  quick setup, accepts IDs 0–63, ORs matcher IDs, and sets executable IDs.
- `plugin/executable/flow_setter/flow_setter.go` accepts `group`, `sequence`,
  `upstream` quick keys and preserves normal argument values in the routing
  context fields `matched_group`, `final_sequence`, `final_upstream`.
- `docs/ai/config-notes.md` reserves `fast_mark` 48 and switch17 bit 49.
- `rust/sequence-core/src/state.rs` already carries `fast_flags: u64` and
  routing metadata fields. Do not create a parallel flags/metadata store.
- Native `execution.rs` supplies host-derived final sequence/upstream values;
  `observer.rs` currently exposes the latter two fields but not the configured
  group. Characterize when those derived values are written relative to
  `flow_setter` before choosing precedence.

## Data flow

```text
YAML plugin definition / sequence quick setup
                  │
                  ▼
path-aware config decode → typed matcher/executable specs → validated program
                                                           │
per-request fresh ExecutionState (flags + routing values) ─┘
                  │
                  ▼
native sequence runner ⇄ controlled async forward
                  │
                  ▼
single completed request → native observer record
```

The compiler must reject unsupported forms before sockets or upstream owners
are created. Runtime execution uses the same existing state that direct sequence
operations and async suspension already own.

## Configuration shape and compile errors

Follow the Go source and current native compiler patterns for accepted external
forms, while modeling only the typed information required by the native
runtime. Resolve matcher/executable names with the compiler's existing
collect-then-resolve graph. Do not embed hidden numeric meaning in names and do
not reserve a bit by silently rejecting an otherwise valid ID; only enforce
documented product/config reservations.

Validation cases must include negative ID, ID 64, malformed scalar/list,
unknown quick key, absent required value if the source contract requires one,
unresolved name, wrong reference type, and at least one valid scalar/list or
quick-setup form for each supported role. Errors should retain the field path.

## Flags semantics

- Matcher semantics are `(fast_flags & configured_mask) != 0` for the configured
  ID set, so multiple IDs OR together.
- Executable semantics are `fast_flags |= configured_mask`; unrelated bits
  remain intact.
- Construct a fresh execution state per admitted query. Reusing caches or
  sequence program objects must not reuse mutable per-query flags.
- Test cross-request isolation over one live UDP or TCP listener, not only by
  constructing two states directly.

## Flow metadata and precedence

Before implementation, inspect Go behavior/tests and native assignment sites in
`execution.rs` / `observer.rs`. Record a small contract table in this file:
configured `matched_group`, `final_sequence`, and `final_upstream` values;
which later terminal host values may supersede each; and which value is
serialized to the native observer. If the source evidence leaves ambiguity,
pause code changes and return the question through the C2C planning/review
conversation rather than inventing a policy.

The implementation should preserve one authoritative per-query `RoutingState`
through async suspension/resume. Populate the observer from that finalized
state plus the actual terminal host outcome, not from a separate plugin-local
copy. Extend only the in-process native observer record needed for this
behavior; leave full API/query audit management for later.

### Frozen precedence table (Slice 0 evidence)

| Field | Go evidence | Native host fallback | Final observer value |
| --- | --- | --- | --- |
| `matched_group` | `plugin/executable/flow_setter/flow_setter.go` stores `KeyMatchedGroup`; `coremain/audit.go` reads it at terminal audit construction | None | Configured value, if present |
| `final_sequence` | `flow_setter` stores `KeyFinalSequence`; the audit reads the stored value without replacing it | `ExecutionFacts::note_origin` records the real named sequence | Configured value wins; host execution position is used only when unset |
| `final_upstream` | `flow_setter` stores `KeyFinalUpstream`; the audit reads the stored value without replacing it | `ResponseSource::Upstream` records the actual response supplier | Configured value wins; actual response supplier is used only when unset |

The Go context is a per-query mutable map (`StoreValue` overwrites an existing
key), so a later `flow_setter` assignment replaces an earlier one. The native
executor therefore keeps the same per-query `RoutingState` across the await;
each later setter overwrites its field, and the observer resolves each field as
`configured.or(host_derived)`. The focused listener test in
`rust/native-host/tests/slice4_fast_mark_flow_setter.rs` verifies the three
configured values after a delayed forward, while the existing execution and
observer tests retain the host-derived fallback behavior.

## Test design and mock boundary

- Unit/compiler tests call the real YAML compiler and inspect typed compiled
  specs/errors. Do not mock `yaml_serde`, reference resolution, or program
  validation.
- Sequence tests use the real `ExecutionMachine` and state transitions.
- Native listener tests use controlled loopback peers so selection and response
  behavior are deterministic. A delayed peer can exercise the await/resume
  boundary. Mock only the external network endpoint if a targeted host-unit
  test cannot use a loopback peer.
- Observer tests assert actual values and precedence; do not stub host metadata
  construction.
- Existing `slice3_composition` remains a regression check. Add a focused
  integration file (suggested `slice4_fast_mark_flow_setter.rs`) rather than
  broadening unrelated tests.

## Compatibility and limits

This task delivers observable subitems only. It does not complete feature
rows P11/P33/P44, generic plugin registration, switch support, complete YAML
compatibility, all observer/audit paths, or production cutover. Update coverage
with exact tested forms and keep every unproven sibling pending.
