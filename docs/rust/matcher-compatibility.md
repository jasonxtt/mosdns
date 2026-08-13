# Rust matcher compatibility matrix

Status: Slice 5 matcher foundation evidence complete; Rust remains experimental.
This is the authoritative contract record for
`08-13-rust-matcher-foundation`. It maps every matcher behavior and consumer to
a fixture owner and a migration status. MosDNS semantics are authoritative;
KixDNS tests are only supplementary.

Last verified against working tree on `2026-08-13`.

## 1. Interface contracts

### domain matcher (`pkg/matcher/domain`)

```go
type Matcher[T any] interface {
    Match(s string) (v T, ok bool)   // s may be fqdn or not; case-insensitive
}
type WriteableMatcher[T any] interface {
    Matcher[T]
    Add(pattern string, v T) error
}
```

- Implementations: `FullMatcher` (`full`), `SubDomainMatcher` (`domain`),
  `RegexMatcher` (`regexp`), `KeywordMatcher` (`keyword`), `MixMatcher`
  (combines all four).
- `MixMatcher` default type is configurable via `SetDefaultMatcher`:
  - `domain.NewDomainMixMatcher()` sets default `domain`
    (used by `domain_set`, `sd_set`, `sd_set_light`, `domain_set_light`,
    `adguard`).
  - `rewrite`/`redirect` set default `full` and use a valued `T`.
- Precedence inside `MixMatcher.Match` (fixed order): `full` → `domain` →
  `regexp` → `keyword`. First non-empty bucket wins.
- Normalization: `NormalizeDomain` = `strings.ToLower(TrimDot(s))`; `TrimDot`
  removes a single trailing `.`. `tryFastNormalize` returns the original string
  without allocation when already lower-case and non-fqdn.
- Keyword matching uses `strings.Contains` over the normalized qname.
- Regex rules are compiled with Go `regexp`; the match input is the
  lower-case non-fqdn name. Go and Rust regex dialects are NOT automatically
  identical; parity fixtures must detect differences and select Go.
- `Add` semantics: duplicate rules replace the stored value (`Len` does not
  grow). `SubDomainMatcher` stores the value on the terminal label node, so a
  shorter suffix rule stays visible below longer rules until the terminal node
  of the longer rule is reached.
- Two distinct text-parse paths exist and must both be reproduced:
  - `pkg/matcher/domain.LoadFromTextReader`: strips inline `#` comments
    (`utils.RemoveComment`), `TrimSpace`, rejects rows containing whitespace
    via `patternOnly`, and propagates parse errors with the line number.
  - `domain_set.loadFileInternal` (and its `LoadFile`): only `TrimSpace`,
    skips whole-line `#` comments and blank rows, then calls
    `MixMatcher.Add(line, struct{}{})` **ignoring errors**. It does not strip
    inline `#` text, so a `full:tagged # note` row is added with the inline
    text in the pattern and does not match `tagged`. Regexp rows that fail to
    compile are silently skipped.
- `full:`/`domain:`/`regexp:`/`keyword:` prefixes split by the first `:`. An
  unrecognised type or a missing default type returns an error.

### netlist matcher (`pkg/matcher/netlist`)

```go
type Matcher interface {
    Match(addr netip.Addr) bool
}
```

- `List` stores masked `netip.Prefix` values in one slice, all converted to
  the IPv4-mapped 16-byte space (`to6`); an IPv4 prefix has `bits + 96`.
- `Append` masks each prefix immediately; `Sort` sorts by address and folds
  overlap: same-address keeps the smaller `bits`, and a prefix whose address
  is already contained by the previous entry is dropped.
- `Contains`/`Match` binary-search; `!list.sorted` panics. `Match` returns
  false for invalid addresses.

### provider contracts (`plugin/data_provider/iface.go`)

```go
type DomainMatcherProvider interface { GetDomainMatcher() domain.Matcher[struct{}] }
type IPMatcherProvider      interface { GetIPMatcher() netlist.Matcher }
type RuleExporter interface {
    GetRules() ([]string, error)          // e.g. "full:google.com", "regexp:.*"
    Subscribe(callback func())
}
type DetailedRuleExporter interface { GetRuleEntries() ([]RuleEntry, error) }
```

## 2. Producer / consumer matrix

Classification: **direct** = adapted to Rust in this task (Slice 4);
**inherited** = derives through a direct matcher and needs no separate adapter;
**deferred** = fixtures/interfaces only, migrated in a follow-up fan-out task.

