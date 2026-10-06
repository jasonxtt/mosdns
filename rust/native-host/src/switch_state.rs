//! Host-owned switch state: the owner registry and its durable single-file
//! mutation.
//!
//! The `/plugins/{tag}/show|post` HTTP routes that drive the mutation API
//! land in S4 of this task; until then the API surface below is exercised by
//! the in-crate registry and admission tests.
#![cfg_attr(not(test), allow(dead_code))]
//!
//! One registry belongs to one committed runtime view. Owners are admitted at
//! startup (or runtime rebind) through bounded, side-effect-free file reads;
//! values publish as one immutable aggregate admission-facts map. A POST is
//! serialized per owner, bounded globally, and completes through a
//! same-directory temporary file and an atomic rename even if the requesting
//! HTTP future is dropped.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use mosdns_sequence_core::AdmissionFacts;

use crate::switch::{SwitchDeclaration, switch_fact_key};

/// The on-disk bound for one switch state file. A larger existing file fails
/// startup; a larger POSTed value is rejected before any disk work.
pub(crate) const MAX_STATE_FILE_BYTES: usize = 1024 * 1024;

/// The global bound for accepted-but-unfinished switch mutations.
const MAX_ACCEPTED_MUTATIONS: usize = 4;

/// A startup (or runtime-rebind) admission failure for one switch owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SwitchAdmissionError {
    pub(crate) tag: String,
    pub(crate) path: PathBuf,
    pub(crate) reason: String,
}

/// The typed rejection of one switch mutation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SwitchMutationError {
    UnknownTag(String),
    /// The owner is not write-eligible until a restart or runtime rebind.
    ReadOnly {
        tag: String,
        reason: String,
    },
    /// The value exceeds the state-file byte bound.
    ValueTooLarge {
        tag: String,
        bytes: usize,
    },
    /// Another mutation already owns this owner.
    Busy {
        tag: String,
    },
    /// The global accepted-mutation budget is exhausted.
    Overloaded,
    /// The registry stopped admitting new work (shutdown).
    Closed,
    /// Host management admission is paused (managed apply or recovery).
    Paused,
    /// A post-rename durability ambiguity fenced the registry.
    RecoveryRequired,
    /// The final precommit revalidation observed an external change.
    Conflict {
        tag: String,
        reason: String,
    },
    /// A precommit I/O failure; the old file and value are unchanged.
    Io {
        tag: String,
        reason: String,
    },
}

/// Deterministic test-only faults at the durable-commit boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SwitchCommitFault {
    /// Create the temporary file, then fail before it is complete.
    WriteTemp,
    /// Finish and sync the temporary file, then fail instead of renaming.
    Rename,
    /// Complete the rename, then fail the directory sync: post-rename
    /// durability becomes ambiguous and the registry must fail closed.
    DirSync,
}

/// Test-only crash points for subprocess durability proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SwitchCommitBoundary {
    /// The temporary file is written and synced, before the rename.
    TempSynced,
    /// The rename completed, before the directory sync.
    Renamed,
    /// The durable commit is complete, before the memory publication.
    Committed,
}

/// The test-only gates one commit carries onto the I/O thread.
#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct CommitGates {
    fault: Option<SwitchCommitFault>,
    crash_after: Option<SwitchCommitBoundary>,
}

#[cfg(not(test))]
#[derive(Clone, Copy, Default)]
struct CommitGates;

#[cfg(test)]
impl CommitGates {
    fn fault_is(self, fault: SwitchCommitFault) -> bool {
        self.fault == Some(fault)
    }

    fn crashes_at(self, boundary: SwitchCommitBoundary) -> bool {
        self.crash_after == Some(boundary)
    }
}

#[cfg(not(test))]
impl CommitGates {
    fn fault_is(self, _fault: SwitchCommitFault) -> bool {
        false
    }

    fn crashes_at(self, _boundary: SwitchCommitBoundary) -> bool {
        false
    }
}

/// The identity of the state file a mutation believes it is replacing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SwitchFileFingerprint {
    /// `(device, inode)` of the regular file, when it exists.
    identity: Option<(u64, u64)>,
    /// sha256 of the file's raw bytes, when it exists.
    content: Option<String>,
}

impl SwitchFileFingerprint {
    fn missing() -> Self {
        Self::default()
    }

    fn of_bytes(identity: Option<(u64, u64)>, bytes: &[u8]) -> Self {
        Self {
            identity,
            content: Some(crate::special_groups::sha256(bytes)),
        }
    }
}

/// The immutable per-admission switch values and the fast bits seeded for
/// owners whose admitted value is `A`. One seed is captured per real DNS
/// datagram or TCP frame and never refreshed inside a request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SwitchAdmissionSeed {
    pub(crate) facts: AdmissionFacts,
    pub(crate) a_bits: u64,
}

/// The public, value-free view of one admitted switch owner used by
/// capability discovery. The current value never leaves the show endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SwitchCapability {
    pub(crate) type_number: u8,
    pub(crate) tag: String,
    pub(crate) readable: bool,
    pub(crate) writable: bool,
    pub(crate) reason: Option<String>,
}

/// One admitted switch owner. The value and fingerprint are only mutated by
/// the owner's serialized publication after a durable commit; eligibility is
/// only refreshed at startup or runtime rebind.
struct SwitchOwner {
    declaration: SwitchDeclaration,
    value: RefCell<String>,
    revision: Cell<u64>,
    fingerprint: RefCell<SwitchFileFingerprint>,
    /// Write eligibility, re-evaluated at startup or runtime rebind.
    writable: Cell<bool>,
    reason: RefCell<Option<String>>,
    /// The state file's parent-directory identity captured at admission.
    parent_identity: Cell<Option<(u64, u64)>>,
    mutating: Cell<bool>,
}

struct SwitchRegistryInner {
    owners: RefCell<Vec<Rc<SwitchOwner>>>,
    facts: RefCell<AdmissionFacts>,
    accepted: Cell<usize>,
    closed: Cell<bool>,
    recovery: Cell<bool>,
    /// The fast bits seeded at admission for owners whose value is `A`.
    a_bits: Cell<u64>,
    tasks: RefCell<tokio::task::JoinSet<()>>,
    drained: tokio::sync::Notify,
    /// Weak host-control hooks, attached by the owning runtime. Present in
    /// every production registry and absent from standalone test registries;
    /// weak so a retired view never keeps the control alive.
    control: RefCell<Option<crate::runtime_snapshot::RuntimeControlWeak>>,
    /// Declarations of owners not yet admitted. A candidate collects them at
    /// preparation without touching the files and admits them after the
    /// management drain, so a late old-owner write cannot be overwritten by
    /// a stale preparation-time read.
    pending: RefCell<Vec<SwitchDeclaration>>,
    #[cfg(test)]
    fault: Cell<Option<SwitchCommitFault>>,
    #[cfg(test)]
    crash_after: Cell<Option<SwitchCommitBoundary>>,
    #[cfg(test)]
    commit_hold: RefCell<Option<crate::managed::PersistGate>>,
}

/// The switch registry owned by one committed runtime view.
pub(crate) struct SwitchRegistry {
    inner: Rc<SwitchRegistryInner>,
}

/// The handle returned when a mutation is accepted. Dropping it never cancels
/// the accepted durable work; it only gives up observing the outcome.
pub(crate) struct SwitchMutationTicket {
    completion: tokio::sync::oneshot::Receiver<Result<(), SwitchMutationError>>,
}

impl SwitchMutationTicket {
    /// Waits for the accepted mutation's terminal outcome.
    pub(crate) async fn complete(self) -> Result<(), SwitchMutationError> {
        self.completion
            .await
            .unwrap_or(Err(SwitchMutationError::Closed))
    }
}

struct AdmittedOwner {
    value: String,
    fingerprint: SwitchFileFingerprint,
    parent_identity: Option<(u64, u64)>,
    writable: bool,
    reason: Option<String>,
}

impl Clone for SwitchRegistry {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl SwitchRegistry {
    /// Admits every declared owner with a bounded, side-effect-free read of
    /// its state file. A malformed, unreadable, or aliased input fails the
    /// whole admission rather than silently destroying state.
    pub(crate) fn build(declarations: &[SwitchDeclaration]) -> Result<Self, SwitchAdmissionError> {
        let registry = Self::rebind(None, declarations)?;
        registry.complete_pending_admissions()?;
        Ok(registry)
    }

