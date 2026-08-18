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
    Matcher(MatcherError),
    Executor(ExecutorError),
    Cancelled,
    BudgetExceeded,
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
    if program.sequence(entry).is_none() {
        return Err(ExecutionError::InvalidEntry(entry));
    }

    let mut scopes = vec![Scope::sequence(ScopeKind::Root, entry)];
    loop {
        match next_step(program, &mut scopes, state, control)? {
            Step::Continue => {}
            Step::Complete(signal) => {
                if let Some(completion) = finish_scope(&mut scopes, signal) {
                    return Ok(completion);
                }
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
        ValidatedExecutable::Inline { target } => {
            scopes.push(Scope::sequence(ScopeKind::Inline, *target));
            Ok(Step::Continue)
        }
    }
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
