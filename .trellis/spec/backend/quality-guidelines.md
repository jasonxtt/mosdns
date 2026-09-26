# Quality Guidelines

## Change discipline

## Scenario: distributed Phase5A control evidence

### 1. Scope / Trigger

Split query generation and server sampling only under a prospectively reviewed
measurement revision. Keep old/new server slots on the same fixed server;
different client hardware is permitted. W1 qualification alone cannot pass A5.

### 2. Signatures

Helper v10 `run --sample-self` rejects `--sut-pid`/`--fixture-pid`. Server
`m5-remote-tools.py sample-server` brackets locally owned SUT/fixture PIDs;
`merge-stage` joins original host-local files offline. `run-m5-w1.py --mode
run --reviewed-head SHA` requires the approved current source commit.

### 3. Contracts

Client stage records `harness_host`, `harness_go_profile`, own PID/CPU and
`sut_pid=0`. Merged `server_resources` contains server host, PID/start/CPU,
counts and bounded bracket duration. Equal numeric PIDs across hosts are
valid. Both Go roles use GOMAXPROCS=1/GOGC=off/GODEBUG=gctrace=1 and absent
GOMEMLIMIT; sample RSS cap262144KiB. Original latency/counters stay unchanged.

### 4. Validation & Error Matrix

| Condition | Result |
|---|---|
| cohost evidence, PID reuse, affinity mismatch, missing role | invalid |
| first sample start differs from owned.json or remote source manifest mismatch | invalid |
| bracket outside25–35.5s, fewer25 samples per role | invalid |
| GC trace, wrong actual environment, RSS over cap | invalid |
| fixed order/input identity mismatch or oracle exit nonzero | invalid |
| original latency guard or 90% interval exceeds frozen margin | unqualified |

### 5. Good/Base/Bad Cases

Good: rebuild merged evidence from immutable client/server originals before
qualification. Base: controls use identical archived baseline in every slot.
Bad: read server PID from client /proc, subtract cross-host clocks, or treat
W1 control PASS as candidate acceptance.

### 6. Tests Required

Reject conflicting PID options; Linux loopback DNS self-sampling records only
the generator; readiness follows first sample and stop adds final sample;
merge preserves latency and separates identical numeric PIDs; short coverage
and wrong profile fail; PID ownership change refuses signal; exit race is benign.

### 7. Wrong vs Correct

Wrong: server PID passed to remote client sampling. Correct: client samples
itself; server samples its own processes; merge enriches separate namespaces.

## Scenario: repeated TCP session availability (M6)

### 1. Scope / Trigger

Repeated performance sessions can leave TIME_WAIT after successful shutdown.
Fix the probe before another prospective fixed control run; retain old evidence.

### 2. Signatures

`m6-server-control.py probe_available(address)` sets SO_REUSEADDR before bind.
`start(root)` creates a fresh owned evidence directory before input/port checks.
`run-m6-w1.py` uses disjoint M6 input/result roots and run IDs.

### 3. Contracts

No SO_REUSEPORT, listen, kernel tuning or service change in the probe. Empty
owned.json and startup-error.txt preserve prelaunch failures; client session.txt
preserves an empty query session. Error evidence includes captured remote
stdout/stderr. No credentials/full process environment. Reuse M5 sampling,
oracles, source manifests and unchanged numerical qualification rules.

### 4. Validation & Error Matrix

TIME_WAIT alone permits bind; active listener refuses bind. Wrong input/host
leaves failure evidence and sends no query. Existing result directory refuses
reuse. Remote failure remains invalid; missing windows are never fabricated.

### 5. Good/Base/Bad Cases

Good: fresh M6 roots, ordinary address reuse and owned cleanup. Base: both
stage windows stay within the same newly started server session. Bad: disable
port checks, enable SO_REUSEPORT, alter OS TIME_WAIT behavior, or overwrite M5.

### 6. Tests Required

Real Linux TCP active-close reproduces plain-bind errno98, then three reuse
probes succeed. An active listener still rejects; early startup failure retains
owned/error files; remote stderr survives;18 M6 IDs are disjoint from M5.

### 7. Wrong vs Correct

