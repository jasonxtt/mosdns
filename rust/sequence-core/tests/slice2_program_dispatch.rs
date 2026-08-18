use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{
    DispatchMetadata, ExecutableSpec, ExecutableTarget, ExecutableTargetSpec, ExecutionCompletion,
    ExecutionControl, ExecutionError, ExecutionState, Executor, ExecutorError, ExecutorOutcome,
    FixtureRef, FixtureSpec, MatchOutcome, Matcher, MatcherError, MatcherSpecInput, ProgramError,
    ProgramSpec, RuleSpec, SequenceId, SequenceRef, SequenceSpec, StateMutation,
    ValidatedExecutable, execute,
};

fn state(qtype: u16) -> ExecutionState {
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
            qtype,
            qclass: 1,
        },
    )
}

struct NoopExecutor;

impl Executor for NoopExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        Ok(ExecutorOutcome::Continue)
    }
}

fn fixture(name: &str) -> FixtureSpec {
    FixtureSpec::new(name, Box::new(NoopExecutor))
}

struct NoopMatcher;

impl Matcher for NoopMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(true, None))
    }
}

fn fixture_exec(name: &str) -> ExecutableSpec {
    ExecutableSpec::Fixture {
        target: FixtureRef::new(name),
    }
}

fn entry(program: &mosdns_sequence_core::ValidatedProgram) -> SequenceId {
    program.sequence_id("main").expect("main entry")
}

fn validation_error(spec: ProgramSpec) -> ProgramError {
    match spec.validate() {
        Ok(_) => panic!("malformed program unexpectedly validated"),
        Err(error) => error,
    }
}

fn normalization_spec() -> ProgramSpec {
    ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(None),
                    RuleSpec::unconditional(Some(Vec::new())),
                    RuleSpec::unconditional(Some(vec![fixture_exec("same")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("same"), fixture_exec("same")])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: mosdns_sequence_core::ExecutableTargetSpec::Sequence(
                            SequenceRef::new("same"),
                        ),
                    }])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: mosdns_sequence_core::ExecutableTargetSpec::Fixture(
                            FixtureRef::new("same"),
                        ),
                    }])),
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("same"),
                        fixture_exec("same"),
                        fixture_exec("same"),
                    ])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![
                    fixture_exec("same"),
                    fixture_exec("same"),
                ]))],
            ),
            SequenceSpec::new("same", Vec::new()),
        ],
        vec![fixture("same")],
    )
}

fn inline_target(
    program: &mosdns_sequence_core::ValidatedProgram,
    sequence: SequenceId,
    rule: usize,
) -> SequenceId {
    match &program.sequence(sequence).expect("sequence").rules[rule].executable {
        Some(ValidatedExecutable::Inline { target }) => *target,
        _ => panic!("multi-exec must normalize to Inline"),
    }
}

fn assert_inline_fixture_rules(
    program: &mosdns_sequence_core::ValidatedProgram,
    inline: SequenceId,
    expected_len: usize,
) {
    let inline = program.sequence(inline).expect("synthetic inline sequence");
    assert_eq!(inline.rules.len(), expected_len);
    for rule in &inline.rules {
        assert_eq!(
            rule.executable,
            Some(ValidatedExecutable::Fixture {
                target: mosdns_sequence_core::ExecutableId(0)
            })
        );
    }
}

#[test]
fn normalization_assigns_stable_ids_and_resolves_typed_targets() {
    let program = normalization_spec().validate().expect("valid program");

    assert_eq!(program.sequence_id("main"), Some(SequenceId(0)));
    assert_eq!(program.sequence_id("target"), Some(SequenceId(1)));
    assert_eq!(program.sequence_id("same"), Some(SequenceId(2)));
    assert_eq!(program.sequence_id("<inline:0>"), None);
    assert_eq!(program.sequence_id("<inline:1>"), None);
    assert_eq!(program.sequence_id("<inline:2>"), None);
    assert_eq!(program.sequences.len(), 6);

    let main = program.sequence(entry(&program)).expect("main sequence");
    assert!(main.rules[0].executable.is_none());
    assert!(main.rules[1].executable.is_none());
    assert!(matches!(
        main.rules[2].executable,
        Some(ValidatedExecutable::Fixture {
            target: mosdns_sequence_core::ExecutableId(0)
        })
    ));

    assert_eq!(inline_target(&program, SequenceId(0), 3), SequenceId(3));
    assert_inline_fixture_rules(&program, SequenceId(3), 2);
    assert_eq!(inline_target(&program, SequenceId(0), 8), SequenceId(4));
    assert_inline_fixture_rules(&program, SequenceId(4), 3);
    assert_eq!(inline_target(&program, SequenceId(1), 0), SequenceId(5));
    assert_inline_fixture_rules(&program, SequenceId(5), 2);

    assert_eq!(
        main.rules[4].executable,
        Some(ValidatedExecutable::Goto {
            target: SequenceId(1)
        })
    );
    assert_eq!(
        main.rules[5].executable,
        Some(ValidatedExecutable::Jump {
            target: SequenceId(1)
        })
    );
    assert_eq!(
        main.rules[6].executable,
        Some(ValidatedExecutable::Try {
            target: ExecutableTarget::Sequence(SequenceId(2))
        })
    );
    assert_eq!(
        main.rules[7].executable,
        Some(ValidatedExecutable::Try {
            target: ExecutableTarget::Fixture(mosdns_sequence_core::ExecutableId(0))
        })
    );
}

