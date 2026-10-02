use std::cell::Cell;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    DispatchMetadata, ExecutableId, ExecutableTarget, ExecutionState, ExecutorError,
    ExecutorOutcome, MatcherError, SequenceId, ValidatedExecutable, ValidatedProgram,
    ValidatedRule,
};

const QTYPE_SOA: u16 = 6;
const QTYPE_PTR: u16 = 12;
const QTYPE_AAAA: u16 = 28;
const QTYPE_HTTPS: u16 = 65;

/// Cooperative cancellation state shared by one root execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationState {
    Active,
    Cancelled,
}

/// A cloneable cancellation signal shared by one root execution and all of
/// its nested scopes. Cancellation is observed at the next matcher or
/// executable dispatch boundary; it does not interrupt a fixture in the
/// middle of one pure call.
#[derive(Debug)]
struct CancellationInner {
    cancelled: AtomicBool,
    parents: Vec<Arc<CancellationInner>>,
}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                parents: Vec::new(),
            }),
        }
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
            || self
                .inner
                .parents
                .iter()
                .any(|parent| cancellation_inner_is_cancelled(parent))
    }

    fn with_parents(local: Self, parent: Self) -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                parents: vec![local.inner, parent.inner],
            }),
        }
    }
}

fn cancellation_inner_is_cancelled(inner: &CancellationInner) -> bool {
    inner.cancelled.load(Ordering::SeqCst)
        || inner
            .parents
            .iter()
            .any(|parent| cancellation_inner_is_cancelled(parent))
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// A current-thread root fuel counter shared by all native branch controls.
/// Cloning this handle shares the counter; it never copies the allowance.
#[derive(Clone, Debug)]
pub struct RootFuelHandle {
    remaining: Rc<Cell<u64>>,
}

impl RootFuelHandle {
    #[must_use]
    pub fn new(remaining: u64) -> Self {
        Self {
            remaining: Rc::new(Cell::new(remaining)),
        }
    }

    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.remaining.get()
    }
}

/// Fuel and cancellation owned by one root invocation and shared by nested
/// `try` scopes. Every matcher or executable dispatch checks cancellation
/// before consuming one fuel unit.
#[derive(Clone, Debug)]
pub struct ExecutionControl {
    pub remaining_fuel: u64,
    pub cancellation: CancellationState,
    cancellation_token: CancellationToken,
    shared_budget: Option<RootFuelHandle>,
}

impl ExecutionControl {
    #[must_use]
    pub fn with_fuel(remaining_fuel: u64) -> Self {
        Self {
            remaining_fuel,
            cancellation: CancellationState::Active,
            cancellation_token: CancellationToken::new(),
            shared_budget: None,
        }
    }

    #[must_use]
    pub fn with_cancellation_token(
        remaining_fuel: u64,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            remaining_fuel,
            cancellation: CancellationState::Active,
            cancellation_token,
            shared_budget: None,
        }
    }

    /// Creates a control backed by a root-owned shared fuel allowance.
    /// Native branch drivers must use this constructor instead of cloning a
    /// legacy control, which would duplicate its local numeric field.
    #[must_use]
    pub fn with_shared_budget(
        shared_budget: RootFuelHandle,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            remaining_fuel: shared_budget.remaining(),
            cancellation: CancellationState::Active,
            cancellation_token,
            shared_budget: Some(shared_budget),
        }
    }

    /// Forks a child control with the same root budget and an independent
    /// cancellation source that remains subordinate to this control.
    #[must_use]
    pub fn fork_child(&self, child_cancellation: CancellationToken) -> Self {
        let shared_budget = self
            .shared_budget
            .clone()
            .unwrap_or_else(|| RootFuelHandle::new(self.remaining_fuel));
        Self::with_shared_budget(
            shared_budget,
            CancellationToken::with_parents(child_cancellation, self.cancellation_token.clone()),
        )
    }

    /// Returns the live shared allowance, or the legacy local field.
    #[must_use]
    pub fn remaining_budget(&self) -> u64 {
        self.shared_budget
            .as_ref()
            .map_or(self.remaining_fuel, RootFuelHandle::remaining)
    }

    /// Charges exactly one canonical matcher/executable dispatch.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::Cancelled`] when this control or one of its
    /// parents is cancelled, or [`ExecutionError::BudgetExceeded`] when the
    /// root shared allowance is exhausted.
    pub fn try_consume(&mut self) -> Result<(), ExecutionError> {
        if self.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        if let Some(shared_budget) = &self.shared_budget {
            let remaining = shared_budget.remaining();
            if remaining == 0 {
                return Err(ExecutionError::BudgetExceeded);
            }
            shared_budget.remaining.set(remaining - 1);
            return Ok(());
        }
        if self.remaining_fuel == 0 {
            return Err(ExecutionError::BudgetExceeded);
        }
        self.remaining_fuel -= 1;
        Ok(())
    }

    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    pub fn cancel(&mut self) {
        self.cancellation = CancellationState::Cancelled;
        self.cancellation_token.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.cancellation, CancellationState::Cancelled)
            || self.cancellation_token.is_cancelled()
    }
}

/// Normal root-level completion versus the typed `exit` signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionCompletion {
    Completed,
    Exited,
}

