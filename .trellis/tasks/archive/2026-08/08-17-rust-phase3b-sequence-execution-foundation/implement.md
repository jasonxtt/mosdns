# Phase 3B sequence execution foundation implementation plan

## Planning and start gate

- Keep `task.json` in `planning`; do not run `task.py start` during this
  planning revision.
- Obtain explicit planning root approval for `prd.md`, `design.md`, and this
  file. Only then activate the task and run `trellis-before-dev` for the Rust
  crate and any narrowly required Go characterization package.
- Preserve all unrelated dirty worktree files. Do not add cgo, ABI, runtime
  exports, selectors, production sequence wiring, or network code.
- Execute one behavior slice at a time. Stop at each stated review gate; do
  not silently broaden the no-network scope to other plugins.

## Slice 0 — product-contract/deviation classification

### Contract extraction work

- Freeze the classification table from `design.md` before writing Rust code:
  `preserve`, `intentional Rust deviation`, or `implementation-only`.
- Use existing YAML/config semantics, docs and current source/tests to confirm
  the preserved sequence-language contract: declaration order,
  short-circuiting, no-op/multi-exec forms, control-flow built-ins, configured
  reject RCODE range, repeated matcher/executable kinds, target semantics and
  reviewed routing/audit labels.
- Add the smallest focused inline characterization before Rust implementation
  and freeze its scope semantics: a multi-exec list normalizes to one
  `Inline(SequenceId)` executable; inline fall-through, `return`, `accept`, and
  `reject` end only the inline scope and resume the outer next rule; `jump`
  returns to the next inline item/rule; `goto` replaces only the inline local
  continuation and, after its target completes, resumes the outer next rule;
  `exit` propagates out unless a nested `try` catches it. Also cover an inline
  list containing `exec1`, `try(target sequence/fixture)`, and `exec3`: normal
  try completion and caught `Exit` continue with `exec3`, while ordinary
  error, `Cancelled`, or `BudgetExceeded` propagates out; the completed inline
  scope then resumes the outer next rule. Do not add a direct try target for
  the synthetic inline sequence.
  These tests are the product-contract confirmation for the scope table in
  `design.md`, not an invitation to infer behavior during implementation.
- Do **not** add Go characterization merely to copy internal behavior. Add the
  smallest read-only Go test only when an externally visible contract remains
  unresolved after source/document review.
- Record the intentional deviations already approved by planning: typed error
  on malformed raw response and bounded fuel/cancellation for cyclic control
  flow.

### Verification

- If Go characterization is added, run the focused package test/race/vet gates
  for only the touched packages.
- `git diff --check`
- Review that every Go-derived fact used by Rust is explicitly classified; no
  `ChainWalker`, normal/fast duplication, naming trick, cgo selector or Go
  fallback may become normative by implication.

### Review gate

Stop and root-review the contract/deviation matrix before writing Rust
behavior. An unresolved product-contract question returns to planning; an
implementation-only Go quirk is not a reason to add Rust parity code.

## Slice 1 — typed state and deterministic response transitions

### Red tests first

- Add `rust/sequence-core` unit tests for the closed `ExecutionState` schema:
  owned query/question data, sorted marks, `u64` flags, typed routing fields,
  and the absence of arbitrary `any`/Go-pointer storage.
- Add response-state red tests for every transition in `design.md`, including
  complete-wire retention after valid inspection, no lossy decoded state,
  synthesized reject response, TTL-only inspection, and the intentional
  malformed-raw typed error without an implicit state transition. Do not add a
  raw-response RCODE/EDNS extended-RCODE parser or widen `dns-core` in this
  slice.
- Add RCODE boundary tests for `0`, `15`, `0xFFF`, and reject values above
  `0xFFF`.

### Minimum implementation

- Add the isolated `rust/sequence-core` crate and workspace membership only;
  keep it pure Rust with `mosdns-dns-core` as its only project-internal
  dependency.
