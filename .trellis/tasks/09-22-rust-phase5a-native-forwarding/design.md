# Design — Rust Phase 5A native forwarding

Status: **planning only**. This document freezes the implementation boundary;
it does not authorize `task.py start`, production-code edits, listener binds,
Linux execution, or benchmark runs.

## 0. Design intent and anchors

The planning anchor is the current `rust` branch:

```text
c28bf6b1f08ec19e784b3f25725d597a995cdfd1
```

The predecessor Go baseline was archived at:

```text
0533c477888055e5425421b1766b057045989946
```

Its manifest and frozen raw evidence are immutable inputs. The archived
manifest SHA-256 is:

```text
a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7
```

The native host is an early final-form Rust host, not an extension of the old
Go/Rust bridge. It owns one Tokio runtime and composes the existing Rust
crates. No crate creates a hidden runtime, and no host path dispatches around
the sequence engine.

The implementation is divided into five reviewable slices. Each slice has an
exact allowlist, focused tests, and a root-review gate. A review `PASS` only
authorizes the next slice named in this document; it never authorizes a new
feature family or a new task.

## 1. Crate and module boundaries

### Existing crates

`rust/dns-core/**` remains the sole DNS wire/parser/framing contract. Reuse
`parse_query`, response validation, stream framing, and
`patch_response_id_ra`. If the existing APIs cannot produce protocol-error
responses, Slice 2 may add one small response constructor in `dns-core` that
accepts the already parsed one-question query and emits SERVFAIL/REFUSED. It
must not parse the request a second time or become a general DNS builder.

`rust/sequence-core/**` remains a pure execution crate. It may gain the
canonical resumable machine and its test seams, but it must not depend on
Tokio, `upstream-core`, sockets, listener code, or host configuration. The
existing control-flow semantics, fuel/cancellation behavior, scope/frame
unwinding, and synchronous fixture tests remain normative.

`rust/upstream-core/**` remains the async exchange/lifecycle owner. The host
constructs numeric UDP/TCP endpoints and calls `Upstream::exchange` with an
owned request and caller-owned deadline/cancellation context. It must not add
new pooling, retry, TC fallback, hostname/bootstrap, secure transport, or
hidden runtime behavior for this task.

### New crate

Slice 2 introduces `rust/native-host/**` as workspace package
`mosdns-native-host`, with binary `mosdns`. Its responsibilities are limited
to:

- the minimal `start -c/--config` argument parser;
- strict YAML decoding and compile-time reference validation;
- one host runtime and shutdown supervisor;
- UDP/TCP listener ownership and connection/request task ownership;
- adaptation between native host configuration and the existing sequence and
  upstream APIs;
- response/error mapping and task-local evidence hooks.

The host does not own a second sequence interpreter, a DNS parser, a second
upstream UDP implementation, product cache/routing behavior, API/WebUI, or
production service wiring.

The initial runtime should use Tokio's current-thread runtime plus a
`LocalSet` if required by the existing sequence fixture object model. This
preserves the current `sequence-core` trait semantics without adding broad
`Send + Sync` requirements to existing matcher/executor fixtures. Tokio
request tasks may still be concurrent and independently cancellable. A future
multi-thread ownership redesign is explicitly deferred; this task makes no
performance claim.

## 2. Query ownership and end-to-end data flow

The ownership invariant is that a request's raw query and final raw response
never borrow a packet buffer that can be reused while async work is pending.

```text
NativeHost
├─ listener owner (UdpSocket or TcpListener)
│  └─ connection/request task
│     ├─ owns raw_query: Vec<u8>
│     ├─ dns_core::parse_query(&raw_query)
│     ├─ ExecutionState (parsed header/question + Raw response slot)
│     ├─ canonical sequence machine
│     │  └─ external forward dispatch: ExecutableId + ExchangeRequest bytes
│     ├─ Upstream::exchange(ExchangeRequest, ExchangeContext)
│     ├─ resume the same sequence machine with response/error
│     └─ commit final wire response or synthesized protocol error
└─ shutdown owner: stop admission -> cancel scopes -> close/drain upstream
   -> await listener/connection/request tasks -> release sockets
```