/// Errors that stop execution. `Exit` is deliberately represented by
/// [`ExecutionCompletion::Exited`] so that only `try` converts it to normal
/// continuation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionError {
    InvalidEntry(SequenceId),
    InvalidFixture(ExecutableId),
    ExternalDispatchUnsupported(ExecutableId),
    Matcher(MatcherError),
    Executor(ExecutorError),
    Cancelled,
    BudgetExceeded,
    WaitingForExternal(ExecutableId),
    InvalidResume {
        expected: ExecutableId,
        received: ExecutableId,
    },
    ResumeNotPending(ExecutableId),
    /// An enclosing-scope watch was requested with no live enclosing scope.
    ScopeWatchConflict(ExecutableId),
    /// A scope completion was resumed with a token other than the pending one.
    InvalidScopeResume {
        expected: WatchToken,
        received: WatchToken,
    },
    /// A scope completion was resumed while no watch notification was pending.
    NoPendingScopeCompletion,
    Finished,
}

/// A typed request for an executable that must be fulfilled by the host.
///
/// The request intentionally contains only stable catalog identity. Query and
/// response bytes remain in the machine's owned [`ExecutionState`] and in the
/// host's owned request buffer; no borrowed executor data crosses an await.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalDispatch {
    executable: ExecutableId,
}

impl ExternalDispatch {
    #[must_use]
    pub const fn executable(self) -> ExecutableId {
        self.executable
    }
}

/// The completion of one watched enclosing scope.
///
/// The watched scope is the scope that contained the executable registered
/// through [`ExecutionMachine::watch_enclosing_scope`]. It is reported at the
/// exact boundary where that scope stops running, before the caller's next
/// rule executes, so an owner can commit a result that the enclosure produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScopeCompletion {
    executable: ExecutableId,
    token: WatchToken,
}

impl ScopeCompletion {
    /// The executable whose enclosing scope completed.
    #[must_use]
    pub const fn executable(self) -> ExecutableId {
        self.executable
    }

    /// The exact watch this notification belongs to.
    ///
    /// A notification is only ever consumed by the owner that armed this token,
    /// so two frames for the same executable can never be confused even when
    /// their boundaries complete out of order.
    #[must_use]
    pub const fn token(self) -> WatchToken {
        self.token
    }
}

/// The opaque identity of one armed enclosing-scope watch.
///
/// Tokens are unique per machine and never reused, so an owner that arms
/// several watches for the same executable can still tell their notifications
/// apart. They are deliberately not derivable from an [`ExecutableId`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct WatchToken(u64);

/// The externally observable progress of one canonical sequence machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineStep {
    Dispatch(ExternalDispatch),
    /// The watched scope finished naturally. Its successor result is publishable.
    ScopeComplete(ScopeCompletion),
    /// The watched scope was unwound by `exit` while the machine kept running.
    /// The owner must invalidate the matching frame and must not publish it.
    ///
    /// This is *not* how cancellation, fuel exhaustion or a terminal executor
    /// error are reported. Those stop the machine: `step`/`resume` return
    /// [`ExecutionError`], every armed watch is dropped without a notification,
    /// and the owner must invalidate the frames it still holds when the drive
    /// ends in an error rather than wait for an abort notification.
    ScopeAborted(ScopeCompletion),
    Complete(ExecutionCompletion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MachineStatus {
    Running,
    Waiting(ExecutableId),
    ScopeCompleted(WatchToken, ExecutableId),
    Complete(ExecutionCompletion),
    Failed,
}

enum StateSlot<'a> {
    Owned(Box<ExecutionState>),
    Borrowed(&'a mut ExecutionState),
}

impl StateSlot<'_> {
    fn as_ref(&self) -> &ExecutionState {
        match self {
            Self::Owned(state) => state,
            Self::Borrowed(state) => state,
        }
    }

    fn as_mut(&mut self) -> &mut ExecutionState {
        match self {
            Self::Owned(state) => state,
            Self::Borrowed(state) => state,
        }
    }
}

enum ControlSlot<'a> {
    Owned(ExecutionControl),
    Borrowed(&'a mut ExecutionControl),
}

impl ControlSlot<'_> {
    fn as_ref(&self) -> &ExecutionControl {
        match self {
            Self::Owned(control) => control,
            Self::Borrowed(control) => control,
        }
    }

    fn as_mut(&mut self) -> &mut ExecutionControl {
        match self {
            Self::Owned(control) => control,
            Self::Borrowed(control) => control,
        }
    }
}

/// A stable identity for one live scope. Identities are never reused inside
/// one machine, so a scope completion can be attributed to the exact scope
/// that was running when a dispatch was observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScopeId(usize);

/// The machine's live scope stack plus the identity counter that makes
/// per-scope observations unambiguous.
#[derive(Clone, Default)]
struct ScopeStack {
    scopes: Vec<Scope>,
    next_id: usize,
    /// The named sequence whose rule most recently started executing, when
    /// that rule started during this machine drive. `None` means no new rule
    /// was entered, so the caller's previous origin still stands.
    rule_origin: Option<SequenceId>,
}

impl ScopeStack {
    fn push_sequence(&mut self, kind: ScopeKind, sequence: SequenceId) {
        let id = self.allocate_id();
        self.scopes.push(Scope::sequence(id, kind, sequence));
    }

    fn push_fixture(&mut self, kind: ScopeKind, fixture: ExecutableId) {
        let id = self.allocate_id();
        self.scopes.push(Scope::fixture(id, kind, fixture));
    }

