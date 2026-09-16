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
7. Read or poll the conversation until it returns PASS, FAIL, or requires user
   input.
8. On FAIL, return to step 1 and repeat only the bounded remediation loop.
9. On PASS, stop. Do not automatically run task.py start, begin Slice 0,
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
