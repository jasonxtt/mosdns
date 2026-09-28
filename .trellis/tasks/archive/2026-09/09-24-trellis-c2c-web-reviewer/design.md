# Design — Trellis C2C web reviewer integration

Status: final design reference. Implementation and owner-authorized archival
are recorded in the parent task's closeout artifacts.

## Design decisions

1. `mosdns-rust` remains the sole controller. It owns task planning,
   authorization, exact commit pairs, the finding ledger, remediation limits,
   verdict parsing, and advancement.
2. `codex-with-chatgpt` keeps the audited workspace/code/data plane read-only
   for review. It may write only its own reviewer-binding metadata. Its normal
   INIT/PLAN/EXECUTED/REVIEW/DONE loop remains available for ordinary C2C
   coding, but a Trellis `REVIEW_ONLY` request cannot delegate lifecycle
   authority to it.
3. The default reviewer is a dedicated C2C reviewer binding, not the ordinary
   per-Codex planning session. The binding survives new Codex conversations
   and contains a concrete project/chat/connector identity.
4. An explicitly selected reviewer conversation bootstraps this integration
   task. For the current run it is the user-provided Codex reviewer task
   `selected Codex bootstrap reviewer (002reviewer)`. The new C2C web reviewer is
   host-level acceptance for this task and becomes the default only for
   subsequent tasks after that acceptance passes.
5. The external repository is developed in a separate
   `codex/trellis-reviewer-compare` worktree/branch. The local default is not
   claimed complete until the external contract and local adapter both pass
   focused tests and host verification.
6. No browser automation, DOM access, unofficial API, cookie/token handling,
   or credentials are added. The host supplies the platform-native ChatGPT
   send/read capability through the injected transport boundary.

## Parent/child work map

The parent owns the cross-repository requirement set and final integration
acceptance. Its independent children are:

- `09-24-trellis-c2c-web-compare`: dedicated external reviewer binding,
  exact `git_compare`, and reviewer-only protocol/skill guidance.
- `09-24-trellis-c2c-reviewer-adapter`: local target precedence, transport,
  exact request/finding contract, and Trellis workflow/spec integration.

The local child can develop against fakes, but it cannot enable or claim the
default until the external child has produced a reviewed binding/compare
contract. Dependencies are written in both child artifacts, not inferred from
directory order.

## Bootstrap reviewer and review order

Before any implementation child or parent slice starts, the user selects an
explicit reviewer conversation and the current host verifies it through the
existing provider-neutral transport contract. For this run the selected
conversation is the Codex task
`selected Codex bootstrap reviewer (002reviewer)`; it is frozen in the task's
authorization snapshot. This prevents the integration from needing the new
C2C reviewer to review its own creation.

Every child/parent unit follows the existing exact-SHA order:

```text
red test → green implementation → validation → commit/push
→ freeze parent_sha/head_sha → atomic review → scoped FAIL remediation commit
→ re-review the new exact range
```

The dedicated C2C web reviewer is exercised only after both child tracks have
reviewed commits and the local host adapter is available. That host-level
acceptance is a separate final integration check; it does not mutate the
already frozen bootstrap reviewer identity mid-run. Its PASS authorizes the
feature as the default for later tasks, not archive/deployment or extra work
on this task.

## External C2C reviewer binding

The current C2C Project behavior deliberately maps a new Codex conversation
to a new ChatGPT chat and reports `reuseSavedChat=false`. Therefore the local
adapter must never use `c2c session --json` or `session.url` as a global
reviewer identity.

The external child adds a separate binding record keyed by C2C workspace:

```text
projectUrl, chatUrl, connectorName, title?, boundAt
```

It exposes `c2c reviewer get/set/clear --json`. `set` accepts a user-selected
existing chat URL and its Project/connector identity; it validates the URL
shape and required fields but does not create a chat or match by display name.
`get` returns the normalized binding. `clear` is explicit. A missing,
ambiguous, changed, or mismatched project/chat/connector blocks resolution;
there is no random-chat or ordinary-session fallback.

The record stores no access token, cookie, MCP output, message body, diff, or
log. If the bound chat becomes unavailable, the user must explicitly rebind
it and re-authorize the affected task.

## Local target resolution and identity

Add a narrow `C2CReviewerBindingSource` boundary in
`.trellis/scripts/common/automation_c2c_web.py`. Production host wiring reads
the external `c2c reviewer get -w <workspace> --json` result; tests inject a
payload/fake source. The normalized target is:

