use std::cell::RefCell;
use std::rc::Rc;

use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{
    ExecutableSpec, ExecutableTargetSpec, ExecutionCompletion, ExecutionControl, ExecutionError,
    ExecutionState, Executor, ExecutorError, ExecutorOutcome, FixtureRef, FixtureSpec,
    MatchOutcome, Matcher, MatcherError, MatcherSpecInput, ProgramSpec, RuleSpec, SequenceId,
    SequenceRef, SequenceSpec, execute,
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

fn fixture(
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
            error: Some(ExecutorError::new("ordinary fixture error")),
        }),
    )
}

#[test]
fn terminal_builtins_preserve_typed_results_and_stop_the_current_scope() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let accept_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![fixture_exec("before")])),
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Accept])),
                RuleSpec::unconditional(Some(vec![fixture_exec("skipped")])),
            ],
        )],
        vec![
            fixture("before", "before", &calls, ExecutorOutcome::Continue),
            fixture("skipped", "skipped", &calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid accept program");
    let mut accept_state = state();
    let mut accept_control = ExecutionControl::with_fuel(20);
    assert_eq!(
        execute(
            &accept_program,
            entry(&accept_program),
            &mut accept_state,
            &mut accept_control,
        ),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(&*calls.borrow(), &["before"]);

    let reject_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![
                ExecutableSpec::Reject { rcode: 0x0fff },
            ]))],
        )],
        Vec::new(),
    )
    .validate()
    .expect("valid explicit reject program");
    let mut reject_state = state();
    let mut reject_control = ExecutionControl::with_fuel(10);
    assert_eq!(
        execute(
            &reject_program,
            entry(&reject_program),
            &mut reject_state,
            &mut reject_control,
        ),
        Ok(ExecutionCompletion::Completed)
    );
    assert_eq!(reject_state.response.synthesized_rcode(), Some(0x0fff));

    let exit_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit])),
                RuleSpec::unconditional(Some(vec![fixture_exec("skipped")])),
            ],
        )],
        vec![fixture(
            "skipped",
            "skipped",
            &calls,
            ExecutorOutcome::Continue,
        )],
    )
    .validate()
    .expect("valid exit program");
    let mut exit_state = state();
    let mut exit_control = ExecutionControl::with_fuel(10);
    assert_eq!(
        execute(
            &exit_program,
            entry(&exit_program),
            &mut exit_state,
            &mut exit_control,
        ),
        Ok(ExecutionCompletion::Exited)
    );
    assert_eq!(&*calls.borrow(), &["before"]);
}

#[test]
fn goto_replaces_root_continuation_and_jump_resumes_it() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let goto_program = ProgramSpec::new(
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
            fixture("target", "target", &calls, ExecutorOutcome::Continue),
            fixture(
                "caller-skipped",
                "caller-skipped",
                &calls,
                ExecutorOutcome::Continue,
            ),
        ],
    )
    .validate()
    .expect("valid goto program");
    let mut goto_state = state();
    let mut goto_control = ExecutionControl::with_fuel(20);
    execute(
        &goto_program,
        entry(&goto_program),
        &mut goto_state,
        &mut goto_control,
    )
    .expect("goto execution");
    assert_eq!(&*calls.borrow(), &["target"]);

    let jump_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after-jump")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("target")])),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Return])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("target-skipped")])),
                ],
            ),
        ],
        vec![
            fixture("target", "target", &calls, ExecutorOutcome::Continue),
            fixture(
                "after-jump",
                "after-jump",
                &calls,
                ExecutorOutcome::Continue,
            ),
            fixture(
                "target-skipped",
                "target-skipped",
                &calls,
                ExecutorOutcome::Continue,
            ),
        ],
    )
    .validate()
    .expect("valid jump program");
    let before_jump = calls.borrow().len();
    let mut jump_state = state();
    let mut jump_control = ExecutionControl::with_fuel(20);
    execute(
        &jump_program,
        entry(&jump_program),
        &mut jump_state,
        &mut jump_control,
    )
    .expect("jump execution");
    assert_eq!(&calls.borrow()[before_jump..], &["target", "after-jump"]);
}

#[test]
fn executor_return_from_a_jumped_target_resumes_the_caller() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("caller-after")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![
                    RuleSpec::unconditional(Some(vec![fixture_exec("fixture-return")])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("target-skipped")])),
                ],
            ),
        ],
        vec![
            FixtureSpec::new(
                "fixture-return",
                Box::new(RecordingExecutor {
                    label: "fixture-return",
                    calls: Rc::clone(&calls),
                    outcome: ExecutorOutcome::Return,
                    error: None,
                }),
            ),
            fixture(
                "caller-after",
                "caller-after",
                &calls,
                ExecutorOutcome::Continue,
            ),
            fixture(
                "target-skipped",
                "target-skipped",
                &calls,
                ExecutorOutcome::Continue,
            ),
        ],
    )
    .validate()
    .expect("valid fixture return program");
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(20);
    execute(&program, entry(&program), &mut state, &mut control).expect("fixture return execution");
    assert_eq!(&*calls.borrow(), &["fixture-return", "caller-after"]);
}

