use crate::normalize::normalize;
use std::collections::HashMap;

/// Reversed-label trie for domain suffix matching (Go's `SubDomainMatcher`).
///
/// Nodes live in one flat arena (node 0 is the root) so that a rule with a
/// very large number of labels neither recurses on build, drop nor count.
pub struct DomainSuffixMatcher<V> {
    nodes: Vec<Node<V>>,
}

struct Node<V> {
    value: Option<V>,
    children: HashMap<String, usize>,
}

impl<V> Node<V> {
    fn empty() -> Self {
        Self {
            value: None,
            children: HashMap::new(),
        }
    }
}

/// Go's `ReverseDomainScanner`: trims one more trailing dot, then yields the
/// labels from the right. Empty labels between or after dots are kept, but a
/// leading empty label (the text starts with `.`) is never yielded.
fn reverse_labels(s: &str) -> Vec<&str> {
    let s = s.strip_suffix('.').unwrap_or(s);
    let mut labels = Vec::new();
    let mut end = s.len();
    while end > 0 {
        let start = s[..end].rfind('.');
        labels.push(&s[start.map_or(0, |dot| dot + 1)..end]);
        end = start.unwrap_or(0);
    }
    labels
}

impl<V> DomainSuffixMatcher<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: vec![Node::empty()],
        }
    }

    /// Add a domain rule (normalised once here, like Go's `Add`).
    pub fn add(&mut self, domain: &str, value: V) {
        self.add_normalized(&normalize(domain), value);
    }

    /// Add a rule that was already normalised by the caller (Go's
    /// `MixMatcher` and the mapper normalise once before the scanner).
    pub fn add_normalized(&mut self, domain: &str, value: V) {
        let mut node = 0;
        for label in reverse_labels(domain) {
            node = match self.nodes[node].children.get(label) {
                Some(&child) => child,
                None => {
                    let child = self.nodes.len();
                    self.nodes.push(Node::empty());
                    self.nodes[node].children.insert(label.to_owned(), child);
                    child
                }
            };
        }
        self.nodes[node].value = Some(value);
    }

    /// Match returns the value for the longest matching suffix.
    pub fn r#match(&self, domain: &str) -> Option<&V> {
        self.match_normalized(&normalize(domain))
    }

    /// Match a name the caller already normalised.
    pub fn match_normalized(&self, domain: &str) -> Option<&V> {
        let mut node = 0;
        let mut last: Option<&V> = self.nodes[0].value.as_ref();
        for label in reverse_labels(domain) {
            match self.nodes[node].children.get(label) {
                Some(&child) => {
                    if self.nodes[child].value.is_some() {
                        last = self.nodes[child].value.as_ref();
                    }
                    node = child;
                }
                None => break,
            }
        }
        last
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.value.is_some())
            .count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<V> Default for DomainSuffixMatcher<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_suffix_matching() {
        let mut m = DomainSuffixMatcher::new();
        m.add("example.com", 1);
        m.add("a.example.com", 2); // more specific

        assert_eq!(m.r#match("example.com"), Some(&1));
        assert_eq!(m.r#match("sub.example.com"), Some(&1));
        assert_eq!(m.r#match("a.example.com"), Some(&2));
        assert_eq!(m.r#match("b.a.example.com"), Some(&2));
        assert_eq!(m.r#match("example.org"), None);
    }

    #[test]
    fn labels_follow_go_scanner() {
        assert_eq!(reverse_labels("example.com"), ["com", "example"]);
        assert_eq!(reverse_labels(".example.com"), ["com", "example"]);
        assert_eq!(reverse_labels("example.com."), ["com", "example"]);
        assert_eq!(reverse_labels("example.com.."), ["", "com", "example"]);
        assert_eq!(reverse_labels("..example.com"), ["com", "example", ""]);
        assert_eq!(reverse_labels("a..b"), ["b", "", "a"]);
        assert!(reverse_labels("").is_empty());
        assert!(reverse_labels(".").is_empty());

        let mut m = DomainSuffixMatcher::new();
        m.add(".example.com", 1);
        m.add("other.org..", 2);
        assert_eq!(m.r#match("example.com."), Some(&1));
        assert_eq!(m.r#match("a.example.com."), Some(&1));
        assert_eq!(m.r#match("other.org."), Some(&2));
        let mut m = DomainSuffixMatcher::new();
        m.add("..", 3);
        assert_eq!(m.r#match("anything."), Some(&3));
    }

    #[test]
    fn a_rule_with_half_a_million_labels_neither_recurses_nor_overflows() {
        let mut m = DomainSuffixMatcher::new();
        let deep = "a.".repeat(524_288);
        m.add(&deep, 1);
        assert_eq!(m.len(), 1);
        assert_eq!(m.r#match(&format!("x.{deep}")), Some(&1));
        drop(m);
    }

    #[test]
    fn root_match_any() {
        let mut m = DomainSuffixMatcher::new();
        m.add(".", 42);

        assert_eq!(m.r#match("anything.example"), Some(&42));
        assert_eq!(m.r#match("a"), Some(&42));
        assert_eq!(m.r#match(""), Some(&42));
    }

    #[test]
    fn case_insensitive() {
        let mut m = DomainSuffixMatcher::new();
        m.add("ExAmPlE.CoM", 1);
        assert_eq!(m.r#match("EXAMPLE.COM"), Some(&1));
        assert_eq!(m.r#match("example.com."), Some(&1));
    }

    #[test]
    fn empty_never_matched() {
        let m: DomainSuffixMatcher<i32> = DomainSuffixMatcher::new();
        assert_eq!(m.r#match("anything"), None);
    }
}
