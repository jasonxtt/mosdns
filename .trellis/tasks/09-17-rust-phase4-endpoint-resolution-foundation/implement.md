# Implementation plan — Rust Phase 4 endpoint resolution foundation

Planning status: implementation authorized; execute only the currently
authorized slice. Each slice uses one behavior at a time: RED public contract
test -> GREEN minimum implementation -> bounded refactor -> focused checks ->
controller diff/check -> web review -> STOP.

## Routing and worktree rules

- Controller: current Codex pane `w6:p4`.
- Executor: user-selected Claude Herdr pane `w6:p2`.
- Reviewer/root gate: user-selected ChatGPT web conversation **建立评审上下文**
  in project **mosdns**:
  <https://chatgpt.com/g/g-p-6a186dc3cda481918bb08784cae25b26-mosdns/c/6aaba177-ce44-83ee-b52a-68b5d84be272>.
  The controller is authorized to send review context and GitHub revisions there
  without another confirmation. Validate the selected executor and reviewer
  route before dispatch/review; never substitute another reviewer silently.
- Preserve all unrelated dirty files, especially `.DS_Store` and concurrent
  migration-document edits. Never reset/rebase/force-push, switch branches, use
  `git add -A`, or stage by directory.
- No production deployment, `mosdns`/`mos-test` service mutation, port 53, host
  wiring, or next-slice progression follows automatically from a review PASS.

## Slice 0 — pure DNS resolver wire contract and dependency gate

- Add RED public tests for A/AAAA query construction, normalized names, EDNS
  size, exact question/ID matching, malformed bounds, rcodes, TC, A/AAAA,
  bounded CNAME chains/loops, first-address selection, and effective TTL.
- Add the minimum pure `dns-core` resolver codec; reuse existing bounded wire
  walkers where contracts match, but do not weaken existing query/response APIs.
- Add exact `getrandom = 0.4.3` to the normal upstream-core graph only when the
  source/license/MSRV/feature audit remains as researched; inject deterministic
  IDs in tests.
- Checks: focused dns-core tests, dependency trees/metadata, fmt, clippy, diff.
- Allowed: `rust/dns-core/**`, exact Cargo manifests/lock, resolver contract
  tests, this task's evidence. Forbidden: socket/cache/composition behavior.

## Slice 1 — typed target, policy, numeric bypass, and publication model

- RED tests define `AddressFamily`, validated target/bootstrap/policy inputs,
  config-version mapping, numeric bypass, expiry metadata, and typed errors.
- Implement public resolver types and reuse/factor the existing normalized DNS
  identity contract without changing DoT/DoH identity behavior.
- Add state-machine tests for fresh/expired entries and atomic publication with
  an injected clock; no network yet.
- Allowed: focused upstream resolver module/re-exports/tests and minimal shared
  identity helper. Forbidden: UDP I/O and transport wiring.

## Slice 2 — bounded bootstrap UDP exchange

- RED loopback tests prove ephemeral connected UDP, immediate send, one-second
  retransmission, valid response, ignored mismatch diagnostics, terminal matching
  errors, caller cancel/deadline, owner close, future abort, and no late success.
- Implement one caller-runtime UDP exchange using the original absolute deadline
  and current cancellation/lifecycle vocabulary. No private timeout, system DNS,
  TCP fallback, detached task, or target-upstream socket.
- Use Tokio paused time for retransmission/deadline tests; fixtures use random
  high loopback ports and explicit synchronization rather than wall sleeps.
- Allowed: resolver UDP module/tests and necessary exact Tokio dev feature.

## Slice 3 — single-flight cache, TTL refresh, and close/drain

- RED concurrency tests prove one leader per target, bounded waiters, independent
  waiter cancellation/deadlines, leader abort recovery, complete-only publish,
  TTL floor/ceiling, refresh replacement, refresh failure preservation, expired
  value rejection, generation wakeup, and idempotent close/drain.
- Implement mutex+Notify generation state with RAII cleanup. Never await under a
  synchronous lock and never spawn hidden background refresh.
- Run focused repeated tests and concurrency stress sufficient to expose missed
  wakeups/late publication; no timing sleeps.
- Allowed: resolver state/lifecycle tests and minimum reuse of existing
  `Lifecycle`. Forbidden: pools and generic reusable transport state.

## Slice 4 — endpoint composition and full contract matrix

- RED composition tests prove one original deadline across resolution and later
  handoff; UDP/TCP receive the numeric address; DoT keeps original SNI identity;
  DoH keeps URL authority/path/identity; numeric `dial_addr` bypasses resolver.
- Add only explicit helper/constructor composition needed by current Rust
  primitives. Do not add YAML parsing, host/plugin ownership, live sequence
  wiring, proxy/socket options, pool/reuse/pipeline, QUIC/HTTP3, or listeners.
- Re-run all existing upstream-core tests to prove no regression in numeric and
  secure foundations.

## Slice 5 — final quality and isolated Linux evidence

- Verify PRD AC1-AC8 and the full error/deviation/deferred matrix.
- Record dependency/license/MSRV evidence accurately. If Rust 1.85 is not
  installed, do not claim it ran; use resolver metadata plus CI/toolchain facts.
