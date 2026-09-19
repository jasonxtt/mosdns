# Slice 0 dependency audit — QUIC stack (recorded 2026-09-18/19)

Method: network egress from the dev shell was denied, so versions were read
from upstream Git history via raw `Cargo.toml` fetches (0.11.x branch HEAD and
h3 master), not from a local `cargo metadata` resolution. A locked local
resolution audit (`cargo metadata` + `cargo tree -e normal`) is still required
in Slice 0 before any QUIC socket code, and must confirm no resolved package
above MSRV 1.85.

## Audited selection

| Crate | Pinned version | License | rust-version | edition | rustls |
|---|---|---|---|---|---|
| `quinn` (0.11.x branch) | `=0.11.7` | MIT OR Apache-2.0 (workspace) | 1.85 (workspace) | 2021 (workspace) | 0.23.5, `default-features = false`, `std` |
| `quinn-proto` (via quinn) | 0.11 (same branch) | MIT OR Apache-2.0 (workspace) | 1.85 (workspace) | 2021 | same 0.23.5 |
| `quinn-udp` (via quinn) | 0.5 (same branch) | MIT OR Apache-2.0 (workspace) | 1.85 (workspace) | 2021 | — |
| `h3` (master) | `=0.0.8` | MIT | 1.74 | 2021 | — (transport-agnostic) |
| `h3-quinn` (master) | `=0.0.10` | MIT | 1.74 | 2021 | via quinn 0.11.7 |

Sources: `quinn-rs/quinn` branch `0.11.x` (`Cargo.toml` workspace root,
`quinn/Cargo.toml`, `quinn-proto/Cargo.toml`); `hyperium/h3` master
(`h3/Cargo.toml`, `h3-quinn/Cargo.toml`).

## Gate checklist (design.md §2)

1. **License**: quinn family MIT OR Apache-2.0; h3 family MIT. Both compatible
   with `GPL-3.0-only`. PASS (pending lockfile confirmation).
2. **MSRV ≤ 1.85**: quinn workspace `rust-version = "1.85"` (exactly at the
   workspace MSRV, not above); h3/h3-quinn `rust-version = "1.74"`.
   No crate declares above 1.85. PASS on declared versions; resolved-graph
   audit still required locally.
3. **TLS-stack alignment** (design.md §2 item 5): quinn 0.11.x workspace uses
   `rustls 0.23.5`, matching this workspace's `rustls 0.23`. PASS on declared
   versions; lock must confirm a single 0.23.x line.
4. **Client-only feature selection**: quinn default =
   `["log", "platform-verifier", "runtime-tokio", "rustls-ring", "bloom"]`.
   This task MUST disable default features and enable exactly
   `runtime-tokio` + `rustls-ring` (+ `log` as needed), explicitly excluding
   `platform-verifier` (platform trust-store auto-loading would violate the
   caller-supplied-roots contract) and `aws-lc-rs` (would introduce a second
   crypto provider beside the reviewed `ring`). quinn-proto default
   (`rustls-ring`, `log`, `bloom`) is acceptable as pulled through the
   narrowed quinn features. h3 has no default features beyond `tracing`
   (optional); h3-quinn is client-usable without server features. Exact
   feature sets are frozen at implementation time in `Cargo.toml` with
   `default-features = false`.
5. **Tree shape**: to be confirmed locally via
   `cargo tree -e normal -p mosdns-upstream-core` after pinning.

## Decision

Audit PASSES on declared metadata. The task proceeds to Slice 0 RED
endpoint/ALPN/byte-shape tests. The Slice 0 exit gate additionally requires
the local locked resolution (`Cargo.lock` + `cargo metadata` showing no
package above 1.85 + `cargo tree -e normal` review) before Slice 1 socket code.
If the local resolution contradicts any row above, the task stops per the
design §2 hard gate.

## Locked resolution audit — 2026-09-19 (Slice 0 exit gate, CLOSED)

Manifest pin (`rust/upstream-core/Cargo.toml`):

```toml
quinn = { version = "=0.11.7", default-features = false, features = ["runtime-tokio", "rustls-ring"] }
h3 = { version = "=0.0.8", default-features = false }
h3-quinn = { version = "=0.0.10", default-features = false }
```

Resolved graph (`cargo metadata --locked`, `cargo tree -e normal -p
mosdns-upstream-core --locked`):

| Crate | Resolved | License | rust-version | Verdict |
|---|---|---|---|---|
| `quinn` | 0.11.7 | MIT OR Apache-2.0 | 1.71 | PASS — exact pin |
| `quinn-proto` | 0.11.18 | MIT OR Apache-2.0 | 1.85 | PASS — `= MSRV`, not above |
| `quinn-udp` | 0.5.15 | MIT OR Apache-2.0 | 1.85 | PASS — `= MSRV`, not above |
| `h3` | 0.0.8 | MIT | 1.70 | PASS — exact pin |
| `h3-quinn` | 0.0.10 | MIT | 1.70 | PASS — exact pin, depends on quinn 0.11.7 |
| `rustls` (single line) | 0.23.45 | Apache-2.0 OR ISC OR MIT | 1.71 | PASS — same 0.23 line as `TlsPolicy` |
| `ring` | 0.17.14 | Apache-2.0 AND ISC | 1.66.0 | PASS — sole crypto provider |

1. **License**: all five audited crates in MIT/Apache-2.0/ISC families,
   compatible with `GPL-3.0-only`. PASS on the lock.
2. **MSRV ≤ 1.85**: `cargo metadata --locked` over the complete resolved
   graph (including lockfile-only entries) reports **no** package whose
   `rust-version` exceeds 1.85. The binding constraints sit exactly at the
   ceiling (`quinn-proto 0.11.18` and `quinn-udp 0.5.15` declare 1.85).
   PASS on the lock.
3. **TLS-stack alignment**: exactly one rustls line, 0.23.45 — the same
   0.23 major the workspace `TlsPolicy` builds from. PASS on the lock.
4. **Client-only feature selection**: activated quinn features are exactly
   `runtime-tokio` + `rustls-ring` (→ `ring` + `rustls 0.23.45`).
   `cargo tree -e features` shows zero `platform-verifier` and zero
   `aws-lc-rs` lines anywhere. `futures-io 0.3.34` enters only as
   h3-quinn's own non-optional transport adapter (`futures-io` is a
   hard dependency of h3-quinn 0.0.10, not a selected feature), alongside
   its `futures-core/util/task/sink/channel` family — no async runtime,
   no server/listener surface. h3/h3-quinn stay at default features
   (no `tracing`, no `datagram`). PASS on the lock.
5. **Tree shape**: `cargo tree -e normal -p mosdns-upstream-core` shows
   `h3 0.0.8`, `h3-quinn 0.0.10 → quinn 0.11.7 + h3`, and
   `quinn 0.11.7 → quinn-proto 0.11.18 + quinn-udp 0.5.15`; the remaining
   new lock entries (`lru-slab`, `rustc-hash`, `socket2`, `cfg_aliases`,
   `fastrand`, `rand`/`rand_core`/`rand_pcg`, `chacha20`, `cpufeatures`,
   `web-time`, `slab`, `memchr`, `futures-*`, `pin-utils`) are their
   transitive closure only, nothing else new. `#![forbid(unsafe_code)]`
   still applies to this crate's own code. PASS.

Gate CLOSED: the Slice 0 exit gate is satisfied. Slice 1 socket code may
proceed when authorized. The previously noted unanchored upstream SHAs
(P2-2, ruled non-blocking) remain as-is; the lock pins above are the
reproducible record.
