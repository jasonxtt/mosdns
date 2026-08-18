# Phase 3B sequence execution foundation design

## 1. Ownership and crate boundary

The implementation target is a new `rust/sequence-core` crate. It is a pure
Rust library and test boundary, not a second runtime or an integration layer.
It may depend on `mosdns-dns-core` for the already-frozen typed query header and
question atoms, but it must not depend on Go, cgo, the plugin registry,
`EntryHandler`, listeners, upstreams, or `mosdns-runtime` exports.

The dependency shape is:

```text
rust/dns-core  ─────►  rust/sequence-core  ─────►  pure Rust contract tests
      │                         │
      └── Phase 3A query ABI    └── no C ABI, no live Go wiring
```

`sequence-core` owns only an explicitly created Rust program and
`ExecutionState`. It does not create a handle visible to Go and does not
change the Phase 3A query ABI, runtime capability bits, status values, or
handle namespace. A future Rust host may consume the crate in a separate
approved task.

## 2. Typed execution state

The state is deliberately closed and typed. There is no `HashMap<u32, Any>`
equivalent and no opaque value escape hatch.

```rust
pub struct ExecutionState {
    pub query: QueryState,
    pub marks: BTreeSet<u32>,
    pub fast_flags: u64,
    pub response: ResponseState,
    pub routing: RoutingState,
}

pub struct QueryState {
    pub header: mosdns_dns_core::QueryHeader,
    pub question: mosdns_dns_core::QuestionInfo,
}

pub struct RoutingState {
    pub domain_set: Option<String>,
    pub matched_group: Option<String>,
    pub final_sequence: Option<String>,
    pub final_upstream: Option<String>,
    pub final_upstream_targets: Option<String>,
    pub selected_upstream: Option<String>,
    pub matched_rule_source: Option<String>,
}
```

`QueryState` owns copied query/question bytes and is the successor to the
Phase 3A immutable query snapshot, not an extension of its C ABI. `BTreeSet`
gives mark snapshots a stable order even though Go's mark map has no order.
The routing fields are the known string projections used by the Go audit
consumer. Trace IDs, server metadata, arbitrary plugin objects, and unknown
`RegKey()` values remain host-owned or out of scope.

The observable test snapshot is a canonical value containing the query
question, sorted marks, `fast_flags`, response state, and routing fields. Tests
compare this value rather than private program pointers or Go implementation
details.

## 3. Response ownership and inspection

The final Rust-native state does not decode an upstream response into a
lossy `id`/`rcode` object. An upstream response remains an owned complete DNS
wire packet, including answer, authority, additional, TTL, CNAME, A/AAAA, and
EDNS sections. A locally synthesized response is a separate state form:

```rust
pub struct OwnedResponseWire(pub Vec<u8>);

pub enum ResponseState {
    None,
    Raw(OwnedResponseWire),
    Synthesized(SynthesizedResponse),
}

pub struct SynthesizedResponse {
    pub rcode: u16, // 0..=0x0fff
}

pub struct ResponseInspection {
    pub ttl: mosdns_dns_core::TtlInfo,
}
```

`SynthesizedResponse` is sufficient for Phase3B local actions such as
`reject`; the query header supplies the request identity when a later host
packs it. It is not a replacement for an upstream wire response. If a future
task needs structured RR mutation, that task must add a complete owned message
representation to `dns-core`; Phase3B does not discard wire sections to fake
one.

The response seam validates/inspects complete raw wire without consuming it:

```rust
pub trait ResponseInspector {
    fn inspect(&self, wire: &[u8])
        -> Result<ResponseInspection, ResponseError>;
}
```

The inspector reuses `mosdns-dns-core::validate_response`/
`observe_response_ttl` and related wire helpers where applicable. Its only
Phase3B inspection value is the already-supported TTL observation; it does not
parse or promise raw-response RCODE, including EDNS extended RCODE. A
successful inspection returns that summary while `ResponseState` remains
`Raw` with the exact bytes. A malformed inspection returns a typed error and
leaves the invalid bytes owned as `Raw` until the caller explicitly clears
them with `set_response(None)`; there is no silent absence transition or hidden
data loss.

