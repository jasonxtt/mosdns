# PRD — Simplify Trellis Execution and Review Automation

## 1. Context and Problem Statement

This is a **Trellis workflow / automation infrastructure** task, not a MosDNS
Rust runtime feature task. It changes only the project's Trellis control
plane: scripts, hooks, config, workflow text, and tests under `.trellis/` and
`.codex/`.

Today the project runs a heavy executor/reviewer **routing state machine** on
top of the official Trellis task/phase system (Trellis 0.6.14). The extra
machinery includes:

- Codex CLI / Desktop / unknown **surface detection** (`detect_surface`,
  `CODEX_SURFACE`, `CODEX_APP_TOOLS_PIPE_PATH`, `CODEX_CLI`, …);
- `codex.host_routes` (`cli -> herdr`, `desktop -> ask`, `unknown -> ask`);
- `auto / ask / herdr / dsh-web / inline` **provider routing** through
  `codex.dispatch_mode`;
- conversation-scoped executor/reviewer slots in
  `.trellis/.runtime/routing/<context>.json` (state version 2, with a v1
  migration path);
- Herdr / DSH Web **discovery** (`herdr agent list`, `ps` scan for
  `dsh web --port`);
- provider-specific `<workflow-state:*>` breadcrumbs:
  `planning-inline/-auto/-herdr/-dsh-web`,
  `in_progress-inline/-auto/-herdr/-dsh-web`;
- target validity / invalidation (`executor_validity`, `reviewer_validity`,
  `invalidate`);
- executor-type-driven dispatch and **review granularity** (Herdr = one
  task-level handoff, native = one behavior slice, etc.).

This design has produced concrete failures:

1. When the user explicitly asked "the current conversation executes
   directly", routing policy still blocked execution (fail-closed `ask`).
2. CLI/Desktop is **environment information**, but it is used to decide **who
   executes** — CLI silently means Herdr.
3. Herdr / DSH Web — which should be optional execution transports — are
   wired into the core Trellis state machine.
4. Executor/reviewer choice is coupled to task lifecycle, workflow-state, and
   review gates.
5. User control ranks below routing defaults.
6. A large amount of long-lived infrastructure exists only to serve past
   collaboration habits.

## 2. Goal

Keep Trellis' core — task directories, planning artifacts, spec system,
validation, finish/archive — and **remove "routing as a permission gate"**.
Replace it with a simple **automatic execution + independent review loop**:

```
Slice 0
  -> implement -> verify -> exact commit/push -> reviewer
  -> FAIL: automatic remediation -> re-review -> ... -> PASS
  -> automatically enter Slice 1
...
Slice N (last authorized)
  -> PASS -> STOP (no auto finish/archive)
```

The Slice range (e.g. Slice 0–3) is authorized by the user once, up front.
**After one Slice PASSes, the system must not ask again whether it may enter
the next pre-authorized Slice.** Only major issues interrupt the user.

## 3. Non-Goals (must NOT change)

- No changes to `rust/upstream-core` runtime behavior, Go MosDNS runtime,
  WebUI, DNS behavior, Phase 5A benchmark semantics, Cargo dependencies,
  production wiring, or deployment. (Exception: pure test-infrastructure path
  fixes required by Trellis hook/test moves.)
- Do **not** remove Trellis itself or its task/planning/spec/validation/
  finish core.
- Do **not** implement browser automation, UI scraping, or unofficial APIs to
  create a plain ChatGPT conversation. The first version asks the user to @ an
  existing ChatGPT conversation as reviewer.
- Do **not** rebuild a second complex state machine under a new name (see
  §11 Acceptance — anti-complexity invariant).

## 4. Executor Requirements

### 4.1 Default executor = current conversation

- The default executor is always the **current conversation** itself.
- The system must never ask "use the current conversation as executor?" when
  the user said nothing. Absence of an override means current.
- Fresh conversations must not need to persist `executor = codex/current`.
  The persisted field is `executor_override = null`, and `null` semantically
  means "effective executor = current conversation".

### 4.2 Explicit user selection wins

Priority order is fixed:

