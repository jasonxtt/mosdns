# Integrate the dedicated C2C reviewer into Trellis

Status: completed. This child belongs to parent
`09-24-trellis-c2c-web-reviewer`; it consumed the external C2C
binding/compare contract. Archival was authorized with the parent on
2026-09-28 after the owner reported several days of normal use without known
issues.

## Goal

Add the local Trellis `c2c-web` reviewer provider while preserving the current
generic reviewer contract, explicit Codex overrides, exact-SHA review loop,
finding ledger, and fail-closed behavior.

## Requirements

- Read a dedicated C2C reviewer binding, never the ordinary planning
  `c2c session` pointer, and normalize it to a stable `provider=c2c-web`
  target with non-secret project/chat/connector metadata.
- Resolve that default only at the pre-start review gate and only when no
  current-turn or persisted explicit reviewer exists.
- Verify and snapshot the exact reviewer identity before activation. A changed,
  missing, ambiguous, or unavailable binding blocks without fallback.
- Keep explicit Codex/plain ChatGPT/Herdr/DSH targets authoritative and
  unchanged.
- Wrap existing Trellis review requests in the external C2C `REVIEW_ONLY`
  contract, including exact SHAs/paths, scope, prohibitions, and stable
  finding output instructions, without pasting bodies.
- Use an injected platform-native ChatGPT send/wait/read adapter with bounded
  polling; no browser/private API implementation.
- Preserve Trellis as verdict/controller authority and enforce explicit,
  stable FAIL findings compatible with the existing remediation ledger.
- Update local workflow/spec docs and deterministic tests only; no MosDNS
  runtime/product changes.

## Acceptance criteria

- [x] Dedicated binding resolution and explicit-target precedence are covered
      by tests, including no invocation of the default source for an explicit
      reviewer.
- [x] Authorization stores the resolved target/evidence and activation rejects
      identity drift.
- [x] Requests are atomic, bounded, exact-range, body-free, and produce the
      required stable PASS/FAIL/finding contract.
- [x] ChatGPT polling handles pending/partial/silent/timeout results without
      interpreting them as PASS.
- [x] Existing generic reviewer, migration, hook, and remediation tests stay
      green.
- [x] Local docs state that the new default is for future tasks after parent
      host-level acceptance; this child does not self-bootstrap.

## Dependency and non-goals

The external child must first provide the dedicated reviewer binding JSON/CLI
shape, `git_compare` schema, and reviewer-only guidance. Local tests may use
fakes before that commit is available, but the default cannot be enabled or
claimed complete until the exact external contract is verified.

Do not add browser automation, unofficial APIs, credential storage, normal
C2C planning, a second lifecycle controller, or MosDNS runtime/UI/config
changes.
