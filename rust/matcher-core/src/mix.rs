use std::collections::HashMap;

use crate::normalize::{NonAsciiRule, normalize};
use crate::regex::compile_go_compatible;
use crate::trie::DomainSuffixMatcher;

const MATCHER_FULL: &str = "full";
const MATCHER_DOMAIN: &str = "domain";
const MATCHER_REGEXP: &str = "regexp";
const MATCHER_KEYWORD: &str = "keyword";

/// Error returned when no default type is set for a bare rule.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct NoDefaultMatcher;

impl std::fmt::Display for NoDefaultMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "default matcher is not set")
    }
}

impl std::error::Error for NoDefaultMatcher {}

/// Error returned when an unsupported type prefix is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedType(pub String);

impl std::fmt::Display for UnsupportedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported match type [{}]", self.0)
    }
}

impl std::error::Error for UnsupportedType {}

/// Combined domain matcher matching Go's `MixMatcher`.
///
/// Precedence: full → domain → regex → keyword.
///
/// Type is for a generic value `V` (use `()` for boolean-only matching).
pub struct MixMatcher<V> {
    default_type: Option<&'static str>,
    full: HashMap<String, V>,
    domain: DomainSuffixMatcher<V>,
    regex: Vec<(regex::Regex, V)>,
    keyword: Vec<(String, V)>,
}

