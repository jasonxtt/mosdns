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

The local Codex task is the controller; execution defaults to the current
conversation and may use a user-selected native sub-agent, Herdr, or
browser-backed DSH Web adapter. No provider is selected from host surface, and
retired MCP DSH is never dispatched.
The selected reviewer may be a ChatGPT web conversation or the current Codex
conversation when the user explicitly chooses self-review. The user remains
the authority for which executor/reviewer is used and whether the next phase
may begin.

Planning and execution stay in the current Codex conversation; a reviewer
selection does not transfer either authority. For the `c2c-web` provider, a
dedicated reviewer binding is resolved only when there is no explicit
current-turn or persisted reviewer target, and only after the binding's
project, chat, and connector identity has been verified. The default binding
is separate from the ordinary C2C planning-session pointer; a missing,
changed, or ambiguous binding blocks rather than falling back to another chat.
This integration task uses the explicitly selected Codex bootstrap reviewer
`codex://threads/01a0d43d-d0aa-7401-af0f-2ca3a45ba519` (`002reviewer`); the
dedicated C2C web reviewer is enabled only after later host-level acceptance.

### 2. Roles and handoff contract

- Local Codex reads the repository, task artifacts, specs, and current review
  result; it controls the selected executor, independently verifies the scoped
  diff, checks the pushed commit, and reports the exact commit to the reviewer
  conversation. In explicit inline mode, Codex also performs the edits.
- The selected reviewer supplies the plan/review decision. A ChatGPT web
  conversation must review the GitHub commit and repository evidence; an
  explicitly selected current-Codex reviewer performs the same evidence-based
  self-review locally. Either must return an explicit PASS or FAIL and identify
  the next authorized scope. An active run may advance only to the next unit
  already frozen in the user's authorization snapshot; review never authorizes
  work beyond that range.
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

Review delivery is atomic. Compose all requirements, evidence, prohibited
actions, and the requested decision in one complete, self-contained message
and send it once for that review attempt. While the reviewer is thinking,
pending, idle, silent, or producing partial output, do not send supplemental,
follow-up, or correction messages and do not interrupt the turn. If bounded
waiting plus platform evidence confirms that the conversation is stuck or its
transport is dead, resend the exact previous complete request unchanged; this
is a retry, not an opportunity to append information. Compact re-review
requests are permitted only after an explicit reviewer result, normally a
scoped `FAIL`.

For `c2c-web`, the atomic request uses `MODE: REVIEW_ONLY`, `STATE: REVIEW`,
and `CONTROLLER: TRELLIS`, includes exact committed parent/head SHAs and
changed paths, and carries no diff or file body. The web reviewer may inspect
the range only through read-only `git_compare`. Its `PLAN`, `DONE`, `BLOCKED`,
or iteration state is transport content and never changes Trellis lifecycle,
scope, or verdict authority.

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
  modify code, start another task, archive the task, or claim acceptance.
- If bounded waiting and platform evidence confirm a stuck/dead conversation
  or transport, resend the exact previous complete request unchanged; never
  send a supplement or correction. If the retry also fails, stop under the
  reviewer transport-failure rule.
- When the reviewer returns a scoped `FAIL`, Codex automatically applies only
  the requested remediation, runs the focused checks, inspects the exact diff,
  stages exact paths, commits, pushes, and sends a new review request to the
  same confirmed conversation. It then resumes the one-minute wait/read loop.
- When the reviewer returns a `FAIL` that requires a scope change, missing
  evidence, user choice, or a different conversation, Codex stops and asks the
  user. It must not widen the diff or change review destinations on its own.
- When the reviewer returns `PASS`, Codex records the result and advances only
  within an active pre-authorized run. A final PASS stops at the authorized
  boundary; PASS never authorizes a new task, production, deployment, or
  unrelated scope.

### 4. Validation and error matrix

| State or event | Required action |
| --- | --- |
| No review conversation or ambiguous destination | Ask the user to select/confirm the conversation; do not send |
| Reviewer returns FAIL with scoped fixes | Apply only those fixes, rerun focused checks, commit/push, and request another review |
| Reviewer returns FAIL requiring a scope change | Stop and ask the user; do not widen the diff |
| Reviewer is active or has not returned a formal result | Wait/read again; do not modify or start the next phase |
| Reviewer returns PASS | Record it; advance only to the next frozen unit, or stop at the approved boundary |
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
9. On PASS, advance only to the next unit in the already-authorized run. Do not
   run `task.py start` again, exceed the frozen range, wire production, deploy,
   archive/finish, or create a follow-up task.

The user's pre-start authorization decides the maximum range. Review PASS
authorizes only the next unit inside that snapshot and never implies
continuation beyond it.

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

## Conversation-scoped automation contract

Trellis keeps three separate concerns: the conversation automation context,
task authorization, and the active review loop. The context lives under
`.trellis/.runtime/automation/<context>.json` and contains only an optional
explicit `executor_override` plus a generic `reviewer` target. A fresh context
uses the current conversation as executor and has no reviewer; missing optional
providers never change that default or block read-only work.

An explicit target wins over the default, regardless of host surface or
provider availability. Herdr and DSH Web are narrow explicit adapters: their
`available`, `dispatch`, and `collect` operations run only when the user has
selected that provider and a host transport is supplied. Adapter discovery is
not performed by session-start or per-turn hooks, and an unavailable explicit
target is not silently replaced.

