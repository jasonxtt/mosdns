# Phase 3B Slice 0 contract/deviation matrix

This is Slice 0 evidence only. The earlier execution incorrectly crossed the
required Slice 0, Slice 1, Slice 2, Slice 3, and Slice 4 root-review stops in
one pass. The existing `rust/sequence-core` is therefore unreviewed WIP, not
review-approved implementation. It is intentionally left untouched here.

The current task remains `in_progress` on branch `rust`; Phase 4 remains
`planning`/deferred. This artifact and the test-only characterization are the
only Phase 3B changes in this recovery pass.

Sources reviewed: this task's `prd.md`, `design.md`, and `implement.md`,
`docs/ai/rust-rewrite-plan.md`, `.trellis/spec/backend/rust-migration.md`,
`plugin/executable/sequence/{config.go,sequence.go,chain.go,built_in.go}`,
`plugin/executable/sequence/sequence_test.go`, and the related
`query_context` behavior.

## Preserve

These are user-observable sequence/config or routing semantics for the future
Rust-native implementation:

| Behavior | Frozen classification |
| --- | --- |
| Rule declaration order | Preserve. |
| Matcher declaration order | Preserve. |
| Matcher false/error short-circuit | Preserve: false skips the rule executable and later matchers; error propagates. |
| Zero matcher | Preserve as unconditional. |
| Missing executable | Preserve as legal fall-through. |
| `exec: []` | Preserve as legal no-op. |
| Multi-exec declaration order | Preserve. |
| Inline child scope | Preserve as a closed child scope whose completion resumes the outer next rule. |
| `goto` in an ordinary/root sequence | Preserve: it replaces the current scope's continuation; caller rules after `goto` do not execute; target fall-through/completion ends the current execution/scope and does not return to the original `goto` caller's next rule. |
| `goto` in a synthetic inline scope | Preserve: it replaces only the inline local continuation; later inline executables are skipped; target completion ends the inline scope, after which the outer sequence's next rule resumes. |
| `jump` | Preserve target return/fall-through to the caller's next continuation. |
| `return` | Preserve current-scope return semantics. |
| `accept` | Preserve current-scope completion. |
| `reject` | Preserve response-setting and current-scope completion. |
| `exit` | Preserve propagation unless caught by `try`. |
| `try` | Preserve catching `Exit` only and continuing after the try. |
| Configured reject RCODE | Preserve every configured value in `0..=0x0FFF`; invalid larger values remain construction errors. |
| Repeated matcher kinds | Preserve; declaration order remains meaningful. |
| Repeated executable kinds | Preserve; declaration order remains meaningful. |
| Sequence/fixture target semantics | Preserve user sequence targets for `goto`/`jump`/`try`, and addressable fixture executable targets for `try`; synthetic inline is not a direct symbolic target. |
| Positive routing labels | Preserve positive qname labels and `switch5`/`switch6` labels (`BANSOA`, `BANPTR`, `BANHTTPS`, `BANAAAA`) where their qtype conditions match. |
| Write-once `domain_set` | Preserve; an existing value is not overwritten. |
| Reversed membership | Preserve the product semantic that a reversed membership match does not claim a positive routing label. |
| Ordinary matcher/executor errors | Preserve propagation and state changes already completed before the error. |

The synthetic inline sequence is a normalization/runtime detail only. This
matrix does not create a symbolic inline catalog or make the synthetic inline
sequence a direct `try`, `goto`, or `jump` target.

## Intentional Rust deviation

Only the two deviations already approved in the Phase 3B planning documents
are recorded:

| Behavior | Frozen Rust deviation |
| --- | --- |
| Malformed raw response | Return a typed error while retaining the owned `Raw` wire; do not copy Go's silent discard during `R()`. |
| Cyclic control flow | Use shared bounded fuel and cooperative cancellation, returning `BudgetExceeded`/`Cancelled`; do not copy Go's unbounded loop or recursive-stack risk. |

No Go characterization is added for `Cancelled` or `BudgetExceeded` because
those are Rust-native deviations.

## Implementation-only

The following Go mechanisms are evidence about the current implementation, not
Rust product-contract requirements:

- Go `ChainWalker`.
- Go recursion.
- Normal/fast duplicated dispatcher.
- `not(...)` matcher name recognition.
- `map[uint32]any`.
- Go pointer/plugin-object ownership.
- cgo ABI.
- `MOSDNS_*_BACKEND` selectors.
- Go mirror/fallback.
- Generation pairing.
- FFI handle/runtime registry.

## Inline characterization evidence

The test-only table-driven characterization in
`plugin/executable/sequence/slice0_inline_characterization_test.go` exercises
the existing Go `NewSequence`/plugin-map seam. Every case passed with the
following observable log and terminal result:

| Case | Actual result |
| --- | --- |
| Multi-exec order then outer rule | Passed: `exec1 -> exec2 -> exec3 -> outer`. |
| Inline `return` | Passed: `exec1 -> outer`; inline tail skipped, outer continued. |
| Inline `accept` | Passed: `exec1 -> outer`; inline tail skipped, outer continued. |
| Inline `reject` | Passed: `exec1 -> outer`; response set to RCODE `5` and inline tail skipped. |
| Inline `jump` target fall-through | Passed: `exec1 -> target-work -> exec3 -> outer`. |
| Inline `jump` target `return` | Passed: `exec1 -> target-work -> exec3 -> outer`. |
| Inline `goto` target | Passed: `exec1 -> target-work -> outer`; inline tail skipped. |
| Uncaught inline `exit` | Passed: `exec1`, `ErrExit` propagated, and outer rule did not run. |
| Inline `try` with normal sequence target | Passed: `exec1 -> target-work -> exec3 -> outer`. |
| Inline `try` with sequence target `Exit` | Passed: `exec1 -> exec3 -> outer`; target `Exit` was caught. |
| Inline `try` with fixture target `Exit` | Passed: `exec1 -> fixture -> exec3 -> outer`; fixture `Exit` was caught. |
| Ordinary inline executor error | Passed: `exec1 -> inline-error`; error propagated and outer rule did not run. |
| Inline `try` with fixture ordinary error | Passed: `exec1 -> fixture`; the ordinary sentinel error propagated and neither `exec3` nor `outer` ran. |

These observations match the approved Slice 0 contract. No Go behavior
discrepancy was found. The Go production sequence implementation was not
modified.
