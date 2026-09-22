# Implementation plan — Rust Phase 5A native forwarding

Status: **in progress — Slice 4 TCP implementation complete; awaiting final root review**.

The root planning review returned `PLANNING: PASS` at
`a5aef2305ef44614753c2de26d4003526f78ade4`; the user then explicitly
approved implementation and all five reviewed slices. The task is now
`in_progress`. Each slice still requires its own root review before the next
slice; no slice PASS authorizes work outside the frozen plan.

## 0. Pre-start gates

- [x] Root review confirms the task path, branch, source anchor, supported
  config subset, async-machine decision, dependency decision, and stop scope.
- [x] `task.py start` is run only after that approval and records the approved
  slice range; no implementation starts from a planning-only status.
- [x] The existing dirty files remain untouched:
  `.trellis/workspace/tom/index.md`,
  `.trellis/workspace/tom/journal-1.md`, and the existing `.DS_Store` files.
- [x] Historical baseline archive, manifest, frozen environment, 36 raw
  evidence directories, configs, workloads, and report provenance are hashed
  and treated as read-only.
- [ ] Every slice begins with a narrow RED test/fixture or invariant, then
  implements the minimum GREEN change, then runs only its allowlisted checks.
- [ ] Every slice stops for explicit root `PASS` before the next slice.

## 1. Slice 0 — relocate baseline entrypoint and report references

### Allowlist

