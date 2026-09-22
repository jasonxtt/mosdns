# Current native-host audit

## Audit scope

Repository: `/Users/tom/github/mosdns-rust`
Branch: `rust`
Planning anchor: `c28bf6b1f08ec19e784b3f25725d597a995cdfd1`
Audit date: 2026-09-22

This is a read-only planning audit. No native host exists yet and no product
source was changed during planning.

## Current architecture facts

### No host or listener implementation exists

The Rust workspace in `rust/Cargo.toml` currently contains `cache-core`,
`matcher-core`, `dns-core`, `sequence-core`, `upstream-core`, and
`runtime`. There is no `native-host` member and no Rust `mosdns` binary. The
Go server/listener packages therefore remain source evidence only; they are
not a Rust implementation dependency.

### `sequence-core` is synchronous and already has the required control state

Relevant symbols:

- `rust/sequence-core/src/engine.rs:61-103` — `ExecutionControl` owns fuel,
  cancellation state, and a cloneable cancellation token.
- `rust/sequence-core/src/engine.rs:126-157` — the public `execute` loop
  drives `next_step` to completion with caller-owned `ExecutionState` and
  `ExecutionControl`.
- `rust/sequence-core/src/engine.rs:166-197` — `Scope` owns a current
  `Frame`, pending fixture identity, and continuations.
- `rust/sequence-core/src/engine.rs:211-244` — `next_step` consumes pending
  fixture dispatches or advances a sequence frame.
- `rust/sequence-core/src/program.rs:69-99` — `Matcher` and `Executor` are
  synchronous pure-Rust traits; neither has a Tokio or network dependency.
- `rust/sequence-core/src/program.rs:173-205` — current executable targets
  include control flow and named fixture IDs.

The existing implementation is therefore close to a resumable machine, but
its internal `Scope`/`Frame`/`next_step` representation is not yet an owned
external-dispatch boundary. A host must not call an executor directly around
the engine, because that would bypass the engine's frame, fuel, cancellation,
and outcome semantics.

Existing fixture tests use non-`Send` test objects such as `Rc`/`RefCell`.
Adding blanket `Send + Sync` bounds to the public traits would be a large,
unrelated semantic change. The initial host plan consequently uses one
caller-owned current-thread Tokio runtime and, where needed, a `LocalSet`.
This is an execution choice in the host, not a runtime dependency in
`sequence-core`.

### `ExecutionState` already owns the response slot

`rust/sequence-core/src/state.rs:7-27` constructs state from a parsed
`QueryHeader` and `QuestionInfo`. `state.rs:40-57` supports an owned raw wire
response and synthesized RCODE state. `state.rs:144-165` keeps raw response
bytes in `OwnedResponseWire(Vec<u8>)` rather than a borrowed packet.

This supports the planned ownership rule: the request task retains its raw
query `Vec<u8>`, the machine owns the execution state, and the async host
copies the upstream response into the state's raw response slot before
resuming the same machine.

### `upstream-core` already supplies the async transport boundary

Relevant symbols in `rust/upstream-core/src/lib.rs`:

- `Endpoint::new` at lines 70-95 validates numeric endpoints and rejects port
  zero before socket setup.
- `ExchangeRequest` at lines 110-145 validates a query through
  `dns-core::parse_query`, records the request ID, and borrows caller-owned
  bytes.
- `TransportCancellation` and `ExchangeContext` at lines 147-228 provide
  caller-owned cancellation and one absolute deadline; cancellation wins a
  simultaneous deadline check.
- `ExchangeResponse` is the owned response boundary at line 543 and later;
  the exchange result exposes owned wire bytes and transport metadata.
- `Upstream` is defined at line 996; `close` is async at line 1091 and
  `exchange` is async at line 1143. The lifecycle tracks in-flight work and
  drains on close.

The host can therefore build `ExchangeRequest` from the request-owned bytes,
await `Upstream::exchange` outside the pure sequence crate, and resume the
machine with an owned response/error. It must not implement another UDP
socket, add a hidden runtime, or wire `UdpTcpPolicy`/TC fallback for W1.

### `dns-core` covers most wire behavior

- `rust/dns-core/src/query.rs:101` — strict query parsing returns the typed
  header/question for one-question queries.
- `rust/dns-core/src/header.rs:96` — `patch_response_id_ra` copies a response,
  restores the request ID, and sets RA after checking response shape.
- `rust/dns-core/src/header.rs:117` — `frame_response` provides UDP/stream
  framing, including the two-byte TCP length prefix.
- `rust/dns-core/src/response.rs:46` — `validate_response` validates upstream
  response wire data.

There is no general response builder in the current public surface. Slice 2
may add a narrowly scoped constructor for SERVFAIL/REFUSED from already parsed
query data, with direct tests, rather than parsing or packing a second DNS
representation in the host.

## Gap list and planned resolution

| Gap | Why it matters | Planned owner |
|---|---|---|
| sync sequence API stops at no async boundary | host would otherwise bypass semantics or create a second engine | Slice 1 `sequence-core` machine + sync adapter |
| no typed native config/compiler | permissive YAML could bind unsupported behavior | Slice 2 `native-host` |
| no native CLI/listener/runtime | no real W1 path exists | Slice 2 assembly, Slice 3 UDP, Slice 4 TCP |
| raw response error builder may be incomplete | SERVFAIL/REFUSED must retain ID/question | Slice 2 only if existing helpers are insufficient |
| current fixtures are not broadly `Send` | multi-thread host would force unrelated trait changes | current-thread host/`LocalSet`; defer redesign |
| historical runner points at pre-archive task path | future measurements cannot resolve archived manifest | Slice 0 explicit manifest path/default |

## Safety invariants for implementation review

1. No listener is bound before the complete config graph is compiled.
2. No raw query buffer is reused while an exchange is pending.
3. The same sequence machine owns every frame/continuation before and after an
   upstream await.
4. A response is committed once, after validation and cancellation checks.
5. Shutdown closes admission before cancelling/awaiting children and upstream.
6. Existing historical baseline paths and hashes remain byte-for-byte stable.
