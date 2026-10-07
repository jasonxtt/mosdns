use std::fmt::{Display, Formatter};

/// A regexp failed the Rust/Go compatibility gate or Rust compilation.
#[derive(Debug)]
pub enum RegexBuildError {
    /// The pattern may be valid Go but uses syntax this engine does not run
    /// with identical semantics. Callers must not treat it as "invalid".
    Unsupported { offset: usize, reason: &'static str },
    /// The pattern is invalid in Go as well (bad repetition, unclosed group,
    /// nested repetition, repeat count above 1000, ...): a reference host
    /// skips such a rule.
    Invalid { offset: usize, reason: &'static str },
    /// The pattern is inside the safe grammar but Rust rejects it; the
    /// grammar admits only constructs Go and Rust both reject in that case.
    Compile(regex::Error),
}

impl RegexBuildError {
    /// True when the reference engine would reject the pattern too.
    #[must_use]
    pub fn is_invalid_in_go(&self) -> bool {
        match self {
            Self::Unsupported { .. } => false,
            Self::Invalid { .. } => true,
            // Rust rejects some patterns Go accepts (`[\\w-a]`, engine limits),
            // so only the one Rust syntax error with the same meaning in Go is
            // taken as "Go rejects it too"; every other compile failure is
            // refused rather than skipped.
            Self::Compile(regex::Error::Syntax(message)) => {
                // The message embeds the pattern, so only its last line (the
                // diagnostic itself) can be trusted.
                message.lines().last()
                    == Some("error: invalid character class range, the start must be <= the end")
            }
            Self::Compile(_) => false,
        }
    }
}

impl Display for RegexBuildError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { offset, reason } => {
                write!(f, "unsupported Go regexp syntax at byte {offset}: {reason}")
            }
            Self::Invalid { offset, reason } => {
                write!(f, "invalid regexp at byte {offset}: {reason}")
            }
            Self::Compile(error) => write!(f, "invalid regexp: {error}"),
        }
    }
}

impl std::error::Error for RegexBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Unsupported { .. } | Self::Invalid { .. } => None,
        }
    }
}

/// Validate and compile one regexp under the Go-compatible grammar.
///
/// Query names are always ASCII, and on ASCII input Go's (ASCII-only) and
/// Rust's (Unicode-aware) `\d \w \s \b` and case folding agree, so those and
/// the usual group/flag/lazy forms are accepted. What is *not* accepted is
/// rejected as [`RegexBuildError::Unsupported`], never silently as invalid.
pub(crate) fn compile_go_compatible(pattern: &str) -> Result<regex::Regex, RegexBuildError> {
    validate_go_compatible(pattern)?;
    regex::Regex::new(pattern).map_err(RegexBuildError::Compile)
}

/// Checks one regexp against the Go-compatible grammar and Rust's compiler.
///
/// # Errors
///
/// [`RegexBuildError::Unsupported`] for valid-Go syntax this engine does not
/// run (callers should refuse it); [`RegexBuildError::Invalid`] and
/// [`RegexBuildError::Compile`] for patterns Go rejects too (callers skip the
/// rule, as the reference host does).
pub fn check_go_regexp(pattern: &str) -> Result<(), RegexBuildError> {
    compile_go_compatible(pattern).map(|_| ())
}

const POSIX_CLASSES: [&str; 14] = [
    "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct",
    "space", "upper", "word", "xdigit",
];

/// Go's repetition count ceiling.
const MAX_REPEAT: u32 = 1000;

fn unsupported(offset: usize, reason: &'static str) -> RegexBuildError {
    RegexBuildError::Unsupported { offset, reason }
}

fn invalid(offset: usize, reason: &'static str) -> RegexBuildError {
    RegexBuildError::Invalid { offset, reason }
}

/// `\` + punctuation is that literal in both engines, except `<`/`>` which
/// Rust reads as word-boundary escapes.
fn is_escapable_punctuation(byte: u8) -> bool {
    byte.is_ascii() && !byte.is_ascii_alphanumeric() && byte != b'<' && byte != b'>'
}