    fn allocate_id(&mut self) -> ScopeId {
        let id = ScopeId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn top(&self) -> Option<ScopeId> {
        self.scopes.last().map(|scope| scope.id)
    }

    fn contains(&self, id: ScopeId) -> bool {
        self.scopes.iter().any(|scope| scope.id == id)
    }

    fn last_mut(&mut self) -> Option<&mut Scope> {
        self.scopes.last_mut()
    }

    fn pop(&mut self) -> Option<Scope> {
        self.scopes.pop()
    }

    fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

/// The single resumable sequence interpreter used by both native hosts and
/// the synchronous compatibility adapter.
pub struct ExecutionMachine<'a> {
    program: &'a ValidatedProgram,
    scopes: ScopeStack,
    state: StateSlot<'a>,
    control: ControlSlot<'a>,
    status: MachineStatus,
    /// Armed enclosing-scope watches in registration order. A machine may hold
    /// several at once so that nested caches each observe their own enclosing
    /// boundary; entries are consumed from the back (LIFO), which is the order
    /// in which their scopes finished.
    watches: Vec<ScopeWatch>,
    /// Watch notifications whose scope already stopped running and which have
    /// not been consumed yet, LIFO.
    pending_scope_completions: Vec<PendingWatchEvent>,
    /// Allocates watch tokens. Tokens are never reused inside one machine.
    next_watch_token: u64,
    last_origin: Option<SequenceId>,
    pending_dispatch_executable: Option<ExecutableId>,
    pending_completion: Option<ExecutionCompletion>,
}

/// One armed enclosing-scope watch: the scope identity observed at dispatch
/// time and the executable that asked for the notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScopeWatch {
    scope: ScopeId,
    executable: ExecutableId,
    token: WatchToken,
}

/// One watch notification waiting to be delivered, in LIFO order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingWatchEvent {
    completion: ScopeCompletion,
    publishable: bool,
}

/// A request-independent continuation. Bind it to its owning program snapshot
/// and a new control root inside the refresh task; it never borrows request state.
pub struct SuccessorRecipe {
    scope: Scope,
    next_scope_id: usize,
    state: ExecutionState,
    origin: Option<SequenceId>,
}

impl SuccessorRecipe {
    /// # Errors
    /// Returns an invalid-entry error if the owning program no longer contains the sequence.
    pub fn bind(
        self,
        program: &ValidatedProgram,
        control: ExecutionControl,
    ) -> Result<ExecutionMachine<'_>, ExecutionError> {
        let entry = self
            .origin
            .or_else(|| self.scope.frame.as_ref().map(|frame| frame.sequence))
            .ok_or(ExecutionError::Finished)?;
        let mut machine = ExecutionMachine::new(program, entry, self.state, control)?;
        machine.scopes.scopes = vec![self.scope];
        machine.scopes.next_id = self.next_scope_id;
        machine.last_origin = self.origin;
        Ok(machine)
    }
}

impl<'a> ExecutionMachine<'a> {
    /// The named sequence that most recently executed a rule. Synthetic inline
    /// scopes are never reported; the nearest enclosing configuration-named
    /// sequence owns the position instead. This is the real executed position
    /// for audit, never a fixed entry tag.
    #[must_use]
    pub fn last_origin(&self) -> Option<SequenceId> {
        self.last_origin
    }

    /// Arms one watch on the scope that enclosed the most recently observed
    /// dispatch. When that exact scope stops running, the machine yields
    /// [`MachineStep::ScopeComplete`] before the caller's next rule executes.
    ///
    /// The watch is a boundary notification, not a re-run: the caller resumes
    /// with [`ExecutionMachine::resume_scope_completion`].
    ///
    /// A machine may hold several watches at once, so nested cache dispatches
    /// can each observe their own enclosing boundary. When scopes stop running
    /// while several watches are bound to them, the notifications are reported
    /// in reverse registration order (LIFO): the innermost, most recently armed
    /// frame first.
    ///
    /// The returned token identifies this exact watch in every later
    /// [`ScopeCompletion`], so an owner can pair a notification with the frame
    /// it armed without guessing from the executable.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::ResumeNotPending`] when `executable` is not the
    /// executable of the most recent dispatch, and
    /// [`ExecutionError::ScopeWatchConflict`] when the machine has no live
    /// enclosing scope to watch.
    pub fn watch_enclosing_scope(
        &mut self,
        executable: ExecutableId,
    ) -> Result<WatchToken, ExecutionError> {
        if self.pending_dispatch_executable != Some(executable) {
            return Err(ExecutionError::ResumeNotPending(executable));
        }
        let Some(scope) = self.scopes.top() else {
            return Err(ExecutionError::ScopeWatchConflict(executable));
        };
        let token = WatchToken(self.next_watch_token);
        self.next_watch_token = self.next_watch_token.wrapping_add(1);
        self.watches.push(ScopeWatch {
            scope,
            executable,
            token,
        });
        // Consume the dispatch token so one dispatch cannot arm twice.
        self.pending_dispatch_executable = None;
        Ok(token)
    }

    /// The number of armed enclosing-scope watches that have not fired yet.
    #[must_use]
    pub fn armed_watch_count(&self) -> usize {
        self.watches.len()
    }

