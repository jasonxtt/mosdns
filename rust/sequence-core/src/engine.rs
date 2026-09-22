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
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
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
}

impl ExecutionControl {
    #[must_use]
    pub fn with_fuel(remaining_fuel: u64) -> Self {
        Self {
            remaining_fuel,
            cancellation: CancellationState::Active,
            cancellation_token: CancellationToken::new(),
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
        }
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

/// The externally observable progress of one canonical sequence machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineStep {
    Dispatch(ExternalDispatch),
    Complete(ExecutionCompletion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MachineStatus {
    Running,
    Waiting(ExecutableId),
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

/// The single resumable sequence interpreter used by both native hosts and
/// the synchronous compatibility adapter.
pub struct ExecutionMachine<'a> {
    program: &'a ValidatedProgram,
    scopes: Vec<Scope>,
    state: StateSlot<'a>,
    control: ControlSlot<'a>,
    status: MachineStatus,
}

impl<'a> ExecutionMachine<'a> {
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
        Ok(Self {
            program,
            scopes: vec![Scope::sequence(ScopeKind::Root, entry)],
            state: StateSlot::Owned(Box::new(state)),
            control: ControlSlot::Owned(control),
            status: MachineStatus::Running,
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
        Ok(Self {
            program,
            scopes: vec![Scope::sequence(ScopeKind::Root, entry)],
            state: StateSlot::Borrowed(state),
            control: ControlSlot::Borrowed(control),
            status: MachineStatus::Running,
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
            MachineStatus::Waiting(executable) => {
                return Err(ExecutionError::WaitingForExternal(executable));
            }
            MachineStatus::Complete(_) | MachineStatus::Failed => {
                return Err(ExecutionError::Finished);
            }
        }
        let result = self.drive(None);
        if result.is_err() {
            self.status = MachineStatus::Failed;
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
            MachineStatus::Running => {
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
                self.status = MachineStatus::Failed;
                return Err(error);
            }
        };
        let result = self.drive(Some(ready));
        if result.is_err() {
            self.status = MachineStatus::Failed;
        }
        result
    }

    fn drive(&mut self, mut ready: Option<Step>) -> Result<MachineStep, ExecutionError> {
        loop {
            let step = if let Some(step) = ready.take() {
                step
            } else {
                let state = self.state.as_mut();
                let control = self.control.as_mut();
                next_step(self.program, &mut self.scopes, state, control)?
            };
            match step {
                Step::Continue => {}
                Step::Dispatch(executable) => {
                    self.status = MachineStatus::Waiting(executable);
                    return Ok(MachineStep::Dispatch(ExternalDispatch { executable }));
                }
                Step::Complete(signal) => {
                    if let Some(completion) = finish_scope(&mut self.scopes, signal) {
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
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeKind {
    Root,
    Inline,
    TryChild,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Frame {
    sequence: SequenceId,
    pc: usize,
}

struct Scope {
    kind: ScopeKind,
    frame: Option<Frame>,
    pending_fixture: Option<ExecutableId>,
    continuations: Vec<Frame>,
}

impl Scope {
    fn sequence(kind: ScopeKind, sequence: SequenceId) -> Self {
        Self {
            kind,
            frame: Some(Frame { sequence, pc: 0 }),
            pending_fixture: None,
            continuations: Vec::new(),
        }
    }

    fn fixture(kind: ScopeKind, fixture: ExecutableId) -> Self {
        Self {
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
    scopes: &mut Vec<Scope>,
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
    run_rule(program, rule, scopes, state, control)
}

fn run_rule(
    program: &ValidatedProgram,
    rule: &ValidatedRule,
    scopes: &mut Vec<Scope>,
    state: &mut ExecutionState,
    control: &mut ExecutionControl,
) -> Result<Step, ExecutionError> {
    for matcher in &rule.matchers {
        consume_dispatch(control)?;
        let outcome = matcher
            .matcher
            .evaluate(state)
            .map_err(ExecutionError::Matcher)?;
        if let Some(mutation) = outcome.mutation {
            state.apply_mutation(mutation);
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
    scopes: &mut Vec<Scope>,
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
                    scopes.push(Scope::sequence(ScopeKind::TryChild, *sequence));
                }
                ExecutableTarget::Fixture(fixture) => {
                    scopes.push(Scope::fixture(ScopeKind::TryChild, *fixture));
                }
            }
            Ok(Step::Continue)
        }
        ValidatedExecutable::Fixture { target } => {
            dispatch_fixture(program, *target, scopes, state)
        }
        ValidatedExecutable::External { target } => dispatch_external(program, *target),
        ValidatedExecutable::Inline { target } => {
            scopes.push(Scope::sequence(ScopeKind::Inline, *target));
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
    scopes: &mut [Scope],
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

fn return_from_current_scope(scopes: &mut [Scope]) -> Step {
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
    scopes: &mut [Scope],
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
    if control.is_cancelled() {
        return Err(ExecutionError::Cancelled);
    }
    if control.remaining_fuel == 0 {
        return Err(ExecutionError::BudgetExceeded);
    }
    control.remaining_fuel -= 1;
    Ok(())
}

fn finish_scope(scopes: &mut Vec<Scope>, signal: ScopeSignal) -> Option<ExecutionCompletion> {
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
                ScopeKind::Inline => propagate_exit(scopes),
                ScopeKind::Root => Some(ExecutionCompletion::Exited),
            }
        }
    }
}

fn propagate_exit(scopes: &mut Vec<Scope>) -> Option<ExecutionCompletion> {
    loop {
        let Some(scope) = scopes.pop() else {
            return Some(ExecutionCompletion::Exited);
        };
        if scopes.is_empty() {
            return Some(ExecutionCompletion::Exited);
        }
        match scope.kind {
            ScopeKind::TryChild => return None,
            ScopeKind::Inline => {}
            ScopeKind::Root => return Some(ExecutionCompletion::Exited),
        }
    }
}
