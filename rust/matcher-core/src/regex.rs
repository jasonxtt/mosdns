use std::fmt::{Display, Formatter};

/// A regexp failed the Rust/Go compatibility gate or Rust compilation.
#[derive(Debug)]
pub enum RegexBuildError {
    /// The pattern uses syntax outside the frozen Go-compatible grammar.
    Unsupported { offset: usize, reason: &'static str },
    /// The pattern is in the safe grammar but is not valid Rust regexp syntax.
    Compile(regex::Error),
}

impl Display for RegexBuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { offset, reason } => {
                write!(f, "unsupported Go regexp syntax at byte {offset}: {reason}")
            }
            Self::Compile(error) => write!(f, "invalid regexp: {error}"),
        }
    }
}

impl std::error::Error for RegexBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Unsupported { .. } => None,
        }
    }
}

/// Validate and compile one regexp under the frozen Go-compatible grammar.
pub(crate) fn compile_go_compatible(pattern: &str) -> Result<regex::Regex, RegexBuildError> {
    validate_go_compatible(pattern)?;
    regex::Regex::new(pattern).map_err(RegexBuildError::Compile)
}

fn validate_go_compatible(pattern: &str) -> Result<(), RegexBuildError> {
    let bytes = pattern.as_bytes();
    let mut in_class = false;
    let mut class_first = false;
    let mut escaped = false;

    for (offset, &byte) in bytes.iter().enumerate() {
        if !byte.is_ascii() {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "non-ASCII pattern",
            });
        }
        if byte < 0x20 || byte == 0x7f {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "control character",
            });
        }

        if escaped {
            let allowed = if in_class {
                is_allowed_class_escape(byte)
            } else {
                is_allowed_escape(byte)
            };
            if !allowed {
                return Err(RegexBuildError::Unsupported {
                    offset,
                    reason: "unlisted backslash escape",
                });
            }
            escaped = false;
            if in_class {
                class_first = false;
            }
            continue;
        }

        if byte == b'\\' {
            escaped = true;
            continue;
        }

        if !in_class && byte == b'(' && bytes.get(offset + 1) == Some(&b'?') {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "extended group",
            });
        }
        if in_class && byte == b'[' {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "nested or POSIX character class",
            });
        }
        if in_class && matches!(byte, b'&' | b'-' | b'~') && bytes.get(offset + 1) == Some(&byte) {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "Rust character-class set operator",
            });
        }
        if byte == b'[' {
            in_class = true;
            class_first = true;
            continue;
        }
        if byte == b']' && in_class {
            if class_first {
                return Err(RegexBuildError::Unsupported {
                    offset,
                    reason: "class-leading ]",
                });
            }
            in_class = false;
            continue;
        }
        if in_class && class_first && byte == b'^' {
            continue;
        }
        if in_class {
            class_first = false;
        }
        if !in_class
            && byte == b'?'
            && offset > 0
            && matches!(bytes[offset - 1], b'*' | b'+' | b'?' | b'}')
        {
            return Err(RegexBuildError::Unsupported {
                offset,
                reason: "lazy or extended quantifier",
            });
        }
    }

    if escaped {
        return Err(RegexBuildError::Unsupported {
            offset: bytes.len().saturating_sub(1),
            reason: "trailing backslash",
        });
    }
    Ok(())
}

fn is_allowed_escape(byte: u8) -> bool {
    matches!(
        byte,
        b'\\'
            | b'.'
            | b'*'
            | b'+'
            | b'?'
            | b'|'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'^'
            | b'$'
    )
}

fn is_allowed_class_escape(byte: u8) -> bool {
    is_allowed_escape(byte) || byte == b'-'
}

/// Compiled regex matcher (Go's `RegexMatcher`).
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
    /// Returns `Err` when the pattern is outside the compatibility grammar or
    /// cannot compile.
    #[allow(clippy::implicit_clone)]
    pub fn add(&mut self, pattern: &str) -> Result<(), RegexBuildError> {
        let re = compile_go_compatible(pattern)?;
        self.patterns.push((re, self.next_id));
        self.next_id += 1;
        Ok(())
    }

    /// Check whether the domain matches any pattern.
    /// The domain should already be lower-case and non-fqdn (same as Go).
    pub fn is_match(&self, domain: &str) -> bool {
        if !domain.is_ascii() {
            return false;
        }
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
    fn go_compatibility_grammar_is_explicit_and_stateful() {
        for pattern in [
            r"\w",
            r"[\w]",
            r"\d",
            r"\s",
            r"(?:foo)",
            r"(?i:foo)",
            r"\p{Han}",
            r"[[:alpha:]]",
            r"[ab&&b]",
            r"[ab~~b]",
            r"[a-z--m-z]",
            r"[]&&]",
            r"[^]&&]",
            r"[]--]",
            r"[]~~]",
            "例",
        ] {
            let mut matcher = RegexMatcher::new();
            assert!(
                matcher.add(pattern).is_err(),
                "pattern {pattern:?} must not enter the Rust matcher"
            );
        }

        for pattern in [
            r"\\w",
            r"\.",
            r"[.]",
            r"[a&b]",
            r"[a~b]",
            r"[a-z]",
            r"[&~]",
            r"[a-zA-Z0-9]",
            r"[^a-z]",
            r"[a*?]",
            r"^ads\..*\.example$",
        ] {
            let mut matcher = RegexMatcher::new();
            assert!(
                matcher.add(pattern).is_ok(),
                "pattern {pattern:?} should be in the safe grammar"
            );
        }

        let raw = regex::Regex::new(r"^[ab&&b]+$").expect("Rust set syntax compiles");
        assert!(!raw.is_match("a"), "Rust set intersection should exclude a");
        regex::Regex::new(r"[]&&]").expect("Rust class set syntax compiles");
    }

    #[test]
    fn empty_matcher_never_matches() {
        let m = RegexMatcher::new();
        assert!(!m.is_match("anything"));
    }
}