#[test]
fn validation_rejects_missing_typed_targets_wrong_namespaces_and_malformed_inputs() {
    let missing_jump = validation_error(ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                target: SequenceRef::new("missing"),
            }]))],
        )],
        Vec::new(),
    ));
    assert_eq!(
        missing_jump,
        ProgramError::MissingSequence("missing".to_owned())
    );

    let missing_try_sequence = validation_error(ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Sequence(SequenceRef::new("missing")),
            }]))],
        )],
        Vec::new(),
    ));
    assert_eq!(
        missing_try_sequence,
        ProgramError::MissingSequence("missing".to_owned())
    );

    let missing_try_fixture = validation_error(ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                target: ExecutableTargetSpec::Fixture(FixtureRef::new("missing")),
            }]))],
        )],
        Vec::new(),
    ));
    assert_eq!(
        missing_try_fixture,
        ProgramError::MissingFixture("missing".to_owned())
    );

    let goto_fixture_name = validation_error(ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                target: SequenceRef::new("fixture-only"),
            }]))],
        )],
        vec![fixture("fixture-only")],
    ));
    assert_eq!(
        goto_fixture_name,
        ProgramError::MissingSequence("fixture-only".to_owned())
    );

    let try_sequence_name_as_fixture = validation_error(ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Fixture(FixtureRef::new("sequence-only")),
                }]))],
            ),
            SequenceSpec::new("sequence-only", Vec::new()),
        ],
        Vec::new(),
    ));
    assert_eq!(
        try_sequence_name_as_fixture,
        ProgramError::MissingFixture("sequence-only".to_owned())
    );

    assert_eq!(
        validation_error(ProgramSpec::new(
            vec![SequenceSpec::new("", Vec::new())],
            Vec::new(),
        )),
        ProgramError::EmptyName { kind: "sequence" }
    );
    assert_eq!(
        validation_error(ProgramSpec::new(
            vec![SequenceSpec::new("main", Vec::new())],
            vec![fixture("")],
        )),
        ProgramError::EmptyName { kind: "fixture" }
    );

    let mut missing_matcher =
        MatcherSpecInput::new(Box::new(NoopMatcher), false, DispatchMetadata::None);
    missing_matcher.matcher = None;
    assert_eq!(
        validation_error(ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::new(vec![missing_matcher], None)],
            )],
            Vec::new(),
        )),
        ProgramError::MissingMatcher
    );
}

struct ObservingMatcher {
    label: &'static str,
    matched: bool,
    mutation: Option<StateMutation>,
    requires_mark: Option<u32>,
    observed_required_mark: Rc<Cell<bool>>,
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl Matcher for ObservingMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        self.calls.borrow_mut().push(self.label);
        if let Some(mark) = self.requires_mark {
            self.observed_required_mark.set(state.marks.contains(&mark));
        }
        Ok(MatchOutcome::new(self.matched, self.mutation.clone()))
    }
}

struct RecordingExecutor {
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl Executor for RecordingExecutor {
    fn execute(&self, _state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        self.calls.borrow_mut().push("exec");
        Ok(ExecutorOutcome::Continue)
    }
}

struct StateWritingExecutor {
    mark: u32,
    outcome: Result<ExecutorOutcome, ExecutorError>,
}

impl Executor for StateWritingExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        state.marks.insert(self.mark);
        self.outcome.clone()
    }
}

fn state_writing_fixture(
    name: &str,
    mark: u32,
    outcome: Result<ExecutorOutcome, ExecutorError>,
) -> FixtureSpec {
    FixtureSpec::new(name, Box::new(StateWritingExecutor { mark, outcome }))
}