    /// Continues an execution that paused on a watch notification.
    ///
    /// The token must be the one carried by the pending notification; a stale
    /// or foreign token is rejected instead of silently releasing the pause.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::InvalidScopeResume`] when `token` is not the
    /// pending watch token, [`ExecutionError::NoPendingScopeCompletion`] when no
    /// watch notification is pending, and [`ExecutionError::Finished`] when the
    /// machine is already terminal.
    pub fn resume_scope_completion(
        &mut self,
        token: WatchToken,
    ) -> Result<MachineStep, ExecutionError> {
        let expected = match self.status {
            MachineStatus::ScopeCompleted(expected, _) => expected,
            MachineStatus::Complete(_) | MachineStatus::Failed => {
                return Err(ExecutionError::Finished);
            }
            MachineStatus::Running | MachineStatus::Waiting(_) => {
                return Err(ExecutionError::NoPendingScopeCompletion);
            }
        };
        if expected != token {
            return Err(ExecutionError::InvalidScopeResume {
                expected,
                received: token,
            });
        }
        self.status = MachineStatus::Running;
        let result = self.drive(None);
        if result.is_err() {
            self.fail_in_place();
        }
        result
    }

    /// Creates an async-capable machine that owns its execution state and
    /// control. The caller may hold it across an await between `step` and
    /// `resume` without retaining a borrow into a fixture or packet buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::InvalidEntry`] when `entry` is not present in
    /// the validated program.
    pub fn new(
        program: &'a ValidatedProgram,
        entry: SequenceId,
        state: ExecutionState,
        control: ExecutionControl,
    ) -> Result<Self, ExecutionError> {
        if program.sequence(entry).is_none() {
            return Err(ExecutionError::InvalidEntry(entry));
        }
        let mut scopes = ScopeStack::default();
        scopes.push_sequence(ScopeKind::Root, entry);
        Ok(Self {
            program,
            scopes,
            state: StateSlot::Owned(Box::new(state)),
            control: ControlSlot::Owned(control),
            status: MachineStatus::Running,
            watches: Vec::new(),
            pending_scope_completions: Vec::new(),
            next_watch_token: 0,
            last_origin: None,
            pending_dispatch_executable: None,
            pending_completion: None,
        })
    }

