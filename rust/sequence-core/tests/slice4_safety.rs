use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Barrier};
use std::thread;

use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{
    CancellationToken, DispatchMetadata, DnsResponseInspector, ExecutableSpec, ExecutableTarget,
    ExecutableTargetSpec, ExecutionCompletion, ExecutionControl, ExecutionError, ExecutionState,
    Executor, ExecutorError, ExecutorOutcome, FixtureRef, FixtureSpec, MatchOutcome, Matcher,
    MatcherError, MatcherSpecInput, OwnedResponseWire, ProgramError, ProgramSpec, ResponseError,
    ResponseState, RoutingState, RuleSpec, SequenceId, SequenceRef, SequenceSpec, StateMutation,
    StateSnapshot, SynthesizedResponse, ValidatedExecutable, execute,
};

fn state() -> ExecutionState {
    ExecutionState::new(
        QueryHeader {
            id: 1,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        QuestionInfo {
            qname_wire: vec![0],
            qtype: 1,
            qclass: 1,
        },
    )
}

fn entry(program: &mosdns_sequence_core::ValidatedProgram) -> SequenceId {
    program.sequence_id("main").expect("main entry")
}

fn fixture_exec(name: &str) -> ExecutableSpec {
    ExecutableSpec::Fixture {
        target: FixtureRef::new(name),
    }
}

struct RecordingExecutor {
    label: &'static str,
    calls: Rc<RefCell<Vec<&'static str>>>,
    outcome: ExecutorOutcome,
    error: Option<ExecutorError>,
}

impl Executor for RecordingExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        self.calls.borrow_mut().push(self.label);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(self.outcome)
    }
}

fn fixture(name: &str, label: &'static str, calls: &Rc<RefCell<Vec<&'static str>>>) -> FixtureSpec {
    FixtureSpec::new(
        name,
        Box::new(RecordingExecutor {
            label,
            calls: Rc::clone(calls),
            outcome: ExecutorOutcome::Continue,
            error: None,
        }),
    )
}

fn outcome_fixture(
    name: &str,
    label: &'static str,
    calls: &Rc<RefCell<Vec<&'static str>>>,
    outcome: ExecutorOutcome,
) -> FixtureSpec {
    FixtureSpec::new(
        name,
        Box::new(RecordingExecutor {
            label,
            calls: Rc::clone(calls),
            outcome,
            error: None,
        }),
    )
}

fn error_fixture(
    name: &str,
    label: &'static str,
    calls: &Rc<RefCell<Vec<&'static str>>>,
) -> FixtureSpec {
    FixtureSpec::new(
        name,
        Box::new(RecordingExecutor {
            label,
            calls: Rc::clone(calls),
            outcome: ExecutorOutcome::Continue,
            error: Some(ExecutorError::new("ordinary executor error")),
        }),
    )
}

#[test]
fn one_fuel_unit_runs_one_fixture_executable_dispatch() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec("fixture")]))],
        )],
        vec![fixture("fixture", "fixture", &calls)],
    )
    .validate()
    .expect("valid fixture program");

    let mut state = state();
    let mut control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*calls.borrow(), &["fixture"]);
    assert_eq!(control.remaining_fuel, 0);
}

struct AlwaysMatcher;

impl Matcher for AlwaysMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(true, None))
    }
}

#[test]
fn one_fuel_unit_is_consumed_per_matcher_and_executable_dispatch() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![MatcherSpecInput::new(
                    Box::new(AlwaysMatcher),
                    false,
                    DispatchMetadata::None,
                )],
                Some(vec![fixture_exec("fixture")]),
            )],
        )],
        vec![fixture("fixture", "fixture", &calls)],
    )
    .validate()
    .expect("valid matcher and fixture program");

    let mut state = state();
    let mut control = ExecutionControl::with_fuel(2);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*calls.borrow(), &["fixture"]);
    assert_eq!(control.remaining_fuel, 0);
}

#[test]
fn try_fixture_has_an_exact_child_dispatch_charge() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("fixture")),
            }]))],
        )],
        vec![fixture("fixture", "fixture", &calls)],
    )
    .validate()
    .expect("valid try fixture program");

    let mut one_fuel_state = state();
    let mut one_fuel_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &program,
            entry(&program),
            &mut one_fuel_state,
            &mut one_fuel_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert!(calls.borrow().is_empty());
    assert_eq!(one_fuel_control.remaining_fuel, 0);

    let mut two_fuel_state = state();
    let mut two_fuel_control = ExecutionControl::with_fuel(2);
    assert_eq!(
        execute(
            &program,
            entry(&program),
            &mut two_fuel_state,
            &mut two_fuel_control,
        ),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*calls.borrow(), &["fixture"]);
    assert_eq!(two_fuel_control.remaining_fuel, 0);
}

#[test]
fn try_fixture_boundary_preserves_budget_error_and_exit_priority() {
    let error_calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("error")),
            }]))],
        )],
        vec![error_fixture("error", "error", &error_calls)],
    )
    .validate()
    .expect("valid try error fixture program");

    let mut budget_error_state = state();
    let mut budget_error_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &error_program,
            entry(&error_program),
            &mut budget_error_state,
            &mut budget_error_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert!(error_calls.borrow().is_empty());
    assert_eq!(budget_error_control.remaining_fuel, 0);

    let mut ordinary_error_state = state();
    let mut ordinary_error_control = ExecutionControl::with_fuel(2);
    assert_eq!(
        execute(
            &error_program,
            entry(&error_program),
            &mut ordinary_error_state,
            &mut ordinary_error_control,
        ),
        Err(ExecutionError::Executor(ExecutorError::Failed(
            "ordinary executor error".to_owned(),
        )))
    );
    assert_eq!(&*error_calls.borrow(), &["error"]);
    assert_eq!(ordinary_error_control.remaining_fuel, 0);

    let exit_calls = Rc::new(RefCell::new(Vec::new()));
    let exit_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("exit")),
            }]))],
        )],
        vec![outcome_fixture(
            "exit",
            "exit",
            &exit_calls,
            ExecutorOutcome::Exit,
        )],
    )
    .validate()
    .expect("valid try exit fixture program");

    let mut budget_exit_state = state();
    let mut budget_exit_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut budget_exit_state,
            &mut budget_exit_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert!(exit_calls.borrow().is_empty());

    let mut caught_exit_state = state();
    let mut caught_exit_control = ExecutionControl::with_fuel(2);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut caught_exit_state,
            &mut caught_exit_control,
        ),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*exit_calls.borrow(), &["exit"]);
    assert_eq!(caught_exit_control.remaining_fuel, 0);
}

#[test]
fn try_fixture_cancellation_is_checked_at_the_child_boundary() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    token.cancel();
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("fixture")),
            }]))],
        )],
        vec![fixture("fixture", "fixture", &calls)],
    )
    .validate()
    .expect("valid cancelled try fixture program");

    let mut state = state();
    let mut control = ExecutionControl::with_cancellation_token(1, token);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::Cancelled)
    );
    assert!(calls.borrow().is_empty());
    assert_eq!(control.remaining_fuel, 1);
}

