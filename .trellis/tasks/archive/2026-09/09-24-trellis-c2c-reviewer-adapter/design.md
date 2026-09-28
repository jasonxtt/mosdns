# Design — local Trellis C2C reviewer adapter

## Binding source and precedence

Add a narrow `C2CReviewerBindingSource` boundary in
`.trellis/scripts/common/automation_c2c_web.py`. Its production host wiring
reads `c2c reviewer get -w <workspace> --json`; tests inject a payload or a
fake source. It must not call `c2c session --json` for default selection.

The normalized target is:

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

No access token, cookie, message, or diff is persisted. The source rejects an
incomplete or mismatched binding. Current-turn and persisted explicit targets
are checked first; only an empty reviewer context invokes this source. On
success, the target is saved and copied into the existing authorization
snapshot/evidence envelope. The current activation equality check remains the
last identity gate.

## Transport and request wrapper

Keep `ReviewerTransport` provider-neutral. Add a C2C-specific wrapper that:

- validates the `c2c-web` target through a supplied host-native verifier;
- converts the existing structured `build_review_request` result into one
  bounded `[C2C] MODE: REVIEW_ONLY` message;
- sends exactly once, waits through a bounded `kind=chatgpt` read/poll
  operation, and reads the same target identity; and
- returns pending/timeout/transport errors to the existing fail-closed path.

The message names the exact Trellis task/unit, full base/head SHAs, changed
paths, validation, acceptance, forbidden scope, and `git_compare` evidence
source. It never includes a diff, log, or file body. The wrapper does not
interpret C2C `PLAN`, `DONE`, or iteration counters.

Re-review messages are permitted only after an explicit result and use the
existing previous-head-as-new-base rule. A confirmed dead transport may retry
the exact previous message unchanged; it may not append a supplement.

## Finding contract

Extend the C2C request instructions and, if needed, the local parser guard so
that a reviewer response has one final line:

```text
P1-1: <stable root cause> [open]
FINAL: FAIL
```

`FINAL: PASS` may contain a summary and no open finding. `FINAL: FAIL`
requires each finding to have a stable `P0/P1/P2/P3-n` ID, root cause, and
explicit `[open]` or `[closed]` status. Existing ledger/root-cause matching
remains authoritative; the adapter must not create a second remediation
counter.

## Documentation and rollout

Update `.trellis/workflow.md` and the backend quality spec in place, preserving
the user's unrelated dirty edits. State clearly:

- planning/execution stay in the current Codex conversation;
- a dedicated C2C reviewer binding is the default only when no explicit target
  exists and only after it is verified;
- this integration task uses the explicitly selected bootstrap reviewer
  conversation (the current run uses Codex reviewer task
  `selected Codex bootstrap reviewer (002reviewer)`);
- exact commit → review ordering is mandatory; and
- C2C protocol state never overrides Trellis.

The code may be present after this child is reviewed, but the parent performs
the host-level C2C acceptance and only then treats the provider as the default
for subsequent tasks.