| Operation | Previous state | New state | Required result |
|---|---|---|---|
| `set_response(None)` | any | `None` | clear previous response state |
| `set_response(synthesized)` | any | `Synthesized` | clear raw bytes |
| `set_raw_response(bytes)` | any | `Raw` | retain the exact complete wire; replace synthesized state |
| `inspect_response`, valid raw | `Raw` | `Raw` | return complete-wire inspection; do not consume bytes |
| `inspect_response`, malformed raw | `Raw` | `Raw` | return typed `MalformedRawResponse`; do not silently clear bytes |
| `inspect_response`, `None`/`Synthesized` | unchanged | unchanged | no raw inspection |

The typed malformed-raw error is an intentional Rust deviation from Go's
silent `R()` discard. The later Rust host owns its externally visible mapping
(for example SERVFAIL). Only synthesized-response validation must accept every
configured reject RCODE in `0..=0x0FFF`; the DNS wire header's four-bit
representation is not a reason to narrow synthesized execution state. A
future executable/host task may add complete structured response inspection
through `dns-core`; Phase3B does not add a DNS/RR/OPT parser.

## 4. Program input, normalization, and validation

Construction has two explicit layers. `ProgramSpec` is the unvalidated input
model and can represent every RuleArgs shape covered by this task.
`ValidatedProgram` is the only model accepted by the run engine. This prevents
the design from claiming to normalize a form that the public construction API
cannot carry.

The unvalidated layer uses symbolic references so definitions can be assembled
before stable runtime IDs exist:

```rust
pub struct ProgramSpec {
    pub sequences: Vec<SequenceSpec>,
    pub fixtures: Vec<FixtureSpec>,
}

pub struct SequenceSpec {
    pub name: String,
    pub rules: Vec<RuleSpec>,
}

pub struct RuleSpec {
    pub matchers: Vec<MatcherSpecInput>, // zero means unconditional
    pub exec: Option<Vec<ExecutableSpec>>, // None and Some(vec![]) are distinct inputs
}

pub enum ExecutableSpec {
    Accept,
    Reject { rcode: u16 },
    Return,
    Goto { target: SequenceRef },
    Jump { target: SequenceRef },
    Exit,
    Try { target: ExecutableTargetSpec },
    Fixture { target: FixtureRef },
}

pub enum ExecutableTargetSpec {
    Sequence(SequenceRef),
    Fixture(FixtureRef),
}

pub struct SequenceRef { pub name: String }
pub struct FixtureRef { pub name: String }
pub struct FixtureSpec { pub name: String, pub executable: FixtureFactory }
```

`exec: None` is a missing executable, `Some(vec![])` is an explicit empty
list, one element is the one-exec form, and multiple elements preserve the
Go declaration-order list form. `MatcherSpecInput` carries a known matcher
fixture/typed matcher definition or an input kind that validation can reject;
it is not an opaque Go callback. The exact fixture factory representation is
an implementation seam, but every fixture has a named catalog entry before
validation.

Normalization and validation produce the runtime model:

```rust
pub struct ValidatedProgram {
    pub sequences: Vec<ValidatedSequence>,
    pub fixtures: BTreeMap<ExecutableId, ValidatedFixture>,
}

pub struct ValidatedSequence {
    pub id: SequenceId,
    pub name: String,
    pub rules: Vec<ValidatedRule>,
}

pub struct ValidatedRule {
    pub matchers: Vec<MatcherSpec>,
    pub executable: Option<ValidatedExecutable>,
}

pub enum ValidatedExecutable {
    Accept,
    Reject { rcode: u16 },
    Return,
    Goto { target: SequenceId },
    Jump { target: SequenceId },
    Exit,
    Try { target: ExecutableTarget },
    Fixture { target: ExecutableId },
    Inline { target: SequenceId },
}

pub enum ExecutableTarget {
    Sequence(SequenceId),
    Fixture(ExecutableId),
}
```

