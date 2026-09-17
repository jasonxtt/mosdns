# Implementation plan — Rust Phase 4 endpoint resolution foundation

Planning status: implementation authorized. The Slice headings below are
behavioral milestones in the reviewed plan. In Herdr mode, one selected Claude
executor receives the active task's full remaining scope (starting at Slice 1
after the reviewed Slice 0) and may progress through those milestones in one
handoff; the controller performs one complete-task diff/check and web-review
gate. In native sub-agent or MCP DSH modes, retain one behavior/slice per job
and the corresponding handoff gate. Every milestone still uses RED public
contract test -> GREEN minimum implementation -> bounded refactor -> focused
checks.

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
  wiring, work outside this active task, or task archive follows automatically
  from a review PASS. Herdr's single assignment may include all remaining
  planned slices in this task; it must still stop at the task boundary.

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

## Slice 0 remediation round 2 — reviewer FAIL on bd869b5 (2026-09-17)

Two in-scope P1 findings; fixes confined to `rust/dns-core/**`. RED public
regressions were added first for each.

### P1-1 — TC is terminal immediately after correlation

TC was still evaluated after the answer/authority/additional walk and after the
extended RCODE was read. A correlated TC=1 packet therefore reported
`Rcode(2)` (header SERVFAIL) or `Rcode(16)` (OPT BADVERS) instead of
`Truncated`, and a TC=1 packet whose declared RR framing was actually cut
reported `Malformed`.

Fix: QR, opcode, ID and exact question correlation are unchanged and still run
first; TC is now checked immediately after the question check, before any record
is walked and before any extended RCODE is read. A correlated truncated response
is terminal, so a cut or hostile body can never be reported as `Malformed` or a
negative answer. Wrong-question precedence is preserved.

New tests: `truncated_wins_over_rcode_for_a_correlated_response` (TC=1 +
matching question + header SERVFAIL, + OPT BADVERS, + plain NOERROR all return
`Truncated`), `truncated_is_reported_before_a_cut_record_walk` (a TC=1 packet
declaring nine answers and holding one returns `Truncated`, while the same body
with TC clear still returns `Malformed`), and
`wrong_question_precedence_is_preserved_for_truncated_packets` (wrong question,
wrong ID and QR-clear still win over TC).

### P1-2 — an OPT record's owner must be the DNS root

The additional-section branch accepted any `TYPE 41` record as an OPT and read
its TTL upper byte as the extended RCODE, so a non-root `TYPE 41` record could
either be accepted or inject an extended RCODE.

Fix: the branch now requires `is_root_name(&owner)` — the expanded owner must be
the single root label `[0]`, which is the only root encoding `read_name` returns.
A non-root `TYPE 41` record is `Malformed` and never contributes an extended
RCODE. The existing checks are retained: an OPT in the authority section is
`Malformed`, a duplicate OPT is `Malformed`, and an unparsable OPT RDLENGTH
still fails the framing walk.

New test: `rejects_an_opt_record_whose_owner_is_not_the_root` — a multi-label
non-root owner with extended RCODE 0 and with extended RCODE 1 (BADVERS) both
return `Malformed`, a single-label non-root owner returns `Malformed`, and a
root owner is still accepted with its extended RCODE still applying
(`Rcode(16)` for extended RCODE 1).

### Checks (branch `rust`, `/Users/tom/github/mosdns-rust`)

