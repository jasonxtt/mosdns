# Implementation plan — bundled query diagnostics

## Execution record — 2026-09-30

The approved implementation was delivered in the current `rust` worktree.
The Rust backend now records final-wire response facts and rich answer
diagnostics, native admission IDs, routing provenance, filtered logs, four
categorical ranks, independent top-300 slowest history, bounded two-slot read
admission, and exact domain drill-down. QueryManager and OverviewManager use
the real fields, show read failures per panel, and expose query/ranking detail
drill-down without fabricating unsupported native sections. The final review
corrections also cover matcher-less local/default `unmatched_rule` identity,
real-listener cache miss/hit/TTL-aging, uncommon raw-RDATA and malformed-wire
projections, upstream-timeout agreement, complete rank membership/lifecycle,
and fallible expensive-read encoding: cancellation is 499, allocation or
serialization failure is 500, and the semaphore permit remains owned until
the worker exits.

All product builds and validation ran through the `mosdns-rust` SSH alias in
`/root/mosdns-rust-querydiag`; no local Cargo or UI build, production deploy,
Go/cgo bridge edit, or push was performed. The full `cargo test --workspace`
run passed, as did `cargo fmt --all -- --check`, workspace clippy with
`-D warnings`, the native-host build, focused native-host and `slice8` tests,
and disposable VM `npm ci && npm run build`. Real UDP/TCP/HTTP/browser proof
is recorded in `research/browser-proof/README.md`; the 400000-record concurrent
read evidence is in `research/browser-proof/vm-validation.md`. The current
narrow code candidate is commit `64b298e2` (`fix(native): close query
diagnostics review gaps`), following the earlier provenance/projection commit
`be8f8ce1`. The final independent C2C review of the exact committed range
remains the last handoff gate.

## State and start gate

Historical planning gate. The planning conversation stopped here as required;
the subsequent user execution approval superseded this handoff and the task
was activated with `task.py start`. The original checklist is preserved below
as the approved plan, while the execution record above and browser-proof
evidence record the delivered slices.

- [ ] Read AGENTS/project/config/rust-handover/rewrite-plan, this task's
  PRD/design/contracts/C2C notes, workflow and native-audit-control spec.
- [ ] Refresh branch/HEAD/task current/list and dirty path inventory. Leave
  auto-commit disabled. Confirm existing archived task completion instead of
  resuming stale roadmap children. Do not edit/stage unrelated dirty documents.
- [ ] Check final plan approval; resolve proposed choices before activation.
- [ ] Run trellis-before-dev for affected native-host/sequence-core/dns-core/UI.
- [ ] Activate this task after the required review/approval gate is met.

## Slice 0 — freeze contracts and red tests

- [ ] Read source anchors in contracts; record precise fixture expectations,
  native-versus-Go decisions and eligible API methods/bodies/content types.
- [ ] Prepare one retained reproducible YAML/rule fixture with parent+child,
  file-backed named sets, inline rules, local reject, cache and two numeric
  controlled peers. Include overwritten intermediate response and rule hot
  update. No public upstream or live config; no mocked host/observer/API.
  Put low-level decoder/provenance/filter invariants in focused Rust tests;
  keep one realistic integrated fixture, not a Cartesian E2E expansion.
- [ ] Add failing end-to-end terminal-field/filter/rank tests at public real
  listener/HTTP seams. Characterize already-satisfied behavior as regression,
  not artificial red evidence. Mock only external peer/time/failure boundaries.

## Slice 1 — final terminal record

- [ ] Implement compiler/provider matcher source identity with generation
  ownership; test multiple providers, first selected match, negation, later
  matcher failure, no-exec rule, child/parent precedence and default branch.
- [ ] Add native ID format/uniqueness across UDP/TCP, nonce-source failure
  and injected counter-exhaustion tests; use direct getrandom facility.
- [ ] Implement native ID, final-only response flags/code/class/answers,
  effective label and configured versus actual upstream projection. Decode
  final wire; test all-or-none answer_details_status/answer_decode_error,
  preserve cache TTL aging, response replacement and no-response.
- [ ] Cover interrupted execution, upstream timeout, cancel and transport-send
  failure through real host listeners/barriers. Preserve prior audit eligibility
  and configured flow_setter precedence without altering DNS answers.
- [ ] Run focused native and dns/sequence tests on VM after each changed behavior.

## Slice 2 — search and ranking HTTP

