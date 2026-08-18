# Rust Phase 3B sequence execution foundation

## Planning status

This task remains `planning`. Do not run `task.py start`, write implementation
code, add cgo/ABI/runtime exports, or wire Rust into the live Go request path
until `prd.md`, `design.md`, and `implement.md` are explicitly root-approved.
The 2026-08-18 migration policy now targets a pure Rust-native host; this task
must not add new hybrid Go/Rust runtime scaffolding.

## Background and confirmed evidence

- Phase 3A (`08-15-rust-phase3-query-execution-core`) is archived and
  root-approved. Its pure Rust `rust/dns-core` query foundation exposes typed
  `QueryHeader`/`QuestionInfo` parsing; its query C ABI remains a separate
  Phase 3A boundary.
- The live request path is still
  `Go EntryHandler -> Go query_context -> Go sequence -> Go upstream`.
  Rust is not selected by the default runtime.
- Go `query_context.Context` currently owns `query`, response state, marks,
  `fastFlags uint64`, and `kv map[uint32]any`
  (`pkg/query_context/context.go:50-77`). `RegKey()`/`StoreValue()` are
  intentionally generic (`pkg/query_context/context.go:283-303`,
  `pkg/query_context/kv.go:26-36`); a pure Rust foundation cannot claim to own
  arbitrary Go `any` values or Go pointers.
- The current Go response behavior is useful discovery evidence:
  `SetResponse` clears raw state, `SetRawResponse` clears decoded state,
  `R()` lazily decodes valid raw bytes, and malformed raw bytes are silently
  discarded (`pkg/query_context/context.go:175-226`,
  `pkg/query_context/context_raw_test.go:33-102`). The first four transitions
  align with the desired Rust state model; silent malformed-response discard
  is an internal Go quirk and is not automatically normative.
- The Go sequence dispatcher exposes routing/audit side effects that may be part
  of the product contract (`plugin/executable/sequence/chain.go:74-228`,
  `plugin/executable/sequence/sequence.go:94-191`): positive anonymous qname
  matches write the rule name; `switch6` + AAAA writes `BANAAAA`; `switch5` +
  SOA/PTR/HTTPS writes `BANSOA`/`BANPTR`/`BANHTTPS`; an existing value is not
  overwritten. The duplicated Go normal/fast implementation itself is not a
  Rust compatibility requirement.
- `MatchConfig.Reverse` wraps the Go matcher in `not(...)`
  (`plugin/executable/sequence/sequence.go:282-323`). The current lack of the
  positive `domain_set` side effect for reversed anonymous matchers originates
  from Go name-string recognition. Rust must not reproduce that string trick;
  Phase3B explicitly defines the product semantic that reversed membership does
  **not** claim the positive set/rule label.
- `RuleArgs` supports zero or more matchers and string or list executable
  forms (`plugin/executable/sequence/config.go:24-104`). The current builder
  treats no matcher as unconditional, no executable as a legal no-op/fall
  through, and multiple executables as an inline sub-sequence
  (`plugin/executable/sequence/sequence.go:214-279`). `try` accepts any
  `Executable`, not only a `Sequence` (`plugin/executable/sequence/built_in.go:61-109`).
- Existing Go sequence tests are discovery evidence for ordered match
  short-circuiting, fall-through, goto/return, jump/return, jump/accept,
  jump-at-end, and reject (`plugin/executable/sequence/sequence_test.go:72-199`).
  Missing cases do not require a blanket Go-parity expansion; only ambiguous
  user-facing semantics need targeted characterization before the Rust
  contract is frozen.
- The known routing/audit values are string-valued in the current Go audit
  projection (`coremain/audit.go:129-147`, `coremain/audit.go:349-380`):
  `domain_set`, `matched_group`, `final_sequence`, `final_upstream`,
  `final_upstream_targets`, `selected_upstream`, and `matched_rule_source`.

## Compatibility policy for this task

Phase3B targets the final Rust-native sequence engine, not an interchangeable
Go/Rust backend. Current Go code/tests are used only to discover existing
behavior. Each relevant behavior is classified before implementation as:

- `preserve`: part of the MosDNS product contract and required in Rust;
- `intentional Rust deviation`: a deliberate safety/correctness/architecture
  improvement that must be documented and tested;
- `implementation-only`: a Go internal detail with no Rust compatibility duty.

User-facing sequence/config semantics and final routing/audit meaning are
`preserve` by default. Go data structures, recursion, fast-path duplication,
error strings, naming tricks, fallback and cgo state are implementation-only by
default.

## Goal

Define and implement, in a later approved execution phase, an isolated pure
Rust sequence/execution foundation that owns an explicit typed execution state,
validated rule programs, matcher dispatch metadata, no-network built-ins, and
bounded control flow. This task does not transfer live query, sequence,
upstream, or server ownership and does not add a Go runtime adapter.

