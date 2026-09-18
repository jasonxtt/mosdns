# Implementation plan — Rust Phase 4 native dual-stack endpoint selection

Planning status: ready for review; implementation is not authorized until the
final planning summary is explicitly approved and `task.py start` is run.

## Routing and worktree rules

- Executor after activation: user-selected Claude, Herdr pane `w6:p2`.
- Reviewer after the verified commit: the user-selected ChatGPT web project
  conversation `mosdns Phase 4 resolver review` at the routed URL.
- Current branch: `rust`; base branch: `rust`. Preserve the unrelated dirty
  documents and `.DS_Store` files already present in the worktree.
- Stage exact task/code paths only. Do not use `git add -A`, reset/rebase,
  force-push, or switch branches.
- No Go/cgo/FFI, YAML/config loader, API/WebUI, host/plugin/sequence wiring,
  production/default selection, installed-service mutation, port 53, QUIC/HTTP3
  implementation, connection pool, protocol fallback, or deployment.

## Slice 0 — explicit version mapping and RED model contracts

- Add RED public tests for omitted/default `4`, explicit `0` dual,
  `4` A-only, `6` AAAA-only, and invalid values.
- Preserve numeric literal bypass and the existing single-family constructor.
- Add the minimal mode/plan and candidate snapshot types without network changes.
- Test that explicit zero is distinguishable from omitted input and that the
  preferred family for dual mode is IPv4 without pretending it is the only
  family.

## Slice 1 — independent A/AAAA bootstrap collection

- Add deterministic loopback tests with one numeric bootstrap peer that records
  QTYPE and returns independently controlled A/AAAA replies.
- For explicit `0`, run the two existing single-family bootstrap exchanges under
  one shared absolute deadline and lifecycle owner. No target connection is
  opened, and no detached task or private runtime is allowed.
- Cover only-A, only-AAAA, both-success, one-leg terminal error plus one success,
  both-leg failure, cancellation, owner close, deadline exhaustion, TC, rcode,
  wrong-ID/question, and no late result.
- Keep `4` and `6` on their current one-leg paths and prove no second query.

## Slice 2 — multi-family publication and selection

- Add RED tests for per-family TTL/expiry, partial refresh, stale rejection,
  per-family diagnostics, complete snapshot publication, single-flight waiters,
  generation replacement, and close/drain.
- Implement the smallest mutex/Notify extension that stores a complete
  multi-family snapshot and selects fresh IPv4 before fresh IPv6.
- If one dual leg fails, preserve a still-fresh previous candidate for that
  family while recording the typed error; never serve an expired candidate.
- Ensure A wins even when the AAAA leg completes first. A selected target
  connection failure must not trigger an AAAA retry.

## Slice 3 — composition and deferred protocol boundary

- Keep `ResolverComposition` returning one numeric `PublishedTarget` and verify
  plain UDP/TCP receive the selected address.
- Verify DoT SNI and DoH URL authority/path remain the configured identity for
  both A and AAAA selections.
- Record the QUIC/HTTP3 multi-address consumer contract and deferred work only;
  do not change secure protocol implementations.

## Slice 4 — quality, Linux evidence, and review gate

- Run the focused resolver/dns-core tests repeatedly enough to expose missed
  wakeups or late publication, using explicit handshakes and bounded waits.