UDP copies each datagram into an owned request before spawning/entering the
machine and sends the resulting wire response only to that datagram's peer.
TCP owns the connection read buffer and preserves the raw framed request until
the corresponding response has been written. A TCP connection processes one
request at a time; separate connections are independent.

`ExchangeRequest` must be built from the owned raw bytes and remain valid
through the await. The machine owns execution state/control, while the host
owns the bytes, deadline, cancellation scope, and upstream object. No borrowed
executor internals cross an await point.

## 3. Strict configuration compilation

The CLI accepts only `mosdns start -c <path>` (with `--config` as an
equivalent spelling). It loads YAML into a private raw representation and
compiles it into a small typed host configuration. Unknown keys are rejected
at every supported level; there is no permissive pass-through to Go config
types.

The accepted shape is exactly:

```text
top-level: log, plugins
log: { level: error }
plugins: one forward + one sequence + one udp_server OR tcp_server
forward.args: { upstreams: [ { addr: udp://IP:port | tcp://IP:port } ] }
sequence.args: [ { exec: $forward_tag } ]
udp_server.args: { entry, listen, enable_audit: false }
tcp_server.args: { entry, listen, idle_timeout: positive integer,
                   enable_audit: false }
```

The compiler enforces unique tags, exactly one plugin of each required role,
declaration-order-independent references, numeric `SocketAddr` values, nonzero
ports, and the transport/config correspondence. It rejects hostnames and
secure/QUIC schemes, all unsupported forward/listener fields, duplicate or
missing references, matchers, inline/anonymous executables, sequence control
flow, sequence-to-sequence calls, audit, TLS, and unknown plugin types before
any socket bind. TCP `idle_timeout: 2` from the frozen baseline is the minimum
compatibility case; YAML timeout configuration for upstream execution is not
added.

The compiler returns a typed error that identifies the path and reason. The
host constructs no listener or upstream socket until compilation succeeds.

## 4. Canonical asynchronous sequence machine

### Problem

`sequence-core` currently exposes synchronous
`Executor::execute(&mut ExecutionState)`, while `upstream-core::Upstream::exchange`
is asynchronous. Calling the forward directly from the host would bypass
sequence semantics; adding a second host interpreter would create divergent
control flow.

### Chosen shape

Add an internal/publicly testable canonical machine around the existing engine
state:

1. `ExecutionMachine` owns the current program cursor, scopes/frames,
   continuations, fuel/cancellation checks, and the mutable `ExecutionState`
   under the existing sequence-core ownership model.
2. `step()` advances only synchronous work until it reaches a terminal
   `ExecutorOutcome`, a typed error/cancellation, or an owned external
   dispatch request. The dispatch contains a stable executable identity and a
   typed request view/copy that the host can turn into `ExchangeRequest`.
3. The machine records exactly one pending dispatch. `resume()` accepts only
   the matching response/error and continues the same frames and state. A
   wrong, duplicate, or post-terminal resume is a deterministic typed error.
4. The existing synchronous `Executor::execute` API becomes a compatibility
   adapter that drives the same canonical machine to completion using the
   current synchronous fixture dispatch. It does not retain a parallel
   control-flow implementation.

The exact public names may be refined during Slice 1, but the invariant is
stable: one state machine, one continuation representation, one outcome
mapping. The machine must not hold a borrowed query slice or executor borrow
across an async boundary. The native host awaits outside the machine and
resumes it with an owned `ExchangeResponse` or typed execution/upstream error.

For the compiled W1 graph, the named forward is represented in the validated
program as an external executable/catalog entry with the same stable
`ExecutableId` space used by the engine. This is an additive dispatch target,
not a host-side sequence parser: the engine still resolves the sequence rule,
advances the frame, records the pending ID, and owns all outcome/unwind
semantics. The host supplies only the external operation and its result.

Because existing sequence fixtures use non-`Send` test objects, the initial
native host runs them on a current-thread runtime/`LocalSet`; Slice 1 must
prove that this is a runtime ownership choice rather than a hidden runtime in
`sequence-core`. Broad trait-bound changes or a multi-thread executor are out
of scope unless required by a focused parity test and root-approved scope
change.