fn single_fixture_program(
    name: &str,
    fixture: FixtureSpec,
) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![fixture_exec(name)]))],
        )],
        vec![fixture],
    )
    .validate()
    .expect("valid single-fixture program")
}

struct MutationMatcher {
    mutation: StateMutation,
}

impl Matcher for MutationMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(true, Some(self.mutation.clone())))
    }
}

struct ErrorMatcher;

impl Matcher for ErrorMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Err(MatcherError::new("matcher failed"))
    }
}

fn caller_state(marker: u32) -> ExecutionState {
    let mut state = state(1);
    state.marks.insert(marker);
    state.routing.matched_rule_source = Some("caller-owned".to_owned());
    state
}

fn assert_caller_state(state: &ExecutionState, marker: u32) {
    let snapshot = state.snapshot();
    assert!(snapshot.marks.contains(&marker));
    assert_eq!(
        snapshot.routing.matched_rule_source.as_deref(),
        Some("caller-owned")
    );
}

#[test]
fn matcher_dispatch_applies_ordered_mutations_before_reverse_and_metadata() {
    let matcher_calls = Rc::new(RefCell::new(Vec::new()));
    let executor_calls = Rc::new(RefCell::new(Vec::new()));
    let observed_required_mark = Rc::new(Cell::new(false));
    let spec = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    MatcherSpecInput::new(
                        Box::new(ObservingMatcher {
                            label: "first",
                            matched: true,
                            mutation: Some(StateMutation::AddMark(7)),
                            requires_mark: None,
                            observed_required_mark: Rc::new(Cell::new(false)),
                            calls: Rc::clone(&matcher_calls),
                        }),
                        false,
                        DispatchMetadata::None,
                    ),
                    MatcherSpecInput::new(
                        Box::new(ObservingMatcher {
                            label: "second",
                            matched: true,
                            mutation: Some(StateMutation::AddMark(49)),
                            requires_mark: Some(7),
                            observed_required_mark: Rc::clone(&observed_required_mark),
                            calls: Rc::clone(&matcher_calls),
                        }),
                        false,
                        DispatchMetadata::AnonymousQname {
                            rule_name: "rule-a".to_owned(),
                        },
                    ),
                ],
                Some(vec![fixture_exec("exec")]),
            )],
        )],
        vec![FixtureSpec::new(
            "exec",
            Box::new(RecordingExecutor {
                calls: Rc::clone(&executor_calls),
            }),
        )],
    );

    let program = spec.validate().expect("valid matcher program");
    let mut state = state(1);
    let mut control = ExecutionControl::with_fuel(20);
    assert_eq!(
        execute(&program, entry(&program), &mut state, &mut control),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*matcher_calls.borrow(), &["first", "second"]);
    assert!(observed_required_mark.get());
    assert_eq!(&*executor_calls.borrow(), &["exec"]);
    assert!(state.marks.contains(&7));
    assert!(state.marks.contains(&49));
    assert_eq!(state.routing.domain_set.as_deref(), Some("rule-a"));
}

#[test]
fn reverse_keeps_matcher_mutation_but_skips_positive_metadata_and_executable() {
    let executor_calls = Rc::new(RefCell::new(Vec::new()));
    let spec = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![MatcherSpecInput::new(
                    Box::new(ObservingMatcher {
                        label: "reverse",
                        matched: true,
                        mutation: Some(StateMutation::AddMark(7)),
                        requires_mark: None,
                        observed_required_mark: Rc::new(Cell::new(false)),
                        calls: Rc::new(RefCell::new(Vec::new())),
                    }),
                    true,
                    DispatchMetadata::AnonymousQname {
                        rule_name: "must-not-appear".to_owned(),
                    },
                )],
                Some(vec![fixture_exec("exec")]),
            )],
        )],
        vec![FixtureSpec::new(
            "exec",
            Box::new(RecordingExecutor {
                calls: Rc::clone(&executor_calls),
            }),
        )],
    );

    let program = spec.validate().expect("valid reverse program");
    let mut state = state(1);
    let mut control = ExecutionControl::with_fuel(10);
    execute(&program, entry(&program), &mut state, &mut control).expect("reverse execution");
    assert!(state.marks.contains(&7));
    assert_eq!(state.routing.domain_set, None);
    assert!(executor_calls.borrow().is_empty());
}

