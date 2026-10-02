> Historical product planning or evidence snapshot. Lifecycle status alone is not acceptance PASS. See [validation summary](../../../validation-summary.md). Raw logs and local workflow state are retained outside Git tracking.

# Closure review — 2026-10-01

Reviewed delivery HEAD `1e7093225b429ca99024f3693464538134ce7aa4`
against the task PRD/design and recorded execution evidence. Read the executing
Codex thread and its final C2C review; the recorded reviewer returned FINAL: PASS
for the remediation range `dc91c3ff..1e709322`.

## Initial result at 1e709322: do not archive yet

### P1 — Secure reuse busy admission has no host fresh-exchange path

`rust/native-host/src/assembly.rs:636–651` handles
`Backpressure(NotSent)` with an owner-managed fresh exchange only for TCP.
The DoT and DoH branches directly return `SecureReuseOwner::exchange` and
`DohReuseOwner::exchange` failures, respectively.

Both secure owners reject an overlapping exchange when their serial lease is
occupied (`rust/upstream-core/src/reuse.rs:1244–1247` and `1948–1951`).
`PoolError::Busy` maps to `UpstreamError::Backpressure(NotSent)` and
`secure_error` preserves that transport error. Therefore, overlapping queries
to one configured DoT/DoH entry fail locally instead of making progress via
the narrowly authorized fresh connection. This violates PRD R6/A6 and the
design's busy-admission contract. This is a source-confirmed missing path;
this closure review did not run a new runtime reproduction or rerun builds.

Required follow-up: add scoped, parent-owned one-shot DoT/DoH exchanges only
for typed pre-send busy admission. Preserve destination, TLS/service identity,
caller deadline and cancellation; parent close must drain those exchanges.
Add native-host tests with a barrier holding the first request in flight,
then prove a second request succeeds through a separate connection for DoT
and DoH (including supported HTTP protocols), followed by close/drain checks.
Keep arbitrary runtime and sent failures non-retriable. Obtain a new review
before closure.

The task and executing conversation remain unarchived. Existing source and
unrelated worktree changes were preserved.

## Re-review at 601b65a4: original P1 closed; eligible for archive

Reviewed exact remediation range
`1e7093225b429ca99024f3693464538134ce7aa4..601b65a45e10b8b534fda5c5052ff310815dec66`.
Both secure owner variants now retain a matching one-shot secure upstream.
Only `Backpressure(NotSent)` enters that path, using the unchanged context;
owner close concurrently drains both reuse and fallback owners.

The regression fixture holds the first request pending until a second physical
connection delivers its query, covering DoT, DoH HTTP/1.1 and HTTP/2. The test
asserts second-request success and subsequently exercises host shutdown/drain.
Read the dedicated reviewer conversation's completed response for this exact
range: `FINAL: PASS`.

Independent check: local and isolated VM assembly/test SHA-256 hashes match.
On `mosdns-rust:/root/mosdns-rust-forwarding.8UEU53/rust`, reran
`cargo test -p mosdns-native-host --test slice9_forwarding
secure_busy_admission_uses_a_fresh_connection_for_dot_and_doh -- --exact`:
1 passed, 0 failed. Also reran the complete `slice9_forwarding` integration
test target: 10 passed, 0 failed. Committed-range whitespace check also passed.

Executor reports fmt, clippy, native-host tests, workspace library tests and
native build passing. Full workspace integration execution encountered remote
disk exhaustion/linker Bus error; it is not recorded as passing. Prior full
delivery validation and UI/browser evidence remain in execution-evidence.md.
No new blocking finding was identified in this targeted re-review. Archive
authorization comes from the user's earlier conditional archive request.
