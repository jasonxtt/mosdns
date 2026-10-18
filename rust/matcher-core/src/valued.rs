use std::collections::{BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use regex::Regex;

use crate::DomainSuffixMatcher;
use crate::normalize::{NonAsciiRule, normalize};
use crate::regex::compile_go_compatible;

/// One domain rule and the result metadata associated with it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValuedRule {
    pub rule: String,
    pub fast_marks: u64,
    pub ctx_marks: Vec<u32>,
    pub joined_tags: String,
    pub joined_sources: String,
}

impl ValuedRule {
    /// Creates a valued rule from the compact result fields used by the ABI.
    pub fn new<I, R, T, S>(
        rule: R,
        fast_marks: u64,
        ctx_marks: I,
        joined_tags: T,
        joined_sources: S,
    ) -> Self
    where
        I: IntoIterator<Item = u32>,
        R: Into<String>,
        T: Into<String>,
        S: Into<String>,
    {
        Self {
            rule: rule.into(),
            fast_marks,
            ctx_marks: ctx_marks.into_iter().collect(),
            joined_tags: joined_tags.into(),
            joined_sources: joined_sources.into(),
        }
    }
}

/// A merged result returned by a valued domain lookup.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValuedMatchResult {
    pub fast_marks: u64,
    pub ctx_marks: Vec<u32>,
    pub joined_tags: String,
    pub joined_sources: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValuedBuildError {
    InvalidRule(String),
    InvalidRegex { rule: String, error: String },
    UnsupportedRule { rule: String, error: String },
}

impl Display for ValuedBuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRule(rule) => write!(f, "invalid valued domain rule: {rule}"),
            Self::InvalidRegex { rule, error } => {
                write!(f, "invalid valued domain regexp {rule:?}: {error}")
            }
            Self::UnsupportedRule { rule, error } => {
                write!(f, "unsupported valued domain rule {rule:?}: {error}")
            }
        }
    }
}

impl std::error::Error for ValuedBuildError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuleKind {
    Full,
    Domain,
    Keyword,
    Regexp,
}

#[derive(Clone, Debug, Default)]
struct AccumulatedResult {
    fast_marks: u64,
    ctx_marks: BTreeSet<u32>,
    joined_tags: String,
    joined_sources: String,
}

impl AccumulatedResult {
    fn merge_rule(&mut self, rule: &ValuedRule) {
        self.fast_marks |= rule.fast_marks;
        self.ctx_marks.extend(rule.ctx_marks.iter().copied());
        append_joined_value(&mut self.joined_tags, &rule.joined_tags);
        append_joined_value(&mut self.joined_sources, &rule.joined_sources);
    }

    fn merge_result(&mut self, other: &Self) {
        self.fast_marks |= other.fast_marks;
        self.ctx_marks.extend(other.ctx_marks.iter().copied());
        append_joined_value(&mut self.joined_tags, &other.joined_tags);
        append_joined_value(&mut self.joined_sources, &other.joined_sources);
    }

    fn into_result(self) -> ValuedMatchResult {
        ValuedMatchResult {
            fast_marks: self.fast_marks,
            ctx_marks: self.ctx_marks.into_iter().collect(),
            joined_tags: self.joined_tags,
            joined_sources: self.joined_sources,
        }
    }
}

enum OverlapRule {
    Keyword {
        pattern: String,
        result: Arc<ValuedMatchResult>,
    },
    Regexp {
        matcher: Regex,
        result: Arc<ValuedMatchResult>,
    },
}

/// Immutable valued domain matcher used by the later Go mapper adapter.
pub struct ValuedDomainMatcher {
    full: HashMap<String, Arc<ValuedMatchResult>>,
    domain: DomainSuffixMatcher<Arc<ValuedMatchResult>>,
    overlaps: Vec<OverlapRule>,
    rule_count: usize,
    pooled_result_count: usize,
}

