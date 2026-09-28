# Research — C2C web reviewer integration source audit

Date: 2026-09-24 (Asia/Shanghai)

Follow-up review input came from the user-selected ChatGPT conversation on
2026-09-25 (Asia/Shanghai). A second follow-up review was read the same day.

## Local Trellis evidence

- `common.automation` already persists generic target identity and leaves the
  reviewer unset by default. `validate_target` accepts opaque provider and
  reference values; no provider-specific C2C validation exists.
- `common.automation_review` already defines the injected transport boundary,
  verifies a target before authorization, sends one request, waits, reads, and
  parses explicit final verdicts. It deliberately contains no HTTP/browser
  implementation.
- `common.automation_run.authorize` requires a concrete reviewer and verified
  transport evidence before activation, and `submit_review` pins exact
  parent/head SHA plus the reviewer target. This is the correct controller
  boundary to preserve.
- `workflow.md` requires reviewer resolution and transport verification before
  `task.py start`, and the quality spec requires atomic review messages,
  bounded wait/read, explicit PASS, and fail-closed transport behavior.
- The current local test suite is `PYTHONPATH=.trellis/scripts python3 -m unittest`
  under `.trellis/tests`;
  existing reviewer tests use an injected fake transport and do not require a
  live ChatGPT session.

## Prior host transport evidence

The archived `09-21-trellis-automation-simplification` task recorded a real
platform-native send/read round trip to the user-selected plain ChatGPT
conversation. The stable conversation id was preserved by target resolution,
send, and read, with `kind=chatgpt`. `wait_threads` was Codex-only, so the
available wait strategy was bounded polling with `read_thread`. No browser
automation, private API, cookie, token, or manual relay was used.

## External C2C evidence

The audited external checkout was a temporary C2C checkout at commit `9663b88753e35c76796c5bce000293e0bd22cd9e`.

- `skill/SKILL.md` defines the C2C control plane as short `[C2C]` messages and
  requires Codex to retain execution ownership. It explicitly separates
  reviewer inspection from execution and says the ChatGPT side reads code and
  diffs through read-only MCP.
- `docs/protocol.md` defines `INIT → PLAN → EXECUTING → EXECUTED → REVIEW →
  PLAN | DONE | BLOCKED | ERROR`, with `DONE` and `PLAN` as ChatGPT protocol
  states. Those states must be treated as transport content in a Trellis
  reviewer-only integration, not as Trellis lifecycle transitions.
- `src/workspace/git.ts` currently exposes only `DiffMode = unstaged | staged |
  head` and computes diffs against the working tree/index/HEAD. It has bounded
  pagination and sensitive-path filtering that should be reused by a compare
  implementation.
- `src/mcp/server.ts` registers `git_diff`, `test_status`,
  `execution_summary`, and `execution_output`; it has no `git_compare` tool.
- `src/execution/records.ts` persists task/iteration/changed-files/test
  metadata for ChatGPT inspection. It is useful evidence but is not a
  substitute for exact commit-range comparison.

## Required planning revisions from the follow-up review

- `automation_run.py::parse_implementation_units()` recognizes numeric
  headings such as `## Slice 0`; parent and child `implement.md` files must
  use that exact form and their Slice meanings must agree.
- Project-mode `session.url` is thread-local planning state because C2C may
  create a new chat for a new Codex conversation. The default reviewer needs
  a separate persistent binding and explicit `c2c reviewer get/set/clear`
  semantics rather than reusing the normal session pointer.
- This integration cannot use the not-yet-built C2C reviewer as its own
  bootstrap reviewer. An explicitly selected reviewer conversation must be
  verified before `task.py start`; this run uses the user-selected ChatGPT
  user-selected ChatGPT conversation. The C2C reviewer is a later host-level
  acceptance and future-task default.
- Exact review order is implementation → validation → commit/push → freeze
  parent/head → review; a FAIL produces a new remediation commit and exact
  re-review range.
- A reviewer FAIL must preserve stable `P0/P1/P2/P3-n` finding IDs, root
  causes, and explicit open/closed status, with one final verdict line, so
  Trellis's existing five-round ledger remains meaningful.

## Latest review revision

The latest review found one structural issue and one wording issue, both now
resolved:

- Parent and child Slice ownership was duplicated. The parent
  `implement.md` now contains only `Slice 0 — cross-repository integration
  acceptance`; external and local implementation Slices exist only in their
  child `implement.md` files. Execution is external child → local child →
  parent acceptance.
- The C2C security boundary now says the reviewed workspace/code/data plane is
  read-only while C2C may write only its own reviewer-binding metadata. This
  matches the new `c2c reviewer set/clear` capability.

## Consequence and approved scope

The local repository owns reviewer target/default resolution and the Trellis
controller contract. A complete default path additionally needs the external
C2C MCP compare capability and a dedicated reviewer binding, so the user
approved a coordinated two-repository implementation on 2026-09-24. The
external change is kept in its own `codex/trellis-reviewer-compare`
branch/worktree and is verified before the local default is enabled. The
parent now has two independently verifiable children: external C2C capability
and local Trellis adapter. The parent has one integration Slice and does not
duplicate child authorization. No implementation Slice is authorized until
the revised planning summary is approved and a bootstrap reviewer is selected.

