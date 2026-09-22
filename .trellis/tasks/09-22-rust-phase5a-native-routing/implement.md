# Implementation plan — native W3 routing

Status: planning only. Numeric units below are a proposed execution plan;
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
- [ ] `SLICE 2: PASS` required before Slice 3.

## Slice 3 — Linux correctness evidence and final gate

Allowlist: task directory, `docs/ai/rust-handover.md`,
`docs/rust/feature-coverage.md` for bounded W3 status/evidence only.
No product fixes in evidence-only slice; failures return to their owning slice.

- [ ] On the previously designated `ssh mosdns-rust`, exact reviewed source in
  fresh temporary directory, record architecture/toolchains/storage check.
  Use a task-owned disk-backed target; no production service/install change.
- [ ] Run native-host W1 UDP/TCP, W2, W3 targets and Rust workspace fmt/tests/
  clippy with --locked, exact commands/env and result summaries. Baseline/corpus
  digests must match before/after; no benchmark runner or local VM.
- [ ] Because shared DNS parsing is affected, run existing runtime ABI tests
  and the repository's Linux staticlib + tagged Go cache/matcher tests; full
  `go test ./...` with serial UI build if required. No Go source edits. If unsafe
  or ABI implementation changes become necessary, stop/review scope first and
  add applicable focused memory-safety/race checks; ordinary tests are not Miri.
- [ ] Record route counters/order, concurrency/lifecycle results, complete
  commands, any failed attempt/retry and cleanup of task-owned remote artifacts.
- [ ] Update coverage/handover: bounded W3 only; 5A basic observability and
  comparable native performance remain open. Keep final reviewer result pending
  until an actual response is received.
- [ ] Commit/push evidence, obtain `FINAL: PASS` for A1–A7, record that actual
  response, report tested/evidence SHA, then stop before finish/archive,
  performance, deployment, additional tasks or full-feature expansion.