/// Validates the byte after a backslash at `at` and returns how many bytes
/// the escape consumes after the backslash.
fn escape_len(bytes: &[u8], at: usize, in_class: bool) -> Result<usize, RegexBuildError> {
    let Some(&byte) = bytes.get(at + 1) else {
        return Err(invalid(at, "trailing backslash"));
    };
    match byte {
        b'd' | b'D' | b'w' | b'W' | b's' | b'S' | b't' | b'n' | b'r' | b'f' => Ok(1),
        b'b' | b'B' | b'A' | b'z' if !in_class => Ok(1),
        b'x' => {
            // `\xHH` or `\x{H...}`.
            let hex = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_hexdigit);
            if hex(at + 2) && hex(at + 3) {
                Ok(3)
            } else if bytes.get(at + 2) == Some(&b'{') {
                let mut end = at + 3;
                while hex(end) {
                    end += 1;
                }
                if end > at + 3 && end - (at + 3) <= 8 && bytes.get(end) == Some(&b'}') {
                    Ok(end - at)
                } else {
                    Err(unsupported(at, "malformed \\x escape"))
                }
            } else {
                Err(unsupported(at, "malformed \\x escape"))
            }
        }
        b if is_escapable_punctuation(b) => Ok(1),
        _ => Err(unsupported(at, "unlisted backslash escape")),
    }
}

/// Parses `{n}`, `{n,}` or `{n,m}` starting at `{`; returns its length, the
/// minimum and the maximum (`u32::MAX` when open-ended).
/// `Ok(None)` means the brace is not a repetition (a literal in Go).
fn repetition_len(bytes: &[u8], at: usize) -> Result<Option<(usize, u32, u32)>, RegexBuildError> {
    // A leading zero (`{01}`) is literal text in Go and a count in Rust.
    let mut leading_zero = false;
    let mut digits = |mut i: usize| {
        let start = i;
        let mut value = 0u32;
        while let Some(d) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
            value = value.saturating_mul(10).saturating_add(u32::from(d - b'0'));
            i += 1;
        }
        if i - start > 1 && bytes[start] == b'0' {
            leading_zero = true;
        }
        (i > start).then_some((i, value))
    };
    let Some((mut i, min)) = digits(at + 1) else {
        return Ok(None);
    };
    let mut max = min;
    if bytes.get(i) == Some(&b',') {
        match digits(i + 1) {
            Some((end, value)) => {
                i = end;
                max = value;
            }
            None => {
                i += 1;
                max = u32::MAX;
            }
        }
    }
    if bytes.get(i) != Some(&b'}') {
        return Ok(None);
    }
    if leading_zero {
        return Err(unsupported(at, "repeat count with a leading zero"));
    }
    if min > MAX_REPEAT || (max != u32::MAX && (max > MAX_REPEAT || max < min)) {
        return Err(invalid(at, "repeat count out of range"));
    }
    Ok(Some((i + 1 - at, min, max)))
}

