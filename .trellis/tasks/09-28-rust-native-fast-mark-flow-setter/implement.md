# Rust-native fast_mark and flow_setter — implementation plan

## Start gates

- [ ] Parent roadmap task receives same-conversation C2C `FINAL: PASS`.
- [ ] Existing canary passes its same-chat review, or the user explicitly
  defers it and authorizes this task to proceed without that result.
- [ ] Activate this child with `task.py start` only after the above gates.
- [ ] Read the Rust project handover/architecture, `.trellis/workflow.md`,
  package spec index, and `trellis-before-dev` context for `rust/native-host`;
  load `rust/sequence-core` context if changing it.

## Ordered behavior slices

### Slice 0 — freeze flow metadata precedence

- [ ] Trace Go `flow_setter` source and any observable tests, then trace native
  host writes to `matched_group`, `final_sequence`, and `final_upstream`.
- [ ] Write the evidence-based precedence table to `design.md` and agree the
  exact observer expectation before modifying product code.
- [ ] If contract evidence is insufficient, stop before code and ask the same
  C2C planning conversation for review of the ambiguity.

### Slice 1 — YAML and compile contract (red → green → refactor)

- [ ] Add failing tests for supported matcher/executable quick setup and normal
  args/reference forms, covering valid IDs at boundary values and path-aware
  compile rejection for malformed/out-of-range/unknown/unresolved/cross-type
  inputs.
- [ ] Implement typed config and compiler resolution using existing native
  compilation patterns. Do not open sockets or add process fallback paths.
- [ ] Refactor for a small shared representation only after behavior is green.

### Slice 2 — real per-query flag behavior (red → green → refactor)

- [ ] Add a failing native-listener integration test using real YAML and a
  controlled peer: matcher IDs OR together, executable sets bits without
  clearing unrelated bits, and a branch changes the peer-visible answer/count.
- [ ] In the same live process, send a second request proving no flag leaks
  between request states.
- [ ] Implement only the minimum sequence program/execution integration; retain
  existing direct-call/cache semantics.
- [ ] Refactor and prove the old composition integration remains unchanged.

### Slice 3 — asynchronous routing metadata and observation

- [ ] Add a failing integration using a delayed loopback forward to force
  sequence suspension/resume after `flow_setter`; assert all values in the
  observer record according to Slice 0's frozen precedence.
- [ ] Implement the minimum config, execution, routing-state, and observer
  changes. Use the existing `RoutingState`; avoid a duplicate metadata channel.
- [ ] Add focused tests for direct completion and async resume if either path
  can differ; then refactor.

## Validation

Run from `rust/`:

1. Focused new native-host tests and the existing
   `slice3_composition` regression integration.
2. `cargo fmt --all -- --check`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test --workspace`

Do not run benchmarks or claim performance/capacity results. Do not alter
front-end or Go builds.

## Evidence, review, and closeout

- [ ] Update `docs/rust/feature-coverage.md` only after tests pass, naming the
  implemented forms and evidence. Keep P11/P33/P44 and 5B rows open.
- [ ] Run `git diff --check`, inspect the exact source/test/doc diff, and commit
  only this task's paths; preserve all unrelated changes.
- [ ] Submit exact `BASE_SHA..HEAD_SHA`, task ID, and paths to the same C2C
  conversation for a complete task-range review. Do not paste logs or diff
  contents into ChatGPT.
- [ ] Fix every finding in a narrow corrective range and re-review in that same
  conversation until explicit `FINAL: PASS`.
- [ ] Record remaining unimplemented forms and explicit deferred work in the
  task and parent roadmap. Do not advance the 5C child until this C2C review
  passes.
