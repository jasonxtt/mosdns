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
    ResponseState as MachineResponseState,
};
use mosdns_upstream_core::{ExchangeResponse, TransportCancellation, UpstreamError};

use crate::assembly::{ForwardAdapter, ForwardCatalog, HostOptions};
use crate::cache::{NativeCacheAdapter, PendingStore};
use crate::config::CompiledConfig;
use crate::observer::{
    CacheStatus, ExecutionCheckpoint, FailureProvenance, LocalFailureKind, QueryTerminalOutcome,
    ResponseSource, ResponseState as ObservedResponseState, TerminalObservation,
    UpstreamAttemptOutcome, UpstreamAttemptRecord,
};

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

/// Facts produced by the canonical execution path for one parsed query.
/// Listener-owned framing and terminal transport outcome are added later.
#[derive(Clone, Debug)]
pub(crate) struct ExecutionResult {
    pub response_wire: Vec<u8>,
    pub response: ObservedResponseState,
    pub cache_status: CacheStatus,
    pub final_sequence: Option<String>,
    pub final_upstream: Option<String>,
    pub upstream_attempts: Vec<UpstreamAttemptRecord>,
    pub failure_provenance: Option<FailureProvenance>,
}

struct ExecutionFacts<'a> {
    capture_audit_details: bool,
    cache_status: CacheStatus,
    response_source: Option<ResponseSource>,
    final_upstream: Option<String>,
    upstream_attempts: Vec<UpstreamAttemptRecord>,
    failure_provenance: Option<FailureProvenance>,
    final_sequence: Option<String>,
    checkpoint: &'a mut ExecutionCheckpoint,
    in_flight_upstream: Option<String>,
    completed: bool,
}

impl ExecutionFacts<'_> {
    fn set_response_source(&mut self, source: ResponseSource) {
        if self.capture_audit_details {
            self.response_source = Some(source);
        }
    }

    fn set_failure_provenance(&mut self, provenance: FailureProvenance) {
        if self.capture_audit_details {
            self.failure_provenance = Some(provenance);
        }
    }

    fn record_upstream_response(&mut self, upstream: String) {
        if self.capture_audit_details {
            self.upstream_attempts.push(UpstreamAttemptRecord {
                upstream: upstream.clone(),
                outcome: UpstreamAttemptOutcome::Response,
            });
            self.final_upstream = Some(upstream.clone());
            self.response_source = Some(ResponseSource::Upstream(upstream));
        } else {
            self.upstream_attempts.push(UpstreamAttemptRecord {
                upstream,
                outcome: UpstreamAttemptOutcome::Response,
            });
        }
    }

    fn record_upstream_failure(
        &mut self,
        upstream: String,
        outcome: UpstreamAttemptOutcome,
        timed_out: bool,
    ) {
        if self.capture_audit_details {
            self.failure_provenance = Some(if timed_out {
                FailureProvenance::UpstreamTimeout {
                    upstream: upstream.clone(),
                }
            } else {
                FailureProvenance::UpstreamFailure {
                    upstream: upstream.clone(),
                }
            });
            self.response_source = Some(ResponseSource::Local);
            self.final_upstream = None;
        }
        self.upstream_attempts
            .push(UpstreamAttemptRecord { upstream, outcome });
    }
}

impl Drop for ExecutionFacts<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        self.checkpoint.capture_partial(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                // A response observed by an earlier leg is not necessarily
                // the final response. Only result_from_wire can establish
                // final response provenance, so an unfinished execution
                // must not publish an intermediate W3 answer here.
                response: ObservedResponseState::NoResponse,
                cache_status: self.cache_status,
                final_sequence: self.final_sequence.clone(),
                final_upstream: None,
                upstream_attempts: self.upstream_attempts.clone(),
                failure_provenance: self.failure_provenance.clone(),
                elapsed: std::time::Duration::ZERO,
            },
            self.in_flight_upstream.clone(),
        );
    }
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
    checkpoint: &mut ExecutionCheckpoint,
) -> ExecutionResult {
    execute_request_with_observation(request, forwards, request_shutdown, checkpoint).await
}

#[cfg(test)]
pub(crate) async fn execute_request_with_executor<E: ExchangeExecutor + ?Sized>(
    request: ExecutionRequest<'_>,
    executor: &E,
    request_shutdown: TransportCancellation,
) -> Vec<u8> {
    let mut checkpoint = ExecutionCheckpoint::new(true);
    execute_request_with_observation(request, executor, request_shutdown, &mut checkpoint)
        .await
        .response_wire
}