```text
scripts/run-phase5a-baseline.sh
docs/rust/phase5a-go-baseline.md
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

Forbidden: `.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/**`,
`tests/phase5a-baseline/**`, all `rust/**`, Go product code, dependency
manifests, deployment, SSH/VM execution, and benchmark execution.

### RED → GREEN

1. RED: a read-only shell assertion resolves the current official runner
   default and demonstrates that the old pre-archive path is stale.
2. GREEN: add `MANIFEST_PATH` with the archived manifest as the default,
   preserve official-mode `MANIFEST_SHA256` verification, and update only the
   report's status/physical paths while retaining historical hashes/provenance.
3. RED: compare archived manifest/environment/raw-evidence/config/workload
   hashes and report the exact changed-path allowlist.
4. GREEN: retain those comparisons in task-local relocation evidence; do not
   rewrite any historical artifact.

### Required checks

```bash
bash -n scripts/run-phase5a-baseline.sh
ARCHIVE_MANIFEST='.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/research/run-manifest.json'
ARCHIVE_SHA256='a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7'
test -f "$ARCHIVE_MANIFEST"
test "$(sha256sum "$ARCHIVE_MANIFEST" | cut -d' ' -f1)" = "$ARCHIVE_SHA256"
test ! -e '.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json'
rg -Fq 'MANIFEST_PATH' scripts/run-phase5a-baseline.sh
rg -Fq "$ARCHIVE_MANIFEST" scripts/run-phase5a-baseline.sh
! rg -Fq 'pending final task review' docs/rust/phase5a-go-baseline.md
git diff --check -- scripts/run-phase5a-baseline.sh docs/rust/phase5a-go-baseline.md \
  .trellis/tasks/09-22-rust-phase5a-native-forwarding
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
```

The actual official runner must not be invoked in this slice. The evidence
record must say that no new measurement was performed.

### Exit gate

Request `SLICE 0: PASS` with confirmation that only the runner default/report
physical links changed, the historical manifest SHA is unchanged, and no
benchmark or product path changed. A PASS authorizes Slice 1 only.

### Slice 0 execution record

Commit: `a884706` (`chore(phase5a): relocate baseline manifest entrypoint`).

The runner now resolves the archived manifest by default or an explicit
`MANIFEST_PATH`; the report says `FINAL: PASS — archived baseline` and points
current physical evidence references at the archive. The archived manifest
SHA, frozen environment file, 36 frozen raw-evidence directories, baseline
configs/workloads, historical runner hash, and raw provenance were not edited.

Checks:

```text
bash -n scripts/run-phase5a-baseline.sh                 PASS
archived manifest SHA-256 and path assertions           PASS
environment-frozen.json present; frozen-* count = 36    PASS
official runner / measurement / VM / SSH                NOT RUN
task.py validate rust-phase5a-native-forwarding          PASS
git diff --check                                         PASS
```

The exact changed paths are the Slice 0 allowlist plus this task's relocation
record. Root review returned `SLICE 0: PASS` with P0/P1/P2=0, authorizing
Slice 1 only.

## 2. Slice 1 — canonical sequence suspension/resume

### Allowlist

```text
rust/sequence-core/**
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

Forbidden: Tokio/upstream/native-host dependencies, listener/config changes,
network I/O, broad `Send + Sync` trait migration, and unrelated refactors.

### RED → GREEN

1. RED: add focused tests that expose the current inability to return an
   external dispatch and resume with preserved frames/state.
2. GREEN: refactor the current loop into one canonical resumable machine with
   stable executable identity, pending-dispatch validation, retained
   continuation/fuel/cancellation state, and typed terminal outcomes.
3. RED: test wrong executable response, duplicate resume, post-terminal resume,
   missing entry, cancellation, fuel exhaustion, and state mutation across a
   pending forward dispatch.
4. GREEN: implement the deterministic errors and retain the existing sync
   `execute` API as an adapter over the same machine.
5. RED: compare existing fixture programs through the old surface and the
   machine, including `try`/frame unwinding and all existing outcomes.
6. GREEN: resolve only the parity failures required by the canonical-machine
   refactor; do not change product semantics.

### Required checks

```bash
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-sequence-core --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-sequence-core \
  --all-targets --locked -- -D warnings
git diff --check -- rust/sequence-core .trellis/tasks/09-22-rust-phase5a-native-forwarding
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
```

Also run `cargo tree --manifest-path rust/Cargo.toml -p mosdns-sequence-core \
--edges normal --locked` to prove no Tokio/upstream/native-host edge. No host
or network command is allowed.

### Exit gate

Request `SLICE 1: PASS` for the exact sequence-core diff. Review must confirm
one machine/one control-flow engine, sync adapter parity, state retention,
typed invalid-resume behavior, and no async/runtime dependency. A PASS
authorizes Slice 2 only.

### Slice 1 execution record

Implementation commits: `cd9e14b` (`feat(sequence): add resumable execution machine`)
and `f17ae97` (`test(sequence): close resumable parity coverage`).

The canonical sequence machine is implemented in `rust/sequence-core`.
`ExecutionMachine` owns state/control for resumable native-host use and also
supports a borrowed mode used by the existing synchronous `execute` adapter;
both modes share the same scope/frame/continuation engine. External
executables have stable catalog IDs and produce an identity-only dispatch.
`resume` validates the pending ID, preserves state and frames, and maps
accepted/rejected/exit/error outcomes through the existing scope rules.

Focused coverage is in `rust/sequence-core/tests/slice5_resumable.rs` and
covers state retention, multiple pending dispatches, wrong/duplicate and
post-terminal resumes, fuel/cancellation boundaries, and sync adapter
behavior. No Tokio, upstream, listener, network, config, or broad trait-bound
change was introduced.

Checks:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo test -p mosdns-sequence-core --all-targets --locked       PASS (65 tests)
cargo clippy -p mosdns-sequence-core --all-targets --locked ... PASS
cargo tree -p mosdns-sequence-core --edges normal --locked      PASS (dns-core only)
git diff --check -- allowlisted paths                         PASS
host/network/VM/SSH/benchmark commands                        NOT RUN
```

The first Slice 1 root review returned `SLICE 1: FAIL` with P0=0, P1=1,
P2=1. The bounded remediation added the required owned-machine versus sync
adapter outcome/state parity matrix (Continue/Return/Accept/Reject/Exit and
ordinary error), nested-try parity, post-dispatch fuel/cancellation checks,
and direct missing-entry coverage. It also corrected the `ExecutableId`
catalog-entry documentation. The same Slice 1 checks were rerun and the
remediation root review returned `SLICE 1: PASS` with P0/P1/P2 all zero.
That review authorized Slice 2 only.

## Slice 2 execution record

Implementation commit: `146b46e` (`feat(native-host): add strict phase5a config assembly`).

The new `mosdns-native-host` package provides the exact `mosdns start -c` /
`--config` CLI shape, duplicate-detecting strict YAML compilation, ordered-
independent W1 graph validation, current-thread host runtime, and one
`ForwardAdapter` delegating to the existing upstream request/context/exchange
APIs. `HostAssembly` is deliberately pre-I/O: it constructs no listener and
does not call an upstream exchange. The accepted frozen UDP/TCP YAML files
compile unchanged; invalid fields, types, schemes, hostnames, ports, roles,
tags, references, sequence forms, audit settings, and timeout forms fail
before assembly.

The narrowly scoped `dns-core::synthesize_response` helper and tests cover
associated SERVFAIL/REFUSED ID, question, QR/RA, RCODE, and zero section
counts. No second parser or general DNS builder was introduced.

Slice 2 checks and dependency evidence are recorded in
`research/slice2-config-check.md`. All required focused checks passed; no
listener bind, network/VM/SSH run, benchmark, production integration, or
historical baseline mutation was performed. The first root review found two
P1 gaps; the bounded remediation was recorded in commit `33318d7`, followed
by documentation commit `48404a3`. Root re-review returned `SLICE 2: PASS`
with P0/P1/P2 all zero and authorized Slice 3 only.

The first Slice 2 root review returned `SLICE 2: FAIL` with P0=0, P1=2,
P2=0. The bounded remediation commit is `33318d7` (`test(phase5a): close
slice2 rejection and DNS edge cases`). It adds independent tests for the
missing fail-closed rejection categories and changes `dns-core` query parsing
to retain a self-contained uncompressed question name, with a compressed-
question SERVFAIL response regression validated by the existing response
walker. The focused checks were rerun successfully. Root re-review then
returned `SLICE 2: PASS` with P0/P1/P2 all zero and authorized Slice 3 only.

## 3. Slice 2 — native host config/CLI/assembly before I/O

### Allowlist

```text
rust/native-host/**
rust/Cargo.toml
rust/Cargo.lock
rust/dns-core/**                 # only a root-reviewed tiny response helper
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

No real listener bind, no Linux/SSH/VM execution, no benchmark, no production
integration, and no changes to the frozen Go corpus.

### RED → GREEN

1. RED: add parser/compiler rejection tests for every unsupported top-level,
   plugin, field, type, scheme, hostname, zero-port, duplicate, missing-ref,
   and invalid-sequence form in `research/w1-config-contract.md`.
2. GREEN: add the minimal `mosdns-native-host` package, `mosdns start -c`
   CLI, strict raw-YAML decode, declaration-order-independent typed
   compilation, and a runtime/forward adapter that is not yet bound to a real
   listener.
3. RED: assert that invalid configurations do not construct/bind listener or
   upstream resources and that valid frozen UDP/TCP YAML compiles unchanged.
4. GREEN: wire pre-I/O assembly and test-only host options (short deadline,
   injected shutdown/cancellation) without adding YAML timeout or unsupported
   fields.
5. RED: inspect dependency metadata/tree, MSRV, license, and Cargo.lock diff.
6. GREEN: keep only the approved `yaml_serde`/`serde` and minimal Tokio
   closure; remove any duplicate parser or unnecessary CLI dependency.
7. RED: test response synthesis for ID/question/QR/RA/RCODE/counts if the
   existing `dns-core` surface is insufficient.
8. GREEN: add only the narrowly scoped `dns-core` helper needed for SERVFAIL
   and REFUSED; do not add a second parser or general response builder.

### Required checks

```bash
cargo tree --manifest-path rust/Cargo.toml --workspace --edges normal --locked
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-native-host \
  --all-targets --locked -- -D warnings
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
git diff --check -- rust/native-host rust/Cargo.toml rust/Cargo.lock rust/dns-core \
  .trellis/tasks/09-22-rust-phase5a-native-forwarding
```

The check is assembly-only: no command may bind the configured W1 ports.

### Exit gate

Request `SLICE 2: PASS` for strict config rejection, no-bind ordering, CLI
shape, dependency audit, sequence/upstream adapter ownership, and response
helper tests. A PASS authorizes Slice 3 only.

## 4. Slice 3 — W1 UDP real path

### Allowlist

```text
rust/native-host/**
rust/dns-core/**               # only narrowly necessary shared fixes
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

Do not implement TCP, cache/routing/API, QUIC/secure transport, production
deployment, or a performance campaign.

### RED → GREEN

1. RED: add `rust/native-host/tests/w1_udp.rs` as the focused loopback UDP
   integration target, covering positive A, NXDOMAIN, distinct
   concurrent transaction IDs/qnames, stalled-upstream timeout, and malformed
   datagram behavior.
2. GREEN: own a real Tokio `UdpSocket`, spawn independent request scopes,
   drive the canonical sequence machine, await existing UDP upstream, and
   send only the matching response peer.
3. RED: assert cancellation and shutdown stop admission, cancel/join pending
   tasks, close/drain upstream, release sockets, and permit rebind.
4. GREEN: implement idempotent supervisor shutdown and join-registry cleanup.
5. RED: assert a sequence with no response maps to REFUSED and execution/
   upstream/deadline failures map to associated SERVFAIL.
6. GREEN: complete response/error mapping with no late write after cancellation.

### Required checks

```bash
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host \
  --test w1_udp --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-native-host \
  --all-targets --locked -- -D warnings
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
git diff --check -- rust/native-host rust/dns-core \
  .trellis/tasks/09-22-rust-phase5a-native-forwarding
```

After `SLICE 3: PASS`, the authorized Linux correctness command may be run via
the repository's `ssh mosdns-rust` alias. It must record task-local evidence,
not touch the archived baseline, and must not be interpreted as performance
comparison evidence.

### Exit gate

Request `SLICE 3: PASS` for real UDP response/NXDOMAIN/concurrency/timeout,
malformed-input isolation, cancellation, shutdown, rebind, and no-leak
evidence. A PASS authorizes Slice 4 only.

## Slice 3 execution record

Implementation commit: `0efb9db` (`feat(native-host): add phase5a UDP
forwarding path`).

The native host now owns one current-thread Tokio runtime plus a local task
set, binds a real UDP listener, copies each datagram into an owned request,
drives the canonical `ExecutionMachine` to the named forward dispatch, awaits
the existing `upstream-core` exchange with one caller-owned deadline and
cancellation scope, resumes the same machine, validates/patches the response,
and sends it only to the original peer. Request tasks are tracked and joined;
shutdown stops admission, cancels pending exchanges, closes the upstream, and
releases the listener for rebinding. Malformed datagrams are dropped without
affecting the listener. Execution/upstream/deadline failures synthesize
SERVFAIL, while a completed sequence with no response maps to REFUSED.

Focused coverage is in `rust/native-host/tests/w1_udp.rs` and the private UDP
unit test. It proves positive response, NXDOMAIN, distinct concurrent IDs and
qnames, malformed-input isolation, timeout-to-SERVFAIL, cancellation without
a late write, shutdown/rebind, and the no-response REFUSED mapping. The test
upstream is an independent local loopback mock; no baseline runner, remote
host, VM, benchmark, deployment, or frozen evidence was used.

Required checks before the first review:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check                    PASS
cargo test --manifest-path rust/native-host/Cargo.toml --test w1_udp --locked PASS (4 tests)
cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked PASS (12 unit + 2 integration targets)
cargo clippy --manifest-path rust/native-host/Cargo.toml --all-targets --locked -- -D warnings PASS
cargo test --manifest-path rust/dns-core/Cargo.toml --all-targets --locked  PASS (existing full suite)
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding   PASS
git diff --check -- allowlisted Slice 3 paths                              PASS
```

The direct native-host Tokio manifest remains unchanged from Slice 2; no new
dependency or lockfile change was needed. The first root review returned
`SLICE 3: FAIL` with P0=0, P1=2, P2=0:

- `UdpServer::serve` retained completed request results in its `JoinSet` until
  shutdown, and returned early on a join failure before draining remaining
  tasks or closing upstream.
- the final UDP `send_to` had a cancellation check/send TOCTOU window and had
  no deterministic pre-send cancellation regression.

The bounded remediation is commit `db9c270` (`fix(native-host): close
UDP tasks on cancellation`). It reaps completed tasks during the active
receive loop, drains all remaining task results, closes upstream before
returning either task or receive errors, and uses a cancellation-first
commit-aware send helper. Deterministic tests cover 128 completed-task
reaps, failure plus remaining-task drain and upstream close, and cancellation
at a pre-send gate. Root re-review in conversation `000` returned
`SLICE 3: PASS` with no new findings and authorized Slice 4.

## 5. Slice 4 — W1 TCP and final stop

### Allowlist

```text
rust/native-host/**
rust/dns-core/**               # only narrowly necessary shared fixes
.trellis/tasks/09-22-rust-phase5a-native-forwarding/**
```

Do not add cache/routing/QUIC/API/WebUI, pooling, secure transports, production
cutover, a new benchmark, or a new task.

### RED → GREEN

1. RED: add `rust/native-host/tests/w1_tcp.rs` as the focused loopback TCP
   integration target, covering partial two-byte framing, positive A,
   NXDOMAIN, sequential requests per connection, concurrent connections,
   effective `idle_timeout: 2`, stalled-upstream timeout, EOF/partial frame,
   and client disconnect.
2. GREEN: own `TcpListener`, per-connection cancellation/task scope, exact
   stream framing, sequential per-connection machine execution, and response
   framing using `dns-core`.
3. RED: assert disconnect cancellation and shutdown/rebind leave no connection
   tasks, upstream in-flight exchanges, or listener sockets.
4. GREEN: finish supervisor joins, deadline/error mapping, and task-local W1
   UDP/TCP evidence.
5. RED: run all final focused/workspace gates and an exact path audit.
6. GREEN: remediate only findings within the frozen allowlist; do not widen
   scope after the final gate.

## Slice 4 execution record

The W1 TCP implementation is contained in the existing native-host package;
no dependency or lockfile change was needed. `TcpServer` owns one Tokio
listener and a local join registry. Each accepted connection receives a child
cancellation scope, reads exact two-byte big-endian DNS frames with a fresh
`idle_timeout: 2` budget per frame, and closes only the affected connection on
EOF, partial framing, zero-length framing, malformed DNS, or client
disconnect. Valid frames run the same `execute_request` sequence path as UDP,
one request at a time per connection, while separate connections remain
concurrent. Responses use `dns-core::frame_response(..., FrameMode::Stream)`
and are written through a cancellation-first gate.

The supervisor stops admission, cancels and drains every connection task, then
closes the existing upstream owner before returning. The integration target
`rust/native-host/tests/w1_tcp.rs` uses an independent loopback TCP upstream
and covers positive A, NXDOMAIN, fragmented request framing, sequential
requests on one connection, concurrent connections, idle timeout, stalled
upstream to SERVFAIL, partial-frame EOF isolation, client disconnect, and
shutdown/rebind.

Focused checks completed locally:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check                 PASS
cargo test --manifest-path rust/native-host/Cargo.toml --test w1_tcp --locked PASS (5 tests)
cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked PASS (12 unit + 3 integration targets, 9 integration tests)
cargo clippy --manifest-path rust/native-host/Cargo.toml --all-targets --locked -- -D warnings PASS
```

No browser, VM, benchmark, deployment, or historical baseline mutation was
performed. Final workspace-wide checks and the separately authorized Linux
W1 correctness evidence remain part of the final gate below.

### Required checks

```bash
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets \
  --all-features --locked -- -D warnings
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding
git diff --check
```

Run the authorized Linux W1 correctness evidence through `ssh mosdns-rust`
only after the focused local tests pass. Record exact commands, host/toolchain
facts, config/workload hashes, response/error counts, shutdown/rebind result,
and limitations under this task's research tree.

### Final exit gate and stop

Request explicit `FINAL: PASS` for the complete W1 UDP/TCP path and exact
allowlist. On `FINAL: PASS`, finish/archive through the normal Trellis process
only when separately authorized, then stop. Do not run W2/W3, claim a
performance verdict, deploy, start a next task, or remove migration
scaffolding.

## 6. Remediation rule

An explicit root `FAIL` may be addressed only in the cited slice and only
within that slice's allowlist. Re-run the focused checks, inspect the exact
diff, and request review of the new parent/head pair. A finding that requires
new config scope, a new dependency family, a different reviewer, or an
additional feature is a planning blocker; stop and ask the user rather than
silently widening the task.
