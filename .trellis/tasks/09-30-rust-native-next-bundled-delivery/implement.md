# Implementation plan — native audit control, DNS card and audit panel

Planning only. Do not run `task.py start`, edit product code, deploy, or claim a
final implementation PASS from this planning turn. The next execution agent
must read `AGENTS.md`, the Rust migration documents, `.trellis/workflow.md`,
this PRD/design and the backend spec index before code changes.

## Start and review gates

- [ ] Confirm the final PRD summary and every `Proposed decisions` choice
  with the user: direct capacity zero, strict capacity JSON body and range,
  log limit 500, persist-before-publish with 5xx on write failure,
  terminal-time single capture sample and the windows endpoint. Obtain a
  separate implementation approval after planning review. The task remains
  `planning` until then; a direction choice alone is not approval.
- [ ] Refresh the exact `rust` HEAD, `task.py current/list`, archived prior 5C
  evidence, parent roadmap and dirty-path inventory. `docs/rust/next-stage-plan.md`
  and parent roadmap documents have pre-existing uncommitted work; do not
  stage or overwrite those documents. The new child's eight task artifacts
  and the exact child-link change in parent `task.json` belong together in
  the planning range. Trellis auto-commit remains disabled.
- [ ] Before implementation approval, validate this child and the parent
  task metadata with `task.py validate`, run `git diff --check`, and inspect
  the precise planning diff: eight child artifacts plus only the new child
  link in parent `task.json`. Never stage all of `.trellis/tasks` or use
  `git add .` in this dirty worktree.
- [ ] Slice 0 freezes source-level contracts from the scoped v1 handlers in
  `coremain/api_audit.go`, `api_audit_v2.go:16-205`,
  `audit.go:205-245,698-844,918-1320`, `state_files.go:41-110`,
  native `observer.rs`, and the current Vue dashboard/System callers. Record
  exact JSON samples, route method/body/type, named window definitions and
  settings precedence under `research/` before implementing their handlers.
  Follow the frozen root-name, unknown-qtype, timestamp and IPv4-mapped
  address projections in design. Any new product
  deviation returns to the PRD and planning review.

## Slice 0 — dynamic audit store and deterministic capture

- [ ] Add public host/listener tests first for static `enable_audit=false`,
  runtime capturing default true, start/stop at an in-flight request barrier,
  capacity 0/1/2 eviction, clear linearization and always-on host metrics.
  Use real UDP and TCP requests; control only a network peer or scheduler
  barrier. Make each newly introduced or changed contract fail first; keep
  already satisfied static gate/eviction behavior as regression coverage.
  Runtime terminal gate, zero capacity, resize/clear and metrics separation
  require new RED evidence.
- [ ] Refactor `QueryObserver` and `HostOptions` to separate lifetime metrics
  from runtime audit store/control. Admission keeps facts for statically
  eligible requests; terminalization samples capturing once and publishes a
  whole record. Keep DNS locks short; no metrics reset from audit controls.
- [ ] Run the focused observer, UDP/TCP and existing native-host management
  regressions on `mosdns-rust`; prove real DNS progress while repeated read
  requests run against a near-full 400000-record ring. Choose a bounded read
  strategy without full-record clone for a small log page.

## Slice 1 — v1 HTTP controls and persistent capacity

- [ ] Add real HTTP+DNS tests for v1 GET/POST methods, response status/body/
  content type, unknown paths/methods, start/stop, clear, capacity zero and
  `0..400000` validation, required integer-only JSON body and rejection of
  unknown fields. Assert old domain-set API behavior still passes.
- [ ] Give `ApiServer` a narrow observer/control handle and retain the
  top-level config base directory in the host. Do not let the API own DNS,
  cache or upstream lifetimes. File-backed config supplies the settings root;
  in-memory test assembly uses an explicit temporary root.
- [ ] Add settings tests: default 100000, canonical `webinfo` precedence
  even when malformed, valid legacy `state/` and root locations, malformed
  JSON/type fallback, missing/null capacity as zero, unknown fields accepted,
  saved out-of-range clamp, saved zero, fresh-host restart, temp-write failure
  and final-replace failure. Keep file loader separate from strict POST parser.
  Verify old file bytes, old runtime capacity/ring, no temp leftovers and a
  5xx failure. A slow injected write must not stall real DNS readers.
- [ ] Implement serialized persist-before-publish on a blocking pool using
  same-directory replacement and one clear/capacity publication step. Reuse
  the existing supervisor; test both listener cleanup/rebind and a running
  side failure. Do not promise crash/power-loss durability.