pub(crate) async fn execute_request_with_observation<E: ExchangeExecutor + ?Sized>(
    request: ExecutionRequest<'_>,
    executor: &E,
    request_shutdown: TransportCancellation,
    checkpoint: &mut ExecutionCheckpoint,
) -> ExecutionResult {
    let ExecutionRequest {
        config,
        cache,
        options,
        raw,
        header,
        question,
    } = request;
    let capture_audit_details = checkpoint.capture_audit_details();
    let multi_forward = config.program.externals.len() > usize::from(config.cache.is_some()) + 1;
    let mut facts = ExecutionFacts {
        capture_audit_details,
        cache_status: if config.cache.is_some() {
            CacheStatus::Undetermined
        } else {
            CacheStatus::NotApplicable
        },
        response_source: None,
        final_upstream: None,
        upstream_attempts: Vec::with_capacity(if multi_forward {
            config.forwards.len()
        } else {
            0
        }),
        failure_provenance: None,
        final_sequence: capture_audit_details.then(|| config.sequence.tag.clone()),
        checkpoint,
        in_flight_upstream: None,
        completed: false,
    };
    let state = ExecutionState::new(header, question.clone());
    // Every external leg shares this one request-owned absolute budget.
    let request_deadline = options
        .admission_deadline
        .unwrap_or_else(|| Instant::now() + options.request_deadline);
    let mut machine = match config.new_machine(state, ExecutionControl::with_fuel(DEFAULT_FUEL)) {
        Ok(machine) => machine,
        Err(_) => {
            facts.cache_status = CacheStatus::NotApplicable;
            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution,
            ));
            return result_from_wire(protocol_error(&header, &question, SERVFAIL), facts);
        }
    };

    let mut pending_store: Option<PendingStore> = None;
    let mut upstream_response = false;
    let mut publication_deadline = None;
    let mut step = match machine.step() {
        Ok(step) => step,
        Err(_) => {
            facts.cache_status = CacheStatus::NotApplicable;
            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution,
            ));
            return result_from_wire(protocol_error(&header, &question, SERVFAIL), facts);
        }
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
                            MachineResponseState::Raw(wire) => wire.as_bytes(),
                            MachineResponseState::None | MachineResponseState::Synthesized(_) => {
                                &[]
                            }
                        });
                    }
                }
                return result_from_state(&machine, &header, &question, facts);
            }
            MachineStep::Dispatch(dispatch) => {
                if config
                    .cache
                    .as_ref()
                    .is_some_and(|cache_config| cache_config.executable == dispatch.executable())
                {
                    let lookup = cache.lookup(raw).ok().flatten();
                    if let Some(wire) = lookup {
                        facts.cache_status = CacheStatus::Hit;
                        facts.set_response_source(ResponseSource::Cache);
                        facts.final_upstream = None;
                        facts.failure_provenance = None;
                        machine.state_mut().set_raw_response(wire);
                        step = match machine
                            .resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
                        {
                            Ok(step) => step,
                            Err(_) => {
                                facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                    LocalFailureKind::InternalExecution,
                                ));
                                return result_from_state(&machine, &header, &question, facts);
                            }
                        };
                        continue;
                    }
                    facts.cache_status = CacheStatus::Miss;
                    pending_store = cache.begin_store(raw).ok().flatten();
                    step = match machine
                        .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
                    {
                        Ok(step) => step,
                        Err(_) => {
                            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                LocalFailureKind::InternalExecution,
                            ));
                            return result_from_state(&machine, &header, &question, facts);
                        }
                    };
                    continue;
                }

                if request_shutdown.is_cancelled() {
                    return canceled_execution(facts);
                }
                if multi_forward && Instant::now() >= request_deadline {
                    set_servfail(&mut machine);
                    facts.set_response_source(ResponseSource::Local);
                    facts.final_upstream = None;
                    facts.set_failure_provenance(FailureProvenance::LocalFailure(
                        LocalFailureKind::NoUsableUpstreamResponse,
                    ));
                    return result_from_state(&machine, &header, &question, facts);
                }
                publication_deadline = Some(request_deadline);
                facts.in_flight_upstream = upstream_identity(config, dispatch.executable());
                let exchange = executor
                    .exchange(
                        dispatch.executable(),
                        raw,
                        request_deadline,
                        request_shutdown.clone(),
                    )
                    .await;
                facts.in_flight_upstream = None;
                if request_shutdown.is_cancelled() {
                    record_canceled_attempt(config, dispatch.executable(), &exchange, &mut facts);
                    return canceled_execution(facts);
                }
                match exchange {
                    Ok(response) => {
                        let upstream = upstream_identity(config, dispatch.executable());
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
                                if let Some(upstream) = upstream {
                                    facts.record_upstream_response(upstream);
                                }
                                facts.failure_provenance = None;
                                upstream_response = true;
                                machine.state_mut().set_raw_response(wire);
                            }
                            None => {
                                if let Some(upstream) = upstream {
                                    facts.record_upstream_failure(
                                        upstream,
                                        UpstreamAttemptOutcome::Failed,
                                        false,
                                    );
                                } else {
                                    facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                        LocalFailureKind::InternalExecution,
                                    ));
                                }
                                upstream_response = false;
                                set_servfail(&mut machine);
                                facts.set_response_source(ResponseSource::Local);
                                facts.final_upstream = None;
                                if multi_forward {
                                    return result_from_state(&machine, &header, &question, facts);
                                }
                            }
                        }
                    }
                    Err(ExchangeError::UnknownExecutable(_)) => {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        upstream_response = false;
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        facts.final_upstream = None;
                        if multi_forward {
                            return result_from_state(&machine, &header, &question, facts);
                        }
                    }
                    Err(error @ ExchangeError::Upstream(_)) => {
                        if let Some(upstream) = upstream_identity(config, dispatch.executable()) {
                            let timeout = matches!(
                                &error,
                                ExchangeError::Upstream(upstream) if is_timeout(upstream)
                            );
                            let outcome = if timeout {
                                UpstreamAttemptOutcome::TimedOut
                            } else {
                                UpstreamAttemptOutcome::Failed
                            };
                            facts.record_upstream_failure(upstream, outcome, timeout);
                        } else {
                            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                LocalFailureKind::InternalExecution,
                            ));
                        }
                        upstream_response = false;
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        facts.final_upstream = None;
                        if multi_forward {
                            return result_from_state(&machine, &header, &question, facts);
                        }
                    }
                }
                step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Continue)) {
                    Ok(step) => step,
                    Err(_) => {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        return result_from_state(&machine, &header, &question, facts);
                    }
                };
            }
        }
    }
}