Slice 1 tests cover: unconditional success, ordinary outcome propagation,
multiple frames/continuations, cancellation/fuel, state retention across a
pending dispatch, wrong executable response, duplicate resume, resume after
finish, and parity between the machine and the existing sync adapter. The
crate remains Tokio/upstream independent.

## 5. Runtime, listeners, and ownership

The host creates one runtime, installs SIGINT/SIGTERM handling, and owns a
shutdown token plus a join registry. The ownership hierarchy is:

```text
NativeHost
  -> listener owners
    -> connection/request tasks
      -> sequence execution scopes
        -> forward owner
          -> upstream-core Upstream
```

Admission is sealed before cancellation. Listener loops observe shutdown and
stop accepting/receiving new work. Request scopes receive cancellation and
their deadlines; upstream close/drain uses the existing `Upstream::close`
lifecycle. The supervisor awaits all child tasks and makes shutdown idempotent.
No detached task, leaked socket, or hidden runtime is acceptable.

### UDP

Use one real Tokio `UdpSocket`. Each datagram gets a new owned request scope,
so concurrent queries cannot share transaction association or mutable response
state. A valid response is sent to the original peer; malformed input is
dropped. Default execution deadline is five seconds, with short deadlines
available only through test host options. Timeout/execution/upstream failures
produce an associated SERVFAIL where a parsed request is available.

### TCP

Use one Tokio `TcpListener` and a per-connection task. Read the two-byte
big-endian DNS length prefix with partial-read handling, reject invalid/zero
frames locally, and process requests sequentially per connection. Write the
same framing around the final response. `idle_timeout` applies to the
baseline's value of two seconds. EOF, partial frame, or disconnect cancels
only that connection; shutdown cancels and joins all connections.

## 6. Forward/upstream composition

The compiled forward contains exactly one numeric `Endpoint` and one forward
executable identity. The host's forward adapter receives an owned request from
the machine, creates `ExchangeRequest::new(&raw_query)`, and calls the
existing async `Upstream::exchange` with one caller-owned deadline and
`TransportCancellation` scope.

Only `Transport::Udp` and `Transport::Tcp` are admitted. The task does not
wire `UdpTcpPolicy`, TC fallback, secure transports, hostname resolution,
bootstrap, pooling, retry, or connection reuse. The upstream result is copied
into the machine's raw response slot and the machine is resumed. The host then
validates/frames the final response through `dns-core` and commits it once.

## 7. Response and error contract

- A valid upstream wire response is retained as raw bytes and associated with
  the original request. Existing response validation and
  `patch_response_id_ra` are reused; the request ID is restored and RA is set
  according to the current server contract.
- NXDOMAIN is a normal raw response and must not be converted to an execution
  failure.
- After a query has been parsed, deadline/upstream/execution failure returns a
  DNS response with the request ID/question, QR set, RA set, and SERVFAIL.
- If the sequence completes without installing a response, return REFUSED.
- If parsing fails, UDP drops the datagram and TCP closes only the connection;
  the host remains available for later requests.
- No response is committed before the machine has completed and all necessary
  upstream ownership checks have passed. Late/cancelled responses cannot be
  written after a request scope is closed.

If the existing `dns-core` helpers cannot create the two synthesized response
forms, Slice 2 adds only the narrowly scoped builder described in Section 1,
with tests for ID, original question, QR/RA, RCODE, and section counts.

## 8. Baseline relocation and independent evidence

Slice 0 is documentation/runner relocation only. It changes the runner to
accept an explicit `MANIFEST_PATH`, defaults it to the archived baseline
manifest, retains and verifies the historical `MANIFEST_SHA256`, and updates
the Go baseline report from pending to archived/final with the physical
archive path. It must not edit the archived manifest, `environment-frozen.json`,
any of the 36 frozen raw evidence directories, baseline configs/workloads, or
the old report's historical provenance. The old runner hash is retained in the
new report.

All new records live under this task's
`.trellis/tasks/09-22-rust-phase5a-native-forwarding/research/results/**`.
They identify the current branch/SHA, host/runtime details, config/workload
hashes, test command, and whether evidence is W1 correctness only. The current
two-CPU `ssh mosdns-rust` host cannot support a performance comparison with
the archived four-CPU Go environment; no throughput/CPU/RSS verdict is
claimed. Planning does not run that host.

