# Native rule providers, `domain_mapper` and `mark` (M1-a1)

Scope: the read side that lets the native host load the rule providers a production
`config_all` uses and route queries by `domain_mapper` marks. Management (source
add/edit/delete, download, auto-update, `show`/`save`/`post`, hot reload) is M1-a2 and is
not part of this contract. Evidence: `docs/rust/validation-records/10-07-m1-a1-rule-providers/`.

## Providers

All providers load **once at compile/start time** from existing disk state, are immutable
afterwards, and never write: no directory creation, download, manifest rewrite or
rule-count update. Relative paths follow the configuration root (`-d`).

| Type | Arguments | Source of rules |
| --- | --- | --- |
| `domain_set_light` | `exps`, `files`, `sets` (ignored) | `exps`, then each file: SRS or text. A missing file is an empty source. |
| `sd_set_light` | `local_config` (required), `socks5` (ignored) | JSON manifest; enabled sources with a `files` path; each `.srs` file. `enable_regexp` gates regexp items. |
| `si_set` | `local_config` (required), `socks5` (ignored) | same manifest shape; CIDRs of enabled `.srs` files; usable in `resp_ip $tag`. |
| `adguard_rule` | `dir` (required), `socks5` (ignored) | `<dir>/config.json` + `<dir>/<id>.rules` of enabled lists. Only block rules are exported; `@@` lines never reach the mapper. Lists that are not on disk are skipped, **not downloaded**. |

A missing, empty or unparsable manifest is an empty source list (diagnostic only); a manifest with a string the reference repairs (lone UTF-16 surrogate escape, invalid UTF-8) is refused instead, because Go would keep its sources. Manifests decode like Go's `encoding/json` into each plugin's own struct (case-insensitive keys applied in document order, `null` = zero value, wrong-typed known fields and non-RFC 3339 `last_updated` fail the whole file; `tools/m1a1-oracle/manifest_parity.py`); an enabled
source whose file is missing, unreadable or not a valid SRS is skipped with a diagnostic.
Unknown arguments are configuration errors. `domain_set_light`/`sd_set_light` have no executable
or matcher form (their reference `Match` is constant false, so `qname $tag` is not supported for them);
`domain_mapper` consumes them, and `si_set` is consumed by `resp_ip`. `adguard_rule` additionally works as a
direct `qname $tag` matcher with the reference semantics: a name matches when no `@@` exception rule
matches and a block rule does (domain rules are case-insensitive, regexps match the normalized name);
`domain_mapper` still consumes only the block export.

## `domain_mapper`

Arguments `rules[]` (`tag`, `mark` 0..=63, `ctx_mark`, `output_tag`), `default_mark` (0..=63),
`default_ctx_mark`, `default_tag`. A rule tag must name a rule provider above; the same provider
may appear in several rules. Execution takes the query name in DNS presentation form and
applies, on a match, fast flags (bit `mark`), context marks, the joined output tags to the
routing `domain_set` field and the joined sources to `matched_rule_source`; with no match it
applies the defaults (flag, context mark, tag; nothing when zero/empty). Matching, inheritance
and merging are the `ValuedDomainMatcher` semantics, verified against the reference mapper.
Rules the reference mapper skips (no type prefix, unknown type, a regexp Go rejects) are skipped with a
diagnostic; a non-regexp rule with a non-ASCII pattern is skipped too (a query name in presentation form is
always ASCII, so such a rule cannot match anyway). Exception: a `domain:`/`keyword:` pattern that the
reference's Unicode lowercasing turns into ASCII (for example the Kelvin sign, U+212A, becomes `k`) can match
ASCII names, which this host does not reproduce, so the configuration is refused naming the rule.

**Regexp rules never disappear silently.** Patterns run on the Rust engine after a Go-compatibility gate
(`matcher-core/src/regex.rs`): the usual Go forms (`\d \w \s \b`, `(?:…)`, `(?i)`/`(?i:…)`, POSIX classes,
`\xHH`, lazy quantifiers, counted repeats up to 1000, escaped punctuation) are accepted and behave identically on
the ASCII names the matcher sees. A pattern Go rejects (nested repetition, bad counts, unbalanced brackets or
parentheses, …) is skipped like the reference does. A pattern that is **valid Go but outside the gate** (`\p{L}`,
`\Q…\E`, named groups, lookaround-looking groups, literal braces, non-ASCII, `\a`/`\v`/`\0`, …) makes the
configuration fail with a message naming the rule, on the `domain_mapper` path and on the direct
`qname $adguard` path alike. Parity evidence: `tools/m1a1-oracle/regexp_parity.py` (257 patterns × 39 names
against Go's `regexp`; no violations).

**Deviation (approved 2026-10-07):** `full:` rules match case-insensitively and rules that
differ only in case are merged; the reference matches `full:` case-sensitively on the raw
name. Everything else is identical on the production corpus.

## `mark`

Quick matcher and executor over **context** marks (`mark N [N…]`, base-10 `u32`); independent of
`fast_mark`.

## Limits and known differences

- SRS files: header must be `SRS` v1–3 + zlib; decoding is bounded in bytes (64 MiB inflated, 128 MiB
  file) and in objects (2,000,000 rules/strings/prefixes per file, 262,144 words per succinct-set
  bitmap, 16 MiB of labels, 64 MiB of materialized keys, nesting 16; declared lengths never pre-allocate). A damaged body yields the rules read before the
  damage (diagnostic); the reference reads on and may desynchronize.
- `domain_set` and `sd_set` are rule exporters in the reference host but are not accepted as `domain_mapper` providers here
  (reported as "does not support rule export"); `config_all` does not use them that way.
- The order of joined source names for one `sd_set_light` is manifest order (the reference's is random).
- Memory: building the mapper is not yet compact; see the resource note in the validation record.
