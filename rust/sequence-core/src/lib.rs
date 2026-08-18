#![forbid(unsafe_code)]

mod engine;
mod program;
mod state;

pub use engine::{
    CancellationState, CancellationToken, ExecutionCompletion, ExecutionControl, ExecutionError,
    execute,
};
pub use program::{
    DispatchMetadata, ExecutableId, ExecutableSpec, ExecutableTarget, ExecutableTargetSpec,
    Executor, ExecutorError, ExecutorOutcome, FixtureRef, FixtureSpec, MatchOutcome, Matcher,
    MatcherError, MatcherSpec, MatcherSpecInput, ProgramError, ProgramSpec, RuleSpec, SequenceId,
    SequenceRef, SequenceSpec, ValidatedExecutable, ValidatedFixture, ValidatedProgram,
    ValidatedRule, ValidatedSequence,
};
pub use state::{
    DnsResponseInspector, ExecutionState, OwnedResponseWire, QueryState, ResponseError,
    ResponseInspection, ResponseInspector, ResponseState, RoutingField, RoutingState,
    StateMutation, StateSnapshot, SynthesizedResponse,
};

#[cfg(test)]
mod slice1_state_tests {
    use crate::{
        DnsResponseInspector, ExecutionState, OwnedResponseWire, ResponseError, ResponseState,
        StateSnapshot, SynthesizedResponse,
    };
    use mosdns_dns_core::{QueryHeader, QuestionInfo};

