# Pre-implementation performance freeze

Frozen: 2026-09-25, before Rust runtime changes. This is a measurement plan,
not a benchmark result or a performance claim. The final candidate's source
commit and executable digest will be recorded in a separate candidate identity
file after it is built and before any official probe run.

## Candidates and immutable inputs

| Input | Frozen identity |
|---|---|
| Rust-before source | `605c30577b79d397b5695618dbd2980e550ca6f3` |
| Rust-before Linux amd64 binary | `/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust`; SHA-256 `370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`; verified present on `mosdns-rust` on 2026-09-25 |
| Rust-before source/build-graph comparison | Full tracked `rust/` subtree is identical between `605c30577b79d397b5695618dbd2980e550ca6f3` and pre-change baseline `64d9cf51468ced016f813bcde84d08337a1c1872`: both have tree `b6f33a14fcbdba8b0d652932e2a9905f4cc405cb`, and `git diff --exit-code OLD..BASE -- rust` is empty. This covers every workspace member, source/test file, and Cargo manifest, not only `native-host`. |
| Rust lockfile | `rust/Cargo.lock`, SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca` |
| Rust build metadata | The tracked Rust subtree contains no `build.rs`, repository `.cargo/config*`, or `rust-toolchain*` at either revision. The lockfile content hash is identical at both revisions. The archived old-binary record identifies source commit `605c30577b79d397b5695618dbd2980e550ca6f3`, the locked release build command, and `rustc 1.95.0 (59807616e 2026-04-14)`; the frozen candidate build uses the same Rust toolchain and locked workspace graph. |
| Phase 5A runner | `scripts/run-phase5a-baseline.sh`, SHA-256 `dd9749238cf917d1360f33fd732cca48c4ae7906ab76d1cb8f5f941e10e1e3d8` |
| Helper source | `tests/phase5a-baseline/cmd/phase5a-baseline/main.go`, SHA-256 `2dec4788eaa3dad251061e380cbd8c32c9122285b852acf8b9a0b2eb81f64da1` |
| Linux helper binary | `/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/phase5a-baseline-helper`; version `phase5a-baseline-helper/v8`; SHA-256 `df4515d7045ddcfd30139b0407e8e9933277c1560137d71ee0f7360d7d537065` |

The current runner digest differs from the first native comparison's frozen
runner digest (`18c1e29c…`). Therefore this task will not invoke the archived
official matrix driver as if it were unchanged. It will use the currently
hashed runner in `pilot` mode, with this task's frozen low/moderate rate plan;
the runner's output hash list and the separate candidate identity file will be
retained for every attempt. The archived v2 comparison remains historical
evidence only. Its 800/1000 QPS attempts, sender-shortfall stages, and
recovery-labelled health checks are not used to prove overhead, overload, or
recovery here.

The Rust-before binary provenance is recorded in the archived
`09-23-rust-phase5a-first-native-performance/research/slice1-build-identities-v1.txt`:
it was built from the full source archive for commit `605c30577b79d397b5695618dbd2980e550ca6f3`
with `cargo build --manifest-path …/rust/Cargo.toml --package
mosdns-native-host --bin mosdns --release --locked`, using
`rustc 1.95.0 (59807616e 2026-04-14)`, and its binary SHA-256 is the value
frozen above. The archived source archive SHA-256 is
`28c3a4e9eb9d0f78c4e0da2dce253a9a1fe4aa81a983897b1287533581086e3b`.
Comparing the complete tracked `rust/` trees and lockfile establishes that
every local workspace source, manifest, and locked dependency input affecting
`mosdns-native-host` is unchanged through the pre-change baseline commit
`64d9cf51468ced016f813bcde84d08337a1c1872`. This establishes source/build-graph
equivalence for the frozen Rust-before comparator; it is not a claim of
bit-for-bit reproducibility across build directories.

The frozen fixture inputs are the existing Phase 5A files:

| Scenario input | SHA-256 |
|---|---|
| `tests/phase5a-baseline/configs/forward-udp.yaml` | `f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729` |
| `tests/phase5a-baseline/configs/forward-tcp.yaml` | `1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1` |
| `tests/phase5a-baseline/configs/cache.yaml` | `7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7` |
| `tests/phase5a-baseline/configs/routing.yaml` | `66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651` |
| `tests/phase5a-baseline/workloads/forward.jsonl` | `32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2` |
| `tests/phase5a-baseline/workloads/cache.jsonl` | `7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed` |
| `tests/phase5a-baseline/workloads/routing.jsonl` | `dbb582dc9d623af54e501639b7a52538b1f1d9e493fa0f0f6a4f4714ea0176c1` |

The audit-on configuration is a byte-preserving overlay of each frozen YAML:
replace its single `enable_audit: false` value with `enable_audit: true` and
change no other byte. Expected overlay SHA-256 values are:

| Audit-on overlay | SHA-256 |
|---|---|
| W1 TCP | `72db879e9fbee4fb87400b54da31dfd41f82f6766dbe6081ab3d4690e42df24c` |
| W2 | `c973586aee0f0381d96256afb305ef773f240a4aab7033303990c013d4cf7158` |
| W3 | `0cc96555e135529a9acb7b49dbbefdf94d1d8b9e8cf2c2d4c37a980e099521b5` |

## Environment and run matrix

- Host: SSH alias `mosdns-rust`; Linux amd64, kernel `7.0.9-x64v3-xanmod1`,
  two online CPUs (`0-1`), Rust/Cargo `1.95.0`, 4,102,578,176 bytes total
  memory, ext4 root, 1,024 open-file limit. Inventory captured at
  `2026-09-25T11:54:57Z`: load averages `0.07 / 0.07 / 0.08`, 3,406,796 KiB
  available memory, 12,330,476 KiB root space free. SUT uses CPU 0; runner,
  generator, and fixtures use CPU 1.
- The existing `/usr/local/bin/mosdns` process (PID 425 at inventory time) is
  left untouched. Test listeners/upstreams use only the frozen high loopback
  ports in the fixture YAML. Port availability is checked before each attempt;
  production host `mos` is never contacted.
- Scenarios: W1 TCP forwarding, W2 cold/warm cache, and W3 routing. Each
  variant starts a fresh process per scenario/repetition. W2 cold and warm
  phases stay in one process as required by the same-process warm lifecycle.
- The runner's required pilot ladder is fixed at 200, 300, 350, and 400 QPS,
  followed by its 200 QPS health-check stage; each stage lasts 3,000 ms, the
  request deadline is 500 ms, late drain is 100 ms, and W2 TTL/safety margin
  are 30,000/500 ms. All offered stages remain at or below the previously
  valid 400 QPS range. Only the 200 and 400 QPS observations are primary
  comparisons. Intermediate stages are supporting low/moderate-load data; the
  final health-check is not recovery evidence, and no stage is called a
  saturation point or capacity bound.
- Three repetitions use an interleaved, balanced candidate order:

  | Repetition | Order |
  |---|---|
  | 1 | Rust-before audit-off → Rust-after audit-off → Rust-after audit-on |
  | 2 | Rust-after audit-off → Rust-after audit-on → Rust-before audit-off |
  | 3 | Rust-after audit-on → Rust-before audit-off → Rust-after audit-off |

- Rust-before always uses the frozen binary above with audit disabled. Both
  Rust-after variants use the same post-implementation binary and differ only
  in the listener YAML overlay. This is not a Go/Rust capacity comparison.
- The release build on the VM uses
  `cargo build --manifest-path rust/Cargo.toml --package mosdns-native-host
  --bin mosdns --release --locked`, with no extra `RUSTFLAGS`. Record the exact
  candidate commit, `rustc --version`, build command, and SHA-256 before any
  candidate run.

## Validity, resource capture, and budgets

The helper's scheduled, sent, received, correct-on-time, wrong-response,
protocol-error, transport-error, timeout, and sender-shortfall fields are kept
for every attempt. A primary stage is valid only when the sender meets every
scheduled slot; sent, received, and correct-on-time counts reconcile exactly;
wrong, late, protocol, transport, timeout, and drop counts are zero; and the
W1/W2/W3 upstream, cache, or route-event oracle passes. Invalid attempts stay
in the result tree with their reason. There is no selective replacement run
and no filtering of failures out of latency percentiles. Fewer than three
valid pairs for a scenario/rate makes that comparison inconclusive.

For each valid stage retain raw request ledgers, stage JSON, counter/event
files, input hashes, runner/helper/binary hashes, process CPU user+system ticks,
peak RSS, FD peaks, CPU affinity, and host load. CPU comes from the VM's
100-Hz `/proc` process clock and is explicitly quantized; a zero-tick window
means “below one tick,” not zero CPU. RSS/FD sampling is process-local. The
results are a short, one-core, controlled-loopback regression probe; they do
not establish multi-core scaling, a capacity ceiling, long-term retention
growth, service recovery, or production performance.

Budgets are frozen before the Rust-after candidate is built:

- Correctness and useful throughput are hard gates: at each primary 200/400
  QPS point all three paired repetitions must be valid and every scheduled
  request must receive the exact expected response before the 500 ms deadline.
  No correctness, latency, or throughput result may be salvaged from an
  invalid attempt.
- For p95 and p99 separately, compare paired repetition deltas for Rust-after
  audit-off versus Rust-before audit-off, and Rust-after audit-on versus
  Rust-after audit-off. A repeatable regression is material only if the
  three-pair median exceeds `max(10%, 2 × control relative MAD)` and at least
  two of three paired deltas exceed that same threshold. Here control relative
  MAD is median absolute deviation divided by the control median. Report p50,
  all per-repetition values, and ranges as context; no “performance win” is
  inferred from passing this regression guard.
- CPU per correct-on-time query has a `max(25%, 2 × control relative MAD)`
  median guardrail, with at least two of three pairs exceeding it before a
  regression is called repeatable. If 100-Hz resolution cannot distinguish a
  candidate from the control, mark CPU comparison inconclusive and retain the
  per-stage upper bound.
- For audit-off RSS, a repeatable increase beyond
  `max(8 MiB, 25% of control median, control max-minus-min)` is a regression.
  For audit-on versus audit-off, a peak increase above 32 MiB is a regression
  for this short probe. These RSS guards do not replace the 100,000-record
  retention bound or prove soak behavior.
- Any repeatable regression beyond these limits blocks Slice 3 review until
  repaired within the approved scope or recorded as a separate corrective
  task. Invalid or noisy measurements remain inconclusive rather than being
  converted into a pass.

Before the first candidate run, compute and preserve a candidate-identity
supplement with the new source commit and executable SHA, recheck every input
hash, and record the final runner/helper digests. This manifest and its
sidecar SHA-256 are not edited based on candidate results.
