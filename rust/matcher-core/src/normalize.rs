use std::fmt::{Display, Formatter};

/// Error returned when a matcher rule is outside the ASCII-only Rust boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonAsciiRule;

impl Display for NonAsciiRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("non-ASCII matcher rules are Go-only")
    }
}

impl std::error::Error for NonAsciiRule {}

/// Normalise a domain string to match Go's `NormalizeDomain`:
/// lowercase + strip a single trailing dot.
///
/// ```
/// # use mosdns_matcher_core::normalize;
/// assert_eq!(normalize("GOOGLE.com."), "google.com");
/// assert_eq!(normalize("google.com"), "google.com");
/// assert_eq!(normalize(""), "");
/// assert_eq!(normalize("."), "");
/// assert_eq!(normalize(".."), ".");  // one trailing dot only
/// assert_eq!(normalize("EXAMPLE.."), "example.");
/// ```
#[must_use]
pub fn normalize(s: &str) -> String {
    let trimmed = s.strip_suffix('.').unwrap_or(s);
    trimmed.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_go_golden() {
        // Copied from pkg/matcher/domain/golden_test.go TestGoldenNormalizeDomain
        let cases = [
            ("google.com", "google.com"),
            ("google.com.", "google.com"),
            ("GOOGLE.com.", "google.com"),
            ("Google.COM", "google.com"),
            ("a.b.C.", "a.b.c"),
            ("", ""),
            (".", ""),
            ("..", "."),
            ("EXAMPLE.", "example"),
            ("EXAMPLE..", "example."),
        ];
        for (input, expected) in &cases {
            assert_eq!(&normalize(input), expected, "normalize({input:?})");
        }
    }
}
