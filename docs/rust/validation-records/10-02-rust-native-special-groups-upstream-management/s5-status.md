# S5 HTTP management and capability validation

**Status:** Exact-source review round 0 returned `P1-1 / FINAL: FAIL`; the
same-ID round 1 review passed the exact repaired range, and the formal Trellis
controller advanced to Slice 6. S5 is accepted. S7 and the separate cumulative
review remain required.

## Exact-source review round 0

The reviewer checked `d43da11ababb0104c84b827f96ffd6262e00ac7e` →
`b290a6de8e9dedeebb34f339fd36a17d55cd00bc` and found **P1-1**: all six S5
mutation paths compile from `profile.base_dir/config.yaml` instead of the root
configuration path used to create the running host. A nonstandard root filename
can therefore make mutations fail or publish a candidate compiled against an
unrelated decoy file. The reviewer requires retaining the real startup path,
using it for all six mutation paths, and adding an HTTP regression with a
non-`config.yaml` root and distinct decoy. Full result: [round 0](s5-review-round-0-result.md).

The original 375/0/3, fmt, and Clippy results remain evidence for the reviewed
candidate only; they do not cover this finding and do not constitute an S5 PASS.

## P1-1 remediation — exact-source review PASS

`ManagedProfile` now retains the canonical root YAML path for file-backed
managed hosts. Startup checks that this path belongs directly to the managed
state root, candidate compilation carries it into each replacement profile,
and all six HTTP mutation paths use that profile path. Profiles compiled only
from in-memory YAML have no persistent root path and fail closed with HTTP 503.

The new regression starts the host from `gateway.yaml` while a distinct valid
but unmanaged `config.yaml` decoy remains beside it. Its first HTTP group write
reproduced the original failure (`400`, candidate requires the frozen managed
opt-in). After the fix, two consecutive group writes succeeded across runtime
snapshots, the decoy bytes stayed unchanged, and restart loaded the committed
group state. See the [RED](evidence/s5-p1-1-root-config-red.log) and
[GREEN](evidence/s5-p1-1-root-config-green1.log) logs.

The repaired isolated-host suite passed **376 tests, 0 failed, 3 ignored**;
ignored subprocess probes are invoked by their parent recovery/ownership tests.
The suite ran serially to avoid a transient UDP port collision observed in the
first default-parallel run. That initial `cache_http` failure is preserved in
[the failed suite log](evidence/s5-p1-1-native-host-suite.log); the isolated
failing test passed on rerun, and the full serial suite passed in
[the final suite log](evidence/s5-p1-1-native-host-suite-serial.log).
[Formatting](evidence/s5-p1-1-fmt-final.log) and strict all-target
[Clippy](evidence/s5-p1-1-clippy-final.log) passed. The exact repaired-source
manifest covers 140 inputs (126 Rust files), with zero local/remote/audit-tree
differences. The independent review object is parent
`b290a6de8e9dedeebb34f339fd36a17d55cd00bc` →
`63b836b7ba0d0046cbb28763950f206b320cf201` (tree
`d17c3940e79dc1a52104fb9652e099f2fd607010`). The round-1 request is
[recorded here](s5-review-round-1-request.md) and has been sent in the dedicated
C2C review conversation; its SHA-256 is
`bc19d247f2f8d7a8b25ad8e52ba073f973f2a5efb543cf37297362ae6ebea339`, and the
manifest SHA-256 is
`c8c7ccee51735ee298497c60a36dd3077c7e8490802fdbba7d369631e6402d66`. At test
and audit creation, the real branch still pointed at
`79d93ae1b3b3253a2d09563444b251aad18eb5df` and its index remained clean. The
dedicated reviewer returned [explicit round-1 PASS](s5-review-round-1-result.md)
for the exact remediation object; the formal run recorded the bound submission,
closed `P1-1`, and advanced to Slice 6 without changing the frozen authorization.

## Implemented behavior

- `/api/v1/capabilities` reports the actual native profile and supported
  protocols/features. Group inventory, create/update/delete, upstream tags/config/
  runtime, diversion-source catalogs and manual rules use the committed runtime
  snapshot and existing response shapes.
- Group create accepts slot `0` or an omitted slot and chooses the first unused
  identity starting at 50. Nonzero slots retain update semantics. Group deletion
  removes owned catalog/manual/override state and leaves external source files.
- Group, upstream, diversion and manual-rule writes compile and validate the
  complete managed candidate from the canonical root YAML path carried by the
  running managed profile before publication. Client disconnect does not
  release a transaction after admission. Cache flush/import and concurrent
  managed writes serialize with configuration publication.
- Unsupported enabled upstream options and protocols return a client error before
  persistence. Disabled unsupported data remains visible and preserved. An
  empty, all-disabled or duplicate-tag upstream configuration is rejected before
  persistence. An occupied listener returns HTTP 409 and preserves the active
  generation and canonical group file. Capabilities for a non-managed profile
  disable write endpoints, whose attempted mutations return HTTP 400.

## Isolated validation

All Rust commands ran through SSH alias `mosdns-rust` in
`/root/mosdns-rust-special-groups-20261003/src`, using
`/root/mosdns-rust-special-groups-20261003/target-candidate`.

- [`cargo test -p mosdns-native-host`](evidence/s5-native-host-suite7.log):
  **375 passed, 0 failed, 3 ignored** across
  29 test binaries. The ignored subprocess probes are explicitly invoked by
  their parent recovery/ownership tests. The full run includes live UDP/TCP DNS,
  HTTP mutation, cache lifecycle and the 400,000-record audit-load test.
- [`cargo fmt --all --check`](evidence/s5-cargo-fmt-check5.log): passed.
- [`cargo clippy -p mosdns-native-host --all-targets -- -D warnings`](evidence/s5-clippy-all-targets3.log): passed.
- Focused HTTP matrix: [all 21 supervisor integration tests](evidence/s5-supervisor-suite2.log) pass.
- The final repaired [140-file source manifest](evidence/s5-p1-1-source-manifest.json)
  contains 126 Rust inputs and reports identical local and isolated-host
  SHA-256 values with no differences. The earlier
  [initial-candidate manifest](evidence/s5-source-manifest.json) is retained as
  evidence for the first S5 candidate; it predates the P1-1 remediation and is
  not the accepted-source manifest. The final review request binds the repaired
  paths to its exact audit commit object.
- Public HTTP RED/GREEN and focused results are retained in `evidence/s5-*`:
  slot allocation, upstream rejection and preservation, route/method status,
  disabled-profile capability flags, group CRUD, manual POST, diversion rename,
  occupied-listener conflict, disconnect ownership and concurrent writes.
- SIGKILL/restart recovery across marker, swap, retirement and cleanup is covered
  by `evidence/s5-sigkill-publication-green1.log` and the full native-host suite.

## Failed attempts retained

The evidence directory preserves the initial formatter failure, failed native
host suites, Clippy failure and first slot-test setup failure. The repairs were:
restore the test-only persistence accessor; update the old method matrix to use
an actually unsupported method after POST became implemented; attach the
capabilities fixture to the managed writer and provide its required audit flag;
publish subprocess readiness by atomic rename to avoid observing a partial file;
and document the three intentionally long lifecycle tests with the repository's
targeted `too_many_lines` allowance. The final suite, formatter and Clippy logs
are separate from those failed attempts.

## Remaining task gates

S5 is accepted after the same-ID exact-source review PASS on the repaired
range. Slice 6's maintained Vue workflow, Slice 7's whole-chain proof, and the
separate cumulative review from `79d93ae1b3b3253a2d09563444b251aad18eb5df` to
the final tested source remain open.