| Owner | Kind | Matcher type | Default type | Value | Status | Fixture owner |
| --- | --- | --- | --- | --- | --- | --- |
| `pkg/matcher/domain` (Full/Sub/Regex/Keyword/Mix) | engine | domain | configurable | `T` | direct (Slice 2) | golden + parity |
| `pkg/matcher/netlist` (List) | engine | netlist | — | bool | direct (Slice 3) | golden + parity |
| `domain_set` | provider+matcher | domain | domain | `struct{}` | direct (Slice 4) | reload/SRS/composition |
| `ip_set` | provider+matcher | netlist | — | bool | direct (Slice 4) | reload/SRS/composition |
| `base_domain` | matcher (composes providers + anonymous set) | domain | domain | `struct{}` | direct (Slice 4) | composition/priority |
| `base_ip` | matcher (composes providers + anonymous set) | netlist | — | bool | direct (Slice 4) | composition/masking |
| `qname` | inherited via `base_domain` | domain | domain | `struct{}` | inherited | — |
| `cname` | inherited via `base_domain` | domain | domain | `struct{}` | inherited | — |
| `client_ip` | inherited via `base_ip` | netlist | — | bool | inherited | — |
| `resp_ip` | inherited via `base_ip` | netlist | — | bool | inherited | — |
| `ptr_ip` | inherited via `base_ip` | netlist | — | bool | inherited | — |
| `sd_set` | provider+matcher | domain | domain | `struct{}` | deferred | rules/composition |
| `sd_set_light` | provider+matcher | domain | domain | `struct{}` | deferred | rules/composition |
| `domain_set_light` | provider+matcher | domain | domain | `struct{}` | deferred | rules/composition |
| `si_set` | provider+matcher | netlist | — | bool | deferred | rules/composition |
| `domain_mapper` | aggregator (consumes `RuleExporter`, compiles results) | domain | domain | valued | deferred | rule text/result fixtures |
| `rewrite` | executable | domain | **full** | `*rewriteTarget` | deferred | — |
| `redirect` | executable | domain | **full** | `string` | deferred | — |
| `adguard` | executable | domain | domain | `struct{}` | deferred | — |
| `hosts` | library (`pkg/hosts`) | domain | n/a (valued) | `*IPs` | deferred | — |

Consumers of `base_domain`/`base_ip` use a `MatchFunc` that selects the query
field: qname/question, cname chain, client address, response address, or PTR
reverse name. These inherit the Rust path once the base matcher is Rust; no
per-matcher adapter is needed.

## 3. Reload contracts

Three transactional replace patterns are used; all follow "build a complete new
snapshot off-path, validate, then atomically publish":

| Provider | Snapshot holder | Publish mechanism | Subscriber notify |
| --- | --- | --- | --- |
| `domain_set` | `mixM *MixMatcher` | `d.mu.Lock()` swap on API `/post` | `notifySubscribers` |
| `ip_set` | `matcherVal atomic.Value` (`MatcherGroup`) | `rebuildSnapshot()` after reload | none |
| `sd_set` | `matcher atomic.Value` (`*MixMatcher`) | `process()` builds new matcher, `Store` | `Subscribe` |
| `si_set` | `matcher atomic.Value` | rebuild on reload | `Subscribe` |
| `domain_set_light` | rebuild on reload | swap | `Subscribe` |

Reload sources: local files (`files`), explicit expressions (`exps`/`ips`),
API `POST` (text for `domain_set`), and SRS binary (`magic "SRS"`, domain and
IP variants). Online rule updates download to local files then trigger the same
reload path; downloads/API/save stay in Go.

Failure policy: an invalid build must not publish a partial snapshot and must
leave the previous matcher active; the error surfaces through the existing
API/log boundary.

## 4. Behavior → fixture mapping

Each row names the fixture that owns the behavior and the package that must
exercise it (Go golden first, then the same vector against pure Rust and the
Linux+cgo adapter).