fn inline_try_program(
    child_rules: Vec<RuleSpec>,
    calls: &Rc<RefCell<Vec<&'static str>>>,
) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new("child", child_rules),
        ],
        vec![
            fixture("exec1", "exec1", calls),
            fixture("child", "child", calls),
            fixture("exec3", "exec3", calls),
            fixture("outer", "outer", calls),
        ],
    )
    .validate()
    .expect("valid inline try program")
}

#[test]
fn inline_try_propagates_child_budget_exhaustion() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = inline_try_program(
        vec![RuleSpec::unconditional(Some(vec![fixture_exec("child")]))],
        &calls,
    );
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(3);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::BudgetExceeded)
    );
    assert_eq!(&*calls.borrow(), &["exec1"]);
    assert_eq!(control.remaining_fuel, 0);
}

#[test]
fn inline_try_propagates_child_cancellation() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "child",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("child-after")])),
                ],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &calls),
            FixtureSpec::new(
                "cancel",
                Box::new(CancellingExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    token: token.clone(),
                }),
            ),
            fixture("child-after", "child-after", &calls),
            fixture("exec3", "exec3", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline cancellation program");

    let mut state = state();
    let mut control = ExecutionControl::with_cancellation_token(20, token);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::Cancelled)
    );
    assert_eq!(&*calls.borrow(), &["exec1", "cancel"]);
}

struct CancellingExecutor {
    label: &'static str,
    calls: Rc<RefCell<Vec<&'static str>>>,
    token: CancellationToken,
}

impl Executor for CancellingExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        self.calls.borrow_mut().push(self.label);
        self.token.cancel();
        Ok(ExecutorOutcome::Continue)
    }
}

struct ExternallyCancelledExecutor {
    label: &'static str,
    calls: Rc<RefCell<Vec<&'static str>>>,
    entered: Arc<Barrier>,
    released: Arc<Barrier>,
}

impl Executor for ExternallyCancelledExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        self.calls.borrow_mut().push(self.label);
        self.entered.wait();
        self.released.wait();
        Ok(ExecutorOutcome::Continue)
    }
}

#[test]
fn shared_cancellation_token_is_observed_at_the_next_dispatch_boundary() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let entered = Arc::new(Barrier::new(2));
    let released = Arc::new(Barrier::new(2));
    let canceller_token = token.clone();
    let canceller_entered = Arc::clone(&entered);
    let canceller_released = Arc::clone(&released);
    let canceller = thread::spawn(move || {
        canceller_entered.wait();
        canceller_token.cancel();
        canceller_released.wait();
    });
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
            ],
        )],
        vec![
            FixtureSpec::new(
                "cancel",
                Box::new(ExternallyCancelledExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    entered,
                    released,
                }),
            ),
            fixture("after", "after", &calls),
        ],
    )
    .validate()
    .expect("valid shared cancellation program");

    let mut state = state();
    let mut control = ExecutionControl::with_cancellation_token(10, token.clone());
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::Cancelled)
    );
    canceller.join().expect("cancellation thread completes");
    assert_eq!(&*calls.borrow(), &["cancel"]);
    assert!(token.is_cancelled());
    assert_eq!(control.remaining_fuel, 9);
}

#[test]
fn cyclic_goto_and_jump_are_bounded_by_shared_fuel() {
    let goto_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("target"),
                }]))],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("main"),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid cyclic goto program");
    let mut goto_state = state();
    let mut goto_control = ExecutionControl::with_fuel(4);
    assert_eq!(
        execute(
            &goto_program,
            entry(&goto_program),
            &mut goto_state,
            &mut goto_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert_eq!(goto_control.remaining_fuel, 0);

    let jump_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                    target: SequenceRef::new("target"),
                }]))],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                    target: SequenceRef::new("main"),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid cyclic jump program");
    let mut jump_state = state();
    let mut jump_control = ExecutionControl::with_fuel(4);
    assert_eq!(
        execute(
            &jump_program,
            entry(&jump_program),
            &mut jump_state,
            &mut jump_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert_eq!(jump_control.remaining_fuel, 0);
}

#[test]
fn nested_try_reuses_root_fuel_without_resetting() {
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                }]))],
            ),
            SequenceSpec::new(
                "a",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("b")),
                }]))],
            ),
            SequenceSpec::new(
                "b",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid nested try cycle");
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(3);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::BudgetExceeded)
    );
    assert_eq!(control.remaining_fuel, 0);
}

#[test]
fn nested_try_observes_the_same_cancellation_token() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "a",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("b")),
                }]))],
            ),
            SequenceSpec::new(
                "b",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("b-after")])),
                ],
            ),
        ],
        vec![
            FixtureSpec::new(
                "cancel",
                Box::new(CancellingExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    token: token.clone(),
                }),
            ),
            fixture("b-after", "b-after", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid nested cancellation program");

    let mut state = state();
    let mut control = ExecutionControl::with_cancellation_token(20, token);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::Cancelled)
    );
    assert_eq!(&*calls.borrow(), &["cancel"]);
}

struct ErrorMatcher;

impl Matcher for ErrorMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Err(MatcherError::new("ordinary matcher error"))
    }
}

#[test]
fn execution_priority_is_cancelled_then_budget_then_ordinary_error_then_exit() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = error_program(&calls);

    let token = CancellationToken::new();
    token.cancel();
    let mut cancelled_state = state();
    let mut cancelled_control = ExecutionControl::with_cancellation_token(0, token);
    assert_eq!(
        execute(
            &error_program,
            entry(&error_program),
            &mut cancelled_state,
            &mut cancelled_control,
        ),
        Err(ExecutionError::Cancelled)
    );
    assert!(calls.borrow().is_empty());

    let mut budget_state = state();
    let mut budget_control = ExecutionControl::with_fuel(0);
    assert_eq!(
        execute(
            &error_program,
            entry(&error_program),
            &mut budget_state,
            &mut budget_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert!(calls.borrow().is_empty());

    let mut error_state = state();
    let mut error_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &error_program,
            entry(&error_program),
            &mut error_state,
            &mut error_control,
        ),
        Err(ExecutionError::Executor(ExecutorError::Failed(
            "ordinary executor error".to_owned(),
        )))
    );
    assert_eq!(&*calls.borrow(), &["error"]);

    let matcher_program = matcher_error_program();
    let mut matcher_state = state();
    let mut matcher_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &matcher_program,
            entry(&matcher_program),
            &mut matcher_state,
            &mut matcher_control,
        ),
        Err(ExecutionError::Matcher(MatcherError::Failed(
            "ordinary matcher error".to_owned(),
        )))
    );

    let exit_program = exit_program(&calls);
    let exit_token = CancellationToken::new();
    exit_token.cancel();
    let mut cancelled_exit_state = state();
    let mut cancelled_exit_control = ExecutionControl::with_cancellation_token(0, exit_token);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut cancelled_exit_state,
            &mut cancelled_exit_control,
        ),
        Err(ExecutionError::Cancelled)
    );

    let mut budget_exit_state = state();
    let mut budget_exit_control = ExecutionControl::with_fuel(0);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut budget_exit_state,
            &mut budget_exit_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );

    let mut exit_state = state();
    let mut exit_control = ExecutionControl::with_fuel(1);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut exit_state,
            &mut exit_control,
        ),
        Ok(ExecutionCompletion::Exited)
    );
    assert_eq!(&*calls.borrow(), &["error", "exit"]);
}