#[allow(clippy::too_many_lines)]
fn validate_go_compatible(pattern: &str) -> Result<(), RegexBuildError> {
    let bytes = pattern.as_bytes();
    let mut i = 0;
    // The previous token was a repetition operator (with its optional lazy `?`).
    let mut after_repeat = false;
    let mut depth = 0usize;
    // Go's `repeatIsValid`: counted repeats multiply down the nesting. Each
    // frame tracks the least budget `n` its content needs (`before`: finished
    // atoms, `last`: the atom a following repeat applies to).
    let mut frames: Vec<(u64, u64)> = vec![(0, 0)];
    while i < bytes.len() {
        let byte = bytes[i];
        if !byte.is_ascii() {
            return Err(unsupported(i, "non-ASCII pattern"));
        }
        if byte < 0x20 || byte == 0x7f {
            return Err(unsupported(i, "control character"));
        }
        let was_repeat = std::mem::take(&mut after_repeat);
        match byte {
            b'\\' => {
                fold_atom(&mut frames);
                i += 1 + escape_len(bytes, i, false)?;
            }
            b'[' => {
                fold_atom(&mut frames);
                i = class_end(bytes, i)?;
            }
            b'(' => {
                fold_atom(&mut frames);
                frames.push((0, 0));
                depth += 1;
                if bytes.get(i + 1) == Some(&b'?') {
                    i = group_prefix_end(bytes, i)?;
                } else {
                    i += 1;
                }
            }
            b')' => {
                if depth == 0 {
                    return Err(invalid(i, "unexpected )"));
                }
                depth -= 1;
                let (before, last) = frames.pop().unwrap_or_default();
                if let Some(parent) = frames.last_mut() {
                    parent.1 = before.max(last);
                }
                i += 1;
            }
            b'|' => {
                fold_atom(&mut frames);
                i += 1;
            }
            b'*' | b'+' | b'?' => {
                if was_repeat {
                    return Err(invalid(i, "nested repetition operator"));
                }
                i += 1;
                if bytes.get(i) == Some(&b'?') {
                    i += 1;
                }
                after_repeat = true;
            }
            b'{' => match repetition_len(bytes, i)? {
                Some((length, min, max)) => {
                    if was_repeat {
                        return Err(invalid(i, "nested repetition operator"));
                    }
                    if let Some(frame) = frames.last_mut() {
                        // Go's `repeatIsValid`, run when the repeat is applied
                        // (and only for min >= 2 or max >= 2): a zero maximum
                        // drops the content, otherwise the content's need is
                        // multiplied by the maximum (the minimum when open
                        // ended; an open `{0,}` leaves it as is).
                        let times = if max == u32::MAX { min } else { max };
                        frame.1 = if max == 0 {
                            0
                        } else if times == 0 {
                            frame.1
                        } else {
                            u64::from(times).saturating_mul(frame.1.max(1))
                        };
                        if (min >= 2 || (max != u32::MAX && max >= 2))
                            && frame.1 > u64::from(MAX_REPEAT)
                        {
                            return Err(invalid(i, "nested repeat counts exceed 1000"));
                        }
                    }
                    i += length;
                    if bytes.get(i) == Some(&b'?') {
                        i += 1;
                    }
                    after_repeat = true;
                }
                None => return Err(unsupported(i, "literal brace")),
            },
            _ => {
                fold_atom(&mut frames);
                i += 1;
            }
        }
    }
    if depth != 0 {
        return Err(invalid(bytes.len(), "missing )"));
    }
    Ok(())
}

/// A new atom starts: the previous one can no longer take a repeat.
fn fold_atom(frames: &mut [(u64, u64)]) {
    if let Some(frame) = frames.last_mut() {
        frame.0 = frame.0.max(frame.1);
        frame.1 = 0;
    }
}

/// Returns the index after the class that opens at `start`.
fn class_end(bytes: &[u8], start: usize) -> Result<usize, RegexBuildError> {
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'^') {
        i += 1;
    }
    let first = i;
    loop {
        let Some(&byte) = bytes.get(i) else {
            return Err(invalid(start, "missing ]"));
        };
        if !byte.is_ascii() {
            return Err(unsupported(i, "non-ASCII pattern"));
        }
        if byte < 0x20 || byte == 0x7f {
            return Err(unsupported(i, "control character"));
        }
        match byte {
            b']' if i == first => return Err(unsupported(i, "class-leading ]")),
            b']' => return Ok(i + 1),
            b'\\' => i += 1 + escape_len(bytes, i, true)?,
            b'[' => {
                // Only `[:name:]` / `[:^name:]` may nest.
                let rest = &bytes[i + 1..];
                let Some(inner) = rest.strip_prefix(b":") else {
                    return Err(unsupported(i, "nested character class"));
                };
                let Some(end) = inner.windows(2).position(|pair| pair == b":]") else {
                    return Err(unsupported(i, "nested character class"));
                };
                let name = inner[..end].strip_prefix(b"^").unwrap_or(&inner[..end]);
                if !POSIX_CLASSES.iter().any(|class| class.as_bytes() == name) {
                    return Err(unsupported(i, "unknown POSIX class"));
                }
                i += 1 + 1 + end + 2;
            }
            b'&' | b'-' | b'~' if bytes.get(i + 1) == Some(&byte) => {
                // `--`, `&&`, `~~` are set operators in Rust, literals in Go.
                return Err(unsupported(i, "Rust character-class set operator"));
            }
            _ => i += 1,
        }
    }
}

