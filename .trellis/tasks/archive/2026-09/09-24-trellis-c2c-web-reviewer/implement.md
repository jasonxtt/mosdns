# Implementation plan — parent integration acceptance

Status: completed by project-owner acceptance on 2026-09-28 after the owner
reported several days of normal use with no known issues. This parent owns only
cross-repository integration acceptance. The external and local implementation
Slices belong exclusively to their child tasks. The original plan below is
preserved as history; the closeout section records the actual disposition.

The original plan required both child review gates and explicit parent
host-level acceptance before archival. On 2026-09-28 the project owner
authorized closure based on reported operational use. The historical source
audit's missing formal parent reviewer verdict remains disclosed below.

## Child dependency order

1. Start `09-24-trellis-c2c-web-compare`; complete its Slices 0–3 and obtain
   explicit bootstrap-reviewer PASS on each exact commit range.
2. Start `09-24-trellis-c2c-reviewer-adapter` only after the external child
   has recorded its reviewed binding/`git_compare` contract; complete its
   Slices 0–2 and obtain bootstrap-reviewer PASS.
3. Start this parent for its single integration Slice and verify both child
   task commits before any host-level acceptance.

Child task artifacts own their implementation checklists; this parent does
not repeat them as parent authorization units.

## Slice 0 — cross-repository integration acceptance

- Verify the external child has a clean reviewed commit containing the
  dedicated reviewer binding, exact `git_compare` schema, and
  `REVIEW_ONLY`/stable-finding contract.
- Verify the local child has a clean reviewed commit containing the
  `c2c-web` target resolver, explicit-target precedence, bounded transport,
  exact request construction, and updated Trellis workflow/spec docs.
- Run local task validation, the complete `.trellis/tests` suite, external
  `test`/`typecheck`/`build` from the dedicated C2C worktree, and
  `git diff --check`. Audit the combined diff against the pre-existing dirty
  baseline; no MosDNS runtime/product files may be included.
- Perform a host-level C2C acceptance using the dedicated reviewer binding:
  resolve `c2c reviewer get --json`, verify project/chat/connector identity,
  send one bounded reviewer-only request for the exact parent/head range,
  wait/read through the platform-native ChatGPT adapter, and confirm the web
  reviewer can use `git_compare` and return the stable PASS/FAIL contract.
- If parent-owned integration/documentation changes are needed, implement
  them only inside this Slice, validate them, then commit/push before review;
  freeze the exact `parent_sha`/`head_sha` and send the atomic request to the
  bootstrap reviewer conversation. A scoped FAIL creates a new remediation commit
  and exact re-review; it does not mutate the reviewed child commits.
- After the host-level C2C acceptance and parent review both return explicit
  PASS, record the dedicated C2C reviewer as the default for subsequent
  tasks. Do not archive, deploy, or authorize unrelated work.

## Validation commands

From the `mosdns-rust` repository root:

```bash
PYTHONPATH=.trellis/scripts python3 -m unittest discover -s .trellis/tests -p 'test_*.py'
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-24-trellis-c2c-web-reviewer
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-24-trellis-c2c-web-compare
python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-24-trellis-c2c-reviewer-adapter
git diff --check
```

From the dedicated `codex-with-chatgpt` worktree:

```bash
corepack pnpm test
corepack pnpm typecheck
corepack pnpm build
```

## Failure and rollback gates

- Missing/changed binding, unsupported compare, invalid SHA/path,
  sensitive-path failure, unavailable transport, or ambiguous verdict blocks
  acceptance. There is no fallback to the planning session, another chat,
  working-tree diff, or another reviewer.
- If either child is incomplete or fails review, do not start the parent.
- If host-level C2C acceptance fails, retain the reviewed child commits and
  explicit reviewer path but do not claim the C2C default is enabled.
- Preserve all unrelated dirty changes; do not reset or clean the worktree.

## Closeout — 2026-09-28

- The owner reports that the integration has been in normal use for several
  days without known issues and explicitly authorizes archiving the parent and
  both children.
- The external child reached reviewed commit
  `f870ce7899eb87f01619e2c5cbf941db297241fc`; its recorded final validation
  was 194 tests, typecheck, build, and `git diff --check`. The branch remains
  local because the configured upstream rejected push permission; no PR or
  upstream publication is claimed.
- The local Trellis implementation and follow-up fixes are on `rust` through
  `3a2d43028d47bd33c7b126060da7ffaa07710861`. Their validation history is
  retained in `research/source-audit.md` and the workspace journal.
- The 2026-09-25 source audit records that the bootstrap reviewer did not
  return a formal parent-range verdict. The owner's operational acceptance
  authorizes this archive; it is not represented as a reviewer `FINAL: PASS`.
- The change did not modify MosDNS runtime or product behavior.