Task authorization snapshots the user-approved implementation units before
`task.py start`; activation after the task is `in_progress` creates the run
from that immutable snapshot. The automation run never writes task lifecycle
state. Review granularity comes from the authorized Slice range, not from the
executor provider or host surface.

Each authorized unit follows implement → validate → exact commit/push →
independent reviewer. The first request for each task/unit is a self-contained
bootstrap, and every review attempt is sent once as one complete message.
Same-task re-reviews may be compact only after an explicit reviewer result and
must remain pinned to exact parent/head SHAs; they are never mid-turn
supplements. If the reviewer conversation is confirmed stuck/dead, resend the
exact previous complete request unchanged. Only an explicit `FINAL: PASS`
advances to the next pre-authorized unit. Pending, partial, idle, or silent
responses are not PASS. A scoped FAIL may be remediated and resubmitted, but
the initial discovery is round zero and the same semantic root cause blocks
after five executed remediation rounds. Open findings, out-of-scope requests,
contradictory PASS results, corrupt run state, or transport failure fail
closed. Final PASS leaves the task `in_progress` and never archives, finishes,
starts another task, wires production, or deploys.

Legacy routing imports are compatibility-only. The deprecated shim forwards
context reads/writes to `common.automation` and raises for removed surface,
dispatch, or provider-policy operations. Existing Herdr and DSH Web discovery
names re-export their explicit adapter implementations so callers can migrate
without restoring host-aware routing semantics.

### Required regressions

- CLI/Desktop markers never select Herdr or DSH Web.
- Missing providers do not block the current conversation; explicit targets
  remain authoritative and provider type does not change Slice granularity.
- Legacy migration preserves only explicit user choices and leaves the source
  file byte-for-byte unchanged.
- A final reviewer PASS never archives or changes task lifecycle.

## Scenario: native DoH HTTP/2 child ownership

### 1. Scope / Trigger

Use this contract for pure Rust DoH HTTP/2 work in `rust/upstream-core`. Hyper's
low-level HTTP/2 client submits connection, request-send, and body-pipe futures
through a caller-supplied executor; using the default Tokio executor would make
those children invisible to owner shutdown.

### 2. Signatures

- `DohUpstream::exchange(ExchangeRequest<'_>, ExchangeContext)` remains one
  fresh numeric connection and one GET.
- The DoH response exposes `SecureHttpVersion::{Http1,Http2}`; DoT exposes no
  HTTP version.
- The private executor implements `hyper::rt::Executor<F>` for every tracked
  `Future<Output = ()> + Send + 'static` submitted by Hyper.

### 3. Contracts

- ALPN offers `h2,http/1.1` in that order. `h2` dispatches to
  `hyper::client::conn::http2`; negotiated or absent HTTP/1.1 dispatches to the
  inline HTTP/1.1 driver. No protocol fallback or request replay is allowed.
- Executor admission registers a child before `tokio::spawn` while the scope
  is unsealed. Teardown seals admission, aborts all registered handles, and
  waits until every child guard drops.
- A shared `Lifecycle` registration is retained by the executor state and its
  child tasks. Dropping the caller future cannot let `close().await` return
  before those children are gone.
- A validated HTTP/2 response is only a candidate until the executor scope has
  sealed and drained. The final lifecycle commit occurs after teardown and
  immediately before returning success, so close/cancel/deadline during drain
  cannot become a late success.
- HTTP/2 is one request per fresh connection in this foundation; pooling,
  multiplexing across callers, resolver/bootstrap, and HTTP/3 are separate
  tasks.

### 4. Validation & Error Matrix

| Condition | Required result |
|---|---|
| h2 response success | validated owned DNS wire, original ID restored, `Http2` metadata |
| RST_STREAM/GOAWAY/EOF before response head | typed terminal failure with `MaybeSent`, no retry |
| caller cancel/owner close after handoff | control error, child scope sealed and drained |
| executor submission after seal | future dropped, no new task or registration |
| unknown negotiated ALPN | typed `UnexpectedAlpn`, `NotSent`, no request |

### 5. Good/Base/Bad Cases

- Good: `TrackedH2Executor` owns an abort handle and lifecycle hold for every
  Hyper child, and `H2ScopeLease::finish()` waits for active count zero.
- Base: a successful response still seals and drains the one-shot h2 driver;
  it does not leave a reusable pool behind.
- Bad: pass `hyper_util::rt::TokioExecutor`, spawn an untracked connection
  driver, or release the exchange registration as soon as the caller future is
  dropped.

### 6. Tests Required

- Real TLS+h2 loopback success must assert service authority/path, original ID,
  HTTP-version metadata, and independent fresh owners.
- Reset, GOAWAY, EOF, caller cancellation, and dropped-future tests must assert
  side-effect state, zero in-flight registrations after close, and no second
  accepted connection. Reset tests must also count application streams on the
  same h2 connection, not only TCP accepts; `requests == 1` and
  `connections == 1` are required.
- A deterministic teardown barrier must park child draining before final commit
  and prove that owner close/caller cancellation/deadline wins; a real h2
  owner-close-after-handoff test is required as well.
- Executor unit tests must park a child, prove pre-spawn accounting, seal and
  drain it, reject post-seal work, and prove the shared lifecycle count reaches
  zero.

### 7. Wrong vs Correct

Wrong: spawn Hyper's h2 futures with `TokioExecutor` and let the parent
exchange's registration drop immediately when the caller aborts.

Correct: supply a sealed `TrackedH2Executor`, retain a shared lifecycle hold in
its state/children, abort and drain every registered child before releasing the
exchange's owner liveness.
