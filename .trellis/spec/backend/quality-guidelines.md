# Quality Guidelines

## Change discipline

- Make the minimum change that satisfies the active task. Do not refactor adjacent code or reformat unrelated files.
- Every changed line must trace to a requirement in the task. Remove only imports or code made unused by that change.
- Preserve unrelated dirty worktree changes. Trellis auto-commit is disabled for this repository.
- Behavior parity comes before cleanup during the Rust migration. Keep a working Go fallback until the task's removal gate is met.

## Verification

- Add a failing regression or parity test before changing behavior, then make it pass.
- Run focused package tests while iterating and `go test ./...` before handing off backend changes.
- Run race tests for concurrency-sensitive Go bridges and cache code.
- Rust crates must pass `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings`.
- FFI changes require Linux+cgo integration coverage plus sanitizer/Miri or an equivalent focused memory-safety check where applicable.
- Cache migration requires golden parity for TTL, negative answers, ECS, exclusion, lazy update, dump/API behavior, raw UDP/TCP/HTTP responses, concurrency, metrics, and fallback.

## Build and deployment

Release/deployment binaries must contain freshly built Vue assets; use repository build scripts rather than a bare `go build`. For deployment work, validate locally, then on `mos-test` (`10.0.0.91`), and promote to production only after confirmation.

## Review gates

- No config/API/metric/audit compatibility regression.
- No panic or ambiguous ownership across FFI.
- No hot-path O(n) scan, global serialization, or excessive cgo calls introduced by the Rust backend.
- No undocumented KixDNS source copy; pin and attribute extracted code.
- If p99, CPU, or RSS regresses beyond the task's accepted threshold, keep Rust experimental rather than making it default.

## External ChatGPT planning and root-review loop

### 1. Scope / Trigger

Use this workflow whenever a task has a phase gate, architecture decision,
acceptance review, or an explicit user request to have a ChatGPT web
conversation review the work. It applies to planning documents and to
implementation changes that must be reviewed through GitHub.

The local Codex task is the executor. The selected ChatGPT web conversation is
the plan owner and root acceptance gate. The user remains the authority for
which conversation is used and whether the next phase may begin.

### 2. Roles and handoff contract

- Local Codex reads the repository, task artifacts, specs, and current review
  result; it modifies the allowed local files, runs verification, commits
  only the scoped diff, pushes the requested branch, and reports the exact
  commit to the reviewer conversation.
- The ChatGPT web conversation supplies the plan/review decision. It must
  review the GitHub commit and repository evidence, return an explicit PASS or
  FAIL, and identify the next authorized scope. It is not an authorization to
  silently start later phases.
- The user selects or confirms the destination conversation. Conversations may
  change between review rounds, but Codex must ask the user for confirmation
  before sending work to a different conversation or when the destination is
  missing or ambiguous.

### 3. Review request and response contract

Every review request must include:

- repository, branch, and full pushed commit;
- GitHub URL or exact commit/tree paths;
- changed-file scope and validation results;
- current task/phase status;
- explicit prohibited actions, such as task start, production wiring, or
  unrelated implementation;
- a request for a formal PASS/FAIL and the next authorized scope.

Codex must read the conversation result after sending the request. An active,
pending, or incomplete response is not PASS. A PASS must be explicit; Codex
must not infer it from a successful push, a green local check, or silence. If
the conversation requests user input, a different conversation, or missing
evidence, Codex stops and asks the user before continuing.

### 3.1 Pending review cadence and automatic remediation

- After sending a review request, Codex must wait/read the selected
  conversation at approximately one-minute intervals until it returns an
  explicit `PASS`, an explicit `FAIL`, or requires user input. Do not busy-poll
  and do not treat a queued user message, an active turn, or an unchanged
  preview as a review result.
- Use the platform's bounded wait primitive when it supports the selected
  conversation. For a ChatGPT conversation that cannot be used as a wait
  target, wait about 60 seconds and then call `read_thread` once; continue
  this bounded cadence while the review remains active. Each read must use the
  latest cursor when one is available.
- A pending review is an expected wait state, not permission to speculate,
  modify code, start another slice, archive the task, or claim acceptance.