## 9. Dependency decision

The host needs only a minimal argument parser and strict YAML decoding. Use
`std::env` for `start -c/--config`; do not add a large CLI framework. Use the
maintained official YAML ecosystem crate `yaml_serde` pinned/reproducibly
locked at the reviewed `0.10.7` release, with `serde` derive as needed. The
crate is compatible with workspace Rust 1.85 and has a normal direct
dependency tree; its license and transitive tree must be recorded with
`cargo tree --edges normal` in Slice 2. Do not introduce the archived
`serde_yaml` package or an unmaintained alternative.

The planned host Tokio feature set is the smallest one needed for the runtime,
net, signal, sync, time, and stream I/O behavior; exact features are frozen
only when the new crate is added. Slice 2 must run `cargo tree --edges normal`,
inspect licenses/MSRV/maintenance, and reject duplicate or unnecessary
runtime/parser dependencies.

## 10. Compatibility matrix

| Surface | Accepted in this task | Rejected/future |
|---|---|---|
| CLI | `start -c/--config` | other commands, daemon/service packaging |
| top-level YAML | `log`, `plugins` | all other keys/includes/state |
| log | `level: error` | other levels/fields |
| plugins | one forward, sequence, one UDP or TCP listener | cache, matcher product wiring, routing, API/WebUI |
| forward | one numeric `udp://`/`tcp://` `addr` | hostname/bootstrap, DoT/DoH/DoQ/DoH3/QUIC, retry/pool/fallback |
| sequence | one unconditional named forward exec | matchers, inline lists, control flow, nested sequences |
| UDP | numeric listen, audit false, concurrent requests | audit, TC-to-TCP wiring, production service |
| TCP | numeric listen, positive idle timeout, audit false, DNS framing | TLS/cert/key, pipeline, pooling, HTTP/3 |
| response | raw valid response, NXDOMAIN, SERVFAIL, REFUSED | cache/routing mutation, full audit/metrics |
| evidence | task-local W1 correctness records | performance claim, production deployment |

## 11. Future implementation paths and allowlists

These are planned allowlists, not current-turn changes.

### Slice 0 — baseline relocation

Allowed: `scripts/run-phase5a-baseline.sh`,
`docs/rust/phase5a-go-baseline.md`,
`.trellis/tasks/09-22-rust-phase5a-native-forwarding/**` research/task docs.

Forbidden: archived baseline files/raw evidence, `tests/phase5a-baseline/**`,
all `rust/**`, Go product code, transport, deployment, and benchmark execution.

### Slice 1 — sequence suspension/resume

Allowed: `rust/sequence-core/**` and task-local docs/research only.

Forbidden: Tokio/upstream dependencies, native host, listeners, network,
config parser, production wiring, and unrelated crate cleanup.

### Slice 2 — host pre-I/O assembly

Allowed: `rust/native-host/**`, workspace Cargo manifest/lock, and only a
root-reviewed tiny `rust/dns-core/**` response helper if required; task-local
docs/research.

Forbidden: real listener bind, VM/SSH execution, benchmark runs, cache/routing,
Go/cgo/backend selectors, production integration, and any change to
`sequence-core` outside Slice 1.

### Slice 3 — W1 UDP

Allowed: the new host/listener and narrowly necessary existing crate fixes,
test fixtures, and task-local W1 UDP evidence. Run only the approved focused
checks and the authorized Linux correctness command after review.

Forbidden: TCP implementation, cache/routing/QUIC, performance campaign,
deployment, or creating a new task.

### Slice 4 — W1 TCP and final stop

Allowed: native-host TCP path, narrowly necessary shared fixes, focused tests,
and task-local W1 UDP/TCP evidence. Run the approved final gates and Linux W1
correctness evidence.

Forbidden: all non-W1 feature families, production/default cutover, benchmark
rerun as a performance verdict, deployment, finish/archive, or a next task
until the root reviewer returns explicit `FINAL: PASS`.
