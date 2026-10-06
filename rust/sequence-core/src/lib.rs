#![forbid(unsafe_code)]

mod engine;
mod program;
mod state;

pub use engine::{
    CancellationState, CancellationToken, ExecutionCompletion, ExecutionControl, ExecutionError,
    ExecutionMachine, ExternalDispatch, MachineStep, RootFuelHandle, ScopeCompletion,
    SuccessorRecipe, WatchToken, execute,
};
pub use program::{
    DispatchMetadata, ExecutableId, ExecutableSpec, ExecutableTarget, ExecutableTargetSpec,
    Executor, ExecutorError, ExecutorOutcome, ExternalRef, ExternalSpec, FixtureRef, FixtureSpec,
    MatchOutcome, Matcher, MatcherError, MatcherSpec, MatcherSpecInput, ProgramError, ProgramSpec,
    RuleSpec, SequenceId, SequenceRef, SequenceSpec, ValidatedExecutable, ValidatedExternal,
    ValidatedFixture, ValidatedProgram, ValidatedRule, ValidatedSequence,
};
pub use state::{
    AdmissionFacts, ClientContext, ClientTransport, DnsResponseInspector, ExecutionState,
    OwnedResponseWire, QueryState, ResponseError, ResponseInspection, ResponseInspector,
    ResponseOrigin, ResponseState, RoutingField, RoutingState, StateMutation, StateSnapshot,
    SynthesizedResponse,
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
                admission_facts: state.admission_facts.clone(),
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
    fn response_generation_tracks_equal_replacements() {
        let mut state = state();
        assert_eq!(state.response_generation(), 0);
        let wire = response_wire();
        state.set_raw_response(wire.clone());
        assert_eq!(state.response_generation(), 1);
        state.set_raw_response(wire);
        assert_eq!(state.response_generation(), 2);
    }

    #[test]
    fn response_origin_survives_decoration_but_not_identical_replacement() {
        let mut state = state();
        let wire = response_wire();
        let origin = crate::ResponseOrigin {
            identity: std::sync::Arc::from("actual"),
            peer: None,
            transport: None,
        };
        state.set_raw_response_with_origin(wire.clone(), Some(origin.clone()));
        state.rewrite_raw_response(wire.clone());
        assert_eq!(state.response_origin(), Some(&origin));
        state.set_raw_response(wire);
        assert!(state.response_origin().is_none());
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
        MatchOutcome, Matcher, MatcherError, MatcherSpecInput, ProgramSpec, RoutingField, RuleSpec,
        SequenceRef, SequenceSpec, StateMutation, execute,
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
                            mutation: Some(StateMutation::SetRouting {
                                field: RoutingField::MatchedRuleSource,
                                value: Some("negated:qname".to_owned()),
                            }),
                            error: None,
                            calls: Rc::new(Cell::new(0)),
                        }),
                        true,
                        DispatchMetadata::AnonymousQname {
                            rule_name: "must-not-appear".to_owned(),
                        },
                    )],
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
        assert_eq!(
            reverse_state.routing.matched_rule_source.as_deref(),
            Some("negated:qname")
        );
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
        ExecutionError, ExecutionMachine, ExecutionState, Executor, ExecutorError, ExecutorOutcome,
        FixtureRef, FixtureSpec, MachineStep, ProgramSpec, RuleSpec, SequenceRef, SequenceSpec,
        execute,
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

    #[test]
    fn a_direct_named_call_returns_to_the_caller_for_every_natural_end() {
        for (name, child_rules, expected) in [
            (
                "fall-through-work",
                vec![RuleSpec::unconditional(Some(vec![fixture_exec("work")]))],
                vec!["work", "after"],
            ),
            (
                "accept",
                vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Accept]))],
                vec!["after"],
            ),
            (
                "reject",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::default_reject(),
                ]))],
                vec!["after"],
            ),
        ] {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let program = ProgramSpec::new(
                vec![
                    SequenceSpec::new(
                        "main",
                        vec![
                            RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                                target: SequenceRef::new("child"),
                            }])),
                            RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                        ],
                    ),
                    SequenceSpec::new("child", child_rules),
                ],
                vec![
                    fixture("after", "after", &calls, ExecutorOutcome::Continue),
                    fixture("work", "work", &calls, ExecutorOutcome::Continue),
                ],
            )
            .validate()
            .expect("valid direct-call program");
            let mut state = state();
            let mut control = ExecutionControl::with_fuel(20);
            execute(&program, entry(&program), &mut state, &mut control)
                .unwrap_or_else(|error| panic!("{name} must return to the caller: {error:?}"));
            assert_eq!(&*calls.borrow(), &expected[..], "{name}");
        }
    }

    #[test]
    fn a_direct_call_exit_propagates_past_the_caller_and_try_still_catches_it() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let spec = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                            target: SequenceRef::new("child"),
                        }])),
                        RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                    ],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
                ),
            ],
            vec![fixture("after", "after", &calls, ExecutorOutcome::Continue)],
        );
        let program = spec.validate().expect("valid exit program");
        let mut state = state();
        let mut control = ExecutionControl::with_fuel(20);
        assert_eq!(
            execute(&program, entry(&program), &mut state, &mut control),
            Ok(ExecutionCompletion::Exited)
        );
        assert!(
            calls.borrow().is_empty(),
            "exit must skip the caller's next rule"
        );

        let caught_calls = Rc::new(RefCell::new(Vec::new()));
        let caught = ProgramSpec::new(
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
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                        target: SequenceRef::new("grandchild"),
                    }]))],
                ),
                SequenceSpec::new(
                    "grandchild",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Exit]))],
                ),
            ],
            vec![fixture(
                "after",
                "after",
                &caught_calls,
                ExecutorOutcome::Continue,
            )],
        )
        .validate()
        .expect("valid caught-exit program");
        let mut caught_state = self::state();
        let mut caught_control = ExecutionControl::with_fuel(20);
        execute(
            &caught,
            entry(&caught),
            &mut caught_state,
            &mut caught_control,
        )
        .expect("try must still catch a nested direct-call exit");
        assert_eq!(&*caught_calls.borrow(), &["after"]);
    }

    #[test]
    fn a_watch_reports_the_real_enclosing_scope_completion_and_the_named_origin() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                            target: SequenceRef::new("child"),
                        }])),
                        RuleSpec::unconditional(Some(vec![fixture_exec("after")])),
                    ],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("upstream"),
                        },
                    ]))],
                ),
            ],
            vec![fixture("after", "after", &calls, ExecutorOutcome::Continue)],
        )
        .with_externals(vec![crate::ExternalSpec::new("upstream")])
        .validate()
        .expect("valid watched program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");
        let MachineStep::Dispatch(dispatch) = machine.step().expect("external dispatch") else {
            panic!("the child external must dispatch");
        };
        assert_eq!(
            machine
                .last_origin()
                .and_then(|id| program.sequence(id))
                .map(|sequence| sequence.name.as_str()),
            Some("child"),
            "the executing origin must be the real child sequence"
        );
        machine
            .watch_enclosing_scope(dispatch.executable())
            .expect("watch arms on the observed dispatch");
        let step = machine
            .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("external resume");
        let MachineStep::ScopeComplete(completion) = step else {
            panic!("the watched child scope must report its completion: {step:?}");
        };
        assert_eq!(completion.executable(), dispatch.executable());
        assert!(
            calls.borrow().is_empty(),
            "the caller's next rule must not run before the owner resumes"
        );
        let step = machine
            .resume_scope_completion(completion.token())
            .expect("scope resume");
        assert!(matches!(step, MachineStep::Complete(_)));
        assert_eq!(&*calls.borrow(), &["after"]);
        assert_eq!(
            machine
                .last_origin()
                .and_then(|id| program.sequence(id))
                .map(|sequence| sequence.name.as_str()),
            Some("main")
        );
    }

    #[test]
    fn a_watch_never_reports_a_synthetic_inline_scope_as_an_origin() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("upstream"),
                    },
                ]))],
            )],
            vec![fixture("after", "after", &calls, ExecutorOutcome::Continue)],
        )
        .with_externals(vec![crate::ExternalSpec::new("upstream")])
        .validate()
        .expect("valid inline program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");
        let MachineStep::Dispatch(dispatch) = machine.step().expect("external dispatch") else {
            panic!("the external must dispatch");
        };
        assert_eq!(
            machine
                .last_origin()
                .and_then(|id| program.sequence(id))
                .map(|sequence| sequence.name.as_str()),
            Some("main")
        );
        assert!(
            !program.sequences.iter().any(|sequence| sequence.synthetic),
            "the single named program has no synthetic scope"
        );
        assert!(
            machine.watch_enclosing_scope(dispatch.executable()).is_ok(),
            "a watch may arm on the observed dispatch"
        );
        assert_eq!(
            machine.watch_enclosing_scope(dispatch.executable()),
            Err(ExecutionError::ResumeNotPending(dispatch.executable())),
            "one dispatch may not arm twice"
        );
    }

    #[test]
    fn a_terminal_machine_error_invalidates_every_armed_watch_without_a_notification() {
        // Cancellation, exhausted budget and terminal executor errors stop the
        // machine instead of unwinding a scope, so they produce no
        // `ScopeAborted`. The contract they must still honour is that no armed
        // watch survives a failed machine: an owner can never be left waiting
        // for a boundary that will not complete.
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("a"),
                    },
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("b"),
                    },
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("c"),
                    },
                ]))],
            )],
            Vec::new(),
        )
        .with_externals(vec![
            crate::ExternalSpec::new("a"),
            crate::ExternalSpec::new("b"),
            crate::ExternalSpec::new("c"),
        ])
        .validate()
        .expect("valid three-leg program");
        // Two fuel units: the first two dispatches consume them and the third
        // rule fails on an exhausted shared budget.
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(3),
        )
        .expect("machine");
        let MachineStep::Dispatch(first) = machine.step().expect("first dispatch") else {
            panic!("the first external must dispatch");
        };
        machine
            .watch_enclosing_scope(first.executable())
            .expect("first watch arms");
        let MachineStep::Dispatch(second) = machine
            .resume(first.executable(), Ok(ExecutorOutcome::Continue))
            .expect("first resume")
        else {
            panic!("the second external must dispatch");
        };
        machine
            .watch_enclosing_scope(second.executable())
            .expect("second watch arms");
        assert_eq!(machine.armed_watch_count(), 2);

        let error = machine
            .resume(second.executable(), Ok(ExecutorOutcome::Continue))
            .expect_err("the exhausted budget must stop the machine");
        assert_eq!(error, ExecutionError::BudgetExceeded);
        assert!(machine.is_finished(), "a failed machine is terminal");
        assert_eq!(
            machine.armed_watch_count(),
            0,
            "no watch may outlive a failed machine"
        );
    }

    #[test]
    fn a_resumed_external_error_also_invalidates_every_armed_watch() {
        // The other terminal transition: the owner supplies an executor error
        // through `resume` instead of the machine discovering a stop while
        // driving. It must invalidate watches exactly like the drive-time path.
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "main",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("a"),
                    },
                    ExecutableSpec::External {
                        target: crate::ExternalRef::new("b"),
                    },
                ]))],
            )],
            Vec::new(),
        )
        .with_externals(vec![
            crate::ExternalSpec::new("a"),
            crate::ExternalSpec::new("b"),
        ])
        .validate()
        .expect("valid two-leg program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(8),
        )
        .expect("machine");
        let MachineStep::Dispatch(first) = machine.step().expect("first dispatch") else {
            panic!("the first external must dispatch");
        };
        machine
            .watch_enclosing_scope(first.executable())
            .expect("first watch arms");
        let MachineStep::Dispatch(second) = machine
            .resume(first.executable(), Ok(ExecutorOutcome::Continue))
            .expect("first resume")
        else {
            panic!("the second external must dispatch");
        };
        machine
            .watch_enclosing_scope(second.executable())
            .expect("second watch arms");
        assert_eq!(machine.armed_watch_count(), 2);

        let error = machine
            .resume(
                second.executable(),
                Err(ExecutorError::new("terminal executor failure")),
            )
            .expect_err("an executor error must stop the machine");
        assert!(matches!(error, ExecutionError::Executor(_)), "{error:?}");
        assert!(machine.is_finished(), "a failed machine is terminal");
        assert_eq!(
            machine.armed_watch_count(),
            0,
            "no watch may outlive a machine that failed through `resume`"
        );
    }

    #[test]
    fn an_exited_scope_aborts_only_its_own_watch_and_the_surviving_scope_keeps_its_token() {
        // main(cache, try(child)) with child(cache, exit). Two watches are armed
        // for the *same* executable in two scopes; only the child scope exits.
        // The exit must invalidate exactly the child's watch, and the surviving
        // root watch must still be reported later with its own token.
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache"),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                            target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                        }])),
                    ],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache"),
                        },
                        ExecutableSpec::Exit,
                    ]))],
                ),
            ],
            Vec::new(),
        )
        .with_externals(vec![crate::ExternalSpec::new("cache")])
        .validate()
        .expect("valid try program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");

        let MachineStep::Dispatch(root_dispatch) = machine.step().expect("root dispatch") else {
            panic!("the root cache must dispatch first");
        };
        let root_token = machine
            .watch_enclosing_scope(root_dispatch.executable())
            .expect("root watch arms");
        let MachineStep::Dispatch(child_dispatch) = machine
            .resume(root_dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("root resume")
        else {
            panic!("the child cache must dispatch next");
        };
        let child_token = machine
            .watch_enclosing_scope(child_dispatch.executable())
            .expect("child watch arms");
        assert_ne!(root_token, child_token, "each watch has its own identity");
        assert_eq!(machine.armed_watch_count(), 2);

        // The child scope exits. Only its watch may be aborted.
        let step = machine
            .resume(child_dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("exit resume");
        let MachineStep::ScopeAborted(aborted) = step else {
            panic!("an exited scope must abort, not complete: {step:?}");
        };
        assert_eq!(
            aborted.token(),
            child_token,
            "the aborted notification belongs to the exited scope's own watch"
        );
        assert_eq!(aborted.executable(), child_dispatch.executable());
        assert_eq!(
            machine.armed_watch_count(),
            1,
            "only the root watch survives"
        );

        // The surviving root boundary is still reported, with the root token.
        let step = machine
            .resume_scope_completion(child_token)
            .expect("abort resume");
        assert!(
            !matches!(step, MachineStep::ScopeAborted(_)),
            "the root scope has not stopped running yet: {step:?}"
        );
        let step = match step {
            MachineStep::Dispatch(dispatch) => machine
                .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
                .expect("main resume"),
            other => other,
        };
        let MachineStep::ScopeComplete(completion) = step else {
            panic!("the surviving root boundary must complete: {step:?}");
        };
        assert_eq!(
            completion.token(),
            root_token,
            "the publishable boundary must be the root watch, never the exited one"
        );
    }

    #[test]
    fn watches_on_scopes_unwound_by_exit_propagation_are_never_orphaned() {
        // main(try(child)) with child(cache, call(grandchild)) and
        // grandchild(cache, exit). `exit` propagates through both the call scope
        // and the try scope, so two scopes vanish in one step. Both watches must
        // be retired; neither may survive against a dead scope identity.
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Try {
                        target: ExecutableTargetSpec::Sequence(SequenceRef::new("child")),
                    }]))],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache"),
                        },
                        ExecutableSpec::Call {
                            target: SequenceRef::new("grandchild"),
                        },
                    ]))],
                ),
                SequenceSpec::new(
                    "grandchild",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache"),
                        },
                        ExecutableSpec::Exit,
                    ]))],
                ),
            ],
            Vec::new(),
        )
        .with_externals(vec![crate::ExternalSpec::new("cache")])
        .validate()
        .expect("valid propagation program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");
        let MachineStep::Dispatch(first) = machine.step().expect("first dispatch") else {
            panic!("the first cache must dispatch");
        };
        machine
            .watch_enclosing_scope(first.executable())
            .expect("first watch arms");
        let MachineStep::Dispatch(second) = machine
            .resume(first.executable(), Ok(ExecutorOutcome::Continue))
            .expect("first resume")
        else {
            panic!("the second cache must dispatch");
        };
        machine
            .watch_enclosing_scope(second.executable())
            .expect("second watch arms");
        assert_eq!(machine.armed_watch_count(), 2);

        let mut step = machine
            .resume(second.executable(), Ok(ExecutorOutcome::Continue))
            .expect("exit resume");
        let mut aborted = 0_usize;
        while let MachineStep::ScopeAborted(completion) = step {
            aborted += 1;
            step = machine
                .resume_scope_completion(completion.token())
                .expect("abort resume");
        }
        assert_eq!(
            aborted, 2,
            "both unwound scopes must be reported, or a watch would be orphaned"
        );
        assert_eq!(machine.armed_watch_count(), 0, "no watch may survive");
    }

    #[test]
    fn nested_watches_are_reported_in_lifo_order_at_their_own_boundaries() {
        // main -> outer(cache_a, inner(cache_b)). Two cache dispatches live in
        // two different enclosing scopes; both arm a watch, and the inner
        // boundary must be reported before the outer one, each before the
        // caller's next rule runs.
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                        target: SequenceRef::new("outer"),
                    }]))],
                ),
                SequenceSpec::new(
                    "outer",
                    vec![
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache_a"),
                        }])),
                        RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                            target: SequenceRef::new("inner"),
                        }])),
                    ],
                ),
                SequenceSpec::new(
                    "inner",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("cache_b"),
                        },
                    ]))],
                ),
            ],
            Vec::new(),
        )
        .with_externals(vec![
            crate::ExternalSpec::new("cache_a"),
            crate::ExternalSpec::new("cache_b"),
        ])
        .validate()
        .expect("valid nested program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");

        let MachineStep::Dispatch(outer_dispatch) = machine.step().expect("outer dispatch") else {
            panic!("the outer cache must dispatch first");
        };
        machine
            .watch_enclosing_scope(outer_dispatch.executable())
            .expect("outer watch arms");

        let MachineStep::Dispatch(inner_dispatch) = machine
            .resume(outer_dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("outer resume")
        else {
            panic!("the inner cache must dispatch next");
        };
        assert_ne!(inner_dispatch.executable(), outer_dispatch.executable());
        machine
            .watch_enclosing_scope(inner_dispatch.executable())
            .expect("inner watch arms on its own enclosing scope");
        assert_eq!(machine.armed_watch_count(), 2);

        let step = machine
            .resume(inner_dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("inner resume");
        let MachineStep::ScopeComplete(completion) = step else {
            panic!("the innermost watched scope must report first: {step:?}");
        };
        assert_eq!(
            completion.executable(),
            inner_dispatch.executable(),
            "LIFO: the inner frame completes before the outer frame"
        );
        assert_eq!(machine.armed_watch_count(), 1);

        let step = machine
            .resume_scope_completion(completion.token())
            .expect("inner scope resume");
        let MachineStep::ScopeComplete(completion) = step else {
            panic!("the outer watched scope must report second: {step:?}");
        };
        assert_eq!(completion.executable(), outer_dispatch.executable());
        assert_eq!(machine.armed_watch_count(), 0);

        assert!(matches!(
            machine
                .resume_scope_completion(completion.token())
                .expect("outer scope resume"),
            MachineStep::Complete(_)
        ));
    }

    #[test]
    fn a_watch_dropped_by_exit_never_reports_a_publishable_completion() {
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
                        target: SequenceRef::new("child"),
                    }]))],
                ),
                SequenceSpec::new(
                    "child",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("upstream"),
                        },
                        ExecutableSpec::Exit,
                    ]))],
                ),
            ],
            Vec::new(),
        )
        .with_externals(vec![crate::ExternalSpec::new("upstream")])
        .validate()
        .expect("valid exit program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");
        let MachineStep::Dispatch(dispatch) = machine.step().expect("dispatch") else {
            panic!("the external must dispatch");
        };
        machine
            .watch_enclosing_scope(dispatch.executable())
            .expect("watch arms");
        let step = machine
            .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("resume");
        assert!(
            !matches!(step, MachineStep::ScopeComplete(_)),
            "an exited scope must not report a publishable completion: {step:?}"
        );
        assert_eq!(machine.armed_watch_count(), 0, "the dead watch was dropped");
    }

    #[test]
    fn a_synthetic_inline_scope_keeps_the_enclosing_origin() {
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new(
                    "main",
                    vec![RuleSpec::unconditional(Some(vec![
                        ExecutableSpec::Call {
                            target: SequenceRef::new("child"),
                        },
                        ExecutableSpec::External {
                            target: crate::ExternalRef::new("upstream"),
                        },
                    ]))],
                ),
                SequenceSpec::new("child", Vec::new()),
            ],
            Vec::new(),
        )
        .with_externals(vec![crate::ExternalSpec::new("upstream")])
        .validate()
        .expect("valid inline program");
        let mut machine = ExecutionMachine::new(
            &program,
            entry(&program),
            state(),
            ExecutionControl::with_fuel(20),
        )
        .expect("machine");
        let step = machine.step().expect("inline list dispatch");
        let MachineStep::Dispatch(dispatch) = step else {
            panic!("the inline external must dispatch: {step:?}");
        };
        assert!(
            program.sequences.iter().any(|sequence| sequence.synthetic),
            "the multi-exec lowering must be marked synthetic"
        );
        assert!(
            program.sequence_id("<inline:0>").is_none(),
            "a synthetic scope is not a public named target"
        );
        assert_eq!(
            machine
                .last_origin()
                .and_then(|id| program.sequence(id))
                .map(|sequence| sequence.name.as_str()),
            Some("main"),
            "a synthetic inline scope must not become the reported origin"
        );
        assert_eq!(dispatch.executable().index(), 0);
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