    /// Builds the registry for one committed runtime view. Owners whose
    /// type, tag, and state file are unchanged carry over as live objects, so
    /// their values are whatever the latest surviving publication left, with
    /// only their filesystem eligibility re-evaluated. New, moved, or changed
    /// identities are queued as pending: their bounded file reads happen in
    /// [`Self::complete_pending_admissions`] after the management drain, so
    /// nothing captured during candidate preparation can publish stale state.
    pub(crate) fn rebind(
        previous: Option<&Self>,
        declarations: &[SwitchDeclaration],
    ) -> Result<Self, SwitchAdmissionError> {
        let previous_owners = previous.map(|registry| registry.inner.owners.borrow().clone());
        let mut owners: Vec<Rc<SwitchOwner>> = Vec::with_capacity(declarations.len());
        let mut pending: Vec<SwitchDeclaration> = Vec::new();
        for declaration in declarations {
            let unchanged = previous_owners.as_ref().and_then(|owners| {
                owners.iter().find(|owner| {
                    owner.declaration.type_number == declaration.type_number
                        && owner.declaration.tag == declaration.tag
                        && owner.declaration.state_file == declaration.state_file
                })
            });
            if let Some(owner) = unchanged {
                owners.push(clone_owner_for_rebind(owner)?);
                continue;
            }
            pending.push(declaration.clone());
        }
        let registry = Self {
            inner: Rc::new(SwitchRegistryInner {
                owners: RefCell::new(owners),
                facts: RefCell::new(AdmissionFacts::default()),
                accepted: Cell::new(0),
                closed: Cell::new(false),
                recovery: Cell::new(false),
                a_bits: Cell::new(0),
                tasks: RefCell::new(tokio::task::JoinSet::new()),
                drained: tokio::sync::Notify::new(),
                control: RefCell::new(None),
                pending: RefCell::new(pending),
                #[cfg(test)]
                fault: Cell::new(None),
                #[cfg(test)]
                crash_after: Cell::new(None),
                #[cfg(test)]
                commit_hold: RefCell::new(None),
            }),
        };
        registry.refresh_aggregate();
        Ok(registry)
    }

    /// Attaches the weak host-control hooks that own this registry. They grant
    /// the management lease for accepted mutations and receive the fatal
    /// post-rename ambiguity.
    pub(crate) fn attach_control(&self, control: crate::runtime_snapshot::RuntimeControlWeak) {
        *self.inner.control.borrow_mut() = Some(control);
    }

    /// Admits every pending owner through bounded reads on the owned I/O
    /// worker. The caller invokes this only after the previous view's
    /// management work has drained and before publication, so each read
    /// observes the files' post-drain state. A failure aborts the candidate
    /// without disturbing the current registry.
    pub(crate) fn complete_pending_admissions(&self) -> Result<(), SwitchAdmissionError> {
        let declarations = std::mem::take(&mut *self.inner.pending.borrow_mut());
        for declaration in declarations {
            let for_worker = declaration.clone();
            let admission = crate::transaction::blocking_io(move || admit(&for_worker))
                .map_err(|error| SwitchAdmissionError {
                    tag: declaration.tag.clone(),
                    path: declaration.state_file.clone(),
                    reason: format!("admission worker failed: {error}"),
                })
                .and_then(|result| result);
            let admission = admission.map_err(|mut error| {
                error.tag = declaration.tag.clone();
                error.path = declaration.state_file.clone();
                error
            })?;
            // Alias defense in depth: two owners must never share one
            // underlying file, even if a later rename bypassed the
            // compiler's lexical checks.
            if let Some(identity) = admission.fingerprint.identity {
                if let Some(existing) = self
                    .inner
                    .owners
                    .borrow()
                    .iter()
                    .find(|other| other.fingerprint.borrow().identity == Some(identity))
                    .cloned()
                {
                    return Err(SwitchAdmissionError {
                        tag: declaration.tag.clone(),
                        path: declaration.state_file.clone(),
                        reason: format!(
                            "state file is the same underlying file as switch `{}`",
                            existing.declaration.tag
                        ),
                    });
                }
            }
            self.inner.owners.borrow_mut().push(Rc::new(SwitchOwner {
                declaration,
                value: RefCell::new(admission.value),
                revision: Cell::new(0),
                fingerprint: RefCell::new(admission.fingerprint),
                writable: Cell::new(admission.writable),
                reason: RefCell::new(admission.reason),
                parent_identity: Cell::new(admission.parent_identity),
                mutating: Cell::new(false),
            }));
        }
        self.rebuild_aggregate();
        Ok(())
    }

    /// Copies the latest durable value state from the previous committed view
    /// into the independent candidate owners after the previous view drains.
    /// Eligibility remains candidate-local, while a mutation that completed
    /// during the drain is not lost when the candidate is published.
    pub(crate) fn rebase_carried_values(&self, previous: &Self) {
        let previous_owners = previous.inner.owners.borrow();
        let owners = self.inner.owners.borrow();
        for owner in owners.iter() {
            let Some(previous_owner) = previous_owners.iter().find(|candidate| {
                candidate.declaration.type_number == owner.declaration.type_number
                    && candidate.declaration.tag == owner.declaration.tag
                    && candidate.declaration.state_file == owner.declaration.state_file
            }) else {
                continue;
            };
            *owner.value.borrow_mut() = previous_owner.value.borrow().clone();
            owner.revision.set(previous_owner.revision.get());
            *owner.fingerprint.borrow_mut() = previous_owner.fingerprint.borrow().clone();
        }
        drop(owners);
        drop(previous_owners);
        self.refresh_aggregate();
    }

    /// Rebuilds the aggregate admission facts and `A` fast bits from the
    /// owners' current values. Runtime publication calls this synchronously;
    /// it never awaits or performs I/O.
    pub(crate) fn refresh_aggregate(&self) {
        self.rebuild_aggregate();
    }

    /// The one coherent derivation: the aggregate and fast bits are always
    /// rebuilt together from the latest committed owner values, so
    /// simultaneous mutations of different owners cannot lose one another's
    /// publication and `A` bits can never go stale.
    fn rebuild_aggregate(&self) {
        rebuild_aggregate(&self.inner);
    }

    /// The immutable seed captured at one real DNS admission.
    pub(crate) fn admission_seed(&self) -> SwitchAdmissionSeed {
        SwitchAdmissionSeed {
            facts: self.inner.facts.borrow().clone(),
            a_bits: self.inner.a_bits.get(),
        }
    }

    /// The exact committed raw value of one configured switch.
    pub(crate) fn show(&self, tag: &str) -> Option<String> {
        self.inner
            .owners
            .borrow()
            .iter()
            .find(|owner| owner.declaration.tag == tag)
            .map(|owner| owner.value.borrow().clone())
    }

    pub(crate) fn is_known(&self, tag: &str) -> bool {
        self.inner
            .owners
            .borrow()
            .iter()
            .any(|owner| owner.declaration.tag == tag)
    }

    pub(crate) fn capabilities(&self) -> Vec<SwitchCapability> {
        self.inner
            .owners
            .borrow()
            .iter()
            .map(|owner| SwitchCapability {
                type_number: owner.declaration.type_number,
                tag: owner.declaration.tag.clone(),
                readable: true,
                writable: owner.writable.get(),
                reason: owner.reason.borrow().clone(),
            })
            .collect()
    }

    /// The immutable aggregate values captured for DNS admission.
    pub(crate) fn admission_facts(&self) -> AdmissionFacts {
        self.inner.facts.borrow().clone()
    }

    /// Whether a post-rename ambiguity has fenced the registry.
    pub(crate) fn recovery_required(&self) -> bool {
        self.inner.recovery.get()
    }

    /// Stops admitting new mutations. Accepted work still completes.
    pub(crate) fn close(&self) {
        self.inner.closed.set(true);
    }

    /// Waits until every accepted mutation reached its terminal state.
    pub(crate) async fn drain(&self) {
        loop {
            reap_finished_tasks(&self.inner);
            if self.inner.accepted.get() == 0 {
                reap_finished_tasks(&self.inner);
                return;
            }
            let notified = self.inner.drained.notified();
            if self.inner.accepted.get() == 0 {
                reap_finished_tasks(&self.inner);
                return;
            }
            notified.await;
        }
    }