fn error_program(calls: &Rc<RefCell<Vec<&'static str>>>) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec("error")]))],
        )],
        vec![error_fixture("error", "error", calls)],
    )
    .validate()
    .expect("valid error program")
}

fn matcher_error_program() -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![MatcherSpecInput::new(
                    Box::new(ErrorMatcher),
                    false,
                    DispatchMetadata::None,
                )],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid matcher error program")
}

fn exit_program(calls: &Rc<RefCell<Vec<&'static str>>>) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec("exit")]))],
        )],
        vec![outcome_fixture(
            "exit",
            "exit",
            calls,
            ExecutorOutcome::Exit,
        )],
    )
    .validate()
    .expect("valid exit program")
}

#[test]
fn canonical_contract_snapshot_is_typed_and_deterministic() {
    let mut state = state();
    state.marks.insert(49);
    state.marks.insert(7);
    state.fast_flags = 0x0001_0000_0000_0001;
    state.routing = RoutingState {
        domain_set: Some("domain-set".to_owned()),
        matched_group: Some("group".to_owned()),
        final_sequence: Some("final-sequence".to_owned()),
        final_upstream: Some("final-upstream".to_owned()),
        final_upstream_targets: Some("target-a,target-b".to_owned()),
        selected_upstream: Some("selected-upstream".to_owned()),
        matched_rule_source: Some("rule-source".to_owned()),
    };
    state
        .set_synthesized_response(0x0fff)
        .expect("configured RCODE is valid");

    assert_eq!(
        state.snapshot(),
        StateSnapshot {
            query: state.query.clone(),
            marks: vec![7, 49],
            fast_flags: 0x0001_0000_0000_0001,
            response: ResponseState::Synthesized(
                SynthesizedResponse::new(0x0fff).expect("configured RCODE is valid"),
            ),
            routing: RoutingState {
                domain_set: Some("domain-set".to_owned()),
                matched_group: Some("group".to_owned()),
                final_sequence: Some("final-sequence".to_owned()),
                final_upstream: Some("final-upstream".to_owned()),
                final_upstream_targets: Some("target-a,target-b".to_owned()),
                selected_upstream: Some("selected-upstream".to_owned()),
                matched_rule_source: Some("rule-source".to_owned()),
            },
        }
    );
}

#[derive(Debug, Eq, PartialEq)]
enum ContractCheck {
    Passed,
    Failed,
}

fn check(value: bool) -> ContractCheck {
    if value {
        ContractCheck::Passed
    } else {
        ContractCheck::Failed
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bContractSnapshot {
    state: Phase3bStateSnapshot,
    program: Phase3bProgramSnapshot,
    matcher: Phase3bMatcherSnapshot,
    control: Phase3bControlSnapshot,
    safety: Phase3bSafetySnapshot,
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bStateSnapshot {
    state_snapshot_is_owned_and_deterministic: ContractCheck,
    raw_wire_retained: ContractCheck,
    raw_inspection_is_non_consuming: ContractCheck,
    malformed_raw_retained_as_error: ContractCheck,
    synthesized_rcode_max: u16,
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bProgramSnapshot {
    zero_matcher_is_unconditional: ContractCheck,
    missing_exec_is_noop: ContractCheck,
    empty_exec_is_noop: ContractCheck,
    inline_order: Vec<&'static str>,
    repeated_matcher_order: Vec<&'static str>,
    repeated_executable_order: Vec<&'static str>,
    same_fixture_repeated: Vec<&'static str>,
    synthetic_inline_is_not_symbolic: ContractCheck,
    typed_namespace_resolution: ContractCheck,
    deterministic_validation_failure: ContractCheck,
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bMatcherSnapshot {
    matcher_order: Vec<&'static str>,
    false_short_circuit: ContractCheck,
    error_short_circuit: ContractCheck,
    mutation_survives_later_error: ContractCheck,
    reverse_preserves_mutation: ContractCheck,
    reversed_membership_claims_positive_label: ContractCheck,
    qname_label: Option<String>,
    switch6_label: Option<String>,
    switch5_labels: Vec<String>,
    domain_set_write_once: ContractCheck,
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bControlSnapshot {
    root_goto_log: Vec<&'static str>,
    root_jump_log: Vec<&'static str>,
    top_return: ExecutionCompletion,
    accept: ExecutionCompletion,
    default_reject_rcode: u16,
    explicit_reject_rcode: u16,
    exit: ExecutionCompletion,
    try_normal_log: Vec<&'static str>,
    try_fixture_log: Vec<&'static str>,
    try_exit_log: Vec<&'static str>,
    try_ordinary_error_propagates: bool,
    inline_fallthrough_log: Vec<&'static str>,
    inline_return_log: Vec<&'static str>,
    inline_accept_log: Vec<&'static str>,
    inline_reject_log: Vec<&'static str>,
    inline_jump_log: Vec<&'static str>,
    inline_goto_log: Vec<&'static str>,
    inline_exit: ExecutionCompletion,
    inline_try_normal_log: Vec<&'static str>,
    inline_try_exit_log: Vec<&'static str>,
}

#[derive(Debug, Eq, PartialEq)]
struct Phase3bSafetySnapshot {
    direct_fixture_remaining_fuel: u64,
    try_fixture_remaining_fuel: u64,
    try_fixture_budget: ExecutionError,
    try_fixture_ordinary_error: ExecutionError,
    try_fixture_exit: ExecutionCompletion,
    try_fixture_cancelled: ExecutionError,
    goto_cycle: ExecutionError,
    jump_cycle: ExecutionError,
    nested_try_cycle: ExecutionError,
    nested_try_cancelled: ExecutionError,
    external_cancel: ExecutionError,
    inline_budget: ExecutionError,
    inline_budget_log: Vec<&'static str>,
    inline_cancelled: ExecutionError,
    inline_cancelled_log: Vec<&'static str>,
    priority_cancelled: ExecutionError,
    priority_budget: ExecutionError,
    priority_ordinary_error: ExecutionError,
    priority_exit: ExecutionCompletion,
}

fn response_wire() -> Vec<u8> {
    let mut wire = vec![
        0x12, 0x34, 0x80, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];
    wire.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&[
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 1,
    ]);
    wire
}

fn run_with_fuel(
    program: &mosdns_sequence_core::ValidatedProgram,
    fuel: u64,
) -> (
    Result<ExecutionCompletion, ExecutionError>,
    u64,
    ExecutionState,
) {
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(fuel);
    let result = execute(program, entry(program), &mut state, &mut control);
    (result, control.remaining_fuel, state)
}

fn run_with_token(
    program: &mosdns_sequence_core::ValidatedProgram,
    fuel: u64,
    token: CancellationToken,
) -> (
    Result<ExecutionCompletion, ExecutionError>,
    u64,
    ExecutionState,
) {
    let mut state = state();
    let mut control = ExecutionControl::with_cancellation_token(fuel, token);
    let result = execute(program, entry(program), &mut state, &mut control);
    (result, control.remaining_fuel, state)
}

fn state_contract_snapshot() -> Phase3bStateSnapshot {
    let mut snapshot_state = state();
    snapshot_state.marks.insert(49);
    snapshot_state.marks.insert(7);
    snapshot_state.fast_flags = 1 << 48;
    snapshot_state.routing.domain_set = Some("rule-a".to_owned());
    let snapshot = snapshot_state.snapshot();
    snapshot_state.marks.insert(100);
    snapshot_state.fast_flags = 0;
    let state_snapshot_is_owned_and_deterministic = check(
        snapshot.marks == vec![7, 49]
            && snapshot.fast_flags == 1 << 48
            && snapshot.routing.domain_set.as_deref() == Some("rule-a")
            && !snapshot.marks.contains(&100),
    );

    let wire = response_wire();
    let mut raw_state = state();
    raw_state.set_raw_response(wire.clone());
    let raw_before_inspection = raw_state.response.clone();
    let inspection = raw_state
        .inspect_response(&DnsResponseInspector)
        .expect("valid raw response inspection");
    let raw_wire_retained =
        check(raw_state.response == ResponseState::Raw(OwnedResponseWire(wire)));
    let raw_inspection_is_non_consuming = check(
        inspection.is_some()
            && raw_state.response == raw_before_inspection
            && inspection
                .expect("raw inspection is present")
                .ttl
                .minimal_ttl
                == 60,
    );

    let mut malformed_state = state();
    malformed_state.set_raw_response(vec![0x80]);
    let malformed_result = malformed_state.inspect_response(&DnsResponseInspector);
    let malformed_raw_retained_as_error = check(
        matches!(
            malformed_result,
            Err(ResponseError::MalformedRawResponse { .. })
        ) && malformed_state.response == ResponseState::Raw(OwnedResponseWire(vec![0x80])),
    );

    let mut synthesized_state = state();
    synthesized_state
        .set_synthesized_response(0x0fff)
        .expect("maximum configured RCODE");
    let synthesized_rcode_max = synthesized_state
        .response
        .synthesized_rcode()
        .expect("synthesized response");

    Phase3bStateSnapshot {
        state_snapshot_is_owned_and_deterministic,
        raw_wire_retained,
        raw_inspection_is_non_consuming,
        malformed_raw_retained_as_error,
        synthesized_rcode_max,
    }
}

fn program_contract_snapshot() -> Phase3bProgramSnapshot {
    let (zero_matcher_is_unconditional, missing_exec_is_noop, empty_exec_is_noop) =
        no_exec_contract_snapshot();
    let inline_order = inline_order_contract_snapshot();
    let (repeated_matcher_order, repeated_executable_order, same_fixture_repeated) =
        repeated_program_contract_snapshot();
    let (
        synthetic_inline_is_not_symbolic,
        typed_namespace_resolution,
        deterministic_validation_failure,
    ) = symbolic_program_contract_snapshot();

    Phase3bProgramSnapshot {
        zero_matcher_is_unconditional,
        missing_exec_is_noop,
        empty_exec_is_noop,
        inline_order,
        repeated_matcher_order,
        repeated_executable_order,
        same_fixture_repeated,
        synthetic_inline_is_not_symbolic,
        typed_namespace_resolution,
        deterministic_validation_failure,
    }
}

fn no_exec_contract_snapshot() -> (ContractCheck, ContractCheck, ContractCheck) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("zero")])),
                RuleSpec::new(Vec::new(), None),
                RuleSpec::new(Vec::new(), Some(Vec::new())),
                RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
            ],
        )],
        vec![
            fixture("zero", "zero", &calls),
            fixture("after", "after", &calls),
        ],
    )
    .validate()
    .expect("valid no-exec normalization program");
    let (result, _, _) = run_with_fuel(&program, 20);
    let log = calls.borrow().clone();
    let main = program.sequence(entry(&program)).expect("validated main");
    (
        check(
            result == Ok(ExecutionCompletion::Completed)
                && main.rules[0].matchers.is_empty()
                && log == ["zero", "after"],
        ),
        check(main.rules[1].executable.is_none() && log == ["zero", "after"]),
        check(main.rules[2].executable.is_none() && log == ["zero", "after"]),
    )
}