impl ValuedDomainMatcher {
    /// Builds an immutable matcher from one complete ordered rule batch.
    ///
    /// Duplicate rule records are merged in input order. Every rule is
    /// validated before the matcher is returned, so construction cannot expose
    /// a partially compiled snapshot.
    ///
    /// The build's temporaries borrow the rule strings from `rules` instead of
    /// copying them (only `full:` rules keep a lowercase copy), so a large
    /// batch costs little more than the matcher.
    pub fn build(rules: &[ValuedRule]) -> Result<Self, ValuedBuildError> {
        // Unique rule strings in first-occurrence order with their merged own
        // results; `slot` finds a rule string's position.
        let mut entries: Vec<(&str, AccumulatedResult)> = Vec::with_capacity(rules.len());
        let mut slot: HashMap<&str, usize> = HashMap::with_capacity(rules.len());
        let mut full_own: HashMap<String, AccumulatedResult> = HashMap::new();
        for rule in rules {
            let (kind, pattern) = parse_rule(&rule.rule)?;
            if kind == RuleKind::Full && pattern.is_ascii() {
                full_own
                    .entry(pattern.to_ascii_lowercase())
                    .or_default()
                    .merge_rule(rule);
            }
            if !pattern.is_ascii() {
                return Err(ValuedBuildError::UnsupportedRule {
                    rule: rule.rule.clone(),
                    error: NonAsciiRule.to_string(),
                });
            }
            let at = *slot.entry(rule.rule.as_str()).or_insert_with(|| {
                entries.push((rule.rule.as_str(), AccumulatedResult::default()));
                entries.len() - 1
            });
            entries[at].1.merge_rule(rule);
        }
        drop(slot);

        let index = DomainIndex::new(&entries);
        // Inheritance without recursion: an ancestor's pattern is strictly
        // shorter than its descendant's (a `domain:` rule of the same name
        // only precedes the `full:` rule that inherits it), so processing by
        // pattern length finishes every ancestor first. Each result becomes
        // its effective result in place. `full:` rules are not ancestors, and
        // their variants inherit separately below, so they are skipped here.
        let mut by_depth: Vec<(usize, bool, usize)> = Vec::with_capacity(entries.len());
        for (at, (key, _)) in entries.iter().enumerate() {
            let (kind, pattern) = parse_rule(key)?;
            if kind != RuleKind::Full {
                by_depth.push((pattern.len(), kind != RuleKind::Domain, at));
            }
        }
        by_depth.sort_unstable();
        for (_, _, at) in by_depth {
            if let Some(ancestor) = inherited_from(entries[at].0, &index)? {
                let (inherited, result) = pair_mut(&mut entries, ancestor, at);
                result.merge_result(inherited);
            }
        }

        let mut pool: HashMap<ValuedMatchResult, Arc<ValuedMatchResult>> = HashMap::new();
        let mut full: HashMap<String, Arc<ValuedMatchResult>> = HashMap::new();
        // `full:` rules that differ only in ASCII case are one rule (approved
        // deviation: they match the same names). Each variant first inherits
        // exactly as in the reference (from its own spelling); the variants'
        // results are then merged in input order.
        let mut full_groups: Vec<(String, Vec<&str>)> = Vec::new();
        let mut full_group_index: HashMap<String, usize> = HashMap::new();
        let mut full_rules = 0usize;
        for (key, _) in &entries {
            let (kind, pattern) = parse_rule(key)?;
            if kind == RuleKind::Full {
                full_rules += 1;
                let lowered = pattern.to_ascii_lowercase();
                let slot = *full_group_index.entry(lowered.clone()).or_insert_with(|| {
                    full_groups.push((lowered, Vec::new()));
                    full_groups.len() - 1
                });
                full_groups[slot].1.push(key);
            }
        }
        drop(full_group_index);

        let full_group_count = full_groups.len();
        for (lowered, keys) in full_groups {
            // The variants' own rules in input order, then what each
            // spelling inherits.
            let mut merged = full_own.remove(&lowered).unwrap_or_default();
            for key in keys {
                if let Some(ancestor) = inherited_from(key, &index)? {
                    merged.merge_result(&entries[ancestor].1);
                }
            }
            // A rule that differs only by the trailing dot replaces the
            // earlier one, as in the reference's map.
            full.insert(
                full_key(&lowered),
                intern_result(&mut pool, merged.into_result()),
            );
        }
        drop(index);

        let unique_rules = entries.len();
        let mut domain = DomainSuffixMatcher::new();
        let mut overlaps = Vec::new();
        for (key, effective) in entries {
            let (kind, pattern) = parse_rule(key)?;
            if kind == RuleKind::Full {
                continue;
            }
            let result = intern_result(&mut pool, effective.into_result());
            match kind {
                RuleKind::Full => unreachable!("handled above"),
                RuleKind::Domain => domain.add(pattern, result),
                RuleKind::Keyword => overlaps.push(OverlapRule::Keyword {
                    pattern: normalize(pattern),
                    result,
                }),
                RuleKind::Regexp => {
                    let matcher = compile_go_compatible(pattern).map_err(|error| match error {
                        crate::RegexBuildError::Unsupported { .. } => {
                            ValuedBuildError::UnsupportedRule {
                                rule: key.to_owned(),
                                error: error.to_string(),
                            }
                        }
                        other => ValuedBuildError::InvalidRegex {
                            rule: key.to_owned(),
                            error: other.to_string(),
                        },
                    })?;
                    overlaps.push(OverlapRule::Regexp { matcher, result });
                }
            }
        }

        Ok(Self {
            full,
            domain,
            overlaps,
            rule_count: unique_rules - full_rules + full_group_count,
            pooled_result_count: pool.len(),
        })
    }

