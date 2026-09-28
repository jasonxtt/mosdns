# Rust-native fast_mark and flow_setter — implementation plan

## Start gates

- [x] Parent roadmap task receives same-conversation C2C `FINAL: PASS`.
- [x] Existing canary passes its same-chat review, or the user explicitly
      defers it and authorizes this task to proceed without that result.
- [x] Activate this child with `task.py start` only after the above gates.
- [x] Read the Rust project handover/architecture, `.trellis/workflow.md`,
      package spec index, and `trellis-before-dev` context for `rust/native-host`;
      load `rust/sequence-core` context if changing it.

## Ordered behavior slices

### Slice 0 — freeze flow metadata precedence

- [x] Trace Go `flow_setter` source and any observable tests, then trace native
      host writes to `matched_group`, `final_sequence`, and `final_upstream`.
- [x] Write the evidence-based precedence table to `design.md` and agree the
      exact observer expectation before modifying product code.
- [x] If contract evidence is insufficient, stop before code and ask the same
      C2C planning conversation for review of the ambiguity.

### Slice 1 — YAML and compile contract (red → green → refactor)

- [x] Add failing tests for supported matcher/executable quick setup and normal
      args/reference forms, covering valid IDs at boundary values and path-aware
      compile rejection for malformed/out-of-range/unknown/unresolved/cross-type
      inputs.
- [x] Implement typed config and compiler resolution using existing native
      compilation patterns. Do not open sockets or add process fallback paths.
- [x] Refactor for a small shared representation only after behavior is green.

### Slice 2 — real per-query flag behavior (red → green → refactor)

- [x] Add a failing native-listener integration test using real YAML and a
      controlled peer: matcher IDs OR together, executable sets bits without
      clearing unrelated bits, and a branch changes the peer-visible answer/count.
- [x] In the same live process, send a second request proving no flag leaks
      between request states.
- [x] Implement only the minimum sequence program/execution integration; retain
      existing direct-call/cache semantics.
- [x] Refactor and prove the old composition integration remains unchanged.

### Slice 3 — asynchronous routing metadata and observation

- [x] Add a failing integration using a delayed loopback forward to force
      sequence suspension/resume after `flow_setter`; assert all values in the
      observer record according to Slice 0's frozen precedence.
- [x] Implement the minimum config, execution, routing-state, and observer
      changes. Use the existing `RoutingState`; avoid a duplicate metadata channel.
- [x] Add focused tests for direct completion and async resume if either path
      can differ; then refactor.

## Validation

Run from `rust/`:

- [x] Focused new native-host tests and the existing
   `slice3_composition` regression integration.
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`

Validation evidence: focused native-host tests, `slice3_composition` (11/11),
native-host unit/integration tests (60 unit tests plus all integration suites),
workspace clippy with warnings denied, and the complete workspace test/doctest
run passed. The full workspace run included the existing long DoH3/QUIC
boundary suite; no benchmark or performance claim is made.

Do not run benchmarks or claim performance/capacity results. Do not alter
front-end or Go builds.

## Evidence, review, and closeout

- [x] Update `docs/rust/feature-coverage.md` only after tests pass, naming the
      implemented forms and evidence. Keep P11/P33/P44 and 5B rows open.
- [x] Run `git diff --check`, inspect the exact source/test/doc diff, and commit
      only this task's paths; preserve all unrelated changes.
- [x] Submit exact `BASE_SHA..HEAD_SHA`, task ID, and paths to the same C2C
  conversation for a complete task-range review. Do not paste logs or diff
  contents into ChatGPT.
- [x] Fix every finding in a narrow corrective range and re-review in that same
  conversation until explicit `FINAL: PASS`.
- Review finding ledger: the initial exact-range C2C review returned
  `FINAL: FAIL` with P1-1 because the cancellation/drop checkpoint used the
  host-derived `final_sequence` even when `flow_setter` had configured one.
  The narrow correction applies configured-over-host precedence in
  `ExecutionFacts::Drop` and adds a cancellation regression covering all three
  routing fields.
- [x] Record remaining unimplemented forms and explicit deferred work in the
  task and parent roadmap. Do not advance the 5C child until this C2C review
  passes.

Review evidence: the initial implementation range
`1cd8b80759683840895bf1b054f7086ffd00d885..4eb4a565de2b977de52b1233b265eac2afba4891`
received C2C `FINAL: FAIL` with P1-1. The narrow correction range
`4eb4a565de2b977de52b1233b265eac2afba4891..da2f7aedbe9bd2e13b37286fd2b67e2e9627b111`
received the same-chat C2C `FINAL: PASS`; P1-1 was closed.