    fn state() -> ExecutionState {
        ExecutionState::new(
            QueryHeader {
                id: 0x1234,
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

    #[test]
    fn state_is_typed_and_snapshot_marks_are_sorted() {
        let mut state = state();
        state.marks.insert(49);
        state.marks.insert(7);
        state.fast_flags = 1 << 48;
        state.routing.domain_set = Some("rule-a".to_owned());

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot,
            StateSnapshot {
                query: state.query.clone(),
                marks: vec![7, 49],
                fast_flags: 1 << 48,
                response: ResponseState::None,
                routing: state.routing.clone(),
            }
        );
    }

    #[test]
    fn raw_response_is_owned_and_inspection_is_non_consuming() {
        let mut state = state();
        let wire = response_wire();
        state.set_raw_response(wire.clone());

        let inspection = state
            .inspect_response(&DnsResponseInspector)
            .expect("valid response")
            .expect("raw response is inspectable");
        assert_eq!(inspection.ttl.minimal_ttl, 60);
        assert_eq!(inspection.ttl.record_count, 1);
        assert_eq!(state.response, ResponseState::Raw(OwnedResponseWire(wire)));
    }

    #[test]
    fn malformed_raw_response_is_retained_as_typed_error() {
        let mut state = state();
        state.set_raw_response(vec![0x80]);

        let error = state
            .inspect_response(&DnsResponseInspector)
            .expect_err("malformed raw response must be observable");
        assert!(matches!(error, ResponseError::MalformedRawResponse { .. }));
        assert_eq!(
            state.response,
            ResponseState::Raw(OwnedResponseWire(vec![0x80]))
        );
    }

    #[test]
    fn synthesized_response_accepts_extended_configured_rcodes() {
        let response = SynthesizedResponse::new(0x0fff).expect("configured range");
        assert_eq!(response.rcode(), 0x0fff);
        assert!(SynthesizedResponse::new(0x1000).is_err());

        let mut state = state();
        state.set_response(ResponseState::Synthesized(response));
        assert_eq!(state.response.synthesized_rcode(), Some(0x0fff));
        state.set_raw_response(response_wire());
        assert!(matches!(state.response, ResponseState::Raw(_)));
        assert_eq!(
            state
                .inspect_response(&DnsResponseInspector)
                .expect("raw inspection")
                .expect("raw response")
                .ttl
                .record_count,
            1
        );
        state.set_synthesized_response(15).expect("valid rcode");
        assert_eq!(state.response.synthesized_rcode(), Some(15));
        assert_eq!(
            state
                .inspect_response(&DnsResponseInspector)
                .expect("synthesized inspection"),
            None
        );
        state.set_response(ResponseState::None);
        assert_eq!(state.response, ResponseState::None);
    }
}

#[cfg(test)]
mod slice2_program_tests {
    use crate::{
        ExecutableSpec, FixtureRef, FixtureSpec, MatcherSpecInput, ProgramError, ProgramSpec,
        RuleSpec, SequenceRef, SequenceSpec, ValidatedExecutable,
    };

    struct NoopExecutor;

    impl crate::Executor for NoopExecutor {
        fn execute(
            &self,
            _state: &mut crate::ExecutionState,
        ) -> Result<crate::ExecutorOutcome, crate::ExecutorError> {
            Ok(crate::ExecutorOutcome::Continue)
        }
    }

    fn fixture(name: &str) -> FixtureSpec {
        FixtureSpec::new(name, Box::new(NoopExecutor))
    }

    #[test]
    fn normalizes_empty_and_multi_exec_forms_to_explicit_inline_scope() {
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::new(Vec::new(), None),
                    RuleSpec::new(Vec::new(), Some(Vec::new())),
                    RuleSpec::new(
                        Vec::new(),
                        Some(vec![
                            ExecutableSpec::Fixture {
                                target: FixtureRef::new("same"),
                            },
                            ExecutableSpec::Fixture {
                                target: FixtureRef::new("same"),
                            },
                        ]),
                    ),
                ],
            )],
            vec![fixture("same")],
        );

        let program = spec.validate().expect("valid program");
        let main = program
            .sequence(program.sequence_id("main").expect("main"))
            .expect("main sequence");
        assert!(main.rules[0].executable.is_none());
        assert!(main.rules[1].executable.is_none());
        let Some(ValidatedExecutable::Inline { target }) = &main.rules[2].executable else {
            panic!("multi-exec must normalize to Inline");
        };
        let inline = program.sequence(*target).expect("inline sequence");
        assert_eq!(inline.rules.len(), 2);
        assert!(matches!(
            inline.rules[0].executable,
            Some(ValidatedExecutable::Fixture { .. })
        ));
        assert!(matches!(
            inline.rules[1].executable,
            Some(ValidatedExecutable::Fixture { .. })
        ));
    }

    #[test]
    fn rejects_ambiguous_or_unresolved_program_definitions_before_execution() {
        let duplicate_sequence = ProgramSpec::new(
            vec![
                SequenceSpec::new("same", Vec::new()),
                SequenceSpec::new("same", Vec::new()),
            ],
            Vec::new(),
        );
        assert!(matches!(
            duplicate_sequence.validate(),
            Err(ProgramError::DuplicateSequenceName(name)) if name == "same"
        ));

        let duplicate_fixture = ProgramSpec::new(
            vec![SequenceSpec::new("main", Vec::new())],
            vec![fixture("same"), fixture("same")],
        );
        assert!(matches!(
            duplicate_fixture.validate(),
            Err(ProgramError::DuplicateFixtureName(name)) if name == "same"
        ));

        let missing_sequence = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("missing"),
                }]))],
            )],
            Vec::new(),
        );
        assert!(matches!(
            missing_sequence.validate(),
            Err(ProgramError::MissingSequence(name)) if name == "missing"
        ));

        let invalid_rcode = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::Reject { rcode: 0x1000 },
                ]))],
            )],
            Vec::new(),
        );
        assert!(matches!(
            invalid_rcode.validate(),
            Err(ProgramError::InvalidRcode(0x1000))
        ));

        let unknown_matcher = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::new(
                    vec![MatcherSpecInput::unknown("not-a-matcher")],
                    None,
                )],
            )],
            Vec::new(),
        );
        assert!(matches!(
            unknown_matcher.validate(),
            Err(ProgramError::UnknownMatcher(kind)) if kind == "not-a-matcher"
        ));

        let unknown_executable = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::Unknown {
                        kind: "not-an-executable".to_owned(),
                    },
                ]))],
            )],
            Vec::new(),
        );
        assert!(matches!(
            unknown_executable.validate(),
            Err(ProgramError::UnknownExecutable(kind)) if kind == "not-an-executable"
        ));

        let missing_fixture = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::Fixture {
                        target: FixtureRef::new("missing"),
                    },
                ]))],
            )],
            Vec::new(),
        );
        assert!(matches!(
            missing_fixture.validate(),
            Err(ProgramError::MissingFixture(name)) if name == "missing"
        ));
    }
}