    /// Returns the number of unique rule strings in this snapshot.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.rule_count
    }

    /// Reports whether this snapshot contains no unique rules.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rule_count == 0
    }

    /// Returns the number of pooled metadata payloads in this snapshot.
    #[must_use]
    pub const fn pooled_result_len(&self) -> usize {
        self.pooled_result_count
    }

    /// Matches a query and merges full, domain, keyword, and regexp results.
    #[must_use]
    pub fn r#match(&self, name: &str) -> Option<ValuedMatchResult> {
        if !name.is_ascii() {
            return None;
        }
        let normalized = normalize(name);
        let mut merged = self
            .full
            .get(&full_key(name))
            .map(|result| result.as_ref().clone());

        if let Some(result) = self.domain.match_normalized(&normalized) {
            merged = Some(merge_results(merged, result));
        }

        for overlap in &self.overlaps {
            let matches = match overlap {
                // The reference ignores a keyword that normalizes to nothing.
                OverlapRule::Keyword { pattern, .. } => {
                    !pattern.is_empty() && normalized.contains(pattern)
                }
                OverlapRule::Regexp { matcher, .. } => matcher.is_match(&normalized),
            };
            if matches {
                let result = match overlap {
                    OverlapRule::Keyword { result, .. } | OverlapRule::Regexp { result, .. } => {
                        result.as_ref()
                    }
                };
                merged = Some(merge_results(merged, result));
            }
        }

        merged
    }
}

/// The `full:` lookup key: ASCII-lowercased, with one trailing dot ensured.
fn full_key(name: &str) -> String {
    let mut key = name.to_ascii_lowercase();
    if !key.ends_with('.') {
        key.push('.');
    }
    key
}

fn parse_rule(rule: &str) -> Result<(RuleKind, &str), ValuedBuildError> {
    let Some((kind, pattern)) = rule.split_once(':') else {
        return Err(ValuedBuildError::InvalidRule(rule.to_owned()));
    };
    let kind = match kind {
        "full" => RuleKind::Full,
        "domain" => RuleKind::Domain,
        "keyword" => RuleKind::Keyword,
        "regexp" => RuleKind::Regexp,
        _ => return Err(ValuedBuildError::InvalidRule(rule.to_owned())),
    };
    Ok((kind, pattern))
}

/// What a rule inherits: the reference merges the results of every
/// `domain:` rule named by the pattern's dotted tails (longest first), and
/// each of those has already merged the tails below it, so the longest
/// existing one carries everything (a `full:` rule starts with the `domain:`
/// rule of its own name, which in turn carries all the tails). Returns that
/// rule's position in the build's entries.
fn inherited_from(key: &str, index: &DomainIndex<'_>) -> Result<Option<usize>, ValuedBuildError> {
    let (kind, pattern) = parse_rule(key)?;
    Ok(index.nearest_ancestor(kind == RuleKind::Full, pattern))
}

