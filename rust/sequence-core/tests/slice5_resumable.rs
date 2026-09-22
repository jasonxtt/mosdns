use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{
    ExecutableId, ExecutableSpec, ExecutionCompletion, ExecutionControl, ExecutionError,
    ExecutionMachine, ExecutionState, Executor, ExecutorError, ExecutorOutcome, ExternalRef,
    ExternalSpec, FixtureRef, FixtureSpec, MachineStep, ProgramSpec, RuleSpec, SequenceSpec,
    StateMutation, execute,
};

fn state() -> ExecutionState {
    ExecutionState::new(
        QueryHeader {
            id: 0x4242,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        QuestionInfo {
            qname_wire: vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0],
            qtype: 1,
            qclass: 1,
        },
    )
}

struct MarkingMatcher;

impl mosdns_sequence_core::Matcher for MarkingMatcher {
    fn evaluate(
        &self,
        _state: &ExecutionState,
    ) -> Result<mosdns_sequence_core::MatchOutcome, mosdns_sequence_core::MatcherError> {
        Ok(mosdns_sequence_core::MatchOutcome::new(
            true,
            Some(StateMutation::AddMark(77)),
        ))
    }
}

struct ContinueExecutor;

impl Executor for ContinueExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        state.apply_mutation(StateMutation::AddMark(88));
        Ok(ExecutorOutcome::Continue)
    }
}

fn external_program() -> mosdns_sequence_core::ValidatedProgram {
    ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::new(
                    vec![mosdns_sequence_core::MatcherSpecInput::new(
                        Box::new(MarkingMatcher),
                        false,
                        mosdns_sequence_core::DispatchMetadata::None,
                    )],
                    Some(vec![ExecutableSpec::External {
                        target: ExternalRef::new("forward"),
                    }]),
                ),
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Accept])),
            ],
        )],
        Vec::new(),
    )
    .with_externals(vec![ExternalSpec::new("forward")])
    .validate()
    .expect("external program validates")
}

#[test]
fn external_dispatch_preserves_state_and_resumes_the_same_frames() {
    let program = external_program();
    let entry = program.sequence_id("main").expect("main entry");
    let mut machine =
        ExecutionMachine::new(&program, entry, state(), ExecutionControl::with_fuel(10))
            .expect("machine creates");

    let MachineStep::Dispatch(dispatch) = machine.step().expect("external dispatch") else {
        panic!("expected external dispatch");
    };
    assert_eq!(dispatch.executable(), ExecutableId(0));
    assert!(machine.state().marks.contains(&77));

    let completion = machine
        .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
        .expect("resume completes the following accept");
    assert_eq!(
        completion,
        MachineStep::Complete(ExecutionCompletion::Completed)
    );
    assert!(machine.state().marks.contains(&77));
}

#[test]
fn wrong_duplicate_and_post_terminal_resumes_are_typed_and_deterministic() {
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![
                RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                    target: ExternalRef::new("first"),
                }])),
                RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                    target: ExternalRef::new("second"),
                }])),
                RuleSpec::unconditional(Some(vec![ExecutableSpec::Accept])),
            ],
        )],
        Vec::new(),
    )
    .with_externals(vec![
        ExternalSpec::new("first"),
        ExternalSpec::new("second"),
    ])
    .validate()
    .expect("two external entries validate");
    let entry = program.sequence_id("main").expect("main entry");
    let mut machine =
        ExecutionMachine::new(&program, entry, state(), ExecutionControl::with_fuel(10))
            .expect("machine creates");
    let MachineStep::Dispatch(first) = machine.step().expect("first dispatch") else {
        panic!("expected first dispatch");
    };

    assert_eq!(
        machine.resume(ExecutableId(999), Ok(ExecutorOutcome::Continue)),
        Err(ExecutionError::InvalidResume {
            expected: first.executable(),
            received: ExecutableId(999),
        })
    );

    let MachineStep::Dispatch(second) = machine
        .resume(first.executable(), Ok(ExecutorOutcome::Continue))
        .expect("first resume yields second dispatch")
    else {
        panic!("expected second dispatch");
    };
    assert_ne!(first.executable(), second.executable());
    assert_eq!(
        machine.resume(first.executable(), Ok(ExecutorOutcome::Continue)),
        Err(ExecutionError::InvalidResume {
            expected: second.executable(),
            received: first.executable(),
        })
    );
    assert_eq!(
        machine.resume(second.executable(), Ok(ExecutorOutcome::Accept)),
        Ok(MachineStep::Complete(ExecutionCompletion::Completed))
    );
    assert_eq!(
        machine.resume(second.executable(), Ok(ExecutorOutcome::Continue)),
        Err(ExecutionError::Finished)
    );
}

#[test]
fn machine_preserves_fuel_and_cancellation_boundaries() {
    let program = external_program();
    let entry = program.sequence_id("main").expect("main entry");

    let mut fuel_machine =
        ExecutionMachine::new(&program, entry, state(), ExecutionControl::with_fuel(0))
            .expect("machine creates");
    assert_eq!(fuel_machine.step(), Err(ExecutionError::BudgetExceeded));
    assert_eq!(fuel_machine.control().remaining_fuel, 0);

    let mut cancellation_machine =
        ExecutionMachine::new(&program, entry, state(), ExecutionControl::with_fuel(10))
            .expect("machine creates");
    cancellation_machine.control_mut().cancel();
    assert_eq!(cancellation_machine.step(), Err(ExecutionError::Cancelled));
}

#[test]
fn sync_execute_remains_an_adapter_over_the_same_machine() {
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            "main",
            vec![RuleSpec::unconditional(Some(vec![
                ExecutableSpec::Fixture {
                    target: FixtureRef::new("continue"),
                },
            ]))],
        )],
        vec![FixtureSpec::new("continue", Box::new(ContinueExecutor))],
    )
    .validate()
    .expect("sync program validates");
    let entry = program.sequence_id("main").expect("main entry");
    let mut sync_state = state();
    let mut sync_control = ExecutionControl::with_fuel(10);
    assert_eq!(
        execute(&program, entry, &mut sync_state, &mut sync_control,),
        Ok(ExecutionCompletion::Completed)
    );
    assert!(sync_state.marks.contains(&88));
}