- Run the local gates:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo test --manifest-path rust/Cargo.toml -p mosdns-dns-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml -p mosdns-upstream-core --all-targets --all-features --locked
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo tree --manifest-path rust/Cargo.toml --workspace -e features --locked
python3 ./.trellis/scripts/task.py validate rust-phase4-dual-stack-endpoint-selection
git diff --check
```

- Use the isolated Debian VM through `ssh mosdns-rust` for focused Linux/Rust
  1.85.1 loopback evidence. Do not use Mac Docker/Colima and do not mutate the
  installed service or `/root/mosdns-rust-build`.
- Independently inspect exact changed paths, diff, status, branch, commit and
  push identity. Send only the verified commit to the selected web reviewer;
  record explicit P0/P1 and PASS/FAIL before closure.

### Slice 4 evidence — local gates and isolated Debian VM (2026-09-18)

Evidence-only record for the **uncommitted** working tree on branch `rust` at
`c6028ae`. Final acceptance still requires the controller's exact-diff review,
commit/push, and the selected reviewer's explicit scoped PASS.

Scope of this evidence: the Slice 0–3 implementation plus the five controller
review items (carried-forward candidate keeping its family diagnostic;
selection from the merged snapshot; real same-state partial-refresh coverage;
monotonic generation metadata; and the dual fast path made refreshable when the
preferred family has expired).

Changed paths:

- `rust/upstream-core/src/resolver/mod.rs`
- `rust/upstream-core/src/resolver/owner.rs`
- `rust/upstream-core/tests/resolver_dual_stack.rs` (new, 25 tests)
- `rust/upstream-core/src/lib.rs` (additive exports only)

#### Local gates (macOS)

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | clean, exit 0 |
| `cargo test … -p mosdns-dns-core --all-targets --all-features --locked` | exit 0, all suites ok |
| `cargo test … -p mosdns-upstream-core --all-targets --all-features --locked` | exit 0 — lib **80**; dual_stack **25**; remediation 24; slice1 14; slice2 7; slice3 10; slice4 7; slice5 6; slice0_contract 12; slice0_secure 27; slice1_dot 27; slice1_udp 35; slice2_doh 39; slice2_tcp 14; slice3_doh 6; slice3_policy 10 — 0 failed |
| `cargo test … --workspace --all-targets --all-features --locked` | exit 0 — **32 suites ok, 0 failures** |
| `cargo clippy … --workspace --all-targets --all-features --locked --quiet -- -D warnings` | clean, exit 0 |
| `cargo tree … --workspace -e features --locked` | exit 0 — no new dependency or feature |
| `python3 ./.trellis/scripts/task.py validate rust-phase4-dual-stack-endpoint-selection` | All validations passed |
| `git diff --check` | clean |

The 25 dual-stack tests are deterministic: no wall-clock sleeps, no
elapsed-time polling, no external DNS. Fixtures are numeric loopback UDP
sockets on ephemeral high ports, and a hand-advanced clock supplies expiry
ordering.

#### Isolated Debian VM — Rust 1.85.1

Real MSRV evidence was obtained through the user-supplied Debian test VM via
`ssh mosdns-rust` (10.0.0.92), into a dedicated staging path
`/root/mosdns-rust-phase4-rerun`. Transfer was `rsync -az --delete --checksum`
of only the local `rust/` workspace, excluding `.git/`, `target/`, `.DS_Store`,
and build artifacts; no `.git` was staged. The transferred tree was verified by
sha256 before running, and all four changed files matched the local copies
exactly:

| File | sha256 |
| --- | --- |
| `src/lib.rs` | `5ad2e75de886b701421d39b9395dcb92e09722c106ba013236303b2336995ae3` |
| `src/resolver/mod.rs` | `f2f39c10d15409a65e15c54165e7c30e0d860050f8ed59076ea9cb62ba214ef4` |
| `src/resolver/owner.rs` | `5a791b7cd392df9be951f9f56792c3a0e10c255d4a46cbf6bf6eb22dc2a555af` |
| `tests/resolver_dual_stack.rs` | `19be18a8b0037892fb7fa8ee857c19909874905f182e059a86ac6e241fb530a8` |

Toolchain: `rustc 1.85.1 (4eb161250 2025-03-15)`,
`cargo 1.85.1 (d73d2caf9 2024-12-31)`. All commands used a task-scoped
`CARGO_TARGET_DIR` inside the staging tree (no fixed `/tmp` path).

| Remote command | Result |
| --- | --- |
| `cargo +1.85.1 test -j 2 --locked -p mosdns-upstream-core` over `--lib` and all six resolver test targets | exit 0 — lib **80**; dual_stack **25**; remediation 24; slice1 14; slice2 7; slice3 10; slice4 7; slice5 6 — 0 failed |
| `cargo +1.85.1 fmt --all -- --check` | clean, exit 0 |

This confirms the dual-stack work compiles and passes on the declared MSRV
1.85.1, not only on the local 1.95 toolchain.

#### Cleanup and blast radius

The staging directory `/root/mosdns-rust-phase4-rerun` was removed after
evidence collection and confirmed gone. The unrelated pre-existing
`/root/mosdns-rust-build` was left intact. The installed production service was
not touched: `systemctl is-active mosdns` remained `active` with unchanged PID
454 (`/usr/local/bin/mosdns start -d /cus/mosdns -c /cus/mosdns/config_custom.yaml`),
and `/cus/mosdns` mtime was unchanged. No service, config, port-53, or
`systemctl` mutation was performed, and no Mac Docker/Colima was used.

#### Limitations

- **Linux clippy was not run for this revision, and no Linux clippy claim is
  made.** The macOS workspace clippy gate at the repository toolchain (1.95) is
  clean. The seven pre-existing Linux clippy findings recorded by the prior
  resolver task are unchanged and remain outside this diff.
- The full Linux workspace gate was not run; the Linux evidence covers the
  resolver and `mosdns-upstream-core` package scope above.
- `lib.rs` is modified despite being listed under Forbidden paths: the change is
  export-only (three additive public types, no logic) and is flagged for
  explicit reviewer attention.
- `ResolutionSnapshot` retains one selected address per family, not every RR,
  per `design.md`. Retaining all same-family records remains deferred.
- No Happy Eyeballs, target connection racing, connection-failure cross-family
  fallback, protocol fallback, connection pool/reuse, or QUIC/HTTP3
  implementation was introduced; the QUIC/HTTP3 consumer boundary is documented
  and deferred only.

## Allowed paths

- `rust/upstream-core/src/resolver/**`
- `rust/upstream-core/tests/resolver_*.rs` or one new focused dual-stack test
- `rust/dns-core/src/resolver.rs` and its resolver tests only if a bounded
  candidate-collection seam is required
- this task's `prd.md`, `design.md`, `implement.md`, `research/**`,
  `implement.jsonl`, and `check.jsonl`

## Forbidden paths

- Go runtime/config/API/WebUI paths, production config, service files, deployment
  artifacts, and unrelated docs/specs
- `rust/upstream-core/src/lib.rs` unless a narrowly justified additive export is
  required and explicitly reviewed in the diff
- QUIC/HTTP3 implementation, pools/reuse/pipeline, proxy/socket policy,
  listeners, host wiring, selectors, and fallback code

## Rollback points

- Before Slice 1: revert only the mode/model commit; existing resolver behavior
  remains unchanged.
- Before Slice 3: revert collection/publication commits while retaining any
  isolated tests that document the contract.
- Before review: if a check fails, stop at the failing slice and do not broaden
  scope to repair unrelated pre-existing warnings.