#[cfg(test)]
mod slice2_dispatch_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use mosdns_dns_core::{QueryHeader, QuestionInfo};

    use crate::{
        DispatchMetadata, ExecutableSpec, ExecutionCompletion, ExecutionControl, ExecutionError,
        ExecutionState, Executor, ExecutorError, ExecutorOutcome, FixtureRef, FixtureSpec,
        MatchOutcome, Matcher, MatcherError, MatcherSpecInput, ProgramSpec, RuleSpec, SequenceRef,
        SequenceSpec, StateMutation, execute,
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

    struct FixedMatcher {
        matched: bool,
        mutation: Option<StateMutation>,
        error: Option<MatcherError>,
        calls: Rc<Cell<u32>>,
    }

    impl Matcher for FixedMatcher {
        fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
            self.calls.set(self.calls.get() + 1);
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(MatchOutcome::new(self.matched, self.mutation.clone()))
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

    fn entry(program: &crate::ValidatedProgram) -> crate::SequenceId {
        program.sequence_id("main").expect("main entry")
    }

    #[test]
    fn matcher_mutations_are_ordered_and_errors_short_circuit_execution() {
        let first_calls = Rc::new(Cell::new(0));
        let second_calls = Rc::new(Cell::new(0));
        let fixture_calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::new(
                    vec![
                        MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: Some(StateMutation::AddMark(7)),
                                error: None,
                                calls: Rc::clone(&first_calls),
                            }),
                            false,
                            DispatchMetadata::None,
                        ),
                        MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: Some(StateMutation::AddMark(49)),
                                error: Some(MatcherError::new("second matcher failed")),
                                calls: Rc::clone(&second_calls),
                            }),
                            false,
                            DispatchMetadata::None,
                        ),
                    ],
                    Some(vec![ExecutableSpec::Fixture {
                        target: FixtureRef::new("exec"),
                    }]),
                )],
            )],
            vec![fixture(
                "exec",
                "exec",
                &fixture_calls,
                ExecutorOutcome::Continue,
            )],
        );
        let program = spec.validate().expect("valid program");
        let mut main_state = state(1);
        let mut control = ExecutionControl::with_fuel(20);
        let result = execute(&program, entry(&program), &mut main_state, &mut control);

        assert!(matches!(result, Err(ExecutionError::Matcher(_))));
        assert_eq!(first_calls.get(), 1);
        assert_eq!(second_calls.get(), 1);
        assert!(main_state.marks.contains(&7));
        assert!(!main_state.marks.contains(&49));
        assert!(fixture_calls.borrow().is_empty());
    }

    #[test]
    fn false_matcher_skips_later_matchers_and_executable() {
        let skipped_calls = Rc::new(Cell::new(0));
        let fixture_calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::new(
                    vec![
                        MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: false,
                                mutation: None,
                                error: None,
                                calls: Rc::new(Cell::new(0)),
                            }),
                            false,
                            DispatchMetadata::None,
                        ),
                        MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: Some(StateMutation::AddMark(99)),
                                error: None,
                                calls: Rc::clone(&skipped_calls),
                            }),
                            false,
                            DispatchMetadata::None,
                        ),
                    ],
                    Some(vec![ExecutableSpec::Fixture {
                        target: FixtureRef::new("exec"),
                    }]),
                )],
            )],
            vec![fixture(
                "exec",
                "exec",
                &fixture_calls,
                ExecutorOutcome::Continue,
            )],
        );
        let program = spec.validate().expect("valid program");
        let mut state = state(1);
        let mut control = ExecutionControl::with_fuel(20);
        assert_eq!(
            execute(&program, entry(&program), &mut state, &mut control),
            Ok(ExecutionCompletion::Completed)
        );
        assert_eq!(skipped_calls.get(), 0);
        assert!(!state.marks.contains(&99));
        assert!(fixture_calls.borrow().is_empty());
    }

    #[test]
    fn positive_metadata_is_write_once_and_reverse_never_claims_label() {
        let fixture_calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::new(
                        vec![MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: None,
                                error: None,
                                calls: Rc::new(Cell::new(0)),
                            }),
                            false,
                            DispatchMetadata::AnonymousQname {
                                rule_name: "rule-a".to_owned(),
                            },
                        )],
                        Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("exec"),
                        }]),
                    ),
                    RuleSpec::new(
                        vec![MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: None,
                                error: None,
                                calls: Rc::new(Cell::new(0)),
                            }),
                            false,
                            DispatchMetadata::AnonymousQname {
                                rule_name: "rule-b".to_owned(),
                            },
                        )],
                        None,
                    ),
                ],
            )],
            vec![fixture(
                "exec",
                "exec",
                &fixture_calls,
                ExecutorOutcome::Continue,
            )],
        );
        let program = spec.validate().expect("valid program");
        let mut main_state = state(1);
        let mut control = ExecutionControl::with_fuel(20);
        execute(&program, entry(&program), &mut main_state, &mut control).expect("execution");
        assert_eq!(main_state.routing.domain_set.as_deref(), Some("rule-a"));

        let reverse_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::new(
                    vec![MatcherSpecInput::new(
                        Box::new(FixedMatcher {
                            matched: false,
                            mutation: None,
                            error: None,
                            calls: Rc::new(Cell::new(0)),
                        }),
                        true,
                        DispatchMetadata::AnonymousQname {
                            rule_name: "must-not-appear".to_owned(),
                        },
                    )],
                    None,
                )],
            )],
            Vec::new(),
        );
        let reverse_program = reverse_spec.validate().expect("valid reverse program");
        let mut reverse_state = state(1);
        let mut reverse_control = ExecutionControl::with_fuel(10);
        execute(
            &reverse_program,
            entry(&reverse_program),
            &mut reverse_state,
            &mut reverse_control,
        )
        .expect("reverse execution");
        assert_eq!(reverse_state.routing.domain_set, None);
    }

    #[test]
    fn qtype_dispatch_metadata_uses_typed_question_values() {
        for (qtype, metadata, expected) in [
            (28, DispatchMetadata::Switch6, "BANAAAA"),
            (6, DispatchMetadata::Switch5, "BANSOA"),
            (12, DispatchMetadata::Switch5, "BANPTR"),
            (65, DispatchMetadata::Switch5, "BANHTTPS"),
        ] {
            let spec = ProgramSpec::new(
                vec![SequenceSpec::new(
                    "main",
                    vec![RuleSpec::new(
                        vec![MatcherSpecInput::new(
                            Box::new(FixedMatcher {
                                matched: true,
                                mutation: None,
                                error: None,
                                calls: Rc::new(Cell::new(0)),
                            }),
                            false,
                            metadata,
                        )],
                        None,
                    )],
                )],
                Vec::new(),
            );
            let program = spec.validate().expect("valid metadata program");
            let mut state = state(qtype);
            let mut control = ExecutionControl::with_fuel(10);
            execute(&program, entry(&program), &mut state, &mut control).expect("execution");
            assert_eq!(state.routing.domain_set.as_deref(), Some(expected));
        }
    }

    #[test]
    fn jump_and_goto_use_explicit_continuations() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                            target: SequenceRef::new("sub"),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("after-jump"),
                        }])),
                    ],
                ),
                SequenceSpec::new(
                    "sub",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("sub-work"),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Return])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("skipped"),
                        }])),
                    ],
                ),
            ],
            vec![
                fixture("sub-work", "sub-work", &calls, ExecutorOutcome::Continue),
                fixture(
                    "after-jump",
                    "after-jump",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
                fixture("skipped", "skipped", &calls, ExecutorOutcome::Continue),
            ],
        );
        let program = spec.validate().expect("valid jump program");
        let mut main_state = state(1);
        let mut control = ExecutionControl::with_fuel(30);
        execute(&program, entry(&program), &mut main_state, &mut control).expect("jump execution");
        assert_eq!(&*calls.borrow(), &["sub-work", "after-jump"]);

        let goto_spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                            target: SequenceRef::new("sub"),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("skipped"),
                        }])),
                    ],
                ),
                SequenceSpec::new(
                    "sub",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::Fixture {
                            target: FixtureRef::new("sub-work"),
                        },
                    ]))],
                ),
            ],
            vec![
                fixture("sub-work", "sub-work", &calls, ExecutorOutcome::Continue),
                fixture("skipped", "skipped", &calls, ExecutorOutcome::Continue),
            ],
        );
        let goto_program = goto_spec.validate().expect("valid goto program");
        let before = calls.borrow().len();
        let mut goto_state = state(1);
        let mut goto_control = ExecutionControl::with_fuel(20);
        execute(
            &goto_program,
            entry(&goto_program),
            &mut goto_state,
            &mut goto_control,
        )
        .expect("goto execution");
        assert_eq!(&calls.borrow()[before..], &["sub-work"]);
    }

    #[test]
    fn try_catches_only_exit_and_continues_the_parent() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                            target: crate::ExecutableTargetSpec::Sequence(SequenceRef::new(
                                "child",
                            )),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Fixture {
                            target: FixtureRef::new("after"),
                        }])),
                    ],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
                ),
            ],
            vec![fixture("after", "after", &calls, ExecutorOutcome::Continue)],
        );
        let program = spec.validate().expect("valid try program");
        let mut state = state(1);
        let mut control = ExecutionControl::with_fuel(20);
        assert_eq!(
            execute(&program, entry(&program), &mut state, &mut control),
            Ok(ExecutionCompletion::Completed)
        );
        assert_eq!(&*calls.borrow(), &["after"]);
    }
}

