# Rust migration next-step roadmap — implementation plan

This parent is a planning and coordination task. It must not be marked ready
for execution until the exact planning range receives `FINAL: PASS` from the
same C2C conversation that supplied the plan.

## 1. Persist the reviewed plan (current step)

- [x] Replace the stale execution-gate text in this task's PRD with the actual
  C2C setup, planning result, canary result, and replacement review result.
- [x] Keep this task's `base_branch` set to `rust`; the repository is the
  dedicated `rust` worktree, and `main` is not the intended base.
- [x] Update `docs/rust/next-stage-plan.md` with the canary → bounded 5B → first
  5C sequence, exact start/review gates, current evidence, and explicit deferrals.
- [x] Complete this task's `design.md` and `implement.md` and both child tasks'
  PRD/design/implement artifacts. Leave seed JSONL manifests in inline mode;
  do not dispatch sub-agents.
- [x] Keep the historical canary task's evidence and terminal superseded status
  unchanged; record its fixed candidate and replacement review by reference.
- [x] Keep `docs/rust/feature-coverage.md` acceptance states unchanged during
  planning.

### Planning validation

- [x] Run `python3 ./.trellis/scripts/task.py validate <task-dir>` for the parent
  and both new children.
- [x] Run `git diff --check` and inspect `git status --short` plus the exact diff.
- Do not run Cargo, frontend, network, remote-host, benchmark, or product tests
  for this documentation-only package.
- [x] Stage only the parent/child planning files, `docs/rust/next-stage-plan.md`,
  and any task-parent metadata that belongs to this task. Commit a narrow
  planning range; do not push.

## 2. Same-chat plan review

- [x] Record exact full `BASE_SHA`, `HEAD_SHA`, task ID, scope, and paths.
- [x] Send a `MODE: REVIEW_ONLY`, `STATE: REVIEW`,
  `CONTROLLER: TRELLIS` request to the same C2C conversation.
- [x] Wait for an explicit `FINAL: PASS`. If the reviewer finds a defect, fix
  it locally, commit only the correction, and ask for another exact-range
  review in the same conversation. Preserve prior failure records.
- [x] C2C returned `FINAL: PASS`; downstream work remained unstarted until the
  current user explicitly approves the next child.
- [ ] If C2C is unavailable or the result is not a pass, keep downstream work
  unstarted and report the exact blocker; do not treat silence as approval.

### Review history

- Initial new-chat review range
  `f32a84a162a2f6f3dc7cb2ff4cd7fc3bfee218df..9c2169f21c38998efde5cfd92005f96d85f20015`
  returned `FINAL: FAIL` with `P1-1`: the PRD's closing state did not name the
  required 5B same-chat review gate before 5C.
- Correction range
  `9c2169f21c38998efde5cfd92005f96d85f20015..58e4f0bfb91d2a6c6b57f460ca8250baa9842eff`
  named the 5B same-chat C2C `FINAL: PASS` gate and retained explicit user
  approval for reordering when 5B is deferred or blocked. The same new C2C
  conversation returned `P1-1 [closed]` and `FINAL: PASS`.

## 3. Resolve canary execution inputs and result

The canary inputs were resolved before execution with these frozen defaults:

1. Use a read-only `config_lite_all` snapshot and record its exact source
   identity; never read or copy live `/cus/mosdns` state.
2. Keep a same-config Go comparator optional, best-effort, non-gating, and do
   not build Go.
3. Use controlled local peers as the hard query/routing/cache oracle.
4. Roll back with identity-checked TERM and prove every owned process/port is
   released; do not claim internal graceful shutdown.

The corrected bounded run on `mosdns-rust` passed. No canary rerun or new
remote operation is part of this parent task; the evidence and exact review
history remain in the original superseded task and archived replacement task.

## 4. Execute the existing canary (completed separately)

- [x] Start only the existing canary task, using its already-reviewed
  `prd.md`, `design.md`, `implement.md`, candidate SHA, provenance gate,
  peer self-tests, service baseline, two listener modes, and owned-resource
  cleanup. Do not alter its scope or add product code.
- [x] Stop before listener startup if source/config/host/PID/port identity or
  isolation cannot be established; preserve the canary's STOP/FAIL distinction.
- [x] If a product defect appears, preserve evidence and create a separate
  remediation scope. Do not silently include a repair in the canary.
- [x] After accepted evidence, commit the exact canary result and send that
  range to the same C2C chat. Resolve findings and repeat until `FINAL: PASS`.
- [x] The canary result and its replacement exact-range `FINAL: PASS` are
  recorded. The next action is the 5B child after the user approves its latest
  planning summary.

## 5. Execute the new 5B child

- [ ] Activate `09-28-rust-native-fast-mark-flow-setter` only after the parent
  plan review and the canary ordering decision allow it.
- [ ] Before implementation, re-read `AGENTS.md`, the Rust handover/architecture
  docs, `.trellis/workflow.md`, and run `trellis-before-dev` for
  `rust/native-host` and, if needed, `rust/sequence-core`.
- [ ] Characterize `flow_setter` versus host-derived terminal metadata and
  record the chosen precedence in the child task before coding.
- [ ] For each observable behavior slice, write a failing YAML-to-runtime or
  runtime integration test, then implement and refactor. Keep sequence and
  compiler real; mock only the external upstream boundary.
- [ ] Run focused tests, `cargo fmt --check`, `cargo clippy --all-targets -D
  warnings`, and the full Rust workspace tests specified by the child.
- [ ] Update only proven feature-coverage subitems. Do not claim complete P11,
  P33, P44, switch support, or 5B.
- [ ] Commit the task result and submit that exact committed range to the same
  C2C conversation. Address findings and repeat until `FINAL: PASS`.

## 6. Execute the new first-5C child

- [ ] Default order: activate `09-28-rust-native-domain-set-management` only
  after 5B has its same-chat review PASS. If 5B is explicitly deferred or
  blocked, obtain the user's explicit decision to reorder before activation.
- [ ] Re-read project instructions and run `trellis-before-dev` for the native
  host/API package before product-code changes.
- [ ] For each behavior, write a failing HTTP or real-listener test first.
  Exercise the real router, rule compiler, published matcher generation, and
  DNS path; inject only persistence failure at the storage boundary.
- [ ] Verify show/save/post compatibility, failed-update rollback, whole
  generation visibility during concurrent queries, restart load, and API/DNS
  shutdown plus rebind.
- [ ] Run targeted native-host tests, `cargo fmt --check`, clippy, and the full
  Rust workspace tests specified by the child. Run selected Linux functional
  E2E only if available within its plan; no performance gate.
- [ ] Update only the delivered domain-set/API subitems. Do not claim full C04,
  C10, C11, C17, all 5C, or complete configuration compatibility.
- [ ] Commit the task result and submit that exact range to the same C2C chat.
  Address findings and repeat until `FINAL: PASS`.

## 7. Parent closeout

- [ ] Verify every executed package has an explicit same-chat `FINAL: PASS`;
  every deferred package is explicitly recorded with its existing
  planning/review status and remains unexecuted.
- [ ] Preserve any failed review and corrective history in task artifacts.
- [ ] Confirm no production/default cutover or hybrid-retirement claim was
  introduced by this sequence.
- [ ] Report the reviewed task ranges, bounded results, deferred scope, and
  remaining final gates to the user. Do not claim completion while any
  authorized, non-deferred package or its review remains open.
