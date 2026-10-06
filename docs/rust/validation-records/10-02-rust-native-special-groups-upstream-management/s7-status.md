# S7 whole-chain validation status

Validation evidence collected 2026-10-03; record updated 2026-10-04.

**State:** S1–S7 isolated validation and exact-source stage reviews passed.
S7 remediation head `3eaeabdd75f74c4eb86db96b46f466afbe4aa3be` received explicit
`FINAL: PASS` on 2026-10-04. The separate cumulative review from baseline
`79d93ae1b3b3253a2d09563444b251aad18eb5df` has now independently passed on
`ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc` after same-ID supplier persistence remediation.
See the [final cumulative result](cumulative-review-remediation-round-2-result.md).
The original S7 measurements below remain historical stage evidence.

The first S7 review returned `P2-1` / `FINAL: FAIL` because this record repeated
conflicting evidence-manifest digests. That immutable reviewed object and failure
remain preserved. The duplicate was removed, the retained digest verified against
the committed manifest, and the same-ID re-review passed. See the
[first result](s7-review-round-1-result.md),
[remediation request](s7-review-remediation-round-1-request.md), and
[remediation PASS](s7-review-remediation-round-1-result.md).

The frozen S7 scope is the whole managed-group chain: native compilation,
ordered routing, independent custom listeners, cache invalidation and fencing,
HTTP mutation, retained unsupported state, final audit provenance, shutdown,
and the existing Vue audit view. The accepted review parent is S6 object
`eec6c4447e221ea304893dd8ddaf5b761a35cdb3` (tree
`44a41971f7729270c8d171a9c58612d2734243a7`). The review object and request are
recorded separately after this validation record so the request can name the
exact immutable head and tree.

## Validation

- `cargo fmt --all -- --check` and strict workspace all-target Clippy passed in
  the final isolated rerun with explicit command and exit-code records:
  [final static checks](evidence/s7-final-static-checks.log).
- `cargo test -p mosdns-native-host --locked -- --test-threads=1`: 378 passed,
  0 failed, 3 subprocess probe entrypoints ignored by direct discovery and
  invoked by their parent tests. See
  [native-host suite](evidence/s7-native-host-suite-retry.log).
- `cargo test --workspace --lib --locked -- --test-threads=1`: 407 passed,
  0 failed, 3 parent-invoked subprocess probes ignored. See
  [workspace libraries](evidence/s7-workspace-libs-retry.log).
- `cargo test --workspace --locked -- --test-threads=1`: 1,179 passed,
  0 failed, 3 parent-invoked subprocess probes ignored across 79 test targets.
  See the [full workspace log](evidence/s7-workspace-full-retry.log).
- The native-host binary build passed. The separately selected DoT and DoH
  peer suites passed 27/27 and 39/39. See the
  [build](evidence/s7-native-build-retry.log),
  [DoT](evidence/s7-dot-peer-retry.log), and
  [DoH](evidence/s7-doh-peer-retry.log) logs.
- The maintained and compatibility Vue builds passed with 619 and 612 modules.
  Both retain the existing Vite chunk-size advisory above 500 kB. Logs:
  [maintained](evidence/s7-maintained-ui-build.log) and
  [compatibility](evidence/s7-compatibility-ui-build.log).
- The integrated native-host test exercises ordered overlapping groups,
  custom-port listeners, custom-only exclusion from main routing, default
  fall-through, group-local cache hits, selective cache invalidation after an
  upstream save, rejection without publishing an unsupported QUIC candidate,
  actual upstream/rule/group audit provenance, and audit-off metric retention.
  See [`special_groups_whole_chain.rs`](../../../../rust/native-host/tests/special_groups_whole_chain.rs).
- The isolated live proof used loopback controlled DNS peers and high ports
  only. It verifies UDP and TCP group routing, main/custom listener isolation,
  cache hit metrics, an HTTP upstream replacement, preservation of an
  unaffected group's warm cache, HTTP 400 for unsupported QUIC with the last
  committed generation still active, and final audit provenance. The Vue query
  detail was opened and captured at
  [the browser screenshot](evidence/s7-browser-run/s7-audit-detail.jpg). There
  was no public DNS or port 53 use. Machine-readable results and traces are in
  [`s7-browser-run`](evidence/s7-browser-run/).
- The S7 exact tested-source closure contains 527 inputs: all 526 accepted S6
  inputs plus the new whole-chain test. Local and isolated-host SHA-256 values
  match for all 527. Four generated HTML entrypoints have new query-string
  build timestamps; inspection confirms only those timestamp values changed.
  See the [source manifest](evidence/s7-source-manifest.json) and
  [evidence manifest](evidence/s7-evidence-manifest.json).
  Source-map SHA-256 is
  `5d598287cf6cc6ad63b1627c322f8deafe5f364bfdc20792ecfc10ae6b667c7a` and its
  manifest-file SHA-256 is
  `cd32bcce468bd898c2d98e2023b74e51f5280af3068b6b5f244572ad968256c2`.
  Evidence-map SHA-256 is
  `d1e65b0e06e2fbcef282aca9c805f7693e82e90eee261df9f43de1ed9572437f` and its
  manifest-file SHA-256 is
  `113a4d870fa4b111b06f894a2ca6cbe60986ee17e29fe0e6c61ef144c89e2274`.

## Retained failed attempt

The first full-workspace attempt exhausted the isolated host's 40 GB filesystem
while linking and failed with `Bus error` / `No space left on device`; the first
separate DoT and DoH attempts also stopped under that disk condition. Their
logs remain at [initial full workspace](evidence/s7-workspace-full-final.log),
[initial DoT](evidence/s7-dot-peer-final.log), and
[initial DoH](evidence/s7-doh-peer-final.log). After removing only this task's
disposable `target-candidate/debug` build cache, the full serial rerun used one
Cargo job, disabled incremental compilation and disabled dev/test debug info.
Every listed retry completed successfully. The failed attempts are retained as
failures and are not counted as passing evidence.

All builds and tests ran on the isolated SSH host in
`/root/mosdns-rust-special-groups-20261003`, using its task-owned source,
`target-candidate`, and evidence directories. The real branch remains at
`79d93ae1b3b3253a2d09563444b251aad18eb5df`; the real index tree remains
`65b21ecc1a3e3fe14f50da3d49e0cb153c1d53ac`.
