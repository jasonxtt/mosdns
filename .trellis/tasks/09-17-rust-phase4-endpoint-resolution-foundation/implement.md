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

## Slice 0 remediation — reviewer FAIL on a9fc802 (2026-09-17)

Three in-scope P1 findings; fixes confined to `rust/dns-core/**`. RED public
regression tests were added for each before the implementation changed.

### P1-1 correlation before terminal matching-query errors

`parse_resolver_response` reported TC and RCODE before it checked the echoed
question, so a reply for a *different* question could be classified as this
query's truncation or negative answer.

Fix: QR, opcode and ID are still checked first (they are the cheapest
correlation), then the full question check runs to completion, and only then are
the 12-bit RCODE and TC evaluated.

New tests: `non_matching_question_is_classified_before_rcode_and_truncation`
(wrong question + SERVFAIL, wrong question + TC=1, wrong question type, absent
question), `matching_question_still_reports_rcode_and_truncation` (the
correlated case still yields `Rcode(2)` / `Truncated`), and
`id_and_opcode_still_precede_question_correlation` (ID mismatch and non-QUERY
opcode still win over a question mismatch).

### P1-2 order-independent CNAME path resolution

The previous selection kept a "reachable set" updated in a single forward pass,
so a valid answer section that lists an address *before* the CNAME introducing
its owner never resolved, and the reported `cname_chain_len` and TTL came from
every visited link rather than the selected path.

Fix: the answer walk now only retains records (`ChainLink`, `AddressRecord`) and
`select_address` performs an explicit iterative traversal over them with a
visited-name path per branch. Reachability no longer depends on record order,
cycles close as soon as a name repeats on the current path, every CNAME owned by
the current name is followed (so branching is resolved by message order, not by
discovery order), and both the effective TTL and `cname_chain_len` come from the
path that reaches the selected record. A hard state budget
`(links + 1) * (max_cname_links + 1) + 1` fails closed against an answer section
crafted to contain exponentially many acyclic paths. No recursion and no
callback into the caller's resolver.

New tests, all built through a declarative `build_answers` helper that fixes
record offsets before emission so an address can genuinely precede its CNAME:

| Test | Proves |
| --- | --- |
| `resolves_a_chain_whose_address_precedes_its_cname_link` | the reversed and forward emissions select identically |
| `resolves_a_two_link_chain_emitted_backwards` | a two-link chain emitted `[A, link1, link0]` resolves, TTL 300, `cname_chain_len` 2 |
| `effective_ttl_uses_only_the_selected_path` | an unrelated 30-second CNAME on a different branch does not move the TTL off 600 or inflate the link count |
| `branching_cname_chooses_the_first_reachable_address_deterministically` | `QNAME -> a`/`QNAME -> b` picks whichever reachable address is first in the message, and swapping the two flips both address and TTL |
| `unreachable_and_cyclic_chains_are_rejected` | an unreachable address is `NoUsableAnswer`; a reachable two-link cycle is `InvalidCnameChain` |

### P1-3 EDNS(0) OPT and the full 12-bit RCODE

The query emits an OPT record, but the response side ignored OPT entirely, so
`BADVERS` (header RCODE 0, extended RCODE 1) was accepted as `NOERROR`.

Fix: `ResolverWireError::Rcode` now carries a `u16`, the additional-section walk
parses the OPT record, and the evaluated RCODE is
`header_low_nibble | (opt_ttl_upper_byte << 4)`. Two OPT records, or an OPT
record in the authority section, are `Malformed`; a malformed OPT RDLENGTH
already failed the framing walk and still does. The OPT contributes no address
and no TTL.

New tests: `reads_the_extended_rcode_from_the_opt_ttl_upper_byte` (BADVERS = 16;
composed value 3 | 2 << 4 = 35; and an extended-RCODE-0 OPT still succeeds with
the answer TTL unaffected), `parses_opt_with_the_executable_version_field`, and
`rejects_a_malformed_opt_record`. The existing `rejects_non_noerror_rcodes` was
updated for the `u16` payload.

### Checks (branch `rust`, `/Users/tom/github/mosdns-rust`)

| Command | Result |
| --- | --- |
| `cargo test … -p mosdns-dns-core --all-targets --all-features --locked` | 53 + 1 + 2 + 38 + 1 passed, 0 failed |
| `cargo test … --workspace --all-targets --all-features --locked` | 25 suites ok, no failures |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | clean |
| `cargo clippy … -p mosdns-dns-core --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo clippy … --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo tree … -p mosdns-dns-core -e normal --locked` | still only `mosdns-dns-core` |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation` | passed |
| `git diff --check` | clean |

Mutation check of the three fixes (backup under `rust/target/`, restored on
EXIT/INT/TERM): evaluating RCODE before the question check, evaluating TC before
the question check, ignoring the OPT extended-RCODE byte, and disabling the
outgoing CNAME edge are each caught by the new tests.

### Limitations carried forward

- The state budget fails closed with `InvalidCnameChain` on a crafted answer
  section with many distinct acyclic paths; a real authoritative answer is far
  below it, but the bound is a safety valve rather than a precise diagnostic.
- A name owned by several CNAME records is ambiguous; the walk follows every
  such record and resolves by message order, which is deterministic but not a
  statement about DNS semantics.
- Still no socket, retransmission, cache, refresh, deadline, or lifecycle
  behavior; that remains Slice 1+.