`ValidatedProgram` is the runtime `Program`; the separate name makes the
pre-run boundary explicit. Normalization and validation must, before any
`ExecutionState` is passed to the engine:

- assign stable `SequenceId`s to user sequences in declaration order, then to
  synthetic inline sequences in deterministic source/rule/executable order;
  only user sequences enter the symbolic sequence-name catalog;
- assign stable `ExecutableId`s to plain fixture entries and build the fixture
  catalog;
- normalize one executable directly and compile a multi-exec list into an
  inline child sequence in declaration order, represented at runtime as
  `ValidatedExecutable::Inline { target }`;
- normalize both missing executable and an empty exec list to a legal no-op;
- resolve `goto`/`jump` only to `SequenceId`s and resolve `try` to the unified
  `ExecutableTarget`, including either a sequence or a plain fixture;
- allow repeated matcher kinds, repeated executable kinds, and repeated calls
  to the same fixture target while preserving declaration order;
- reject duplicate names within the sequence catalog or within the fixture
  catalog, missing targets, wrong target kinds, unknown matcher/executable
  kinds, invalid RCODE values outside `0..=0x0FFF`, and other malformed
  definitions before execution. Sequence and fixture namespaces are typed and
  may reuse the same spelling across namespaces.

Cycles are valid program graphs at construction time. They are protected by
the shared execution fuel contract rather than rejected as a graph-shape
error; this keeps validation deterministic while allowing a deliberate safety
test for cyclic control flow. No state mutation is permitted before this
normalization/validation step succeeds.

### Inline executable scope

An `Inline` target is a synthetic sequence created by normalization. Its
single-executable child rules preserve the source list order and execute as
one ordinary executable invocation:

```text
outer sequence
  -> Inline(SequenceId)
       -> exec 1
       -> exec 2
       -> ...
       -> inline scope completes
  -> outer next rule
```

The inline scope is explicit and closed. Normal fall-through, `return`,
`accept`, and `reject` end the current inline scope and return normal
completion to the outer sequence, which then proceeds with its next rule.
`jump` pushes and later resumes a continuation inside the inline scope;
when that scope falls off, control returns to the outer next rule. `goto`
replaces the inline scope's local continuation, runs its target to completion,
and then returns normal completion to the outer next rule; it cannot jump into
or permanently discard the outer continuation. An `exit` propagates out of
the inline scope as the typed exit signal unless a nested `try` catches it.

An inline child may itself contain a `Try` executable. That `Try` can target
only a user-addressable sequence or a fixture executable through
`ExecutableTarget`; normal child completion continues with the next inline
item, `Exit` is swallowed and also continues with the next inline item, and an
ordinary error, `Cancelled`, or `BudgetExceeded` propagates out of the inline
scope. The synthetic inline sequence is not in the user sequence-name catalog
and is not a direct `try`/`goto`/`jump` target. Once the inline scope completes,
control resumes the outer next rule. These are preserved product semantics and
are locked by the Slice 0 characterization cases before implementation. The
runtime must not infer them from a generic block or from the old Go `isInline`
field.

## 5. Matcher dispatch and typed metadata

The matcher seam is pure and exposes state changes without string parsing:

```rust
pub trait Matcher {
    fn evaluate(&self, state: &ExecutionState)
        -> Result<MatchOutcome, MatcherError>;
}

pub struct MatchOutcome {
    pub matched: bool,
    pub mutation: Option<StateMutation>,
}

pub struct MatcherSpec {
    pub matcher: Box<dyn Matcher>,
    pub reverse: bool,
    pub dispatch_metadata: DispatchMetadata,
}

pub enum DispatchMetadata {
    None,
    AnonymousQname { rule_name: String },
    Switch6,
    Switch5,
}
```

This is the only matcher mutation channel. A matcher receives an immutable
state view and cannot directly modify `ExecutionState`; all matcher-produced
changes are returned as the typed `StateMutation` and applied by the engine.
The dispatch order is fixed:

1. the matcher reads the current state and returns a boolean plus an optional
   typed mutation;
