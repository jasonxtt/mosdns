# Design — Simplify Trellis Execution and Review Automation

## 1. Current-State Map (from code inspection at baseline 1489ebf)

### 1.1 Where routing lives today

| Component | Role today |
|---|---|
| `.trellis/scripts/common/codex_routing.py` | v2 state at `.trellis/.runtime/routing/<ctx>.json` (`surface`, `executor`, `reviewer` slots), v1→v2 migration, `detect_surface`, `resolve_codex_provider`, Herdr/DSH Web discovery, validity/invalidation, `selection_prompt` |
| `.trellis/scripts/codex_routing.py` | CLI: `show/discover/validate/prompt/set-inline/set-surface/set-executor/set-reviewer/clear-*/invalidate` |
| `.trellis/scripts/common/config.py` | `get_codex_dispatch_mode` (default `auto`), `get_codex_host_routes` (`cli→herdr`, `desktop→ask`, `unknown→ask`) |
| `.trellis/scripts/common/workflow_phase.py` | `resolve_effective_platform`: codex → `codex-sub-agent/-inline/-herdr/-dsh-web/-auto` virtual platforms; consumes `dispatch_mode` + `host_routes` |
| `.codex/hooks/inject-workflow-state.py` | per-turn: `_codex_mode_banner`, `_codex_routing_banner`, `resolve_breadcrumb_key` (status+provider → `*-inline/-herdr/-dsh-web/-auto` tags) |
| `.codex/hooks/session-start.py` | session orientation; does not itself run discovery (verified) but emits workflow text that references routing |
| `.trellis/scripts/common/git_context.py` | `_resolve_codex_routing()` for `--platform codex` phase filtering |
| `.trellis/scripts/common/task_store.py` | uses `get_codex_dispatch_mode(repo_root) == "auto"` to decide jsonl seeding |
| `.trellis/workflow.md` | `[workflow-state:*]` provider-variant blocks + `[codex-herdr]` / `[codex-dsh-web, codex-auto]` platform-marker sections |
| `.trellis/spec/backend/quality-guidelines.md` | "Host-aware Codex routing" contract section |
| `.trellis/tests/test_codex_routing.py` (318 lines), `test_codex_hook.py` (93 lines) | lock in the old behavior |

### 1.2 Root design flaw

Three orthogonal concerns were merged into one state machine:

- **who executes** (a transport choice),
- **what may run** (user authorization),
- **where the task is** (Trellis lifecycle).

Surface detection (a read-only environment fact) became an authorization
input, and provider type became a review-granularity input. The redesign
separates the three and deletes surface/host routing entirely.

## 2. Target Architecture

### 2.1 Three small concepts (and only three)

**(A) Conversation automation context** — conversation-scoped, gitignored,
one JSON file per conversation under a new directory
`.trellis/.runtime/automation/<context>.json`:

```json
{
  "version": 1,
  "context_key": "codex_<id>",
  "executor_override": null,
  "reviewer": null,
  "updated_at": "..."
}
```

- `executor_override = null` ⇒ effective executor = current conversation.
  There is no persisted "current" value to store.
- `executor_override` / `reviewer` are generic targets:
  `{provider, reference, label?}` (+ free-form `metadata` allowed for
  adapters, e.g. Herdr workspace/pane). No `surface`, no `selected_by`
  policy semantics beyond provenance metadata; selection legitimacy is
  enforced by who can write the file (the user-facing CLI/commands), not by
  stored policy.
- **Migration metadata**: after a one-way legacy read the context records
  `migrated_from: {path, sha256}` as a source fingerprint; the legacy file
  itself is never renamed or deleted (see §2.5).
- Effective-executor resolution is a pure function:

```
resolve_executor(ctx) =
  ctx.executor_override if set and provider supported else CURRENT
```

  Unavailability is detected at dispatch time by the provider adapter, not
  pre-computed into routing policy.

**(B) Active automation run** — runtime state (gitignored), one per
conversation (or per task; keyed by context):

```json
{
  "version": 1,
  "task": ".trellis/tasks/09-21-...",
  "authorized_units": ["Slice 0", "Slice 1", "Slice 2", "Slice 3"],
  "authorized_at": "...",
  "current_unit": "Slice 0",
  "reviewer_bootstrap_sent": false,
  "auto_advance": true,
  "auto_remediate": true,
  "max_same_finding_rounds": 5,
  "auto_finish": false,
  "status": "running | blocked | authorized_scope_complete",
  "units": {
    "Slice 0": {
      "phase": "pending | implementing | ready_for_review | awaiting_review | remediating | passed",
      "submission": {
        "parent_sha": null,
        "head_sha": null,
        "review_round": 0,
        "request_kind": null,
        "submitted_to": null
      },
      "findings": {
        "P1-1": {"root_cause": "...", "failed_remediation_rounds": 2, "status": "open"}
      },
      "result": null
    }
  }
}
```

