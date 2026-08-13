use crate::normalize;
use std::collections::HashMap;

/// Exact domain matcher (Go's `FullMatcher`).
pub struct FullMatcher<V> {
    map: HashMap<String, V>,
}

impl<V> FullMatcher<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn add(&mut self, domain: &str, value: V) {
        self.map.insert(normalize(domain), value);
    }

    pub fn r#match(&self, domain: &str) -> Option<&V> {
        self.map.get(&normalize(domain))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl<V> Default for FullMatcher<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let mut m = FullMatcher::new();
        m.add("exact.example", 1);
        assert_eq!(m.r#match("exact.example"), Some(&1));
        assert_eq!(m.r#match("exact.example."), Some(&1));
        assert_eq!(m.r#match("sub.exact.example"), None);
        assert_eq!(m.r#match("unrelated"), None);
    }

    #[test]
    fn case_insensitive() {
        let mut m = FullMatcher::new();
        m.add("UPPER.example", 1);
        assert_eq!(m.r#match("upper.example"), Some(&1));
        assert_eq!(m.r#match("UPPER.EXAMPLE."), Some(&1));
    }

    #[test]
    fn replace_value() {
        let mut m = FullMatcher::new();
        m.add("example", 1);
        m.add("example", 2);
        assert_eq!(m.r#match("example"), Some(&2));
        assert_eq!(m.len(), 1);
    }
}