## Scope

### In scope

- A new isolated Rust sequence-core boundary, preferably
  `rust/sequence-core`, as a pure `rlib`/test crate with no cgo, no C ABI, and
  no `mosdns-runtime` export.
- A successor state model to the Phase 3A immutable query snapshot:
  owned typed query/question data, marks, `u64` fast flags, exact response
  transitions, and the known string routing/audit fields listed above.
- Program/rule construction and validation, including target resolution and
  the existing user-facing `RuleArgs` forms that remain part of the MosDNS
  configuration contract.
- Ordered matcher dispatch, typed matcher metadata, reverse matching, and the
  reviewed routing/audit side effects that are part of the sequence product
  contract.
- Pure Rust versions of only the sequence built-ins `accept`, `reject`,
  `return`, `goto`, `jump`, `exit`, and `try`, plus pure Rust fake
  matchers/executors used to test state mutation and error propagation.
- An explicit continuation stack, typed control-flow/error results, and a
  deterministic fuel/cancellation safety contract for cycles and abandoned
  execution.
- A product-contract/deviation matrix plus pure Rust contract tests. Targeted Go
  characterization is used only where the external semantic is ambiguous. Go
  production code, runtime selection, and live request wiring remain unchanged.

### Explicitly out of scope

- Porting all no-network plugins. In particular, do not add
  `rewrite`, `ttl`, `black_hole`, `ecs_handler`, or other plugin ownership in
  this task; later host-execution tasks may cover those plugins.
- Any cgo adapter, C header, ABI symbol/status/capability, `mosdns-runtime`
  export, backend selector, live `EntryHandler`/`query_context` bridge,
  sequence production wiring, upstream/network call, listener, WebUI, config,
  metrics, audit schema, cache, matcher production, or OpenWrt change.
- Claiming compatibility for arbitrary `query_context.RegKey()` values,
  arbitrary Go `any` values, Go pointers, plugin registry objects, or Go
  callbacks.
- Defining a Phase 4 transport/upstream cancellation ABI. Phase 4 foundation
  work may begin after Phase3B is root-reviewed and archived; Phase4
  production/live wiring still requires the later Rust host ownership gate.

## Requirements

### R1 — Correct task ownership metadata and planning gate

- `task.json` must identify `branch: rust`, `base_branch: rust`, and
  `scope: Phase 3B sequence/execution ownership foundation`.
- The task remains `planning` until the revised planning summary is explicitly
  approved. No `task.py start` is permitted during this planning revision.

### R2 — Freeze an explicit typed `ExecutionState`

The design must freeze a concrete state schema. It must not use a generic
`values` field or imply ownership of Go's `map[uint32]any`:

- `query`: an owned successor snapshot containing the Phase 3A typed query
  header/question (`id`, header fields needed by the snapshot, owned
  `qname_wire`, `qtype`, and `qclass`). It contains no Go pointer and does not
  modify or extend the Phase 3A C ABI.
- `marks`: a typed set of `u32` marks with deterministic snapshot ordering.
- `fast_flags`: a `u64` bitset.
- `response`: the exact three-state model in R3.
- `routing`: typed optional `String` fields for `domain_set`,
  `matched_group`, `final_sequence`, `final_upstream`,
  `final_upstream_targets`, `selected_upstream`, and
  `matched_rule_source`.

Unknown `RegKey()`/`any` values are explicitly unsupported by this foundation;
they must be rejected, omitted from the typed state, or deferred at the
boundary, never silently represented as a generic Rust value. This state is a
new pure Rust ownership model following the Phase 3A immutable query snapshot,
not an ABI revision or a request to mutate Phase 3A.

### R3 — Define deterministic Rust response-state semantics

`ResponseState` must distinguish `None`, `Raw(OwnedResponseWire)`, and
`Synthesized(SynthesizedResponse)` with explicit, typed transitions. A raw
upstream response remains a complete owned wire packet; reading it never
decodes it into a lossy `id`/`rcode` state:

- setting `None` clears any raw/synthesized response;
- setting `Synthesized` clears raw bytes;
- setting raw bytes replaces any synthesized response and retains all wire
  sections, records, TTLs, and EDNS data;
- inspecting a valid raw response validates/observes the complete wire and
  returns only the already-supported typed TTL observation while leaving `Raw`
  unchanged; Phase3B does not promise raw-response RCODE or EDNS extended
  RCODE parsing;
- inspecting malformed raw bytes returns a typed `MalformedRawResponse` (or
  equivalent) error and leaves the invalid wire owned as `Raw` until an
  explicit clear. This is an **intentional Rust deviation** from the current
  Go `R()` quirk; the later Rust host must define the external SERVFAIL/error
  mapping;