impl<V> MixMatcher<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            default_type: None,
            full: HashMap::new(),
            domain: DomainSuffixMatcher::new(),
            regex: Vec::new(),
            keyword: Vec::new(),
        }
    }

    /// Set the default matcher type for bare rules without a `type:` prefix.
    /// Pass `"full"` or `"domain"`.
    pub fn set_default(&mut self, type_name: &'static str) {
        self.default_type = Some(type_name);
    }

    /// Add a rule string.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern is invalid (bad regexp, unsupported
    /// type prefix, or missing default type for a bare rule).
    pub fn add(&mut self, rule: &str, value: V) -> Result<(), Box<dyn std::error::Error>> {
        let (type_name, pattern) = match split_rule(rule, self.default_type) {
            Ok(v) => (v.0.to_string(), v.1.to_string()),
            Err(e) => return Err(e.into()),
        };
        let pattern = if type_name.as_str() != MATCHER_REGEXP {
            // full/domain/keyword patterns are normalised.
            if !pattern.is_ascii() {
                return Err(Box::new(NonAsciiRule));
            }
            normalize(&pattern)
        } else {
            if !pattern.is_ascii() {
                return Err(Box::new(NonAsciiRule));
            }
            pattern
        };
        if type_name == MATCHER_FULL {
            self.full.insert(pattern, value);
            Ok(())
        } else if type_name == MATCHER_DOMAIN {
            self.domain.add(&pattern, value);
            Ok(())
        } else if type_name == MATCHER_KEYWORD {
            self.keyword.push((pattern, value));
            Ok(())
        } else if type_name == MATCHER_REGEXP {
            let re = compile_go_compatible(&pattern)?;
            self.regex.push((re, value));
            Ok(())
        } else {
            Err(Box::new(UnsupportedType(type_name.to_string())))
        }
    }

    /// Match a domain against all sub-matchers in precedence order.
    /// Returns `Some(&V)` from the first sub-matcher that matches.
    #[must_use]
    pub fn r#match(&self, domain: &str) -> Option<&V> {
        if !domain.is_ascii() {
            return None;
        }
        let d = normalize(domain);
        // 1. Full
        if let Some(v) = self.full.get(&d) {
            return Some(v);
        }
        // 2. Domain suffix
        if let Some(v) = self.domain.r#match(&d) {
            return Some(v);
        }
        // 3. Regex
        for (re, v) in &self.regex {
            if re.is_match(&d) {
                return Some(v);
            }
        }
        // 4. Keyword
        for (kw, v) in &self.keyword {
            if d.contains(kw) {
                return Some(v);
            }
        }
        None
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.full.len() + self.domain.len() + self.regex.len() + self.keyword.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<V> Default for MixMatcher<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Split `rule` at the first `:`. Without a colon, `default_type` is used if
/// set; otherwise `NoDefaultMatcher` is returned.
fn split_rule<'a>(
    rule: &'a str,
    default_type: Option<&'static str>,
) -> Result<(&'a str, &'a str), String> {
    if let Some(pos) = rule.find(':') {
        let type_name = &rule[..pos];
        let pattern = &rule[pos + 1..];
        match type_name {
            MATCHER_FULL | MATCHER_DOMAIN | MATCHER_REGEXP | MATCHER_KEYWORD => {
                return Ok((type_name, pattern));
            }
            _ => return Err(format!("unsupported match type [{type_name}]")),
        }
    }
    match default_type {
        Some(t) => Ok((t, rule)),
        None => Err("default matcher is not set".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> MixMatcher<i32> {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix
    }

    #[test]
    fn precedence_full_over_domain() {
        let mut mix = m();
        mix.add("full:exact.example", 10).unwrap();
        mix.add("domain:example", 20).unwrap();
        assert_eq!(mix.r#match("exact.example"), Some(&10));
        assert_eq!(mix.r#match("sub.exact.example"), Some(&20));
        assert_eq!(mix.r#match("other.example"), Some(&20));
        assert_eq!(mix.r#match("no-match.com"), None);
    }

    #[test]
    fn domain_wins_over_regex_and_keyword() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("regexp:^exact\\.example$", 30).unwrap();
        mix.add("keyword:example", 40).unwrap();
        mix.add("domain:example", 50).unwrap();

        assert_eq!(mix.r#match("exact.example"), Some(&50));
        assert_eq!(mix.r#match("myexample.net"), Some(&40)); // keyword
        assert_eq!(mix.r#match("other"), None);
    }

    #[test]
    fn regex_before_keyword() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("regexp:^re\\.", 10).unwrap();
        mix.add("keyword:re", 20).unwrap();

        assert_eq!(mix.r#match("re.test"), Some(&10));
        assert_eq!(mix.r#match("xre.y"), Some(&20));
        assert_eq!(mix.r#match("other"), None);
    }

    #[test]
    fn default_domain_adds_suffix() {
        let mut mix = m();
        mix.add("example.com", 1).unwrap();
        assert_eq!(mix.r#match("a.example.com"), Some(&1));
        assert_eq!(mix.r#match("example.com.evil"), None);
    }

    #[test]
    fn default_full_adds_exact() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_FULL);
        mix.add("example.com", 1).unwrap();
        assert_eq!(mix.r#match("example.com"), Some(&1));
        assert_eq!(mix.r#match("a.example.com"), None);
    }

    #[test]
    fn keyword_substring_not_boundary() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("keyword:example", 1).unwrap();
        assert!(mix.r#match("notexample.com").is_some());
        assert!(mix.r#match("examp.com").is_none());
    }

    #[test]
    fn root_rule_matches_everything() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("domain:.", 99).unwrap();
        assert_eq!(mix.r#match("anything.example"), Some(&99));
        assert_eq!(mix.r#match("a"), Some(&99));
        assert_eq!(mix.r#match(""), Some(&99));
    }

    #[test]
    fn duplicate_full_replaces_value() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("full:exact.example", 1).unwrap();
        mix.add("full:exact.example", 2).unwrap();
        assert_eq!(mix.r#match("exact.example"), Some(&2));
    }

    #[test]
    fn bad_regexp_returns_error() {
        let mut mix = m();
        assert!(mix.add("regexp:[", 1).is_err());
    }

    #[test]
    fn non_ascii_domain_rules_are_rejected_before_snapshot_build() {
        for rule in [
            "full:例.example",
            "domain:例.example",
            "keyword:例",
            "regexp:^例$",
        ] {
            let mut mix = m();
            assert!(mix.add(rule, 1).is_err(), "rule {rule:?} must be rejected");
        }
    }

    #[test]
    fn unsupported_type_returns_error() {
        let mut mix = m();
        assert!(mix.add("foo:bar", 1).is_err());
    }

    #[test]
    fn missing_default_returns_error_for_bare_rule() {
        let mut mix: MixMatcher<i32> = MixMatcher::new();
        assert!(mix.add("bare", 1).is_err());
    }

    #[test]
    fn typed_rule_works_without_default() {
        let mut mix: MixMatcher<i32> = MixMatcher::new();
        mix.add("full:exact.example", 1).unwrap();
        assert_eq!(mix.r#match("exact.example"), Some(&1));
    }

    #[test]
    fn keyword_empty_matches_everything() {
        let mut mix = m();
        mix.add("keyword:", 1).unwrap();
        assert!(mix.r#match("anything").is_some());
    }

    #[test]
    fn longer_domain_shadows_shorter() {
        let mut mix = MixMatcher::new();
        mix.set_default(MATCHER_DOMAIN);
        mix.add("domain:example", 10).unwrap();
        mix.add("domain:a.example", 20).unwrap();
        assert_eq!(mix.r#match("a.example"), Some(&20));
        assert_eq!(mix.r#match("b.a.example"), Some(&20));
        assert_eq!(mix.r#match("b.example"), Some(&10));
    }
}
