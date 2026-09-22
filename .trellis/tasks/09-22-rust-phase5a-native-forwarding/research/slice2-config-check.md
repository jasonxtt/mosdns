# Slice 2 config/CLI/assembly evidence

## Scope and commit

Slice 1 received the bounded remediation review result `SLICE 1: PASS` with
P0/P1/P2 all zero. Slice 2 was then implemented at:

```text
146b46e feat(native-host): add strict phase5a config assembly
```

The implementation changed only the approved Slice 2 product paths:

- `rust/native-host/**`
- `rust/Cargo.toml`
- `rust/Cargo.lock`
- `rust/dns-core/src/header.rs`
- `rust/dns-core/src/lib.rs`

The pre-existing `.trellis/workspace/**` and `.DS_Store` changes were not
staged or committed.

## Configuration contract checks

`mosdns-native-host` decodes YAML into a private duplicate-detecting raw
representation and compiles only the frozen W1 subset. The compiler rejects
unknown fields at every supported level, duplicate plugin tags and YAML keys,
duplicate role plugins, missing roles/references, non-numeric hostnames,
zero ports, unsupported schemes, audit=true, non-integer/zero TCP timeouts,
and sequence forms other than one named `$forward` executable. It collects
all plugin tags before resolving the sequence/listener references, so the
accepted graph is declaration-order independent.

The exact frozen UDP and TCP fixture files are included by tests without
modification. Their listen/upstream ports, transport kinds, entry tag, and
TCP `idle_timeout: 2` compile into the typed graph unchanged.

`HostAssembly` owns one current-thread Tokio runtime, one
`ForwardAdapter`, and one existing `upstream-core::Upstream`. The adapter
constructs `ExchangeRequest` and `ExchangeContext` only when a later request
runner calls its async method; Slice 2 calls no network method and owns no
listener. Invalid YAML returns before `HostAssembly` construction. The
assembly tests assert the upstream lifecycle remains `Open` and expose no
listener socket.

## Root-review remediation

The first Slice 2 root review returned `SLICE 2: FAIL` with P0=0, P1=2, P2=0.
The bounded remediation is:

```text
33318d7 test(phase5a): close slice2 rejection and DNS edge cases
```

The native-host rejection tests now independently exercise unsupported
scheme, wrong YAML type, missing listener-to-sequence reference, invalid
sequence control syntax, `enable_audit: true`, negative TCP timeout, and
non-integer TCP timeout. The audit case no longer contains a duplicate key, so
it proves semantic rejection rather than only YAML duplicate detection.

The DNS query decoder now stores a self-contained uncompressed question name
when accepted input used a compression pointer. The response helper can
therefore copy the parsed `QuestionInfo` into a new packet without retaining a
pointer into the old query. A compressed-question regression constructs
SERVFAIL and validates the complete response through the existing `dns-core`
response walker.

`dns-core::synthesize_response` is the narrowly scoped protocol-error helper
needed by later W1 request handling. It accepts an already parsed
`QueryHeader`/`QuestionInfo`, preserves the request ID/question, sets QR and
RA, emits the requested four-bit RCODE, and emits zero answer/authority/
additional counts. Tests cover SERVFAIL (2), REFUSED (5), and invalid RCODE
rejection; it does not parse a second query or become a general DNS builder.

## Dependency audit

The normal locked workspace tree was inspected with:

```text
cargo tree --manifest-path rust/Cargo.toml --workspace --edges normal --locked
```

The new direct dependencies are:

| crate | locked version | license | MSRV | purpose |
|---|---:|---|---:|---|
| `yaml_serde` | 0.10.7 | MIT OR Apache-2.0 | 1.82 | YAML decoding |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | 1.56 | deserializer contract |
| `tokio` | 1.53.1 | MIT | 1.71 | host-owned current-thread runtime |

The workspace MSRV remains Rust 1.85. `std::env::args_os` is used instead of
adding a CLI framework. No second YAML parser, Go bridge, cache/routing
dependency, or production integration was added. The lockfile additions are
the approved `yaml_serde` closure (`libyaml-rs`, `ryu`) plus the existing
Tokio feature closure; the tree contains no duplicate YAML parser.

## Required checks

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets --locked  PASS (10 tests)
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --locked     PASS (55 unit + existing integration tests)
cargo clippy --manifest-path rust/Cargo.toml -p mosdns-native-host --all-targets --locked -- -D warnings  PASS
python3 ./.trellis/scripts/task.py validate rust-phase5a-native-forwarding                         PASS
git diff --check -- approved Slice 2 paths                                                      PASS
```

No listener port was bound. No browser, VM, SSH host, benchmark, deployment,
frozen baseline corpus, or historical evidence was touched.