fn upstream_identity(config: &CompiledConfig, executable: ExecutableId) -> Option<String> {
    config
        .forwards
        .iter()
        .find(|forward| forward.executable == executable)
        .map(|forward| {
            forward
                .upstream_tag
                .as_deref()
                .unwrap_or(&forward.tag)
                .to_owned()
        })
}

fn is_timeout(error: &UpstreamError) -> bool {
    match error {
        UpstreamError::DeadlineExceeded(_) => true,
        UpstreamError::Diagnosed { cause, .. } => {
            matches!(
                cause,
                mosdns_upstream_core::TerminalError::DeadlineExceeded(_)
            )
        }
        UpstreamError::TcpFallback { cause, .. } => is_timeout(cause),
        _ => false,
    }
}

fn is_canceled(error: &UpstreamError) -> bool {
    match error {
        UpstreamError::Cancelled(_) => true,
        UpstreamError::Diagnosed { cause, .. } => {
            matches!(cause, mosdns_upstream_core::TerminalError::Cancelled(_))
        }
        UpstreamError::TcpFallback { cause, .. } => is_canceled(cause),
        _ => false,
    }
}

fn record_canceled_attempt(
    config: &CompiledConfig,
    executable: ExecutableId,
    exchange: &Result<ExchangeResponse, ExchangeError>,
    facts: &mut ExecutionFacts,
) {
    let Some(upstream) = upstream_identity(config, executable) else {
        return;
    };
    let outcome = match exchange {
        Ok(_) => UpstreamAttemptOutcome::Response,
        Err(ExchangeError::Upstream(error)) if is_timeout(error) => {
            UpstreamAttemptOutcome::TimedOut
        }
        Err(ExchangeError::Upstream(error)) if is_canceled(error) => {
            UpstreamAttemptOutcome::Canceled
        }
        Err(ExchangeError::Upstream(_)) => UpstreamAttemptOutcome::Failed,
        Err(ExchangeError::UnknownExecutable(_)) => return,
    };
    facts
        .upstream_attempts
        .push(UpstreamAttemptRecord { upstream, outcome });
}

fn canceled_execution(mut facts: ExecutionFacts) -> ExecutionResult {
    facts.response_source = None;
    facts.final_upstream = None;
    result_from_wire(Vec::new(), facts)
}

fn result_from_state(
    machine: &ExecutionMachine<'_>,
    header: &QueryHeader,
    question: &QuestionInfo,
    mut facts: ExecutionFacts,
) -> ExecutionResult {
    if facts.cache_status == CacheStatus::Undetermined {
        facts.cache_status = CacheStatus::NotApplicable;
    }
    let response = response_from_state(machine, header, question);
    result_from_wire(response, facts)
}

fn result_from_wire(response_wire: Vec<u8>, mut facts: ExecutionFacts) -> ExecutionResult {
    let response = observed_response(
        &response_wire,
        facts
            .response_source
            .take()
            .unwrap_or(ResponseSource::Local),
    );
    let final_upstream = match &response {
        ObservedResponseState::Dns {
            source: ResponseSource::Upstream(_),
            ..
        } => facts.final_upstream.take(),
        ObservedResponseState::Dns { .. } | ObservedResponseState::NoResponse => None,
    };
    facts.completed = true;
    ExecutionResult {
        response_wire,
        response,
        cache_status: facts.cache_status,
        final_sequence: facts.final_sequence.take(),
        final_upstream,
        upstream_attempts: std::mem::take(&mut facts.upstream_attempts),
        failure_provenance: facts.failure_provenance.take(),
    }
}

