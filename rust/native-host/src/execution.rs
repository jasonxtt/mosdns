use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use mosdns_dns_core::{
    FrameMode, QueryHeader, QuestionInfo, frame_response, inspect_response_header,
    observe_answer_addresses, observe_response_metadata, patch_response_id_ra, synthesize_response,
    validate_response,
};
use mosdns_sequence_core::{
    ExecutableId, ExecutionControl, ExecutionMachine, ExecutionState, ExecutorOutcome, MachineStep,
    ResponseState,
};
use mosdns_upstream_core::{ExchangeResponse, TransportCancellation, UpstreamError};

use crate::assembly::{ForwardAdapter, ForwardCatalog, HostOptions};
use crate::cache::{NativeCacheAdapter, PendingStore};
use crate::config::CompiledConfig;

const DEFAULT_FUEL: u64 = 64;
const SERVFAIL: u8 = 2;
const REFUSED: u8 = 5;

/// The host-side async exchange seam. It keeps the canonical sequence machine
/// independent of transport details while preserving the borrowed query's
/// lifetime through the one exchange future.
pub(crate) trait ExchangeExecutor {
    fn exchange<'a>(
        &'a self,
        executable: ExecutableId,
        query: &'a [u8],
        deadline: Instant,
        cancellation: TransportCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<ExchangeResponse, ExchangeError>> + 'a>>;
}

/// A host dispatch failure that preserves the distinction between a missing
/// validated executable identity and an upstream transport failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExchangeError {
    UnknownExecutable(ExecutableId),
    Upstream(UpstreamError),
}

pub(crate) struct ExecutionRequest<'a> {
    pub config: &'a CompiledConfig,
    pub cache: &'a NativeCacheAdapter,
    pub options: &'a HostOptions,
    pub raw: &'a [u8],
    pub header: QueryHeader,
    pub question: QuestionInfo,
}

impl ExchangeExecutor for ForwardAdapter {
    fn exchange<'a>(
        &'a self,
        executable: ExecutableId,
        query: &'a [u8],
        deadline: Instant,
        cancellation: TransportCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<ExchangeResponse, ExchangeError>> + 'a>> {
        if executable != self.executable() {
            return Box::pin(async move { Err(ExchangeError::UnknownExecutable(executable)) });
        }
        Box::pin(async move {
            ForwardAdapter::exchange(self, query, deadline, cancellation)
                .await
                .map_err(ExchangeError::Upstream)
        })
    }
}

/// Shared W1/W2 request driver used by both UDP and TCP listeners.
pub(crate) async fn execute_request(
    request: ExecutionRequest<'_>,
    forwards: &ForwardCatalog,
    request_shutdown: TransportCancellation,
) -> Vec<u8> {
    execute_request_with_executor(request, forwards, request_shutdown).await
}