fn inline_order_contract_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("first"), fixture_exec("second")])),
                RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
            ],
        )],
        vec![
            fixture("first", "first", &calls),
            fixture("second", "second", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline order program");
    let (result, _, _) = run_with_fuel(&program, 20);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    let log = calls.borrow().clone();
    log[..2].to_vec()
}

fn repeated_program_contract_snapshot() -> (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>)
{
    let matcher_calls = Rc::new(RefCell::new(Vec::new()));
    let matcher_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    snapshot_matcher("m1", &matcher_calls, true, None, None),
                    snapshot_matcher("m2", &matcher_calls, true, None, None),
                ],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid repeated matcher program");
    let (matcher_result, _, _) = run_with_fuel(&matcher_program, 20);
    assert_eq!(matcher_result, Ok(ExecutionCompletion::Completed));
    let matcher_order = matcher_calls.borrow().clone();

    let executable_calls = Rc::new(RefCell::new(Vec::new()));
    let executable_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![
                fixture_exec("same"),
                fixture_exec("same"),
            ]))],
        )],
        vec![fixture("same", "same", &executable_calls)],
    )
    .validate()
    .expect("valid repeated executable program");
    let (executable_result, _, _) = run_with_fuel(&executable_program, 20);
    assert_eq!(executable_result, Ok(ExecutionCompletion::Completed));
    let executable_order = executable_calls.borrow().clone();
    (matcher_order, executable_order.clone(), executable_order)
}

fn symbolic_program_contract_snapshot() -> (ContractCheck, ContractCheck, ContractCheck) {
    (
        synthetic_inline_snapshot(),
        typed_namespace_snapshot(),
        deterministic_validation_failure_snapshot(),
    )
}

fn synthetic_inline_snapshot() -> ContractCheck {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let inline_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![
                fixture_exec("first"),
                fixture_exec("second"),
            ]))],
        )],
        vec![
            fixture("first", "first", &calls),
            fixture("second", "second", &calls),
        ],
    )
    .validate()
    .expect("valid synthetic inline program");
    let inline_target = match &inline_program
        .sequence(entry(&inline_program))
        .expect("inline main")
        .rules[0]
        .executable
    {
        Some(ValidatedExecutable::Inline { target }) => *target,
        other => panic!("expected synthetic inline, got {other:?}"),
    };
    let symbolic_try = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Sequence(SequenceRef::new("<inline:0>")),
            }]))],
        )],
        Vec::new(),
    );
    check(
        inline_program.sequence_id("<inline:0>").is_none()
            && inline_program.sequence(inline_target).is_some()
            && matches!(
                symbolic_try.validate(),
                Err(ProgramError::MissingSequence(name)) if name == "<inline:0>"
            ),
    )
}

fn typed_namespace_snapshot() -> ContractCheck {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let typed_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Sequence(SequenceRef::new("same")),
                    }])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Fixture(FixtureRef::new("same")),
                    }])),
                ],
            ),
            SequenceSpec::new("same", Vec::new()),
        ],
        vec![fixture("same", "same", &calls)],
    )
    .validate()
    .expect("valid typed namespace program");
    let typed_main = typed_program
        .sequence(entry(&typed_program))
        .expect("typed main");
    check(
        matches!(
            typed_main.rules[0].executable,
            Some(ValidatedExecutable::Try {
                target: ExecutableTarget::Sequence(_)
            })
        ) && matches!(
            typed_main.rules[1].executable,
            Some(ValidatedExecutable::Try {
                target: ExecutableTarget::Fixture(_)
            })
        ),
    )
}