#[cfg(test)]
mod slice3_control_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use mosdns_dns_core::{QueryHeader, QuestionInfo};

    use crate::{
        ExecutableSpec, ExecutableTargetSpec, ExecutionCompletion, ExecutionControl,
        ExecutionError, ExecutionState, Executor, ExecutorError, ExecutorOutcome, FixtureRef,
        FixtureSpec, ProgramSpec, RuleSpec, SequenceRef, SequenceSpec, execute,
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

    fn entry(program: &crate::ValidatedProgram) -> crate::SequenceId {
        program.sequence_id("main").expect("main entry")
    }

    fn fixture_exec(name: &str) -> ExecutableSpec {
        ExecutableSpec::Fixture {
            target: FixtureRef::new(name),
        }
    }

    #[test]
    fn terminal_builtins_have_typed_scope_results_and_reject_state() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
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
        );
        let program = spec.validate().expect("valid accept program");
        let mut accept_state = state();
        let mut control = ExecutionControl::with_fuel(20);
        assert_eq!(
            execute(&program, entry(&program), &mut accept_state, &mut control),
            Ok(ExecutionCompletion::Completed)
        );
        assert_eq!(&*calls.borrow(), &["before"]);
        assert_eq!(accept_state.response.synthesized_rcode(), None);

        let reject_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::default_reject()])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("skipped")])),
                ],
            )],
            vec![fixture(
                "skipped",
                "skipped",
                &calls,
                ExecutorOutcome::Continue,
            )],
        );
        let reject_program = reject_spec.validate().expect("valid reject program");
        let mut reject_state = state();
        let mut reject_control = ExecutionControl::with_fuel(20);
        execute(
            &reject_program,
            entry(&reject_program),
            &mut reject_state,
            &mut reject_control,
        )
        .expect("reject execution");
        assert_eq!(reject_state.response.synthesized_rcode(), Some(5));

        let exit_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
            )],
            Vec::new(),
        );
        let exit_program = exit_spec.validate().expect("valid exit program");
        let mut exit_state = state();
        let mut exit_control = ExecutionControl::with_fuel(5);
        assert_eq!(
            execute(
                &exit_program,
                entry(&exit_program),
                &mut exit_state,
                &mut exit_control,
            ),
            Ok(ExecutionCompletion::Exited)
        );
    }

    #[test]
    fn top_level_return_completes_and_jump_at_end_falls_back_to_parent_end() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let return_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Return])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("skipped")])),
                ],
            )],
            vec![fixture(
                "skipped",
                "skipped",
                &calls,
                ExecutorOutcome::Continue,
            )],
        );
        let return_program = return_spec.validate().expect("valid top-level return");
        let mut return_state = state();
        let mut return_control = ExecutionControl::with_fuel(10);
        assert_eq!(
            execute(
                &return_program,
                entry(&return_program),
                &mut return_state,
                &mut return_control,
            ),
            Ok(ExecutionCompletion::Completed)
        );
        assert!(calls.borrow().is_empty());

        let jump_spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Jump {
                        target: SequenceRef::new("target"),
                    }]))],
                ),
                SequenceSpec::new(
                    "target",
                    vec![RuleSpec::unconditional(Some(vec![fixture_exec("target")]))],
                ),
            ],
            vec![fixture(
                "target",
                "target",
                &calls,
                ExecutorOutcome::Continue,
            )],
        );
        let jump_program = jump_spec.validate().expect("valid jump-at-end");
        let mut jump_state = state();
        let mut jump_control = ExecutionControl::with_fuel(20);
        execute(
            &jump_program,
            entry(&jump_program),
            &mut jump_state,
            &mut jump_control,
        )
        .expect("jump-at-end execution");
        assert_eq!(&*calls.borrow(), &["target"]);
    }

    #[test]
    fn inline_scope_resumes_outer_rules_after_return_accept_or_goto() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let return_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![
                        fixture_exec("inline-a"),
                        ExecutableSpec::Return,
                        fixture_exec("inline-skipped"),
                    ])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            )],
            vec![
                fixture("inline-a", "inline-a", &calls, ExecutorOutcome::Continue),
                fixture(
                    "inline-skipped",
                    "inline-skipped",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
                fixture("outer", "outer", &calls, ExecutorOutcome::Continue),
            ],
        );
        let return_program = return_spec.validate().expect("valid inline return");
        let mut return_state = state();
        let mut return_control = ExecutionControl::with_fuel(30);
        execute(
            &return_program,
            entry(&return_program),
            &mut return_state,
            &mut return_control,
        )
        .expect("inline return execution");
        assert_eq!(&*calls.borrow(), &["inline-a", "outer"]);

        let goto_spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![
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
                    vec![RuleSpec::unconditional(Some(vec![fixture_exec(
                        "target-work",
                    )]))],
                ),
            ],
            vec![
                fixture(
                    "inline-skipped",
                    "inline-skipped",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
                fixture("outer", "outer", &calls, ExecutorOutcome::Continue),
                fixture(
                    "target-work",
                    "target-work",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
            ],
        );
        let goto_program = goto_spec.validate().expect("valid inline goto");
        let before = calls.borrow().len();
        let mut goto_state = state();
        let mut goto_control = ExecutionControl::with_fuel(30);
        execute(
            &goto_program,
            entry(&goto_program),
            &mut goto_state,
            &mut goto_control,
        )
        .expect("inline goto execution");
        assert_eq!(&calls.borrow()[before..], &["target-work", "outer"]);
    }

    #[test]
    fn inline_exit_propagates_but_nested_try_catches_exit_and_continues() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let propagate_spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                ],
            )],
            vec![fixture("outer", "outer", &calls, ExecutorOutcome::Continue)],
        );
        let propagate_program = propagate_spec.validate().expect("valid exit scope");
        let mut propagate_state = state();
        let mut propagate_control = ExecutionControl::with_fuel(20);
        assert_eq!(
            execute(
                &propagate_program,
                entry(&propagate_program),
                &mut propagate_state,
                &mut propagate_control,
            ),
            Ok(ExecutionCompletion::Exited)
        );
        assert!(calls.borrow().is_empty());

        let nested_spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![
                            fixture_exec("inline-before"),
                            ExecutableSpec::Try {
                                target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                            },
                            fixture_exec("inline-after"),
                        ])),
                        RuleSpec::unconditional(Some(vec![fixture_exec("outer")])),
                    ],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
                ),
            ],
            vec![
                fixture(
                    "inline-before",
                    "inline-before",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
                fixture(
                    "inline-after",
                    "inline-after",
                    &calls,
                    ExecutorOutcome::Continue,
                ),
                fixture("outer", "outer", &calls, ExecutorOutcome::Continue),
            ],
        );
        let nested_program = nested_spec.validate().expect("valid nested try");
        let before = calls.borrow().len();
        let mut nested_state = state();
        let mut nested_control = ExecutionControl::with_fuel(30);
        execute(
            &nested_program,
            entry(&nested_program),
            &mut nested_state,
            &mut nested_control,
        )
        .expect("nested try execution");
        assert_eq!(
            &calls.borrow()[before..],
            &["inline-before", "inline-after", "outer"]
        );
    }

    #[test]
    fn try_propagates_ordinary_fixture_errors() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let error_fixture = FixtureSpec::new(
            "error",
            Box::new(RecordingExecutor {
                label: "error",
                calls: Rc::clone(&calls),
                outcome: ExecutorOutcome::Continue,
                error: Some(ExecutorError::new("fixture failed")),
            }),
        );
        let spec = ProgramSpec::new(
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
                error_fixture,
                fixture("after", "after", &calls, ExecutorOutcome::Continue),
            ],
        );
        let program = spec.validate().expect("valid error try");
        let mut state = state();
        let mut control = ExecutionControl::with_fuel(20);
        assert!(matches!(
            execute(&program, entry(&program), &mut state, &mut control),
            Err(ExecutionError::Executor(_))
        ));
        assert_eq!(&*calls.borrow(), &["error"]);
    }

    #[test]
    fn try_catches_exit_from_a_fixture_target() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Fixture(FixtureRef::new("exit-fixture")),
                    }])),
                    RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                ],
            )],
            vec![
                fixture(
                    "exit-fixture",
                    "exit-fixture",
                    &calls,
                    ExecutorOutcome::Exit,
                ),
                fixture("after", "after", &calls, ExecutorOutcome::Continue),
            ],
        );
        let program = spec.validate().expect("valid fixture try");
        let mut state = state();
        let mut control = ExecutionControl::with_fuel(20);
        execute(&program, entry(&program), &mut state, &mut control)
            .expect("fixture exit must be caught");
        assert_eq!(&*calls.borrow(), &["exit-fixture", "after"]);
    }
}