    /// Accepts one mutation. The returned ticket observes a durable commit
    /// that runs as owned host work: dropping the ticket, or the HTTP future
    /// that created it, never cancels the accepted write.
    pub(crate) fn post(
        &self,
        tag: &str,
        value: String,
    ) -> Result<SwitchMutationTicket, SwitchMutationError> {
        let inner = &self.inner;
        if inner.recovery.get() {
            return Err(SwitchMutationError::RecoveryRequired);
        }
        if inner.closed.get() {
            return Err(SwitchMutationError::Closed);
        }
        let owner = inner
            .owners
            .borrow()
            .iter()
            .find(|owner| owner.declaration.tag == tag)
            .cloned()
            .ok_or_else(|| SwitchMutationError::UnknownTag(tag.to_owned()))?;
        if value.len() > MAX_STATE_FILE_BYTES {
            return Err(SwitchMutationError::ValueTooLarge {
                tag: tag.to_owned(),
                bytes: value.len(),
            });
        }
        if !owner.writable.get() {
            return Err(SwitchMutationError::ReadOnly {
                tag: tag.to_owned(),
                reason: owner
                    .reason
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| "switch is not writable".to_owned()),
            });
        }
        if inner.accepted.get() >= MAX_ACCEPTED_MUTATIONS {
            return Err(SwitchMutationError::Overloaded);
        }
        if owner.mutating.get() {
            return Err(SwitchMutationError::Busy {
                tag: tag.to_owned(),
            });
        }
        // Accepted mutations hold the host management lease through durable
        // completion and publication, so a managed apply drains them.
        let lease = inner
            .control
            .borrow()
            .as_ref()
            .and_then(|control| control.begin_management_mutation());
        if inner.control.borrow().is_some() && lease.is_none() {
            return Err(SwitchMutationError::Paused);
        }
        owner.mutating.set(true);
        inner.accepted.set(inner.accepted.get() + 1);
        // Completed owned-task records are reaped as new work arrives so the
        // tracked task storage stays bounded through unbounded sequential
        // mutation traffic.
        reap_finished_tasks(inner);

        let revision = owner.revision.get() + 1;

        let work = DurableWork {
            path: owner.declaration.state_file.clone(),
            expected: owner.fingerprint.borrow().clone(),
            expected_parent: owner.parent_identity.get(),
            bytes: value.clone().into_bytes(),
            gates: commit_gates(inner),
            hold: commit_hold(inner),
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task_inner = Rc::clone(inner);
        let task_owner = Rc::clone(&owner);
        inner.tasks.borrow_mut().spawn_local(async move {
            let outcome = commit_and_publish(&task_inner, &task_owner, work, revision, value).await;
            // Slots and the management lease are released before the
            // requester is acknowledged.
            task_owner.mutating.set(false);
            task_inner
                .accepted
                .set(task_inner.accepted.get().saturating_sub(1));
            task_inner.drained.notify_waiters();
            drop(lease);
            let _ = sender.send(outcome);
        });
        Ok(SwitchMutationTicket {
            completion: receiver,
        })
    }

    /// Arms one deterministic commit fault. Only tests call this.
    #[cfg(test)]
    pub(crate) fn inject_fault(&self, fault: SwitchCommitFault) {
        self.inner.fault.set(Some(fault));
    }

    /// Aborts the process at one commit boundary. Only the subprocess crash
    /// tests call this.
    #[cfg(test)]
    pub(crate) fn inject_crash_after(&self, boundary: SwitchCommitBoundary) {
        self.inner.crash_after.set(Some(boundary));
    }

