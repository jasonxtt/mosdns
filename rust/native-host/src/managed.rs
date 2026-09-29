//! Managed file-backed `domain_set` providers.
//!
//! Each managed tag owns exactly one writable `.txt` rule file and one
//! immutable published generation. A replacement compiles a complete candidate,
//! persists it with a same-directory temporary file and rename, and only then
//! publishes the new generation, so a reader can never observe a half-applied
//! update and a failed replacement leaves both the file and the live matcher
//! unchanged.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mosdns_matcher_core::MixMatcher;

/// Which step of the safe same-directory replacement fails.
///
/// Production never arms a fault. A test arms one to prove that old file bytes
/// and the old generation survive and that the temporary file is removed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PersistFault {
    /// Replace normally.
    #[default]
    None,
    /// Create the temporary file, then fail before it is complete.
    WriteTemp,
    /// Finish the temporary file, then fail instead of the final rename.
    Rename,
}

/// One immutable published generation: the accepted rule text and the compiled
/// matcher built from exactly those rules.
pub(crate) struct DomainSetGeneration {
    rules: Vec<String>,
    matcher: Rc<MixMatcher<()>>,
}

impl DomainSetGeneration {
    pub(crate) fn new(rules: Vec<String>, matcher: MixMatcher<()>) -> Self {
        Self {
            rules,
            matcher: Rc::new(matcher),
        }
    }
}

/// One managed provider keyed by its configured tag.
pub struct ManagedDomainSet {
    tag: String,
    file: PathBuf,
    source_path: String,
    generation: RefCell<Rc<DomainSetGeneration>>,
    fault: Cell<PersistFault>,
}

impl ManagedDomainSet {
    pub(crate) fn new(
        tag: String,
        file: PathBuf,
        source_path: String,
        rules: Vec<String>,
        matcher: MixMatcher<()>,
    ) -> Self {
        Self {
            tag,
            file,
            source_path,
            generation: RefCell::new(Rc::new(DomainSetGeneration::new(rules, matcher))),
            fault: Cell::new(PersistFault::None),
        }
    }

    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    #[must_use]
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    /// The accepted rule text of the current generation.
    #[must_use]
    pub fn rules(&self) -> Vec<String> {
        self.generation.borrow().rules.clone()
    }

    /// The number of accepted rules in the current generation.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.generation.borrow().matcher.len()
    }

    /// True when the current generation's matcher accepts `domain`. The handle
    /// is taken once, so one evaluation always uses one whole generation.
    #[must_use]
    pub fn matches(&self, domain: &str) -> bool {
        let generation = Rc::clone(&self.generation.borrow());
        generation.matcher.r#match(domain).is_some()
    }

    /// Persists the current generation without publishing anything new.
    pub(crate) fn save(&self) -> Result<(), ManagedSetError> {
        let rules = self.rules();
        persist_rules_atomically(&self.file, &rules, self.fault.get())
            .map_err(ManagedSetError::Persist)
    }

    /// Compiles, persists and then publishes one complete candidate. Returns
    /// the number of accepted rules. An error leaves the file, the generation
    /// and the temporary-file directory unchanged.
    pub(crate) fn replace(&self, values: &[String]) -> Result<usize, ManagedSetError> {
        let (matcher, accepted) = compile_candidate(values);
        persist_rules_atomically(&self.file, &accepted, self.fault.get())
            .map_err(ManagedSetError::Persist)?;
        let count = accepted.len();
        *self.generation.borrow_mut() = Rc::new(DomainSetGeneration::new(accepted, matcher));
        Ok(count)
    }

    /// Arms one narrow persistence failure. Only tests call this.
    #[doc(hidden)]
    pub fn inject_persist_fault(&self, fault: PersistFault) {
        self.fault.set(fault);
    }
}

impl fmt::Debug for ManagedDomainSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedDomainSet")
            .field("tag", &self.tag)
            .field("file", &self.file)
            .field("rules", &self.rule_count())
            .finish()
    }
}

/// Builds one complete candidate from submitted values.
///
/// A submitted value is normalized exactly like a rule-file line: outer
/// whitespace is trimmed and an empty or whole-line `#` value is skipped. This
/// keeps POST, the persisted file and a restart producing the same effective
/// rule set, and it prevents an empty value from becoming a root suffix rule
/// that would match every domain.
fn compile_candidate(values: &[String]) -> (MixMatcher<()>, Vec<String>) {
    let mut matcher = MixMatcher::new();
    matcher.set_default("domain");
    let mut accepted = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        if matcher.add(value, ()).is_ok() {
            accepted.push(value.to_owned());
        }
    }
    (matcher, accepted)
}

/// A managed-provider failure that guarantees no partial publication.
#[derive(Debug)]
pub enum ManagedSetError {
    Persist(io::Error),
}

impl fmt::Display for ManagedSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persist(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ManagedSetError {}

/// One query-time handle for a compiled `domain_set`. Query-only shapes read a
/// fixed startup matcher; managed shapes read the provider's current
/// generation at every evaluation.
#[derive(Clone)]
pub(crate) enum DomainSetHandle {
    Fixed(Rc<MixMatcher<()>>),
    Managed(Rc<ManagedDomainSet>),
}

impl DomainSetHandle {
    pub(crate) fn matches(&self, domain: &str) -> bool {
        match self {
            Self::Fixed(matcher) => matcher.r#match(domain).is_some(),
            Self::Managed(provider) => provider.matches(domain),
        }
    }

    pub(crate) fn rule_count(&self) -> usize {
        match self {
            Self::Fixed(matcher) => matcher.len(),
            Self::Managed(provider) => provider.rule_count(),
        }
    }
}

/// Writes `rules` to `path` through a same-directory temporary file and a
/// final rename. A failure removes the temporary file and never touches the
/// existing target. This gives whole-file replacement and process-restart
/// retention; it does not claim power-loss durability.
fn persist_rules_atomically(path: &Path, rules: &[String], fault: PersistFault) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rule file has no file name"))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".tmp-{}", std::process::id()));
    let temporary = directory.join(temporary_name);

    let outcome = write_temporary_and_replace(&temporary, path, rules, fault);
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

fn write_temporary_and_replace(
    temporary: &Path,
    target: &Path,
    rules: &[String],
    fault: PersistFault,
) -> io::Result<()> {
    if fault == PersistFault::WriteTemp {
        // Leave a real temporary file behind so the cleanup path is exercised.
        let mut partial = File::create(temporary)?;
        partial.write_all(b"# injected partial write\n")?;
        partial.flush()?;
        return Err(io::Error::other("injected temporary-write failure"));
    }
    {
        let file = File::create(temporary)?;
        let mut writer = BufWriter::new(file);
        for rule in rules {
            writer.write_all(rule.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    if fault == PersistFault::Rename {
        return Err(io::Error::other("injected final replace failure"));
    }
    std::fs::rename(temporary, target)
}