## External child handoff — reviewed 2026-09-25

The external child was implemented in the local `codex-with-chatgpt` checkout,
branch `codex/trellis-reviewer-compare`. Its final reviewed commit is
`f870ce7899eb87f01619e2c5cbf941db297241fc`, based on the audited
`9663b88753e35c76796c5bce000293e0bd22cd9e`. The selected reviewer for this
run is the local Codex task `002reviewer`, not the earlier ChatGPT conversation.

The exact review history was:

- `9663b887... → a47b015f...`: Slice 0 PASS.
- `a47b015f... → 22028c3b...`: Slice 1 initially found `P2-1` and `P3-1`.
- `22028c3b... → 02c213fa...`: `P2-1` and `P3-1` resolved; `P2-2` found.
- `02c213fa... → 0214fb70...`: `P2-2` resolved; PASS.
- `0214fb70... → f870ce78...`: Slice 2 PASS.

The final external contract is:

- `c2c reviewer get --workspace <workspace> --json` returns
  `{"ok":true,"binding":null}` when unset, or a normalized binding with
  only `projectUrl`, `chatUrl`, `connectorName`, optional `title`, and
  `boundAt`. `set` accepts `--project-url`, `--url`, `--connector-name`, and
  optional `--title`; `clear` removes only the dedicated binding and never
  changes `session.url`.
- The read-only MCP tool `git_compare` requires `base_sha`, `head_sha`, and
  accepts optional repository-relative `path`, `offset`, and `max_bytes`
  (`1024..262144`, default `65536`). Its output is
  `{isRepo, baseSha, headSha, totalBytes, offset, returnedBytes, hasMore,
  nextOffset, diff}`. It compares the exact committed `base_sha..head_sha`
  range, filters sensitive paths and both sides of renames, and keeps dirty
  worktree/index/HEAD modes separate.
- Invalid revisions and unsafe paths fail closed with stable compare errors;
  the tool is read-only and requires `git.read`.
- `MODE: REVIEW_ONLY` requires `STATE: REVIEW`, `CONTROLLER: TRELLIS`, exact
  SHA/path evidence, no planning/execution/edit/task creation, and one final
  `FINAL: PASS` or `FINAL: FAIL` line. FAIL findings use stable `P0/P1/P2/P3-n`
  IDs with root cause and `[open|closed]`; the same root cause keeps its ID on
  re-review.

External validation at the final commit: `corepack pnpm test` (194 tests),
`corepack pnpm typecheck`, `corepack pnpm build`, and `git diff --check` all
passed. The branch was kept local because the configured GitHub credentials
could not push to the upstream repository; the local reviewed worktree remains
the handoff source until the parent decides how to publish it.

## Parent acceptance attempt — 2026-09-25

The local and external validation gates passed again: the local Trellis suite
reported 58 tests, task validation passed for the parent and both children,
Python compilation and `git diff --check` passed; the external checkout
reported 194 tests, typecheck, build, and `git diff --check` passed. The local
integration range is `09d6a7b..dbb6a886e07f229d210757d656604e425d321b88` and
contains only Trellis scripts, tests, workflow, and spec files; no MosDNS
runtime/product files are included.

Host-level C2C acceptance is blocked fail-closed. On the target workspace,
`c2c reviewer get --json` returned `{"ok":true,"binding":null}`;
`c2c session --json` reported no Project/chat/connector, and the doctor report
showed the local Bridge/MCP healthy but the ChatGPT connector unset; tunnel
status reported no public tunnel, `loggedIn=false`, and that the user's
connection choice is still required. No reviewer binding was invented or
written, and the planning session was not used as a fallback.

The explicitly selected Codex bootstrap reviewer `002reviewer` was sent one
atomic parent-range review request. The platform completed the turn without
returning any assistant message or formal verdict; one identical retry was
made and was also silent. Because no `FINAL: PASS` or `FINAL: FAIL` was
returned, the parent review is not recorded as passed.

## Owner operational acceptance and archive disposition — 2026-09-28

The preceding parent-acceptance section is a historical snapshot of the
2026-09-25 review attempt. On 2026-09-28, the project owner reported that the
integration had been in normal use for several days without known issues and
explicitly authorized archiving the parent and both children. This owner
acceptance is the basis for the lifecycle closeout; it does not retroactively
create a formal parent-level reviewer `FINAL: PASS`.

The local Trellis integration and follow-up fixes are on `rust` through
`3a2d43028d47bd33c7b126060da7ffaa07710861`. The external child remains at
`f870ce7899eb87f01619e2c5cbf941db297241fc` on the local
`codex/trellis-reviewer-compare` branch. On this date the configured GitHub
upstream still exposed only `main` at
`9663b88753e35c76796c5bce000293e0bd22cd9e`; a push dry-run returned HTTP 403.
No external PR, merge, or upstream publication is claimed. The local
`mosdns-rust` implementation is already present on `origin/rust`.

The owner's operational acceptance and exact archive authorization are also
recorded in `owner-acceptance-2026-09-28.md`.