    /// Creates the same machine over caller-owned state for the legacy sync
    /// adapter. Control flow is shared with [`ExecutionMachine::new`].
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::InvalidEntry`] when `entry` is not present in
    /// the validated program.
    pub fn borrowed(
        program: &'a ValidatedProgram,
        entry: SequenceId,
        state: &'a mut ExecutionState,
        control: &'a mut ExecutionControl,
    ) -> Result<Self, ExecutionError> {
        if program.sequence(entry).is_none() {
            return Err(ExecutionError::InvalidEntry(entry));
        }
        let mut scopes = ScopeStack::default();
        scopes.push_sequence(ScopeKind::Root, entry);
        Ok(Self {
            program,
            scopes,
            state: StateSlot::Borrowed(state),
            control: ControlSlot::Borrowed(control),
            status: MachineStatus::Running,
            watches: Vec::new(),
            pending_scope_completions: Vec::new(),
            next_watch_token: 0,
            last_origin: None,
            pending_dispatch_executable: None,
            pending_completion: None,
        })
    }

    /// Captures the remaining rules in the currently enclosing scope as an
    /// owned child machine. The pending executable that caused the capture is
    /// excluded: its owner supplies the result exactly once through the
    /// original machine. The child has no enclosing watch, so a parent cache
    /// token can never be consumed by a policy branch.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::ResumeNotPending`] unless the machine is
    /// waiting for a completion, or [`ExecutionError::InvalidEntry`] when it
    /// has no enclosing scope.
    pub fn fork_successor(&self, cancellation: CancellationToken) -> Result<Self, ExecutionError> {
        if !matches!(self.status, MachineStatus::Waiting(_)) {
            return Err(ExecutionError::ResumeNotPending(
                self.pending_dispatch_executable
                    .unwrap_or(ExecutableId(usize::MAX)),
            ));
        }
        let Some(scope) = self.scopes.scopes.last().cloned() else {
            return Err(ExecutionError::InvalidEntry(SequenceId(usize::MAX)));
        };
        self.fork_with_scopes(vec![scope], cancellation)
    }

    /// Captures only this enclosing successor as owned data, with no client control.
    /// # Errors
    /// Returns `Finished` unless an external dispatch is pending in a live scope.
    pub fn capture_successor(&self) -> Result<SuccessorRecipe, ExecutionError> {
        if !matches!(self.status, MachineStatus::Waiting(_)) {
            return Err(ExecutionError::Finished);
        }
        self.capture_scope()
    }

    /// Captures a branch successor before executing a direct policy target.
    /// # Errors
    /// Returns `Finished` unless the branch is running in a live scope.
    pub fn capture_branch_successor(&self) -> Result<SuccessorRecipe, ExecutionError> {
        if !matches!(self.status, MachineStatus::Running) {
            return Err(ExecutionError::Finished);
        }
        self.capture_scope()
    }

    fn capture_scope(&self) -> Result<SuccessorRecipe, ExecutionError> {
        let scope = self
            .scopes
            .scopes
            .last()
            .cloned()
            .ok_or(ExecutionError::Finished)?;
        Ok(SuccessorRecipe {
            scope,
            next_scope_id: self.scopes.next_id,
            state: self.state.as_ref().clone(),
            origin: self.last_origin,
        })
    }

    /// Forks a running branch at its current continuation. This is private to
    /// the native current-thread branch driver in spirit, but remains typed so
    /// the host never clones a local fuel field by accident.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::Finished`] unless the machine is running, or
    /// [`ExecutionError::InvalidEntry`] when the machine has no scopes.
    pub fn fork_branch(&self, cancellation: CancellationToken) -> Result<Self, ExecutionError> {
        if !matches!(self.status, MachineStatus::Running) {
            return Err(ExecutionError::Finished);
        }
        self.fork_with_scopes(self.scopes.scopes.clone(), cancellation)
    }

    fn fork_with_scopes(
        &self,
        scope_values: Vec<Scope>,
        cancellation: CancellationToken,
    ) -> Result<Self, ExecutionError> {
        if scope_values.is_empty() {
            return Err(ExecutionError::InvalidEntry(SequenceId(usize::MAX)));
        }
        let mut scopes = ScopeStack::default();
        scopes.scopes = scope_values;
        scopes.next_id = self.scopes.next_id;
        let state = self.state.as_ref().clone();
        let control = self.control.as_ref().fork_child(cancellation);
        Ok(Self {
            program: self.program,
            scopes,
            state: StateSlot::Owned(Box::new(state)),
            control: ControlSlot::Owned(control),
            status: MachineStatus::Running,
            watches: Vec::new(),
            pending_scope_completions: Vec::new(),
            next_watch_token: 0,
            last_origin: self.last_origin,
            pending_dispatch_executable: None,
            pending_completion: None,
        })
    }

    #[must_use]
    pub fn state(&self) -> &ExecutionState {
        self.state.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut ExecutionState {
        self.state.as_mut()
    }

    #[must_use]
    pub fn control(&self) -> &ExecutionControl {
        self.control.as_ref()
    }

    pub fn control_mut(&mut self) -> &mut ExecutionControl {
        self.control.as_mut()
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            MachineStatus::Complete(_) | MachineStatus::Failed
        )
    }

    /// Runs synchronous sequence work until an external operation or terminal
    /// completion is reached.
    ///
    /// # Errors
    ///
    /// Returns a typed execution, cancellation, budget, or machine-state
    /// error. A machine is terminal after an execution error.
    pub fn step(&mut self) -> Result<MachineStep, ExecutionError> {
        match self.status {
            MachineStatus::Running => {}
            // Both pause states must be released by their owning resume call
            // rather than by another `step`, so they report the same reason.
            // Both pause states must be released by their owning resume call
            // rather than by another `step`: a pending external by `resume`, a
            // pending watch notification by `resume_scope_completion` with its
            // own token.
            MachineStatus::Waiting(executable) | MachineStatus::ScopeCompleted(_, executable) => {
                return Err(ExecutionError::WaitingForExternal(executable));
            }
            MachineStatus::Complete(_) | MachineStatus::Failed => {
                return Err(ExecutionError::Finished);
            }
        }
        let result = self.drive(None);
        if result.is_err() {
            self.fail_in_place();
        }
        result
    }

    /// Supplies the result of the exact pending external operation and drives
    /// the same frames to the next external operation or final completion.
    ///
    /// # Errors
    ///
    /// Returns a typed execution error when the response identity is wrong,
    /// no dispatch is pending, the machine is terminal, or the resumed
    /// outcome cannot be applied.
    pub fn resume(
        &mut self,
        executable: ExecutableId,
        outcome: Result<ExecutorOutcome, ExecutorError>,
    ) -> Result<MachineStep, ExecutionError> {
        let expected = match self.status {
            MachineStatus::Waiting(expected) => expected,
            MachineStatus::Running | MachineStatus::ScopeCompleted(..) => {
                return Err(ExecutionError::ResumeNotPending(executable));
            }
            MachineStatus::Complete(_) | MachineStatus::Failed => {
                return Err(ExecutionError::Finished);
            }
        };
        if expected != executable {
            return Err(ExecutionError::InvalidResume {
                expected,
                received: executable,
            });
        }
        self.status = MachineStatus::Running;
        let ready = match outcome {
            Ok(outcome) => {
                let scopes = &mut self.scopes;
                let state = self.state.as_mut();
                executor_outcome_to_step(outcome, scopes, state)
            }
            Err(error) => Err(ExecutionError::Executor(error)),
        };
        let ready = match ready {
            Ok(ready) => ready,
            Err(error) => {
                self.fail_in_place();
                return Err(error);
            }
        };
        let result = self.drive(Some(ready));
        if result.is_err() {
            self.fail_in_place();
        }
        result
    }

    /// Marks the machine terminally failed and invalidates every armed watch.
    ///
    /// A failed machine has no boundary left that could complete, so leaving a
    /// watch armed would let an owner wait forever for a notification that can
    /// never arrive. The invalidation is silent: the caller receives `Err` and
    /// must drop the frames it holds.
    fn fail(&mut self, error: ExecutionError) -> Result<MachineStep, ExecutionError> {
        self.fail_in_place();
        Err(error)
    }

    fn fail_in_place(&mut self) {
        self.watches.clear();
        self.pending_scope_completions.clear();
        self.pending_completion = None;
        self.pending_dispatch_executable = None;
        self.status = MachineStatus::Failed;
    }

    fn drive(&mut self, mut ready: Option<Step>) -> Result<MachineStep, ExecutionError> {
        loop {
            if let Some(event) = self.pending_scope_completions.pop() {
                self.pending_dispatch_executable = None;
                self.status = MachineStatus::ScopeCompleted(
                    event.completion.token,
                    event.completion.executable,
                );
                return Ok(if event.publishable {
                    MachineStep::ScopeComplete(event.completion)
                } else {
                    MachineStep::ScopeAborted(event.completion)
                });
            }
            if let Some(completion) = self.pending_completion.take() {
                self.pending_dispatch_executable = None;
                self.status = MachineStatus::Complete(completion);
                return Ok(MachineStep::Complete(completion));
            }
            let step = if let Some(step) = ready.take() {
                step
            } else {
                let state = self.state.as_mut();
                let control = self.control.as_mut();
                match next_step(self.program, &mut self.scopes, state, control) {
                    Ok(step) => step,
                    // A terminal stop (cancellation, exhausted budget, a
                    // matcher or executor error) ends the machine. No boundary
                    // can publish afterwards, so every armed watch is
                    // invalidated rather than left pointing at a scope that
                    // will never complete.
                    Err(error) => return self.fail(error),
                }
            };
            if let Some(origin) = self.scopes.rule_origin.take() {
                self.last_origin = Some(origin);
            }
            match step {
                Step::Continue => {}
                Step::Dispatch(executable) => {
                    self.pending_dispatch_executable = Some(executable);
                    self.status = MachineStatus::Waiting(executable);
                    return Ok(MachineStep::Dispatch(ExternalDispatch { executable }));
                }
                Step::Complete(signal) => {
                    let leaving_scope = self.scopes.top();
                    let completion = finish_scope(&mut self.scopes, signal);
                    // `finish_scope` may unwind several scopes at once when an
                    // `exit` propagates, so every watch whose scope is no longer
                    // live must be retired here; otherwise it would keep a dead
                    // scope identity and could never be notified or invalidated.
                    // Watches are visited from the back, i.e. LIFO.
                    //
                    // This loop only runs when `next_step` produced a step. A
                    // cancellation, budget or executor error propagates out of
                    // `drive` and fails the machine, so the owner must drop its
                    // own frames on that path; no notification is emitted.
                    let mut fired: Vec<PendingWatchEvent> = Vec::new();
                    let mut index = self.watches.len();
                    while index > 0 {
                        index -= 1;
                        let watch = self.watches[index];
                        if self.scopes.contains(watch.scope) {
                            continue;
                        }
                        self.watches.remove(index);
                        // Only the scope that produced a natural completion made
                        // a publishable successor result. Every other vanished
                        // scope was unwound by `exit` or a terminal stop, and its
                        // frame must be invalidated instead of published.
                        let publishable = matches!(signal, ScopeSignal::Completed)
                            && leaving_scope == Some(watch.scope);
                        fired.push(PendingWatchEvent {
                            completion: ScopeCompletion {
                                executable: watch.executable,
                                token: watch.token,
                            },
                            publishable,
                        });
                    }
                    if !fired.is_empty() {
                        // `fired` is in reverse registration order and the LIFO
                        // pop below delivers the last element first, so push it
                        // reversed to keep the most recent frame first.
                        for event in fired.into_iter().rev() {
                            self.pending_scope_completions.push(event);
                        }
                        self.pending_completion = completion;
                        self.pending_dispatch_executable = None;
                        self.status = MachineStatus::Running;
                        continue;
                    }
                    if let Some(completion) = completion {
                        self.pending_dispatch_executable = None;
                        self.status = MachineStatus::Complete(completion);
                        return Ok(MachineStep::Complete(completion));
                    }
                }
            }
        }
    }
}