- Implement `ExecutionState`, `QueryState`, `RoutingState`, `ResponseState`,
  `OwnedResponseWire`, `SynthesizedResponse`, the non-consuming response
  inspection seam returning only the supported TTL observation, canonical
  snapshots, and typed state mutation helpers.
- Do not add runtime exports, C records, ABI capabilities, Go bridge code, or
  production plugin changes.

### Verification and rollback

- `cargo fmt --manifest-path rust/Cargo.toml --all --check`
- `cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked`
- `cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked -- -D warnings`
- If the state schema needs an unapproved generic escape hatch, stop and
  return to planning; do not implement it. Removing the isolated crate and
  its tests is the rollback path.

## Slice 2 — `ProgramSpec` normalization and matcher dispatch

### Red tests first

- Add Rust tests for the complete `ProgramSpec -> ValidatedProgram` boundary:
  one exec, no exec, an empty exec list, and a multi-exec list that preserves
  declaration order, becomes an inline child sequence, and produces a runtime
  `ValidatedExecutable::Inline { target }` variant.
- Explicitly accept repeated matcher kinds, repeated executable kinds in a
  multi-exec, and repeated calls to the same fixture target. Explicitly reject
  duplicate sequence names within the sequence catalog and duplicate fixture
  names within the fixture catalog before execution; sequence and fixture
  namespaces may reuse the same spelling.
- Add target-resolution tests for `goto`/`jump` sequence targets, `try` to a
  sequence target, and `try` to a plain fixture executable. Missing targets,
  wrong target kinds, unknown matcher/executable kinds, invalid RCODE, and
  malformed definitions must be rejected before execution and before any
  `ExecutionState` mutation.
- Add fake matcher tests for declaration order, false/error short-circuit,
  the single typed mutation channel, and mutation order. Verify that reverse
  flips the boolean only, never rolls back matcher mutation, and that
  dispatcher metadata runs only after the effective match decision.
- Verify existing `domain_set` is not overwritten, false matchers skip later
  matchers/executables, and mutations from already completed matchers remain
  deterministic if a later matcher errors.
- Verify the borrowed-state API leaves the same caller-owned state observable
  after `Completed`, `Exited`, matcher/executor error, `Cancelled`, and
  `BudgetExceeded`.
- Add typed metadata tests for positive qname, switch6, switch5, write-once
  state, and the explicit reverse semantic that reversed membership never
  claims the positive set/rule label. Do not reproduce Go normal/fast paths.

### Minimum implementation

- Implement the unvalidated `ProgramSpec`/`SequenceSpec`/`RuleSpec` model,
  symbolic target references, fixture catalog, normalization, stable IDs, and
  `ValidatedProgram`. Compile multi-exec lists to inline child sequences and
  resolve `goto`/`jump`/`try` targets before exposing the program to execution.
- Implement `Matcher` with an immutable `&ExecutionState` view,
  `MatchOutcome`, `StateMutation`, `MatcherSpec`, and typed
  `DispatchMetadata`; there must be no direct state-mutation alternative.
- Implement ordered rule dispatch without parsing Go matcher names or invoking
  Go callbacks. Apply matcher mutation, reverse the boolean, then apply the
  reviewed positive dispatcher metadata with write-once routing semantics.

### Verification and review gate

- Run sequence-core focused tests and clippy; rerun any narrowly touched Go
  characterization tests only if Slice 0 required them.
- Review observable state snapshots against the frozen product contract.
- Stop for root review if normalization, target resolution, reverse matching,
  mutation order, or metadata semantics differ from the approved Rust
  contract. Go implementation-only differences are not failures.

## Slice 3 — explicit continuation control flow

### Red tests first

- Add tests for accept/reject terminal behavior, default and explicit RCODE,
  goto without return, jump with return continuation, jump-at-end, top-level
  return, and exit.
- Add tests for try swallowing only Exit, continuing afterward, propagating
  ordinary executor/matcher errors, and targeting both a sequence and a plain
  executable.
- Add an inline multi-exec fixture whose middle executable is `try` to a user
  sequence or fixture; verify normal/Exit continuation to the next inline
  executable and propagation of ordinary error, cancellation, and budget
  exhaustion. Do not add direct `try -> Inline` construction.