/// The ancestor's result and the descendant's, which are distinct entries.
fn pair_mut<'v, T>(entries: &'v mut [(&str, T)], ancestor: usize, at: usize) -> (&'v T, &'v mut T) {
    assert_ne!(ancestor, at, "a rule never inherits from itself");
    if ancestor < at {
        let (low, high) = entries.split_at_mut(at);
        (&low[ancestor].1, &mut high[0].1)
    } else {
        let (low, high) = entries.split_at_mut(ancestor);
        (&high[0].1, &mut low[at].1)
    }
}

/// The `domain:` rules, indexed by their dot-separated labels from the right,
/// so the rules a pattern's dotted tails name are found in one walk of the
/// pattern without building a string per tail. Flat arena (node 0 is the
/// root): a rule with a huge number of labels must not recurse. Labels are
/// borrowed from the rule strings.
struct DomainIndex<'a> {
    nodes: Vec<IndexNode<'a>>,
}

#[derive(Default)]
struct IndexNode<'a> {
    children: HashMap<&'a str, usize>,
    /// The entry of a rule (`domain:<labels>`) that ends here.
    entry: Option<usize>,
}

impl<'a> DomainIndex<'a> {
    fn new<T>(entries: &[(&'a str, T)]) -> Self {
        let mut nodes = vec![IndexNode::default()];
        for (at, (key, _)) in entries.iter().enumerate() {
            if let Some(pattern) = key.strip_prefix("domain:") {
                let mut node = 0;
                for label in pattern.rsplit('.') {
                    node = match nodes[node].children.get(label) {
                        Some(&child) => child,
                        None => {
                            nodes.push(IndexNode::default());
                            let child = nodes.len() - 1;
                            nodes[node].children.insert(label, child);
                            child
                        }
                    };
                }
                nodes[node].entry = Some(at);
            }
        }
        Self { nodes }
    }

    /// The longest `domain:` rule among those a pattern inherits from (its
    /// own name for `full:` rules, then each tail after a dot).
    fn nearest_ancestor(&self, include_whole: bool, pattern: &str) -> Option<usize> {
        let labels = pattern.split('.').count();
        let mut nearest = None;
        let mut node = 0;
        for (consumed, label) in pattern.rsplit('.').enumerate() {
            let Some(&child) = self.nodes[node].children.get(label) else {
                break;
            };
            node = child;
            let tail_start = labels - 1 - consumed;
            if let Some(entry) = self.nodes[node].entry
                && (tail_start >= 1 || include_whole)
            {
                nearest = Some(entry);
            }
        }
        nearest
    }
}

fn intern_result(
    pool: &mut HashMap<ValuedMatchResult, Arc<ValuedMatchResult>>,
    result: ValuedMatchResult,
) -> Arc<ValuedMatchResult> {
    if let Some(existing) = pool.get(&result) {
        return Arc::clone(existing);
    }
    let shared = Arc::new(result.clone());
    pool.insert(result, Arc::clone(&shared));
    shared
}

fn merge_results(
    current: Option<ValuedMatchResult>,
    next: &ValuedMatchResult,
) -> ValuedMatchResult {
    let mut merged = current.map_or_else(AccumulatedResult::default, |result| AccumulatedResult {
        fast_marks: result.fast_marks,
        ctx_marks: result.ctx_marks.into_iter().collect(),
        joined_tags: result.joined_tags,
        joined_sources: result.joined_sources,
    });
    let next = AccumulatedResult {
        fast_marks: next.fast_marks,
        ctx_marks: next.ctx_marks.iter().copied().collect(),
        joined_tags: next.joined_tags.clone(),
        joined_sources: next.joined_sources.clone(),
    };
    merged.merge_result(&next);
    merged.into_result()
}

fn append_joined_value(joined: &mut String, value: &str) {
    for part in value
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if joined
            .split('|')
            .map(str::trim)
            .any(|existing| existing == part)
        {
            continue;
        }
        if !joined.is_empty() {
            joined.push('|');
        }
        joined.push_str(part);
    }
}

#[cfg(test)]
mod tests {
    use super::{ValuedBuildError, ValuedDomainMatcher, ValuedRule};

