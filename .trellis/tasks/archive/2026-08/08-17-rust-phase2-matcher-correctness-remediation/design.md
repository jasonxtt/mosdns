# Design: Phase 2 matcher correctness remediation

## Boundary and invariants

This task keeps the experimental matcher architecture intact:

```text
Go control plane builds accepted rules
        |
        +--> immutable Go candidate (always authoritative)
        |
        +--> optional Rust candidate (only when the complete batch is proven safe)
```

The default runtime remains Go-only. No new selector, cgo symbol, ABI version,
or production query-path integration is introduced. A Rust build failure can
remove only the Rust optimization for the new generation; it cannot reject a
valid Go update or leave an old Rust snapshot paired with new Go state.

## Regexp compatibility policy

The policy is centralized in `rust/matcher-core/src/regex.rs` and is called by
every Rust construction path that can compile a regexp:

- `RegexMatcher::add`;
- `MixMatcher::add`;
- `ValuedDomainMatcher::build`.

The first safe set is deliberately conservative. It admits the following
explicit grammar:

- ASCII literals;
- `^` and `$` anchors;
- `.`;
- plain capturing groups `(...)`, alternation `|`, and the quantifiers `*`,
  `+`, `?`, `{n}`, `{n,}`, and `{n,m}`;
- explicit ASCII character classes and ranges such as `[.]`, `[a-z]`,
  `[a-zA-Z0-9]`, and their negated forms `[^a-z]`;
- only escapes for regex metacharacters (for example `\\.`, `\\\\`, `\\[`, and
  `\\]`).

It rejects every `(?...` extension (including `(?:...)` and `(?i:...)`), lazy
or otherwise extended quantifiers, Perl shorthand classes (`\\w`, `\\W`,
`\\d`, `\\D`, `\\s`, `\\S`), Unicode classes or escapes (`\\p`, `\\P`, and
equivalent Unicode-dependent forms), non-ASCII pattern bytes, and any
backslash escape or class extension not explicitly listed above. The parser
must track escape state and character-class state; a collection of
`strings.Contains` checks is not an acceptable validator.

The validator returns a typed compatibility error distinct from a Rust compile
error. Both errors make the complete Rust candidate ineligible; they do not
make the Go candidate invalid. This avoids trying to merge a partially Rust
compiled rule batch with a complete Go batch.

The valued matcher must call the shared validator before constructing its
`regex::Regex`; it must not retain a private direct compile path. The existing
C ABI maps an ineligible build to its existing invalid-argument status, so no
ABI/header change is needed. Go builders already receive an error and can
publish the Go candidate.

Tests use `^\\w+$` with a non-ASCII input as the required compileable-but-
divergent fixture: Go's regexp accepts the pattern but treats the shorthand as
ASCII, while Rust's default Unicode interpretation differs. The test proves
that this rule is rejected from the Rust candidate and that the Go oracle still
produces the authoritative result. Additional fixtures cover `\\d` and `\\s`
and a representative compatible ASCII pattern.

## Normalization and query fallback

Exact Unicode parity is not assumed in this remediation. The Rust candidate is
restricted to ASCII rule text and ASCII query input:

- every Rust matcher construction path rejects non-ASCII `full`, `domain`, and
  `keyword` rule text, as well as non-ASCII regexp patterns, before a snapshot
  can be published;
- the Linux matcher adapter repeats that rule check before entering cgo as a
  fast defense-in-depth preflight;
- a shared Go helper reports whether a query string is safe for Rust matching;
- `domain_set`, `sd_set`, `base_domain`, and `domain_mapper` check that helper
  before calling Rust; unsupported input goes directly to the paired Go
  matcher;
- `si_set` has no domain normalization and is covered only by the build-failure
  generation contract.

The Rust runtime matcher entrypoints also reject a non-ASCII domain query with
the existing `InvalidArgument` status. The Go consumers preflight first, so
normal provider operation does not trip a Rust generation; a future direct
Rust caller cannot receive a silently incorrect positive match.

For `base_domain` anonymous rules, the Rust wrapper remains at the same matcher
position as the anonymous Go matcher. Its query guard returns "not handled"
for non-ASCII input without closing the handle, allowing the existing matcher
group to continue to the anonymous Go matcher. The next ASCII query may still
use the same Rust handle.

The matcher-core boundary is authoritative: `MixMatcher::add` and
`ValuedDomainMatcher::build` reject non-ASCII domain/keyword/regexp rule text,
and direct Rust matching cannot turn a non-ASCII query into a positive result.
The runtime C entrypoints return the existing `InvalidArgument` status for such
queries. The Go adapter's preflight is only a fast, defense-in-depth shortcut.

For ASCII input, Rust `to_ascii_lowercase` plus one trailing-dot removal is
byte-for-byte equivalent to Go's frozen operation. For non-ASCII input, Go's
`strings.ToLower(strings.TrimSuffix(s, "."))` remains authoritative. The
query-level branch is not an error from the Rust handle, so it cannot trip a
circuit breaker or close a healthy generation. A future task may expand the
safe set only after a complete Go/Rust Unicode mapping corpus proves parity.

## Generation publication

### `domain_set` POST

1. Parse the request and build the complete Go candidate.
2. Attempt the Rust candidate. An error is logged and converted to `nil` Rust
   for this generation; it is not returned as HTTP 500.
3. Persist the accepted rule file; persistence failure closes the new Rust
   candidate (if any), leaves the old generation active, and returns the
   existing error.
4. Publish the Go candidate and optional Rust handle together under the existing
   update/state locks.
5. Close the retired Rust handle after the replacement is visible.

### `ip_set` POST and `/flush`

Use the same order with the accepted prefix list and the existing file-save
error behavior. A Rust build failure publishes the new Go prefix generation,
returns the existing success response, and retires the old Rust handle after
publication. `si_set` already follows the desired generation behavior; its
existing reload/update tests are reviewed and extended only if they do not
assert the same lifecycle, rather than assuming a `/post` or `/flush` API.

Candidate construction remains outside the match read lock. During a slow
candidate build, readers continue using the old paired generation. No reader
can observe a new Go candidate with an old Rust snapshot.

## Runtime error classification

Build-time unsupported/invalid patterns are generation construction failures
and select Go-only publication. Runtime Rust errors retain the existing
same-generation fallback and circuit-breaker behavior. Query-level non-ASCII
input is handled before the Rust call and is neither a runtime error nor a
generation failure. No new ABI status or Go error class is required.

## Spec correction

The current `.trellis/spec/backend/rust-migration.md` provider matrix still
says “Rust build error -> update error, old Go/Rust generation remains
active.” For this task, the task-specific contract supersedes that stale line:

- valid Go candidate plus persistence success plus Rust-only build failure →
  publish the new Go-only generation and return success;
- Go parse or persistence failure → preserve the old complete generation and
  return the existing error.

The implementation finish gate must run `trellis-update-spec` to replace the
stale provider rule after the behavior is verified. The spec is intentionally
not edited during this planning-only turn.

## Test seams

- Rust unit tests exercise the validator and every construction path without
  cgo.
- Linux Go adapter tests exercise rule/query ASCII preflight and the existing
  injected builder seams.
- Provider tests inject Rust builders and fake handles to observe HTTP result,
  active Go/Rust pairing, close count, and replacement ordering.
- Linux+cgo integration tests, when available, prove the real Rust build rejects
  unsafe patterns; macOS/non-cgo tests remain stub/default Go-only evidence.

## Non-goals

This design does not address cache expiry, Phase 0 global compatibility or
benchmark closure, matcher ABI naming, sequence execution, upstream transport,
server listeners, WebUI, or default Rust enablement.