- When the reviewer returns a scoped `FAIL`, Codex automatically applies only
  the requested remediation, runs the focused checks, inspects the exact diff,
  stages exact paths, commits, pushes, and sends a new review request to the
  same confirmed conversation. It then resumes the one-minute wait/read loop.
- When the reviewer returns a `FAIL` that requires a scope change, missing
  evidence, user choice, or a different conversation, Codex stops and asks the
  user. It must not widen the diff or change review destinations on its own.
- When the reviewer returns `PASS`, Codex records the result and stops at the
  authorized boundary. `PASS` never automatically authorizes the next Slice.

### 4. Validation and error matrix

| State or event | Required action |
| --- | --- |
| No review conversation or ambiguous destination | Ask the user to select/confirm the conversation; do not send |
| Reviewer returns FAIL with scoped fixes | Apply only those fixes, rerun focused checks, commit/push, and request another review |
| Reviewer returns FAIL requiring a scope change | Stop and ask the user; do not widen the diff |
| Reviewer is active or has not returned a formal result | Wait/read again; do not modify or start the next phase |
| Reviewer returns PASS | Record the result and stop at the approved boundary |
| Push fails or remote differs from local | Diagnose the push/branch state; do not claim review requested |
| Unrelated dirty files are present | Preserve them and stage exact task files only |

### 5. Execution loop

The normal loop is:

1. Read the latest reviewer conversation and local repository state.
2. Confirm the destination conversation with the user when it is new,
   changed, or ambiguous.
3. Apply only the review-scoped plan to the local worktree.
4. Run focused validation and inspect the exact diff.
5. Stage exact files, commit with a descriptive message, and push the requested
   branch. Never use git add -A in this worktree.
6. Send the full commit and evidence to the selected ChatGPT conversation.
7. Wait/read the selected conversation at approximately one-minute intervals
   until it returns explicit PASS, explicit FAIL, or requires user input.
8. On a scoped FAIL, automatically return to step 1 and repeat only the
   bounded remediation loop; on a scope-changing FAIL or user-input request,
   stop and ask the user.
9. On PASS, stop. Do not automatically run task.py start, begin a later Slice,
   advance the phase, wire production, or create a follow-up task.

The user explicitly decides when a passed review should become the next phase.
Review PASS authorizes only the scope stated by the reviewer and never implies
automatic continuation.

### 6. Required checks and evidence

Before each push, Codex records the relevant task validation, JSON/YAML or
manifest parsing, focused tests/builds, lint/format checks, exact changed
paths, branch, and pushed commit. For planning-only work, it also confirms
that no implementation code, dependency, production wiring, or task status
change was introduced. A review round is incomplete until the remote branch
contains the reported commit.

### 7. Good / Base / Bad cases

- Good: Codex receives a FAIL tied to five planning lines, edits only those
  files, runs validation, pushes one commit, sends that commit to the
  confirmed conversation, reads the next formal result, and stops on PASS.
- Base: the conversation is still active after a push; Codex reports that the
  review is pending and makes no speculative changes.
- Bad: Codex chooses another conversation without confirmation, treats git
  push or task.py validate as PASS, starts the next phase after a review, or
  stages unrelated dirty files.

### 8. Wrong vs Correct

Wrong: push a local change, assume the review passed, run the next phase, or
send the change to a different ChatGPT conversation without telling the user.

Correct: ask the user to confirm the destination conversation, push the exact
scoped commit, send its GitHub evidence, read the explicit reviewer result,
repeat bounded fixes until PASS, then stop and wait for the user's next-phase
decision.

## MCP DSH controlled execution

Use this protocol when the user authorizes MCP DSH as the implementation
executor. DSH is a bounded worker, not an authority to widen scope, commit,
push, start a later slice, or change the project's review destination.

### Safety and ownership

- The parent Codex verifies repository path, branch, status, remote, and
  worktree state before dispatch. It preserves unrelated dirty files and
  requires a clean isolated worktree for normal DSH execution.
- Prefer `workspace_mode: clean`; use `snapshot` only when the user explicitly
  requires current uncommitted or ignored state to be included.