```json
{
  "provider": "c2c-web",
  "reference": "<canonical reviewer chat URL/id>",
  "label": "C2C web reviewer",
  "metadata": {
    "project_url": "<canonical project URL>",
    "chat_url": "<same canonical chat identity>",
    "connector_name": "<connector name>",
    "binding_source": "c2c-reviewer"
  }
}
```

Resolution order is:

1. current-turn explicit target;
2. persisted explicit target;
3. dedicated C2C reviewer binding, invoked only when a review is required;
4. fail-closed activation error.

When step 3 succeeds, the target is persisted and copied into the immutable
authorization snapshot/evidence envelope. The existing activation equality
check remains the final identity gate. An explicit target never invokes C2C
resolution and remains authoritative even when C2C is unavailable.

## Reviewer-only message and transport flow

The local request builder keeps its structured evidence fields and wraps them
in one bounded message similar to:

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: <task>
UNIT: <authorized unit>
BASE_SHA: <full parent SHA>
HEAD_SHA: <full head SHA>
PATHS: <exact changed paths>
INSTRUCTION: Inspect only the committed range with git_compare(base_sha, head_sha).
             Review the listed unit and return one explicit final verdict.
             Do not plan, execute, edit, create a task, change Trellis state,
             or treat C2C DONE/PLAN/iteration state as controller authority.
```

It contains task/unit, exact SHA/path/evidence/scope/prohibitions, but no
pasted diff, log, or file body. The web reviewer reads code through the
read-only MCP data plane.

The injected transport sequence is:

```text
verify target → send exactly once → bounded ChatGPT wait/read
→ parse Trellis verdict and finding ledger
```

For `kind=chatgpt`, waiting uses bounded ChatGPT-specific read/poll behavior;
the generic Codex-only `wait_threads` helper is not assumed. Pending, idle,
silent, partial, timeout, or malformed output is not PASS. A confirmed dead
transport may retry the exact previous complete message unchanged; it may not
send a supplement.

## Stable finding contract

The reviewer response has exactly one final verdict line. PASS may contain a
concise summary and no open findings. FAIL must carry stable IDs, root cause,
and explicit status:

```text
P1-1: <stable root cause> [open]
P1-2: <resolved root cause> [closed]
FINAL: FAIL
```

IDs use `P0/P1/P2/P3-n`. The same substantive finding keeps the same ID on
re-review; a new ID is allowed only for a genuinely new root cause. Trellis's
existing parser/ledger remains the authority for status, scope, and the
five-round same-root-cause limit. C2C does not add another remediation
counter or lifecycle state.

## Exact committed comparison in C2C

The external `git_compare` tool accepts full `base_sha` and `head_sha`, an
optional repository-relative path, and byte pagination (`offset`, `max_bytes`).
It compares the two committed trees directly (`git diff base_sha head_sha`),
not working tree/index/HEAD or a merge-base approximation. Before returning
content, it:

- resolves both revisions as commits and requires canonical full-SHA identity;
- rejects malformed revisions, option injection, path escapes, and unsafe
  repository scope;
- reuses the existing NUL-delimited path inventory and sensitive-file rules,
  filtering both sides of renames;
- applies the existing output cap and line-safe pagination; and
- returns explicit base/head and continuation metadata so it cannot be
  confused with `git_diff` modes.

The MCP schema uses snake_case inputs and existing camelCase outputs, is
read-only, and requires `git.read`. Invalid revisions/paths are errors, not
successful empty results.

## Compatibility, rollout, and rollback

Existing explicit Codex, Herdr, DSH Web, and plain ChatGPT targets continue
through their current injected transports. The default change is limited to
the no-explicit-reviewer path after pre-start binding verification.

Rollout:

1. Bootstrap the parent/children with the explicitly selected reviewer
   conversation; the current run uses Codex reviewer task
   `selected Codex bootstrap reviewer (002reviewer)`.
2. Implement and review the external child in its own worktree/branch.
3. Implement and review the local adapter against the external exact
   contract.
4. Run host-level C2C acceptance using the dedicated binding, then document
   it as the default for subsequent tasks.

If any gate fails, retain explicit reviewer selection and stop. A rollback
removes only the C2C default registration/docs; it does not reset unrelated
dirty changes or silently switch an active task's reviewer.

## Security and failure boundaries

- Never persist credentials, message bodies, MCP output, cookies, or tokens.
- Never use a working-tree diff to satisfy an exact committed submission.
- Never treat C2C `DONE`, `PLAN`, iteration limits, or green local tests as
  Trellis PASS.
- Treat missing binding, changed identity, unsupported compare, invalid SHAs,
  unsafe/sensitive paths, transport failure, and ambiguous verdict as blocked
  or fail-closed conditions.