| Command | Result |
| --- | --- |
| `cargo test … -p mosdns-dns-core --all-targets --all-features --locked` | 53 + 1 + 2 + 42 + 1 passed, 0 failed |
| `cargo test … --workspace --all-targets --all-features --locked` | 25 suites ok, no failures |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | clean |
| `cargo clippy … -p mosdns-dns-core --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo clippy … --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo tree … -p mosdns-dns-core -e normal --locked` | still only `mosdns-dns-core` |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation` | passed |
| `git diff --check` | clean |

Mutation check (backup under `rust/target/`, restored on EXIT/INT/TERM): moving
the TC check back after the answer walk, moving it back after the RCODE check,
removing the root-owner requirement, and tolerating a non-root OPT are each
caught by the new tests.

### Limitations carried forward

- Previously recorded limitations are unchanged: the CNAME state budget fails
  closed with `InvalidCnameChain` on a crafted answer section, a name owned by
  several CNAME records resolves by message order, and there is still no socket,
  retransmission, cache, refresh, deadline, or lifecycle behavior (Slice 1+).
- A TC=1 response is never inspected for records, so its body framing is not
  validated; that is deliberate, since a truncated body is by definition
  incomplete and this codec performs no TCP retry.

## Full remaining task — Herdr single assignment (2026-09-17)

Executed after the reviewed Slice 0 PASS at `e1339f1`. In Herdr mode the selected
executor receives the active task's full remaining scope, so Slice 1 through
Slice 5 below are the reviewed plan's behavioral milestones, each built
RED public contract test -> GREEN minimum implementation -> bounded refactor.

### Changed paths

- `rust/upstream-core/src/resolver/mod.rs` (new, 739 lines): typed model —
  `AddressFamily` (re-exported from `dns-core`, Slice 0 wire contract untouched),
  `Clock`/`SystemClock`, `ConfigVersion`, `ResolutionTarget`,
  `BootstrapEndpoint`, `ResolutionPolicy`, `ResolvedDestination`,
  `PublishedTarget`, `ResolverState`, `ResolverError`, `resolve_numeric`.
- `rust/upstream-core/src/resolver/bootstrap.rs` (new, 276 lines): the bounded
  connected UDP bootstrap exchange and the injectable `ResolutionIdSource`.
- `rust/upstream-core/src/resolver/owner.rs` (new, 593 lines):
  `BootstrapResolver` (single-flight generation, lifecycle/reuse, one absolute
  deadline), `ResolverComposition`, `ResolvedUpstream`.
- `rust/upstream-core/src/lib.rs`: module declaration, crate-root re-exports, and
  one additive `Lifecycle::register_owned` wrapper over the existing
  `register_shared` admission gate. No existing item changed or removed.
- `rust/upstream-core/tests/resolver_slice1.rs` .. `resolver_slice5.rs` (new,
  423 + 235 + 449 + 261 + 370 lines).

`rust/dns-core/**`, `rust/Cargo.toml`, and `rust/Cargo.lock` are **unchanged**:
no manifest or lockfile edit was needed, and no DNS library was added.

### Slice 1 — typed model, config mapping, numeric bypass, deterministic expiry

RED: `resolver_slice1.rs` failed with unresolved
`resolver::{AddressFamily, Clock, ConfigVersion, ...}` imports while the module
was an empty scaffold. GREEN: 17 public contract tests pass, covering the
`0/4 -> IPv4`, `6 -> IPv6` mapping and rejection of every other version; numeric
dial bypass with no expiry; hostname normalization (case, trailing root dot,
label rules, 253-octet limit); numeric bootstrap `host:port` validation;
policy defaults and bounds; deterministic injected-clock expiry at the exact
boundary; publication with identity separation; and the state model (fresh
serving, expired value retained as evidence but never served, failed refresh
never replacing a published value).

### Slice 2 — bounded connected UDP bootstrap exchange

RED: `resolver_slice2.rs` failed on the unresolved `BootstrapResolver` import.
GREEN: 7 tests pass on loopback fixtures with random high ports and no wall
sleep, covering a real one-address resolution, TTL clamps, an already-expired
deadline, caller cancellation, numeric-target bypass, and close/drain; with
in-crate tests for the blocking bind/connect/send/receive paths.

### Slice 3 — single-flight, refresh, last-known-good, close

RED: `resolver_slice3.rs` failed against the not-yet-existing single-flight
behavior. GREEN: 10 tests pass with a hand-advanced clock and a query-counting
fixture: a fresh publication is served with exactly one bootstrap query; an
expired publication triggers exactly one real refresh; a failed refresh
publishes nothing and leaves only a typed diagnostic; concurrent callers do not
fan out into more than one query; an aborted leader lets a later caller lead a
fresh generation (the RAII `LeaderGuard` completes the abandoned generation);
close is idempotent and rejects later work; an expired deadline and a closed
owner each produce **zero** datagrams; and a caller's short deadline is honored
rather than replaced by a private timeout.

### Slice 4 — composition into the existing boundaries

RED first. GREEN: 7 tests pass, proving a resolved destination feeds the numeric
`Endpoint`, `ResolvedUpstream` and the secure constructors; DoT keeps the
caller's original SNI identity; DoH keeps the original URL authority, path, and
query; a numeric `dial_addr` bypasses the resolver; and resolution plus handoff
share one original absolute deadline with no fresh budget.

### Slice 5 — end-to-end boundary and the full error matrix

RED first. GREEN: 6 tests pass. The central test resolves a hostname through an
explicitly bound loopback bootstrap socket whose answer is the *second*,
separately bound loopback target socket's own address and port, then drives a
**real** plain UDP exchange through the existing `Upstream` transport to that
target and asserts the returned wire, the response ID, the target port, and the
unchanged original deadline. The composition is exercised end to end, not
mocked. The remaining tests cover the full constructor error matrix, a
non-response datagram being ignored rather than treated as an answer, a terminal
negative rcode publishing nothing, close/drain, and identity separation.

A bounded test deadline (`STEP`) wraps every await and every fixture loop has a
`FIXTURE_TIMEOUT`, so no test can hang: an earlier fixture that answered with an
unroutable address was replaced by the two-socket reachable design above.

### Checks (branch `rust`, `/Users/tom/github/mosdns-rust`)

| Command | Result |
| --- | --- |
| `cargo test … --workspace --all-targets --all-features --locked` | 30 suites ok, 0 failures |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | clean |
| `cargo clippy … --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo tree … -p mosdns-upstream-core -e normal --locked` | no new DNS library |
| `cargo tree … -p mosdns-dns-core -e normal --locked` | still only `mosdns-dns-core` |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation` | passed |
| `git diff --check` | clean |

Mutation checks (each with a trap-restored copy of the resolver module) confirm
the suites catch: a swapped config-version family, a removed TTL clamp, a
removed numeric bypass, an unenforced expiry, a stale value served as fresh, a
removed bootstrap-numeric check, a disabled single-flight leader gate, a removed
leader-abort recovery guard, a resolver dialing the wrong port, a publication
that ignores the resolved address, and a resolution that drops the resolved TTL.
One survivor — the outer deadline re-check in the owner — is redundant with the
exchange's own `check_at`, which enforces the same deadline, so the observable
contract is unchanged.

### Limitations

- No Rust 1.85 run: `rustc 1.95.0` (Homebrew) is the only installed toolchain,
  so MSRV is evidenced indirectly (every resolved package declares at most
  `rust-version = 1.85`; none is above it). No claim is made that this compiled
  under 1.85.
- No Linux evidence: this assignment ran on macOS only. The resolver's exchange
  is loopback-tested locally; Linux loopback evidence is still outstanding and
  is required before final task closure, per the task's own gate.
- The default `SteppingIdSource` is deterministic and **not** unpredictable; a
  production caller must supply an unpredictable `ResolutionIdSource` through
  `BootstrapResolver::with_id_source`. This is documented on the type.
- Native dual-stack resolution / Happy Eyeballs remains the reviewed future
  follow-up: `AddressFamily` is single-family (`0/4` = IPv4, `6` = IPv6) and no
  address racing was added.
- Connection pooling/reuse/pipeline, socket policy/proxy, QUIC/HTTP3, server
  listeners, YAML/host/API/WebUI wiring, and Phase 6 retirement remain out of
  scope and untouched.

## Controller audit remediation (2026-09-17)

The controller's independent audit of `27d9f37` found eight contract gaps.
All eight were addressed with RED tests first. Commit `e81dec8`.

### P1 — single-flight generations carry a token

`SingleFlight` now assigns a monotonic `GenerationId` to every generation, and a
waiter attaches to the specific token it observed. A waiter whose own generation
finished can no longer race a new leader's `try_lead`, see `running` with no
result recorded yet, and adopt the newer generation as its own: it returns
`AlreadyResolving` instead. `complete` also ignores a superseded token, so a
stale leader cannot overwrite a newer generation's state.

Tests: `a_waiter_attached_to_a_finished_generation_cannot_adopt_the_next_one`,
`a_waiter_attached_to_a_finished_generation_receives_its_result`,
`a_superseded_leader_cannot_complete_a_newer_generation` (crate-internal, where
the token interleaving is directly constructible), plus
`a_waiter_never_attaches_to_a_later_generation` and
`concurrent_waiters_observe_only_their_own_generation` at the public boundary.

### P2 — publication goes through the lifecycle linearization gate

Both the DNS-result path and the numeric bypass now call the existing
`Lifecycle::commit_final_response` under the same lock as registration and
`begin_close`. Owner close, caller cancellation, and the caller's original
absolute deadline are evaluated there, so a close that wins the gate prevents
publication and a committed publication cannot be reversed by a later close.
Test: `owner_close_wins_over_publication`.

### P3 — the numeric bypass honors the terminal controls

The numeric path previously published without checking the caller's budget or
close. It now applies the same controls before publishing and through the same
gate. Tests: `numeric_bypass_honors_an_expired_deadline`,
`numeric_bypass_honors_caller_cancellation`, `numeric_bypass_honors_owner_close`.

### P4 — the owner's TTL policy governs the wire parse

The exchange hard-coded `dns_core::CnameChainPolicy::default()`, so `dns-core`'s
own clamp silently overrode a custom `ResolutionPolicy` floor or ceiling.
`ResolutionPolicy::dns_core_policy()` now derives an equivalent codec policy
(same min/max; the codec's CNAME link bound keeps its reviewed default, since it
is not a resolver policy knob), and the exchange parses under it.
Tests: `a_custom_policy_bound_reaches_the_wire_parse` and
`a_custom_policy_ceiling_reaches_the_wire_parse`, each configured so the
resolver's bound differs from the `dns-core` default and only the resolver's own
bound can produce the asserted TTL.

### P5 — destination family is validated

`ResolvedDestination::new` returns the typed `FamilyMismatch` when the address is
not in the declared family, instead of publishing a mismatched pair. The
infallible `new_literal` keeps a debug assertion. Test:
`a_destination_cannot_disagree_with_its_family`.

### P6 — no wall-clock sleeps in the resolver tests

`resolver_slice3.rs` had `sleep(100ms)`, `sleep(10ms)`, and a wall-clock
`elapsed()` assertion. All three are gone: the "no traffic" assertions now drop
the resolver and then read the fixture's own counter, the abort-recovery test
uses a cooperative `yield_now` spin, and the short-deadline test asserts the
typed outcome plus no publication rather than an elapsed bound. The blocking
`std::sync::Barrier` in the remediation file was replaced by `tokio::sync::
Barrier`, because blocking a `current_thread` runtime also deadlocked it.
Every await in the resolver tests is now wrapped in a file-wide bounded helper.

Two blocking-fixture `elapsed()` bounds remain, in `resolver_slice5.rs` and
`resolver_remediation.rs`. They bound only a mock server's own drain loop, the
same pattern as the reviewed Slice 1/2 fixtures; they order nothing in the
tests, and the client side of every exchange is deadline-bounded.

### P7 — unpredictable IDs by default

`OsIdSource` draws from the operating system through `getrandom` and refuses to
construct when no entropy is available (`UnpredictableIdsUnavailable`) rather
than degrading to a predictable sequence. `getrandom 0.4.3` (MIT OR
Apache-2.0, MSRV 1.85) becomes a direct dependency; the lockfile change is a
single line adding it to `mosdns-upstream-core`'s dependency list, because it
was already resolved transitively via uuid/moka. The deterministic stepping
source is reachable only through the explicitly named
`with_deterministic_ids_for_tests`, and `uses_unpredictable_ids()` exposes which
path was taken without revealing any ID. Tests:
`the_default_construction_uses_unpredictable_ids` (crate-internal),
`the_production_default_id_source_is_not_deterministic`, and
`an_injected_deterministic_source_is_explicit_and_test_only`.

### P8 — exchange correlation coverage

Added focused tests for a wrong-ID datagram being ignored while the real answer
still wins, a correlated terminal rcode being typed, and the retransmission
being byte-identical to the original query. No real transport was weakened.

### Honest limitations

- No mutation sweep was completed for this remediation round. An earlier
  in-place sweep destroyed the working source when it was interrupted mid-
  restore, and was recovered from `e81dec8`; the user then directed that
  expensive mutation sweeps not be re-run, so these eight fixes are evidenced by
  the tests above rather than by mutation-kill results. An isolated-worktree
  sweep was also interrupted before producing results.
- The P3 early control check is redundant with the lifecycle gate for the
  numeric path; removing both is caught by the tests, removing only the early
  check is not. It is kept as defense in depth.
- Rust 1.85 is still not installed and was not run; MSRV evidence remains
  indirect (every resolved package declares at most 1.85, none above).
- No Linux evidence: this round ran on macOS only. The task's Linux loopback
  gate remains outstanding before final closure.
- Native dual-stack resolution / Happy Eyeballs is still the reviewed future
  follow-up; `AddressFamily` remains single-family.

### Checks (branch `rust`, `/Users/tom/github/mosdns-rust`, commit e81dec8)

| Command | Result |
| --- | --- |
| resolver tests | slice1 17, slice2 7, slice3 10, slice4 7, slice5 6, remediation 15 — all passed |
| `cargo test … --workspace --all-targets --all-features --locked` | 31 suites ok, 0 failures |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | clean |
| `cargo clippy … --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-endpoint-resolution-foundation` | passed |
| `git diff --check` | clean |
