# Rust Phase 5A native forwarding

## Goal

Establish the smallest final-form pure-Rust MosDNS host that can start with
the existing Phase 5A W1 YAML shape and complete one real DNS data path:

`start -c YAML -> listener -> asynchronous sequence -> forward/upstream -> response`

The work must reuse the existing Rust `sequence-core`, `dns-core`, and
`upstream-core` contracts, preserve the frozen Go baseline as historical
evidence, and stop after W1 UDP and TCP acceptance. This is an experimental
native-host foundation, not a production cutover.

## Supported configuration and behavior

- `mosdns start -c <config>` is the only required CLI form; `--config` may be
  its equivalent spelling.
- Top-level YAML contains only `log` and `plugins`. The supported log shape is
  only `log.level: error`.
- The plugin set contains exactly one named `forward`, one named `sequence`,
  and one listener: either `udp_server` or `tcp_server`. Tags are unique and
  all references are validated independently of declaration order.
- `forward` contains exactly one numeric `udp://` or `tcp://` upstream with
  only an `addr` field. Hostnames, secure transports, bootstrap, pooling,
  retry/fallback, and unsupported forward options are rejected.
- `sequence` contains exactly one unconditional rule that executes the named
  forward plugin. Matchers, control-flow variants, inline/anonymous
  executables, and all other sequence forms are rejected.
- `udp_server` accepts `entry`, numeric `listen`, and `enable_audit: false`.
  `tcp_server` additionally accepts a positive integer `idle_timeout`.
  Audit, TLS, and all other listener fields are rejected.
- Configuration errors are reported before socket bind. Malformed inbound DNS
  is handled locally: UDP drops the datagram and TCP closes only the affected
  connection.
- Valid queries receive the upstream wire response, including NXDOMAIN, with
  request association preserved. Execution/upstream/deadline errors return a
  correctly associated SERVFAIL; a sequence with no response returns REFUSED.
- UDP requests may execute concurrently without cross-association. TCP uses
  DNS length framing, supports partial reads, and processes requests
  sequentially per connection while allowing concurrent connections.
- Shutdown stops admission, cancels and joins in-flight work, closes/drains
  upstream ownership, and permits clean rebinding without leaked tasks or
  sockets.

## Non-goals and constraints

- Do not migrate cache, rule routing, matcher product wiring,
  `special_groups`, API/WebUI, full audit/metrics, include/config packages,
  persistent state, or any W2/W3 behavior.
- Do not add DoT, DoH, DoQ, DoH3, QUIC, bootstrap/hostname resolution, UDP
  TC-to-TCP fallback, pooling/retry expansion, or connection reuse.
- Do not add Go/cgo bridges, backend selectors, `rust/runtime` integration,
  production/default wiring, deployment, release work, or a follow-up task.
- The existing sequence engine remains the single semantic engine. The native
  host must not bypass it or introduce a second sequence interpreter.
- Historical baseline manifest, frozen environment record, raw evidence,
  configs, workloads, and the old report provenance remain unchanged. Any
  new measurements use task-local independent evidence.
- Linux execution evidence is limited to the explicitly authorized W1 checks;
  the current two-CPU SSH host is not a performance-comparable replacement
  for the archived four-CPU Go baseline.

## Acceptance criteria

- [ ] The baseline runner/report relocation is planned and implemented as a
  separate first slice without mutating historical manifest or raw evidence.
- [ ] The sequence engine exposes one canonical resumable execution path whose
  synchronous API is an adapter, and its semantics remain parity-tested.
- [ ] Supported YAML compiles strictly before any bind; every listed
  unsupported or malformed shape fails closed with a useful error.
- [ ] W1 UDP passes positive response, NXDOMAIN, concurrent association,
  timeout-to-SERVFAIL, cancellation, shutdown, rebind, and task/socket cleanup
  checks.
- [ ] W1 TCP passes positive response, NXDOMAIN, partial framing, sequential
  per-connection behavior, concurrent connections, idle timeout, timeout and
  disconnect handling, shutdown, rebind, and cleanup checks.
- [ ] Focused Rust tests, formatting/lint checks, task validation, and the
  authorized Linux W1 evidence are recorded in the task-local research tree.
- [ ] No production deployment or broader migration work is performed.

## Stop boundary

After W1 UDP and W1 TCP receive explicit root-review `PASS`, stop with the
task in the approved final state. Do not begin cache/routing/API work, run a
new benchmark campaign, deploy the binary, or create/start another task.
