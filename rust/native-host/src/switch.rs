use std::path::{Path, PathBuf};

use mosdns_sequence_core::{
    ExecutionState, Executor, ExecutorError, ExecutorOutcome, MatchOutcome, Matcher, MatcherError,
};

use crate::config::ConfigError;

/// The highest accepted switch plugin type number.
pub(crate) const SWITCH_TYPE_COUNT: u8 = 17;

/// The type number of one `switchN` plugin kind, or `None` when the name is
/// not one of the seventeen accepted switch types. Only the canonical
/// spelling counts: `switch01` and `switch001` are not aliases of `switch1`.
pub(crate) fn switch_type_number(kind: &str) -> Option<u8> {
    let digits = kind.strip_prefix("switch")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u8 = digits.parse().ok()?;
    let canonical = digits == number.to_string();
    ((1..=SWITCH_TYPE_COUNT).contains(&number) && canonical).then_some(number)
}

/// The query fast flag a switch type owns at DNS admission.
///
/// switch1..14 use bits 32..45, switch16 uses 47 and switch17 uses 49.
/// switch15 is value-only. Bits 46 and 48 belong to other owners and are
/// never seeded or cleared by a switch.
pub(crate) fn switch_bit(type_number: u8) -> Option<u32> {
    match type_number {
        1..=14 => Some(u32::from(type_number) + 31),
        16 => Some(47),
        17 => Some(49),
        _ => None,
    }
}

/// The admission-facts key the host seeds for one switch type. The native
/// host reserves slots 0..=16 for switches; other fact keys stay available to
/// future host owners.
pub(crate) fn switch_fact_key(type_number: u8) -> u32 {
    u32::from(type_number) - 1
}

/// One compiled switch declaration. Declarations are immutable compile
/// output: the mutable current value and its file live in the host's switch
/// registry, never in the program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchDeclaration {
    /// The plugin type number, 1..=17. At most one declaration per type.
    pub type_number: u8,
    /// The configured tag, unique across every plugin.
    pub tag: String,
    /// The resolved state-file path from `args.initial_value`.
    pub state_file: PathBuf,
    /// The configuration location that declared this switch.
    pub source_path: String,
}

/// Trims outer quote characters the way the reference quick setup does. The
/// remaining expectation, including an empty one, is kept verbatim.
pub(crate) fn trim_expectation(raw: &str) -> &str {
    raw.trim_matches(|c| c == '"' || c == '\'')
}

/// The quick `switchN <expected>` value matcher.
///
/// `A` on a bit-backed switch still reads the live query fast flag, so a
/// later query-local `fast_mark` can change that answer. Every other
/// expectation, and every switch15 expectation, compares the immutable value
/// admitted with the request.
pub(crate) struct SwitchMatcher {
    type_number: u8,
    bit: Option<u32>,
    expected: String,
}

impl SwitchMatcher {
    pub(crate) fn new(type_number: u8, expected: String) -> Self {
        Self {
            type_number,
            bit: switch_bit(type_number),
            expected,
        }
    }
}

impl Matcher for SwitchMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let matched = match self.bit {
            Some(bit) if self.expected == "A" => state.fast_flags & (1_u64 << bit) != 0,
            _ => state
                .admission_value(switch_fact_key(self.type_number))
                .is_some_and(|value| value == self.expected),
        };
        Ok(MatchOutcome::new(matched, None))
    }
}

/// A declared switch's `$tag` executable. The reference plugin just continues
/// the chain, so this is a no-op that still consumes the ordinary executable
/// dispatch.
pub(crate) struct SwitchNoopExecutor;

impl Executor for SwitchNoopExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        Ok(ExecutorOutcome::Continue)
    }
}

/// Resolves one declared state-file path against its declaration base and
/// normalizes it lexically, exactly the way cache dump targets resolve.
pub(crate) fn resolved_state_file(
    declared: &str,
    base_dir: &Path,
    error_path: &str,
) -> Result<PathBuf, ConfigError> {
    let resolved = crate::config::resolve_relative(declared, base_dir);
    let absolute = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .map_err(|error| ConfigError::new(error_path, error.to_string()))?
            .join(resolved)
    };
    Ok(crate::special_groups::lexical_path(&absolute))
}

/// The `FileKey` identity of an existing regular file, used to detect
/// hard-link aliases between state files and other product artifacts.
#[cfg(unix)]
pub(crate) fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let path = path.to_path_buf();
    let metadata = crate::transaction::blocking_io(move || std::fs::metadata(path))
        .ok()?
        .ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some((metadata.dev(), metadata.ino()))
}

/// Lexically normalized artifact identity for collision comparison.
pub(crate) fn normalized_artifact(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    crate::special_groups::lexical_path(&absolute)
}

/// Reports whether two paths name the same underlying file: by lexical
/// normalization always, and by canonical path or inode identity when both
/// files currently exist. Non-existing paths only collide lexically.
pub(crate) fn paths_collide(first: &Path, second: &Path) -> bool {
    if normalized_artifact(first) == normalized_artifact(second) {
        return true;
    }
    if let (Some(first), Some(second)) = (
        crate::config::canonicalized_path(first),
        crate::config::canonicalized_path(second),
    ) {
        if first == second {
            return true;
        }
    }
    #[cfg(unix)]
    {
        if let (Some(first), Some(second)) = (file_identity(first), file_identity(second)) {
            return first == second;
        }
    }
    false
}