- `authorized_units` is a **snapshot** taken from `implement.md` (or the
  user's explicit list) at authorization time. Later edits to `implement.md`
  never extend it.
- **Durable per-unit review state** (this is what makes compaction/restart
  recovery idempotent): `phase` distinguishes not-started vs implementing vs
  submitted-and-waiting vs remediating vs passed; `submission` pins the exact
  parent/head SHAs, the review round, whether the request was a full
  `bootstrap` or a compact `rereview`, and which reviewer target it went to;
  `reviewer_bootstrap_sent` (run-level, per task) records whether the
  reviewer already received this task's full bootstrap. A fresh turn
  reconstructs "what am I doing?" from these fields alone: it must never
  re-send a duplicate review, re-review the wrong SHA, mistake a first
  review for a remediation, re-bootstrap a reviewer, or re-commit/advance a
  unit that is already recorded.
- `findings` implements the per-finding `failed_remediation_rounds` counters
  with the exact semantics of PRD §6.3: discovering a finding leaves the
  counter at 0; it increments only when a remediation was executed AND
  resubmitted AND the reviewer still fails the same root cause; BLOCKED at
  5. Identity is the reviewer's stable ID plus a normalized root-cause
  summary; a renamed ID with the same root cause maps to the same counter
  (semantic match performed by the controller when recording results — an
  LLM judgment recorded in state, not a fuzzy string algorithm).

**(C) Task lifecycle** — unchanged Trellis `task.json.status`
(`planning` / `in_progress` / completed-by-archive). Automation never writes
task status; `task.py start/finish/archive` stay the only writers.

### 2.2 Executor / reviewer model

- **Executor providers**: `current` (implicit, never stored, built-in), plus
  explicit adapters `herdr` and `dsh-web`. No other providers are added in
  this task — extensibility is a seam, not a deliverable, and speculative
  providers with no real capability (e.g. `codex-thread`) are out of scope.
- **Currently supported explicit providers must stay operational.** The
  existing Herdr helpers (`discover_herdr` / `parse_herdr_inventory` /
  candidate validation) and the DSH Web browser-endpoint helper
  (`discover_dsh_web` / reference normalization) are **moved** into their
  respective adapter modules — not deleted. The Slice 1/4 cleanup removes
  their *policy* role (auto-selection, surface routing, per-turn/session
  discovery), never their *transport* capability. A persisted
  `provider=herdr` override must still actually dispatch; schema without a
  working adapter is a regression.
- Each adapter is a thin module with `available(target) -> bool`,
  `dispatch(unit_prompt)`, `collect()`; the review loop around them is
  identical.
- **Reviewer providers**: default `chatgpt` (plain conversation reference),
  plus any user-specified provider. Same generic target shape.
- **Granularity rule**: review/dispatch granularity = user's authorized units
  from `implement.md`. Provider type never changes it (deletes today's
  Herdr-task-level vs native-slice-level split).
- **Discovery**: `herdr`/`dsh-web` discovery helpers survive only inside
  their adapters and run only when the user explicitly selected that
  provider. Nothing scans processes at session start or per turn.

### 2.3 Review loop (controller algorithm)

**Entry gate (fixed order, closes the planning-gate hole).** The loop's step
1 is reachable only after ALL of:

1. planning artifacts complete;
2. planning review PASS where the task requires it;
3. reviewer target resolved AND its transport verified usable (§2.4a) —
   before `task.py start`, never after;
4. user authorized the unit range → `authorized_units` snapshot persisted;
5. `task.py start` run exactly once AND `task.json.status == in_progress`
   confirmed by re-reading the file;
6. automation run created with `status=running`.

`authorize`/run-creation refuses while task status is `planning`. Automation
state never writes task status — `task.py start/finish/archive` remain the
only writers — but a run cannot start until step 5 has happened.

Per authorized unit, the controller (the current conversation, or the
explicit executor under controller supervision):

1. Load task artifacts + specs; implement only the current unit
   (RED→GREEN→refactor where applicable).
2. Focused validation + unit-boundary validation required by the task.
3. Inspect exact diff; stage exact paths; commit; push the task branch.
4. Verify remote tip == reported head SHA.
5. Send review request to the persisted reviewer:
   - first request of a task → full self-contained bootstrap (PRD §5.4 field
     list + automation-contract education);
   - re-review of same task → compact: previous findings, remediation
     parent/head, exact diff scope, validation, scoped re-review ask.
6. Wait for explicit `FINAL: PASS` / `FINAL: FAIL` / user-input signal.
   Idle/pending/silence/partial ⇒ still pending; never treated as PASS.
7. On FAIL:
   - all findings scoped & safe ⇒ auto-remediate (no user prompt), increment
     each finding's consecutive counter; counter reaching 5 with reviewer
     still failing it ⇒ `status = blocked`, ask user;
   - any finding out of scope / unsafe / contract-conflicting ⇒ immediate
     `blocked`, ask user;
   - reviewer self-contradiction/oscillation ⇒ immediate `blocked`.
8. On PASS: record unit result; if more authorized units → advance
   automatically; else `status = authorized_scope_complete`, stop, print the
   final report (units, SHAs, rounds, reviewer, validation), and do NOT
   archive/finish.

The loop is driven by the controller conversation turn-by-turn; the run-state
file is the durable memory across turns/compaction (§2.1(B)).

### 2.4a Reviewer transport contract (ChatGPT)

The reviewer is real I/O, not just a prompt template:

- The concrete `reference` comes from a platform-resolved `@` conversation
  target supplied by the user.
- Send/wait/read go through the host's **platform-native** ChatGPT
  conversation capability. Repo Python code never calls unofficial ChatGPT
  APIs and never browser-automates ChatGPT.
- A `verify_reviewer_transport(target)` check runs before `task.py start`
  (entry gate step 3): if the current host cannot send/read that plain
  ChatGPT conversation → major issue → ask the user.
- Transport failure never degrades into silent self-review; there is no
  fallback reviewer.
- In code this is a narrow adapter interface (`send(request)`,
  `wait_result(timeout)`, `read()`) with the platform-native implementation
  provided by the controller host; the repo-side contract, request builders,
  and result parser are unit-tested against a fake adapter.

### 2.4 Reviewer bootstrap content (first request per task)

Template sections: reviewer role; repo + branch + task dir; task goal;
relevant scope/contracts (paths to prd/design/implement); authorized unit
range; automation contract (the six bullets from PRD §5.4: review only the
submitted unit; scoped FAILs auto-remediate; PASS authorizes the next
pre-authorized unit only; PASS ≠ beyond-range authorization; final PASS ≠
archive/new-task/production/deployment); current unit; base/head full SHAs;
GitHub commit/tree URLs; exact changed paths; unit acceptance criteria;
validation evidence; explicit forbidden scope; required output format
(`FINAL: PASS` or `FINAL: FAIL` + stable finding IDs with closed/open
markers).

### 2.5 Migration (one-way, tested, rollback-safe)

New module function `migrate_legacy_routing(repo_root, context_key)`:

- Reads old `.trellis/.runtime/routing/<ctx>.json` (v1 or v2) exactly once.
- **Provenance-gated preserve**: a legacy target migrates automatically ONLY
  when its stored provenance is `selected_by == "user"`.
  - Legacy reviewer `selected_by="user"` + concrete reference → new
    `reviewer` target.
  - Legacy executor `selected_by="user"` + still-supported provider
    (`herdr`, `dsh-web`) → `executor_override`; `codex/current` ⇒ `null`
    (that is the default anyway).
  - **Ambiguous provenance — `migration`, `policy`, `auto`, missing, or
    produced by the legacy v1→v2 in-memory migration (which stamps
    `selected_by="migration"`) — is NEVER silently bound.** Such a reviewer
    is either surfaced to the user as a candidate requiring one explicit
    confirmation or dropped to `null` (⇒ the normal ask-once flow); such an
    executor becomes `executor_override = null`. The reviewer is precisely
    the role the user must confirm once, so silent migration of an
    unconfirmed reviewer is forbidden.
- `surface`, host-route results, MCP `dsh` ⇒ dropped, not represented in the
  new schema at all.
- **Legacy file is left in place byte-for-byte.** The new context records
  `migrated_from: {path, sha256}` (source fingerprint) and never reads the
  legacy file again. No rename, no `.migrated` suffix: renaming would make
  the old state invisible to a rolled-back checkout, breaking the promised
  rollback. Cleanup of legacy files is a separate future task.
- Unreadable/corrupt legacy state ⇒ treated as empty (`executor_override =
  null`, `reviewer = null`); never blocks.

### 2.6 Workflow / hook / config changes

- **`workflow.md`**: keep `[workflow-state:no_task|planning|in_progress|completed]`
  only. Delete the eight provider-variant blocks
  (`planning-inline/-auto/-herdr/-dsh-web`, `in_progress-...`) and the
  `[codex-herdr]` / `[codex-dsh-web, codex-auto]` platform-marker sections;
  fold any still-true guidance into the generic blocks. Rewrite the generic
  `in_progress` body to the target semantics (PRD §17): default current
  executor; explicit override wins; execute only authorized units; per-unit
  implement→validate→commit/push→review; scoped FAIL auto-remediation; PASS
  auto-advances; missing reviewer resolved once up front; no auto-finish;
  interrupt only on major issues or the 5-round limit.
- **`inject-workflow-state.py`**: `resolve_breadcrumb_key` returns the plain
  status for all platforms (provider/surface parameters removed or ignored);
  `_codex_mode_banner` / `_codex_routing_banner` collapse into one small
  automation banner: `executor=current|<override>; reviewer=<target|missing>;
  run=<none|task/current-unit/status>`. Missing reviewer ⇒ informational
  line only; planning/read-only work is not blocked.
- **`session-start.py`**: no discovery, no routing resolution; if an
  automation context exists, one line summarizing executor/reviewer/run.
- **`workflow_phase.resolve_effective_platform`**: codex maps to
  `codex-sub-agent` (existing default) or `codex-inline` only; herdr/dsh-web/
  auto virtual platforms removed along with their workflow.md blocks.
- **`config.yaml` / `common/config.py`**: `codex.dispatch_mode` simplified to
  `inline | sub-agent(auto)` semantics (project default becomes `inline` =
  current conversation); `host_routes` key removed; parsers tolerate its
  legacy presence with a deprecation warning, never acting on it.
- **`git_context.py` / `task_store.py`**: drop `resolve_codex_provider` /
  surface usage; `task_store` keeps jsonl seeding based on the simplified
  dispatch mode only.
- **`quality-guidelines.md`**: replace the "Host-aware Codex routing"
  contract with the new automation contract (three-state separation, explicit
  override precedence, reviewer bootstrap, remediation rules).

### 2.7 Compatibility shim

`common/codex_routing.py` + `codex_routing.py` become thin deprecated shims
forwarding to `common/automation.py` / `automation.py` for one release of
this task's lifetime (Slice 4 removes or reduces them). Shims hold no state
machine of their own: `load_state` forwards to the automation context (with
migration), `set_target(executor|reviewer)` maps to override/reviewer writes,
`surface`/`dispatch`/discovery APIs either raise `DeprecationWarning`-style
errors or no-op with a clear message, per the final implement plan.

## 3. Data Flow

```
user: "执行 Slice 0-3"
  -> gate: artifacts complete + planning PASS (if required)
  -> resolve reviewer target; verify transport usable (else ask once / stop)
  -> snapshot authorized_units from implement.md
  -> task.py start (exactly once); confirm task.status == in_progress
  -> create automation run (status=running)
  -> loop per unit (§2.3), persisting run state after every step
  -> status=authorized_scope_complete -> report -> stop
```

## 4. Error / Edge Handling

- **Corrupt conversation context** → fail to safe defaults
  (`executor_override=null`, `reviewer=null`); never blocks read-only work.
- **Corrupt active automation run** → fail CLOSED. The run carries the
  authorization snapshot, per-unit progress, and remediation counters;
  treating it as absent could bypass the authorized range or the 5-round
  limit. Unparseable run ⇒ `BLOCKED`: no implementation, no auto-advance,
  ask the user / require explicit recovery.
- Reviewer send/read failure or vanished target → `blocked`, ask user; never
  auto-swap reviewer, never degrade to self-review.
- Executor override target unavailable → ask user once; no silent fallback.
- Git safety conditions (force push, history rewrite, unrelated dirty paths,
  remote divergence) → stop and ask (§7 of PRD).
- Compaction-safe: all durable decisions live in the two state files; the
  in_progress breadcrumb + run file fully reorient a fresh turn.

## 5. Rollout / Rollback

- Slices land independently on branch `rust`; each Slice is reviewed.
- Rollback = revert the Slice's exact commit(s); legacy state files are
  preserved **in place** by migration (read-once + fingerprint marker in the
  new context), so a rolled-back old checkout still finds its routing state
  exactly where it expects it.
- Migration is lazy (runs on first automation load per conversation), so
  mixed old/new checkouts do not break.

## 6. Test Strategy (summary; full matrix in implement.md)

- Unit: context model defaults, override precedence, migration preserve/
  discard rules, run snapshot immutability, finding counters (incl. rename),
  5-round block, auto-advance, final-stop.
- Hook: breadcrumb key resolution ignores provider/surface; banner content;
  no discovery invoked at session start (assert via mocked subprocess never
  called).
- Negative regression: CLI marker ⇏ herdr; Desktop ⇏ dsh-web; missing
  Herdr/DSH Web ⇏ block; routing defaults ≯ explicit target; provider ≠
  granularity; final PASS ≠ archive.
- Existing suite: `test_codex_routing.py` / `test_codex_hook.py` rewritten
  against the new modules; full `.trellis/tests` run must pass.