- Add shared-control tests: a try recursion/cycle cannot reset fuel; nested
  try shares the root fuel and observes the same cancellation; a child
  `BudgetExceeded`, `Cancelled`, or ordinary error is never swallowed, while
  child `Exit` is swallowed and lets the parent continue.
- Add invalid-target and no-panic tests for every control-flow definition.

### Minimum implementation

- Implement typed executable variants, including `Inline(SequenceId)`, and an
  explicit frame/continuation stack. Do not copy recursive `ChainWalker` calls
  into Rust.
- Implement the exact goto/jump/return/fall-through semantics from
  `design.md`; keep `accept` and `reject` terminal.
- Pass one root `ExecutionControl` and the shared `ExecutionState` through
  every nested try child. Detach only the child's continuation stack; never
  allocate a fresh fuel budget or cancellation state for a child.
- Return `Result<ExecutionCompletion, ExecutionError>` from the borrowed-state
  root/internal API. Treat Exit as a control signal only; `try` alone may
  convert it into continuation. Never return state by value or hide it on an
  error path.

### Verification and rollback

- Focused sequence-core tests and clippy pass.
- A failed preserved product-contract case rolls back the slice to its red
  tests. Intentional deviations are judged against their Rust contract, not Go
  parity.

## Slice 4 — fuel, cancellation, and contract closure

### Red tests first

- Add a cyclic goto/jump fixture that must return `BudgetExceeded` within a
  fixed step count and never recurse or hang.
- Add cancellation tests at the defined dispatch boundary, including the
  priority cases `Cancelled > BudgetExceeded > ordinary error > Exit`.
- Add nested-try resource tests that prove the same root fuel/cancellation
  control reaches every child and that `try A -> try B -> try A` cannot evade
  the budget by resetting it.
- Add canonical snapshot contract tests for the complete reviewed preserve/
  deviation matrix. Do not add blanket Go parity for implementation-only
  behavior.
- Assert that state mutations remain visible through every completion/error
  result because `ExecutionState` is caller-owned, including cancellation and
  budget exhaustion.

### Minimum implementation

- Add the one-per-root-invocation fuel counter and cooperative cancellation
  token/check seam; share it with every try child.
- Enforce the frozen priority and document that pure fixture calls are only
  cancellable at the next boundary.
- Close any state/engine ownership gaps revealed by the reviewed contract tests
  without adding generic values or live host dependencies.

### Verification and review gate

- `cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked`
- `cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked -- -D warnings`
- Run focused Go tests only for any Go characterization files actually touched
  in Slice 0.
- Stop for root review before the final full-workspace gate.

## Final quality gate

After all slices are root-reviewed, run the complete planned gate without
enabling any Rust runtime selector:

```text
go test -count=1 ./...
go test -race -count=1 ./plugin/executable/sequence ./pkg/query_context
go vet ./...
go build ./...
CGO_ENABLED=0 go test -count=1 ./...
CGO_ENABLED=0 go build ./...

cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo build --manifest-path rust/Cargo.toml --workspace --release --locked

python3 ./.trellis/scripts/task.py validate .trellis/tasks/08-17-rust-phase3b-sequence-execution-foundation
git diff --check
```

The final report must include the reviewed product-contract/deviation matrix,
any targeted Go characterization evidence actually needed, Rust contract-test
results, fuel/cancellation safety evidence, no-ABI/no-live-wiring scope check,
and any Darwin/Linux platform caveat. No Linux+cgo gate is expected because
this task deliberately has no cgo or ABI surface.

## Commit/archive gate

- Do not stage or commit planning artifacts before explicit planning root
  approval and `task.py start`.
- After implementation and final root approval, stage only this task's
  production-approved crate/tests and task artifacts; never use `git add -A`.
- Run the normal Trellis finish/commit/archive flow only after the final
  implementation review. Do not start Phase 4 from this task; Phase4's pure
  foundation may be planned after this task is completed, while its live
  production wiring remains gated on Rust host ownership.