    /// Parks every durable commit inside the I/O step. Only tests call this.
    #[cfg(test)]
    pub(crate) fn inject_commit_hold(&self, gate: crate::managed::PersistGate) {
        *self.inner.commit_hold.borrow_mut() = Some(gate);
    }
}

#[cfg(test)]
fn commit_hold(inner: &SwitchRegistryInner) -> Option<crate::managed::PersistGate> {
    inner.commit_hold.borrow().clone()
}

#[cfg(not(test))]
fn commit_hold(_inner: &SwitchRegistryInner) -> Option<crate::managed::PersistGate> {
    None
}

#[cfg(test)]
fn commit_gates(inner: &SwitchRegistryInner) -> CommitGates {
    CommitGates {
        fault: inner.fault.get(),
        crash_after: inner.crash_after.get(),
    }
}

#[cfg(not(test))]
fn commit_gates(_inner: &SwitchRegistryInner) -> CommitGates {
    CommitGates
}

async fn commit_and_publish(
    inner: &Rc<SwitchRegistryInner>,
    owner: &Rc<SwitchOwner>,
    work: DurableWork,
    revision: u64,
    value: String,
) -> Result<(), SwitchMutationError> {
    let tag = owner.declaration.tag.clone();
    let joined = tokio::task::spawn_blocking(move || durable_replace(&work)).await;
    let committed = joined
        .map_err(|error| SwitchMutationError::Io {
            tag: tag.clone(),
            reason: format!("mutation worker failed: {error}"),
        })
        .and_then(|result| result.map_err(|error| switch_mutation_error(tag.clone(), error)));
    match committed {
        Ok(new_fingerprint) => {
            *owner.value.borrow_mut() = value;
            owner.revision.set(revision);
            *owner.fingerprint.borrow_mut() = new_fingerprint;
            // The aggregate and fast bits are derived from the latest
            // committed owner set in one synchronous step, so concurrent
            // publications of different owners compose instead of racing
            // stale captured maps, and A bits never go stale.
            rebuild_aggregate(inner);
            Ok(())
        }
        Err(error @ SwitchMutationError::RecoveryRequired) => {
            // A post-rename ambiguity fences the registry, stops admission,
            // and stops the host.
            inner.recovery.set(true);
            if let Some(control) = inner.control.borrow().as_ref() {
                control.fatal_recovery();
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// The one coherent aggregate derivation shared by publication, runtime
/// publication, and rebind completion.
fn rebuild_aggregate(inner: &SwitchRegistryInner) {
    let mut facts: BTreeMap<u32, Arc<str>> = BTreeMap::new();
    let mut a_bits = 0_u64;
    for owner in inner.owners.borrow().iter() {
        let value = owner.value.borrow().clone();
        if value == "A" {
            if let Some(bit) = crate::switch::switch_bit(owner.declaration.type_number) {
                a_bits |= 1_u64 << bit;
            }
        }
        facts.insert(
            switch_fact_key(owner.declaration.type_number),
            Arc::from(value.as_str()),
        );
    }
    *inner.facts.borrow_mut() = Arc::new(facts);
    inner.a_bits.set(a_bits);
}

/// Reaps owned mutation tasks that finished; their outcome was already
/// delivered through the ticket channel.
fn reap_finished_tasks(inner: &SwitchRegistryInner) {
    while let Some(result) = inner.tasks.borrow_mut().try_join_next() {
        let _ = result;
    }
}

fn switch_mutation_error(tag: String, error: DurableError) -> SwitchMutationError {
    match error {
        DurableError::Conflict(reason) => SwitchMutationError::Conflict { tag, reason },
        DurableError::Io(reason) => SwitchMutationError::Io { tag, reason },
        DurableError::Ambiguous => SwitchMutationError::RecoveryRequired,
    }
}

/// The owned payload of one accepted mutation, moved onto the I/O thread.
struct DurableWork {
    path: PathBuf,
    expected: SwitchFileFingerprint,
    expected_parent: Option<(u64, u64)>,
    bytes: Vec<u8>,
    gates: CommitGates,
    /// Test-only parking gate inside the durable commit.
    hold: Option<crate::managed::PersistGate>,
}

enum DurableError {
    /// The final precommit revalidation observed an external change; nothing
    /// was replaced.
    Conflict(String),
    /// A pre-rename I/O failure; the temporary file is removed and the old
    /// file is untouched.
    Io(String),
    /// The rename completed but durability could not be confirmed. The final
    /// file must be preserved and the registry fenced.
    Ambiguous,
}

/// The durable single-file replacement. Every step runs on the I/O thread
/// through the pinned parent directory:
///
/// revalidate parent/file identity -> write+sync a same-directory temporary
/// -> final revalidation -> atomic rename -> sync directory.
///
/// This is not a compare-and-swap against uncoordinated external writers:
/// only conflicts observable at the final revalidation are rejected.
fn durable_replace(work: &DurableWork) -> Result<SwitchFileFingerprint, DurableError> {
    let name = work
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DurableError::Io(format!(
                "state file `{}` has an unusable file name",
                work.path.display()
            ))
        })?;
    if let Some(hold) = &work.hold {
        hold.wait();
    }
    let pinned = PinnedDirectory::open(work.path.parent().unwrap_or_else(|| Path::new("")))
        .map_err(|error| DurableError::Io(format!("cannot open state directory: {error}")))?;
    if pinned.identity() != work.expected_parent {
        return Err(DurableError::Conflict(
            "the state file's parent directory changed since admission".to_owned(),
        ));
    }
    let current = pinned
        .target_fingerprint(name)
        .map_err(|error| DurableError::Io(format!("cannot revalidate state file: {error}")))?;
    if current != work.expected {
        return Err(DurableError::Conflict(
            "the state file changed since admission; refusing to replace an external write"
                .to_owned(),
        ));
    }

    let temp = temp_name(name);
    // The replacement identity is captured from the temporary file before
    // the point of no return: rename(2) within the pinned directory
    // preserves the inode, so this is the committed file's identity and no
    // fallible verification remains after the durable rename. A failure
    // here is an ordinary precommit I/O error, never a post-rename one.
    let identity = match pinned.write_replacement(&temp, &work.bytes, work.gates) {
        Ok(identity) => identity,
        Err(error) => {
            // The temporary file may exist even after a partial write failure.
            pinned.remove_temporary(&temp);
            return Err(DurableError::Io(format!(
                "cannot write state replacement: {error}"
            )));
        }
    };
    // The final precommit revalidation: an external change observed here is
    // the only conflict the write promises to reject.
    let outcome = match pinned.target_fingerprint(name) {
        Err(error) => Err(DurableError::Io(format!(
            "cannot revalidate state file: {error}"
        ))),
        Ok(revalidated) if revalidated != work.expected => Err(DurableError::Conflict(
            "the state file changed during the commit; refusing to replace an external write"
                .to_owned(),
        )),
        Ok(_) => pinned.finalize_replacement(&temp, name, work.gates),
    };
    if let Err(error) = outcome {
        // Pre-rename failures clean the owned temporary file; the old file
        // and value are unchanged. Post-rename ambiguity keeps everything.
        if !matches!(error, DurableError::Ambiguous) {
            pinned.remove_temporary(&temp);
        }
        return Err(error);
    }
    if work.gates.crashes_at(SwitchCommitBoundary::Committed) {
        std::process::abort();
    }
    Ok(SwitchFileFingerprint::of_bytes(identity, &work.bytes))
}

/// The filesystem result of walking a state file's ancestors: every existing
/// one must be a real directory, and the state directory's identity is
/// captured when it exists.
enum ParentState {
    Missing,
    Present(Option<(u64, u64)>),
}

fn walk_parents(
    parent: &Path,
    tag: &str,
    state_file: &Path,
) -> Result<ParentState, SwitchAdmissionError> {
    let fail = |reason: String| SwitchAdmissionError {
        tag: tag.to_owned(),
        path: state_file.to_path_buf(),
        reason,
    };
    let mut parent_exists = !parent.as_os_str().is_empty();
    for ancestor in parent.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(fail(format!(
                        "path component `{}` is not a directory",
                        ancestor.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if ancestor == parent {
                    parent_exists = false;
                } else {
                    return Err(fail(format!(
                        "path component `{}` is missing: {error}",
                        ancestor.display()
                    )));
                }
            }
            Err(error) => {
                return Err(fail(format!(
                    "cannot inspect path component `{}`: {error}",
                    ancestor.display()
                )));
            }
        }
    }
    if parent_exists {
        let identity = directory_identity(parent).map_err(|error| {
            fail(format!(
                "cannot identify state directory `{}`: {error}",
                parent.display()
            ))
        })?;
        Ok(ParentState::Present(Some(identity)))
    } else {
        Ok(ParentState::Missing)
    }
}

/// Re-evaluates one carried owner's filesystem eligibility at runtime rebind:
/// the value, revision, and fingerprint stay whatever the latest publication
/// left; only write eligibility and the captured parent identity refresh.
fn refresh_owner_eligibility(owner: &Rc<SwitchOwner>) -> Result<(), SwitchAdmissionError> {
    let declaration = &owner.declaration;
    let parent = declaration
        .state_file
        .parent()
        .unwrap_or_else(|| Path::new(""));
    match walk_parents(parent, &declaration.tag, &declaration.state_file)? {
        ParentState::Present(identity) => {
            owner.writable.set(true);
            *owner.reason.borrow_mut() = None;
            owner.parent_identity.set(identity);
        }
        ParentState::Missing => {
            owner.writable.set(false);
            *owner.reason.borrow_mut() = Some(format!(
                "state directory `{}` does not exist; create it and restart or rebind to                  re-evaluate write eligibility",
                parent.display()
            ));
            owner.parent_identity.set(None);
        }
    }
    Ok(())
}

/// Copies a carried owner before re-evaluating its filesystem eligibility.
/// Candidate preparation must never mutate the owner held by the currently
/// committed runtime view: a later candidate failure must leave that view's
/// capability and POST behavior unchanged.
fn clone_owner_for_rebind(
    owner: &Rc<SwitchOwner>,
) -> Result<Rc<SwitchOwner>, SwitchAdmissionError> {
    let clone = Rc::new(SwitchOwner {
        declaration: owner.declaration.clone(),
        value: RefCell::new(owner.value.borrow().clone()),
        revision: Cell::new(owner.revision.get()),
        fingerprint: RefCell::new(owner.fingerprint.borrow().clone()),
        writable: Cell::new(owner.writable.get()),
        reason: RefCell::new(owner.reason.borrow().clone()),
        parent_identity: Cell::new(owner.parent_identity.get()),
        // Runtime candidate preparation happens after the previous view's
        // management drain; a published clone starts with no local mutation.
        mutating: Cell::new(false),
    });
    refresh_owner_eligibility(&clone)?;
    Ok(clone)
}

/// One admitted owner from its declaration's bounded file read.
fn admit(declaration: &SwitchDeclaration) -> Result<AdmittedOwner, SwitchAdmissionError> {
    let path = &declaration.state_file;
    let fail = |reason: String| SwitchAdmissionError {
        tag: declaration.tag.clone(),
        path: path.clone(),
        reason,
    };
    if path.file_name().is_none() {
        return Err(fail("state file must name a file".to_owned()));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    // Every existing ancestor must be a real directory: a symlink component
    // or non-directory ancestor rejects owner admission outright.
    let parent_identity = match walk_parents(parent, &declaration.tag, path)? {
        ParentState::Present(identity) => identity,
        ParentState::Missing => None,
    };

    let (value, fingerprint) = match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(fail("state file is not a regular file".to_owned()));
            }
            let mut file = std::fs::File::open(path)
                .map_err(|error| fail(format!("cannot read state file: {error}")))?;
            let mut raw = String::new();
            let mut limited = (&mut file).take(MAX_STATE_FILE_BYTES as u64 + 1);
            io::Read::read_to_string(&mut limited, &mut raw)
                .map_err(|error| fail(format!("cannot read state file: {error}")))?;
            if raw.len() > MAX_STATE_FILE_BYTES {
                return Err(fail(format!(
                    "state file exceeds the {} byte bound",
                    MAX_STATE_FILE_BYTES
                )));
            }
            // The legacy startup rule trims surrounding whitespace; the POST
            // path stores exact bytes, so an immediate show and a restart can
            // legitimately differ.
            (
                raw.trim().to_owned(),
                SwitchFileFingerprint::of_bytes(file_identity(&metadata), raw.as_bytes()),
            )
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (String::new(), SwitchFileFingerprint::missing())
        }
        Err(error) => {
            return Err(fail(format!("cannot inspect state file: {error}")));
        }
    };

    let (writable, reason) = match parent_identity {
        Some(_) => (true, None),
        None => (
            false,
            Some(format!(
                "state directory `{}` does not exist; create it and restart or rebind to \
                 re-evaluate write eligibility",
                parent.display()
            )),
        ),
    };
    Ok(AdmittedOwner {
        value,
        fingerprint,
        parent_identity,
        writable,
        reason,
    })
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_identity(_metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn directory_identity(_path: &Path) -> io::Result<(u64, u64)> {
    Ok((0, 0))
}

// ---------------------------------------------------------------------------
// Pinned state-directory operations
// ---------------------------------------------------------------------------

/// The state directory held open for one durable commit. On Linux the
/// temporary file and rename are issued relative to this directory's file
/// descriptor, so a concurrent path swap cannot redirect the replacement
/// outside the captured identity. Other platforms fall back to path-based
/// operations guarded by the same identity checks.
struct PinnedDirectory {
    #[cfg(target_os = "linux")]
    directory: std::os::fd::OwnedFd,
    #[cfg(not(target_os = "linux"))]
    directory: std::fs::File,
    /// `(device, inode)` of the open directory, when the platform exposes it.
    identity: Option<(u64, u64)>,
    /// The directory path used by the non-descriptor fallback. Unused on
    /// Linux, where every operation is issued relative to the descriptor.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    root: PathBuf,
}

impl PinnedDirectory {
    fn open(parent: &Path) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsFd as _;
            let directory = rustix::fs::open(
                parent,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )?;
            let identity = rustix::fs::fstat(directory.as_fd())
                .ok()
                .map(|stat| (stat.st_dev, stat.st_ino));
            Ok(Self {
                directory,
                identity,
                root: parent.to_path_buf(),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let directory = std::fs::File::open(parent)?;
            let identity = directory_identity(parent).ok();
            Ok(Self {
                directory,
                identity,
                root: parent.to_path_buf(),
            })
        }
    }

    fn identity(&self) -> Option<(u64, u64)> {
        self.identity
    }

    /// The current fingerprint of the target, `missing` when it does not
    /// exist. A symlink or non-regular target is an error, never a match.
    fn target_fingerprint(&self, name: &str) -> io::Result<SwitchFileFingerprint> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsFd as _;
            match rustix::fs::statat(
                self.directory.as_fd(),
                name,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(stat) => {
                    if stat.st_mode & 0o170_000 != 0o100_000 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "state file is not a regular file",
                        ));
                    }
                    let bytes = self.read_target(name)?;
                    Ok(SwitchFileFingerprint::of_bytes(
                        Some((stat.st_dev, stat.st_ino)),
                        &bytes,
                    ))
                }
                Err(rustix::io::Errno::NOENT) => Ok(SwitchFileFingerprint::missing()),
                Err(error) => Err(io::Error::other(error.to_string())),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let path = self.root.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_file() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "state file is not a regular file",
                        ));
                    }
                    let mut file = std::fs::File::open(&path)?;
                    let mut raw = Vec::new();
                    let mut limited = (&mut file).take(MAX_STATE_FILE_BYTES as u64 + 1);
                    io::Read::read_to_end(&mut limited, &mut raw)?;
                    Ok(SwitchFileFingerprint::of_bytes(
                        file_identity(&metadata),
                        &raw,
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Ok(SwitchFileFingerprint::missing())
                }
                Err(error) => Err(error),
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn read_target(&self, name: &str) -> io::Result<Vec<u8>> {
        use std::os::fd::AsFd as _;
        let file = rustix::fs::openat(
            self.directory.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?;
        let mut file = std::fs::File::from(file);
        let mut raw = Vec::new();
        let mut limited = (&mut file).take(MAX_STATE_FILE_BYTES as u64 + 1);
        io::Read::read_to_end(&mut limited, &mut raw)?;
        Ok(raw)
    }

    /// Writes the replacement bytes to an exclusive same-directory temporary
    /// file, syncs it, and returns the file's identity. The rename that
    /// installs this file preserves the inode, so the returned identity is
    /// the committed file's identity.
    fn write_replacement(
        &self,
        temp: &str,
        bytes: &[u8],
        gates: CommitGates,
    ) -> io::Result<Option<(u64, u64)>> {
        #[cfg(target_os = "linux")]
        let identity = {
            use std::os::fd::AsFd as _;
            let file = rustix::fs::openat(
                self.directory.as_fd(),
                temp,
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::EXCL
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::from_bits(0o644).unwrap_or_else(rustix::fs::Mode::empty),
            )?;
            let mut file = std::fs::File::from(file);
            if gates.fault_is(SwitchCommitFault::WriteTemp) {
                io::Write::write_all(&mut file, b"partial")?;
                return Err(io::Error::other("injected temporary-write failure"));
            }
            io::Write::write_all(&mut file, bytes)?;
            file.sync_all()?;
            file_identity(&file.metadata()?)
        };
        #[cfg(not(target_os = "linux"))]
        let identity = {
            let path = self.root.join(temp);
            let mut file = std::fs::File::create(&path)?;
            if gates.fault_is(SwitchCommitFault::WriteTemp) {
                io::Write::write_all(&mut file, b"partial")?;
                return Err(io::Error::other("injected temporary-write failure"));
            }
            io::Write::write_all(&mut file, bytes)?;
            file.sync_all()?;
            file_identity(&file.metadata()?)
        };
        if gates.crashes_at(SwitchCommitBoundary::TempSynced) {
            std::process::abort();
        }
        Ok(identity)
    }

    /// The final rename and directory sync. A directory-sync failure after a
    /// successful rename is a fatal ambiguity, not an ordinary I/O error.
    fn finalize_replacement(
        &self,
        temp: &str,
        name: &str,
        gates: CommitGates,
    ) -> Result<(), DurableError> {
        if gates.fault_is(SwitchCommitFault::Rename) {
            return Err(DurableError::Io("injected final rename failure".to_owned()));
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat(self.directory.as_fd(), temp, self.directory.as_fd(), name)
                .map_err(|error| {
                    DurableError::Io(format!("cannot rename state replacement: {error}"))
                })?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = &self.directory;
            std::fs::rename(self.root.join(temp), self.root.join(name)).map_err(|error| {
                DurableError::Io(format!("cannot rename state replacement: {error}"))
            })?;
        }
        if gates.crashes_at(SwitchCommitBoundary::Renamed) {
            std::process::abort();
        }
        if gates.fault_is(SwitchCommitFault::DirSync) {
            return Err(DurableError::Ambiguous);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::fsync(self.directory.as_fd()).map_err(|_| DurableError::Ambiguous)?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.directory
                .sync_all()
                .map_err(|_| DurableError::Ambiguous)?;
        }
        Ok(())
    }

    fn remove_temporary(&self, temp: &str) {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsFd as _;
            let _ =
                rustix::fs::unlinkat(self.directory.as_fd(), temp, rustix::fs::AtFlags::empty());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = std::fs::remove_file(self.root.join(temp));
        }
    }
}

