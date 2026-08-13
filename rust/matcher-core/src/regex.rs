/// Compiled regex matcher (Go's `RegexMatcher`).
///
/// Note: Go and Rust regex dialects are not automatically identical. Simple
/// patterns (common in DNS rule sets) are compatible, but `RegexMatcher` must
/// not silently reinterpret a rule. Unsupported patterns should fall through
/// to the Go path (enforced at the FFI boundary, not in this crate).
pub struct RegexMatcher {
    patterns: Vec<(regex::Regex, u64)>, // (compiled regex, monotonically increasing id)
    next_id: u64,
}

impl RegexMatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            next_id: 0,
        }
    }

    /// Try to add a regex pattern.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the pattern cannot compile (same behaviour as
    /// Go's `regexp.Compile`).
    #[allow(clippy::implicit_clone)]
    pub fn add(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let re = regex::Regex::new(pattern)?;
        self.patterns.push((re, self.next_id));
        self.next_id += 1;
        Ok(())
    }

    /// Check whether the domain matches any pattern.
    /// The domain should already be lower-case and non-fqdn (same as Go).
    pub fn is_match(&self, domain: &str) -> bool {
        self.patterns.iter().any(|(re, _)| re.is_match(domain))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

impl Default for RegexMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_matching() {
        let mut m = RegexMatcher::new();
        assert!(m.add("^ads\\..*\\.example$").is_ok());
        assert!(m.add("^.*\\.ads\\.example$").is_ok());

        assert!(m.is_match("ads.foo.example"));
        assert!(m.is_match("x.ads.example"));
        assert!(!m.is_match("normal.example"));
        assert!(!m.is_match("ads.example"));
    }

    #[test]
    fn invalid_pattern_rejected() {
        let mut m = RegexMatcher::new();
        assert!(m.add("[").is_err());
    }

    #[test]
    fn empty_matcher_never_matches() {
        let m = RegexMatcher::new();
        assert!(!m.is_match("anything"));
    }
}