#[cfg(test)]
mod slice5_shared_control_tests {
    use crate::{CancellationToken, ExecutionControl, RootFuelHandle};

    #[test]
    fn child_forks_debit_one_root_budget_without_replenishing_siblings() {
        let root = RootFuelHandle::new(3);
        let mut parent =
            ExecutionControl::with_shared_budget(root.clone(), CancellationToken::new());
        let mut child = parent.fork_child(CancellationToken::new());
        assert_eq!(parent.remaining_budget(), 3);
        assert!(child.try_consume().is_ok());
        assert_eq!(root.remaining(), 2);
        assert!(parent.try_consume().is_ok());
        assert_eq!(root.remaining(), 1);
        assert!(child.try_consume().is_ok());
        assert_eq!(root.remaining(), 0);
        assert!(parent.try_consume().is_err());
        assert!(child.try_consume().is_err());
    }

    #[test]
    fn child_cancellation_does_not_cancel_root_or_sibling() {
        let root = RootFuelHandle::new(4);
        let parent = ExecutionControl::with_shared_budget(root, CancellationToken::new());
        let child_cancel = CancellationToken::new();
        let mut sibling = parent.fork_child(CancellationToken::new());
        let child = parent.fork_child(child_cancel.clone());
        child_cancel.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!sibling.is_cancelled());
        assert!(sibling.try_consume().is_ok());
    }

    #[test]
    fn shared_mode_is_distinct_from_legacy_local_control() {
        let legacy = ExecutionControl::with_fuel(1);
        let clone = legacy.clone();
        assert_eq!(legacy.remaining_budget(), 1);
        assert_eq!(clone.remaining_budget(), 1);
        let root = RootFuelHandle::new(1);
        let mut shared =
            ExecutionControl::with_shared_budget(root.clone(), CancellationToken::new());
        assert!(shared.try_consume().is_ok());
        assert_eq!(root.remaining(), 0);
    }
}