- [ ] Typed filter parse; q exact/fuzzy, repeated clients, domain_set/effective,
  domain/answer IP/CNAME and unknown args. Add exact logs/domain native
  predicate and 404-only Vue legacy fallback; test cross-field collisions and
  suffix domains. Preserve page and limit contracts.
- [ ] Implement coherent filtered count/page and all rank/slowest projections.
  Test empty/top-k/ties, Unicode/percent/+ decoding, duplicate/empty values,
  unknown provenance vs genuine unmatched, filter AND/OR, case/IP normalization, overflow,
  unsupported params, clear/stop/resize/evict and approved slowest retention.
- [ ] Keep read work off DNS thread; two running jobs/no queue/503, permits owned
  through worker exit, cancellation polling and shared immutable records.
  At/near full ring prove DNS progress while real clients read ranks/logs.
  Separate structural-bound proof, large-record bytes/projection and VM-sized
  near-full rich-record progress fixture; preserve peak
  memory and timing observations as diagnostics, not formal performance PASS.

## Slice 3 — Vue list, details and rankings

- [ ] Reuse QueryManager and OverviewManager. Minimal adaptations for per-panel errors and real
  fields, load-more/search/quick filters/details and ranking drill-down;
  preserve draft/error behavior and Go backend behavior.
- [ ] Ensure nonimplemented capture/alias/upstream operations cannot show
  synthetic success. Record unsupported sections honestly without rewriting
  the UI into a native-only product. Avoid sensitive table/CSS regressions.
- [ ] VM native binary + disposable Vite source/build + owned SSH tunnel;
  local browser sends only real API interactions. Generate real UDP/TCP DNS
  cases with controlled peers. Verify browser actions and API/wire agreement,
  including empty/error/refresh/clear/stop/start states and navigation.
- [ ] Recheck DNS card/System controls and local-rule save/query closure as
  regression; no need to rerun the old formal benchmark matrix.

## Slice 4 — integrated validation and one final review

- [ ] Final exact-candidate VM checks from isolated `<remote>/rust`:
  `cargo fmt --all -- --check`
  `cargo clippy --workspace --all-targets -- -D warnings`
  `cargo test --workspace`
  `cargo build -p mosdns-native-host`
  The package is mosdns-native-host and its binary is named mosdns.
- [ ] Target integration tests with `cargo test -p mosdns-native-host --test
  <new-test-name>` plus existing observability, slice6_management_http,
  slice8_audit_read_http and config/composition suites. Names for new tests
  are determined during implementation; don't pretend they already exist.
- [ ] In disposable VM UI copy, install using the repository lockfile and run
  `npm ci` and `npm run build` (package-lock.json and package.json). Product build/test execution only
  through `ssh mosdns-rust`; don't build on macOS or alternate VM.
- [ ] Source sync includes fixture directories such as
  `rust/native-host/tests/phase5a-baseline`; do not repeat the previous missing
  fixture archive failure. Exclude target/node_modules/generated UI assets.
- [ ] Record exact candidate revision, command list/config/fixture sources,
  focused/full outcomes, original failures/corrections, browser actions,
  peer counts, owned PID/ports/tunnel identity and complete cleanup/rebind.
- [ ] Update only proven coverage subitems and necessary backend contracts.
- [ ] Obtain complete independent review of exact task changes. C2C transport
  may be reused if authorized in the execution conversation; exact committed
  review requires explicit commit authorization and narrow paths. Do not
  substitute planning advice for final review or mark task complete early.
- [ ] Commit/archive/push only within authorization then current workflow.
  Never stage -A or unrelated changes. Production and Phase 6 stay gated.

## Planning verification (original planning conversation)

- [x] PRD convergence, no hidden product choices; detailed contracts and
  risks captured, one PRD and one integrated acceptance kept.
- [x] New project C2C discussion completed; source-verified advice incorporated
  or reasoned rejection recorded in research/c2c-discussion.md.
- [x] task.py validate and git diff --check; inspect precise planning paths.
- [x] Execution prompt saved; the planning conversation stopped without
  implementation, before the later execution approval.

Planning validation on 2026-09-30: context manifests each contain five
existing source/spec links, the task was planning with base_branch rust at the
handoff, and JSON/links/new-file whitespace checks plus git diff --check
passed. Product verification was intentionally not run in that planning
conversation; the later execution record above contains the VM-only evidence.
