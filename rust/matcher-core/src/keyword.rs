use crate::normalize;

/// Keyword substring matcher (Go's `KeywordMatcher`).
///
/// Iterates over all keywords and returns the value for the first match.
/// The domain is lowered once, not per-keyword, to match Go's alloc-optimised
/// path (Go uses `strings.Contains` with pre-lowered qname).
pub struct KeywordMatcher<V> {
    keywords: Vec<(String, V)>,
}

impl<V> KeywordMatcher<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            keywords: Vec::new(),
        }
    }

    pub fn add(&mut self, keyword: &str, value: V) {
        self.keywords.push((normalize(keyword), value));
    }

    pub fn r#match(&self, domain: &str) -> Option<&V> {
        let lowered = normalize(domain);
        for (kw, v) in &self.keywords {
            if lowered.contains(kw) {
                return Some(v);
            }
        }
        None
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.keywords.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }
}

impl<V> Default for KeywordMatcher<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_match() {
        let mut m = KeywordMatcher::new();
        m.add("example", 1);
        assert_eq!(m.r#match("example.com"), Some(&1));
        assert_eq!(m.r#match("sub.example.org"), Some(&1));
        assert_eq!(m.r#match("notexample.com"), Some(&1));
        assert_eq!(m.r#match("examp.com"), None);
    }

    #[test]
    fn empty_keyword_matches_everything() {
        let mut m = KeywordMatcher::new();
        m.add("", 1);
        assert_eq!(m.r#match("anything"), Some(&1));
    }

    #[test]
    fn case_insensitive() {
        let mut m = KeywordMatcher::new();
        m.add("ExAmPlE", 1);
        assert_eq!(m.r#match("EXAMPLE.COM"), Some(&1));
    }

    #[test]
    fn replace_value() {
        let mut m = KeywordMatcher::new();
        m.add("kw", 1);
        m.add("kw", 2);
        // KeywordMatcher adds duplicates; the first-added match wins.
        assert_eq!(m.r#match("kw"), Some(&1)); // first match
    }
}