## Slice 2 — bounded v2 read endpoints

- [ ] Add HTTP tests for retained-ring `/stats`, `/stats/windows`, and
  `/logs?page=&limit=` with real DNS requests. Show stop/clear/resize/eviction
  change audit numbers while `MetricsSnapshot` continues growing. A recent
  window and an incomplete window must report truthful coverage; empty ring
  must omit `coverage_start`. Add a narrow
  injectable test clock for both record admission wall time and window now;
  prove cutoff equality, before/after cutoff and eviction-driven completeness
  using real DNS requests.
- [ ] Pin JSON projection for qname, client IP, qtype, timestamp and elapsed
  duration using source-characterized examples. Test newest-first order,
  default/invalid pagination, page 1/page 2/out-of-range, empty ring and Vue
  limit 160, accepted limit 500 and explicit 400 for 501. Reject every
  unsupported filter/search key rather than silently
  ignoring it. Keep JSON encoding and expensive calculation off observer locks.
- [ ] Implement only the bounded serializer/handlers. No rank, complete
  AuditLog, trace ID, answer list, `/metrics` or Go-runtime metric substitute.

## Slice 3 — maintained Vue to real DNS

- [ ] Build and serve an exact-source disposable Vite copy with the real
  native API; verify `coremain/www` in the main worktree remains byte-identical.
  Drive known real UDP/TCP DNS requests and check the DNS card's count,
  current rate, latency, recent logs and time-window popover against API/ring
  facts. The card must have no audit API warning; record unrelated Overview
  rank 404/page error as deferred. Use a stable no-new-query observation point
  when comparing separately fetched responses. No mocked HTTP or DNS answers.
- [ ] Use the System audit panel to stop/start/clear/change capacity. Verify
  DNS keeps answering while stopped; the audit card pauses and resumes; clear
  resets; capacity rereads and survives a fresh host. Cause one controlled
  HTTP action/transport failure and confirm visible UI error without an
  optimistic state claim. Slice 1 alone proves actual persistence failure
  transaction semantics. If unrelated
  System reload calls block the panel, isolate that section's failure state
  in the smallest Vue change, without synthesizing other APIs.
- [ ] Record the exact fixture, ports, user actions, HTTP bodies, DNS oracle,
  file bytes, restart and process/socket cleanup under `research/browser-proof/`.
  Do not claim full OverviewManager/System/QueryManager compatibility.

## Final validation and handoff

- [ ] On the `mosdns-rust` SSH alias only, run the relevant focused Rust tests
  after each slice and one final `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, native host build and bounded real-process HTTP+DNS
  functional proof against isolated files/ports. Run `npm ci` and
  `npm run build` only in a disposable source snapshot on that VM; do not run
  frontend and Go builds in parallel. Run native and Vite on VM loopback,
  proxy VM Vite to VM API, and connect local Chrome through a task-owned SSH
  tunnel. Record tunnel PID/ports/source revision and clean it up. No live
  `/cus/mosdns` or production service.
- [ ] Re-run full validation only after a substantive code correction, not
  after documentation-only edits. Record candidate SHA, actual commands,
  result counts, failures/corrections, asset cleanliness and verified cleanup.
- [ ] Update only proven C08/C11 subitems in
  `docs/rust/feature-coverage.md`; do not mark whole rows or 5C complete.
  Preserve unrelated dirty paths. Run `git diff --check`, inspect exact
  product/task changes, stage exact paths and make coherent commits with
  Trellis auto-commit disabled.
- [ ] Send the exact committed base..HEAD, path list and evidence locations to
  the agreed reviewer. Correct real findings and repeat until explicit final
  PASS. Keep the task `in_progress` until the user directs completion/archive;
  do not deploy or replace the default binary.

## Known risks and rollback points

1. Dynamic capturing can be implemented incorrectly by checking at admission
   or by pairing a terminal-time flag with a missing audit context. The
   in-flight barrier tests are the gate before HTTP work.
2. Deriving v2 stats from lifetime metrics yields plausible but wrong counts.
   Ring eviction/clear/stop tests must prevent that shortcut.
3. HTTP snapshots of a 400000-entry ring can stall DNS if copied/serialized
   under the observer mutex. Keep lock scope measured and review a bounded
   concurrent-load proof before enabling the API.
4. Capacity write before publication has a failure boundary at final rename.
   Retain old bytes/state and remove temp files on all failed steps.
5. Vue pages make unrelated API calls. A bounded card/panel PASS must not be
   reported as complete System, Overview or QueryManager compatibility.