Wrong: ordinary bind treats closed TIME_WAIT sockets as occupied listeners.
Correct: SO_REUSEADDR allows closed sessions while active listeners still fail.

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

## Performance measurement self-control

When a frozen short probe repeatedly reports changing latency regressions
without a confirmed source cause, pin a bounded identical-binary diagnostic
before making another speculative correction. Preserve binary/config hashes,
slot order, every attempt, and the unchanged analyzer. Diagnostic pairing
labels must explicitly disclose when all slots use the same executable and
audit flag. The Phase 5A example command is
`bash run-slice3-self-control-w1.sh`; its `attempt-order.tsv`, `sut.json`,
`input-hashes.sha256`, and raw manifest provide the assertion boundaries.

If identical inputs cross the frozen guard, record a measurement limitation
and stop acceptance advancement under the major-issue rule. This neither
waives the candidate regression nor proves the candidate's overhead is zero.
Do not change old thresholds or select attempts until a PASS appears. A new
measurement protocol needs explicit authorization and review before acceptance
restarts. Source evidence: the active query-observability task's
`research/slice3-self-control-assessment.md` (W1 TCP 400 p95/p99 false crossings).

## Measurement clock sampling contract

1. **Scope:** the Phase 5A helper v9 resource sampler on Linux.
2. **Signatures:** `resourceClockTicksPerSecond() (int64, error)` resolves the
   host constant; `sampleProcessGroup(..., clockTicks int64)` passes it to
   `readResourceSample(pid int, hz int64)`.
3. **Contract:** resolve before `executeStage` records its start timestamp;
   reuse the positive frequency for every role/sample. M2's diagnostic
   `PHASE5A_MEASUREMENT_PROFILE=m2` requires `GOMAXPROCS=1`, Rust pilot mode,
   and helper v9. The runner records `measurement_profile` and
   `gomaxprocs_environment` in each `environment.txt`; qualification validates
   these fields per attempt. Legacy protocols keep their original behavior.
4. **Errors:** missing command, malformed, zero, or negative CLK_TCK fails
   before measured load; never silently substitute 100 on Linux.
5. **Cases:** valid frequency preserves CPU tick/second conversions; archived
   v8 remains selectable only with its pinned identity; per-target subprocesses
   during measured traffic are forbidden.
6. **Tests:** Linux sampling with a fake PATH clock command must not execute
   it; retain per-role RSS/FD samples. Exercise valid/invalid/missing host
   constant resolution. Missing/wrong M2 parallelism must fail before SUT
   launch. A single old latency guard crossing blocks control qualification
   even if pooled equivalence intervals pass. Preserve existing DNS/counter/event helper tests.
7. **Wrong vs correct:** spawning `getconf` from `readResourceSample` disturbs
   measured traffic. Resolve once before load and pass `hz` into the sampler.

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
supplements. A send exception is terminal for that transport instance and
never permits an immediate retry, even with the same message. If the host
provides bounded evidence that the reviewer conversation or transport is
stuck/dead, use the transport's evidence-gated replacement operation to create
a new verified transport and resend the exact previous complete request
unchanged; it may not append a supplement. Only an explicit
`FINAL: PASS` advances to the next pre-authorized unit. Pending, partial,
idle, or silent responses are not PASS. A scoped FAIL may be remediated and resubmitted, but
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

## Bounded low-load real-host regression (M7)

When a user explicitly approves simplified measurement, freeze the new scope
and numeric rule before candidate traffic. A100QPS W1 comparison is limited
regression evidence, not capacity/fullA5 or retroactive qualification of old
failed controls. Run actual pinned old/new-off/new-on binaries/configs on the
same server, with three balanced repetitions,3000 correct scheduled/sent/
received per run and zero errors/shortfall. Freeze paired median p95/p99<=1.10
before execution; CPU/RSS remain auxiliary. Preserve failed attempts, input
hashes, response/sender/session-counter oracles and owned process cleanup.
Incomplete evidence must fail closed. Unit tests cover exact ordering,
shortfall and repeated latency regression; one prospective and one result
review suffice for the authorized nine-run batch. Never silently reinterpret
this as high-load equivalence or rerun until a desirable result appears.