fn observed_response(response_wire: &[u8], source: ResponseSource) -> ObservedResponseState {
    if response_wire.is_empty() {
        ObservedResponseState::NoResponse
    } else {
        let rcode = observe_response_metadata(response_wire)
            .map(|metadata| metadata.rcode)
            .unwrap_or_else(|_| {
                u16::from(response_wire.get(3).copied().unwrap_or_default() & 0x0f)
            });
        ObservedResponseState::Dns { rcode, source }
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
        MachineResponseState::Raw(wire) => wire.0.clone(),
        MachineResponseState::Synthesized(response) => {
            let rcode = u8::try_from(response.rcode()).unwrap_or(SERVFAIL);
            protocol_error(header, question, rcode)
        }
        MachineResponseState::None => protocol_error(header, question, REFUSED),
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
    use std::num::NonZeroUsize;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{
        DispatchMetadata, ExecutableId, ExecutableSpec, ExternalRef, ExternalSpec,
        MatcherSpecInput, ProgramSpec, RuleSpec, SequenceSpec,
    };
    use mosdns_upstream_core::{
        Endpoint, ExchangeResponse, SideEffectState, Transport, TransportCancellation,
        UpstreamError,
    };

    use super::{
        ExchangeExecutor, ExecutionCheckpoint, ExecutionRequest, execute_request_with_executor,
        execute_request_with_observation,
    };
    use crate::assembly::{ForwardAdapter, HostOptions};
    use crate::cache::{CacheTestClock, NativeCacheAdapter};
    use crate::config::{
        CachePluginConfig, CompiledConfig, ForwardConfig, ListenerConfig, ListenerKind, LogLevel,
        SequenceConfig, compile_yaml,
    };
    use crate::observer::{
        CacheStatus, FailureProvenance, QueryObserver, QueryTerminalOutcome, QueryTransport,
        ResponseSource, ResponseState, TerminalObservation,
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

    struct TimeoutExchange;

    struct PendingExchange {
        entered: Rc<Cell<bool>>,
    }

    struct FirstLegThenPendingExchange {
        first_leg: ExecutableId,
        entered_pending_leg: Rc<Cell<bool>>,
    }

    struct RoutePathExchange {
        b: ExecutableId,
        calls: Rc<RefCell<Vec<ExecutableId>>>,
        b_answer_ip: [u8; 4],
        b_servfail: bool,
    }

    struct InterleavedExchange {
        calls: Rc<RefCell<Vec<(u16, ExecutableId)>>>,
        cancelled_request: u16,
        barrier_executable: ExecutableId,
        barrier: Arc<tokio::sync::Barrier>,
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

    impl ExchangeExecutor for PendingExchange {
        fn exchange<'a>(
            &'a self,
            _executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.entered.set(true);
            Box::pin(async {
                std::future::pending::<Result<ExchangeResponse, super::ExchangeError>>().await
            })
        }
    }

    impl ExchangeExecutor for FirstLegThenPendingExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            if executable == self.first_leg {
                let response = response(query);
                return Box::pin(async move {
                    let id = u16::from_be_bytes([response[0], response[1]]);
                    Ok(ExchangeResponse::new(
                        response,
                        id,
                        id,
                        Transport::Udp,
                        false,
                    ))
                });
            }

            self.entered_pending_leg.set(true);
            Box::pin(async {
                std::future::pending::<Result<ExchangeResponse, super::ExchangeError>>().await
            })
        }
    }

    impl ExchangeExecutor for TimeoutExchange {
        fn exchange<'a>(
            &'a self,
            _executable: ExecutableId,
            _query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            Box::pin(async {
                Err(super::ExchangeError::Upstream(
                    UpstreamError::DeadlineExceeded(SideEffectState::Sent),
                ))
            })
        }
    }

    impl ExchangeExecutor for RoutePathExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            query: &'a [u8],
            _deadline: std::time::Instant,
            _cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            self.calls.borrow_mut().push(executable);
            let response = if executable == self.b && self.b_servfail {
                upstream_servfail(query)
            } else {
                let answer_ip = if executable == self.b {
                    self.b_answer_ip
                } else {
                    [198, 51, 100, 1]
                };
                response_with_ip(query, answer_ip)
            };
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

    impl ExchangeExecutor for InterleavedExchange {
        fn exchange<'a>(
            &'a self,
            executable: ExecutableId,
            query: &'a [u8],
            _deadline: std::time::Instant,
            cancellation: TransportCancellation,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ExchangeResponse, super::ExchangeError>>
                    + 'a,
            >,
        > {
            let request_id = u16::from_be_bytes([query[0], query[1]]);
            self.calls.borrow_mut().push((request_id, executable));
            let barrier = Arc::clone(&self.barrier);
            let wait_for_b = executable == self.barrier_executable;
            let cancel = request_id == self.cancelled_request;
            let response = self.response.clone();
            Box::pin(async move {
                if wait_for_b {
                    barrier.wait().await;
                }
                if cancel && wait_for_b {
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

    fn response_with_ip(query: &[u8], address: [u8; 4]) -> Vec<u8> {
        let mut response = response(query);
        let address_offset = response.len() - address.len();
        response[address_offset..].copy_from_slice(&address);
        response
    }

    fn query_name(id: u16, name: &str) -> Vec<u8> {
        let mut query = vec![(id >> 8) as u8, id as u8, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.trim_end_matches('.').split('.') {
            query.push(u8::try_from(label.len()).expect("test label length"));
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        query
    }

    fn execute_observed<E: ExchangeExecutor + ?Sized>(
        config: &CompiledConfig,
        cache: &NativeCacheAdapter,
        options: &HostOptions,
        request: &[u8],
        executor: &E,
    ) -> super::ExecutionResult {
        let (header, question) = parse_query(request).expect("query");
        let mut checkpoint = ExecutionCheckpoint::new(true);
        futures_like_block_on(super::execute_request_with_observation(
            super::ExecutionRequest {
                config,
                cache,
                options,
                raw: request,
                header,
                question,
            },
            executor,
            TransportCancellation::new(),
            &mut checkpoint,
        ))
    }

    #[test]
    fn audit_disabled_execution_keeps_metric_facts_without_audit_details() {
        let config = compile_yaml(include_str!(
            "../../../tests/phase5a-baseline/configs/forward-udp.yaml"
        ))
        .expect("W1 configuration");
        let request = query(30);
        let (_, question) = parse_query(&request).expect("query");
        let executor = MockExchange {
            calls: Rc::new(Cell::new(0)),
            response: response(&request),
            fail: false,
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let cancellation = TransportCancellation::new();
        let observer = Arc::new(QueryObserver::new(
            false,
            ["phase5a_forward".to_owned()],
            NonZeroUsize::new(8).expect("nonzero audit capacity"),
        ));
        let mut admitted = observer.admit(
            "192.0.2.30:53000".parse().expect("client address"),
            QueryTransport::Udp,
            &question,
            cancellation.clone(),
        );
        let (header, question) = parse_query(&request).expect("query");
        let result = futures_like_block_on(execute_request_with_observation(
            ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &request,
                header,
                question,
            },
            &executor,
            cancellation,
            admitted.execution_checkpoint(),
        ));
        assert!(result.final_sequence.is_none());
        assert!(result.final_upstream.is_none());
        assert!(result.failure_provenance.is_none());
        assert_eq!(result.upstream_attempts.len(), 1);
        assert_eq!(result.upstream_attempts[0].upstream, "phase5a_forward");

        admitted.capture_execution(TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: result.response,
            cache_status: result.cache_status,
            final_sequence: result.final_sequence,
            final_upstream: result.final_upstream,
            upstream_attempts: result.upstream_attempts,
            failure_provenance: result.failure_provenance,
            elapsed: Duration::ZERO,
        });
        admitted.finish(QueryTerminalOutcome::SendSucceeded);

        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(metrics.in_flight, 0);
        assert_eq!(metrics.send_succeeded_total, 1);
        assert_eq!(metrics.response_code_totals.get(&0), Some(&1));
        assert_eq!(metrics.cache_not_applicable_total, 1);
        let forward = metrics
            .forward_attempts_by_upstream
            .get("phase5a_forward")
            .expect("configured forward metrics");
        assert_eq!(forward.attempts_total, 1);
        assert_eq!(forward.responses_total, 1);
        assert!(observer.audit_snapshot().records.is_empty());
    }

    #[test]
    fn execution_observation_tracks_w1_w2_and_servfail_provenance() {
        let w1 = compile_yaml(include_str!(
            "../../../tests/phase5a-baseline/configs/forward-udp.yaml"
        ))
        .expect("W1 configuration");
        let w1_cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("W1 cache");
        let request = query(31);
        let w1_executor = MockExchange {
            calls: Rc::new(Cell::new(0)),
            response: response(&request),
            fail: false,
        };
        let direct = execute_observed(
            &w1,
            &w1_cache,
            &HostOptions::default(),
            &request,
            &w1_executor,
        );
        assert!(
            matches!(direct.response, ResponseState::Dns { rcode: 0, source: ResponseSource::Upstream(ref upstream) } if upstream == "phase5a_forward")
        );
        assert_eq!(direct.cache_status, CacheStatus::NotApplicable);
        assert_eq!(direct.final_upstream.as_deref(), Some("phase5a_forward"));
        assert_eq!(direct.upstream_attempts.len(), 1);
        assert_eq!(
            direct.upstream_attempts[0].outcome,
            super::UpstreamAttemptOutcome::Response
        );

        let w2 = config();
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100)).expect("W2 cache");
        let calls = Rc::new(Cell::new(0));
        let executor = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
            fail: false,
        };
        let cold = execute_observed(&w2, &cache, &HostOptions::default(), &request, &executor);
        let warm_request = query(32);
        let warm = execute_observed(
            &w2,
            &cache,
            &HostOptions::default(),
            &warm_request,
            &executor,
        );
        assert_eq!(cold.cache_status, CacheStatus::Miss);
        assert_eq!(cold.final_upstream.as_deref(), Some("forward"));
        assert_eq!(warm.cache_status, CacheStatus::Hit);
        assert_eq!(
            warm.response,
            ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Cache
            }
        );
        assert_eq!(warm.final_upstream, None);
        assert!(warm.upstream_attempts.is_empty());
        assert_eq!(calls.get(), 1, "warm cache hit must not dispatch upstream");

        let timeout = execute_observed(
            &w2,
            &NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("timeout cache"),
            &HostOptions::default(),
            &query(33),
            &TimeoutExchange,
        );
        assert_eq!(
            timeout.response,
            ResponseState::Dns {
                rcode: 2,
                source: ResponseSource::Local
            }
        );
        assert_eq!(
            timeout.failure_provenance,
            Some(FailureProvenance::UpstreamTimeout {
                upstream: "forward".to_owned()
            })
        );
        assert_eq!(timeout.upstream_attempts.len(), 1);
        assert_eq!(
            timeout.upstream_attempts[0].outcome,
            super::UpstreamAttemptOutcome::TimedOut
        );

        let mut upstream_servfail = response(&request);
        upstream_servfail[3] = 0x82;
        upstream_servfail[6] = 0;
        upstream_servfail[7] = 0;
        upstream_servfail
            .truncate(12 + parse_query(&request).expect("question").1.qname_wire.len() + 4);
        let upstream_servfail_executor = MockExchange {
            calls: Rc::new(Cell::new(0)),
            response: upstream_servfail,
            fail: false,
        };
        let upstream_servfail_result = execute_observed(
            &w1,
            &w1_cache,
            &HostOptions::default(),
            &request,
            &upstream_servfail_executor,
        );
        assert_eq!(
            upstream_servfail_result.response,
            ResponseState::Dns {
                rcode: 2,
                source: ResponseSource::Upstream("phase5a_forward".to_owned())
            }
        );
        assert_eq!(upstream_servfail_result.failure_provenance, None);
    }

    #[test]
    fn dropped_w2_execution_retains_cache_miss_and_interrupted_forward_attempt() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let local = tokio::task::LocalSet::new();
        let observer = std::sync::Arc::new(QueryObserver::new(
            true,
            ["forward".to_owned()],
            std::num::NonZeroUsize::new(2).expect("audit capacity"),
        ));
        let entered = Rc::new(Cell::new(false));

        local.block_on(&runtime, async {
            let task_observer = std::sync::Arc::clone(&observer);
            let task_entered = Rc::clone(&entered);
            let runner = tokio::task::spawn_local(async move {
                let config = config();
                let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("W2 cache");
                let options = HostOptions::default();
                let raw = query(84);
                let (header, question) = parse_query(&raw).expect("query");
                let cancellation = TransportCancellation::new();
                let mut admitted = task_observer.admit(
                    "192.0.2.84:53000".parse().expect("client address"),
                    QueryTransport::Udp,
                    &question,
                    cancellation.clone(),
                );
                let _ = execute_request_with_observation(
                    ExecutionRequest {
                        config: &config,
                        cache: &cache,
                        options: &options,
                        raw: &raw,
                        header,
                        question,
                    },
                    &PendingExchange {
                        entered: task_entered,
                    },
                    cancellation,
                    admitted.execution_checkpoint(),
                )
                .await;
                panic!("pending upstream exchange unexpectedly returned");
            });

            while !entered.get() {
                tokio::task::yield_now().await;
            }
            runner.abort();
            let _ = runner.await;
        });

        let audit = observer.audit_snapshot();
        assert_eq!(audit.records.len(), 1);
        let record = &audit.records[0];
        assert_eq!(record.cache_status, CacheStatus::Miss);
        assert_eq!(record.final_sequence.as_deref(), Some("root"));
        assert_eq!(record.final_upstream, None);
        assert_eq!(record.response, ResponseState::NoResponse);
        assert_eq!(record.upstream_attempts.len(), 1);
        assert_eq!(record.upstream_attempts[0].upstream, "forward");
        assert_eq!(
            record.upstream_attempts[0].outcome,
            super::UpstreamAttemptOutcome::Interrupted
        );
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.cache_misses_total, 1);
        assert_eq!(metrics.cache_not_applicable_total, 0);
        assert_eq!(metrics.cache_undetermined_total, 0);
        let forward = metrics
            .forward_attempts_by_upstream
            .get("forward")
            .expect("forward metrics");
        assert_eq!(forward.attempts_total, 1);
        assert_eq!(forward.interrupted_total, 1);
    }

    #[test]
    fn dropped_w3_execution_does_not_publish_intermediate_response_as_final() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let local = tokio::task::LocalSet::new();
        let observer = std::sync::Arc::new(QueryObserver::new(
            true,
            ["a".to_owned(), "b".to_owned()],
            std::num::NonZeroUsize::new(2).expect("audit capacity"),
        ));
        let entered_pending_leg = Rc::new(Cell::new(false));

        local.block_on(&runtime, async {
            let task_observer = std::sync::Arc::clone(&observer);
            let task_entered = Rc::clone(&entered_pending_leg);
            let runner = tokio::task::spawn_local(async move {
                let (config, _a, b) = two_leg_config();
                let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
                let options = HostOptions::default();
                let raw = query_name(85, "dropped-w3.test");
                let (header, question) = parse_query(&raw).expect("query");
                let cancellation = TransportCancellation::new();
                let mut admitted = task_observer.admit(
                    "192.0.2.85:53000".parse().expect("client address"),
                    QueryTransport::Udp,
                    &question,
                    cancellation.clone(),
                );
                let _ = execute_request_with_observation(
                    ExecutionRequest {
                        config: &config,
                        cache: &cache,
                        options: &options,
                        raw: &raw,
                        header,
                        question,
                    },
                    &FirstLegThenPendingExchange {
                        first_leg: b,
                        entered_pending_leg: task_entered,
                    },
                    cancellation,
                    admitted.execution_checkpoint(),
                )
                .await;
                panic!("second upstream leg unexpectedly returned");
            });

            while !entered_pending_leg.get() {
                tokio::task::yield_now().await;
            }
            runner.abort();
            let _ = runner.await;
        });

        let audit = observer.audit_snapshot();
        assert_eq!(audit.records.len(), 1);
        let record = &audit.records[0];
        assert_eq!(record.qname, "dropped-w3.test.");
        assert_eq!(record.cache_status, CacheStatus::NotApplicable);
        assert_eq!(record.final_sequence.as_deref(), Some("root"));
        assert_eq!(record.final_upstream, None);
        assert_eq!(record.response, ResponseState::NoResponse);
        assert_eq!(
            record
                .upstream_attempts
                .iter()
                .map(|attempt| (attempt.upstream.as_str(), attempt.outcome))
                .collect::<Vec<_>>(),
            [
                ("b", super::UpstreamAttemptOutcome::Response),
                ("a", super::UpstreamAttemptOutcome::Interrupted),
            ]
        );
    }

    #[test]
    fn execution_observation_reports_actual_w3_final_route_and_ordered_legs() {
        let config = compile_yaml(include_str!(
            "../../../tests/phase5a-baseline/configs/routing.yaml"
        ))
        .expect("W3 configuration");
        let a = config
            .forwards
            .iter()
            .find(|forward| forward.upstream_tag.as_deref() == Some("route_a"))
            .expect("route A");
        let b = config
            .forwards
            .iter()
            .find(|forward| forward.upstream_tag.as_deref() == Some("route_b"))
            .expect("route B");
        let c = config
            .forwards
            .iter()
            .find(|forward| forward.upstream_tag.as_deref() == Some("route_c"))
            .expect("route C");
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");

        let direct_calls = Rc::new(RefCell::new(Vec::new()));
        let direct_executor = RoutePathExchange {
            b: b.executable,
            calls: Rc::clone(&direct_calls),
            b_answer_ip: [192, 0, 2, 11],
            b_servfail: false,
        };
        let direct = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(41, "domain-hit.test"),
            &direct_executor,
        );
        assert_eq!(direct_calls.borrow().as_slice(), &[a.executable]);
        assert_eq!(direct.final_upstream.as_deref(), Some("route_a"));

        let route_a_calls = Rc::new(RefCell::new(Vec::new()));
        let route_a_executor = RoutePathExchange {
            b: b.executable,
            calls: Rc::clone(&route_a_calls),
            b_answer_ip: [192, 0, 2, 10],
            b_servfail: false,
        };
        let route_a = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(42, "unmatched.test"),
            &route_a_executor,
        );
        assert_eq!(
            route_a_calls.borrow().as_slice(),
            &[b.executable, a.executable]
        );
        assert_eq!(
            route_a
                .upstream_attempts
                .iter()
                .map(|attempt| attempt.upstream.as_str())
                .collect::<Vec<_>>(),
            ["route_b", "route_a"]
        );
        assert_eq!(route_a.final_upstream.as_deref(), Some("route_a"));

        let route_c_calls = Rc::new(RefCell::new(Vec::new()));
        let route_c_executor = RoutePathExchange {
            b: b.executable,
            calls: Rc::clone(&route_c_calls),
            b_answer_ip: [192, 0, 2, 11],
            b_servfail: false,
        };
        let route_c = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(43, "unmatched-c.test"),
            &route_c_executor,
        );
        assert_eq!(
            route_c_calls.borrow().as_slice(),
            &[b.executable, c.executable]
        );
        assert_eq!(
            route_c
                .upstream_attempts
                .iter()
                .map(|attempt| attempt.upstream.as_str())
                .collect::<Vec<_>>(),
            ["route_b", "route_c"]
        );
        assert_eq!(route_c.final_upstream.as_deref(), Some("route_c"));

        let negative_b_calls = Rc::new(RefCell::new(Vec::new()));
        let negative_b_executor = RoutePathExchange {
            b: b.executable,
            calls: Rc::clone(&negative_b_calls),
            b_answer_ip: [192, 0, 2, 11],
            b_servfail: true,
        };
        let negative_b = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(44, "negative-b.test"),
            &negative_b_executor,
        );
        assert_eq!(
            negative_b_calls.borrow().as_slice(),
            &[b.executable, c.executable]
        );
        assert_eq!(
            negative_b
                .upstream_attempts
                .iter()
                .map(|attempt| (attempt.upstream.as_str(), attempt.outcome))
                .collect::<Vec<_>>(),
            [
                ("route_b", super::UpstreamAttemptOutcome::Response),
                ("route_c", super::UpstreamAttemptOutcome::Response),
            ]
        );
        assert_eq!(negative_b.final_upstream.as_deref(), Some("route_c"));
        assert_eq!(negative_b.failure_provenance, None);
        assert_eq!(
            negative_b.response,
            ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Upstream("route_c".to_owned())
            }
        );
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
                upstream_tag: None,
                endpoint,
                executable: forward_id,
            },
            forwards: vec![ForwardConfig {
                tag: "forward".to_owned(),
                upstream_tag: None,
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
                    upstream_tag: None,
                    endpoint,
                    executable: a_id,
                },
                forwards: vec![
                    ForwardConfig {
                        tag: "a".to_owned(),
                        upstream_tag: None,
                        endpoint,
                        executable: a_id,
                    },
                    ForwardConfig {
                        tag: "b".to_owned(),
                        upstream_tag: None,
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
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = FailingExchange {
            calls: Rc::clone(&calls),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let execution = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &request,
            &executor,
        );
        validate_response(&execution.response_wire).expect("SERVFAIL response");
        assert_eq!(execution.response_wire[3] & 0x0f, super::SERVFAIL);
        assert_eq!(
            execution.response,
            ResponseState::Dns {
                rcode: 2,
                source: ResponseSource::Local
            }
        );
        assert_eq!(execution.final_upstream, None);
        assert_eq!(
            execution.failure_provenance,
            Some(FailureProvenance::UpstreamFailure {
                upstream: "b".to_owned()
            })
        );
        assert_eq!(execution.upstream_attempts.len(), 1);
        assert_eq!(
            execution.upstream_attempts[0].outcome,
            super::UpstreamAttemptOutcome::Failed
        );
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
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = SecondLegFailExchange {
            calls: Rc::clone(&calls),
            first_response: response(&request),
            fail_id: a,
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let execution = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &request,
            &executor,
        );
        validate_response(&execution.response_wire).expect("SERVFAIL response");
        assert_eq!(execution.response_wire[3] & 0x0f, super::SERVFAIL);
        assert_eq!(execution.final_upstream, None);
        assert_eq!(
            execution.failure_provenance,
            Some(FailureProvenance::UpstreamFailure {
                upstream: "a".to_owned()
            })
        );
        assert_eq!(
            execution
                .upstream_attempts
                .iter()
                .map(|attempt| (attempt.upstream.as_str(), attempt.outcome))
                .collect::<Vec<_>>(),
            [
                ("b", super::UpstreamAttemptOutcome::Response),
                ("a", super::UpstreamAttemptOutcome::Failed),
            ]
        );
        assert_eq!(calls.borrow().as_slice(), &[b, a]);
    }

    #[test]
    fn interleaved_request_cancellation_does_not_cross_request_state() {
        let (config, a, b) = two_leg_config();
        let cancelled_request = query(17);
        let live_request = query(18);
        let cancelled_id = u16::from_be_bytes([cancelled_request[0], cancelled_request[1]]);
        let live_id = u16::from_be_bytes([live_request[0], live_request[1]]);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = InterleavedExchange {
            calls: Rc::clone(&calls),
            cancelled_request: cancelled_id,
            barrier_executable: b,
            barrier: Arc::new(tokio::sync::Barrier::new(2)),
            response: response(&cancelled_request),
        };
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");

        let cancelled_options = HostOptions::default();
        let live_options = HostOptions::default();
        let (cancelled_header, cancelled_question) =
            parse_query(&cancelled_request).expect("cancelled query");
        let (live_header, live_question) = parse_query(&live_request).expect("live query");
        let cancelled_cancellation = TransportCancellation::new();
        let live_cancellation = TransportCancellation::new();
        let (cancelled_response, live_response) = futures_like_block_on(async {
            tokio::join!(
                execute_request_with_executor(
                    super::ExecutionRequest {
                        config: &config,
                        cache: &cache,
                        options: &cancelled_options,
                        raw: &cancelled_request,
                        header: cancelled_header,
                        question: cancelled_question,
                    },
                    &executor,
                    cancelled_cancellation,
                ),
                execute_request_with_executor(
                    super::ExecutionRequest {
                        config: &config,
                        cache: &cache,
                        options: &live_options,
                        raw: &live_request,
                        header: live_header,
                        question: live_question,
                    },
                    &executor,
                    live_cancellation,
                ),
            )
        });
        assert!(
            cancelled_response.is_empty(),
            "cancelled request must not publish"
        );
        validate_response(&live_response).expect("uncancelled request response");
        assert_eq!(
            u16::from_be_bytes([live_response[0], live_response[1]]),
            live_id
        );
        let calls = calls.borrow();
        assert_eq!(calls.len(), 3, "two B legs must rendezvous before live A");
        assert!(calls[..2].iter().all(|(request_id, executable)| {
            *executable == b && (*request_id == cancelled_id || *request_id == live_id)
        }));
        assert_eq!(calls[2], (live_id, a));
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