```
current-turn explicit user selection
> persisted explicit conversation override
> default current conversation
```

Examples that must all be honored: "let Herdr w6:p2 do it", "use DSH Web",
"let another Codex do it", "do it in this conversation".

**Invariant:** a user's explicit instruction in the current conversation to
use a specific available executor overrides all routing defaults. Routing
defaults MUST NOT act as authorization gates.

### 4.3 Override lifecycle

- An explicit override persists for the current conversation until the user
  changes it, restores current, or the target becomes actually unavailable.
- If the target is unavailable: ask the user once. **Never silently fall
  back** to another executor.

## 5. Reviewer Requirements

### 5.1 Default reviewer type

- Default reviewer provider: **plain ChatGPT conversation** (`provider =
  chatgpt`) — a normal ChatGPT chat (not a Codex thread, not Work; may live
  in the `mosdns` Project).
- Default provider ≠ concrete target. When a fresh conversation needs to
  start review-required implementation and no reviewer is bound, the system
  asks the user **once**: "this run needs an independent reviewer; default
  type is a plain ChatGPT conversation; please @ an existing conversation."
- A future `create_chatgpt_conversation()` capability may be added later as a
  reviewer adapter without changing the Trellis core. Not in scope now.

### 5.2 Generic reviewer model

The reviewer data structure is generic and not hard-wired to ChatGPT:

```
reviewer:
  provider: <provider>
  reference: <opaque reference>
  label: <optional>
```

Priority:

```
current-turn explicit reviewer
> persisted reviewer
> default provider chatgpt + ask for concrete target
```

### 5.3 Reviewer lifecycle

- Once chosen, the reviewer is reused for the whole conversation and across
  subsequent tasks in it; the user is not re-asked.
- Replacement only on explicit user request.
- Reviewer unavailable → interrupt the user. **Never silently swap
  reviewers.**

### 5.4 Self-contained first-review bootstrap

The reviewer conversation may contain nothing but a "hi". The controller must
not assume the reviewer knows any history, even if the conversation sits in
the `mosdns` ChatGPT Project.

The **first review request of every new task** must include at least:

- reviewer role; repository; branch; active Trellis task; task goal;
- relevant scope/contracts; user-authorized Slice range; automation contract;
- current Slice; base full SHA; head full SHA; GitHub commit/tree reference;
- exact changed paths; current Slice acceptance criteria; validation
  evidence; explicit forbidden scope; required PASS/FAIL format.

The bootstrap must explicitly teach the new automation contract:

- user already authorized Slice 0 through Slice 3 (the actual range);
- reviewer reviews only the currently submitted Slice;
- scoped FAILs are automatically remediated and resubmitted;
- PASS authorizes the controller to enter the next **pre-authorized** Slice;
- PASS does NOT authorize work beyond the original range;
- final Slice PASS does NOT authorize archive, a new task, production wiring,
  deployment, or unrelated scope.

**Re-reviews** within the same task are compact: previous findings, exact
remediation parent/head, exact diff scope, validation, scoped re-review
request. **A new task on the same reviewer conversation always gets a fresh
full bootstrap.**

### 5.5 Reviewer transport contract (the loop must be real)

A persisted `provider=chatgpt` reviewer target is only meaningful if the
controller can actually reach it:

- The concrete reference comes from a platform-resolved `@` conversation
  target supplied by the user.
- The controller sends requests and waits/reads responses through the
  **platform-native** ChatGPT conversation capability of the host it runs
  on.
- Repo Python code MUST NOT call unofficial ChatGPT APIs and MUST NOT
  browser-automate ChatGPT.
- Before `task.py start` (see §8.1), the controller verifies the target is
  actually send/read-able.
- If the current host cannot send/read that plain ChatGPT conversation →
  major issue → ask the user.
- Reviewer transport unavailable NEVER degrades into silent self-review.

### 5.6 Transport feasibility is a pre-implementation exit gate

The ChatGPT reviewer transport is a **required external capability**, not an
assumption. There is currently no verified evidence that the Codex host
exposes a send/read/wait action into a plain ChatGPT conversation, so this
capability MUST be proven before any implementation Slice mutates the
existing working routing system.

