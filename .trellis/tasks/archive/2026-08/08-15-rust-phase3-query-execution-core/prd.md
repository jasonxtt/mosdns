# Rust Phase 3 query execution core

## Goal

Define and implement the first bounded part of Phase 3: establish a versioned
Rust query-state snapshot and DNS wire foundation behind the existing
experimental runtime without changing MosDNS configuration, plugin, routing,
audit, wire, or default Go-only behavior.

The user value is a reversible, measurable path toward native Rust request
ownership. The first deliverable must establish a stable query boundary that
later sequence, upstream, and server work can reuse without repeatedly
serializing the same request through Go/Rust.

## Confirmed facts

- Phase 2 is archived and verified on branch `rust`. Rust remains opt-in via
  `MOSDNS_MATCHER_BACKEND=rust` and the default runtime remains Go-only.
- The single Rust static library currently contains `cache-core`,
  `matcher-core`, and the `runtime` ABI crate. No query-core crate or query ABI
  exists yet.
- Go `pkg/query_context.Context` owns the mutable request state passed through
  plugins: query and EDNS options, decoded or raw response, server metadata,
  key/value fields, marks, fast flags, and copy semantics.
- Go `plugin/executable/sequence` owns matcher evaluation, executable calls,
  recursive `jump`/`goto`/`return`/`try` control flow, `accept`/`reject`/`exit`,
  fast-path compilation, and audit-side effects.
- `pkg/server_handler.EntryHandler` creates the Go context, runs the Go entry,
  handles `ErrExit` and errors, applies response EDNS/UDP/TCP framing, and
  supports the existing raw-response path.
- Existing raw-wire and context tests are in
  `pkg/query_context/context_raw_test.go` and
  `pkg/server_handler/entry_handler_raw_test.go`; sequence behavior is covered
  by `plugin/executable/sequence/sequence_test.go` and related plugin tests.
- The migration plan requires one Rust query-context owner, coarse ABI calls,
  panic/length/handle safety, Go fallback, Linux-first evidence, and no
  production or default-backend switch.
- `/Users/tom/github/mosdns-rust-cache` is a read-only prototype/reference;
  it is not a drop-in implementation or a replacement for current MosDNS
  control-plane behavior.

## Selected first increment

This task takes the recommended **query-state and wire foundation** boundary.
It does not migrate general sequence control flow yet. Rust will receive an
immutable per-query snapshot, inspect/transform only the explicitly supported
wire data, and return caller-owned results through one coarse opt-in call. Go's
existing `query_context.Context` remains authoritative until a later sequence
task explicitly transfers ownership.

## Requirements

### R1 — Preserve the control-plane boundary

Go continues to own configuration loading, plugin registration and lifecycle,
WebUI/API/runtime files, and all components not explicitly migrated. No YAML,
API, metrics, audit, dump, WebUI, OpenWrt, lite, docker, or production-service
change is part of this task.

### R2 — Define one query-state ownership contract

The planning artifacts must specify which side owns the request and response
bytes, query metadata, EDNS/ECS state, cancellation, and errors at every
boundary. In this first increment Go owns the live `query_context.Context`;
Rust owns only a copied immutable snapshot for the duration of its handle.
Rust must not mutate Go query state or return Rust-owned pointers.

### R3 — Implement the wire foundation

Add a pure Rust `dns-core` layer for strict DNS wire inspection and the
specific transformations already required by MosDNS compatibility behavior:
query/header/question validation, EDNS/DO/ECS extraction, response TTL aging
and replacement, and safe response-header/framing helpers. Preserve the
existing cache wire behavior through shared code or a compatibility shim; no
cache API or semantics may change.

### R4 — Add one coarse, versioned query boundary

Expose a versioned, length-safe query snapshot/inspection ABI from the existing
Rust static library. It must use typed handles, explicit capabilities, panic
containment, caller-owned output buffers, deterministic close behavior, and a
Go adapter with a non-cgo/default stub. Select it only with
`MOSDNS_QUERY_BACKEND=rust`; the default path must not load Rust.

### R5 — Keep the migration reversible

The Rust path is selected only by an explicit experimental mechanism. ABI,
construction, runtime, unsupported-input, timeout, or panic failures must be
observable and fall back to the established Go path without stale or partially
updated request state. Default builds must work without Rust, cgo, or a Rust
toolchain.

### R6 — Use observable behavior slices

Each implementation slice must name its public Go/Rust boundary, start with a
failing behavior test, implement the smallest green change, and cover both
default Go-only and Linux+cgo paths where applicable. Sequence and wire parity
must be compared against current Go behavior, not only against internal Rust
tests.

### R7 — Keep Phase 3 bounded

The first task must not silently absorb upstream transports, server listeners,
Go host replacement, WebUI, or a Rust-default rollout. Any sequence or
query-context work beyond the approved first increment requires an explicit
slice boundary and acceptance evidence.

## Acceptance criteria

The following are observable completion gates:

- [x] Go golden fixtures freeze query snapshot ownership, question/header
  validation, EDNS/DO/ECS extraction, response TTL behavior, and framing
  decisions, including malformed and unsupported inputs.
- [x] `rust/dns-core` passes pure Rust unit/property/malformed tests and the
  existing cache wire behavior remains byte/field compatible.
- [x] The versioned query ABI/header exposes capability/version negotiation,
  typed snapshot handles, caller-owned result buffers, required-length
  reporting, panic containment, deterministic close, and misuse tests.
- [x] The Go adapter's `MOSDNS_QUERY_BACKEND=rust` path matches the Go oracle
  through one coarse snapshot inspection/transform call; Rust build/ABI/runtime
  failure falls back to the same Go result and never mutates the live context.
- [x] Default `go test ./...`, `go vet ./...`, `go build ./...`, `CGO_ENABLED=0`
  tests, and tagged stub tests pass without Rust or cgo.
- [x] Linux+cgo normal/race adapter tests, Rust fmt/test/clippy/release, and
  an isolated `mos-test` or equivalent host verification pass for the opt-in
  boundary.
- [x] No sequence, upstream, server, WebUI, OpenWrt, lite, docker, production,
  or default-backend behavior is changed; Rust remains experimental/Go-only.

## Out of scope

- Rust ownership of upstream transports or connection pools;
- UDP/TCP/TLS/HTTP/QUIC server listeners;
- Go `coremain` replacement, YAML/plugin registration rewrite, WebUI/API work,
  release fan-out, OpenWrt/lite/docker changes, or production deployment;
- enabling Rust by default or removing the Go implementation;
- importing the prototype repository as a subtree or adopting KixDNS JSON
  pipeline semantics.