- Every dispatch prompt names the active task, one behavior/blocker/slice, an
  exact allowed-file list, required checks, and forbidden paths/actions.
- DSH must not use destructive broad commands, `git add -A`, commit, push,
  production wiring, or files outside the repository. The parent inspects the
  complete DSH diff and calls `dsh_apply` explicitly; a DSH summary is never
  sufficient evidence for applying changes.
- If the DSH changed-file list, base, patch, or scope check is unexpected, do
  not apply it. End the session, preserve the main tree, and investigate the
  exact discrepancy.

### Dispatch and timeout contract

- Use read-only `dsh_investigate` only when the implementation choice or
  repository state is genuinely ambiguous. Otherwise use one focused execute
  job directly.
- One job handles one behavior or one narrowly related blocker. Do not send a
  whole phase or several independent remediation families to one job.
- For work that may take longer than one MCP request, use asynchronous
  `dsh_start`, then bounded `dsh_wait` calls of at most about 60 seconds. A
  still-running result is not a failure and must not cause a duplicate job.
- After an idle execute session, call `dsh_diff` once, inspect changed paths
  and the complete patch, apply only the reviewed whitelist, then run checks in
  the parent worktree and call `dsh_end`.
- Keep prompts and reports concise: return changed paths, diff summary,
  commands, exit status, and failures; do not dump unrelated source or ignored
  build output. Do not assume DSH is free or outside Codex usage accounting;
  verify actual usage in the product billing/usage view when cost matters.

### Verification and review loop

- DSH first writes the RED test for the selected behavior and records the real
  focused failure. It then implements the minimum GREEN change and runs only
  the focused checks needed for that behavior.
- The parent reruns focused tests after apply, then runs the full workspace
  checks only at the final slice boundary: tests, warnings-denied clippy,
  format, manifest/tree inspection, task validation, and diff checks as
  applicable.
- Before commit, stage exact reviewed paths only. After push, send one compact
  review request containing the full commit, diff scope, evidence, status, and
  forbidden follow-on scope to the same confirmed ChatGPT conversation.
- Wait/read that review at approximately one-minute intervals. Active,
  pending, unchanged, or timed-out reads are not PASS. A scoped FAIL starts a
  new clean DSH job for only that remediation; a scope-changing FAIL requires
  user input; an explicit PASS closes the current slice and stops.

### Good / bad examples

- Good: `dsh_start(clean)` for one TCP framing behavior, bounded waits, one
  complete diff inspection, exact apply, focused test, final full checks, and
  root review before stopping.
- Bad: a single monolithic DSH job for all Phase4 slices, a synchronous MCP
  call held for many minutes, applying a patch without reading it, or letting
  DSH continue from Slice1 into Slice2 without a new user authorization.

## Herdr-hosted Codex and Claude execution

### 1. Scope / Trigger

Use this routing contract when the current coding session is running through
Herdr and the user has selected the adjacent Claude Code pane as executor.
It prevents an ordinary local Codex session, an unrelated Claude pane, or an
ambiguous terminal from being mistaken for the authorized execution topology.

### 2. Detectable roles and routing contract

At session start, inspect the Herdr inventory and detection evidence:

```text
herdr agent list
herdr agent explain <current-pane>
herdr agent explain <executor-pane>
```

Herdr mode is active only when all of these are true:

- the current/focused pane is detected as `agent: codex`;
- a pane in the same Herdr workspace is detected as `agent: claude`;
- both panes have the repository as their working directory; and
- the Claude pane is the user-selected adjacent/right executor pane.

`herdr agent explain` is the authoritative detection evidence. A matching
terminal title or the mere presence of the `herdr` executable is not enough.
If any condition is missing or ambiguous, do not auto-route work or approve a
prompt; ask the user to confirm the executor topology.

When the contract is active:

- Codex remains the dispatcher/controller: it owns scope, worktree safety,
  exact diff inspection, validation, commit/push verification, and the root
  review loop.
- Claude Code in the selected right pane is the implementation executor. It
  may edit only the authorized slice and must stop at that slice boundary.
- The fixed `rust0916` ChatGPT conversation remains the reviewer/root gate.
- Claude reports back through Herdr, and every report starts with `我是Claude`;
  it includes changed paths, validation, exact commit/push state, and a review
  request. A report without that marker is not treated as Claude's handoff.