**Feasibility gate (runs before Slice 0, or as the first part of Slice 0,
strictly before any de-routing change):**

1. Resolve a real, user-selected plain ChatGPT conversation.
2. Perform a real host-level probe proving that the host can: send a message
   to that exact conversation; later read the reviewer response; and keep
   the target identity stable across the round trip.
3. No browser/UI automation and no unofficial API may be used for this
   proof.
4. Record the observed capability/API/tool contract as research evidence in
   this task.

**Outcomes:**

- Probe succeeds → continue with the planned ChatGPT reviewer adapter.
- Probe fails (the host exposes no supported plain-ChatGPT conversation
  transport) → **STOP before any de-routing mutation** and ask the user to
  choose a revised reviewer transport/design. Acceptable alternatives at that
  point (user's decision, never silently chosen by Trellis) include an
  official browser-use driver for the ChatGPT UI, a Codex detached reviewer,
  another reviewer transport, or a temporary manual relay.

The default reviewer requirement (plain ChatGPT conversation) is unchanged;
only the proof obligation is added.

## 6. Review Loop Requirements

Per authorized Slice:

1. read task artifacts and relevant specs;
2. implement current Slice only;
3. RED → GREEN → refactor while green where applicable;
4. run focused validation;
5. run Slice boundary validation required by the task;
6. inspect exact diff;
7. stage exact paths only;
8. commit; 9. push the requested branch;
10. verify remote contains the exact reported commit;
11. send review request to the persisted reviewer;
12. wait/read until explicit `FINAL: PASS` / `FINAL: FAIL` / user input.

None of these count as PASS: local green, push success, idle/pending/silent
reviewer, unchanged preview, partial response. Only an explicit
`FINAL: PASS` counts.

### 6.1 Automatic remediation of scoped FAILs

If every finding is inside the current Slice, fixable within the current
task/Slice authorization, needs no dangerous action, and needs no new product
decision, the controller **must not ask the user**. It automatically:
fix → focused validation → exact diff → exact commit/push → scoped re-review,
until PASS.

### 6.2 Finding IDs

The first review request asks the reviewer to use stable finding IDs
(`P1-1`, `P1-2`, …): same substantive finding keeps its ID across re-reviews;
closed findings are explicitly marked closed; new findings get new IDs. The
controller must recognize the same root cause semantically even if the
reviewer renames an ID — renaming must not bypass the counter.

### 6.3 Five-round same-finding limit (exact counter semantics)

The counter field is `failed_remediation_rounds`, per finding/root cause:

- The initial review that **discovers** a finding does NOT increment it —
  discovery leaves the counter at 0.
- It increments by 1 only when ALL three hold: a remediation for that root
  cause was actually executed, the resulting new commit was submitted for
  re-review, and the reviewer still judges the same root cause unresolved.
- When the counter reaches 5 and the reviewer still fails that finding, the
  run becomes `BLOCKED` and asks the user.

Worked example: initial FAIL (0) → fix #1 + re-review FAIL (1) → fix #2 (2)
→ fix #3 (3) → fix #4 (4) → fix #5 + re-review FAIL (5) → BLOCKED. This is
a per-finding consecutive counter, **not** a global "5 reviews per task"
cap. A new finding (P1-2) gets its own counter; a closed finding stops
counting. Tests must cover the initial FAIL plus exactly five remediations
to pin the boundary (blocking at 5, not at 4). Reviewer self-contradiction
or oscillating requirements is a major issue immediately — no need to
mechanically wait for 5.

### 6.4 Auto-advance

On `FINAL: PASS` the current Slice is recorded as PASS; if further
pre-authorized Slices exist, the run advances automatically (Slice0 → 1 → 2
→ 3 → STOP). No "may I start Slice 1?" prompts — that range was
pre-authorized.

### 6.5 Final PASS behavior

After the final authorized Slice PASSes:

- run status = `authorized_scope_complete`; then **stop and report**:
  completed Slices, each Slice's final reviewed SHA, remediation round
  counts, reviewer, validation evidence;
- task status stays as-is (e.g. `in_progress`);
- **no automatic archive**, no start of unauthorized Slices;
- wait for explicit user instruction (`finish`, `archive`, "continue
  Slice 4…", "start new task…").

## 7. Major Issues (interrupt immediately, no 5-round wait)

- **Scope**: reviewer demands changes outside the current Slice, a new
  unauthorized Slice, a new task, PRD changes, production wiring, deployment.
- **Contract conflict**: PRD/design/implement contradict each other;
  reviewer requirement conflicts with the task contract; fix needs a user
  product/architecture decision.
- **Git safety**: force push, reset/rebase/history rewrite, risky branch
  switch, overwriting unrelated dirty state, unresolvable remote divergence.
- **Security/external effects**: secrets, credential changes, production
  access, destructive external actions.
- **Reviewer failure**: reviewer target vanished, conversation cannot be
  sent/read, reviewer demands a different conversation, ambiguous review
  destination, or the §5.6 feasibility probe shows the host has no supported
  plain-ChatGPT conversation transport.

## 8. Authorization Model Requirements

Three kinds of state must stay separate — never again merged into one routing
state machine:

1. **Conversation automation context** (conversation-scoped, gitignored):
   `executor_override` (null = current), `reviewer` (generic target).
2. **Active automation run** (runtime state): task, `authorized_units`
   (snapshotted list), `current_unit`, `auto_advance`, `auto_remediate`,
   `max_same_finding_rounds` (default 5), `auto_finish: false`, `status`.
3. **Task lifecycle** (existing Trellis `task.json.status`: planning /
   in_progress / completed).

**Authorization snapshot**: "execute Slice 0–3" snapshots exactly those units
from `implement.md` at start; a Slice 4 added later is not implicitly
authorized. "Finish the current task" snapshots all implementation units
planned at that moment; later additions still need new authorization.

### 8.1 Activation order (planning gate is preserved)

An automation run may only be created after the Trellis planning gate, in
this fixed order:

1. planning artifacts complete for the task;
2. planning review PASS, where the task requires it;
3. reviewer target resolved (§5) AND its transport verified usable (§5.5) —
   this MUST happen before `task.py start`; a missing or unusable reviewer
   blocks here, never after start;
4. user explicitly authorizes the unit range → snapshot `authorized_units`;
5. `task.py start` exactly once → confirm `task.json.status == in_progress`;
6. only then create/start the automation run;
7. implementation begins.

A task still in `status=planning` must never enter implementation through an
automation run. Automation state never writes task status — `task.py
start/finish/archive` remain the only writers — but the run refuses to start
until `task.py start` has happened.

### 8.2 Run durability and fail-closed corruption handling

The run state must be sufficient to resume idempotently across turns and
conversation compaction. Per unit it durably records at least:

- a unit phase distinguishing: not started / implementing / ready for review
  / awaiting review / remediating / passed;
- the current submission: parent SHA, head SHA, review round, request kind
  (`bootstrap` vs `rereview`), and the reviewer target it was submitted to;
- whether the full reviewer bootstrap has already been sent for this task.

A resumed turn must be able to tell "review request already sent, still
waiting" from "not yet sent", and must never re-send a duplicate review,
re-review the wrong SHA, treat a first review as a remediation, re-bootstrap
a reviewer, or re-commit/re-advance a unit.

**Corruption policy differs by state kind.** Corrupt *conversation context*
fails to safe defaults (`executor_override=null`, `reviewer=null`) and never
blocks read-only work. Corrupt *active automation run* is fail-CLOSED: it
carries the user's authorization snapshot, per-unit progress, and remediation
counters, so treating it as absent could bypass the authorized range or the
5-round limit. A corrupt/unparseable run ⇒ `BLOCKED`: no implementation, no
auto-advance, ask the user / require explicit recovery.

## 9. Config / Workflow / Provider Requirements

- These semantics leave the core: `CLI -> Herdr`, `Desktop -> ask`,
  `unknown -> ask`, `host_routes`, surface-decides-executor, auto provider
  resolution.
- `codex.dispatch_mode` (if Trellis 0.6.14 still needs it) reverts to the
  simplest near-official semantics; project default recommendation:
  `inline` / current conversation. It must no longer express Herdr / DSH Web
  / reviewer.
- Herdr and DSH Web become **optional explicit executor adapters** — only
  relevant when the user explicitly picks them — **but the providers that are
  supported today must remain operational**. An explicit "use Herdr w6:p2"
  or "use DSH Web" must still actually execute; a schema that can store
  `provider=herdr` without any working adapter is a regression. The minimum
  set is: `current` (built-in), `herdr` (keep the necessary
  available/dispatch/collect capability), `dsh-web` (keep the browser
  endpoint adapter). Do NOT speculatively add providers with no real
  capability (no `codex-thread` in this task).
- Review granularity comes from the user's authorized units / `implement.md`,
  never from the executor provider. Whatever the executor (current Codex,
  Herdr, DSH Web, native sub-agent), an authorized Slice 0–3 runs the same
  Slice review loop.
- No session-start auto-discovery of Herdr/DSH Web; discovery must not change
  the executor or block the current conversation. Discovery happens only when
  the user explicitly requests that provider.
- Workflow-state breadcrumbs keep only task-lifecycle states
  (`no_task`, `planning`, `in_progress`, `completed`). Provider-specific
  variants (`planning-inline/-auto/-herdr/-dsh-web`,
  `in_progress-inline/-auto/-herdr/-dsh-web`) are removed from core
  semantics; if kept temporarily for hook compatibility they must all resolve
  to the same generic body with **identical permission semantics**.

## 10. Migration Requirements

Old state: `.trellis/.runtime/routing/<context>.json`. New implementation
needs one small, explicit, one-way, tested migration:

- **Preserve** only proven explicit user choices: a legacy target migrates
  automatically only when its recorded provenance is `selected_by="user"`.
  - Legacy reviewer with `selected_by="user"` and a concrete reference → new
    conversation automation context.
  - Legacy executor with `selected_by="user"` and a still-supported provider
    → `executor_override`.
- **Ambiguous provenance is never silently bound**: any legacy reviewer or
  executor whose provenance is `migration`, `policy`, `auto`, or unknown —
  including reviewers produced by the legacy v1→v2 in-memory migration —
  must NOT become the new persisted reviewer/override. It is either shown to
  the user as a candidate requiring one explicit confirmation, or dropped to
  `null` (`reviewer=null` ⇒ the normal ask-once flow; `executor_override=
  null` ⇒ default current). Silent binding of an unconfirmed reviewer is
  forbidden because the reviewer is exactly the role the user must confirm
  once.
- **Discard**: surface, CLI/Desktop detection results, host_routes results,
  auto-selected providers, retired MCP DSH, implicit Herdr selection,
  provider-specific workflow modes.
- **Rollback safety**: the legacy file MUST be left in place byte-for-byte.
  The new system reads it once, records a migration marker plus a source
  fingerprint (e.g. content hash) in the new automation context, and never
  depends on the legacy file again. Renaming or deleting the legacy file is
  forbidden: an old checkout must still find its routing state intact after a
  rollback. Cleanup of legacy files is a separate future task.

## 11. Acceptance Criteria

1. **Default executor**: fresh conversation, no state → effective executor =
   current conversation; reviewer = missing; default reviewer provider =
   chatgpt. Planning/read-only work is not blocked by a missing reviewer;
   starting review-required implementation asks once.
2. **Executor precedence**: no override → current; explicit Herdr → Herdr;
   explicit DSH Web → DSH Web; explicit current → clears override;
   unavailable explicit target → ask once, no fallback. No "CLI implies
   Herdr", no "Desktop implies DSH Web".
3. **Reviewer**: explicit ChatGPT conversation persists; explicit other
   reviewer persists; unavailable reviewer stops the run; reviewer is never
   silently replaced.
4. **Authorization**: "Slice 0–3" snapshots exactly 0/1/2/3; PASS advances
   0→1→2→3 and never enters 4; final PASS → `authorized_scope_complete`, no
   archive.
5. **Remediation**: scoped FAIL → automatic fix; the initial discovering
   FAIL leaves `failed_remediation_rounds = 0`; each completed remediation +
   re-review still failing the same root cause increments it; rounds 1–4
   continue, round 5 → BLOCKED + user; new finding → independent counter;
   closed finding stops counting; ID renaming cannot bypass the counter.
6. **Activation order**: no automation run exists before `task.py start`;
   reviewer resolution + transport verification happen before `task.py
   start`; `status=planning` can never enter implementation via a run;
   automation never writes task status.
7. **Reviewer transport**: the default ChatGPT reviewer is reachable through
   a platform-native conversation send/read/wait capability; repo code never
   calls unofficial ChatGPT APIs or browser-automates ChatGPT; unusable
   transport before start → ask user; transport failure never degrades into
   silent self-review. The §5.6 feasibility probe runs and passes (recorded
   as research evidence) BEFORE any Slice mutates the existing routing
   system; on probe failure the task stops pre-mutation and waits for the
   user's transport decision.
8. **Scope/major issues**: out-of-scope reviewer demand or any §7 condition →
   immediate STOP + user question.
9. **Pending is not PASS**: idle/pending/silence/partial responses never
   count as PASS; only explicit `FINAL: PASS`.
10. **Bootstrap**: new task's first request is self-contained (§5.4 list);
   same-task remediation requests are compact; same reviewer + new task →
   fresh full bootstrap.
11. **Migration**: legacy surface/host-route/auto executor state has no effect
   on the new executor; legacy reviewer/executor migrates only with
   `selected_by="user"` provenance (ambiguous provenance → confirm-or-null,
   never silent); retired MCP DSH is not preserved; the legacy file is left
   in place and the migration marker + fingerprint live in the new context.
12. **Negative tests** prove the old behavior is gone: CLI marker does NOT
    imply Herdr; Desktop marker does NOT imply DSH Web; absence of Herdr does
    NOT block the current executor; absence of DSH Web does NOT block the
    current executor; routing defaults cannot override an explicit user
    target; provider type cannot alter Slice review granularity; final Slice
    PASS does NOT archive.
13. **Anti-complexity invariant**: the new implementation keeps only three
    concepts — conversation automation context (executor override +
    reviewer), task authorization (task + authorized units), review loop
    state (current unit, PASS/FAIL, finding remediation counters). No
    surface-specific policy, host routes, provider-specific workflow-state,
    provider-specific review semantics, auto provider election, or
    candidate-election-as-permission.
14. **Scope guard**: full Trellis test suite passes; no MosDNS runtime code,
    Cargo manifests, WebUI, DNS behavior, Phase 5A benchmark semantics, or
    deployment files are modified.

## 12. Suggested Code Shape (guidance, not a hard contract)

- New: `.trellis/scripts/automation.py` +
  `.trellis/scripts/common/automation.py` replacing the core logic of
  `.trellis/scripts/codex_routing.py` + `.trellis/scripts/common/codex_routing.py`.
- `codex_routing.py` may temporarily remain as a thin deprecated shim
  forwarding to `automation.py` (no state machine of its own); a later
  cleanup removes the shim.

## 13. Files Expected in Scope (to be confirmed by code search in design)

`.trellis/config.yaml`, `.trellis/workflow.md`,
`.trellis/scripts/codex_routing.py`, `.trellis/scripts/common/codex_routing.py`,
`.trellis/scripts/common/config.py`, `.trellis/scripts/common/workflow_phase.py`,
`.trellis/scripts/common/git_context.py`, `.trellis/scripts/common/task_store.py`,
`.codex/hooks/inject-workflow-state.py`, `.codex/hooks/session-start.py`,
`.codex/hooks/inject-subagent-context.py`,
`.trellis/spec/backend/quality-guidelines.md`,
`.trellis/tests/test_codex_routing.py`, `.trellis/tests/test_codex_hook.py`,
plus any new `automation` modules/tests.

## 14. Out of Scope for the Final PASS

Final authorized Slice PASS does not authorize: archive/finish, a new task,
production wiring, deployment, or anything beyond the original authorized
Slice range.
