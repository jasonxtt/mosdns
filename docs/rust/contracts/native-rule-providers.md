# Native rule providers, `domain_mapper` and `mark` (M1-a1, M1-a2)

Scope: the read side that lets the native host load the rule providers a production
`config_all` uses and route queries by `domain_mapper` marks (M1-a1), and the management side
(source add/edit/delete, download, auto-update, `show`/`save`/`post`, hot reload; M1-a2, section
"Management" below). Evidence: `docs/rust/validation-records/10-07-m1-a1-rule-providers/` and
`docs/rust/validation-records/10-07-m1-a2-rule-management/`.

## Providers

All providers load at compile/start time from existing disk state; compiling (and `check`)
never writes. Writes, downloads and reloads happen only in a running host, through the
management API and the background jobs described under "Management". Relative paths follow
the configuration root (`-d`).

| Type | Arguments | Source of rules |
| --- | --- | --- |
| `domain_set_light` | `exps`, `files`, `sets` (ignored) | `exps`, then each file: SRS or text. A missing file is an empty source. |
| `sd_set_light` | `local_config` (required), `socks5` (downloads) | JSON manifest; enabled sources with a `files` path; each `.srs` file. `enable_regexp` gates regexp items. |
| `si_set` | `local_config` (required), `socks5` (downloads) | same manifest shape; CIDRs of enabled `.srs` files; usable in `resp_ip $tag`. |
| `adguard_rule` | `dir` (required), `socks5` | `<dir>/config.json` + `<dir>/<id>.rules` of enabled lists. Only block rules are exported; `@@` lines never reach the mapper. Lists that are not on disk are skipped at compile time and downloaded in the background once the host serves. |

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
- Memory: the mapper's matcher is not yet compact (about 150 bytes per `domain:` rule; Mem-2). See the
  resource notes in the M1-a2 and Mem-1 validation records.

## Management (M1-a2)

The reference API is served under `/plugins/{tag}` for each provider tag; methods, paths, status
codes, content types and JSON bodies follow the reference handlers (task research
`go-semantics.md`):

- `sd_set_light`/`si_set`: `GET /config`, `PUT|DELETE /config/{name}`, `POST /update/{name}` (202,
  background). `PUT` with non-blank `files` and `url` downloads, then reloads.
- `adguard_rule`: `GET|POST /rules`, `PUT|DELETE /rules/{id}`, `POST /update`, `POST /update/{id}`.
  The WebUI addresses the fixed tag `adguard`.
- `domain_set_light`: `GET /show` (`q`, `limit` default 100, `offset`), `GET /save`, `POST /post`
  (`{"values": [...]}`), only when the first file is a `.txt` file.

Manifests are kept in memory, written as the reference writes them (indented, sorted by name, or
by id for AdGuard; zero AdGuard `last_updated` omitted) through a same-directory temporary file and
a rename, and only then adopted in memory; a failed write changes nothing (500). Manifests written
by either implementation are read by the other (round-trip evidence in the record). Downloads:
HTTP/HTTPS with system roots, at most 10 redirects, optional SOCKS5 (no authentication, remote
name resolution), 60 s (`sd_set_light`/`si_set`) or 30 s (AdGuard) overall, 64 MiB body limit;
validated (SRS decode) before the rule file is replaced by rename. Downloads of one source never
overlap: a request during a download makes it run once more. Auto-update uses the reference
cadence (first check 1 min after start; sources every 10 min, AdGuard every minute) and predicate.
Errors are logged, as in the reference.

Hot reload: after a change the provider's own state (rule counts, the `si_set` list, the AdGuard
direct matcher) and every `domain_mapper` naming it are rebuilt off the DNS thread (one dedicated
build thread, so builds never run concurrently) and published as a whole generation; a query keeps
the generation it read; a failed build keeps the previous generation. Caches are not flushed (the
reference does not either). Shutdown cancels downloads and joins every job inside the host drain.

Deliberate differences (data safety or not configuration):

- A manifest that exists but cannot be parsed is never overwritten; changes to that provider fail
  (the reference starts empty and overwrites it on the next change).
- `domain_set_light` `/post` values are normalized like file lines (trim, skip blank and `#`, drop
  values with a line break), as approved for `domain_set` (A1); `/save` refuses a non-`.txt` first
  file (the reference would overwrite an SRS file with text); after `/post` the provider holds what
  a restart reads (the reference drops `exps` from memory until restart).
- Downloaded content and posted values that this host would refuse at start (valid-Go regexps
  outside the gate, Unicode-lowercase rules) are rejected, so a change can never make the next start
  fail. AdGuard downloads are otherwise unvalidated text, as in the reference.
- `DELETE` keeps a rule file that another source of a configured provider still reads, however its
  path is spelled or linked (the reference deletes it and the other source loses its rules).
- A download whose source was removed or repointed meanwhile is discarded; files are always
  replaced by rename (the reference writes SRS files in place).
- No `HTTP(S)_PROXY` environment proxy (the reference's default transport honours it); use `socks5`.
- New `last_updated` values are written in UTC (`…Z`); the instant is the same.
- Auto-update reloads only when a download succeeded (the reference reloads after every batch).
- Start-up count reconciliation rewrites only counts read while compiling (enabled sources); the
  reference also recounts disabled AdGuard lists at start.

Known limits with `native_management` (not used by `config_all`): a managed apply compiles a new
view with its own rule-source state. When the new view is installed the retired view is fenced at
once (no new jobs, no writes, mutating requests 503) and its jobs are joined during retirement;
every view's owner of one manifest or rule file shares that file's write lock, so a write already
under way finishes before the new view's first write and is never applied after it. A background rule write that
lands between a managed candidate's compile and its commit makes that apply fail with the existing
retryable input conflict. `domain_set_light` with `exps` or several files: `/show` lists `exps` and
every file and `/post` writes the first file, so posting the shown list back duplicates them there
(the reference has the same result after a restart).

Memory: a rebuild of a large mapper briefly holds the old generation (queries may still use it), the
rule text, the build's temporaries and the new generation. After publication the host returns the freed
pages to the system (glibc `malloc_trim` on the build thread; a fixed glibc mmap threshold and no
transparent huge pages for the process keep them returnable), so with the production rule set the
resident size settles near the start-up level after any number of rule-source rebuilds (numbers in
`docs/rust/validation-records/10-08-mem-1/`). The replaced generation is freed off the DNS thread. Not covered
yet: a `native_management` configuration apply (not used by `config_all`) is not followed by a trim.
