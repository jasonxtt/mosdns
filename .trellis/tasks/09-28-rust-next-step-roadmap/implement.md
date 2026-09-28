# Rust migration next-step roadmap — implementation plan

This parent is a planning and coordination task. It must not be marked ready
for execution until the exact planning range receives `FINAL: PASS` from the
same C2C conversation that supplied the plan.

## 1. Persist the reviewed plan (current step)

- [ ] Replace the stale browser-blocker text in this task's PRD with the actual
  C2C setup and planning result.
- [ ] Keep this task's `base_branch` set to `rust`; the repository is the
  dedicated `rust` worktree, and `main` is not the intended base.
- [ ] Update `docs/rust/next-stage-plan.md` with the canary → bounded 5B → first
  5C sequence, exact start/review gates, current evidence, and explicit deferrals.
- [ ] Complete this task's `design.md` and `implement.md` and both child tasks'
  PRD/design/implement artifacts. Leave seed JSONL manifests in inline mode;
  do not dispatch sub-agents.
- [ ] Keep the existing canary task's files and status unchanged. Record its
  fixed candidate and user-decision gate by reference only.
- [ ] Keep `docs/rust/feature-coverage.md` acceptance states unchanged during
  planning.

### Planning validation

- Run `python3 ./.trellis/scripts/task.py validate <task-dir>` for the parent
  and both new children.
- Run `git diff --check` and inspect `git status --short` plus the exact diff.
- Do not run Cargo, frontend, network, remote-host, benchmark, or product tests
  for this documentation-only package.
- Stage only the parent/child planning files, `docs/rust/next-stage-plan.md`,
  and any task-parent metadata that belongs to this task. Commit a narrow
  planning range; do not push.

## 2. Same-chat plan review

- [ ] Record exact full `BASE_SHA`, `HEAD_SHA`, task ID, scope, and paths.
- [ ] Send a `MODE: REVIEW_ONLY`, `STATE: REVIEW`,
  `CONTROLLER: TRELLIS` request to the same C2C conversation.
- [ ] Wait for an explicit `FINAL: PASS`. If the reviewer finds a defect, fix
  it locally, commit only the correction, and ask for another exact-range
  review in the same conversation. Preserve prior failure records.
- [ ] If C2C is unavailable or the result is not a pass, keep downstream work
  unstarted and report the exact blocker; do not treat silence as approval.

## 3. Resolve canary execution inputs with the user

After plan review PASS, present one concise decision request with these
recommended defaults:

1. Use a read-only `config_lite_all` snapshot and record its exact source
   identity; never read or copy live `/cus/mosdns` state.
2. Keep a same-config Go comparator optional, best-effort, non-gating, and do
   not build Go.
3. Use controlled local peers as the hard query/routing/cache oracle.
4. Roll back with identity-checked TERM and prove every owned process/port is
   released; do not claim internal graceful shutdown.

Ask whether the user authorizes running the existing canary or defers it. If
deferred, explicitly ask whether 5B may proceed without the remote result.
Proceed according to the answer; no remote operation before it.

## 4. Execute the existing canary, if authorized

- [ ] Start only the existing canary task, using its already-reviewed
  `prd.md`, `design.md`, `implement.md`, candidate SHA, provenance gate,
  peer self-tests, service baseline, two listener modes, and owned-resource
  cleanup. Do not alter its scope or add product code.
- [ ] Stop before listener startup if source/config/host/PID/port identity or
  isolation cannot be established; preserve the canary's STOP/FAIL distinction.
- [ ] If a product defect appears, preserve evidence and create a separate
  remediation scope. Do not silently include a repair in the canary.
- [ ] After accepted evidence, commit the exact canary result and send that
  range to the same C2C chat. Resolve findings and repeat until `FINAL: PASS`.
- [ ] Start 5B only after canary PASS, unless the user explicitly allowed 5B
  after deferring the canary.

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

- [ ] Activate `09-28-rust-native-domain-set-management` only after 5B has its
  same-chat review PASS.
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

- [ ] Verify every planned package is either completed with an explicit
  same-chat `FINAL: PASS`, or explicitly deferred by the user and recorded.
- [ ] Preserve any failed review and corrective history in task artifacts.
- [ ] Confirm no production/default cutover or hybrid-retirement claim was
  introduced by this sequence.
- [ ] Report the reviewed task ranges, bounded results, deferred scope, and
  remaining final gates to the user. Do not claim all planned work is complete
  while a package, task, or review remains open.