- Required local gates:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo tree --manifest-path rust/Cargo.toml --workspace -e features --locked
python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation
git diff --check
```

- Obtain Linux loopback evidence through repository CI or an explicitly isolated
  test-host run. Do not mutate an installed service or production paths.
- Controller inspects exact status, changed paths, full diff, checks, branch,
  commit and push identity, then sends the verified revision to the web
  conversation above. Record explicit P0/P1 disposition and PASS/FAIL. PASS closes only Slice 5; task
  archive and the dual-stack follow-up each require separate user authorization.

## Required follow-up after this task

Create a separate Trellis task for native dual-stack endpoint resolution and
address selection/racing. Its planning must cover concurrent/staggered A+AAAA,
Happy Eyeballs policy, address ordering, per-family failure memory, multi-address
cache shape, QUIC/HTTP3 interaction, and compatibility with `bootstrap_version`.
Do not lose this item merely because the single-family foundation passes.

## Slice 0 evidence — pure resolver wire codec (2026-09-17)

Scope: `rust/dns-core/**` only. No socket, cache, timer, runtime, composition,
Go/cgo, or config change; `rust/upstream-core` untouched.

### Changed paths

- `rust/dns-core/src/resolver.rs` (new, 589 lines): `AddressFamily`,
  `QueryIdSource` trait, `CnameChainPolicy`, `SelectedAddress`,
  `ResolverWireError`, `build_resolver_query`, `parse_resolver_response`,
  plus the private bounded `read_name` walker and `decode_address`.
- `rust/dns-core/tests/resolver_wire.rs` (new, 1028 lines): 27 public contract
  tests.
- `rust/dns-core/src/lib.rs`: module declaration, crate-root re-exports, and
  the module-surface doc list. No existing item changed or removed.

### RED

`cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --test
resolver_wire --locked` with only the module file, the `lib.rs` re-exports, and
the test file present:

```text
error[E0432]: unresolved imports `resolver::AddressFamily`, ... (10 items)
error: could not compile `mosdns-dns-core` (lib) due to 1 previous error
EXIT=101
```

### GREEN

Same command after the implementation: `test result: ok. 27 passed; 0 failed`.

Four fixture/implementation defects were found and fixed while turning this
green, each by tightening the real assertion rather than relaxing the codec:

1. `read_name` added a spurious `+1` to the offset after following a
   compression pointer, so every compressed answer owner was mis-framed.
2. Three test fixtures computed answer offsets relative to the header instead of
   the post-question answer section start.
3. `cname_chain` pointed each record's owner at itself instead of at the
   previous record's rdata; the chain's first owner must point at the QNAME.
4. One AAAA case echoed an A question, so it failed correlation before reaching
   the rdata-length check it meant to exercise.

### Discriminating-test check

A trap-protected mutation sweep over `rust/dns-core/src/resolver.rs` (backup
under `rust/target/`, restored on EXIT/INT/TERM) confirmed the suite catches:
EDNS payload changes, RD cleared, TC check removal, RCODE check removal, ID
check removal, question check removal, TTL clamp removal, chain-TTL reset per
link, last-match-wins selection, chain cycle check removal, chain link-bound
disable, name length limit relaxation, label charset relaxation, authority walk
removal, and opcode check removal.

Three mutations initially survived and two real test gaps were closed as a
result (the CNAME link must be the min-TTL source, not the address RR; a
second in-wire-order match must not win; the authority/additional framing still
has to be walked). Two remaining survivors are semantically equivalent guards:
`rdata.len() != family.rdata_len()` is redundant because `decode_address`
already rejects every non-exact width, and the CNAME `chain_ttl` accumulator is
redundant with the address branch's own `chain_ttl.min(ttl)`.

### Checks (all run from `/Users/tom/github/mosdns-rust`, branch `rust`)

| Command | Result |
| --- | --- |
| `cargo test ... -p mosdns-dns-core --all-targets --all-features --locked` | 53 + 1 + 2 + 27 + 1 passed, 0 failed |
| `cargo test ... --workspace --all-targets --all-features --locked` | 25 suites ok, no failures |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | clean |
| `cargo clippy ... -p mosdns-dns-core --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo clippy ... --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo tree ... -p mosdns-dns-core -e normal --locked` | only `mosdns-dns-core`; no external crate |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation` | passed |
| `git diff --check` | clean |

### Dependency gate

No manifest or lockfile change was made. `getrandom 0.4.3` was therefore **not
added**, because this slice genuinely has no production randomness: the
injected `QueryIdSource` is the only ID path, and the crate's normal dependency
tree is empty. The audit below records that the researched dependency was
eligible rather than rejected.

- Exact version and checksum: `0.4.3`,
  `300e883d756b2e4ec94e02791f39b04b522276138852cfc41d9fb7e904106099`
  (`rust/Cargo.lock`).
- License: `MIT OR Apache-2.0`; edition 2024; `rust-version = "1.85"` — within
  the workspace MSRV.
- Features: `std`, `sys_rng`, `wasm_js` only, and none is a default; a
  `getrandom = "0.4.3"` dependency with `default-features = false` resolves to
  the same lock entry and off-graph `libc`/`r-efi` target dependencies. Public
  API: `getrandom::fill(&mut [u8])`.
- Lock graph: present only via `uuid 1.24.0 <- moka 0.12.12 <- cache-core <-
  runtime`. `mosdns-dns-core` has no reference to `getrandom`.

### MSRV

`rustc 1.95.0` (Homebrew, system default) and `rustc 1.96.0` (rustup stable) are
installed; **Rust 1.85 was not installed and was not run**, so this slice makes
no claim that it compiled under 1.85. MSRV evidence is indirect: every resolved
package declares at most `rust-version = 1.85` (12 packages exactly at 1.85,
none above it) per `cargo metadata` plus this lockfile, and the change uses no
API newer than the crate's existing code.

### Limitations carried forward

- This slice has no socket, retransmission, cache, refresh, or lifecycle
  behavior; those are Slice 1+.
- CNAME chain links are not required to appear in chain order, and a chain link
  that is not ultimately resolved is tolerated rather than typed as a dangling
  chain. Both behaviors are more permissive than the design prose and are only
  reachable on an untrusted response; the selection returned is unaffected.
- Duplicate CNAME targets are collapsed by the chain set, so pathological
  repeated-target chains are bounded but not individually diagnosed.