/// Validates `(?...` at `start` and returns the index after its prefix.
fn group_prefix_end(bytes: &[u8], start: usize) -> Result<usize, RegexBuildError> {
    let mut i = start + 2;
    match bytes.get(i) {
        Some(b':') => Ok(i + 1),
        _ => {
            // Flags `i m s U`, optionally `-` and more flags, then `)` or `:`.
            let mut seen = 0;
            let mut negated = false;
            while let Some(&flag) = bytes.get(i) {
                match flag {
                    b'i' | b'm' | b's' | b'U' => seen += 1,
                    b'-' if !negated => {
                        negated = true;
                        seen = 0;
                    }
                    b')' | b':' if seen > 0 => {
                        // `(?i)` ends the group scope like a flag group, not a
                        // capture: it opens no group to close.
                        return Ok(if flag == b')' { i } else { i + 1 });
                    }
                    _ => return Err(unsupported(start, "extended group")),
                }
                i += 1;
            }
            Err(unsupported(start, "extended group"))
        }
    }
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
        // Valid Go that this engine does not run: reported as unsupported,
        // never as "invalid", so callers can refuse instead of dropping.
        for pattern in [
            r"\p{Han}",
            r"\pL",
            r"\Q.\E",
            r"\1",
            r"\Z",
            r"\<",
            r"[ab&&b]",
            r"[ab~~b]",
            r"[a-z--m-z]",
            r"[]&&]",
            r"[^]&&]",
            r"[]--]",
            r"[]~~]",
            r"[[a-z]]",
            r"[[:nope:]]",
            r"(?=x)",
            r"(?<=x)",
            r"(?<n>x)",
            r"(?P<n>a)b",
            r"(?P<n>a)(?P<n>b)",
            r"(?P<1n>a)",
            r"(?x)",
            r"a{,3}",
            r"a{",
            r"a{01}",
            r"a{1,02}",
            "例",
            r"[^例]",
        ] {
            let error = compile_go_compatible(pattern).expect_err(pattern);
            assert!(
                matches!(error, RegexBuildError::Unsupported { .. }),
                "pattern {pattern:?} must be unsupported, got {error}"
            );
        }
        // Invalid in Go too.
        for pattern in [
            "a**",
            "a+*",
            "(",
            ")",
            "a)",
            "[",
            "[a",
            "a{2}{3}",
            "(a{0,33}){0,33}",
            "(a{2}){501}",
            "((a{10}){10}){11}",
            "((a{33}){33}){0}",
            "((a{33}){33}){0,0}",
            "((a{600}){0,}){2}",
            "((a{600}){1,}){2}",
            "a{1001}",
            "a{3,2}",
            r"a\",
        ] {
            let error = compile_go_compatible(pattern).expect_err(pattern);
            assert!(error.is_invalid_in_go(), "pattern {pattern:?}: {error}");
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
            r"\w",
            r"[\w-]",
            r"\d+",
            r"\s",
            r"\bfoo\b",
            r"\x41",
            r"\x{41}",
            r"\-",
            r"(?:foo|bar)\.test",
            r"(?i)Foo\.test",
            r"(?i:foo)\.test",
            r"[[:alpha:]]",
            r"[[:^digit:]x]",
            r"a{2,5}",
            r"(a{0,31}){0,31}",
            r"(a{2}){500}",
            r"((a{10}){10}){10}",
            r"((a{30}){30}){0}",
            r"(a{600}){0,}",
            r"(a{600}){1,}",
            r"((a{600}){1}){1}",
            r"(a{0}){1000}",
            r"(a{3}b{300}){3}",
            r"(a{1,}){1000}",
            r"a{2,}?",
            r"a*?b",
            r"(a*)*b",
            r"^*",
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
    fn engine_limits_are_refused_not_treated_as_invalid() {
        // 300 nested groups: valid Go, over Rust's default nesting limit.
        let pattern = format!("{}ads{}", "(".repeat(300), ")".repeat(300));
        let error = compile_go_compatible(&pattern).expect_err("over the nest limit");
        assert!(!error.is_invalid_in_go(), "{error}");
        // A plain syntax error stays "invalid in Go too".
        assert!(
            compile_go_compatible("[z-a]")
                .expect_err("bad range")
                .is_invalid_in_go()
        );
        // Valid Go that Rust refuses to compile must be refused, not skipped.
        let error = compile_go_compatible(r"[\w-a]").expect_err("class range with an escape");
        assert!(!error.is_invalid_in_go(), "{error}");
        // The Rust message embeds the pattern: text in the pattern must not
        // pass for the diagnostic.
        let error = compile_go_compatible(
            r"invalid character class range, the start must be <= the end|[\w-a]",
        )
        .expect_err("pattern text imitating the diagnostic");
        assert!(!error.is_invalid_in_go(), "{error}");
    }

    #[test]
    fn empty_matcher_never_matches() {
        let m = RegexMatcher::new();
        assert!(!m.is_match("anything"));
    }
}
