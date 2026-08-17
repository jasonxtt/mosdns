# Phase 3 boundary research

Date: 2026-08-15

## Repository evidence

- `rust/Cargo.toml` currently has only `cache-core`, `matcher-core`, and
  `runtime` workspace members. The runtime header exports cache, domain/IP,
  and valued-domain matcher handles; it exports no query-context or sequence
  ABI.
- `pkg/query_context/context.go` keeps the mutable request owner in Go. The
  context contains the query, client/response/upstream EDNS options, decoded or
  raw response bytes, server metadata, key/value state, normal marks, fast
  flags, and copy/ownership methods. `SetRawResponse` and `R` defer or trigger
  wire decoding; `CopyTo` deep-copies message/response state.
- `pkg/server_handler/entry_handler.go` creates the context, applies pre-fast
  flags and audit state, executes a Go `sequence.Executable`, converts errors
  to SERVFAIL/REFUSED behavior, and handles raw response ID/RA/EDNS/UDP/TCP
  framing.
- `plugin/executable/sequence/iface.go` exposes `Executable`,
  `RecursiveExecutable`, and `Matcher` interfaces over the Go context. The
  sequence compiler in `sequence.go` creates a fast path when matcher and
  executable contracts allow it; `chain.go` executes ordinary and recursive
  chains. `built_in.go` defines `accept`, `reject`, `return`, `exit`, `try`,
  `jump`, and `goto` semantics.
- Existing public behavior tests include
  `pkg/query_context/context_raw_test.go`,
  `pkg/server_handler/entry_handler_raw_test.go`,
  `plugin/executable/sequence/sequence_test.go`, and cache/matcher parity and
  ABI suites from Phase 1/2.

## Migration constraints found in project docs

- `docs/ai/rust-rewrite-plan.md` defines Phase 3 as DNS wire/TTL/EDNS/ECS,
  stable Rust query context, matcher/no-network executable dispatch, and
  sequence control flow. It explicitly requires a single query-context owner.
- `docs/ai/rust-handover.md` requires one coarse Rust runtime, Linux-first
  verification, Go fallback, and no default-backend or production switch.
- The existing Rust migration tasks intentionally leave upstream and server
  data-plane work for later phases. `/Users/tom/github/mosdns-rust-cache` is a
  read-only reference, not an implementation source to merge wholesale.

## Planning implication

The first Phase 3 review should choose the smallest public boundary that can
prove ownership and wire parity. Adding the full sequence executor to that
first boundary is possible, but it couples query-state ABI design to recursive
control-flow and plugin semantic compatibility; it should be an explicit user
decision rather than an assumed scope expansion.
