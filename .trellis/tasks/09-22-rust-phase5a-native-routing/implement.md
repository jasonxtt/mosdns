# Implementation plan — native W3 routing

Status: reviewed planning (PLANNING: PASS at `7d684c5dee67afeb1eea298813c25b632a6f8438`).
See `research/planning-review.md`. Numeric units below are a proposed execution plan;
there is no user-authorized W3 run. Do not start or send implementation work.

## Pre-start gates

- [ ] User approves this final plan for implementation; reviewer then/first
  supplies explicit planning PASS on its exact pushed SHA. A planning review
  alone does not authorize task.py start.
- [ ] Read AGENTS, project-context/config-notes, handover/rewrite plan, current
  workflow, trellis-before-dev and affected specs (load long rust-migration
  sections directly to avoid truncation).
- [ ] If approved, reuse executor `01a0c7fa-e0ea-7f52-adb0-f3789e7a7bdb` and
  reviewer `01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3` unless user changes them;
  bind actual thread transport and snapshot only Slice 0–3 before start/activate.
- [ ] Preserve existing unrelated workflow/quality/journal/.DS_Store changes;
  exact staging only, session_auto_commit stays false.
- [ ] Capture frozen corpus/baseline digests and initial Git state. Prior W1/W2
  archives and all tests/phase5a-baseline inputs stay read-only.

For each slice: behavior-focused RED, minimum GREEN, focused checks, exact diff
inspection and commit/push, then one complete request to the chosen reviewer.
Include source parent/head, tests, acceptance and stop boundary. Explicit PASS
is required before the next frozen unit; scoped FAIL permits only bounded
remediation and re-review. Do not infer PASS or record a future PASS early.

Common checks: cargo fmt workspace check; task.py validate; git diff --check.
Rust affected packages use `cargo test --manifest-path rust/Cargo.toml -p <pkg>
--all-targets --locked` and corresponding clippy `-- -D warnings`. Existing
checks need not be repeatedly broadened unless changed code justifies it.

## Slice 0 — native matcher adapters and Answer-address inspection

Allowlist: `rust/native-host/**`, `rust/dns-core/**` narrow observer/tests,
`rust/Cargo.lock` path edge only, task directory. matcher-core reused read-only;
no new dependency/version/features, no accepted W3 YAML or live W3 service yet.

- [ ] RED: FullMatcher-backed qname adapter exact/case/trailing-dot/miss and
  ambiguous/non-ASCII wire-label tests. Config-rule helper rejects unsupported
  values; no lossy string conversion.
- [ ] RED/GREEN: narrow DNS observer for Answer A/AAAA; mixed section/multiple
  records/compressed owners/CNAME/OPT, malformed RDATA and trailing truncation.
- [ ] GREEN: native resp_ip + `_true` Matcher adapters, immutable state and
  rebuilt matcher-core IpPrefixList. None/Synthesized/empty answers miss.
- [ ] Native-host, dns-core, matcher-core tests; affected clippy, common checks;
  dependency tree proves matcher-core direct use, no runtime/cgo dependency.
- [ ] `SLICE 0: PASS` required before Slice 1.

## Slice 1 — multiple forwards in the canonical request driver

Allowlist: `rust/native-host/**`, task directory. No sequence-core production
change, upstream transport rewrite, new DNS helper or YAML broadening here.

- [ ] RED/GREEN: validated executable-ID -> upstream owner catalog; W1/W2 use
  one entry. Exercise a test-built W3 ProgramSpec before parser acceptance.
- [ ] Prove matcher -> external dispatch -> resume inside exec list -> Exit
  follows the one machine; unknown executable IDs fail and never fall back.
- [ ] One W3 deadline across both legs; record identical Instant in exchange
  seam; pre-next-leg cancellation/deadline check; transport/validation failure
  stops the chain and replaces stale B with SERVFAIL; valid empty B proceeds.
