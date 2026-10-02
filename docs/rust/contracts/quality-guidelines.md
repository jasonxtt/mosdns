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

## Audit rendering and benchmark evidence buffering

### 1. Scope / Trigger

Audit-enabled admission and Phase5A client ledger I/O are per-query paths.
Remove avoidable work without treating source-level improvements as measured
tail-latency fixes. Old measurement artifacts remain immutable.

### 2. Signatures

`QueryObserver::admit` renders parsed qname only with audit enabled.
`requestLedgerWriter.write(requestRecord) error` and `close() error` retain
their interfaces; no new flags or configuration values.

### 3. Contracts

Rendering preserves root/trailing dot, escaped dot/backslash and three-digit
decimal nonprintable-byte encoding; one final-sized String, no label Vec or
formatted temporary strings. Ledger buffers at most64KiB under its existing
mutex, persists full buffers and flushes then syncs/closes at normal completion.
JSONL content/record counts and sender rules stay unchanged.

### 4. Validation & Error Matrix

Buffered write/flush/sync/close failures invalidate the run; closed writer
rejects writes. Abnormal process termination may omit pending buffered records;
missing/incomplete evidence is never a PASS. No dropped schedule slot is excused.

### 5. Good / Base / Bad Cases

Good:20concurrent records stay buffered then close writes exactly20valid rows.
Base:400records cross64KiB and all remain readable after close.
Bad:closed backing file causes final flush error rather than successful stage.

### 6. Tests Required

Admission tests assert output/capacity for root, multi-label and escaped names,
plus metrics/eviction. Ledger tests assert concurrency, bounded full-buffer
flush, final flush error and post-close rejection; run Go race/vet and native
W1/W2/W3 regressions before review.

### 7. Wrong vs Correct

Wrong: buffer records but ignore Flush errors, then claim sender jitter fixed.
Correct: preserve error/oracle gates, rebuild/pin revised helper and candidate,
and require separately frozen measured evidence before a performance claim.

M8 post-remediation validation retains M7's exact nine-run100QPS30s rules.
Rebuilt helper must be identical across old/off/on variants and both hosts;
artifact generation filenames need not change its CLI interface version.
Pin source/binary hashes, verify staged tracked Rust files, use new result
roots and never overwrite old evidence or replace an old failed attempt.

## Fixture answer IDs must match the helper contract

### 1. Scope / Trigger

When a real-host driver launches Phase5A fixtures, listener readiness alone
does not establish that the fixture implements the intended answers/routes.
M9 accepted underscore IDs and returned NXDOMAIN for every W3 query.

### 2. Signatures

`phase5a-baseline-helper fixture --upstream-id ID --network udp --addr ADDRESS`
feeds `fixtureAnswer(upstreamID, qname, qtype)`. Controller fixture specs must
use the exact IDs recognized by that helper function.

### 3. Contracts

Current recognized IDs:forward,cache,route-a,route-b,route-c. Routing IDs use
hyphens; counter filenames/journal identities must match. A socket bind and
affinity check do not validate the answer contract. No DNS readiness warm-up
is allowed before a cold-cache measurement.

### 4. Validation & Error Matrix

Unknown ID may still bind successfully but returns the fallback NXDOMAIN;
response/routing/counter oracles then fail and block acceptance. Wrong fixture
answers are measurement defects, not a candidate performance result. Stop a
major mismatch and preserve failed/unstarted slots without silent replacements.

### 5. Good / Base / Bad Cases

Good:route-a appears in helper switch and controller specs. Base:cache uses
its recognized identity without pre-warming. Bad:route_a binds yet cannot
answer the expected W3 cases.

### 6. Tests Required

Check controller fixture IDs independently against helper answer dispatch
cases; the M9 regression is RED for underscore IDs and GREEN for hyphens.
Existing response, ordered-route journal and per-upstream counters must pass
on a separately authorized fixed batch before measured acceptance.

### 7. Wrong vs Correct

Wrong: treat successful UDP bind as proof that route_a works, then repeat
failed measurements until passing. Correct: verify exact helper identities,
retain invalid evidence, repair offline and authorize/pin any new traffic batch.

## Native distributed W3 offline routing evidence

### 1. Scope / Trigger

A native-only before/after W3 batch uses separate client/server clocks and the
legacy shared-clock/barrier oracle cannot validate its retained journals.

### 2. Signatures

`m10-route-oracle.py --raw-root PATH --result-root NEW_PATH --workload PATH
--cleanup-proof PATH --postbatch-identity PATH` derives separate evidence.

### 3. Contracts

Join each unique client DNS ID plus exact question to every upstream event;
require complete request/fixture sequences and frozen A, B→A, B→C order.
Validate client intervals at nanosecond precision on the client clock only.
Preserve original FAIL, rows and timestamps. Verify reviewed tool/config/binary
identities, all other oracles, manifests and unchanged latency/count gates.

### 4. Validation & Error Matrix

Reused IDs, gaps, duplicates, unmatched/extra/missing events, question/run/count
mismatch or wrong route reject. Only exact known ESRCH cleanup diagnostics may
use a complete independently verified PID/start exit receipt. Any other error
blocks. Handle ProcessLookupError at initial /proc read and post-signal polling.

### 5. Good / Base / Bad Cases

Good: complete unique-ID proof without clock adjustment. Base: original failed
oracle remains in the bundle. Bad: assume matching qnames imply identity, shift
server timestamps, overwrite runner failure, or waive missing cleanup evidence.

### 6. Tests Required

Cover unsynchronized clocks, unique-ID ambiguity, path/order and completeness
tampering, nanosecond intervals, and both /proc exit races. Reuse original batch
only with explicit final review of the independent proof; no automatic rerun.

### 7. Wrong vs Correct

Wrong: rename the original driver verdict PASS. Correct: keep it FAIL and submit
separate reproducible offline proofs and exit receipts for bounded acceptance.
