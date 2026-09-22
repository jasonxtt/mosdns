# Implementation plan — Rust Phase 5A native forwarding

Status: **in progress — Slice 0 implementation**.

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
record. The task remains `in_progress` and is stopped here pending root
`SLICE 0: PASS`.

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
