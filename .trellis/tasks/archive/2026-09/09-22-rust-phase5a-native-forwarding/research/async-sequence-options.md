# Async sequence boundary options

## Decision context

`rust/sequence-core/src/engine.rs` currently exposes a synchronous
`execute(program, entry, state, control)` loop. The loop already stores
explicit `Scope`, `Frame`, `pending_fixture`, and continuation state, while
`rust/upstream-core/src/lib.rs` exposes async `Upstream::exchange` and
caller-owned cancellation/deadline controls. Phase 5A needs one sequence
semantic engine that can pause at the forward boundary and resume after the
upstream future completes.

The task must keep `sequence-core` independent of Tokio and `upstream-core`.
Existing matcher/executor fixtures are synchronous and include non-`Send`
test objects, so a broad trait-bound migration is not a safe prerequisite for
the W1 host.

## Options considered

### A. Make `Executor` an async trait

Change the core trait to return a boxed future and let the forward executor
await upstream I/O.

Rejected for this task. It couples a pure sequence crate to an async calling
convention, introduces lifetime/borrow choices at every fixture, risks
requiring `Send` futures for existing `Rc` fixtures, and still leaves the
engine's frame/continuation state hidden inside one future. It also makes the
synchronous API a compatibility special case instead of the canonical path.

### B. Implement a second host-side sequence interpreter

Parse the Phase 5A one-rule sequence directly in the host and call upstream
without changing `sequence-core`.

Rejected. It would silently bypass fuel, cancellation, frame unwinding,
control-flow semantics, and future sequence parity. The host would become a
second interpreter that can drift from the existing engine.

### C. Add a canonical resumable machine and keep sync execution as an adapter

Refactor the existing engine loop into a stateful machine that advances
synchronously until it reaches completion/error or a typed external dispatch.
The machine records one pending executable identity and owns all frames,
continuations, fuel, cancellation, and `ExecutionState`. The host awaits the
external operation outside `sequence-core`, then resumes the same machine with
an owned result.

Chosen. It preserves one semantic engine, makes the async suspension point
explicit, and keeps `sequence-core` free of Tokio/upstream dependencies. The
current sync API drives the same machine to completion using the existing
synchronous fixture catalog.

### D. Make the whole engine a generator/coroutine abstraction

Expose an unstable or custom generator-like API that yields external work.

Rejected for the first host. It adds a language/runtime abstraction without
improving the ownership contract over a small explicit state machine, and it
would complicate stable Rust/MSRV support and diagnostics.

## Chosen machine contract

The exact public names are implementation work, but the contract is fixed:

```text
new(program, entry, state, control)
  -> machine

step(machine)
  -> Continue / ExternalDispatch(id, owned request metadata)
   | Completed(outcome) / ExecutionError

resume(machine, matching id, owned response or typed error)
  -> step result
```

The machine must reject a wrong executable ID, a duplicate resume, and a
resume after terminal completion. `step()` checks cancellation/fuel at the
same dispatch boundaries as the current engine. It must preserve all state
mutations and continuation frames across a pending dispatch.

For the W1 graph, the external dispatch is the named forward executable. The
host converts its owned raw query into `ExchangeRequest` and resumes with the
owned upstream wire or a typed failure. A future executable can use the same
seam, but no additional Phase 5A executable is admitted.

The validated program may therefore gain an explicit external executable
catalog entry alongside the existing pure fixture entries. It must use the
existing `ExecutableId` identity and engine dispatch path; it must not be
implemented as a host-side one-rule interpreter or as an `upstream-core`
dependency in `sequence-core`.

The sync adapter must exercise the same machine. For existing pure fixtures it
resolves the dispatch synchronously; for a pending external/native forward,
the adapter reports the typed boundary error rather than inventing a hidden
runtime. Tests must prove that ordinary fixture programs retain their current
outcomes and state snapshots.

## Ownership and runtime decision

No borrowed `ExecutionState`, query slice, or trait object may be held across
the host's await. The machine may use the existing crate-internal borrowing
shape while `step()` runs, but the returned dispatch contains only stable IDs
and owned/copyable request metadata. The host owns the raw query and upstream
future, then calls `resume()` after the await.

The initial host uses a current-thread Tokio runtime and `LocalSet` if needed
by the existing non-`Send` fixture model. This does not add Tokio to
`sequence-core`; it only chooses a safe host executor for the current API.
Slice 1 must include compile-time/dependency checks proving the crate remains
pure and tests proving cancellation is observed at the same boundaries.

## Required Slice 1 tests

- machine and sync adapter agree for unconditional `Continue`, `Return`,
  `Accept`, `Reject`, `Exit`, nested `try`, and ordinary errors;
- frame/continuation and state mutations survive an external dispatch;
- cancellation and fuel behavior is identical before and after suspension;
- matching response resumes exactly once;
- wrong executable, duplicate resume, post-finish resume, and missing entry
  fail deterministically;
- the crate dependency tree has no Tokio/upstream/native-host edge;
- focused sequence tests and existing sequence tests pass without a host or
  network socket.