fn deterministic_validation_failure_snapshot() -> ContractCheck {
    let duplicate_one = ProgramSpec::new(
        vec![
            SequenceSpec::new("same", Vec::new()),
            SequenceSpec::new("same", Vec::new()),
        ],
        Vec::new(),
    )
    .validate();
    let duplicate_two = ProgramSpec::new(
        vec![
            SequenceSpec::new("same", Vec::new()),
            SequenceSpec::new("same", Vec::new()),
        ],
        Vec::new(),
    )
    .validate();
    check(
        matches!(
            &duplicate_one,
            Err(ProgramError::DuplicateSequenceName(name)) if name == "same"
        ) && matches!(
            &duplicate_two,
            Err(ProgramError::DuplicateSequenceName(name)) if name == "same"
        ),
    )
}

struct SnapshotMatcher {
    label: &'static str,
    calls: Rc<RefCell<Vec<&'static str>>>,
    matched: bool,
    mutation: Option<StateMutation>,
    error: bool,
}

impl Matcher for SnapshotMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        self.calls.borrow_mut().push(self.label);
        if self.error {
            return Err(MatcherError::new("snapshot matcher error"));
        }
        Ok(MatchOutcome::new(self.matched, self.mutation.clone()))
    }
}

fn snapshot_matcher(
    label: &'static str,
    calls: &Rc<RefCell<Vec<&'static str>>>,
    matched: bool,
    mutation: Option<StateMutation>,
    error: Option<bool>,
) -> MatcherSpecInput {
    MatcherSpecInput::new(
        Box::new(SnapshotMatcher {
            label,
            calls: Rc::clone(calls),
            matched,
            mutation,
            error: error.unwrap_or(false),
        }),
        false,
        DispatchMetadata::None,
    )
}

fn snapshot_reverse_matcher(
    calls: &Rc<RefCell<Vec<&'static str>>>,
    metadata: DispatchMetadata,
) -> MatcherSpecInput {
    MatcherSpecInput::new(
        Box::new(SnapshotMatcher {
            label: "reverse",
            calls: Rc::clone(calls),
            matched: true,
            mutation: Some(StateMutation::AddMark(49)),
            error: false,
        }),
        true,
        metadata,
    )
}

fn snapshot_metadata_matcher(
    label: &'static str,
    calls: &Rc<RefCell<Vec<&'static str>>>,
    metadata: DispatchMetadata,
) -> MatcherSpecInput {
    MatcherSpecInput::new(
        Box::new(SnapshotMatcher {
            label,
            calls: Rc::clone(calls),
            matched: true,
            mutation: None,
            error: false,
        }),
        false,
        metadata,
    )
}

fn metadata_label_for_qtype(qtype: u16, metadata: DispatchMetadata) -> Option<String> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![snapshot_metadata_matcher("metadata", &calls, metadata)],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid metadata program");
    let mut state = state();
    state.query.question.qtype = qtype;
    let mut control = ExecutionControl::with_fuel(2);
    execute(&program, entry(&program), &mut state, &mut control).expect("metadata execution");
    state.routing.domain_set
}

fn matcher_contract_snapshot() -> Phase3bMatcherSnapshot {
    let matcher_order = matcher_order_snapshot();
    let (false_short_circuit, error_short_circuit, mutation_survives_later_error) =
        matcher_short_circuit_snapshot();
    let (reverse_preserves_mutation, reversed_membership_claims_positive_label) =
        reverse_matcher_snapshot();
    let (qname_label, switch6_label, switch5_labels, domain_set_write_once) =
        routing_matcher_snapshot();
    Phase3bMatcherSnapshot {
        matcher_order,
        false_short_circuit,
        error_short_circuit,
        mutation_survives_later_error,
        reverse_preserves_mutation,
        reversed_membership_claims_positive_label,
        qname_label,
        switch6_label,
        switch5_labels,
        domain_set_write_once,
    }
}

fn matcher_order_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    snapshot_matcher("m1", &calls, true, None, None),
                    snapshot_matcher("m2", &calls, true, None, None),
                ],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid matcher order program");
    let (result, _, _) = run_with_fuel(&program, 2);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn matcher_short_circuit_snapshot() -> (ContractCheck, ContractCheck, ContractCheck) {
    let false_calls = Rc::new(RefCell::new(Vec::new()));
    let false_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    snapshot_matcher("false", &false_calls, false, None, None),
                    snapshot_matcher("later", &false_calls, true, None, None),
                ],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid false short-circuit program");
    let (false_result, _, _) = run_with_fuel(&false_program, 1);
    let false_short_circuit = check(
        false_result == Ok(ExecutionCompletion::Completed)
            && false_calls.borrow().as_slice() == ["false"],
    );

    let error_calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    snapshot_matcher("before", &error_calls, true, None, None),
                    snapshot_matcher("error", &error_calls, true, None, Some(true)),
                ],
                Some(vec![fixture_exec("skipped")]),
            )],
        )],
        vec![fixture("skipped", "skipped", &error_calls)],
    )
    .validate()
    .expect("valid matcher error short-circuit program");
    let (error_result, _, _) = run_with_fuel(&error_program, 3);
    let error_short_circuit = check(
        matches!(
            error_result,
            Err(ExecutionError::Matcher(MatcherError::Failed(message)))
                if message == "snapshot matcher error"
        ) && error_calls.borrow().as_slice() == ["before", "error"],
    );

    let mutation_calls = Rc::new(RefCell::new(Vec::new()));
    let mutation_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    snapshot_matcher(
                        "mutate",
                        &mutation_calls,
                        true,
                        Some(StateMutation::AddMark(7)),
                        None,
                    ),
                    snapshot_matcher("error", &mutation_calls, true, None, Some(true)),
                ],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid mutation error program");
    let (mutation_result, _, mutation_state) = run_with_fuel(&mutation_program, 2);
    let mutation_survives_later_error = check(
        matches!(
            mutation_result,
            Err(ExecutionError::Matcher(MatcherError::Failed(_)))
        ) && mutation_state.marks.contains(&7),
    );
    (
        false_short_circuit,
        error_short_circuit,
        mutation_survives_later_error,
    )
}

fn reverse_matcher_snapshot() -> (ContractCheck, ContractCheck) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![snapshot_reverse_matcher(
                    &calls,
                    DispatchMetadata::AnonymousQname {
                        rule_name: "positive-rule".to_owned(),
                    },
                )],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid reverse matcher program");
    let (result, _, state) = run_with_fuel(&program, 1);
    (
        check(result == Ok(ExecutionCompletion::Completed) && state.marks.contains(&49)),
        check(state.routing.domain_set.is_some()),
    )
}