| Behavior | Fixture (executable) | Owner package |
| --- | --- | --- |
| `NormalizeDomain`: lower, single trailing dot, empty, multiple trailing dots | `TestGoldenNormalizeDomain` | `pkg/matcher/domain` |
| `MixMatcher` precedence full→domain→regex→keyword | `TestGoldenMixMatcher/precedence_*`, `domain_wins_*`, `regex_before_*` | `pkg/matcher/domain` |
| Default matcher type selection (domain vs full) | `TestGoldenMixMatcher/default_domain_*`, `default_full_*` | `pkg/matcher/domain` |
| Duplicate rule replace + value | `TestGoldenMixMatcher/duplicate_*` | `pkg/matcher/domain` |
| Invalid rules: bad regexp, unknown type, missing default | `TestGoldenMixMatcherInvalidRules` | `pkg/matcher/domain` |
| Root (`.`) and keyword-empty rules | `TestGoldenMixMatcher/root_rule_*`, `keyword_empty_string_*` | `pkg/matcher/domain` |
| Keyword `strings.Contains` substring semantics | `TestGoldenMixMatcher/keyword_is_substring_*` | `pkg/matcher/domain` |
| Longer suffix rule shadows shorter | `TestGoldenMixMatcher/longer_domain_rule_*` | `pkg/matcher/domain` |
| IPv4/IPv6/mapped-16 normalization + masking | `TestGoldenListMaskingAndMapping` | `pkg/matcher/netlist` |
| Overlapping prefix collapse + dedup | `TestGoldenListOverlapCollapse` | `pkg/matcher/netlist` |
| Binary-search containment boundaries | `TestGoldenListContainmentBoundaries` | `pkg/matcher/netlist` |
| Invalid/zero prefixes, unsorted panic | `TestGoldenListInvalidInput` | `pkg/matcher/netlist` |
| `MatcherGroup` composition (domain_set.otherM, ip_set.MatcherGroup) | `TestGoldenDomainSetComposition`, `TestGoldenIPSetComposition` | `plugin/data_provider/domain_set`, `ip_set` |
| Atomic reload: `POST /post` replace + subscriber notify | `TestGoldenDomainSetReloadViaPost` | `plugin/data_provider/domain_set` |
| Atomic snapshot reload (`rebuildSnapshot`) | `TestGoldenIPSetSnapshotReload` | `plugin/data_provider/ip_set` |
| RuleExporter `GetRules` copy + subscribe fan-out | `TestGoldenDomainSetRuleExporter` | `plugin/data_provider/domain_set` |
| Text file parsing incl. inline-`#` NOT stripped | `TestGoldenDomainSetTextLoad` | `plugin/data_provider/domain_set` |
| IP text parse (comments, host addr) + invalid row fails file | `TestGoldenIPSetPlainLoad`, `TestGoldenIPSetInvalidLineErrorsFile` | `plugin/data_provider/ip_set` |
| SRS domain parse | `TestGoldenDomainSetSRSLoad` | `plugin/data_provider/domain_set` |
| SRS IP parse (range→prefix, incl. mapped normalize) | `TestGoldenIPSetSRSLoad`, `TestGoldenIPSetNormalizePrefix` | `plugin/data_provider/ip_set` |
| Real rule-set representative subset load | `TestGoldenRealRuleSetLoad` (Slice 0c) | `plugin/data_provider/domain_set` |
| `domain_mapper` rule text aggregation + result compilation (deferred, fixtures only) | design artifact (Slice 0c) | `plugin/data_provider/domain_mapper` |

## 5. Migration boundary reminders

- Do not change YAML fields, API bodies/status codes, rule-file text, SRS
  behavior, metrics, audit/query-context fields, or `special_groups` semantics.
- `domain_mapper`, `sd_set`, `sd_set_light`, `si_set`, `domain_set_light`,
  `adguard`, `hosts`, `rewrite`, `redirect` keep their Go ownership in this
  task; only their input fixtures and compatible interfaces are in scope.
- Go `regexp` vs Rust regex differences must be detected by parity fixtures and
  must select Go, never silently reinterpret a rule.
- Reload must be transactional; no partial snapshot may be published.

## 6. Slice 5 evidence boundary

- Direct Linux+cgo positive/negative FFI assertions cover `domain_set`,
  `ip_set`, base-domain Files/SRS/inline rules, and base-IP Files/inline
  rules. The surrounding `MatchFunc`/CNAME and Go fallback tests remain in
  place.
- `scripts/benchmark-rust-matchers.sh` and
  `docs/rust/benchmarks/matcher-foundation.md` record fixed-fixture build,
  lookup, index-entry, Go-side allocation, and transitional cgo evidence.
  Per-object Rust heap/RSS is explicitly not claimed at this ABI boundary.
- `scripts/smoke-rust-matcher-mos-test.sh` exercises valid and malformed API
  reloads, concurrent query/reload, Go-only fallback mode, and process restart
  using only temporary files and high loopback ports on `mos-test`.
- Provider fan-out (`sd_set`, `sd_set_light`, `domain_set_light`, `si_set`)
  and `domain_mapper` remain deferred and require an independently approved
  continuation task. No status in this matrix authorizes their migration.
