# Rust Phase 2 matcher correctness remediation

## Planning status

This task is planning-only until the compatibility boundary is reviewed. Do
not run `task.py start` or change production code before `design.md` and
`implement.md` are approved.

The requested compatibility decision is confirmed: a batch containing any
regexp outside the proven safe set publishes Go-only for that generation, and
a query-level normalization limitation falls back to the paired Go matcher
without disabling the Rust generation.

The task is intentionally limited to three Phase 2 correctness gaps. Cache
expiry, Phase 0 closure, ABI naming, and migration-status documentation are
separate follow-up work.

## Goal

Make the experimental Rust matcher path safe to use as a generation-level
optimization without changing the accepted Go rule/configuration behavior:

1. Rust regexp matching must run only for patterns proven equivalent to Go
   `regexp` semantics; unsupported or unproven patterns use the same
   generation's Go matcher.
2. Domain normalization must either match Go's
   `strings.ToLower(strings.TrimSuffix(s, "."))` exactly or fall back for the
   individual unsupported query without disabling a healthy generation.
3. A valid Go candidate must still be published when an experimental Rust
   candidate cannot be built; `domain_set` and `ip_set` POST/flush must not
   turn a Rust-only failure into HTTP 500.

Go remains the authoritative behavior oracle and the default runtime remains
Go-only. No Rust matcher is enabled by this task in a default build.

## Requirements

### Regexp compatibility gate

- Define one conservative, testable Rust-regexp compatibility policy shared by
  the plain domain matcher, `sd_set`, and valued `domain_mapper` matcher
  construction.
- A pattern must be admitted to Rust only when the policy proves its matching
  behavior is equivalent to Go. A pattern that Rust can compile but the policy
  cannot prove is not a Rust build success.
