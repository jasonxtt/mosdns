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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use mosdns_matcher_core::MixMatcher;
use mosdns_upstream_core::TransportCancellation;

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

/// A test-only gate that parks the blocking persistence step so a test can
/// prove that a DNS reader is still served while an update is in flight.
///
/// The wait is bounded: if persistence were (wrongly) running on the DNS
/// runtime thread, the bound turns a hang into an ordinary test failure.
#[derive(Clone)]
pub struct PersistGate {
    state: Arc<(Mutex<bool>, Condvar)>,
    arrived: Arc<AtomicUsize>,
    bound: Duration,
}

impl PersistGate {
    /// A gate that parks persistence for at most three seconds per waiter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(false), Condvar::new())),
            arrived: Arc::new(AtomicUsize::new(0)),
            bound: Duration::from_secs(3),
        }
    }

    /// True once at least one persistence step has entered the gate.
    #[must_use]
    pub fn arrived(&self) -> bool {
        self.arrived.load(Ordering::SeqCst) > 0
    }

    /// Releases every waiter immediately.
    pub fn release(&self) {
        let (lock, condvar) = &*self.state;
        let mut released = lock.lock().unwrap_or_else(|error| error.into_inner());
        *released = true;
        condvar.notify_all();
    }

    pub(crate) fn wait(&self) {
        self.arrived.fetch_add(1, Ordering::SeqCst);
        let (lock, condvar) = &*self.state;
        let mut released = lock.lock().unwrap_or_else(|error| error.into_inner());
        while !*released {
            let (guard, timeout) = condvar
                .wait_timeout(released, self.bound)
                .unwrap_or_else(|error| error.into_inner());
            released = guard;
            if timeout.timed_out() {
                return;
            }
        }
    }
}

impl Default for PersistGate {
    fn default() -> Self {
        Self::new()
    }
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
    /// Serializes one whole update (compile, persist, publish) per provider, so
    /// two updates can never interleave between the file and the generation.
    update: tokio::sync::Mutex<()>,
    /// Test-only gate that holds an update immediately before publication.
    publish_barrier: RefCell<Option<TransportCancellation>>,
    /// Test-only gate that parks the blocking persistence step.
    persist_gate: RefCell<Option<PersistGate>>,
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
            update: tokio::sync::Mutex::new(()),
            publish_barrier: RefCell::new(None),
            persist_gate: RefCell::new(None),
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
    pub(crate) async fn save(&self) -> Result<(), ManagedSetError> {
        let _update = self.update.lock().await;
        let rules = self.rules();
        let file = self.file.clone();
        let fault = self.fault.get();
        // File I/O runs on the blocking pool, never on the DNS runtime thread.
        tokio::task::spawn_blocking(move || persist_rules_atomically(&file, &rules, fault))
            .await
            .map_err(|error| ManagedSetError::Blocking(error.to_string()))?
            .map_err(ManagedSetError::Persist)
    }

    /// Compiles, persists and then publishes one complete candidate. Returns
    /// the number of accepted rules. An error leaves the file, the generation
    /// and the temporary-file directory unchanged.
    ///
    /// The candidate compile and the file write run on the blocking pool, so the
    /// single-threaded DNS runtime keeps serving queries while an update is in
    /// flight. Updates are serialized per provider and the generation is
    /// exchanged in one step only after the file write succeeded.
    pub(crate) async fn replace(&self, values: &[String]) -> Result<usize, ManagedSetError> {
        let _update = self.update.lock().await;
        let file = self.file.clone();
        let fault = self.fault.get();
        let values = values.to_vec();
        let gate = self.persist_gate.borrow().clone();
        let (matcher, accepted) = tokio::task::spawn_blocking(move || {
            let (matcher, accepted) = compile_candidate(&values);
            if let Some(gate) = gate {
                gate.wait();
            }
            persist_rules_atomically(&file, &accepted, fault)?;
            Ok::<_, io::Error>((matcher, accepted))
        })
        .await
        .map_err(|error| ManagedSetError::Blocking(error.to_string()))?
        .map_err(ManagedSetError::Persist)?;

        // Test-only deterministic gate. Production leaves it unset, so the only
        // work between the successful write and the swap is this check.
        let barrier = self.publish_barrier.borrow().clone();
        if let Some(barrier) = barrier {
            barrier.cancelled().await;
        }

        let count = accepted.len();
        *self.generation.borrow_mut() = Rc::new(DomainSetGeneration::new(accepted, matcher));
        Ok(count)
    }

    /// Arms one narrow persistence failure. Only tests call this.
    #[doc(hidden)]
    pub fn inject_persist_fault(&self, fault: PersistFault) {
        self.fault.set(fault);
    }

    /// Arms one test-only gate that holds an update immediately before
    /// publication until the token is cancelled. Only tests call this.
    #[doc(hidden)]
    pub fn inject_publish_barrier(&self, barrier: TransportCancellation) {
        *self.publish_barrier.borrow_mut() = Some(barrier);
    }

    /// Arms one test-only gate that parks the blocking persistence step. Only
    /// tests call this.
    #[doc(hidden)]
    pub fn inject_persist_gate(&self, gate: PersistGate) {
        *self.persist_gate.borrow_mut() = Some(gate);
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
    /// The blocking candidate/persistence task could not be joined.
    Blocking(String),
}

impl fmt::Display for ManagedSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persist(error) => write!(formatter, "{error}"),
            Self::Blocking(error) => write!(formatter, "blocking update task failed: {error}"),
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