fn assert_completed_state_observability() {
    let mut completed_state = caller_state(100);
    let completed_program = single_fixture_program(
        "completed",
        state_writing_fixture("completed", 101, Ok(ExecutorOutcome::Continue)),
    );
    let mut completed_control = ExecutionControl::with_fuel(10);
    assert_eq!(
        execute(
            &completed_program,
            entry(&completed_program),
            &mut completed_state,
            &mut completed_control,
        ),
        Ok(ExecutionCompletion::Completed)
    );
    assert_caller_state(&completed_state, 100);
    assert!(completed_state.marks.contains(&101));
}

fn assert_exited_state_observability() {
    let mut exited_state = caller_state(102);
    let exited_program = single_fixture_program(
        "exited",
        state_writing_fixture("exited", 103, Ok(ExecutorOutcome::Exit)),
    );
    let mut exited_control = ExecutionControl::with_fuel(10);
    assert_eq!(
        execute(
            &exited_program,
            entry(&exited_program),
            &mut exited_state,
            &mut exited_control,
        ),
        Ok(ExecutionCompletion::Exited)
    );
    assert_caller_state(&exited_state, 102);
    assert!(exited_state.marks.contains(&103));
}

fn assert_matcher_error_state_observability() {
    let mut matcher_error_state = caller_state(104);
    let matcher_error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::new(
                vec![
                    MatcherSpecInput::new(
                        Box::new(MutationMatcher {
                            mutation: StateMutation::AddMark(105),
                        }),
                        false,
                        DispatchMetadata::None,
                    ),
                    MatcherSpecInput::new(Box::new(ErrorMatcher), false, DispatchMetadata::None),
                ],
                None,
            )],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid matcher error program");
    let mut matcher_error_control = ExecutionControl::with_fuel(10);
    let matcher_error_result = execute(
        &matcher_error_program,
        entry(&matcher_error_program),
        &mut matcher_error_state,
        &mut matcher_error_control,
    );
    assert!(matches!(
        matcher_error_result,
        Err(ExecutionError::Matcher(MatcherError::Failed(message))) if message == "matcher failed"
    ));
    assert_caller_state(&matcher_error_state, 104);
    assert!(matcher_error_state.marks.contains(&105));
}

fn assert_executor_error_state_observability() {
    let mut executor_error_state = caller_state(106);
    let executor_error_program = single_fixture_program(
        "executor-error",
        state_writing_fixture(
            "executor-error",
            107,
            Err(ExecutorError::new("executor failed")),
        ),
    );
    let mut executor_error_control = ExecutionControl::with_fuel(10);
    let executor_error_result = execute(
        &executor_error_program,
        entry(&executor_error_program),
        &mut executor_error_state,
        &mut executor_error_control,
    );
    assert!(matches!(
        executor_error_result,
        Err(ExecutionError::Executor(ExecutorError::Failed(message)))
            if message == "executor failed"
    ));
    assert_caller_state(&executor_error_state, 106);
    assert!(executor_error_state.marks.contains(&107));
}

fn assert_cancelled_state_observability() {
    let mut cancelled_state = caller_state(108);
    let cancelled_before = cancelled_state.snapshot();
    let cancelled_program = single_fixture_program(
        "cancelled",
        state_writing_fixture("cancelled", 109, Ok(ExecutorOutcome::Continue)),
    );
    let mut cancelled_control = ExecutionControl::with_fuel(10);
    cancelled_control.cancel();
    assert_eq!(
        execute(
            &cancelled_program,
            entry(&cancelled_program),
            &mut cancelled_state,
            &mut cancelled_control,
        ),
        Err(ExecutionError::Cancelled)
    );
    assert_eq!(cancelled_state.snapshot(), cancelled_before);
}

fn assert_budget_exhausted_state_observability() {
    let mut budget_state = caller_state(110);
    let budget_before = budget_state.snapshot();
    let budget_program = single_fixture_program(
        "budget",
        state_writing_fixture("budget", 111, Ok(ExecutorOutcome::Continue)),
    );
    let mut budget_control = ExecutionControl::with_fuel(0);
    assert_eq!(
        execute(
            &budget_program,
            entry(&budget_program),
            &mut budget_state,
            &mut budget_control,
        ),
        Err(ExecutionError::BudgetExceeded)
    );
    assert_eq!(budget_state.snapshot(), budget_before);
}

#[test]
fn caller_owned_state_remains_observable_for_all_execution_outcomes() {
    assert_completed_state_observability();
    assert_exited_state_observability();
    assert_matcher_error_state_observability();
    assert_executor_error_state_observability();
    assert_cancelled_state_observability();
    assert_budget_exhausted_state_observability();
}