fn temp_name(name: &str) -> String {
    format!(".{name}.tmp-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::{
        CommitGates, MAX_ACCEPTED_MUTATIONS, MAX_STATE_FILE_BYTES, SwitchCommitBoundary,
        SwitchCommitFault, SwitchRegistry,
    };
    use crate::managed::PersistGate;
    use crate::switch::SwitchDeclaration;

    use std::path::{Path, PathBuf};
    use std::time::Duration;

    struct Root {
        path: PathBuf,
    }

    impl Root {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("switch-state-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("test root");
            Self { path }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn declaration(type_number: u8, tag: &str, state_file: &Path) -> SwitchDeclaration {
        SwitchDeclaration {
            type_number,
            tag: tag.to_owned(),
            state_file: state_file.to_path_buf(),
            source_path: "$.plugins[switch].args".to_owned(),
        }
    }

    fn build_error(declarations: &[SwitchDeclaration]) -> super::SwitchAdmissionError {
        match SwitchRegistry::build(declarations) {
            Ok(_) => panic!("admission must be rejected"),
            Err(error) => error,
        }
    }

    fn post_error(registry: &SwitchRegistry, tag: &str, value: &str) -> super::SwitchMutationError {
        match registry.post(tag, value.to_owned()) {
            Ok(_) => panic!("mutation must be rejected"),
            Err(error) => error,
        }
    }

    fn run_async<F: Future>(future: F) -> F::Output {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        tokio::task::LocalSet::new().block_on(&runtime, future)
    }

    async fn post(
        registry: &SwitchRegistry,
        tag: &str,
        value: &str,
    ) -> Result<(), super::SwitchMutationError> {
        registry
            .post(tag, value.to_owned())
            .expect("mutation is accepted")
            .complete()
            .await
    }

    async fn wait_until_gate_arrived(gate: &PersistGate) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !gate.arrived() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "held commit never reached the gate"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[test]
    fn missing_state_file_starts_empty_and_creates_nothing() {
        let root = Root::new("missing");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &root.path("state1.txt"))])
            .expect("missing file admits");
        assert_eq!(registry.show("sw1").as_deref(), Some(""));
        assert!(!root.path("state1.txt").exists());
    }

    #[test]
    fn startup_trims_surrounding_whitespace_but_post_stores_exact_bytes() {
        let root = Root::new("trim");
        let file = root.path("state1.txt");
        std::fs::write(&file, "  B \n\t").expect("initial file");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        assert_eq!(registry.show("sw1").as_deref(), Some("B"));

        run_async(async {
            post(&registry, "sw1", "  exact value \n")
                .await
                .expect("commit");
        });
        assert_eq!(registry.show("sw1").as_deref(), Some("  exact value \n"));
        assert_eq!(
            std::fs::read(&file).expect("file"),
            b"  exact value \n".to_vec()
        );

        // The legacy restart trim applies again on the exact POSTed bytes.
        let restarted = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        assert_eq!(restarted.show("sw1").as_deref(), Some("exact value"));
    }

    #[test]
    fn malformed_existing_state_files_fail_admission_without_touching_them() {
        let root = Root::new("malformed");
        let invalid = root.path("invalid.txt");
        std::fs::write(&invalid, [0xFF_u8, 0xFE, 0x00, 0x01]).expect("invalid utf8");
        let error = build_error(&[declaration(1, "sw1", &invalid)]);
        assert!(error.reason.contains("read"), "{error:?}");
        assert_eq!(
            std::fs::read(&invalid).expect("unchanged"),
            [0xFF_u8, 0xFE, 0x00, 0x01]
        );

        let oversize = root.path("oversize.txt");
        let huge = vec![b'A'; MAX_STATE_FILE_BYTES + 1];
        std::fs::write(&oversize, &huge).expect("oversize");
        let error = build_error(&[declaration(1, "sw1", &oversize)]);
        assert!(error.reason.contains("exceeds"), "{error:?}");
        assert_eq!(std::fs::read(&oversize).expect("unchanged"), huge);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unreadable = root.path("unreadable.txt");
            std::fs::write(&unreadable, b"secret").expect("unreadable");
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))
                .expect("chmod");
            // A privileged test process can read anything; calibrate on a
            // control file so the subcase only asserts where the OS refuses.
            let control = root.path("control.txt");
            std::fs::write(&control, b"control").expect("control");
            std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o000))
                .expect("control chmod");
            if std::fs::File::open(&control).is_err() {
                let error = build_error(&[declaration(1, "sw1", &unreadable)]);
                assert!(error.reason.contains("read"), "{error:?}");
            } else {
                eprintln!("skipping unreadable-state-file subcase: running privileged");
            }
            std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o644))
                .expect("control restore");
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644))
                .expect("restore");
            assert_eq!(std::fs::read(&unreadable).expect("unchanged"), b"secret");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_nonregular_state_files_reject_admission() {
        let root = Root::new("symlinks");
        let target = root.path("target.txt");
        std::fs::write(&target, b"A").expect("target");
        let link = root.path("link.txt");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = build_error(&[declaration(1, "sw1", &link)]);
        assert!(error.reason.contains("regular"), "{error:?}");

        let linked_dir = root.path("linked_dir");
        std::os::unix::fs::symlink(&root.path, &linked_dir).expect("symlink dir");
        let error = build_error(&[declaration(1, "sw1", &linked_dir.join("state.txt"))]);
        assert!(error.reason.contains("not a directory"), "{error:?}");

        let error = build_error(&[declaration(1, "sw1", Path::new("/dev/null"))]);
        assert!(error.reason.contains("regular"), "{error:?}");
    }

    #[test]
    fn first_post_creates_the_state_file_when_the_parent_exists() {
        let root = Root::new("first-write");
        let file = root.path("state1.txt");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        run_async(async {
            post(&registry, "sw1", "A").await.expect("commit");
        });
        assert_eq!(std::fs::read(&file).expect("created"), b"A".to_vec());
        // No temporary file survives a successful replacement.
        let leftovers: Vec<_> = std::fs::read_dir(&root.path)
            .expect("dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
    }

    #[test]
    fn missing_parent_is_read_only_until_a_restart_or_rebind() {
        let root = Root::new("readonly");
        let file = root.path("missing_dir/state1.txt");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        let error = post_error(&registry, "sw1", "A");
        match error {
            super::SwitchMutationError::ReadOnly { reason, .. } => {
                assert!(reason.contains("restart or rebind"), "{reason}");
            }
            other => panic!("expected ReadOnly, got {other:?}"),
        }

        // Creating the directory now does not refresh the cached eligibility.
        std::fs::create_dir_all(root.path("missing_dir")).expect("dir");
        let error = post_error(&registry, "sw1", "A");
        assert!(matches!(error, super::SwitchMutationError::ReadOnly { .. }));

        // A restart re-evaluates and the write succeeds.
        let restarted = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        run_async(async {
            post(&restarted, "sw1", "A").await.expect("commit");
        });
        assert_eq!(std::fs::read(&file).expect("file"), b"A".to_vec());
    }

    #[test]
    fn post_stores_exact_arbitrary_utf8_values() {
        let root = Root::new("exact");
        let file = root.path("state1.txt");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        for value in ["", "B", "multi\nline", "带空格 的值", "  spaced  "] {
            run_async(async {
                post(&registry, "sw1", value).await.expect("commit");
            });
            assert_eq!(
                registry.show("sw1").as_deref(),
                Some(value),
                "committed value for {value:?}"
            );
            assert_eq!(
                std::fs::read(&file).expect("file"),
                value.as_bytes().to_vec(),
                "file bytes for {value:?}"
            );
        }
    }

    #[test]
    fn oversized_posted_values_are_rejected_before_disk_work() {
        let root = Root::new("too-large");
        let file = root.path("state1.txt");
        std::fs::write(&file, b"old").expect("old");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        let huge = "A".repeat(MAX_STATE_FILE_BYTES + 1);
        let error = post_error(&registry, "sw1", &huge);
        assert!(matches!(
            error,
            super::SwitchMutationError::ValueTooLarge { .. }
        ));
        assert_eq!(std::fs::read(&file).expect("unchanged"), b"old");
    }

    #[test]
    fn precommit_faults_preserve_the_old_state_and_clean_the_temporary() {
        for fault in [SwitchCommitFault::WriteTemp, SwitchCommitFault::Rename] {
            let root = Root::new("fault");
            let file = root.path("state1.txt");
            std::fs::write(&file, b"old").expect("old");
            let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
            registry.inject_fault(fault);
            let error = run_async(async {
                registry
                    .post("sw1", "new".to_owned())
                    .expect("fault is accepted")
                    .complete()
                    .await
                    .expect_err("injected fault must fail")
            });
            assert!(
                matches!(error, super::SwitchMutationError::Io { .. }),
                "{fault:?} -> {error:?}"
            );
            assert_eq!(std::fs::read(&file).expect("old file"), b"old");
            assert_eq!(registry.show("sw1").as_deref(), Some("old"));
            let leftovers: Vec<_> = std::fs::read_dir(&root.path)
                .expect("dir")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
                .collect();
            assert!(leftovers.is_empty(), "{fault:?} left: {leftovers:?}");
        }
    }

    #[test]
    fn externally_changed_state_files_conflict_without_replacement() {
        let root = Root::new("conflict");
        let file = root.path("state1.txt");
        std::fs::write(&file, b"committed").expect("initial");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");

        // An external content edit is observed and refused.
        std::fs::write(&file, b"external").expect("external edit");
        let error = run_async(async {
            registry
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await
                .expect_err("external edit must conflict")
        });
        match error {
            super::SwitchMutationError::Conflict { reason, .. } => {
                assert!(reason.contains("changed"), "{reason}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(std::fs::read(&file).expect("external kept"), b"external");
        assert_eq!(registry.show("sw1").as_deref(), Some("committed"));

        // No unconditional overwrite after an observed conflict.
        let error = run_async(async {
            registry
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await
                .expect_err("second write must still conflict")
        });
        assert!(matches!(error, super::SwitchMutationError::Conflict { .. }));
        assert_eq!(std::fs::read(&file).expect("external kept"), b"external");

        // A replacement with identical content but a new identity conflicts too.
        std::fs::remove_file(&file).expect("remove");
        std::fs::write(&file, b"committed").expect("recreated");
        let restarted = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        std::fs::remove_file(&file).expect("remove again");
        std::fs::write(&file, b"committed").expect("recreated again");
        let error = run_async(async {
            restarted
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await
                .expect_err("identity change must conflict")
        });
        assert!(matches!(error, super::SwitchMutationError::Conflict { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn a_swapped_parent_directory_is_detected_and_never_written_through() {
        let root = Root::new("parent-swap");
        let dir = root.path("dir");
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("state1.txt");
        std::fs::write(&file, b"committed").expect("initial");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");

        // Swap the directory identity at the captured path.
        std::fs::rename(&dir, root.path("dir-real")).expect("move");
        std::fs::create_dir_all(&dir).expect("decoy dir");
        std::fs::write(dir.join("state1.txt"), b"decoy").expect("decoy");

        let error = run_async(async {
            registry
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await
                .expect_err("parent swap must conflict")
        });
        match error {
            super::SwitchMutationError::Conflict { reason, .. } => {
                assert!(reason.contains("parent"), "{reason}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(root.path("dir-real/state1.txt")).expect("original kept"),
            b"committed"
        );
        assert_eq!(
            std::fs::read(dir.join("state1.txt")).expect("decoy untouched"),
            b"decoy"
        );
    }

    #[test]
    fn hardlinked_owners_reject_admission() {
        let root = Root::new("hardlink");
        std::fs::write(root.path("a.txt"), b"A").expect("a");
        std::fs::hard_link(root.path("a.txt"), root.path("b.txt")).expect("hard link");
        let error = build_error(&[
            declaration(1, "sw1", &root.path("a.txt")),
            declaration(2, "sw2", &root.path("b.txt")),
        ]);
        assert!(error.reason.contains("same underlying file"), "{error:?}");
    }

    #[test]
    fn concurrency_slots_are_bounded_per_owner_and_globally() {
        let root = Root::new("slots");
        let declarations: Vec<_> = (1..=5)
            .map(|n| {
                let name = format!("state{n}.txt");
                declaration(n, &format!("sw{n}"), &root.path(&name))
            })
            .collect();
        let registry = SwitchRegistry::build(&declarations).expect("admit");
        let gate = PersistGate::new();
        registry.inject_commit_hold(gate.clone());

        let mut tickets = Vec::new();
        run_async(async {
            // A second mutation on the same owner is busy while the first is
            // parked in the I/O gate.
            tickets.push(
                registry
                    .post("sw1", "v1".to_owned())
                    .expect("first mutation is accepted"),
            );
            assert!(matches!(
                registry.post("sw1", "again".to_owned()),
                Err(super::SwitchMutationError::Busy { .. })
            ));
            for n in 2..=4 {
                tickets.push(
                    registry
                        .post(&format!("sw{n}"), format!("v{n}"))
                        .expect("four concurrent mutations are accepted"),
                );
            }
            // The fifth global mutation is overloaded.
            assert!(matches!(
                registry.post("sw5", "v5".to_owned()),
                Err(super::SwitchMutationError::Overloaded)
            ));
            wait_until_gate_arrived(&gate).await;
            gate.release();
            for ticket in tickets {
                ticket.complete().await.expect("held commit completes");
            }
        });
        for n in 1..=4 {
            assert_eq!(
                std::fs::read(root.path(&format!("state{n}.txt"))).expect("file"),
                format!("v{n}").into_bytes()
            );
        }
        assert_eq!(MAX_ACCEPTED_MUTATIONS, 4);
    }

    #[test]
    fn dropping_the_request_future_does_not_cancel_accepted_work() {
        let root = Root::new("dropped");
        let file = root.path("state1.txt");
        std::fs::write(&file, b"old").expect("old");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        run_async(async {
            // Drop the ticket immediately after acceptance.
            drop(registry.post("sw1", "new".to_owned()).expect("accepted"));
            registry.drain().await;
        });
        assert_eq!(std::fs::read(&file).expect("committed"), b"new");
        assert_eq!(registry.show("sw1").as_deref(), Some("new"));
    }

    #[test]
    fn shutdown_stops_admission_and_drains_accepted_work() {
        let root = Root::new("shutdown");
        let file = root.path("state1.txt");
        std::fs::write(&file, b"old").expect("old");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        let gate = PersistGate::new();
        registry.inject_commit_hold(gate.clone());

        run_async(async {
            let ticket = registry.post("sw1", "new".to_owned()).expect("accepted");
            wait_until_gate_arrived(&gate).await;
            registry.close();
            assert!(matches!(
                registry.post("sw1", "later".to_owned()),
                Err(super::SwitchMutationError::Closed)
            ));
            gate.release();
            ticket.complete().await.expect("accepted work completes");
            registry.drain().await;
        });
        assert_eq!(std::fs::read(&file).expect("committed"), b"new");
        assert_eq!(registry.show("sw1").as_deref(), Some("new"));
    }

    #[test]
    fn post_rename_ambiguity_is_fatal_and_preserves_the_final_file() {
        let root = Root::new("ambiguous");
        let file = root.path("state1.txt");
        std::fs::write(&file, b"old").expect("old");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        registry.inject_fault(SwitchCommitFault::DirSync);

        let error = run_async(async {
            registry
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await
                .expect_err("directory sync failure is fatal")
        });
        assert!(
            matches!(error, super::SwitchMutationError::RecoveryRequired),
            "{error:?}"
        );
        assert!(registry.recovery_required());
        // The final file keeps the new, complete content; memory never claims
        // a rollback and further mutations are fenced.
        assert_eq!(std::fs::read(&file).expect("final file"), b"new");
        assert_eq!(registry.show("sw1").as_deref(), Some("old"));
        let error = post_error(&registry, "sw1", "again");
        assert!(matches!(
            error,
            super::SwitchMutationError::RecoveryRequired
        ));
        // A restart honestly reads whichever complete file was retained.
        let restarted = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        assert_eq!(restarted.show("sw1").as_deref(), Some("new"));
    }

    #[test]
    fn admission_facts_publish_the_new_aggregate_after_a_commit() {
        let root = Root::new("facts");
        let registry = SwitchRegistry::build(&[
            declaration(1, "sw1", &root.path("state1.txt")),
            declaration(2, "sw2", &root.path("state2.txt")),
        ])
        .expect("admit");
        let before = registry.admission_facts();
        assert_eq!(before.get(&0).map(std::ops::Deref::deref), Some(""));
        run_async(async {
            post(&registry, "sw1", "A").await.expect("commit");
        });
        let after = registry.admission_facts();
        assert_eq!(after.get(&0).map(std::ops::Deref::deref), Some("A"));
        assert_eq!(after.get(&1).map(std::ops::Deref::deref), Some(""));
        // The previous aggregate stays immutable for already-admitted queries.
        assert_eq!(before.get(&0).map(std::ops::Deref::deref), Some(""));
    }

    // -----------------------------------------------------------------------
    // Subprocess crash durability
    // -----------------------------------------------------------------------

    #[test]
    #[ignore = "driven by crash_boundaries_are_recoverable"]
    fn switch_crash_child() {
        let root = PathBuf::from(std::env::var("MOSDNS_TEST_SWITCH_CRASH_ROOT").expect("root"));
        let boundary = std::env::var("MOSDNS_TEST_SWITCH_CRASH_BOUNDARY").expect("boundary");
        let boundary = match boundary.as_str() {
            "after_temp_sync" => SwitchCommitBoundary::TempSynced,
            "after_rename" => SwitchCommitBoundary::Renamed,
            "after_commit" => SwitchCommitBoundary::Committed,
            other => panic!("unknown boundary {other}"),
        };
        std::fs::write(root.join("started"), b"").expect("started marker");
        let file = root.join("state1.txt");
        let registry = SwitchRegistry::build(&[declaration(1, "sw1", &file)]).expect("admit");
        registry.inject_crash_after(boundary);
        run_async(async {
            let result = registry
                .post("sw1", "new".to_owned())
                .expect("accepted")
                .complete()
                .await;
            // The after-commit boundary aborts before this line; the earlier
            // boundaries abort inside the I/O step.
            let _ = result;
        });
    }

    #[test]
    fn crash_boundaries_are_recoverable() {
        for (boundary, expected_file, expected_value) in [
            ("after_temp_sync", "old", "old"),
            ("after_rename", "new", "new"),
            ("after_commit", "new", "new"),
        ] {
            let root = std::env::temp_dir()
                .join(format!("switch-crash-{}-{boundary}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("crash root");
            std::fs::write(root.join("state1.txt"), b"old").expect("old");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "switch_state::tests::switch_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MOSDNS_TEST_SWITCH_CRASH_ROOT", &root)
                .env("MOSDNS_TEST_SWITCH_CRASH_BOUNDARY", boundary)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("crash child");
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                match child.try_wait().expect("child status") {
                    Some(status) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            assert_eq!(
                                status.signal(),
                                Some(6),
                                "{boundary}: child must abort, status {status:?}"
                            );
                        }
                        #[cfg(not(unix))]
                        {
                            assert!(!status.success(), "{boundary}: child must fail");
                        }
                        break;
                    }
                    None if std::time::Instant::now() >= deadline => {
                        child.kill().expect("kill");
                        child.wait().expect("wait");
                        panic!("{boundary}: crash child never aborted");
                    }
                    None => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            // The file is complete old or complete new; never torn.
            assert_eq!(
                std::fs::read(root.join("state1.txt")).expect("final file"),
                expected_file.as_bytes().to_vec(),
                "{boundary}"
            );
            // Restart loads the retained file before readiness.
            let restarted =
                SwitchRegistry::build(&[declaration(1, "sw1", &root.join("state1.txt"))])
                    .expect("restart admits the retained file");
            assert_eq!(
                restarted.show("sw1").as_deref(),
                Some(expected_value),
                "{boundary}"
            );
            // A fresh write on the restarted registry still works.
            run_async(async {
                post(&restarted, "sw1", "after-crash")
                    .await
                    .expect("commit");
            });
            assert_eq!(
                std::fs::read(root.join("state1.txt")).expect("file"),
                b"after-crash".to_vec()
            );
            std::fs::remove_dir_all(&root).expect("cleanup");
        }
    }

    #[test]
    fn simultaneous_different_owner_commits_compose_in_the_aggregate() {
        let root = Root::new("concurrent-facts");
        let registry = SwitchRegistry::build(&[
            declaration(1, "sw1", &root.path("s1.txt")),
            declaration(2, "sw2", &root.path("s2.txt")),
        ])
        .expect("admit");
        let gate = PersistGate::new();
        registry.inject_commit_hold(gate.clone());

        run_async(async {
            // Both owners mutate at the same time; the publications land in
            // whichever order the I/O worker finishes them.
            let first = registry
                .post("sw1", "A".to_owned())
                .expect("first accepted");
            let second = registry
                .post("sw2", "Y".to_owned())
                .expect("second accepted");
            wait_until_gate_arrived(&gate).await;
            gate.release();
            first.complete().await.expect("first commit");
            second.complete().await.expect("second commit");
        });
        // Both committed values survive: the aggregate is derived from the
        // latest committed owner set, never from a captured map.
        let facts = registry.admission_facts();
        assert_eq!(facts.get(&0).map(std::ops::Deref::deref), Some("A"));
        assert_eq!(facts.get(&1).map(std::ops::Deref::deref), Some("Y"));
        // The A fast bit for switch1 (bit 32) is seeded together with the
        // value, in the same derivation.
        let seed = registry.admission_seed();
        assert_eq!(seed.a_bits, 1_u64 << 32);
    }

    #[test]
    fn a_captured_admission_seed_is_immutable_across_later_publications() {
        let root = Root::new("seed-immutable");
        let registry =
            SwitchRegistry::build(&[declaration(1, "sw1", &root.path("s1.txt"))]).expect("admit");
        let seed = registry.admission_seed();
        let captured = seed.facts.clone();
        run_async(async {
            post(&registry, "sw1", "B").await.expect("commit");
        });
        // The already-captured seed observes its own immutable snapshot; a
        // later POST only publishes a fresh aggregate for future admissions.
        assert_eq!(seed.facts.get(&0).map(std::ops::Deref::deref), Some(""));
        assert!(std::sync::Arc::ptr_eq(&seed.facts, &captured));
        assert_eq!(
            registry
                .admission_facts()
                .get(&0)
                .map(std::ops::Deref::deref),
            Some("B")
        );
    }

    #[test]
    fn a_to_non_a_and_non_a_to_a_flip_the_aggregate_and_fast_bits() {
        let root = Root::new("a-flips");
        let registry =
            SwitchRegistry::build(&[declaration(1, "sw1", &root.path("s1.txt"))]).expect("admit");
        run_async(async {
            post(&registry, "sw1", "A").await.expect("commit");
        });
        assert_eq!(registry.admission_seed().a_bits, 1_u64 << 32);
        run_async(async {
            post(&registry, "sw1", "B").await.expect("commit");
        });
        assert_eq!(
            registry.admission_seed().a_bits,
            0,
            "the A bit must clear when the committed value is no longer A"
        );
        run_async(async {
            post(&registry, "sw1", "A").await.expect("commit");
        });
        assert_eq!(registry.admission_seed().a_bits, 1_u64 << 32);
    }

    #[test]
    fn runtime_rebind_refreshes_write_eligibility_from_the_filesystem() {
        let root = Root::new("rebind-eligibility");
        let file = root.path("missing_dir/s1.txt");
        let declarations = [declaration(1, "sw1", &file)];
        let registry = SwitchRegistry::build(&declarations).expect("admit");
        let before = run_async_post_error(&registry, "sw1", "A");
        assert!(matches!(
            before,
            super::SwitchMutationError::ReadOnly { .. }
        ));

        // Creating the parent directory plus a runtime rebind re-evaluates
        // eligibility without disturbing the committed value.
        std::fs::create_dir_all(root.path("missing_dir")).expect("dir");
        let rebind = SwitchRegistry::rebind(Some(&registry), &declarations).expect("rebind");
        run_async(async {
            post(&rebind, "sw1", "A")
                .await
                .expect("rebind refreshes eligibility");
        });
        assert_eq!(std::fs::read(&file).expect("file"), b"A".to_vec());
    }

    #[test]
    fn failed_candidate_rebind_does_not_mutate_the_committed_owner() {
        let root = Root::new("rebind-transaction");
        let unchanged = root.path("missing_dir/s1.txt");
        let declarations = [declaration(1, "sw1", &unchanged)];
        let registry = SwitchRegistry::build(&declarations).expect("admit");
        assert!(matches!(
            post_error(&registry, "sw1", "A"),
            super::SwitchMutationError::ReadOnly { .. }
        ));

        std::fs::create_dir_all(root.path("missing_dir")).expect("parent");
        let invalid = root.path("invalid");
        std::fs::create_dir_all(&invalid).expect("invalid directory");
        let candidate = SwitchRegistry::rebind(
            Some(&registry),
            &[
                declaration(1, "sw1", &unchanged),
                declaration(2, "sw2", &invalid),
            ],
        )
        .expect("candidate construction");
        assert!(candidate.inner.owners.borrow()[0].writable.get());
        assert!(candidate.complete_pending_admissions().is_err());

        // The failed candidate must not make the committed view writable.
        assert!(!registry.inner.owners.borrow()[0].writable.get());
        assert!(matches!(
            post_error(&registry, "sw1", "A"),
            super::SwitchMutationError::ReadOnly { .. }
        ));
    }

    fn run_async_post_error(
        registry: &SwitchRegistry,
        tag: &str,
        value: &str,
    ) -> super::SwitchMutationError {
        match registry.post(tag, value.to_owned()) {
            Ok(_) => panic!("mutation must be rejected"),
            Err(error) => error,
        }
    }

    #[test]
    fn sequential_mutation_traffic_keeps_tracked_task_storage_bounded() {
        let root = Root::new("task-reaping");
        let registry =
            SwitchRegistry::build(&[declaration(1, "sw1", &root.path("s1.txt"))]).expect("admit");
        run_async(async {
            for round in 0..100 {
                post(&registry, "sw1", &format!("v{round}"))
                    .await
                    .expect("commit");
            }
        });
        // Completed owned-task records are reaped as new work arrives; at
        // most the still-live tail remains.
        let tracked = registry.inner.tasks.borrow().len();
        assert!(
            tracked <= MAX_ACCEPTED_MUTATIONS,
            "task storage grew to {tracked}"
        );
        run_async(async {
            registry.drain().await;
        });
        assert_eq!(
            registry.inner.tasks.borrow().len(),
            0,
            "drain reaps every finished task"
        );
    }

    #[test]
    fn gates_default_to_inert() {
        let gates = CommitGates::default();
        assert!(!gates.fault_is(SwitchCommitFault::WriteTemp));
        assert!(!gates.crashes_at(SwitchCommitBoundary::Committed));
    }
}
