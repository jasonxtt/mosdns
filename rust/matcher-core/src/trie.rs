use crate::normalize::normalize;
use std::collections::HashMap;

/// Reversed-label trie for domain suffix matching (Go's `SubDomainMatcher`).
pub struct DomainSuffixMatcher<V> {
    root: Node<V>,
}

struct Node<V> {
    value: Option<V>,
    children: HashMap<String, Node<V>>,
}

impl<V> DomainSuffixMatcher<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Node {
                value: None,
                children: HashMap::new(),
            },
        }
    }

    /// Add a domain rule.
    pub fn add(&mut self, domain: &str, value: V) {
        let domain = normalize(domain);
        if domain.is_empty() {
            // Normalised "." → root value.
            self.root.value = Some(value);
            return;
        }
        let labels: Vec<&str> = domain.rsplit('.').collect();
        let mut node = &mut self.root;
        for label in &labels {
            node = node
                .children
                .entry(label.to_string())
                .or_insert_with(|| Node {
                    value: None,
                    children: HashMap::new(),
                });
        }
        node.value = Some(value);
    }

    /// Match returns the value for the longest matching suffix.
    pub fn r#match(&self, domain: &str) -> Option<&V> {
        let d = normalize(domain);
        if d.is_empty() {
            return self.root.value.as_ref();
        }
        let labels: Vec<&str> = d.rsplit('.').collect();
        let mut node = &self.root;
        let mut last: Option<&V> = self.root.value.as_ref();
        for label in &labels {
            match node.children.get(*label) {
                Some(child) => {
                    if child.value.is_some() {
                        last = child.value.as_ref();
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
        self.count(&self.root)
    }

    #[allow(clippy::self_only_used_in_recursion)]
    fn count(&self, node: &Node<V>) -> usize {
        let mut n = usize::from(node.value.is_some());
        for child in node.children.values() {
            n += self.count(child);
        }
        n
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