2. the engine applies that matcher mutation;
3. `reverse` flips only the boolean and never rolls back the mutation;
4. if the effective match is true, the dispatcher applies typed metadata
   side effects;
5. if the effective match is false, the rule stops and its executable is
   skipped; a matcher error immediately propagates and no later matcher or
   executable runs.

Mutations from already completed matchers remain observable if a later matcher
errors; a matcher that itself returns an error has no second, implicit direct-
mutation channel. Dispatcher metadata therefore has a deterministic position
after the effective match decision. A reversed membership matcher does not
receive the positive qname/set/switch routing label: this is an explicit Rust
product semantic, not a reproduction of Go's `not(...)` name-prefix behavior.

For positive metadata, the dispatcher applies:

- anonymous qname → the captured rule name;
- `Switch6` with question type AAAA → `BANAAAA`;
- `Switch5` with SOA/PTR/HTTPS → `BANSOA`/`BANPTR`/`BANHTTPS`.

It performs these writes only when `routing.domain_set` is unset. Rust has one
typed dispatcher path; Go's duplicated normal/fast machinery is
implementation-only and is not reproduced.

Matcher errors stop the rule and sequence immediately. A false matcher stops
the current rule's matcher list and skips its executable; an empty matcher
list proceeds directly to the executable/no-op behavior.

## 6. Executable and control-flow model

The validated executable model above is the typed variant produced by the
`ProgramSpec -> ValidatedProgram` pass. It has no unresolved names. At runtime
a plain fixture is looked up in the validated fixture catalog; a
`try` sequence target dispatches that sequence as an executable target, while
a `try` fixture target dispatches the fixture. The distinction is resolved
before execution, so a missing or wrong-kind target cannot partially mutate
state.

The run engine uses an explicit `Vec<Continuation>` rather than recursive Go
walkers:

```rust
struct Frame {
    sequence: SequenceId,
    pc: usize,
}

struct Continuation {
    return_to: Frame,
}
```

The active frame points at the next rule. A `jump` pushes its caller's next
frame and starts the target. A `goto` discards the current scope's continuation
stack and starts the target. `return` pops one continuation; with no
continuation it completes the current scope normally. Falling off a jumped
sequence resumes its continuation; falling off a goto target completes that
scope. `accept` and `reject` terminate the current scope normally: at the root
they complete the root invocation, while inside `Inline`/`try` they return
normal completion to the caller and the caller proceeds according to the
inline-scope rules above. `exit` is a control signal, not an ordinary success
or error.

`try` runs its target as a detached child execution with no inherited return
continuation. This preserves the sequence-language meaning without inheriting
Go's recursive walker implementation. The detachment applies only to the
continuation stack: the child receives the same mutable `ExecutionState` and
the same root invocation `ExecutionControl`. It resumes the parent after a
normal child completion or a caught `Exit`; it propagates every other typed
error.

## 7. Errors, fuel, and cancellation

Construction and execution use separate typed results. `ExecutionState` is
always caller-owned. Every root invocation also receives one shared control
object:

```rust
pub struct ExecutionControl {
    pub remaining_fuel: u64,
    pub cancellation: CancellationState,
}

pub fn execute(
    program: &ValidatedProgram,
    entry: SequenceId,
    state: &mut ExecutionState,
    control: &mut ExecutionControl,
) -> Result<ExecutionCompletion, ExecutionError>;

pub enum ExecutionCompletion {
    Completed,
    Exited,
}

pub enum ExecutionError {
    Matcher(MatcherError),
    Executor(ExecutorError),
    Cancelled,
    BudgetExceeded,
}
```

`ProgramError` covers validation and never reaches the run loop. The public
`execute` function creates no state ownership transfer; it initializes the
root continuation scope and calls an internal scope runner with the same
`&mut ExecutionState` and `&mut ExecutionControl` references. Every ordinary
matcher/executable dispatch, `goto`, `jump`, and `try` child uses those same
references; one fuel unit is consumed per matcher or executable dispatch at
the defined boundary, and cancellation is checked at that same boundary.
Entering a `try` target never creates a fresh default fuel budget or
cancellation state. Nested `try` recursion therefore cannot reset the budget:
`try A -> try B -> try A` eventually returns `BudgetExceeded`.