#[test]
fn jump_target_fall_through_resumes_the_caller() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after-jump")])),
                ],
            ),
            SequenceSpec::new(
                "target",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec(
                    "target-work",
                )]))],
            ),
        ],
        vec![
            fixture(
                "target-work",
                "target-work",
                &calls,
                ExecutorOutcome::Continue,
            ),
            fixture(
                "after-jump",
                "after-jump",
                &calls,
                ExecutorOutcome::Continue,
            ),
        ],
    )
    .validate()
    .expect("valid jump fall-through program");
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(20);
    execute(&program, entry(&program), &mut state, &mut control)
        .expect("jump fall-through execution");
    assert_eq!(&*calls.borrow(), &["target-work", "after-jump"]);
}

fn inline_jump_program(
    target_rules: Vec<RuleSpec>,
    calls: &Rc<RefCell<Vec<&'static str>>>,
) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
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
            SequenceSpec::new("target", target_rules),
        ],
        vec![
            fixture("exec1", "exec1", calls, ExecutorOutcome::Continue),
            fixture(
                "target-work",
                "target-work",
                calls,
                ExecutorOutcome::Continue,
            ),
            fixture("exec3", "exec3", calls, ExecutorOutcome::Continue),
            fixture("outer", "outer", calls, ExecutorOutcome::Continue),
            fixture(
                "target-skipped",
                "target-skipped",
                calls,
                ExecutorOutcome::Continue,
            ),
        ],
    )
    .validate()
    .expect("valid inline jump program")
}

#[test]
fn inline_jump_preserves_local_continuation_and_outer_next_rule() {
    let fall_through_calls = Rc::new(RefCell::new(Vec::new()));
    let fall_through_program = inline_jump_program(
        vec![RuleSpec::unconditional(Some(vec![fixture_exec(
            "target-work",
        )]))],
        &fall_through_calls,
    );
    let mut fall_through_state = state();
    let mut fall_through_control = ExecutionControl::with_fuel(40);
    execute(
        &fall_through_program,
        entry(&fall_through_program),
        &mut fall_through_state,
        &mut fall_through_control,
    )
    .expect("inline jump fall-through execution");
    assert_eq!(
        &*fall_through_calls.borrow(),
        &["exec1", "target-work", "exec3", "outer"]
    );

    let return_calls = Rc::new(RefCell::new(Vec::new()));
    let return_program = inline_jump_program(
        vec![
            RuleSpec::unconditional(Some(vec![fixture_exec("target-work")])),
            RuleSpec::unconditional(Some(vec![ExecutableSpec::Return])),
            RuleSpec::unconditional(Some(vec![fixture_exec("target-skipped")])),
        ],
        &return_calls,
    );
    let mut return_state = state();
    let mut return_control = ExecutionControl::with_fuel(40);
    execute(
        &return_program,
        entry(&return_program),
        &mut return_state,
        &mut return_control,
    )
    .expect("inline jump return execution");
    assert_eq!(
        &*return_calls.borrow(),
        &["exec1", "target-work", "exec3", "outer"]
    );
}

fn inline_terminal_program(
    terminal: ExecutableSpec,
    calls: &Rc<RefCell<Vec<&'static str>>>,
) -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
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
            fixture("exec1", "exec1", calls, ExecutorOutcome::Continue),
            fixture(
                "inline-skipped",
                "inline-skipped",
                calls,
                ExecutorOutcome::Continue,
            ),
            fixture("outer", "outer", calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid inline terminal program")
}

#[test]
fn inline_accept_and_reject_end_only_the_inline_scope() {
    let accept_calls = Rc::new(RefCell::new(Vec::new()));
    let accept_program = inline_terminal_program(ExecutableSpec::Accept, &accept_calls);
    let mut accept_state = state();
    let mut accept_control = ExecutionControl::with_fuel(40);
    execute(
        &accept_program,
        entry(&accept_program),
        &mut accept_state,
        &mut accept_control,
    )
    .expect("inline accept execution");
    assert_eq!(&*accept_calls.borrow(), &["exec1", "outer"]);
    assert_eq!(accept_state.response.synthesized_rcode(), None);

    let reject_calls = Rc::new(RefCell::new(Vec::new()));
    let reject_program = inline_terminal_program(ExecutableSpec::default_reject(), &reject_calls);
    let mut reject_state = state();
    let mut reject_control = ExecutionControl::with_fuel(40);
    execute(
        &reject_program,
        entry(&reject_program),
        &mut reject_state,
        &mut reject_control,
    )
    .expect("inline reject execution");
    assert_eq!(&*reject_calls.borrow(), &["exec1", "outer"]);
    assert_eq!(reject_state.response.synthesized_rcode(), Some(5));
}

struct ErrorMatcher;

impl Matcher for ErrorMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Err(MatcherError::new("ordinary matcher error"))
    }
}

