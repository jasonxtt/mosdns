# Implementation plan — C2C exact compare and reviewer-only protocol

Status: completed; archival with the parent was authorized on 2026-09-28.
The original implementation plan is preserved below. The final branch is
reviewed and validated locally but remains unpublished to the upstream
repository because push permission was denied.

## Slice 0 — external baseline and binding contract

- Create/verify the dedicated external worktree at the audited base commit.
- Write red tests for separate reviewer binding storage and
  `c2c reviewer get/set/clear --json` validation.
- Write red tests proving ordinary project planning `session.url` is not
  returned as the default reviewer binding.
- Implement the smallest binding record/CLI surface with project/chat/
  connector identity only and fail-closed mismatch validation.
- Run external focused tests, typecheck, and build.
- Commit/push this exact slice, record `parent_sha` and `head_sha`, then send
  it to the explicit bootstrap reviewer conversation. On scoped FAIL, make a new
  remediation commit and re-review the exact new range.

## Slice 1 — exact committed `git_compare`

- Add red unit tests for exact two-commit output, dirty worktree isolation,
  full-SHA/revision validation, unsafe path rejection, sensitive renames,
  deterministic pagination, and output caps.
- Add red MCP integration tests for registration, schema, `git.read` scope,
  invalid revision/path errors, and unchanged `git_diff` behavior.
- Implement shared inventory/output helpers and the read-only `git_compare`
  handler.
- Run the focused tests, then the full external test/typecheck/build gates.
- Commit/push, freeze exact parent/head SHAs, send one atomic bootstrap review,
  and remediate only within this Slice if it returns scoped FAIL.

## Slice 2 — reviewer-only protocol and skill guidance

- Add red/contract checks for the reviewer-only mode text and stable finding
  output requirements.
- Update `docs/protocol.md` and `skill/SKILL.md` while leaving normal
  INIT/PLAN/EXECUTING/EXECUTED behavior intact.
- Run all external gates.
- Commit/push before review; submit the exact commit pair to the same
  bootstrap reviewer and follow the bounded remediation rule.

## Slice 3 — external child handoff

- Record the final reviewed external commit, MCP schema, reviewer-binding
  command/JSON shape, and any compatibility notes in the parent research
  artifact.
- Do not enable or claim the local default until the parent integration child
  consumes this exact contract.

## Closeout — 2026-09-28

- Final reviewed branch: `codex/trellis-reviewer-compare` at
  `f870ce7899eb87f01619e2c5cbf941db297241fc`, based on
  `9663b88753e35c76796c5bce000293e0bd22cd9e`.
- Recorded validation at the final commit: 194 tests, typecheck, build, and
  `git diff --check` passed.
- The branch is present in the local external checkout. The configured
  upstream currently exposes only `main` at the audited base; a push dry-run
  returned HTTP 403. No PR, merge, or upstream push is claimed.
- The owner authorized archival after reporting several days of normal use
  without known issues. This owner acceptance does not rewrite the scoped
  review history or claim the parent reviewer returned a verdict.