- [ ] Request isolation with interleaved mocks; shutdown and error cleanup visit
  every catalog owner. Keep W1 TCP/UDP and W2 cache behavior unchanged.
- [ ] Run all native-host and sequence-core tests, native-host clippy, common
  checks; inspect shutdown/drain paths for UDP and TCP.
- [ ] `SLICE 1: PASS` required before Slice 2.

## Slice 2 — strict W3 graph and real routing E2E

Allowlist: `rust/native-host/**`, task directory. Baseline inputs immutable.
Out-of-scope source defects return to the owning slice with reviewer review.

- [ ] RED/GREEN: unchanged W1/W2/W3 YAML fixtures compile; negative matrix covers
  every grammar/graph/field/ref/order/count/transport rejection in PRD.
- [ ] Alternate plugin/upstream names, declaration order, full domain, IP rule
  and numeric endpoints prove no fixture-value or role-name hardcoding.
- [ ] Add `w3_routing.rs` with three controlled UDP upstreams and all frozen
  corpus rows. Exact counts/forbidden legs/order plus final DNS assertions;
  deliberately tampered route evidence rejected even if answer is unchanged.
- [ ] Mixed-route concurrency, first/second-leg stalls and failures, malformed/
  mismatched responses, valid negative B, cancellation after witnessed receipt
  on B and final A/C, no later send/leg, every-owner close and rebind.
- [ ] Run all native-host targets and affected package checks/clippy, common
  checks. No Linux remote run before this slice's explicit PASS.
- [x] `SLICE 2: PASS` returned by the designated reviewer for the exact
  remediation head `33e826ccd89a5db039bfc4d92aaf0593907dd95b` after the
  corpus-driven route oracle, A/C cancellation barrier, question assertions,
  and negative configuration matrix were re-reviewed.

## Slice 3 — Linux correctness evidence and final gate

Allowlist: task directory, `docs/ai/rust-handover.md`,
`docs/rust/feature-coverage.md` for bounded W3 status/evidence only.
No product fixes in evidence-only slice; failures return to their owning slice.

- [x] On the previously designated `ssh mosdns-rust`, exact reviewed source in
  fresh temporary directory, record architecture/toolchains/storage check.
  Use a task-owned disk-backed target; no production service/install change.
- [x] Run native-host W1 UDP/TCP, W2, W3 targets and Rust workspace fmt/tests/
  clippy with --locked, exact commands/env and result summaries. Baseline/corpus
  digests must match before/after; no benchmark runner or local VM.
- [x] Because shared DNS parsing is affected, run existing runtime ABI tests
  and the repository's Linux staticlib + tagged Go cache/matcher tests; full
  `go test ./...` with serial UI build if required. No Go source edits. If unsafe
  or ABI implementation changes become necessary, stop/review scope first and
  add applicable focused memory-safety/race checks; ordinary tests are not Miri.
- [x] Record route counters/order, concurrency/lifecycle results, complete
  commands, any failed attempt/retry and cleanup of task-owned remote artifacts.
- [x] Update coverage/handover: bounded W3 only; 5A basic observability and
  comparable native performance remain open. Keep final reviewer result pending
  until an actual response is received.
- [ ] Commit/push evidence, obtain `FINAL: PASS` for A1–A7, record that actual
  response, report tested/evidence SHA, then stop before finish/archive,
  performance, deployment, additional tasks or full-feature expansion.

### Slice 3 execution evidence (review pending)

- The exact reviewed source `33e826ccd89a5db039bfc4d92aaf0593907dd95b` was
  staged from `git archive` into the fresh remote artifact
  `/tmp/mosdns-phase5a-slice3-w3-33e826c/repo` on `ssh mosdns-rust`; the
  disk-backed Cargo target was `/root/mosdns-phase5a-slice3-w3-target-33e826c`.
  Remote Linux was amd64 (`Linux mosdns-rust 7.0.9-x64v3-xanmod1`, `x86_64`),
  with `rustc/cargo 1.95.0`, `go1.26.4 linux/amd64`, Python 3.13.5,
  rustfmt 1.9.0-stable and clippy 0.1.95. `/tmp` was a 2 GiB tmpfs; all
  compilation used the task-owned target on the 24 GiB root disk with
  `CARGO_BUILD_JOBS=1`.
- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` passed.
  `cargo test --manifest-path rust/native-host/Cargo.toml --all-targets
  --locked` passed 25 unit, 8 Slice 1, 8 config, 5 W1 TCP, 4 W1 UDP, 6 W2
  and 6 W3 tests. The W3 run observed the three corpus paths exactly as
  `A`, `B -> A`, and `B -> C`, with forbidden legs absent; malformed/mismatch,
  failure, concurrent cancellation and listener rebind cases also passed.
- `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets
  --locked` passed every workspace target (including runtime ABI 20, query ABI
  11, valued ABI 6, sequence-core 65, and all upstream-core targets), and the
  corresponding workspace clippy command with `--locked -- -D warnings`
  passed. The source tree digest excluding generated `rust/target` was
  `3256 files / 70540315 bytes / c35d075a9b6b1a793aac98cdc50919095c5261c5cce6ddff790991773f095073`.
- `go test ./...` and `go vet ./...` passed. The Linux staticlib was built with
  `scripts/build-rust-cache.sh` using the disk-backed target and copied only
  into the task checkout's `rust/target/release` for cgo; its SHA-256 was
  `8847a4b6213a5920fb6d2b6fb863a90ee7cda437a5745024dfebce6218544187`.
  Tagged cache/query/server and plugin/pkg matcher tests passed in normal and
  race modes. Tagged data-provider matcher tests passed after excluding the
  existing `plugin/data_provider/matcher_adapter` typed-nil interface case.
- The official tagged data-provider command and its race counterpart were also
  run exactly. Both fail only at
  `TestSlice2RealDomainAdapterRejectsUnsafeRegexp`: the unchanged Go adapter
  returns a typed-nil `*snapshot` inside the `DomainSnapshot` interface after
  the expected unsafe-regexp rejection, and the test calls `Close()` at
  `slice2_integration_linux_test.go:14`, panicking at `adapter_linux.go:187`.
  No Go or ABI source changed in this task; the failure is recorded rather
  than fixed in the evidence-only slice.
- Before and after remote execution, the frozen-input digests were unchanged:
  `.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline` = 2378 files,
  60755103 bytes, `538733b18dd516df21c27b998830c97ceae760c85702bda07391d2984a82634e`;
  `tests/phase5a-baseline` = 10 files, 41922 bytes,
  `34678ca9acd6072ad2a01d429fd513d70e7dd48c899fbfc7cb7e4df160cc6b2d`.
  The task-owned remote checkout and target were removed after validation. No
  benchmark runner, VM, deployment, production/default cutover or UI build
  was run; the final reviewer result remains pending.

#### Reproducible command ledger

The following commands were run exactly against the task-owned remote artifact;
each command exited 0 unless marked `EXIT 1`.

```text
git archive --format=tar 33e826ccd89a5db039bfc4d92aaf0593907dd95b | ssh mosdns-rust 'set -eu; test ! -e /tmp/mosdns-phase5a-slice3-w3-33e826c; mkdir -p /tmp/mosdns-phase5a-slice3-w3-33e826c/repo; tar -xf - -C /tmp/mosdns-phase5a-slice3-w3-33e826c/repo; test -f /tmp/mosdns-phase5a-slice3-w3-33e826c/repo/Cargo.toml; printf "%s\n" source-staged'  # remote EXIT 1 (wrong root manifest assertion; local pipeline status was not propagated; subsequent cargo commands used rust/Cargo.toml successfully)
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && printf "SOURCE=33e826ccd89a5db039bfc4d92aaf0593907dd95b\\n" && uname -a && printf "ARCH=" && uname -m && rustc --version && cargo --version && go version && python3 --version && printf "RUSTFMT=" && rustfmt --version && printf "CLIPPY=" && cargo clippy --version && printf "TMP=" && df -h /tmp | tail -n 1 && printf "ROOT=" && df -h / | tail -n 1'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && cargo fmt --manifest-path rust/Cargo.toml --all -- --check'  # EXIT 0
ssh mosdns-rust 'set -eu; test ! -e /root/mosdns-phase5a-slice3-w3-target-33e826c; mkdir /root/mosdns-phase5a-slice3-w3-target-33e826c; cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo; CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-w3-target-33e826c CARGO_BUILD_JOBS=1 cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-w3-target-33e826c CARGO_BUILD_JOBS=1 cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-w3-target-33e826c CARGO_BUILD_JOBS=1 cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && go test ./...'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && go vet ./...'  # EXIT 0
ssh mosdns-rust 'set -eu; cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo; CARGO_TARGET_DIR=/root/mosdns-phase5a-slice3-w3-target-33e826c CARGO_BUILD_JOBS=1 scripts/build-rust-cache.sh; mkdir -p rust/target/release; cp /root/mosdns-phase5a-slice3-w3-target-33e826c/release/libmosdns_runtime.a rust/target/release/libmosdns_runtime.a; test -s rust/target/release/libmosdns_runtime.a; sha256sum rust/target/release/libmosdns_runtime.a'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 go test -tags mosdns_rust ./plugin/executable/cache ./pkg/cache ./pkg/query_context ./pkg/server_handler ./pkg/matcher/... ./plugin/matcher/... -count=1'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter ./plugin/matcher/... -count=1'  # EXIT 1: typed-nil panic at matcher_adapter slice2_integration_linux_test.go:14 -> adapter_linux.go:187
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/matcher/... -count=1'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 go test -race -tags mosdns_rust ./plugin/executable/cache ./pkg/cache ./pkg/query_context ./pkg/server_handler ./pkg/matcher/... ./plugin/matcher/... -count=1'  # EXIT 0
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -race -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter ./plugin/matcher/base_domain ./plugin/matcher/base_ip -count=1'  # EXIT 1: same typed-nil panic at matcher_adapter slice2_integration_linux_test.go:14 -> adapter_linux.go:187
ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -race -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/matcher/base_domain ./plugin/matcher/base_ip -count=1'  # EXIT 0
ssh mosdns-rust 'python3 -' <<'PY'  # EXIT 0; shutil.rmtree only /tmp/mosdns-phase5a-slice3-w3-33e826c and /root/mosdns-phase5a-slice3-w3-target-33e826c
python3 .trellis/scripts/task.py validate .trellis/tasks/09-22-rust-phase5a-native-routing  # EXIT 0
git diff --check  # EXIT 0
git add .trellis/tasks/09-22-rust-phase5a-native-routing/implement.md docs/ai/rust-handover.md docs/rust/feature-coverage.md && git diff --cached --check && git commit -m 'docs: record native W3 Linux evidence' && git push origin rust  # EXIT 0; evidence commit 8150b7e06331612343683f42ea415e5b776387a6
```

The frozen-input digest command was run before and after the checks as
`ssh mosdns-rust 'cd /tmp/mosdns-phase5a-slice3-w3-33e826c/repo && python3 -'`
with this exact script, covering exactly the two scopes below:

```python
from pathlib import Path
import hashlib
for scope in ('.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline', 'tests/phase5a-baseline'):
    root = Path(scope)
    rows = []
    total = 0
    for path in sorted(item for item in root.rglob('*') if item.is_file()):
        data = path.read_bytes()
        total += len(data)
        rows.append(path.as_posix().encode() + b'\0' + str(len(data)).encode() + b'\0' + hashlib.sha256(data).hexdigest().encode() + b'\n')
    print(scope, len(rows), total, hashlib.sha256(b''.join(rows)).hexdigest())
```

Both runs produced the digests recorded above.
