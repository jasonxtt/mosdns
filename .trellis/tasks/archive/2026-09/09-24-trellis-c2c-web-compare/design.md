# Design — C2C exact compare, reviewer binding, and reviewer-only mode

## External repository boundary

Work in a separate `codex/trellis-reviewer-compare` branch/worktree of
`codex-with-chatgpt`. The normal `session` file remains the planning-session
binding described by the existing C2C Project model. It must not be reused as
the cross-Codex default reviewer identity.

Add a separate reviewer-binding record keyed by the C2C workspace id. The
record contains only:

```text
projectUrl, chatUrl, connectorName, title?, boundAt
```

`c2c reviewer set` validates that the chat and project belong to the
ChatGPT URL shapes already accepted by C2C and that the required identity
fields are present. `get --json` returns a normalized binding; `clear` is
explicit. It never creates a chat, follows a display name, or copies the
ordinary planning session pointer. A project/chat/connector mismatch is an
error rather than a fallback.

The stored binding is intentionally not a credential. It contains no access
token, cookie, MCP output, message body, diff, or log.

## `git_compare` implementation

Refactor the existing `git_diff` path inventory and bounded output logic into
shared helpers where practical. `git_compare` then:

1. accepts full hexadecimal `base_sha` and `head_sha` plus `path`, `offset`,
   and `max_bytes`;
2. resolves both revisions with `git rev-parse --verify --end-of-options
   <sha>^{commit}` and requires canonical full-commit equality;
3. inventories changed paths using a two-endpoint `git diff base_sha
   head_sha`, not `HEAD`, the index, or a merge-base range;
4. filters unsafe/sensitive paths on both sides of a rename and applies the
   existing repository path scope rules;
5. fetches the bounded content diff for safe paths and paginates without
   splitting a line; and
6. returns `{ isRepo, baseSha, headSha, totalBytes, offset, returnedBytes,
   hasMore, nextOffset, diff }`.

The MCP input remains snake_case and the output remains camelCase. The tool
has a read-only annotation and uses the existing `git.read` scope. Invalid
revisions and unsafe paths are explicit errors, not successful empty results.

## Reviewer-only protocol

Add a mode that reuses `STATE: REVIEW` as content but does not alter the
normal C2C state machine:

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: ...
UNIT: ...
BASE_SHA: ...
HEAD_SHA: ...
PATHS: ...

INSTRUCTION:
Use git_compare for the exact committed range. Do not plan, execute, edit,
create tasks, or interpret C2C DONE/PLAN/iteration limits as Trellis state.
Return one explicit FINAL: PASS or FINAL: FAIL.
```

The response contract is:

```text
P1-1: <stable root cause> [open]
P1-2: <stable root cause> [closed]
FINAL: FAIL
```

PASS may contain a concise summary and only the unique `FINAL: PASS` line.
FAIL must include stable IDs and status; a re-review keeps an ID for the same
root cause and uses a new unused ID only for a genuinely new finding. This is
guidance for the reviewer, not a second Trellis parser/controller.

## Bootstrap and review order

This child is reviewed by the explicitly selected bootstrap reviewer
conversation; the current run uses Codex reviewer task
`selected Codex bootstrap reviewer (002reviewer)`. The required order is:

```text
red tests → implementation → validation → commit/push → fixed parent/head
→ reviewer request → scoped FAIL remediation commit/re-review
```

The parent host-level C2C acceptance happens only after this child has a clean
reviewed commit and the local adapter can consume its `reviewer get` and
`git_compare` contracts.