### 3. Prompt approval contract

The Codex controller may automatically approve a visible Claude confirmation
only after reading the exact command or action. Safe approval includes:

- repository-local reads, searches, status/log/show/diff and metadata checks;
- explicit read-only inspection of the local Cargo registry or other pinned
  dependency sources when needed to verify an API or ownership contract;
- repository-local format, build, test, clippy, task validation and other
  explicitly requested checks;
- edits confined to the user-authorized source, test, fixture, and task-evidence
  whitelist for the active slice;
- creating or removing unique temporary files under a task-scoped temporary
  directory, with cleanup scoped to those exact paths; and
- the authorized exact-path commit and push to the requested branch.

The controller must reject or redirect a prompt when it contains any of the
following:

- broad or unresolved deletion, overwrite, or recursive cleanup;
- `git reset`, rebase, force-push, branch switching, or history rewriting;
- `git add -A` or staging unrelated dirty files;
- secrets, credentials, private keys, or any write outside the repository;
- writes of marker/temp files at the repository root or other fixed paths that
  can collide with user files; or
- a command whose scope cannot be determined from the visible prompt.

For a rejected fixed-path temporary write, redirect Claude to an explicit
`mktemp -d` directory or a unique task-scoped path and a narrowly scoped
cleanup trap. Never approve first and inspect the resulting damage later.

The normal monitoring loop is:

```text
herdr agent wait <claude-pane> --until blocked --timeout 60000
herdr agent read <claude-pane> --source visible --lines 20
herdr agent send-keys <claude-pane> enter       # safe, reviewed prompt
herdr agent send-keys <claude-pane> 2 enter     # reject/choose safe alternative
```

Use bounded waits rather than busy polling. An idle/finished Claude pane is a
handoff state, not permission to start the next slice; the reviewer must still
return an explicit PASS and the user must authorize the next slice.

### 4. Validation and error matrix

| Condition | Required action |
|---|---|
| Codex + same-workspace right Claude detected | Route the authorized slice to Claude and monitor Herdr |
| Detection missing or panes/cwds disagree | Stop routing and ask the user |
| Visible command is read/build/test/fmt/clippy/validate or exact scoped Git action | Inspect it, then approve if scope is exact |
| Fixed-path write, broad delete, reset/rebase/force-push, secret access, or unknown command | Reject and request a bounded safe alternative |
| Claude reports without `我是Claude` or omits commit/evidence | Treat as incomplete handoff and request a corrected report |
| Reviewer active/pending or no explicit PASS | Wait; do not modify or begin another slice |
| Reviewer explicit scoped FAIL | Apply only the requested remediation, then repeat review |

### 5. Good / Base / Bad cases

- Good: `agent explain` confirms Codex `w6:p1` and Claude `w6:p2` in the
  same repository; a visible `cargo test --locked` prompt and a read-only
  pinned Hyper source inspection are inspected and approved; a root-level
  marker write is rejected and replaced with a unique temporary path.
- Base: Claude is idle after pushing a scoped commit; Codex reads the pane,
  independently verifies the commit, sends it to `rust0916`, and waits.
- Bad: approve every Claude prompt because it is in a trusted pane, accept a
  report without `我是Claude`, stage `.DS_Store` files, or start Slice2 merely
  because Slice1 passed.

### 6. Tests and evidence required

- Record the Herdr detection evidence (`agent list`/`agent explain`) when this
  routing is activated.
- Before review, inspect `git status`, exact changed paths, complete diff,
  `git diff --check`, focused/full required checks, branch and pushed commit.
- Preserve unrelated dirty files and never claim a reviewer PASS from local
  green output, an idle pane, or a successful push alone.

### 7. Wrong vs Correct

Wrong: see the `herdr` command, assume the right pane is Claude, press Enter
on every confirmation, and let the executor continue into the next slice.

Correct: verify both pane identities and matching cwd, inspect each visible
command, approve only the bounded safe set, reject unsafe writes/history
operations, require the `我是Claude` handoff, and stop at the explicit review
and user-authorization boundary.
