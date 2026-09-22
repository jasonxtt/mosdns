# Native-host dependency audit

## Workspace baseline

`rust/Cargo.toml` defines the workspace, Rust edition 2024, workspace license
`GPL-3.0-only`, and MSRV `1.85`. Existing workspace crates already provide
Tokio in `upstream-core`; `sequence-core` depends only on `mosdns-dns-core`
and must remain free of async/runtime dependencies.

The native host needs three dependency categories only:

1. standard-library argument parsing for `start -c/--config`;
2. strict YAML decoding into private config types;
3. the already-established Tokio runtime/net/signal/time/sync/stream I/O
   surface needed to own listeners and upstream awaits.

## YAML choice

The reviewed choice is the maintained official YAML ecosystem crate:

```toml
yaml_serde = "=0.10.7"
serde = { version = "1", features = ["derive"] }
```

The exact dependency addition is deferred to Slice 2. The pinned version,
license, MSRV, repository, and normal dependency tree must be rechecked at
implementation time and recorded in task-local evidence.

Evidence reviewed during planning:

- `yaml_serde` 0.10.7 metadata: MIT OR Apache-2.0, Rust 1.82, official YAML
  organization repository, with normal dependencies on `indexmap`, `itoa`,
  `libyaml-rs`, `ryu`, and `serde`.
- The official project README describes it as the maintained YAML serde
  implementation/fork.
- The original `serde-yaml` release page is archived/deprecated and is not a
  new host dependency.

References:

- https://github.com/yaml/yaml-serde
- https://raw.githubusercontent.com/yaml/yaml-serde/main/Cargo.toml
- https://github.com/dtolnay/serde-yaml/releases

`serde_yaml` must not be introduced merely because the Go/Rust migration
documentation uses that historical name. Unknown-field rejection and typed
validation remain host policy and must be tested regardless of parser choice.

## CLI choice

Use `std::env::args_os` with a small parser for the single required command.
Do not add clap, structopt, a service manager, or a large CLI framework for
one subcommand. Invalid/missing arguments must fail before config I/O/bind and
return a nonzero result with the usage error.

## Tokio and existing transport reuse

The new host should enable only the Tokio features needed by its actual code:
runtime construction, net, signal, sync/cancellation, time, and stream I/O.
The exact feature list is a Slice 2 decision because it depends on the
existing `upstream-core` API surface. No Tokio dependency is permitted in
`sequence-core`; no crate may create its own runtime.

The host must call `upstream-core::Endpoint`, `ExchangeRequest`,
`ExchangeContext`, `Upstream::exchange`, and `Upstream::close`. It must not
duplicate UDP/TCP transport code or add composite fallback. `dns-core` remains
the sole query/response parser/framer.

## Slice 2 dependency gate

Before the host binds anything, the implementation must record:

- `cargo tree --workspace --edges normal` and the new host's direct tree;
- exact versions, licenses, repositories, and MSRV for newly direct crates;
- absence of duplicate YAML parsers or unnecessary CLI frameworks;
- `cargo fmt --check`, focused host tests, and the workspace lint/test plan;
- the Cargo.lock diff limited to the approved dependency closure.

Any dependency that is unmaintained, exceeds workspace MSRV, has an unclear
license, or introduces an unnecessary runtime/CLI stack requires planning
re-review before use.
