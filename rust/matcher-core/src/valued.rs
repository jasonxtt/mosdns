use std::collections::{BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use regex::Regex;

use crate::{DomainSuffixMatcher, normalize};

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

pub const VALUED_RULE_BATCH_VERSION: u8 = 1;
pub const VALUED_RESULT_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValuedEncodingError {
    InvalidVersion(u8),
    InvalidUtf8,
    LengthOverflow,
    Truncated,
    TrailingBytes,
    Build(ValuedBuildError),
}

impl Display for ValuedEncodingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion(version) => {
                write!(f, "unsupported valued encoding version {version}")
            }
            Self::InvalidUtf8 => write!(f, "valued encoding contains invalid UTF-8"),
            Self::LengthOverflow => write!(f, "valued encoding length exceeds u32"),
            Self::Truncated => write!(f, "truncated valued encoding"),
            Self::TrailingBytes => write!(f, "trailing bytes in valued encoding"),
            Self::Build(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ValuedEncodingError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValuedBuildError {
    InvalidRule(String),
    InvalidRegex { rule: String, error: String },
}

impl Display for ValuedBuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRule(rule) => write!(f, "invalid valued domain rule: {rule}"),
            Self::InvalidRegex { rule, error } => {
                write!(f, "invalid valued domain regexp {rule:?}: {error}")
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

#[derive(Clone)]
struct AggregatedRule {
    order: usize,
    result: AccumulatedResult,
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
    pub fn build(rules: &[ValuedRule]) -> Result<Self, ValuedBuildError> {
        let mut aggregated: HashMap<String, AggregatedRule> = HashMap::new();
        for (order, rule) in rules.iter().enumerate() {
            parse_rule(&rule.rule)?;
            let entry = aggregated
                .entry(rule.rule.clone())
                .or_insert_with(|| AggregatedRule {
                    order,
                    result: AccumulatedResult::default(),
                });
            entry.result.merge_rule(rule);
        }

        let mut ordered_keys: Vec<String> = aggregated.keys().cloned().collect();
        ordered_keys.sort_by_key(|key| aggregated[key].order);

        let mut effective = HashMap::with_capacity(aggregated.len());
        for key in &ordered_keys {
            effective_result(key, &aggregated, &mut effective)?;
        }

        let mut pool: HashMap<ValuedMatchResult, Arc<ValuedMatchResult>> = HashMap::new();
        let mut full = HashMap::new();
        let mut domain = DomainSuffixMatcher::new();
        let mut overlaps = Vec::new();

        for key in &ordered_keys {
            let (kind, pattern) = parse_rule(key)?;
            let result = intern_result(&mut pool, effective[key].clone().into_result());
            match kind {
                RuleKind::Full => {
                    full.insert(normalize(pattern), result);
                }
                RuleKind::Domain => domain.add(pattern, result),
                RuleKind::Keyword => overlaps.push(OverlapRule::Keyword {
                    pattern: normalize(pattern),
                    result,
                }),
                RuleKind::Regexp => {
                    let matcher =
                        Regex::new(pattern).map_err(|error| ValuedBuildError::InvalidRegex {
                            rule: key.clone(),
                            error: error.to_string(),
                        })?;
                    overlaps.push(OverlapRule::Regexp { matcher, result });
                }
            }
        }

        Ok(Self {
            full,
            domain,
            overlaps,
            rule_count: ordered_keys.len(),
            pooled_result_count: pool.len(),
        })
    }

    /// Builds an immutable matcher from the versioned ABI rule batch.
    pub fn build_encoded(bytes: &[u8]) -> Result<Self, ValuedEncodingError> {
        let rules = decode_valued_rule_batch(bytes)?;
        Self::build(&rules).map_err(ValuedEncodingError::Build)
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
        let normalized = normalize(name);
        let mut merged = self
            .full
            .get(&normalized)
            .map(|result| result.as_ref().clone());

        if let Some(result) = self.domain.r#match(&normalized) {
            merged = Some(merge_results(merged, result));
        }

        for overlap in &self.overlaps {
            let matches = match overlap {
                OverlapRule::Keyword { pattern, .. } => normalized.contains(pattern),
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

fn effective_result(
    key: &str,
    aggregated: &HashMap<String, AggregatedRule>,
    cache: &mut HashMap<String, AccumulatedResult>,
) -> Result<AccumulatedResult, ValuedBuildError> {
    if let Some(result) = cache.get(key) {
        return Ok(result.clone());
    }

    let (kind, pattern) = parse_rule(key)?;
    let mut result = aggregated
        .get(key)
        .map_or_else(AccumulatedResult::default, |entry| entry.result.clone());

    let mut ancestors = Vec::new();
    if kind == RuleKind::Full {
        ancestors.push(format!("domain:{pattern}"));
    }

    let mut suffix = pattern;
    while let Some(dot) = suffix.find('.') {
        suffix = &suffix[dot + 1..];
        ancestors.push(format!("domain:{suffix}"));
    }

    for ancestor in ancestors {
        if aggregated.contains_key(&ancestor) {
            let inherited = effective_result(&ancestor, aggregated, cache)?;
            result.merge_result(&inherited);
        }
    }
    cache.insert(key.to_owned(), result.clone());
    Ok(result)
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

/// Encodes one complete valued-rule batch for the runtime ABI.
pub fn encode_valued_rule_batch(rules: &[ValuedRule]) -> Result<Vec<u8>, ValuedEncodingError> {
    let count = u32::try_from(rules.len()).map_err(|_| ValuedEncodingError::LengthOverflow)?;
    let mut out = Vec::new();
    out.push(VALUED_RULE_BATCH_VERSION);
    out.extend_from_slice(&count.to_le_bytes());
    for rule in rules {
        push_string(&mut out, &rule.rule)?;
        out.extend_from_slice(&rule.fast_marks.to_le_bytes());
        let ctx_count =
            u32::try_from(rule.ctx_marks.len()).map_err(|_| ValuedEncodingError::LengthOverflow)?;
        out.extend_from_slice(&ctx_count.to_le_bytes());
        for mark in &rule.ctx_marks {
            out.extend_from_slice(&mark.to_le_bytes());
        }
        push_string(&mut out, &rule.joined_tags)?;
        push_string(&mut out, &rule.joined_sources)?;
    }
    Ok(out)
}

/// Decodes one complete valued-rule batch from the runtime ABI.
pub fn decode_valued_rule_batch(bytes: &[u8]) -> Result<Vec<ValuedRule>, ValuedEncodingError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut cursor = 0;
    let version = read_u8(bytes, &mut cursor)?;
    if version != VALUED_RULE_BATCH_VERSION {
        return Err(ValuedEncodingError::InvalidVersion(version));
    }
    let count = read_u32(bytes, &mut cursor)? as usize;
    let remaining = bytes.len().saturating_sub(cursor);
    // A rule has five fixed-width fields even when every variable-length
    // field is empty. Reject an impossible count before allocating based on
    // untrusted input.
    if count > remaining / 24 {
        return Err(ValuedEncodingError::Truncated);
    }
    let mut rules = Vec::with_capacity(count);
    for _ in 0..count {
        let rule = read_string(bytes, &mut cursor)?;
        let fast_marks = read_u64(bytes, &mut cursor)?;
        let ctx_count = read_u32(bytes, &mut cursor)? as usize;
        let remaining = bytes.len().saturating_sub(cursor);
        if ctx_count > remaining / 4 {
            return Err(ValuedEncodingError::Truncated);
        }
        let mut ctx_marks = Vec::with_capacity(ctx_count);
        for _ in 0..ctx_count {
            ctx_marks.push(read_u32(bytes, &mut cursor)?);
        }
        let joined_tags = read_string(bytes, &mut cursor)?;
        let joined_sources = read_string(bytes, &mut cursor)?;
        rules.push(ValuedRule {
            rule,
            fast_marks,
            ctx_marks,
            joined_tags,
            joined_sources,
        });
    }
    if cursor != bytes.len() {
        return Err(ValuedEncodingError::TrailingBytes);
    }
    Ok(rules)
}

/// Encodes one matched result into the caller-owned ABI payload format.
pub fn encode_valued_match_result(
    result: &ValuedMatchResult,
) -> Result<Vec<u8>, ValuedEncodingError> {
    let ctx_count =
        u32::try_from(result.ctx_marks.len()).map_err(|_| ValuedEncodingError::LengthOverflow)?;
    let mut out = Vec::new();
    out.push(VALUED_RESULT_VERSION);
    out.extend_from_slice(&result.fast_marks.to_le_bytes());
    out.extend_from_slice(&ctx_count.to_le_bytes());
    for mark in &result.ctx_marks {
        out.extend_from_slice(&mark.to_le_bytes());
    }
    push_string(&mut out, &result.joined_tags)?;
    push_string(&mut out, &result.joined_sources)?;
    Ok(out)
}

/// Decodes one caller-owned ABI result payload for parity and misuse tests.
pub fn decode_valued_match_result(bytes: &[u8]) -> Result<ValuedMatchResult, ValuedEncodingError> {
    let mut cursor = 0;
    let version = read_u8(bytes, &mut cursor)?;
    if version != VALUED_RESULT_VERSION {
        return Err(ValuedEncodingError::InvalidVersion(version));
    }
    let fast_marks = read_u64(bytes, &mut cursor)?;
    let ctx_count = read_u32(bytes, &mut cursor)? as usize;
    let remaining = bytes.len().saturating_sub(cursor);
    if ctx_count > remaining / 4 {
        return Err(ValuedEncodingError::Truncated);
    }
    let mut ctx_marks = Vec::with_capacity(ctx_count);
    for _ in 0..ctx_count {
        ctx_marks.push(read_u32(bytes, &mut cursor)?);
    }
    let joined_tags = read_string(bytes, &mut cursor)?;
    let joined_sources = read_string(bytes, &mut cursor)?;
    if cursor != bytes.len() {
        return Err(ValuedEncodingError::TrailingBytes);
    }
    Ok(ValuedMatchResult {
        fast_marks,
        ctx_marks,
        joined_tags,
        joined_sources,
    })
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), ValuedEncodingError> {
    let len = u32::try_from(value.len()).map_err(|_| ValuedEncodingError::LengthOverflow)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, ValuedEncodingError> {
    let value = *bytes.get(*cursor).ok_or(ValuedEncodingError::Truncated)?;
    *cursor += 1;
    Ok(value)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, ValuedEncodingError> {
    let end = cursor
        .checked_add(4)
        .ok_or(ValuedEncodingError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(ValuedEncodingError::Truncated)?;
    *cursor = end;
    let mut encoded = [0_u8; 4];
    encoded.copy_from_slice(value);
    Ok(u32::from_le_bytes(encoded))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, ValuedEncodingError> {
    let end = cursor
        .checked_add(8)
        .ok_or(ValuedEncodingError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(ValuedEncodingError::Truncated)?;
    *cursor = end;
    let mut encoded = [0_u8; 8];
    encoded.copy_from_slice(value);
    Ok(u64::from_le_bytes(encoded))
}

fn read_string(bytes: &[u8], cursor: &mut usize) -> Result<String, ValuedEncodingError> {
    let len = read_u32(bytes, cursor)? as usize;
    let end = cursor
        .checked_add(len)
        .ok_or(ValuedEncodingError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(ValuedEncodingError::Truncated)?;
    *cursor = end;
    String::from_utf8(value.to_vec()).map_err(|_| ValuedEncodingError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::{
        ValuedBuildError, ValuedDomainMatcher, ValuedEncodingError, ValuedMatchResult, ValuedRule,
        decode_valued_match_result, decode_valued_rule_batch, encode_valued_match_result,
        encode_valued_rule_batch,
    };

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
    fn versioned_encodings_round_trip_and_reject_malformed_lengths() {
        let rules = vec![
            ValuedRule::new("domain:example.com", 3, [20, 10], "base|tag", "source"),
            ValuedRule::new("keyword:child", 4, [30], "keyword", "keyword-source"),
        ];
        let encoded_rules = encode_valued_rule_batch(&rules).expect("encode valued rules");
        assert_eq!(
            decode_valued_rule_batch(&encoded_rules).expect("decode valued rules"),
            rules
        );

        let mut truncated = encoded_rules.clone();
        truncated.pop();
        assert_eq!(
            decode_valued_rule_batch(&truncated),
            Err(ValuedEncodingError::Truncated)
        );
        let mut trailing = encoded_rules.clone();
        trailing.push(0);
        assert_eq!(
            decode_valued_rule_batch(&trailing),
            Err(ValuedEncodingError::TrailingBytes)
        );
        assert_eq!(
            decode_valued_rule_batch(&[1, 0xff, 0xff, 0xff, 0xff]),
            Err(ValuedEncodingError::Truncated)
        );
        let mut invalid_rule_utf8 = encoded_rules.clone();
        invalid_rule_utf8[9] = 0xff;
        assert_eq!(
            decode_valued_rule_batch(&invalid_rule_utf8),
            Err(ValuedEncodingError::InvalidUtf8)
        );

        let result = ValuedMatchResult {
            fast_marks: 7,
            ctx_marks: vec![10, 20, 30],
            joined_tags: "tag|other".to_owned(),
            joined_sources: "source".to_owned(),
        };
        let encoded_result = encode_valued_match_result(&result).expect("encode valued result");
        assert_eq!(
            decode_valued_match_result(&encoded_result).expect("decode valued result"),
            result
        );
        assert_eq!(
            decode_valued_match_result(&[1]),
            Err(ValuedEncodingError::Truncated)
        );
        let mut invalid_result_utf8 = encoded_result.clone();
        invalid_result_utf8[29] = 0xff;
        assert_eq!(
            decode_valued_match_result(&invalid_result_utf8),
            Err(ValuedEncodingError::InvalidUtf8)
        );
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
