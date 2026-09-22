# W2 execution state

Recorded before `task.py start` on 2026-09-22.

## Worktree boundary

Branch: `rust` (`origin/rust` at `e40563d4a45949916400218a637dfc3a3cca6cdc`).

Pre-existing dirty paths, preserved verbatim and excluded from task commits:

- `.trellis/spec/backend/quality-guidelines.md`
- `.trellis/workflow.md`
- `.trellis/workspace/tom/index.md`
- `.trellis/workspace/tom/journal-1.md`
- `.trellis/.DS_Store`
- `.trellis/tasks/.DS_Store`
- `.trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/.DS_Store`

## Frozen-input digests

The following deterministic tree digest was computed over every regular file
under each scope. For each scope, paths were sorted and the digest input was
`repository-relative-path`, file byte length, and the file's SHA-256, separated by NUL
bytes and terminated by LF. No baseline runner or benchmark was executed.

| Scope | Files | Bytes | Tree SHA-256 |
|---|---:|---:|---|
| `.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline` | 2378 | 60755103 | `538733b18dd516df21c27b998830c97ceae760c85702bda07391d2984a82634e` |
| `tests/phase5a-baseline` | 10 | 41922 | `34678ca9acd6072ad2a01d429fd513d70e7dd48c899fbfc7cb7e4df160cc6b2d` |

These scopes are read-only acceptance inputs for W2. The W1 archive and the
historical baseline remain outside the implementation allowlists.

## Automation authorization

The executor context is `codex_01a0c7fa-e0ea-7f52-adb0-f3789e7a7bdb`.
The designated reviewer is `codex://threads/01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3`
(`成为001号 reviewer`). A native `mcp__codex_app__read_thread` probe resolved
that exact idle reviewer thread before the authorization snapshot was written.
The authorization snapshot covers only `Slice 0`, `Slice 1`, `Slice 2`, and
`Slice 3`; no task start or automation activation had occurred when this
record was created.

## Slice 1 implementation evidence

The Slice 1 implementation was committed and pushed on 2026-09-22:

- Parent: `245404663f2039d1782fa3e611c9f0512f6fa9c4`
- Pushed head: `f113590b55324476b222fe31f58df27696834125`
- Commit: `feat(phase5a): add native cache execution adapter`
- Changed paths are limited to `rust/native-host/**`, the one
  `mosdns-native-host -> mosdns-cache-core` lockfile edge, the narrow
  `rust/dns-core` response metadata helper, and this task's evidence/docs.
- No `sequence-core`, Go/runtime, listener configuration, baseline archive, or
  `tests/phase5a-baseline` path was changed.

RED/GREEN and review-scope evidence:

- Pre-implementation adapter tests failed at compile time for missing
  `CacheTestClock`/`NativeCacheAdapter` symbols.
- The final local run passed native-host all targets (15 unit, 8 adapter, 2
  strict-config, 5 TCP, 4 UDP), cache-core 11, dns-core 55 unit plus 47
  integration, and sequence-core 65 tests, all with `--locked`.
- Native-host all-targets clippy with `-D warnings`, workspace format check,
  task validation, `git diff --check`, and the native-host normal dependency
  tree inspection passed. No native-host source reference to runtime/cgo,
  ABI handles, or cache ABI entry points was found.
- The implementation uses one HostAssembly-owned adapter and a shared
  canonical `ExecutionMachine` driver; UDP and TCP only admit, frame, send,
  and supervise requests.

## Slice 2 implementation evidence

Slice 2 remains bounded to `rust/native-host/**` and this task directory. The
frozen baseline and historical archive were not edited. The implementation
adds strict W2 compilation in `rust/native-host/src/config.rs`, config RED/
GREEN coverage in `rust/native-host/tests/slice2_config.rs`, and controlled
loopback UDP correctness coverage in `rust/native-host/tests/w2_cache.rs`.

- RED first failed at the unchanged W1-only plugin-count gate for the exact
  `tests/phase5a-baseline/configs/cache.yaml` input.
- GREEN accepts only the reviewed four-plugin graph: integer cache values
  `size: 64` and `lazy_cache_ttl: 0`, UDP upstream/listener, audit false, and
  the exact flat `$cache -> $forward` sequence. W1 UDP/TCP compilation remains
  accepted and W2 rejects wrong counts, roles, fields, values, types, refs,
  order, repeats, audit, TCP and non-UDP upstream before assembly.
- Native-host all-targets locked tests passed 15 unit, 8 Slice 1 adapter, 5
  config, 5 W1 TCP, 4 W1 UDP and 5 W2 tests. Cache-core (11), dns-core (55
  unit plus 47 integration) and sequence-core (65) affected tests passed.
  Native-host all-targets clippy with `-D warnings`, workspace format,
  `git diff --check`, and task-local checks passed.
- The W2 UDP tests record the controlled-upstream counter assertions for cold
  non-singleflight and warm zero-delta behavior, deterministic expiry and
  ref forwarding, ID/buffer isolation, EDNS/non-IN bypass, valid upstream
  SERVFAIL, invalid/TC/OPT/mismatched response no-publication, cancellation,
  shutdown and rebind. No Linux remote run, benchmark, VM, deployment or
  production cutover was performed.