#[test]
fn try_runs_normal_targets_and_propagates_matcher_and_executor_errors() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let normal_sequence_program = ProgramSpec::new(
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
            fixture("child", "child", &calls, ExecutorOutcome::Continue),
            fixture("after", "after", &calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid normal try sequence");
    let mut normal_state = state();
    let mut normal_control = ExecutionControl::with_fuel(20);
    execute(
        &normal_sequence_program,
        entry(&normal_sequence_program),
        &mut normal_state,
        &mut normal_control,
    )
    .expect("normal try completion");
    assert_eq!(&*calls.borrow(), &["child", "after"]);

    let error_calls = Rc::new(RefCell::new(Vec::new()));
    let executor_error_program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Fixture(FixtureRef::new("error")),
                }])),
                RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
            ],
        )],
        vec![
            error_fixture("error", "error", &error_calls),
            fixture("after", "after", &error_calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid executor error try");
    let mut executor_error_state = state();
    let mut executor_error_control = ExecutionControl::with_fuel(20);
    assert!(matches!(
        execute(
            &executor_error_program,
            entry(&executor_error_program),
            &mut executor_error_state,
            &mut executor_error_control,
        ),
        Err(ExecutionError::Executor(ExecutorError::Failed(message)))
            if message == "ordinary fixture error"
    ));
    assert_eq!(&*error_calls.borrow(), &["error"]);
}

#[test]
fn try_propagates_an_ordinary_matcher_error() {
    let matcher_error_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                    target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                }]))],
            ),
            SequenceSpec::new(
                "child",
                vec![RuleSpec::new(
                    vec![MatcherSpecInput::new(
                        Box::new(ErrorMatcher),
                        false,
                        mosdns_sequence_core::DispatchMetadata::None,
                    )],
                    None,
                )],
            ),
        ],
        Vec::new(),
    )
    .validate()
    .expect("valid matcher error try");
    let mut matcher_error_state = state();
    let mut matcher_error_control = ExecutionControl::with_fuel(20);
    assert!(matches!(
        execute(
            &matcher_error_program,
            entry(&matcher_error_program),
            &mut matcher_error_state,
            &mut matcher_error_control,
        ),
        Err(ExecutionError::Matcher(MatcherError::Failed(message)))
            if message == "ordinary matcher error"
    ));
}

#[test]
fn inline_try_normal_and_exit_continue_to_the_next_inline_item_and_outer_rule() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let normal_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("normal")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "normal",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &calls, ExecutorOutcome::Continue),
            fixture("target", "target", &calls, ExecutorOutcome::Continue),
            fixture("exec3", "exec3", &calls, ExecutorOutcome::Continue),
            fixture("outer", "outer", &calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid inline normal try");
    let mut normal_state = state();
    let mut normal_control = ExecutionControl::with_fuel(40);
    execute(
        &normal_program,
        entry(&normal_program),
        &mut normal_state,
        &mut normal_control,
    )
    .expect("inline normal try execution");
    assert_eq!(&*calls.borrow(), &["exec1", "target", "exec3", "outer"]);

    let exit_calls = Rc::new(RefCell::new(Vec::new()));
    let exit_program = ProgramSpec::new(
        vec![
            SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("exec1"),
                        ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("exit-target")),
                        },
                        fixture_exec("exec3"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            ),
            SequenceSpec::new(
                "exit-target",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
            ),
        ],
        vec![
            fixture("exec1", "exec1", &exit_calls, ExecutorOutcome::Continue),
            fixture("exec3", "exec3", &exit_calls, ExecutorOutcome::Continue),
            fixture("outer", "outer", &exit_calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid inline exit try");
    let mut exit_state = state();
    let mut exit_control = ExecutionControl::with_fuel(40);
    execute(
        &exit_program,
        entry(&exit_program),
        &mut exit_state,
        &mut exit_control,
    )
    .expect("inline exit try execution");
    assert_eq!(&*exit_calls.borrow(), &["exec1", "exec3", "outer"]);
}

#[test]
fn inline_try_propagates_an_ordinary_error_without_running_following_items() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![
                    fixture_exec("exec1"),
                    ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Fixture(FixtureRef::new("error")),
                    },
                    fixture_exec("exec3"),
                ])),
                RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
            ],
        )],
        vec![
            fixture("exec1", "exec1", &calls, ExecutorOutcome::Continue),
            error_fixture("error", "error", &calls),
            fixture("exec3", "exec3", &calls, ExecutorOutcome::Continue),
            fixture("outer", "outer", &calls, ExecutorOutcome::Continue),
        ],
    )
    .validate()
    .expect("valid inline error try");
    let mut state = state();
    let mut control = ExecutionControl::with_fuel(40);
    assert!(matches!(
        execute(&program, entry(&program), &mut state, &mut control),
        Err(ExecutionError::Executor(ExecutorError::Failed(message)))
            if message == "ordinary fixture error"
    ));
    assert_eq!(&*calls.borrow(), &["exec1", "error"]);
}