fn routing_matcher_snapshot() -> (Option<String>, Option<String>, Vec<String>, ContractCheck) {
    let qname_label = metadata_label_for_qtype(
        1,
        DispatchMetadata::AnonymousQname {
            rule_name: "qname-rule".to_owned(),
        },
    );
    let switch6_label = metadata_label_for_qtype(28, DispatchMetadata::Switch6);
    let switch5_labels = [6, 12, 65]
        .into_iter()
        .map(|qtype| metadata_label_for_qtype(qtype, DispatchMetadata::Switch5))
        .collect::<Option<Vec<_>>>()
        .expect("switch5 labels");

    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::new(
                    vec![snapshot_metadata_matcher(
                        "first",
                        &calls,
                        DispatchMetadata::AnonymousQname {
                            rule_name: "first".to_owned(),
                        },
                    )],
                    None,
                ),
                RuleSpec::new(
                    vec![snapshot_metadata_matcher(
                        "second",
                        &calls,
                        DispatchMetadata::AnonymousQname {
                            rule_name: "second".to_owned(),
                        },
                    )],
                    None,
                ),
            ],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid write-once program");
    let (_, _, state) = run_with_fuel(&program, 2);
    (
        qname_label,
        switch6_label,
        switch5_labels,
        check(state.routing.domain_set.as_deref() == Some("first")),
    )
}

fn root_goto_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("caller-skipped")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
            ),
        ],
        vec![
            fixture("target", "target", &calls),
            fixture("caller-skipped", "caller-skipped", &calls),
        ],
    )
    .validate()
    .expect("valid root goto program");
    let (result, _, _) = run_with_fuel(&program, 20);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn root_jump_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
            ),
        ],
        vec![
            fixture("target", "target", &calls),
            fixture("after", "after", &calls),
        ],
    )
    .validate()
    .expect("valid root jump program");
    let (result, _, _) = run_with_fuel(&program, 20);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn try_sequence_snapshot() -> (Vec<&'static str>, Vec<&'static str>) {
    let normal_calls = Rc::new(RefCell::new(Vec::new()));
    let normal_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                ],
            ),
            SequenceSpec::new(
                "child",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("child")]))],
            ),
        ],
        vec![
            fixture("child", "child", &normal_calls),
            fixture("after", "after", &normal_calls),
        ],
    )
    .validate()
    .expect("valid try normal program");
    let (normal_result, _, _) = run_with_fuel(&normal_program, 20);
    assert_eq!(normal_result, Ok(ExecutionCompletion::Completed));
    let try_normal_log = normal_calls.borrow().clone();

    let exit_calls = Rc::new(RefCell::new(Vec::new()));
    let exit_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                ],
            ),
            SequenceSpec::new(
                "child",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
            ),
        ],
        vec![fixture("after", "after", &exit_calls)],
    )
    .validate()
    .expect("valid try exit program");
    let (exit_result, _, _) = run_with_fuel(&exit_program, 20);
    assert_eq!(exit_result, Ok(ExecutionCompletion::Completed));
    let try_exit_log = exit_calls.borrow().clone();

    (try_normal_log, try_exit_log)
}

fn try_fixture_snapshot() -> (Vec<&'static str>, bool) {
    let fixture_calls = Rc::new(RefCell::new(Vec::new()));
    let fixture_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Fixture(FixtureRef::new("fixture")),
                }])),
                RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
            ],
        )],
        vec![
            fixture("fixture", "fixture", &fixture_calls),
            fixture("after", "after", &fixture_calls),
        ],
    )
    .validate()
    .expect("valid try fixture program");
    let (fixture_result, _, _) = run_with_fuel(&fixture_program, 20);
    assert_eq!(fixture_result, Ok(ExecutionCompletion::Completed));
    let try_fixture_log = fixture_calls.borrow().clone();

    let error_calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("error")),
            }]))],
        )],
        vec![error_fixture("error", "error", &error_calls)],
    )
    .validate()
    .expect("valid try ordinary error program");
    let (error_result, _, _) = run_with_fuel(&error_program, 2);
    let try_ordinary_error_propagates = matches!(
        error_result,
        Err(ExecutionError::Executor(ExecutorError::Failed(message)))
            if message == "ordinary executor error"
    ) && error_calls.borrow().as_slice() == ["error"];

    (try_fixture_log, try_ordinary_error_propagates)
}

fn try_control_snapshot() -> Phase3bControlSnapshot {
    let (try_normal_log, try_exit_log) = try_sequence_snapshot();
    let (try_fixture_log, try_ordinary_error_propagates) = try_fixture_snapshot();

    Phase3bControlSnapshot {
        root_goto_log: root_goto_snapshot(),
        root_jump_log: root_jump_snapshot(),
        top_return: root_terminal_result(ExecutableSpec::Return),
        accept: root_terminal_result(ExecutableSpec::Accept),
        default_reject_rcode: root_reject_rcode(ExecutableSpec::default_reject()),
        explicit_reject_rcode: root_reject_rcode(ExecutableSpec::Reject { rcode: 0x0fff }),
        exit: root_terminal_result(ExecutableSpec::Exit),
        try_normal_log,
        try_fixture_log,
        try_exit_log,
        try_ordinary_error_propagates,
        inline_fallthrough_log: inline_fallthrough_snapshot(),
        inline_return_log: inline_terminal_log(ExecutableSpec::Return).1,
        inline_accept_log: inline_terminal_log(ExecutableSpec::Accept).1,
        inline_reject_log: inline_terminal_log(ExecutableSpec::default_reject()).1,
        inline_jump_log: inline_jump_snapshot(),
        inline_goto_log: inline_goto_snapshot(),
        inline_exit: inline_exit_snapshot(),
        inline_try_normal_log: inline_try_continuation_snapshot(false),
        inline_try_exit_log: inline_try_continuation_snapshot(true),
    }
}

fn root_terminal_result(executable: ExecutableSpec) -> ExecutionCompletion {
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![executable]))],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid root terminal program");
    let (result, _, _) = run_with_fuel(&program, 2);
    result.expect("root terminal completion")
}

fn root_reject_rcode(executable: ExecutableSpec) -> u16 {
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![executable]))],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid root reject program");
    let (result, _, state) = run_with_fuel(&program, 2);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    state.response.synthesized_rcode().expect("reject response")
}

fn inline_terminal_log(terminal: ExecutableSpec) -> (ExecutionCompletion, Vec<&'static str>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![
                    fixture_exec("exec1"),
                    terminal,
                    fixture_exec("inline-skipped"),
                ])),
                RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
            ],
        )],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("inline-skipped", "inline-skipped", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline terminal program");
    let (result, _, _) = run_with_fuel(&program, 20);
    (
        result.expect("inline terminal completion"),
        calls.borrow().clone(),
    )
}

