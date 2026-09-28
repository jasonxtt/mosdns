# Integrate Codex with ChatGPT as the default Trellis reviewer

Status: completed by project-owner acceptance on 2026-09-28. The owner reports
normal use for several days with no known issues and explicitly authorized
archival. The historical parent reviewer attempt remains as recorded in
`research/source-audit.md`; no formal parent-level reviewer `FINAL: PASS` is
claimed. See `research/owner-acceptance-2026-09-28.md`.

## Goal and user value

Make a separately bound `codex-with-chatgpt` ChatGPT web conversation the
default independent reviewer for Trellis runs, while keeping planning and
execution in the user-selected Codex conversation. An explicitly selected
Codex conversation remains a valid reviewer override. Trellis remains the
only controller of task planning, authorized Slice range, exact commit pair,
review verdict, remediation ledger, five-round limit, and auto-advance.

## Confirmed facts and revisions from the reference review

- The current repository already has a provider-neutral reviewer target,
  pre-start transport verification, exact `parent_sha`/`head_sha` submissions,
  explicit final parsing, scoped remediation, and a five-round same-root-cause
  limit.
- The current C2C Project mode intentionally allows a new Codex conversation
  to open a new ChatGPT chat and reports `reuseSavedChat=false`; ordinary
  `session.url` therefore identifies a planning conversation in that Codex
  thread, not a cross-thread default reviewer. A dedicated C2C reviewer
  binding is required.
- The current C2C MCP surface has only working-tree/index/HEAD `git_diff`
  modes; exact committed-range review requires a new read-only
  `git_compare(base_sha, head_sha, ...)` capability.
- This task itself cannot bootstrap through the new reviewer being built. The
  user must explicitly bind a reviewer before its `task.py start`; for this
  run that reviewer is the selected ChatGPT conversation above, verified via
  the existing platform-native read contract. The dedicated C2C web reviewer
  is host-level acceptance for this integration and becomes the default only
  for later tasks after that acceptance.
- Existing unrelated dirty worktree changes must be preserved.

## Requirements

R1. Resolve the default reviewer only when review is required and no explicit
reviewer target is bound. Resolve it from the dedicated C2C reviewer binding,
not the normal planning `session.url`. Require stable project/chat/connector
identity; never guess by display name, create a random chat, or silently
switch after identity loss.

R2. Preserve precedence: a current-turn explicit target wins over a persisted
target, and a persisted explicit target wins over the C2C default. An explicit
Codex internal reviewer never invokes C2C resolution. Planning remains in the
current/user-selected Codex conversation.

R3. Add a reviewer-only request carrying task/unit, exact base/head SHAs,
exact changed paths, validation, scope, prohibitions, and an explicit final
verdict request. Do not paste diffs, logs, or file bodies. `FINAL: PASS` may
contain only a conclusion; `FINAL: FAIL` must include stable `P0/P1/P2/P3-n`
finding IDs, root cause, and `open`/`closed` status. The same substantive
finding keeps its ID across re-reviews; new IDs are only for genuinely new
findings. There is one final verdict line.

R4. Add read-only C2C `git_compare(base_sha, head_sha, path, offset,
max_bytes)` with exact two-commit semantics, containment/sensitive filtering,
pagination, size caps, and fail-closed revision/path handling.

R5. Keep verdict authority in Trellis. C2C protocol states and iteration
limits are transport content only; Trellis parses the stable finding ledger,
enforces exact SHA/remediation rules, and advances only within authorization.

R6. Add deterministic tests for dedicated binding resolution, explicit
override precedence, bootstrap gating, request/finding composition, stable
send/read transport, bounded ChatGPT polling, exact compare security and
pagination, and failure-closed handling. Mock only host transport/session and
Git subprocess boundaries; do not use live browser/API credentials in tests.

R7. Update Trellis and C2C documentation without changing MosDNS runtime,
YAML/config semantics, WebUI/API behavior, production deployment, or default
Rust release status.

## Parent responsibility and execution order

The parent has exactly one implementation unit: `Slice 0 — cross-repository
integration acceptance`. The external and local implementation units belong
only to their respective child tasks; their `## Slice N` headings are not
duplicated in the parent.

Execution order is therefore:

1. external child starts, completes its own Slices, and receives explicit
   bootstrap-reviewer PASS;
2. local adapter child starts after the reviewed external contract is
   available, completes its own Slices, and receives bootstrap-reviewer PASS;
3. parent starts for its single integration Slice, verifies both child
   commits, performs host-level C2C acceptance, and receives final PASS.

Child dependencies are written in their artifacts and are not inferred merely
from directory order.

## Acceptance criteria

- [x] A1: A dedicated configured C2C reviewer binding resolves to
      `provider=c2c-web`, preserves project/chat/connector identity, and does
      not reuse the planning session pointer; missing/ambiguous/changed
      identity blocks.
- [x] A2: The bootstrap reviewer is explicitly selected and verified before
      this task starts; for this run it is the referenced ChatGPT conversation
      above, and the new C2C default is not used to review its own
      construction.
- [x] A3: Explicit Codex or other reviewer targets remain authoritative and
      do not invoke C2C default resolution.
- [x] A4: Review messages are atomic, bounded, exact-range, body-free, and
      require one explicit PASS/FAIL; FAIL output is stable enough for the
      existing finding ledger and five-round limit.
- [x] A5: The web reviewer can inspect exactly `base_sha..head_sha` through
      bounded read-only MCP; invalid SHAs/path escapes/sensitive paths fail
      closed and dirty working-tree changes cannot alter the result.
- [x] A6: C2C `DONE`/`PLAN`/iteration state cannot bypass Trellis authority;
      pending/partial/silent responses are not PASS.
- [x] A7: Both child tracks and the parent host-level acceptance pass their
      focused/full checks. No MosDNS runtime/product file changes occur.
      Parent host acceptance is based on the owner's report of several days of
      normal use with no known issues; this does not create a missing formal
      reviewer verdict.

## Out of scope

- Browser automation, DOM scraping, unofficial ChatGPT APIs, token/cookie
  handling, and credential storage.
- Replacing the Trellis lifecycle or adding a parallel C2C controller.
- Using the normal C2C planning session as the default reviewer binding.
- Default planning through ChatGPT web.
- Automatic reviewer fallback or silent identity switching.
- Production deployment, Rust-native runtime changes, and WebUI/API work.

## Approved scope and task map

The user approved modifying both repositories. The parent has two children:

- `09-24-trellis-c2c-web-compare`: external binding, exact compare, and
  reviewer-only protocol.
- `09-24-trellis-c2c-reviewer-adapter`: local Trellis adapter, precedence,
  transport, workflow/spec integration.

The parent owns cross-child acceptance, host-level C2C verification, and the
final integration review. The task uses the explicitly selected reviewer
conversation as bootstrap; only after the parent acceptance is the dedicated
C2C reviewer the default for subsequent tasks.