The internal scope runner may create a fresh empty continuation vector for a
detached `try` child, but it never takes ownership of state or control. Thus
success, `Exited`, ordinary matcher/executor error, `Cancelled`, and
`BudgetExceeded` all return while the caller's current `ExecutionState`
remains observable. In particular, a matcher mutation completed before a
later matcher error is still present in the borrowed state. Cancellation
remains observable inside nested `try`, and `BudgetExceeded`/`Cancelled` are
never converted to `Exit` success. The outcome priority is fixed and tested as:

```text
Cancelled > BudgetExceeded > ordinary matcher/executor error > Exit
```

`try` may convert only the final `Exit` signal into continuation. It never
swallows cancellation, budget exhaustion, or an ordinary error. If multiple
terminal conditions are observed at one boundary, the shared control resolves
them in the stated priority order before returning to the parent. The engine
does not promise to interrupt an arbitrary fixture call in the middle of a
single pure dispatch; cancellation is guaranteed at the next defined
boundary. A finite default/test fuel budget ensures cyclic goto/jump graphs
and nested `try` cycles return `BudgetExceeded` instead of hanging or
overflowing the stack. This is an intentional Rust deviation for safety and
determinism: non-terminating programs receive a bounded error instead of
inheriting Go's ability to hang or overflow.

## 8. Phase 3A reuse and compatibility boundary

`sequence-core` consumes the public typed `QueryHeader` and `QuestionInfo`
values from `mosdns-dns-core` and copies them into `QueryState`. It does not
call the Phase 3A ABI, add a symbol, alter runtime capability bits, or add a
Go selector. The later Rust host may decide how to construct this state; that
ownership transfer is not part of Phase3B.

The Go sequence and Go query context remain unchanged runtime owners while this
branch is incomplete. Rust tests construct `ExecutionState` directly. Go tests
or source inspection are used only when an externally visible semantic is
unclear; no cgo or cross-language callback exists in Phase3B.

## 9. Product-contract and deviation strategy

Before implementation, maintain this reviewed classification:

| Behavior | Classification | Phase3B decision |
|---|---|---|
| rule/matcher declaration order and short-circuit | preserve | typed ordered Rust execution |
| no matcher / no exec / multi-exec forms | preserve | normalize existing config semantics |
| complete upstream response wire ownership | preserve | inspect without lossy decode or consumption |
| `goto`/`jump`/`return`/`accept`/`reject`/`exit`/`try` | preserve | explicit Rust control flow |
| caller-owned state after completion/error | preserve | borrowed root/internal execution API |
| positive qname/switch routing labels and write-once `domain_set` | preserve | typed dispatcher metadata |
| reversed membership label | preserve semantic | reverse never claims the positive set/rule label |
| repeated matcher/executable kinds | preserve | valid; declaration order retained |
| malformed raw response silently becoming absence | intentional Rust deviation | typed malformed-response error |
| infinite control-flow hang/stack recursion | intentional Rust deviation | shared fuel + `BudgetExceeded` |
| Go `ChainWalker`, normal/fast duplication, `map[uint32]any`, name parsing | implementation-only | do not port |
| Go cgo selectors/fallback/ABI state | implementation-only/transitional | do not extend into sequence-core |

Targeted Go characterization is added only when this matrix cannot be resolved
from documented/config semantics and current source. Rust red tests are written
against the reviewed Rust contract, not blanket Go parity.

## 10. Rollback and non-goals

The implementation is isolated to the new crate and Go test fixtures. If a
slice fails review, the new crate/test changes can be reverted without
touching production Go, the runtime static library, the Phase 3A ABI, or any
live selector. Each slice ends with a root-review stop before the next slice.

No network, upstream, listener, server, WebUI, configuration, cache, matcher
or broad no-network plugin implementation is hidden behind this foundation.
