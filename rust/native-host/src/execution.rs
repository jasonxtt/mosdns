use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use mosdns_dns_core::{
    FrameMode, QueryHeader, QuestionInfo, frame_response, inspect_response_header,
    patch_response_id_ra, synthesize_response, validate_response,
};
use mosdns_sequence_core::{
    ExecutionControl, ExecutionMachine, ExecutionState, ExecutorOutcome, MachineStep, ResponseState,
};
use mosdns_upstream_core::{ExchangeResponse, TransportCancellation, UpstreamError};

use crate::assembly::{ForwardAdapter, HostOptions};
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
        query: &'a [u8],
        deadline: Instant,
        cancellation: TransportCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<ExchangeResponse, UpstreamError>> + 'a>>;
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
        query: &'a [u8],
        deadline: Instant,
        cancellation: TransportCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<ExchangeResponse, UpstreamError>> + 'a>> {
        Box::pin(ForwardAdapter::exchange(
            self,
            query,
            deadline,
            cancellation,
        ))
    }
}

/// Shared W1/W2 request driver used by both UDP and TCP listeners.
pub(crate) async fn execute_request(
    request: ExecutionRequest<'_>,
    forward: &ForwardAdapter,
    request_shutdown: TransportCancellation,
) -> Vec<u8> {
    execute_request_with_executor(request, forward, request_shutdown).await
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
    let mut machine = match config.new_machine(state, ExecutionControl::with_fuel(DEFAULT_FUEL)) {
        Ok(machine) => machine,
        Err(_) => return protocol_error(&header, &question, SERVFAIL),
    };

    let mut pending_store: Option<PendingStore> = None;
    let mut upstream_response = false;
    let mut publication_deadline = None;
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

                if dispatch.executable() != config.forward.executable {
                    return protocol_error(&header, &question, SERVFAIL);
                }
                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                let deadline = Instant::now() + options.request_deadline;
                publication_deadline = Some(deadline);
                let exchange = executor
                    .exchange(raw, deadline, request_shutdown.clone())
                    .await;
                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                match exchange {
                    Ok(response) => {
                        let accepted = patch_response_id_ra(response.wire(), header.id)
                            .ok()
                            .filter(|wire| {
                                inspect_response_header(wire).is_ok()
                                    && validate_response(wire).is_ok()
                            });
                        match accepted {
                            Some(wire) => {
                                upstream_response = true;
                                machine.state_mut().set_raw_response(wire);
                            }
                            None => {
                                upstream_response = false;
                                set_servfail(&mut machine);
                            }
                        }
                    }
                    Err(_) => {
                        upstream_response = false;
                        set_servfail(&mut machine);
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
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{
        ExecutableId, ExecutableSpec, ExternalRef, ExternalSpec, ProgramSpec, RuleSpec,
        SequenceSpec,
    };
    use mosdns_upstream_core::{Endpoint, ExchangeResponse, Transport};

    use super::{ExchangeExecutor, execute_request_with_executor};
    use crate::assembly::HostOptions;
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

    impl ExchangeExecutor for MockExchange {
        fn exchange<'a>(
            &'a self,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: mosdns_upstream_core::TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<ExchangeResponse, mosdns_upstream_core::UpstreamError>,
                    > + 'a,
            >,
        > {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Box::pin(async { Err(mosdns_upstream_core::UpstreamError::Connect) });
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