### Slice 2 reviewer remediation

The first Slice 2 review at pushed head
`5075d12adeab809bb34928a53af5503218375f35` returned `SLICE 2: FAIL` with two
scoped findings:

- P1-1 required direct checks against both immutable rows in
  `tests/phase5a-baseline/workloads/cache.jsonl`, with separate cold and warm
  lifecycles and a post-prefill counter barrier. `w2_cache.rs` now reads and
  asserts the exact `cache-a`/`cache-b` qname and answer fields, then runs each
  row through its own cold lifecycle and a fresh warm lifecycle.
- P2-1 required proof that cancellation occurs after the request reaches the
  upstream. The shutdown test now uses a controlled blackhole upstream and
  waits for its receipt counter before cancelling, then asserts no response,
  no cache publication and successful listener rebind.

The remediation is limited to `rust/native-host/tests/w2_cache.rs` plus this
task evidence. Post-remediation native-host tests/clippy, format, and diff
checks pass; no remote Linux, benchmark, VM, deployment or production work
was run.

## Slice 3 Linux and final evidence

The exact pushed Slice 2 head reviewed for the Linux gate was
`b558d153cad9ad8e3ffaf18a6e2dde82329e32e0`. A fresh archive artifact was
created at `/tmp/mosdns-phase5a-slice3.qP7kFM/repo` on `mosdns-rust` (Linux
amd64: `Linux mosdns-rust 7.0.9-x64v3-xanmod1 ... x86_64`) and removed after
validation. The remote toolchains were `rustc 1.95.0`, `cargo 1.95.0`,
`go1.26.4 linux/amd64`, and Python 3.13.5. The remote host initially lacked
rustfmt/clippy; only those standard rustup components were installed.

Commands and results:

- `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo && cargo fmt
  --manifest-path rust/Cargo.toml --all -- --check'`: PASS.
- `cargo test -p mosdns-native-host --all-targets --locked`: PASS, with 15
  unit, 8 adapter, 5 config, 5 W1 TCP, 4 W1 UDP, and 6 W2 tests. The W2
  tests directly consumed both immutable `workloads/cache.jsonl` rows and
  asserted cold counts, warm zero-delta counts, expiry/reforward,
  EDNS/non-IN bypass, invalid/TC/OPT/mismatched no-publication, cancellation
  after upstream receipt, and shutdown/rebind with no late write.
- `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets
  --locked`: PASS. The first run in the fresh `/tmp` checkout hit a linker
  `SIGBUS` while linking upstream tests because the 2 GiB tmpfs filled. After
  removing only that checkout's target and retrying with
  `CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-target CARGO_BUILD_JOBS=1`,
  every workspace target passed. The exact retry command was
  `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo &&
  CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-target CARGO_BUILD_JOBS=1
  cargo test --manifest-path rust/Cargo.toml --workspace --all-targets
  --locked'`. The exact corresponding workspace clippy command was
  `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo &&
  CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-target CARGO_BUILD_JOBS=1
  cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets
  --locked -- -D warnings'`; PASS.
- `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo && go test ./...'`:
  PASS. The exact static-library command was
  `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo &&
  CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-target CARGO_BUILD_JOBS=1
  scripts/build-rust-cache.sh'`: PASS; the resulting library was copied to
  `rust/target/release/libmosdns_runtime.a`. The exact focused commands were
  `ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3.qP7kFM/repo && CGO_ENABLED=1
  go test -tags mosdns_rust ./plugin/executable/cache ./pkg/cache
  ./pkg/query_context ./pkg/server_handler -count=1'` and the same command
  with `go test -race`; PASS in both modes.

The exact-tree digest comparison was run both locally and in the fresh remote
artifact. Each matched the frozen values above: the archived baseline had
2378 files / 60755103 bytes /
`538733b18dd516df21c27b998830c97ceae760c85702bda07391d2984a82634e`,
and `tests/phase5a-baseline` had 10 files / 41922 bytes /
`34678ca9acd6072ad2a01d429fd513d70e7dd48c899fbfc7cb7e4df160cc6b2d`.

The post-review focused memory-safety gate also passed locally: with
`rustc 1.99.0-nightly`,
`MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-ignore-leaks' rustup run nightly cargo
miri test --manifest-path rust/cache-core/Cargo.toml --all-targets` passed 11/11,
and
`MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-ignore-leaks' rustup run nightly cargo
miri test --manifest-path rust/runtime/Cargo.toml --test abi_contract` passed
20/20.
Miri emitted only known crossbeam-epoch/tagptr integer-pointer warnings and no
Miri error. No benchmark, VM, deployment, production/default cutover, W3,
sanitizer campaign, or final reviewer PASS has been recorded yet. The selected
reviewer request is pending; normal Trellis finish/archive remains outside
this run.

## Received final result and closure

The prior paragraph records the state before the final response. The selected
reviewer subsequently returned `SLICE 3: PASS` / `FINAL: PASS` for
`c6b7f80226a13fa9ab81945fe782fe2c7c6bb5d0`, with all A1–A8 accepted.
The executor run is `authorized_scope_complete`, all four units passed and all
findings closed. The user then authorized archive after independent checks.
See `closure-review.md`; no additional product or benchmark work was performed.
