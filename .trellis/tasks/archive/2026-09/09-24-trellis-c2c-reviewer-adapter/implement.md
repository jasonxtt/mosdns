# Implementation plan — local Trellis C2C reviewer adapter

Status: completed; archival with the parent was authorized on 2026-09-28 after
the owner reported several days of normal use without known issues. The
original implementation plan is preserved below.

## Slice 0 — dedicated binding and reviewer precedence

- Add red tests for valid/invalid dedicated binding payloads, planning-session
  separation, explicit current-turn/persisted precedence, default resolution
  only at the review gate, target persistence, and activation identity drift.
- Implement `automation_c2c_web.py` source/normalization helpers and the
  minimal authorization integration.
- Preserve explicit `set-reviewer` and all existing provider behavior.
- Run focused local tests and the full Trellis test suite.
- Commit/push before review, freeze exact parent/head SHAs, submit to the
  explicit bootstrap reviewer conversation, and apply only scoped FAIL remediation as
  new commits.

## Slice 1 — reviewer-only request and bounded transport

- Add red tests for C2C wrapper composition, body-free bounded messages,
  stable target identity, exactly-once send, pending/timeout polling, exact
  retry behavior, and re-review gating.
- Add red tests for explicit PASS and stable FAIL finding IDs/statuses while
  preserving the existing ledger and five-round limit.
- Implement the injected C2C transport wrapper and parser/request changes.
- Run focused tests, the full local suite, and `git diff --check`.
- Commit/push, record exact parent/head, send one atomic bootstrap review,
  and remediate only within this Slice on scoped FAIL.

## Slice 2 — workflow/spec integration and child handoff

- Update `.trellis/workflow.md` and
  `.trellis/spec/backend/quality-guidelines.md` without overwriting unrelated
  dirty changes.
- Add regression tests for docs-facing/default behavior where practical.
- Run local suite, task validation, and scoped diff audit.
- Commit/push before review, use the exact SHA pair, and finish only after the
  bootstrap reviewer returns explicit PASS.
- Record the external child commit/schema dependency in the parent research
  artifact; leave host-level C2C acceptance to the parent.

## Closeout — 2026-09-28

- The local adapter and follow-up fixes are on branch `rust` through
  `3a2d43028d47bd33c7b126060da7ffaa07710861`; they are present in
  `origin/rust`.
- The parent source audit records local Trellis validation and the exact
  implementation range. No MosDNS runtime/product files were changed.
- The project owner authorized archival after reporting several days of
  normal use without known issues. The historical parent reviewer silence is
  recorded in the parent source audit and is not restated as a reviewer PASS.