#[cfg(test)]
mod slice4_safety_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use mosdns_dns_core::{QueryHeader, QuestionInfo};

    use crate::{
        ExecutableSpec, ExecutableTargetSpec, ExecutionControl, ExecutionError, ExecutionState,
        Executor, ExecutorOutcome, FixtureRef, FixtureSpec, ProgramSpec, RuleSpec, SequenceRef,
        SequenceSpec, execute,
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

    struct CountingExecutor {
        calls: Rc<RefCell<u32>>,
    }

    impl Executor for CountingExecutor {
        fn execute(
            &self,
            _state: &mut ExecutionState,
        ) -> Result<ExecutorOutcome, crate::ExecutorError> {
            *self.calls.borrow_mut() += 1;
            Ok(ExecutorOutcome::Continue)
        }
    }

    fn fixture(name: &str, calls: &Rc<RefCell<u32>>) -> FixtureSpec {
        FixtureSpec::new(
            name,
            Box::new(CountingExecutor {
                calls: Rc::clone(calls),
            }),
        )
    }

    fn entry(program: &crate::ValidatedProgram) -> crate::SequenceId {
        program.sequence_id("main").expect("main entry")
    }

    #[test]
    fn cyclic_goto_is_bounded_by_shared_fuel() {
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Goto {
                    target: SequenceRef::new("main"),
                }]))],
            )],
            Vec::new(),
        );
        let program = spec.validate().expect("valid cyclic program");
        let mut state = state();
        let mut control = ExecutionControl::with_fuel(7);
        assert_eq!(
            execute(&program, entry(&program), &mut state, &mut control),
            Err(ExecutionError::BudgetExceeded)
        );
        assert_eq!(control.remaining_fuel, 0);
    }

    #[test]
    fn cancellation_has_priority_at_the_next_dispatch_boundary() {
        let calls = Rc::new(RefCell::new(0));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::Fixture {
                        target: FixtureRef::new("work"),
                    },
                ]))],
            )],
            vec![fixture("work", &calls)],
        );
        let program = spec.validate().expect("valid cancellation program");
        let mut state = state();
        let mut control = ExecutionControl::with_fuel(0);
        control.cancel();
        assert_eq!(
            execute(&program, entry(&program), &mut state, &mut control),
            Err(ExecutionError::Cancelled)
        );
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn budget_is_checked_before_fixture_error_or_exit() {
        let calls = Rc::new(RefCell::new(0));
        let spec = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
            )],
            Vec::new(),
        );
        let program = spec.validate().expect("valid budget program");
        let mut exit_state = state();
        let mut control = ExecutionControl::with_fuel(0);
        assert_eq!(
            execute(&program, entry(&program), &mut exit_state, &mut control),
            Err(ExecutionError::BudgetExceeded)
        );

        let nested_spec = ProgramSpec::new(
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
        );
        let nested_program = nested_spec.validate().expect("valid nested cycle");
        let mut nested_state = state();
        let mut nested_control = ExecutionControl::with_fuel(9);
        assert_eq!(
            execute(
                &nested_program,
                entry(&nested_program),
                &mut nested_state,
                &mut nested_control,
            ),
            Err(ExecutionError::BudgetExceeded)
        );
        assert_eq!(nested_control.remaining_fuel, 0);
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn invalid_entry_is_an_error_without_state_transfer() {
        let spec = ProgramSpec::new(vec![SequenceSpec::new("main", Vec::new())], Vec::new());
        let program = spec.validate().expect("valid program");
        let mut state = state();
        let before = state.snapshot();
        let mut control = ExecutionControl::with_fuel(5);
        assert_eq!(
            execute(&program, crate::SequenceId(999), &mut state, &mut control),
            Err(ExecutionError::InvalidEntry(crate::SequenceId(999)))
        );
        assert_eq!(state.snapshot(), before);
    }
}