- synthesized local responses must represent the existing user-facing reject
  range `0..=0x0FFF`; do not narrow RCODE to `0..=15`. Full wire/EDNS/server
  packing remains a later host/server contract. Complete structured RR
  mutation and complete structured response inspection are deferred to later
  `dns-core`/host tasks; Phase3B does not add another DNS/RR/OPT parser.

### R4 — Matchers and dispatcher side effects are observable

- Rules run in declaration order; matchers inside a rule run in order and
  short-circuit on false or error. A failed matcher does not run that rule's
  executable.
- Matcher fixtures read an immutable state view and return at most one typed
  mutation result; the engine is the sole applier. The Rust seam must make
  mutation ordering and error propagation observable without Go callbacks.
- Rust uses one typed dispatcher path; it does not reproduce Go's duplicated
  normal/fast machinery or parse Go-generated matcher-name strings. Typed
  metadata covers the preserved audit semantics: positive anonymous qname
  rule-name assignment, `switch6`/AAAA → `BANAAAA`, and
  `switch5`/SOA/PTR/HTTPS → `BANSOA`/`BANPTR`/`BANHTTPS`.
- These routing/audit writes remain write-once for this path: an existing
  `domain_set` is not overwritten.
- `reverse` changes the boolean meaning only. A reversed membership match does
  not claim the positive qname/set/switch routing label; this is an explicit
  Rust product semantic, not a reproduction of Go's `not(...)` naming quirk.

### R5 — Match actual `RuleArgs` and target semantics

- Zero matchers mean unconditional execution.
- Missing executable is a legal no-op/fall-through rule, not malformed input.
- `exec: []` and multi-exec lists normalize to an inline child sequence;
  an empty list is a no-op and a non-empty list is represented at runtime by
  `ValidatedExecutable::Inline { target: SequenceId }`.
- `try` may target any user-addressable validated sequence or fixture
  executable, but never a synthetic inline sequence.
- Repeated matcher kinds and repeated executable kinds are valid and preserve
  declaration order. Duplicate names inside the sequence catalog or inside the
  fixture catalog are rejected because symbolic resolution would be ambiguous;
  the two typed namespaces may use the same spelling independently.
- Unknown executable/matcher types, missing targets, invalid target kinds, and
  other malformed program definitions fail deterministically during
  construction/validation, before execution; no panic is used for control
  flow.

### R6 — Freeze no-network built-ins and control flow

The pure Rust foundation must cover `accept`, `reject`, `return`, `goto`,
`jump`, `exit`, and `try`:

- `accept` and `reject` are terminal for the current sequence.
- `reject` defaults to REFUSED and preserves explicit RCODE values through
  `0x0FFF`.
- `goto` replaces the current continuation and does not return to its caller.
- `jump` pushes an explicit return continuation; `return` resumes it, and a
  top-level `return` completes without a parent continuation.
- Falling off a jumped target returns to its continuation; falling off a goto
  target completes that execution.
- An inline child is a closed execution scope: fall-through, `return`,
  `accept`, and `reject` end only that child and resume the outer sequence's
  next rule; `jump` returns within the child; `goto` replaces only the child's
  local continuation and resumes the outer next rule after its target
  completes. `exit` propagates out of inline unless a nested `try` catches it.
  An inline child may contain a `try` whose target is only a user sequence or
  fixture: child normal completion or caught `Exit` continues with the next
  inline executable, while ordinary error, cancellation, or budget exhaustion
  propagates out. The synthetic inline sequence itself is not a direct
  `try`/`goto`/`jump` target.
- `exit` produces a typed sequence-exit signal. `try` catches only that signal,
  resumes the caller's next instruction, and propagates ordinary matcher,
  executor, cancellation, and budget errors.

### R7 — Deterministic bounded termination and cancellation

The engine must not copy Go recursion without protection. It must use an
explicit execution fuel/step budget (or an equivalent deterministic mechanism)
and a cooperative cancellation seam:

- cyclic `goto`/`jump` programs terminate with `BudgetExceeded` rather than
  hanging or overflowing the stack;
- cancellation is observable as `Cancelled` at a defined execution boundary;
- one root invocation owns the shared fuel, cancellation state, and
  `ExecutionState`; `try` detaches only its continuation stack and never
  resets fuel or cancellation for a child;
- the public root execution API receives caller-owned `&mut ExecutionState`
  and `&mut ExecutionControl` and returns only completion/error status; state
  remains observable after success, exit, ordinary error, cancellation, or
  budget exhaustion;
- the priority is frozen as `Cancelled > BudgetExceeded > ordinary
  matcher/executor error > Exit`; only `try` converts `Exit` into continuation;