fn inline_fallthrough_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("exec1"), fixture_exec("exec2")])),
                RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
            ],
        )],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("exec2", "exec2", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline fall-through program");
    let (result, _, _) = run_with_fuel(&program, 20);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn inline_jump_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Jump {
                            target: SequenceRef::new("target"),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("target", "target", &calls),
            fixture("exec3", "exec3", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline jump program");
    let (result, _, _) = run_with_fuel(&program, 30);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn inline_goto_snapshot() -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Goto {
                            target: SequenceRef::new("target"),
                        },
                        fixture_exec("inline-skipped"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("target", "target", &calls),
            fixture("inline-skipped", "inline-skipped", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline goto program");
    let (result, _, _) = run_with_fuel(&program, 30);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn inline_exit_snapshot() -> ExecutionCompletion {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![
                    fixture_exec("exec1"),
                    ExecutableSpec::Exit,
                    fixture_exec("inline-skipped"),
                ])),
                RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
            ],
        )],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("inline-skipped", "inline-skipped", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline exit program");
    let (result, _, _) = run_with_fuel(&program, 20);
    assert_eq!(&*calls.borrow(), &["exec1"]);
    result.expect("inline exit completion")
}

fn inline_try_continuation_snapshot(exit_child: bool) -> Vec<&'static str> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let child_rule = if exit_child {
        RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))
    } else {
        RuleSpec::unconditional(Some(vec![fixture_exec("child")]))
    };
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new("child", vec![child_rule]),
        ],
        vec![
            fixture("exec1", "exec1", &calls),
            fixture("child", "child", &calls),
            fixture("exec3", "exec3", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid inline try continuation program");
    let (result, _, _) = run_with_fuel(&program, 30);
    assert_eq!(result, Ok(ExecutionCompletion::Completed));
    calls.borrow().clone()
}

fn safety_contract_snapshot() -> Phase3bSafetySnapshot {
    let direct_calls = Rc::new(RefCell::new(Vec::new()));
    let direct_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec("fixture")]))],
        )],
        vec![fixture("fixture", "fixture", &direct_calls)],
    )
    .validate()
    .expect("valid direct fuel program");
    let (direct_result, direct_fixture_remaining_fuel, _) = run_with_fuel(&direct_program, 1);
    assert_eq!(direct_result, Ok(ExecutionCompletion::Completed));
    assert_eq!(&*direct_calls.borrow(), &["fixture"]);

    let try_calls = Rc::new(RefCell::new(Vec::new()));
    let try_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("fixture")),
            }]))],
        )],
        vec![fixture("fixture", "fixture", &try_calls)],
    )
    .validate()
    .expect("valid try fuel program");
    let (try_result, try_fixture_remaining_fuel, _) = run_with_fuel(&try_program, 2);
    assert_eq!(try_result, Ok(ExecutionCompletion::Completed));
    assert_eq!(&*try_calls.borrow(), &["fixture"]);

    let (try_fixture_budget, try_fixture_ordinary_error, try_fixture_exit) =
        try_fixture_safety_results();
    let try_fixture_cancelled = try_fixture_cancelled_result();
    let (goto_cycle, jump_cycle) = cycle_results();
    let nested_try_cycle = nested_try_cycle_result();
    let nested_try_cancelled = nested_try_cancelled_result();
    let external_cancel = external_cancel_result();

    let budget_calls = Rc::new(RefCell::new(Vec::new()));
    let budget_program = inline_try_program(
        vec![RuleSpec::unconditional(Some(vec![fixture_exec("child")]))],
        &budget_calls,
    );
    let (inline_budget, _, _) = run_with_fuel(&budget_program, 3);

    let (inline_cancelled, inline_cancelled_log) = inline_cancel_result();
    let (priority_cancelled, priority_budget, priority_ordinary_error, priority_exit) =
        priority_results();

    Phase3bSafetySnapshot {
        direct_fixture_remaining_fuel,
        try_fixture_remaining_fuel,
        try_fixture_budget,
        try_fixture_ordinary_error,
        try_fixture_exit,
        try_fixture_cancelled,
        goto_cycle,
        jump_cycle,
        nested_try_cycle,
        nested_try_cancelled,
        external_cancel,
        inline_budget: inline_budget.expect_err("inline budget must propagate"),
        inline_budget_log: budget_calls.borrow().clone(),
        inline_cancelled,
        inline_cancelled_log,
        priority_cancelled,
        priority_budget,
        priority_ordinary_error,
        priority_exit,
    }
}

fn try_fixture_safety_results() -> (ExecutionError, ExecutionError, ExecutionCompletion) {
    let error_calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("error")),
            }]))],
        )],
        vec![error_fixture("error", "error", &error_calls)],
    )
    .validate()
    .expect("valid try fixture error program");
    let (budget_result, _, _) = run_with_fuel(&error_program, 1);
    assert!(error_calls.borrow().is_empty());
    let try_fixture_budget = budget_result.expect_err("try fixture budget");
    let (error_result, _, _) = run_with_fuel(&error_program, 2);
    assert_eq!(&*error_calls.borrow(), &["error"]);
    let try_fixture_ordinary_error = error_result.expect_err("try fixture ordinary error");

    let exit_calls = Rc::new(RefCell::new(Vec::new()));
    let exit_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("exit")),
            }]))],
        )],
        vec![outcome_fixture(
            "exit",
            "exit",
            &exit_calls,
            ExecutorOutcome::Exit,
        )],
    )
    .validate()
    .expect("valid try fixture exit program");
    let (exit_result, _, _) = run_with_fuel(&exit_program, 2);
    assert_eq!(&*exit_calls.borrow(), &["exit"]);
    (
        try_fixture_budget,
        try_fixture_ordinary_error,
        exit_result.expect("try catches fixture exit"),
    )
}

fn try_fixture_cancelled_result() -> ExecutionError {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    token.cancel();
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("fixture")),
            }]))],
        )],
        vec![fixture("fixture", "fixture", &calls)],
    )
    .validate()
    .expect("valid try fixture cancellation program");
    let (result, _, _) = run_with_token(&program, 1, token);
    assert!(calls.borrow().is_empty());
    result.expect_err("try fixture cancellation")
}

fn cycle_results() -> (ExecutionError, ExecutionError) {
    let goto_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("target"),
                }]))],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("main"),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid goto cycle");
    let (goto_result, _, _) = run_with_fuel(&goto_program, 4);

    let jump_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                    target: SequenceRef::new("target"),
                }]))],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                    target: SequenceRef::new("main"),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid jump cycle");
    let (jump_result, _, _) = run_with_fuel(&jump_program, 4);
    (
        goto_result.expect_err("goto cycle budget"),
        jump_result.expect_err("jump cycle budget"),
    )
}

fn nested_try_cycle_result() -> ExecutionError {
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                }]))],
            ),
            SequenceSpec::new(
                "a",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("b")),
                }]))],
            ),
            SequenceSpec::new(
                "b",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                }]))],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid nested try cycle");
    let (result, _, _) = run_with_fuel(&program, 3);
    result.expect_err("nested try cycle budget")
}

fn nested_try_cancelled_result() -> ExecutionError {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("a")),
                }]))],
            ),
            SequenceSpec::new(
                "a",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("b")),
                }]))],
            ),
            SequenceSpec::new(
                "b",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                ],
            ),
        ],
        vec![
            FixtureSpec::new(
                "cancel",
                Box::new(CancellingExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    token: token.clone(),
                }),
            ),
            fixture("after", "after", &calls),
        ],
    )
    .validate()
    .expect("valid nested try cancellation program");
    let (result, _, _) = run_with_token(&program, 20, token);
    assert_eq!(&*calls.borrow(), &["cancel"]);
    result.expect_err("nested try cancellation")
}