pub(crate) async fn execute_request_with_executor<E: ExchangeExecutor + ?Sized>(
    request: ExecutionRequest<'_>,
    executor: &E,
    request_shutdown: TransportCancellation,
) -> Vec<u8> {
    let ExecutionRequest {
        config,
        cache,
        options,
        raw,
        header,
        question,
    } = request;
    let state = ExecutionState::new(header, question.clone());
    // Every external leg shares this one request-owned absolute budget.
    let request_deadline = options
        .admission_deadline
        .unwrap_or_else(|| Instant::now() + options.request_deadline);
    let mut machine = match config.new_machine(state, ExecutionControl::with_fuel(DEFAULT_FUEL)) {
        Ok(machine) => machine,
        Err(_) => return protocol_error(&header, &question, SERVFAIL),
    };

    let mut pending_store: Option<PendingStore> = None;
    let mut upstream_response = false;
    let mut publication_deadline = None;
    let multi_forward = config.program.externals.len() > usize::from(config.cache.is_some()) + 1;
    let mut step = match machine.step() {
        Ok(step) => step,
        Err(_) => return protocol_error(&header, &question, SERVFAIL),
    };

    loop {
        match step {
            MachineStep::Complete(_) => {
                if upstream_response
                    && !request_shutdown.is_cancelled()
                    && publication_deadline.is_some_and(|deadline| Instant::now() < deadline)
                {
                    if let Some(token) = pending_store.take() {
                        // No await occurs between this gate and the cache
                        // insert. The token itself rechecks response
                        // eligibility and obtains the publication time.
                        let _ = token.publish(match &machine.state().response {
                            ResponseState::Raw(wire) => wire.as_bytes(),
                            ResponseState::None | ResponseState::Synthesized(_) => &[],
                        });
                    }
                }
                return response_from_state(&machine, &header, &question);
            }
            MachineStep::Dispatch(dispatch) => {
                if config
                    .cache
                    .as_ref()
                    .is_some_and(|cache_config| cache_config.executable == dispatch.executable())
                {
                    let lookup = cache.lookup(raw).ok().flatten();
                    if let Some(wire) = lookup {
                        machine.state_mut().set_raw_response(wire);
                        step = match machine
                            .resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
                        {
                            Ok(step) => step,
                            Err(_) => return response_from_state(&machine, &header, &question),
                        };
                        continue;
                    }
                    pending_store = cache.begin_store(raw).ok().flatten();
                    step = match machine
                        .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
                    {
                        Ok(step) => step,
                        Err(_) => return response_from_state(&machine, &header, &question),
                    };
                    continue;
                }

                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                if multi_forward && Instant::now() >= request_deadline {
                    set_servfail(&mut machine);
                    return response_from_state(&machine, &header, &question);
                }
                publication_deadline = Some(request_deadline);
                let exchange = executor
                    .exchange(
                        dispatch.executable(),
                        raw,
                        request_deadline,
                        request_shutdown.clone(),
                    )
                    .await;
                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                match exchange {
                    Ok(response) => {
                        let accepted = if multi_forward {
                            qualify_response(response.wire(), header.id, &question)
                        } else {
                            patch_response_id_ra(response.wire(), header.id)
                                .ok()
                                .filter(|wire| {
                                    inspect_response_header(wire).is_ok()
                                        && validate_response(wire).is_ok()
                                })
                        };
                        match accepted {
                            Some(wire) => {
                                upstream_response = true;
                                machine.state_mut().set_raw_response(wire);
                            }
                            None => {
                                upstream_response = false;
                                set_servfail(&mut machine);
                                if multi_forward {
                                    return response_from_state(&machine, &header, &question);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        upstream_response = false;
                        set_servfail(&mut machine);
                        if multi_forward {
                            return response_from_state(&machine, &header, &question);
                        }
                    }
                }
                step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Continue)) {
                    Ok(step) => step,
                    Err(_) => return response_from_state(&machine, &header, &question),
                };
            }
        }
    }
}

fn qualify_response(
    response: &[u8],
    request_id: u16,
    request_question: &QuestionInfo,
) -> Option<Vec<u8>> {
    let wire = patch_response_id_ra(response, request_id).ok()?;
    let header = inspect_response_header(&wire).ok()?;
    let metadata = observe_response_metadata(&wire).ok()?;
    if !header.qr || metadata.opcode != 0 {
        return None;
    }
    let question = metadata.question.as_ref()?;
    if !question
        .qname_wire
        .eq_ignore_ascii_case(&request_question.qname_wire)
        || question.qtype != request_question.qtype
        || question.qclass != request_question.qclass
    {
        return None;
    }
    observe_answer_addresses(&wire).ok()?;
    validate_response(&wire).ok()?;
    Some(wire)
}

fn set_servfail(machine: &mut ExecutionMachine<'_>) {
    let _ = machine
        .state_mut()
        .set_synthesized_response(u16::from(SERVFAIL));
}

pub(crate) fn response_from_state(
    machine: &ExecutionMachine<'_>,
    header: &QueryHeader,
    question: &QuestionInfo,
) -> Vec<u8> {
    match &machine.state().response {
        ResponseState::Raw(wire) => wire.0.clone(),
        ResponseState::Synthesized(response) => {
            let rcode = u8::try_from(response.rcode()).unwrap_or(SERVFAIL);
            protocol_error(header, question, rcode)
        }
        ResponseState::None => protocol_error(header, question, REFUSED),
    }
}

pub(crate) fn protocol_error(header: &QueryHeader, question: &QuestionInfo, rcode: u8) -> Vec<u8> {
    synthesize_response(header, question, rcode).unwrap_or_else(|_| {
        synthesize_response(header, question, SERVFAIL).unwrap_or_else(|_| Vec::new())
    })
}

#[allow(dead_code)]
pub(crate) fn frame_native_response(response: &[u8], mode: FrameMode) -> Option<Vec<u8>> {
    frame_response(response, mode).ok()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{
        DispatchMetadata, ExecutableId, ExecutableSpec, ExternalRef, ExternalSpec,
        MatcherSpecInput, ProgramSpec, RuleSpec, SequenceSpec,
    };
    use mosdns_upstream_core::{
        Endpoint, ExchangeResponse, Transport, TransportCancellation, UpstreamError,
    };

    use super::{ExchangeExecutor, execute_request_with_executor};
    use crate::assembly::{ForwardAdapter, HostOptions};
    use crate::cache::{CacheTestClock, NativeCacheAdapter};
    use crate::config::{
        CachePluginConfig, CompiledConfig, ForwardConfig, ListenerConfig, ListenerKind, LogLevel,
        SequenceConfig,
    };

    struct MockExchange {
        calls: Rc<Cell<u32>>,
        response: Vec<u8>,
        fail: bool,
    }

    struct RecordingExchange {
        calls: Rc<RefCell<Vec<(ExecutableId, std::time::Instant)>>>,
        response: Vec<u8>,
    }

    struct FailingExchange {
        calls: Rc<RefCell<Vec<ExecutableId>>>,
    }

    struct SecondLegFailExchange {
        calls: Rc<RefCell<Vec<ExecutableId>>>,
        first_response: Vec<u8>,
        fail_id: ExecutableId,
    }

    struct CancelFirstExchange {
        calls: Rc<RefCell<Vec<ExecutableId>>>,
        exchange_count: Rc<Cell<u32>>,
        response: Vec<u8>,
    }

    impl ExchangeExecutor for FailingExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: mosdns_upstream_core::TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.borrow_mut().push(executable);
            Box::pin(async { Err(super::ExchangeError::Upstream(UpstreamError::Connect)) })
        }
    }

    impl ExchangeExecutor for SecondLegFailExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.borrow_mut().push(executable);
            if executable == self.fail_id {
                return Box::pin(async {
                    Err(super::ExchangeError::Upstream(UpstreamError::Connect))
                });
            }
            let response = self.first_response.clone();
            Box::pin(async move {
                let id = u16::from_be_bytes([response[0], response[1]]);
                Ok(ExchangeResponse::new(
                    response,
                    id,
                    id,
                    Transport::Udp,
                    false,
                ))
            })
        }
    }

    impl ExchangeExecutor for CancelFirstExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.borrow_mut().push(executable);
            let cancel = self.exchange_count.get() == 0;
            self.exchange_count.set(self.exchange_count.get() + 1);
            let response = self.response.clone();
            Box::pin(async move {
                if cancel {
                    cancellation.cancel();
                }
                let id = u16::from_be_bytes([response[0], response[1]]);
                Ok(ExchangeResponse::new(
                    response,
                    id,
                    id,
                    Transport::Udp,
                    false,
                ))
            })
        }
    }

    impl ExchangeExecutor for RecordingExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            _query: &'a [u8],
            deadline: std::time::Instant,
            _cancellation: mosdns_upstream_core::TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.borrow_mut().push((executable, deadline));
            let response = self.response.clone();
            Box::pin(async move {
                let id = u16::from_be_bytes([response[0], response[1]]);
                Ok(ExchangeResponse::new(
                    response,
                    id,
                    id,
                    Transport::Udp,
                    false,
                ))
            })
        }
    }

    impl ExchangeExecutor for MockExchange {
        fn exchange<'a>(
            &'a self,
            _executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: mosdns_upstream_core::TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Box::pin(async {
                    Err(super::ExchangeError::Upstream(
                        mosdns_upstream_core::UpstreamError::Connect,
                    ))
                });
            }
            let response = self.response.clone();
            Box::pin(async move {
                let id = u16::from_be_bytes([response[0], response[1]]);
                Ok(ExchangeResponse::new(
                    response,
                    id,
                    id,
                    Transport::Udp,
                    false,
                ))
            })
        }
    }

    fn query(id: u16) -> Vec<u8> {
        vec![
            (id >> 8) as u8,
            id as u8,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            7,
            b'e',
            b'x',
            b'a',
            b'm',
            b'p',
            b'l',
            b'e',
            0,
            0,
            1,
            0,
            1,
        ]
    }

    fn response(query: &[u8]) -> Vec<u8> {
        let (header, question) = parse_query(query).expect("query");
        let mut response = vec![
            (header.id >> 8) as u8,
            header.id as u8,
            0x81,
            0x80,
            0,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
        ];
        response.extend_from_slice(&question.qname_wire);
        response.extend_from_slice(&[0, 1, 0, 1]);
        response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 10, 0, 4, 192, 0, 2, 1]);
        response
    }

    fn malformed_address_response(query: &[u8]) -> Vec<u8> {
        let mut response = response(query);
        let rdlength_offset = response.len() - 6;
        response[rdlength_offset..rdlength_offset + 2].copy_from_slice(&3_u16.to_be_bytes());
        response
    }

    fn config() -> CompiledConfig {
        let forward = "forward".to_owned();
        let cache = "cache".to_owned();
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "root",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::External {
                        target: ExternalRef::new(cache.clone()),
                    },
                    ExecutableSpec::External {
                        target: ExternalRef::new(forward.clone()),
                    },
                ]))],
            )],
            Vec::new(),
        )
        .with_externals(vec![
            ExternalSpec::new(cache.clone()),
            ExternalSpec::new(forward.clone()),
        ])
        .validate()
        .expect("program");
        let cache_id = program.external(ExecutableId(0)).expect("cache id").id;
        let forward_id = program.external(ExecutableId(1)).expect("forward id").id;
        let endpoint = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        CompiledConfig {
            log_level: LogLevel::Error,
            forward: ForwardConfig {
                tag: forward,
                endpoint,
                executable: forward_id,
            },
            forwards: vec![ForwardConfig {
                tag: "forward".to_owned(),
                endpoint,
                executable: forward_id,
            }],
            cache: Some(CachePluginConfig {
                tag: cache,
                executable: cache_id,
            }),
            sequence: SequenceConfig {
                tag: "root".to_owned(),
                sequence: program.sequence_id("root").expect("root"),
                forward_executable: forward_id,
            },
            listener: ListenerConfig {
                tag: "listener".to_owned(),
                kind: ListenerKind::Udp,
                entry: "root".to_owned(),
                listen: "127.0.0.1:1".parse().expect("listen"),
                enable_audit: false,
                idle_timeout: None,
            },
            program,
        }
    }

    fn two_leg_config() -> (CompiledConfig, ExecutableId, ExecutableId) {
        let a = "a".to_owned();
        let b = "b".to_owned();
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "root",
                vec![
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                        target: ExternalRef::new(b.clone()),
                    }])),
                    RuleSpec::new(
                        vec![MatcherSpecInput::new(
                            Box::new(crate::matchers::TrueMatcher),
                            false,
                            DispatchMetadata::None,
                        )],
                        Some(vec![
                            ExecutableSpec::External {
                                target: ExternalRef::new(a.clone()),
                            },
                            ExecutableSpec::Exit,
                        ]),
                    ),
                    RuleSpec::unconditional(Some(vec![ExecutableSpec::External {
                        target: ExternalRef::new(b.clone()),
                    }])),
                ],
            )],
            Vec::new(),
        )
        .with_externals(vec![
            ExternalSpec::new(a.clone()),
            ExternalSpec::new(b.clone()),
        ])
        .validate()
        .expect("two leg program");
        let a_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == a).then_some(*id))
            .expect("a external");
        let b_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == b).then_some(*id))
            .expect("b external");
        let endpoint = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        (
            CompiledConfig {
                log_level: LogLevel::Error,
                forward: ForwardConfig {
                    tag: a,
                    endpoint,
                    executable: a_id,
                },
                forwards: vec![
                    ForwardConfig {
                        tag: "a".to_owned(),
                        endpoint,
                        executable: a_id,
                    },
                    ForwardConfig {
                        tag: "b".to_owned(),
                        endpoint,
                        executable: b_id,
                    },
                ],
                cache: None,
                sequence: SequenceConfig {
                    tag: "root".to_owned(),
                    sequence: program.sequence_id("root").expect("root"),
                    forward_executable: a_id,
                },
                listener: ListenerConfig {
                    tag: "listener".to_owned(),
                    kind: ListenerKind::Udp,
                    entry: "root".to_owned(),
                    listen: "127.0.0.1:1".parse().expect("listen"),
                    enable_audit: false,
                    idle_timeout: None,
                },
                program,
            },
            a_id,
            b_id,
        )
    }

    #[test]
    fn canonical_machine_resumes_two_external_legs_with_one_deadline() {
        let (config, a, b) = two_leg_config();
        let request = query(12);
        let (header, question) = parse_query(&request).expect("query");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = RecordingExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let options = HostOptions::with_deadline(Duration::from_secs(1));
        let result = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &options,
                raw: &request,
                header,
                question,
            },
            &executor,
            mosdns_upstream_core::TransportCancellation::new(),
        ));
        validate_response(&result).expect("final response");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, b);
        assert_eq!(calls[1].0, a);
        assert_eq!(calls[0].1, calls[1].1);
    }

    #[test]
    fn failed_first_multi_forward_leg_stops_before_the_next_external() {
        let (config, a, b) = two_leg_config();
        let request = query(13);
        let (header, question) = parse_query(&request).expect("query");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = FailingExchange {
            calls: Rc::clone(&calls),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let options = HostOptions::default();
        let response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &options,
                raw: &request,
                header,
                question,
            },
            &executor,
            mosdns_upstream_core::TransportCancellation::new(),
        ));
        validate_response(&response).expect("SERVFAIL response");
        assert_eq!(response[3] & 0x0f, super::SERVFAIL);
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], b);
        assert_ne!(a, b);
    }

    #[test]
    fn mismatched_question_is_terminal_and_never_reaches_the_next_external() {
        let (config, _a, b) = two_leg_config();
        let request = query(14);
        let mut wrong_query = query(15);
        wrong_query[13] = b'x';
        let (header, question) = parse_query(&request).expect("query");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = RecordingExchange {
            calls: Rc::clone(&calls),
            response: response(&wrong_query),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let options = HostOptions::default();
        let response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &options,
                raw: &request,
                header,
                question,
            },
            &executor,
            mosdns_upstream_core::TransportCancellation::new(),
        ));
        validate_response(&response).expect("SERVFAIL response");
        assert_eq!(response[3] & 0x0f, super::SERVFAIL);
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, b);
    }

    #[test]
    fn malformed_address_rdata_is_terminal_for_multi_forward_w3() {
        let (config, _a, b) = two_leg_config();
        let request = query(15);
        let (header, question) = parse_query(&request).expect("query");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = RecordingExchange {
            calls: Rc::clone(&calls),
            response: malformed_address_response(&request),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &request,
                header,
                question,
            },
            &executor,
            TransportCancellation::new(),
        ));
        validate_response(&response).expect("SERVFAIL response");
        assert_eq!(response[3] & 0x0f, super::SERVFAIL);
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, b);
    }

    #[test]
    fn failed_second_multi_forward_leg_does_not_republish_first_answer() {
        let (config, a, b) = two_leg_config();
        let request = query(16);
        let (header, question) = parse_query(&request).expect("query");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = SecondLegFailExchange {
            calls: Rc::clone(&calls),
            first_response: response(&request),
            fail_id: a,
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &request,
                header,
                question,
            },
            &executor,
            TransportCancellation::new(),
        ));
        validate_response(&response).expect("SERVFAIL response");
        assert_eq!(response[3] & 0x0f, super::SERVFAIL);
        assert_eq!(calls.borrow().as_slice(), &[b, a]);
    }

    #[test]
    fn cancellation_after_first_leg_isolated_from_the_next_request() {
        let (config, _a, b) = two_leg_config();
        let first_request = query(17);
        let second_request = query(18);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = CancelFirstExchange {
            calls: Rc::clone(&calls),
            exchange_count: Rc::new(Cell::new(0)),
            response: response(&first_request),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");

        let first_cancellation = TransportCancellation::new();
        let (header, question) = parse_query(&first_request).expect("query");
        let first_response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &first_request,
                header,
                question,
            },
            &executor,
            first_cancellation,
        ));
        assert!(
            first_response.is_empty(),
            "cancelled request must not publish"
        );

        let second_cancellation = TransportCancellation::new();
        let (header, question) = parse_query(&second_request).expect("query");
        let second_response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &second_request,
                header,
                question,
            },
            &executor,
            second_cancellation,
        ));
        validate_response(&second_response).expect("uncancelled request response");
        assert_eq!(calls.borrow().as_slice(), &[b, b, _a]);
    }

    #[test]
    fn cache_hit_skips_the_exact_forward_dispatch_and_miss_publishes_at_completion() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let first = query(1);
        let response = response(&first);
        let calls = Rc::new(Cell::new(0));
        let mock = MockExchange {
            calls: Rc::clone(&calls),
            response: response.clone(),
            fail: false,
        };
        let assembly = config();
        let first_options = HostOptions::default();
        let cancellation = mosdns_upstream_core::TransportCancellation::new();
        let (first_header, first_question) = parse_query(&first).expect("query");
        let first_result = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &first_options,
                raw: &first,
                header: first_header,
                question: first_question,
            },
            &mock,
            cancellation.clone(),
        ));
        assert_eq!(calls.get(), 1);
        validate_response(&first_result).expect("first response");
        let second = query(2);
        let second_options = HostOptions::default();
        let (second_header, second_question) = parse_query(&second).expect("query");
        let second_result = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &second_options,
                raw: &second,
                header: second_header,
                question: second_question,
            },
            &mock,
            cancellation,
        ));
        assert_eq!(calls.get(), 1);
        assert_eq!([second_result[0], second_result[1]], [0, 2]);
    }

    fn upstream_servfail(query: &[u8]) -> Vec<u8> {
        let (_, question) = parse_query(query).expect("query");
        let mut wire = response(query);
        wire[3] = 0x82;
        wire[6] = 0;
        wire[7] = 0;
        wire.truncate(12 + question.qname_wire.len() + 4);
        wire
    }

    #[test]
    fn unknown_executable_id_fails_closed_without_selecting_the_only_forward() {
        let config = config();
        let endpoint = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        let wrong_owner = ForwardAdapter::new(ExecutableId(99), endpoint);
        let request = query(11);
        let (header, question) = parse_query(&request).expect("query");
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let options = HostOptions::default();
        let response = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &options,
                raw: &request,
                header,
                question,
            },
            &wrong_owner,
            mosdns_upstream_core::TransportCancellation::new(),
        ));
        validate_response(&response).expect("SERVFAIL response");
        assert_eq!(response[3] & 0x0f, super::SERVFAIL);
    }

    #[test]
    fn upstream_servfail_is_cacheable_but_local_servfail_is_not() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let assembly = config();
        let first = query(3);
        let calls = Rc::new(Cell::new(0));
        let mock = MockExchange {
            calls: Rc::clone(&calls),
            response: upstream_servfail(&first),
            fail: false,
        };
        let options = HostOptions::default();
        let (header, question) = parse_query(&first).expect("query");
        let cancellation = mosdns_upstream_core::TransportCancellation::new();
        let _ = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &options,
                raw: &first,
                header,
                question,
            },
            &mock,
            cancellation.clone(),
        ));
        let second = query(4);
        let (header, question) = parse_query(&second).expect("query");
        let _ = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &options,
                raw: &second,
                header,
                question,
            },
            &mock,
            cancellation,
        ));
        assert_eq!(calls.get(), 1, "a valid upstream SERVFAIL has 5s retention");

        let clock = CacheTestClock::new(100);
        let local_cache = NativeCacheAdapter::for_test(clock).expect("local cache");
        let local_calls = Rc::new(Cell::new(0));
        let local_mock = MockExchange {
            calls: Rc::clone(&local_calls),
            response: Vec::new(),
            fail: true,
        };
        let first = query(5);
        let (header, question) = parse_query(&first).expect("query");
        let cancellation = mosdns_upstream_core::TransportCancellation::new();
        let _ = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &local_cache,
                options: &options,
                raw: &first,
                header,
                question,
            },
            &local_mock,
            cancellation.clone(),
        ));
        let second = query(6);
        let (header, question) = parse_query(&second).expect("query");
        let _ = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &local_cache,
                options: &options,
                raw: &second,
                header,
                question,
            },
            &local_mock,
            cancellation,
        ));
        assert_eq!(
            local_calls.get(),
            2,
            "local synthesized SERVFAIL is not cached"
        );
        assert!(local_cache.is_empty());
    }

    #[test]
    fn publication_deadline_drops_the_pending_token_without_changing_w1_output() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let assembly = config();
        let first_query = query(7);
        let calls = Rc::new(Cell::new(0));
        let mock = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&first_query),
            fail: false,
        };
        let options = HostOptions::with_deadline(Duration::ZERO);
        let (header, question) = parse_query(&first_query).expect("query");
        let cancellation = mosdns_upstream_core::TransportCancellation::new();
        let first = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &options,
                raw: &first_query,
                header,
                question,
            },
            &mock,
            cancellation.clone(),
        ));
        validate_response(&first).expect("deadline preserves response");
        assert!(cache.is_empty(), "deadline must drop the first token");
        let second_query = query(8);
        let second_options = HostOptions::default();
        let (header, question) = parse_query(&second_query).expect("query");
        let _ = futures_like_block_on(execute_request_with_executor(
            super::ExecutionRequest {
                config: &assembly,
                cache: &cache,
                options: &second_options,
                raw: &second_query,
                header,
                question,
            },
            &mock,
            cancellation,
        ));
        assert_eq!(calls.get(), 2, "deadline must prevent publication");
    }

    fn futures_like_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }
}