/// Executes one validated sequence using caller-owned state and control.
///
/// The engine uses an explicit scope/continuation stack. Nested `try` scopes
/// receive the same state and control references and never receive a fresh
/// fuel budget.
///
/// # Errors
///
/// Returns a typed matcher/executor, cancellation, budget or invalid-entry
/// error. State remains in the caller's `state` on every result.
pub fn execute(
    program: &ValidatedProgram,
    entry: SequenceId,
    state: &mut ExecutionState,
    control: &mut ExecutionControl,
) -> Result<ExecutionCompletion, ExecutionError> {
    let mut machine = ExecutionMachine::borrowed(program, entry, state, control)?;
    loop {
        match machine.step()? {
            MachineStep::Complete(completion) => return Ok(completion),
            MachineStep::Dispatch(dispatch) => {
                let fixture = program.fixture(dispatch.executable()).ok_or(
                    ExecutionError::ExternalDispatchUnsupported(dispatch.executable()),
                )?;
                let outcome = fixture
                    .executable
                    .execute(machine.state_mut())
                    .map_err(ExecutionError::Executor)?;
                machine.resume(dispatch.executable(), Ok(outcome))?;
            }
            MachineStep::ScopeComplete(completion) => {
                // The synchronous adapter never arms a watch, so this boundary
                // is unreachable here; continue as the owner would.
                machine.resume_scope_completion(completion.token())?;
            }
            MachineStep::ScopeAborted(completion) => {
                // Same as above: this adapter never arms a watch.
                machine.resume_scope_completion(completion.token())?;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeKind {
    Root,
    /// A direct named call. Its natural completion, `accept`, and `reject`
    /// return to the caller; `exit` propagates past it.
    CallChild,
    Inline,
    TryChild,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Frame {
    sequence: SequenceId,
    pc: usize,
}

#[derive(Clone)]
struct Scope {
    id: ScopeId,
    kind: ScopeKind,
    frame: Option<Frame>,
    pending_fixture: Option<ExecutableId>,
    continuations: Vec<Frame>,
}

impl Scope {
    fn sequence(id: ScopeId, kind: ScopeKind, sequence: SequenceId) -> Self {
        Self {
            id,
            kind,
            frame: Some(Frame { sequence, pc: 0 }),
            pending_fixture: None,
            continuations: Vec::new(),
        }
    }

    fn fixture(id: ScopeId, kind: ScopeKind, fixture: ExecutableId) -> Self {
        Self {
            id,
            kind,
            frame: None,
            pending_fixture: Some(fixture),
            continuations: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeSignal {
    Completed,
    Exited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Continue,
    Complete(ScopeSignal),
    Dispatch(ExecutableId),
}

fn next_step(
    program: &ValidatedProgram,
    scopes: &mut ScopeStack,
    state: &mut ExecutionState,
    control: &mut ExecutionControl,
) -> Result<Step, ExecutionError> {
    let Some(scope) = scopes.last_mut() else {
        return Ok(Step::Complete(ScopeSignal::Completed));
    };

    if let Some(fixture) = scope.pending_fixture.take() {
        consume_dispatch(control)?;
        return dispatch_fixture(program, fixture, scopes, state);
    }

    let Some(frame) = scope.frame.as_mut() else {
        return Ok(Step::Complete(ScopeSignal::Completed));
    };
    let sequence_id = frame.sequence;
    let Some(sequence) = program.sequence(sequence_id) else {
        return Err(ExecutionError::InvalidEntry(sequence_id));
    };
    if frame.pc >= sequence.rules.len() {
        if let Some(next) = scope.continuations.pop() {
            scope.frame = Some(next);
            return Ok(Step::Continue);
        }
        return Ok(Step::Complete(ScopeSignal::Completed));
    }
    let pc = frame.pc;
    frame.pc += 1;
    let rule = &sequence.rules[pc];
    if !sequence.synthetic {
        // Only a configuration-named sequence is a real executed origin. A
        // synthetic inline scope keeps the enclosing named sequence as the
        // reported origin instead of inventing an execution position.
        scopes.rule_origin = Some(sequence_id);
    }
    run_rule(program, rule, scopes, state, control)
}

fn run_rule(
    program: &ValidatedProgram,
    rule: &ValidatedRule,
    scopes: &mut ScopeStack,
    state: &mut ExecutionState,
    control: &mut ExecutionControl,
) -> Result<Step, ExecutionError> {
    let mut deferred_routing = Vec::new();
    for matcher in &rule.matchers {
        consume_dispatch(control)?;
        let outcome = matcher
            .matcher
            .evaluate(state)
            .map_err(ExecutionError::Matcher)?;
        if let Some(mutation) = outcome.mutation {
            if matches!(
                &mutation,
                crate::StateMutation::SetRoutingFields { .. }
                    | crate::StateMutation::SetRouting {
                        field: crate::RoutingField::MatchedRuleSource,
                        ..
                    }
            ) {
                deferred_routing.push(mutation);
            } else {
                state.apply_mutation(mutation);
            }
        }
        let effective_match = if matcher.reverse {
            !outcome.matched
        } else {
            outcome.matched
        };
        if !effective_match {
            return Ok(Step::Continue);
        }
        if !matcher.reverse {
            apply_dispatch_metadata(state, &matcher.dispatch_metadata);
        }
    }

    let Some(executable) = &rule.executable else {
        return Ok(Step::Continue);
    };
    for mutation in deferred_routing {
        state.apply_mutation(mutation);
    }
    // A matcher-less executable is the canonical default/unmatched route. It
    // must remain eligible for the public `unmatched_rule` sentinel instead
    // of being relabelled as an inline rule merely because the compiler gave
    // every executable-bearing rule a private source identity.
    if !rule.matchers.is_empty() && state.routing.matched_rule_source.is_none() {
        if let Some(source) = &rule.audit_source {
            state.apply_mutation(crate::StateMutation::SetRouting {
                field: crate::RoutingField::MatchedRuleSource,
                value: Some(source.clone()),
            });
        }
    }
    consume_dispatch(control)?;
    dispatch_executable(program, executable, scopes, state)
}

fn apply_dispatch_metadata(state: &mut ExecutionState, metadata: &DispatchMetadata) {
    if state.routing.domain_set.is_some() {
        return;
    }
    let value = match metadata {
        DispatchMetadata::None => None,
        DispatchMetadata::AnonymousQname { rule_name } => Some(rule_name.clone()),
        DispatchMetadata::Switch6 => {
            (state.query.question.qtype == QTYPE_AAAA).then(|| "BANAAAA".to_owned())
        }
        DispatchMetadata::Switch5 => match state.query.question.qtype {
            QTYPE_SOA => Some("BANSOA".to_owned()),
            QTYPE_PTR => Some("BANPTR".to_owned()),
            QTYPE_HTTPS => Some("BANHTTPS".to_owned()),
            _ => None,
        },
    };
    if let Some(value) = value {
        state.routing.domain_set = Some(value);
    }
}

fn dispatch_executable(
    program: &ValidatedProgram,
    executable: &ValidatedExecutable,
    scopes: &mut ScopeStack,
    state: &mut ExecutionState,
) -> Result<Step, ExecutionError> {
    match executable {
        ValidatedExecutable::Accept => Ok(Step::Complete(ScopeSignal::Completed)),
        ValidatedExecutable::Return => Ok(return_from_current_scope(scopes)),
        ValidatedExecutable::Reject { rcode } => {
            state
                .set_synthesized_response(*rcode)
                .map_err(|_| ExecutionError::Executor(ExecutorError::InvalidRcode(*rcode)))?;
            Ok(Step::Complete(ScopeSignal::Completed))
        }
        ValidatedExecutable::Exit => Ok(Step::Complete(ScopeSignal::Exited)),
        ValidatedExecutable::Call { target } => {
            scopes.push_sequence(ScopeKind::CallChild, *target);
            Ok(Step::Continue)
        }
        ValidatedExecutable::Goto { target } => {
            let Some(scope) = scopes.last_mut() else {
                return Ok(Step::Complete(ScopeSignal::Completed));
            };
            scope.frame = Some(Frame {
                sequence: *target,
                pc: 0,
            });
            scope.continuations.clear();
            Ok(Step::Continue)
        }
        ValidatedExecutable::Jump { target } => {
            let Some(scope) = scopes.last_mut() else {
                return Ok(Step::Complete(ScopeSignal::Completed));
            };
            let Some(return_to) = scope.frame else {
                return Ok(Step::Complete(ScopeSignal::Completed));
            };
            scope.continuations.push(return_to);
            scope.frame = Some(Frame {
                sequence: *target,
                pc: 0,
            });
            Ok(Step::Continue)
        }
        ValidatedExecutable::Try { target } => {
            match target {
                ExecutableTarget::Sequence(sequence) => {
                    scopes.push_sequence(ScopeKind::TryChild, *sequence);
                }
                ExecutableTarget::Fixture(fixture) => {
                    scopes.push_fixture(ScopeKind::TryChild, *fixture);
                }
            }
            Ok(Step::Continue)
        }
        ValidatedExecutable::Fixture { target } => {
            dispatch_fixture(program, *target, scopes, state)
        }
        ValidatedExecutable::External { target } => dispatch_external(program, *target),
        ValidatedExecutable::Inline { target } => {
            scopes.push_sequence(ScopeKind::Inline, *target);
            Ok(Step::Continue)
        }
    }
}

fn dispatch_external(
    program: &ValidatedProgram,
    executable: ExecutableId,
) -> Result<Step, ExecutionError> {
    if program.external(executable).is_none() {
        return Err(ExecutionError::InvalidFixture(executable));
    }
    Ok(Step::Dispatch(executable))
}

fn dispatch_fixture(
    program: &ValidatedProgram,
    fixture: ExecutableId,
    scopes: &mut ScopeStack,
    state: &mut ExecutionState,
) -> Result<Step, ExecutionError> {
    // The enclosing rule executable owns this dispatch unit. A fixture target
    // is the executable's call, so charging again here would double-count it.
    let Some(fixture) = program.fixture(fixture) else {
        return Err(ExecutionError::InvalidFixture(fixture));
    };
    let outcome = fixture
        .executable
        .execute(state)
        .map_err(ExecutionError::Executor)?;
    executor_outcome_to_step(outcome, scopes, state)
}

fn return_from_current_scope(scopes: &mut ScopeStack) -> Step {
    let Some(scope) = scopes.last_mut() else {
        return Step::Complete(ScopeSignal::Completed);
    };
    if let Some(next) = scope.continuations.pop() {
        scope.frame = Some(next);
        Step::Continue
    } else {
        Step::Complete(ScopeSignal::Completed)
    }
}

fn executor_outcome_to_step(
    outcome: ExecutorOutcome,
    scopes: &mut ScopeStack,
    state: &mut ExecutionState,
) -> Result<Step, ExecutionError> {
    match outcome {
        ExecutorOutcome::Continue => Ok(Step::Continue),
        ExecutorOutcome::Return => Ok(return_from_current_scope(scopes)),
        ExecutorOutcome::Accept => Ok(Step::Complete(ScopeSignal::Completed)),
        ExecutorOutcome::Reject { rcode } => {
            state
                .set_synthesized_response(rcode)
                .map_err(|_| ExecutionError::Executor(ExecutorError::InvalidRcode(rcode)))?;
            Ok(Step::Complete(ScopeSignal::Completed))
        }
        ExecutorOutcome::Exit => Ok(Step::Complete(ScopeSignal::Exited)),
    }
}

fn consume_dispatch(control: &mut ExecutionControl) -> Result<(), ExecutionError> {
    control.try_consume()
}

fn finish_scope(scopes: &mut ScopeStack, signal: ScopeSignal) -> Option<ExecutionCompletion> {
    let Some(scope) = scopes.pop() else {
        return Some(ExecutionCompletion::Completed);
    };
    match signal {
        ScopeSignal::Completed => {
            if scopes.is_empty() {
                Some(ExecutionCompletion::Completed)
            } else {
                None
            }
        }
        ScopeSignal::Exited => {
            if scopes.is_empty() {
                return Some(ExecutionCompletion::Exited);
            }
            match scope.kind {
                ScopeKind::TryChild => None,
                ScopeKind::Inline | ScopeKind::CallChild => propagate_exit(scopes),
                ScopeKind::Root => Some(ExecutionCompletion::Exited),
            }
        }
    }
}

fn propagate_exit(scopes: &mut ScopeStack) -> Option<ExecutionCompletion> {
    loop {
        let Some(scope) = scopes.pop() else {
            return Some(ExecutionCompletion::Exited);
        };
        if scopes.is_empty() {
            return Some(ExecutionCompletion::Exited);
        }
        match scope.kind {
            ScopeKind::TryChild => return None,
            ScopeKind::Inline | ScopeKind::CallChild => {}
            ScopeKind::Root => return Some(ExecutionCompletion::Exited),
        }
    }
}