fn external_cancel_result() -> ExecutionError {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let entered = Arc::new(Barrier::new(2));
    let released = Arc::new(Barrier::new(2));
    let canceller_token = token.clone();
    let canceller_entered = Arc::clone(&entered);
    let canceller_released = Arc::clone(&released);
    let canceller = thread::spawn(move || {
        canceller_entered.wait();
        canceller_token.cancel();
        canceller_released.wait();
    });
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
            ],
        )],
        vec![
            FixtureSpec::new(
                "cancel",
                Box::new(ExternallyCancelledExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    entered,
                    released,
                }),
            ),
            fixture("after", "after", &calls),
        ],
    )
    .validate()
    .expect("valid external cancellation program");
    let (result, _, _) = run_with_token(&program, 10, token);
    canceller.join().expect("cancellation thread completes");
    assert_eq!(&*calls.borrow(), &["cancel"]);
    result.expect_err("external cancellation")
}

fn inline_cancel_result() -> (ExecutionError, Vec<&'static str>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let token = CancellationToken::new();
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "child",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("cancel")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("child-after")])),
                ],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &calls),
            FixtureSpec::new(
                "cancel",
                Box::new(CancellingExecutor {
                    label: "cancel",
                    calls: Rc::clone(&calls),
                    token: token.clone(),
                }),
            ),
            fixture("child-after", "child-after", &calls),
            fixture("exec3", "exec3", &calls),
            fixture("outer", "outer", &calls),
        ],
    )
    .validate()
    .expect("valid canonical inline cancellation program");
    let (result, _, _) = run_with_token(&program, 20, token);
    (
        result.expect_err("inline cancellation"),
        calls.borrow().clone(),
    )
}

fn priority_results() -> (
    ExecutionError,
    ExecutionError,
    ExecutionError,
    ExecutionCompletion,
) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec("error")]))],
        )],
        vec![error_fixture("error", "error", &calls)],
    )
    .validate()
    .expect("valid priority error program");
    let cancelled_token = CancellationToken::new();
    cancelled_token.cancel();
    let (cancelled_result, _, _) = run_with_token(&error_program, 0, cancelled_token);
    let priority_cancelled = cancelled_result.expect_err("cancelled priority");
    let (budget_result, _, _) = run_with_fuel(&error_program, 0);
    let priority_budget = budget_result.expect_err("budget priority");
    let (ordinary_result, _, _) = run_with_fuel(&error_program, 1);
    let priority_ordinary_error = ordinary_result.expect_err("ordinary error priority");

    let exit_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid priority exit program");
    let (exit_result, _, _) = run_with_fuel(&exit_program, 1);
    (
        priority_cancelled,
        priority_budget,
        priority_ordinary_error,
        exit_result.expect("exit priority result"),
    )
}

fn build_phase3b_contract_snapshot() -> Phase3bContractSnapshot {
    Phase3bContractSnapshot {
        state: state_contract_snapshot(),
        program: program_contract_snapshot(),
        matcher: matcher_contract_snapshot(),
        control: try_control_snapshot(),
        safety: safety_contract_snapshot(),
    }
}

#[test]
fn phase3b_reviewed_contract_snapshot_is_complete_and_deterministic() {
    assert_eq!(
        build_phase3b_contract_snapshot(),
        Phase3bContractSnapshot {
            state: Phase3bStateSnapshot {
                state_snapshot_is_owned_and_deterministic: ContractCheck::Passed,
                raw_wire_retained: ContractCheck::Passed,
                raw_inspection_is_non_consuming: ContractCheck::Passed,
                malformed_raw_retained_as_error: ContractCheck::Passed,
                synthesized_rcode_max: 0x0fff,
            },
            program: Phase3bProgramSnapshot {
                zero_matcher_is_unconditional: ContractCheck::Passed,
                missing_exec_is_noop: ContractCheck::Passed,
                empty_exec_is_noop: ContractCheck::Passed,
                inline_order: vec!["first", "second"],
                repeated_matcher_order: vec!["m1", "m2"],
                repeated_executable_order: vec!["same", "same"],
                same_fixture_repeated: vec!["same", "same"],
                synthetic_inline_is_not_symbolic: ContractCheck::Passed,
                typed_namespace_resolution: ContractCheck::Passed,
                deterministic_validation_failure: ContractCheck::Passed,
            },
            matcher: Phase3bMatcherSnapshot {
                matcher_order: vec!["m1", "m2"],
                false_short_circuit: ContractCheck::Passed,
                error_short_circuit: ContractCheck::Passed,
                mutation_survives_later_error: ContractCheck::Passed,
                reverse_preserves_mutation: ContractCheck::Passed,
                reversed_membership_claims_positive_label: ContractCheck::Failed,
                qname_label: Some("qname-rule".to_owned()),
                switch6_label: Some("BANAAAA".to_owned()),
                switch5_labels: vec![
                    "BANSOA".to_owned(),
                    "BANPTR".to_owned(),
                    "BANHTTPS".to_owned(),
                ],
                domain_set_write_once: ContractCheck::Passed,
            },
            control: Phase3bControlSnapshot {
                root_goto_log: vec!["target"],
                root_jump_log: vec!["target", "after"],
                top_return: ExecutionCompletion::Completed,
                accept: ExecutionCompletion::Completed,
                default_reject_rcode: 5,
                explicit_reject_rcode: 0x0fff,
                exit: ExecutionCompletion::Exited,
                try_normal_log: vec!["child", "after"],
                try_fixture_log: vec!["fixture", "after"],
                try_exit_log: vec!["after"],
                try_ordinary_error_propagates: true,
                inline_fallthrough_log: vec!["exec1", "exec2", "outer"],
                inline_return_log: vec!["exec1", "outer"],
                inline_accept_log: vec!["exec1", "outer"],
                inline_reject_log: vec!["exec1", "outer"],
                inline_jump_log: vec!["exec1", "target", "exec3", "outer"],
                inline_goto_log: vec!["exec1", "target", "outer"],
                inline_exit: ExecutionCompletion::Exited,
                inline_try_normal_log: vec!["exec1", "child", "exec3", "outer"],
                inline_try_exit_log: vec!["exec1", "exec3", "outer"],
            },
            safety: Phase3bSafetySnapshot {
                direct_fixture_remaining_fuel: 0,
                try_fixture_remaining_fuel: 0,
                try_fixture_budget: ExecutionError::BudgetExceeded,
                try_fixture_ordinary_error: ExecutionError::Executor(ExecutorError::Failed(
                    "ordinary executor error".to_owned()
                ),),
                try_fixture_exit: ExecutionCompletion::Completed,
                try_fixture_cancelled: ExecutionError::Cancelled,
                goto_cycle: ExecutionError::BudgetExceeded,
                jump_cycle: ExecutionError::BudgetExceeded,
                nested_try_cycle: ExecutionError::BudgetExceeded,
                nested_try_cancelled: ExecutionError::Cancelled,
                external_cancel: ExecutionError::Cancelled,
                inline_budget: ExecutionError::BudgetExceeded,
                inline_budget_log: vec!["exec1"],
                inline_cancelled: ExecutionError::Cancelled,
                inline_cancelled_log: vec!["exec1", "cancel"],
                priority_cancelled: ExecutionError::Cancelled,
                priority_budget: ExecutionError::BudgetExceeded,
                priority_ordinary_error: ExecutionError::Executor(ExecutorError::Failed(
                    "ordinary executor error".to_owned()
                ),),
                priority_exit: ExecutionCompletion::Exited,
            },
        }
    );
}