- Freeze the first grammar explicitly: ASCII literals, `^`, `$`, `.`, plain
  `(...)`, `|`, `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, explicit ASCII classes
  and ranges (including `[^...]`), and escapes for regex metacharacters only.
  Reject every `(?...` extension, lazy/extended quantifier, Unicode class,
  shorthand class, unlisted escape, and non-ASCII pattern. The validator must
  track escape and character-class state rather than use substring filters.
- An unsupported regexp must not make an otherwise valid Go rule batch
  invalid. The safe default is to disable the Rust candidate for that complete
  generation and retain the complete Go candidate, avoiding mixed-generation
  results.
- Add golden cases where both engines compile but differ, including the
  ASCII-vs-Unicode behavior of Go/Rust shorthand classes such as `\\w`,
  `\\d`, and `\\s` on non-ASCII input. Tests must assert that the Rust path is
  not selected, not merely that one compiler rejects a pattern.
- Cover all affected entrypoints, including valued `domain_mapper`; a policy
  implemented only in the low-level `RegexMatcher` is insufficient if another
  builder constructs `regex::Regex` directly.
- Keep build-time unsupported rules distinct from runtime failures. A query
  that cannot be normalized or evaluated safely must use the same-generation
  Go matcher and must not permanently circuit-break an otherwise healthy Rust
  generation.
- Enforce the ASCII rule boundary inside matcher-core itself for full/domain/
  keyword/regexp rules and reject non-ASCII domain queries at the Rust runtime
  boundary with the existing invalid-argument classification. Go adapter
  preflight is defense-in-depth, not the correctness barrier.

### Normalization parity

- Freeze Go's actual one-trailing-dot removal and Unicode lower-casing behavior
  with Go golden fixtures containing non-ASCII labels, case mappings, empty and
  repeated trailing dots, and inputs that may expand or otherwise differ under
  Rust Unicode lowering.
- Either implement a Rust normalization proven byte-for-byte equivalent for
  the accepted input set, or classify the unproven input as query-level
  unsupported and use the paired Go matcher for that query.
- Apply the same decision to plain domain, suffix/domain, keyword/regexp, and
  valued-domain matching. Unsupported input must not close or replace a healthy
  generation. This query-level fallback explicitly covers `domain_set`,
  `sd_set`, `base_domain` anonymous matchers, and `domain_mapper`.
  `si_set` is an IP-prefix provider and is covered only by the transactional
  build-failure requirement.

### Transactional Go generation fallback

- `domain_set` POST and `ip_set` POST/flush must build the Go candidate and
  attempt Rust construction before persistence, preserving the existing
  build → persist → publish relative order.
- If the Go candidate and file operation are valid but Rust construction fails,
  publish the new Go generation with no Rust handle, log a warning, return the
  existing success response, and retire the previous Rust handle only after
  the replacement is visible.
- If Go parsing or file persistence fails, preserve the previous generation and
  return the existing error; do not partially publish a Rust candidate.
- If persistence fails after a Rust candidate was built, close that new
  candidate and preserve the previous complete generation.
- The published Go and Rust snapshots (when present) must represent the same
  accepted rule/prefix batch. Never combine an old Rust snapshot with a new Go
  candidate.
- Add provider seam/lifecycle tests for domain_set POST and ip_set POST/flush
  build failures, including replacement/close ordering and old-generation
  retention during candidate construction.
- Preserve the already-correct same-generation fallback behavior in `sd_set`,
  `si_set`, and valued `domain_mapper`; add regression coverage where the new
  regexp/normalization policy forces Go-only behavior.

### Scope and safety constraints

- Rust remains experimental and opt-in; default Go-only behavior is unchanged.
- Do not add a new backend selector, cgo/ABI symbol, runtime handle namespace,
  EntryHandler/sequence integration, upstream/network code, or fallback policy
  outside the matcher providers named above.
- Do not modify cache, Phase 0 documentation/benchmarks, WebUI, coremain,
  OpenWrt, local files, or unrelated provider behavior.
- Each behavior slice must follow red test → minimum implementation → focused
  verification, with no staging, commit, archive, or Phase 3B start in this
  task.

### TDD seams to freeze before implementation

- Rust matcher seam: compatibility classification/validation is observable
  before a pattern is admitted to a snapshot.
- Go provider seams: existing Rust builders remain injectable so tests can
  force build success, unsupported classification, and hard failure.
- Observable generation snapshot: Go rules/prefixes, optional Rust handle,
  active/retired status, match result, HTTP status/body, and close count.

## Acceptance Criteria

- [ ] A reviewed `design.md` freezes the conservative regexp policy, exact
      normalization/fallback classification, error classes, and generation
      publication order.
- [ ] A reviewed `implement.md` orders the three behavior slices with red
      tests, focused gates, rollback points, and review stops.
- [ ] Rust regexp golden tests prove that compatible patterns match Go and
      that compileable-but-divergent patterns are rejected from Rust; coverage
      reaches plain domain, `sd_set`, and valued `domain_mapper` paths, with
      parser-state fixtures for escaped and class-local syntax.
- [ ] matcher-core itself rejects non-ASCII domain rules and the Rust runtime
      rejects non-ASCII domain queries; adapter preflight is additional
      defense-in-depth.
- [ ] Unicode normalization fixtures either show exact Go parity or prove
      query-level Go fallback without disabling the generation; coverage
      includes `base_domain` anonymous matchers and no unsupported query changes
      the next query's Rust availability.
- [ ] `domain_set` POST and `ip_set` POST/flush publish a valid new Go
      generation and return success when only Rust construction fails; old
      Rust handles are retired exactly once after replacement publication.
- [ ] Existing `sd_set`, `si_set`, and valued `domain_mapper` generation and
      runtime fallback tests remain green, with new regressions for the
      unsupported regexp/normalization cases.
- [ ] Focused Go tests, race tests, vet, Rust fmt/tests/clippy, and the
      repository's existing default Go-only gates pass.
- [ ] The verified publication rule is synchronized into
      `.trellis/spec/backend/rust-migration.md` via `trellis-update-spec` after
      implementation; the stale rule is not edited during planning.
- [ ] No cache, ABI, server/sequence/upstream, WebUI, coremain, OpenWrt, or
      unrelated local-file changes are introduced; no Phase 3B work starts.

## Notes

- Go compatibility here is a behavior oracle and paired fallback, not a
  permanent hybrid runtime design. The project can remove the oracle only
  after the full Rust host owns the request path and the same contracts are
  independently frozen.
- This is a complex task: `design.md` and `implement.md` are required before
  `task.py start`.
