# Add exact Git compare and reviewer-only protocol to C2C

Status: completed. This child is part of parent task
`09-24-trellis-c2c-web-reviewer`; archival was authorized with the parent on
2026-09-28. The final reviewed external branch remains local because upstream
push permission was denied; see the parent closeout record.

## Goal

Keep the reviewed workspace/code/data plane in
`XiaoDuoYa/codex-with-chatgpt` read-only while allowing the tool to write only
its own reviewer-binding metadata. It must inspect an exact Trellis
`base_sha..head_sha` range and understand a Trellis-owned reviewer-only
request without taking over the project lifecycle.

## Requirements

- Add a read-only MCP `git_compare` tool for two full commit SHAs, optional
  repository-relative path, and bounded byte pagination.
- Reuse the existing sensitive-path filtering, path containment, rename
  handling, batching, output caps, and line-safe pagination from `git_diff`.
- Reject malformed/non-commit revisions, option injection, traversal,
  absolute paths, and unsafe/sensitive rename endpoints with stable failures.
- Keep `git_diff` modes and existing MCP behavior unchanged.
- Add a dedicated reviewer binding separate from the ordinary per-Codex
  planning session. Provide `c2c reviewer get/set/clear --json` (or the
  equivalent documented CLI surface) storing only project/chat/connector
  identity, so a new Codex planning conversation cannot silently become the
  default reviewer.
- Add a `REVIEW_ONLY` protocol mode/instruction. Trellis remains the
  controller; C2C `PLAN`, `DONE`, iteration limits, and normal execution
  checkpoints cannot change Trellis state.
- Require reviewer output to contain one explicit final line. `FINAL: PASS`
  may contain no findings. `FINAL: FAIL` must include stable `P0/P1/P2/P3-n`
  finding IDs, a root-cause description, and an explicit `open` or `closed`
  status for each finding; the same substantive finding keeps the same ID on
  re-review.
- Add deterministic unit/integration tests and run test, typecheck, and build.

## Acceptance criteria

- [x] `git_compare` distinguishes committed-range output from working-tree,
      index, and `HEAD` diff modes.
- [x] Revision/path/security and pagination tests pass, including sensitive
      renames and unrelated dirty worktree changes.
- [x] Reviewer binding survives a new Codex conversation and cannot be
      confused with `session.url`; mismatched project or connector fails
      closed.
- [x] Reviewer-only documentation gives the exact SHA/path evidence source,
      prohibited actions, PASS/FAIL format, and stable finding contract.
- [x] Existing normal C2C coding protocol and all existing tests remain green.

## Dependencies and non-goals

The parent task must use an explicitly selected reviewer conversation as the
bootstrap reviewer for this child; the current run uses Codex reviewer task
`selected Codex bootstrap reviewer (002reviewer)`. The new C2C reviewer cannot review
its own transport before it exists. The local Trellis child may use fake C2C
responses until this child produces its committed schema and CLI contract.

Do not add Trellis Slice/remediation/task-lifecycle logic, browser automation,
private ChatGPT API calls, credentials, or MosDNS-specific behavior here.