- this is an intentional safety contract that may be stricter than Go for
  non-terminating programs. Infinite-loop parity with Go is not claimed.

### R8 — Extract and classify the MosDNS sequence contract

Before Rust implementation, build a compact compatibility/deviation matrix from
existing docs/config semantics plus targeted Go source/tests. Do **not** create
Go characterization tests merely to clone internal behavior. The matrix must
classify at least:

- `preserve`: declaration order, false/error short-circuit, unconditional and
  no-op rules, multi-exec declaration order and inline scope,
  `goto`/`jump`/`return`,
  `accept`/`reject`/`exit`/`try`, exact configured reject RCODE range, repeated
  matcher/executable kinds, target validation, matcher/executor error
  propagation, and reviewed routing/audit labels;
- `intentional Rust deviation`: bounded fuel/cancellation for cycles and typed
  malformed-raw-response errors instead of silent discard;
- `implementation-only`: `ChainWalker` recursion, Go normal/fast duplication,
  `not(...)` name-prefix recognition, Go `map[uint32]any`, cgo/selector/fallback
  machinery and other internal layout choices.

If an externally visible behavior remains ambiguous after source/document
review, add the smallest Go characterization needed to classify it. Rust red
tests are written against the **reviewed Rust contract**, not against blanket
Go parity.

## TDD seams and compatibility boundaries

- Public Rust interfaces under test: typed `ExecutionState`, complete-wire
  response inspection/synthesized response transitions, matcher/executor
  traits, typed matcher metadata, `ProgramSpec` → `ValidatedProgram`
  normalization including `Inline(SequenceId)`, explicit continuation
  execution, fuel/cancellation, borrowed-state `ExecutionCompletion`/
  `ExecutionError`, and canonical observable snapshots.
- Mock boundary: pure Rust fake matchers and executors only. No cgo, Go
  callback, plugin registry, `EntryHandler`, listener, upstream, or server
  dependency.
- Every behavior slice has a failing Rust test before its minimum
  implementation. Any Go characterization test is discovery evidence only and
  never runtime wiring or an automatic parity requirement.

## Acceptance criteria

- [ ] `task.json` has `branch=rust`, `base_branch=rust`, and the Phase3B scope;
      task status remains `planning` until explicit root approval.
- [ ] Reviewed `design.md` freezes `rust/sequence-core`, the typed state
      schema, complete-wire response ownership and inspection errors,
      `Inline(SequenceId)` scope semantics, program/target validation, matcher
      metadata, explicit continuation stack, borrowed-state typed
      control-flow/error API,
      fuel/cancellation priority, Phase3A reuse, and no live integration.
- [ ] Reviewed `implement.md` orders product-contract/deviation classification
      → targeted characterization only where needed → Rust red tests → minimum
      implementation → quality gates, with rollback points and root-review
      stops.
- [ ] The Phase3B contract matrix classifies required preserve/deviation/
      implementation-only behaviors; any Go characterization added is narrowly
      tied to an unresolved product-contract question rather than blanket
      parity.
- [ ] Pure Rust tests cover typed state ownership, complete-wire response
      inspection/synthesized transitions, all listed built-ins, inline scope,
      matcher ordering/short-circuit,
      reviewed dispatcher side effects, reverse semantics, repeated kinds,
      catalog-name validation, explicit continuation behavior, cancellation,
      budget termination, and no panic paths.
- [ ] Rust formatting, tests, clippy, and release build pass; existing Go
      default, race, vet, build, and `CGO_ENABLED=0` gates remain green.
- [ ] No cgo/ABI symbols, runtime exports, Go adapter/selector,
      `EntryHandler`, listener, sequence production wiring, upstream, cache,
      matcher production, WebUI, coremain, OpenWrt, or configuration behavior
      changes are made.
- [ ] Rust remains experimental and unused by the default runtime; no
      `MOSDNS_*_BACKEND` switch or fallback policy is introduced.
- [ ] Phase 4 pure UDP/TCP foundation is unblocked by Phase3B completion and
      archive; production use remains blocked until the later Rust-native host
      and hybrid-retirement gates are complete.

## Open questions

There are no blocking product or scope questions for this planning revision.
The typed schema, response semantics, dispatcher metadata, safety priority,
crate boundary, and Phase4 dependency above are the decisions to be reviewed
by root before `task.py start`.

## Notes

- Go is a behavior-discovery reference, not the normative sequence
  specification. This task creates no Go fallback or hybrid runtime boundary.
- “No-network executables” is deliberately limited to the listed sequence
  built-ins and pure Rust fixtures. It does not claim that all current MosDNS
  no-network plugins have Rust ownership.
- The Rust state is a successor pure Rust ownership model for the immutable
  Phase 3A query snapshot. It does not revise the Phase 3A C ABI.