    #[test]
    fn full_rules_that_differ_only_in_case_merge_instead_of_overwriting() {
        let rules = vec![
            ValuedRule::new("full:Case.Example.com", 1, [], "first", "src-a"),
            ValuedRule::new("full:case.example.com", 2, [], "second", "src-b"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        for name in ["case.example.com.", "CASE.EXAMPLE.COM.", "Case.Example.Com"] {
            let result = matcher.r#match(name).expect("case-insensitive full match");
            assert_eq!(result.fast_marks, 3, "{name}");
            assert_eq!(result.joined_tags, "first|second", "{name}");
            assert_eq!(result.joined_sources, "src-a|src-b", "{name}");
        }
    }

    #[test]
    fn a_trailing_dot_makes_a_different_full_rule_and_the_later_one_replaces() {
        let rules = vec![
            ValuedRule::new("full:x.example", 1, [], "A", "sa"),
            ValuedRule::new("full:x.example.", 2, [], "B", "sb"),
            ValuedRule::new("full:y.example..", 4, [], "C", "sc"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        // The reference keeps one entry per dotted name; the later rule wins.
        let result = matcher.r#match("x.example.").expect("match");
        assert_eq!((result.fast_marks, result.joined_tags.as_str()), (2, "B"));
        // A doubled dot is a different name and never matches a query name.
        assert!(matcher.r#match("y.example.").is_none());
        assert!(matcher.r#match("y.example").is_none());
    }

    #[test]
    fn case_variant_full_rules_join_tags_and_sources_in_input_order() {
        let rules = vec![
            ValuedRule::new("full:X.example", 1, [], "A", "sa"),
            ValuedRule::new("full:x.example", 2, [], "B", "sb"),
            ValuedRule::new("full:X.example", 4, [], "C", "sc"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        let result = matcher.r#match("x.example.").expect("match");
        assert_eq!(result.fast_marks, 7);
        assert_eq!(result.joined_tags, "A|B|C");
        assert_eq!(result.joined_sources, "sa|sb|sc");
        assert_eq!(matcher.len(), 1, "one normalized rule");
    }

    #[test]
    fn case_variants_keep_each_spellings_own_inheritance() {
        // `full:X.example` inherits only from the domain rule spelled `X.example`
        // (reference behaviour), even though it also matches `x.example.`.
        let rules = vec![
            ValuedRule::new("full:X.example", 1, [], "A", "sa"),
            ValuedRule::new("domain:x.example", 2, [], "B", "sb"),
            ValuedRule::new("domain:X.example", 4, [], "C", "sc"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        for query in ["X.example.", "x.example.", "x.EXAMPLE."] {
            let result = matcher.r#match(query).expect("match");
            // The two domain rules share one trie node (normalized), the later one wins.
            assert_eq!(
                (result.fast_marks, result.joined_tags.as_str()),
                (5, "A|C"),
                "{query}"
            );
        }
    }

    #[test]
    fn a_long_chain_of_nested_domain_rules_builds_without_recursion() {
        // 6,000 rules, each a dotted tail of the previous one, listed from
        // the deepest to the shallowest.
        let mut rules = Vec::new();
        for depth in (1..=6_000usize).rev() {
            let name = format!("{}example", "a.".repeat(depth));
            rules.push(ValuedRule::new(
                format!("domain:{name}"),
                1 << (depth % 40),
                [],
                format!("t{}", depth % 7),
                "s",
            ));
        }
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        let result = matcher
            .r#match(&format!("x.{}example.", "a.".repeat(6_000)))
            .expect("match");
        // Every rule on the chain contributes (the deepest inherits them all).
        assert_eq!(result.joined_tags.split('|').count(), 7);
    }

    #[test]
    fn a_huge_dotted_domain_rule_builds_and_drops_without_recursion() {
        let deep = format!("domain:{}", "a.".repeat(524_288));
        let rules = vec![
            ValuedRule::new(&deep, 1, [], "d", "s"),
            ValuedRule::new("domain:a", 2, [], "e", "s"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        assert!(matcher.r#match("b.a.").is_some());
        drop(matcher);
    }

    #[test]
    fn a_huge_dotted_keyword_does_not_allocate_per_suffix() {
        // 524288 labels: one string per suffix would need hundreds of GiB.
        let keyword = format!("keyword:{}a", "a.".repeat(524_288));
        let rules = vec![
            ValuedRule::new(&keyword, 1, [], "k", "s"),
            ValuedRule::new("domain:a", 2, [], "d", "s"),
            ValuedRule::new("domain:a.a", 4, [], "e", "s"),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        // The keyword inherits from both domain rules, the longer tail first.
        let result = matcher
            .r#match(&format!("{}ax.", "a.".repeat(524_288)))
            .expect("match");
        assert_eq!(result.fast_marks, 7);
        assert_eq!(result.joined_tags, "k|e|d");
    }

    #[test]
    fn merges_overlapping_rule_types_and_inherits_domain_results() {
        let rules = vec![
            ValuedRule::new("domain:example.com", 1, [10], "base", "base-source"),
            ValuedRule::new("full:child.example.com", 2, [20], "full", "full-source"),
            ValuedRule::new("keyword:child", 4, [40], "keyword", "keyword-source"),
            ValuedRule::new(
                r"regexp:^child\.example\.com$",
                8,
                [80],
                "regex",
                "regex-source",
            ),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");

        let result = matcher
            .r#match("child.example.com.")
            .expect("overlapping rule match");

        assert_eq!(result.fast_marks, 15);
        assert_eq!(result.ctx_marks, vec![10, 20, 40, 80]);
        assert_eq!(result.joined_tags, "full|base|keyword|regex");
        assert_eq!(
            result.joined_sources,
            "full-source|base-source|keyword-source|regex-source"
        );
    }

    #[test]
    fn rejects_invalid_rule_types_and_regexps() {
        let unknown = [ValuedRule::new("nope:example.com", 0, [], "", "")];
        assert!(matches!(
            ValuedDomainMatcher::build(&unknown),
            Err(ValuedBuildError::InvalidRule(_))
        ));

        let invalid_regex = [ValuedRule::new("regexp:[", 0, [], "", "")];
        assert!(matches!(
            ValuedDomainMatcher::build(&invalid_regex),
            Err(ValuedBuildError::InvalidRegex { .. })
        ));
    }

    #[test]
    fn rejects_non_ascii_rules_and_unsafe_regexps_before_build() {
        for rule in [
            "full:例.example",
            "domain:例.example",
            "keyword:例",
            "regexp:^例$",
            r"regexp:^\p{L}+$",
        ] {
            let rules = [ValuedRule::new(rule, 0, [], "", "")];
            assert!(
                ValuedDomainMatcher::build(&rules).is_err(),
                "rule {rule:?} must not build a Rust valued matcher"
            );
        }
    }

    #[test]
    fn empty_snapshot_is_valid_and_has_no_match() {
        let matcher = ValuedDomainMatcher::build(&[]).expect("empty snapshot is valid");
        assert_eq!(matcher.len(), 0);
        assert_eq!(matcher.pooled_result_len(), 0);
        assert!(matcher.r#match("example.com").is_none());
    }

    #[test]
    fn duplicate_payloads_are_merged_and_pooled() {
        let rules = [
            ValuedRule::new("domain:example.com", 1, [30, 10], "a|b", "source"),
            ValuedRule::new("domain:example.com", 2, [10, 20], "b|c", "source|other"),
            ValuedRule::new(
                "domain:other.example",
                3,
                [30, 10, 20],
                "a|b|c",
                "source|other",
            ),
        ];
        let matcher = ValuedDomainMatcher::build(&rules).expect("valid valued rules");
        assert_eq!(matcher.len(), 2);
        assert_eq!(matcher.pooled_result_len(), 1);
        let result = matcher.r#match("example.com").expect("domain match");
        assert_eq!(result.fast_marks, 3);
        assert_eq!(result.ctx_marks, vec![10, 20, 30]);
        assert_eq!(result.joined_tags, "a|b|c");
        assert_eq!(result.joined_sources, "source|other");
    }

    #[test]
    fn concurrent_reads_share_one_immutable_snapshot() {
        use std::sync::Arc;
        use std::thread;

        let matcher = Arc::new(
            ValuedDomainMatcher::build(&[ValuedRule::new(
                "domain:example.com",
                1,
                [10],
                "tag",
                "source",
            )])
            .expect("valid valued rules"),
        );
        let workers = (0..8)
            .map(|_| {
                let matcher = Arc::clone(&matcher);
                thread::spawn(move || {
                    for _ in 0..1000 {
                        assert!(matcher.r#match("www.example.com").is_some());
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("reader thread");
        }
    }

    #[test]
    fn inheritance_and_overlap_order_are_deterministic() {
        let matcher = ValuedDomainMatcher::build(&[
            ValuedRule::new("domain:com", 1, [], "com", "com-source"),
            ValuedRule::new("domain:example.com", 2, [], "example", "example-source"),
            ValuedRule::new("full:child.example.com", 4, [], "full", "full-source"),
            ValuedRule::new("keyword:child", 8, [], "keyword", "keyword-source"),
            ValuedRule::new(
                r"regexp:^child\.example\.com$",
                16,
                [],
                "regexp",
                "regexp-source",
            ),
        ])
        .expect("valid valued rules");

        let result = matcher
            .r#match("CHILD.EXAMPLE.COM.")
            .expect("deterministic overlap match");
        assert_eq!(result.fast_marks, 31);
        assert_eq!(result.joined_tags, "full|example|com|keyword|regexp");
        assert_eq!(
            result.joined_sources,
            "full-source|example-source|com-source|keyword-source|regexp-source"
        );
    }

    #[test]
    fn empty_keywords_never_match() {
        let matcher = ValuedDomainMatcher::build(&[
            ValuedRule::new("keyword:", 1, [], "empty", "s"),
            ValuedRule::new("keyword:.", 2, [], "dot", "s"),
            ValuedRule::new("keyword:real", 4, [], "real", "s"),
        ])
        .expect("valid valued rules");
        assert!(matcher.r#match("unrelated.example.").is_none());
        assert_eq!(matcher.r#match("a.real.example.").unwrap().fast_marks, 4);
    }

    #[test]
    fn large_result_payload_is_sorted_and_deduplicated() {
        let marks: Vec<u32> = (0..4096).rev().chain(0..4096).collect();
        let matcher = ValuedDomainMatcher::build(&[ValuedRule::new(
            "domain:large.example",
            1,
            marks,
            "tag|tag",
            "source|source",
        )])
        .expect("large valued rule");
        let result = matcher
            .r#match("large.example")
            .expect("large domain match");
        assert_eq!(result.ctx_marks, (0..4096).collect::<Vec<_>>());
        assert_eq!(result.joined_tags, "tag");
        assert_eq!(result.joined_sources, "source");
    }

    #[test]
    fn generated_payloads_preserve_merge_invariants() {
        let rules = (0..64_u32)
            .map(|seed| {
                ValuedRule::new(
                    "domain:property.example",
                    1_u64 << (seed % 63),
                    [seed % 7, (seed * 3) % 7],
                    format!("tag-{}|tag-{}", seed % 3, seed % 3),
                    format!("source-{}|source-{}", seed % 5, seed % 5),
                )
            })
            .collect::<Vec<_>>();
        let matcher = ValuedDomainMatcher::build(&rules).expect("generated valued rules");
        let result = matcher
            .r#match("property.example")
            .expect("generated domain match");

        assert_eq!(result.ctx_marks, (0..7).collect::<Vec<_>>());
        assert_eq!(result.joined_tags, "tag-0|tag-1|tag-2");
        assert_eq!(
            result.joined_sources,
            "source-0|source-1|source-2|source-3|source-4"
        );
        assert_ne!(result.fast_marks, 0);
    }
}
