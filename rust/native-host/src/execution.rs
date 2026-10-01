use std::cell::{Cell, RefCell};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mosdns_dns_core::{
    FrameMode, QueryHeader, QuestionInfo, ResponseError, frame_response, inspect_response_header,
    observe_answer_addresses, observe_answer_records, observe_response_metadata,
    patch_response_id_ra, synthesize_response, validate_response,
};
use mosdns_sequence_core::{
    CancellationToken, ExecutableId, ExecutionControl, ExecutionError, ExecutionMachine,
    ExecutionState, ExecutorError, ExecutorOutcome, MachineStep,
    ResponseState as MachineResponseState, RootFuelHandle, RoutingState, SequenceId,
};
use mosdns_upstream_core::{ExchangeResponse, TransportCancellation, UpstreamError};

use crate::assembly::{ForwardAdapter, ForwardCatalog, HostOptions};
use crate::cache::{NativeCacheAdapter, PendingStore};
use crate::config::{CompiledConfig, FallbackConfig, NativeTarget, PreferenceConfig};
use crate::observer::{
    AnswerDetailsStatus, AttemptRegistrationHook, AuditAnswer, CacheStatus, ExecutionCheckpoint,
    FailureProvenance, LocalFailureKind, QueryTerminalOutcome, ResponseDetails, ResponseFlags,
    ResponseSource, ResponseState as ObservedResponseState, TerminalObservation,
    UpstreamAttemptLedger, UpstreamAttemptLedgerHandle, UpstreamAttemptList,
    UpstreamAttemptOutcome, UpstreamAttemptRecord, UpstreamDiagnosticAttempt,
    UpstreamDiagnosticBranch, UpstreamDiagnosticSelected, UpstreamDiagnostics,
    UpstreamMetricAttempt, UpstreamTransport,
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

    /// Executes one host-owned forward invocation. The default keeps the W1/W2
    /// single-owner seam unchanged; a catalog may override it to fan out the
    /// invocation's original entry subset without extending sequence-core's
    /// generic External payload.
    fn exchange_invocation<'a>(
        &'a self,
        executable: ExecutableId,
        query: &'a [u8],
        deadline: Instant,
        cancellation: TransportCancellation,
        _attempts: UpstreamAttemptLedgerHandle,
    ) -> Pin<Box<dyn Future<Output = Result<InvocationExchange, ExchangeError>> + 'a>> {
        Box::pin(async move {
            self.exchange(executable, query, deadline, cancellation)
                .await
                .map(|response| InvocationExchange {
                    response,
                    selected_entry: None,
                    selected_peer: None,
                })
        })
    }
}

pub(crate) struct InvocationExchange {
    pub response: ExchangeResponse,
    pub selected_entry: Option<usize>,
    pub selected_peer: Option<SocketAddr>,
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

struct BranchContext<'a, E: ExchangeExecutor + ?Sized> {
    config: &'a CompiledConfig,
    cache: &'a NativeCacheAdapter,
    executor: &'a E,
    raw: Rc<Vec<u8>>,
    header: QueryHeader,
    question: QuestionInfo,
    deadline: Instant,
    root_cancellation: TransportCancellation,
    branch_cancellation: TransportCancellation,
    trace: Option<Rc<RefCell<BranchTrace>>>,
    branch_metrics: Rc<RefCell<UpstreamAttemptList>>,
    cache_accessed: Rc<Cell<bool>>,
    allow_empty_response: bool,
    branch_id: Option<usize>,
}

impl<E: ExchangeExecutor + ?Sized> Clone for BranchContext<'_, E> {
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            cache: self.cache,
            executor: self.executor,
            raw: Rc::clone(&self.raw),
            header: self.header,
            question: self.question.clone(),
            deadline: self.deadline,
            root_cancellation: self.root_cancellation.clone(),
            branch_cancellation: self.branch_cancellation.clone(),
            trace: self.trace.clone(),
            branch_metrics: self.branch_metrics.clone(),
            cache_accessed: Rc::clone(&self.cache_accessed),
            allow_empty_response: self.allow_empty_response,
            branch_id: self.branch_id,
        }
    }
}

impl<E: ExchangeExecutor + ?Sized> BranchContext<'_, E> {
    fn raw(&self) -> &[u8] {
        self.raw.as_slice()
    }

    fn with_query(&self, raw: Vec<u8>, question: QuestionInfo) -> Self {
        Self {
            raw: Rc::new(raw),
            question,
            ..self.clone()
        }
    }

    fn with_transport_cancellation(&self, branch_cancellation: TransportCancellation) -> Self {
        Self {
            branch_cancellation,
            ..self.clone()
        }
    }

    fn with_branch(&self, branch_id: Option<usize>) -> Self {
        Self {
            branch_id,
            // A policy sibling is a new branch path. Its cache access budget
            // must not inherit a sibling's dynamic access, while all clones
            // of this new context continue to share the same path-local cell.
            cache_accessed: Rc::new(Cell::new(false)),
            ..self.clone()
        }
    }

    fn with_empty_response_allowed(&self) -> Self {
        Self {
            allow_empty_response: true,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug)]
struct BranchTrace {
    branches: Vec<UpstreamDiagnosticBranch>,
    attempts: Vec<UpstreamDiagnosticAttempt>,
    candidates: std::collections::BTreeMap<usize, UpstreamDiagnosticSelected>,
    selected: Option<UpstreamDiagnosticSelected>,
    pending_attempts: std::collections::BTreeMap<u64, usize>,
    next_pending_attempt: u64,
}

impl BranchTrace {
    fn new(qtype: u16) -> Self {
        Self {
            branches: vec![UpstreamDiagnosticBranch {
                id: 0,
                parent_id: None,
                role: "root".to_owned(),
                policy: None,
                qtype,
                decision: "completed".to_owned(),
            }],
            attempts: Vec::new(),
            candidates: std::collections::BTreeMap::new(),
            selected: None,
            pending_attempts: std::collections::BTreeMap::new(),
            next_pending_attempt: 1,
        }
    }

    fn add_branch(
        &mut self,
        parent_id: Option<usize>,
        role: &str,
        policy: &str,
        qtype: u16,
    ) -> usize {
        let id = self.branches.len();
        self.branches.push(UpstreamDiagnosticBranch {
            id,
            parent_id,
            role: role.to_owned(),
            policy: Some(policy.to_owned()),
            qtype,
            decision: "skipped".to_owned(),
        });
        id
    }

    fn mark(&mut self, id: Option<usize>, decision: &str) {
        if let Some(id) = id {
            if let Some(branch) = self.branches.get_mut(id) {
                branch.decision = decision.to_owned();
            }
        }
    }

    fn start(&mut self, id: Option<usize>) {
        if let Some(id) = id {
            if let Some(branch) = self.branches.get_mut(id) {
                branch.decision = "completed".to_owned();
            }
        }
    }

    fn record_attempt(
        &mut self,
        branch_id: Option<usize>,
        qtype: u16,
        entry: String,
        peer: Option<SocketAddr>,
        transport: Option<UpstreamTransport>,
        outcome: UpstreamAttemptOutcome,
    ) {
        self.attempts.push(UpstreamDiagnosticAttempt {
            ordinal: self.attempts.len(),
            branch_id,
            qtype: Some(qtype),
            entry,
            peer,
            transport,
            outcome,
        });
    }

    fn begin_attempt(&mut self, branch_id: Option<usize>, qtype: u16, entry: String) -> u64 {
        let token = self.next_pending_attempt;
        self.next_pending_attempt = self.next_pending_attempt.saturating_add(1);
        let index = self.attempts.len();
        self.attempts.push(UpstreamDiagnosticAttempt {
            ordinal: index,
            branch_id,
            qtype: Some(qtype),
            entry,
            peer: None,
            transport: None,
            outcome: UpstreamAttemptOutcome::Interrupted,
        });
        self.pending_attempts.insert(token, index);
        token
    }

    fn complete_attempt(
        &mut self,
        token: u64,
        entry: String,
        peer: Option<SocketAddr>,
        transport: Option<UpstreamTransport>,
        outcome: UpstreamAttemptOutcome,
    ) {
        let Some(index) = self.pending_attempts.remove(&token) else {
            return;
        };
        if let Some(attempt) = self.attempts.get_mut(index) {
            attempt.entry = entry;
            attempt.peer = peer;
            attempt.transport = transport;
            attempt.outcome = outcome;
        }
    }

    fn candidate(
        &mut self,
        branch_id: Option<usize>,
        entry: String,
        peer: Option<SocketAddr>,
        transport: Option<UpstreamTransport>,
    ) {
        if let Some(branch_id) = branch_id {
            if let (Some(peer), Some(transport)) = (peer, transport) {
                self.candidates.insert(
                    branch_id,
                    UpstreamDiagnosticSelected {
                        branch_id: Some(branch_id),
                        entry,
                        peer,
                        transport,
                    },
                );
            }
        }
    }

    fn select(&mut self, branch_id: Option<usize>, inherited_network: bool) {
        if let Some(selected) = branch_id.and_then(|id| self.candidates.get(&id).cloned()) {
            self.selected = Some(selected);
        } else if !inherited_network {
            self.selected = None;
        }
    }

    fn clear_selection(&mut self, terminal_decision: Option<&str>) {
        self.selected = None;
        if let Some(decision) = terminal_decision {
            for branch in &mut self.branches {
                if branch.decision == "selected" {
                    branch.decision = decision.to_owned();
                }
            }
        }
    }

    fn registration_hook(
        trace: Rc<RefCell<Self>>,
        config: &CompiledConfig,
        executable: ExecutableId,
        branch_id: Option<usize>,
        qtype: u16,
    ) -> AttemptRegistrationHook {
        let entry_names = config
            .forward_invocations
            .iter()
            .find(|invocation| invocation.executable == executable)
            .and_then(|invocation| config.forward_definitions.get(invocation.definition))
            .map(|definition| {
                definition
                    .entries
                    .iter()
                    .map(|entry| entry.identity.clone())
                    .collect::<Vec<_>>()
            });
        Rc::new(RefCell::new(move |entry_index| {
            let entry = entry_names
                .as_ref()
                .and_then(|entries| entries.get(entry_index).cloned())
                .unwrap_or_else(|| format!("executable:{:?}:{}", executable, entry_index));
            trace.borrow_mut().begin_attempt(branch_id, qtype, entry)
        }))
    }
}

struct BranchLedgerGuard<'a> {
    config: &'a CompiledConfig,
    trace: Option<Rc<RefCell<BranchTrace>>>,
    branch_metrics: Rc<RefCell<UpstreamAttemptList>>,
    branch_id: Option<usize>,
    qtype: u16,
    root_cancellation: TransportCancellation,
    executable: ExecutableId,
    ledger: UpstreamAttemptLedgerHandle,
    trace_token: Option<u64>,
    recorded: bool,
}

impl<'a> BranchLedgerGuard<'a> {
    fn new<E: ExchangeExecutor + ?Sized>(
        context: &BranchContext<'a, E>,
        executable: ExecutableId,
        ledger: UpstreamAttemptLedgerHandle,
        trace_token: Option<u64>,
    ) -> Self {
        Self {
            config: context.config,
            trace: context.trace.clone(),
            branch_metrics: context.branch_metrics.clone(),
            branch_id: context.branch_id,
            qtype: context.question.qtype,
            root_cancellation: context.root_cancellation.clone(),
            executable,
            ledger,
            trace_token,
            recorded: false,
        }
    }

    fn record(
        &mut self,
        selected_entry: Option<usize>,
        fallback_transport: Option<UpstreamTransport>,
        fallback_outcome: UpstreamAttemptOutcome,
    ) {
        record_branch_ledger_data(BranchLedgerRecord {
            config: self.config,
            trace: self.trace.as_ref(),
            branch_metrics: &self.branch_metrics,
            root_cancelled: self.root_cancellation.is_cancelled(),
            branch_id: self.branch_id,
            qtype: self.qtype,
            executable: self.executable,
            ledger: &self.ledger,
            trace_token: self.trace_token,
            selected_entry,
            fallback_transport,
            fallback_outcome,
        });
        self.recorded = true;
    }
}

impl Drop for BranchLedgerGuard<'_> {
    fn drop(&mut self) {
        if self.recorded {
            return;
        }
        let outcome = if self.root_cancellation.is_cancelled() {
            UpstreamAttemptOutcome::Canceled
        } else {
            UpstreamAttemptOutcome::Interrupted
        };
        self.record(None, None, outcome);
    }
}

fn new_branch_ledger<E: ExchangeExecutor + ?Sized>(
    context: &BranchContext<'_, E>,
    executable: ExecutableId,
) -> UpstreamAttemptLedgerHandle {
    let ledger = UpstreamAttemptLedger::default().into_shared();
    if let Some(trace) = &context.trace {
        let hook = BranchTrace::registration_hook(
            trace.clone(),
            context.config,
            executable,
            context.branch_id,
            context.question.qtype,
        );
        ledger.borrow_mut().set_registration_hook(hook);
    }
    ledger
}

fn trace_start(
    context: &BranchContext<'_, impl ExchangeExecutor + ?Sized>,
    branch_id: Option<usize>,
) {
    if let Some(trace) = &context.trace {
        trace.borrow_mut().start(branch_id);
    }
}

fn trace_outcome(
    context: &BranchContext<'_, impl ExchangeExecutor + ?Sized>,
    branch_id: Option<usize>,
    outcome: &BranchOutcome,
    selected: bool,
) {
    if let Some(trace) = &context.trace {
        let mut trace = trace.borrow_mut();
        trace.mark(
            branch_id,
            if selected {
                "selected"
            } else if outcome.is_success() {
                "completed"
            } else {
                "failed"
            },
        );
        if selected {
            trace.select(branch_id, outcome.source.is_some());
        }
    }
}

struct BranchOutcome {
    state: ExecutionState,
    source: Option<String>,
    error: Option<ExecutionError>,
    cache_accessed: bool,
}

impl BranchOutcome {
    fn success(state: ExecutionState, source: Option<String>) -> Self {
        Self {
            state,
            source,
            error: None,
            cache_accessed: false,
        }
    }

    fn failure(state: ExecutionState, error: ExecutionError) -> Self {
        Self {
            state,
            source: None,
            error: Some(error),
            cache_accessed: false,
        }
    }

    fn is_success(&self) -> bool {
        self.error.is_none() && !matches!(self.state.response, MachineResponseState::None)
    }
}

fn absorb_branch_cache<E: ExchangeExecutor + ?Sized>(
    context: &BranchContext<'_, E>,
    outcome: &BranchOutcome,
) {
    if outcome.error.is_none() && outcome.cache_accessed {
        context.cache_accessed.set(true);
    }
}

fn commit_branch_winner<E: ExchangeExecutor + ?Sized>(
    outcome: BranchOutcome,
    context: &BranchContext<'_, E>,
    fallback_state: &ExecutionState,
) -> BranchOutcome {
    if context.root_cancellation.is_cancelled() {
        return BranchOutcome::failure(fallback_state.clone(), ExecutionError::Cancelled);
    }
    if Instant::now() >= context.deadline {
        return BranchOutcome::failure(fallback_state.clone(), ExecutionError::BudgetExceeded);
    }
    absorb_branch_cache(context, &outcome);
    outcome
}

fn ensure_branch_alive<E: ExchangeExecutor + ?Sized>(
    machine: &ExecutionMachine<'_>,
    context: &BranchContext<'_, E>,
) -> Result<(), ExecutionError> {
    if context.root_cancellation.is_cancelled()
        || context.branch_cancellation.is_cancelled()
        || machine.control().is_cancelled()
    {
        return Err(ExecutionError::Cancelled);
    }
    if Instant::now() >= context.deadline {
        return Err(ExecutionError::BudgetExceeded);
    }
    Ok(())
}

fn consume_external_target<E: ExchangeExecutor + ?Sized>(
    machine: &mut ExecutionMachine<'_>,
    context: &BranchContext<'_, E>,
) -> Result<(), ExecutionError> {
    ensure_branch_alive(machine, context)?;
    machine.control_mut().try_consume()
}

/// Facts produced by the canonical execution path for one parsed query.
/// Listener-owned framing and terminal transport outcome are added later.
#[derive(Clone, Debug)]
pub(crate) struct ExecutionResult {
    pub response_wire: Vec<u8>,
    pub response: ObservedResponseState,
    pub response_details: ResponseDetails,
    pub cache_status: CacheStatus,
    pub final_sequence: Option<String>,
    pub matched_group: Option<String>,
    pub domain_set: Option<String>,
    pub effective_tag: Option<String>,
    pub matched_rule_source: Option<String>,
    pub final_upstream: Option<String>,
    pub upstream_targets: Option<String>,
    pub selected_upstream: Option<String>,
    pub upstream_attempts: UpstreamAttemptList,
    pub upstream_diagnostics: Option<UpstreamDiagnostics>,
    pub failure_provenance: Option<FailureProvenance>,
}

struct ExecutionFacts<'a> {
    capture_audit_details: bool,
    cache_status: CacheStatus,
    response_source: Option<ResponseSource>,
    upstream_attempts: UpstreamAttemptList,
    selected_peer: Option<SocketAddr>,
    upstream_diagnostics: Option<UpstreamDiagnostics>,
    policy_trace: Option<Rc<RefCell<BranchTrace>>>,
    branch_metrics: Rc<RefCell<UpstreamAttemptList>>,
    failure_provenance: Option<FailureProvenance>,
    policy_failure: Option<ExecutionError>,
    /// The real named-sequence execution position, materialized only when
    /// detailed audit capture is enabled.
    final_sequence: Option<String>,
    routing: RoutingState,
    /// Routing facts committed by the response that ultimately owns the wire.
    /// A parent sequence may replace a child response without touching the
    /// child's routing fields, so the live machine routing state is not by
    /// itself a safe final-audit source.
    response_routing: Option<RoutingState>,
    last_response_generation: u64,
    routing_changed_since_response: bool,
    current_origin: Option<SequenceId>,
    routing_origin: Option<SequenceId>,
    config: &'a CompiledConfig,
    checkpoint: &'a mut ExecutionCheckpoint,
    cancellation: TransportCancellation,
    in_flight_executable: Option<ExecutableId>,
    completed: bool,
}

impl ExecutionFacts<'_> {
    fn enable_policy_trace(&mut self, qtype: u16) -> Option<Rc<RefCell<BranchTrace>>> {
        if !self.capture_audit_details {
            return None;
        }
        if self.policy_trace.is_none() {
            self.policy_trace = Some(Rc::new(RefCell::new(BranchTrace::new(qtype))));
            if let Some(diagnostics) = self.upstream_diagnostics.as_mut() {
                for attempt in &mut diagnostics.attempts {
                    attempt.branch_id = Some(0);
                    attempt.qtype = Some(qtype);
                }
                if let Some(selected) = diagnostics.selected.as_mut() {
                    selected.branch_id = Some(0);
                }
            }
        }
        self.policy_trace.clone()
    }

    fn install_root_attempt_trace(&mut self, executable: ExecutableId, qtype: u16) {
        let Some(trace) = &self.policy_trace else {
            return;
        };
        let hook =
            BranchTrace::registration_hook(trace.clone(), self.config, executable, Some(0), qtype);
        self.checkpoint
            .attempt_ledger_handle()
            .borrow_mut()
            .set_registration_hook(hook);
    }

    fn finalize_policy_trace(&mut self) {
        let Some(trace) = self.policy_trace.as_ref() else {
            return;
        };
        let trace = trace.borrow();
        let Some(diagnostics) = self.upstream_diagnostics.as_mut() else {
            return;
        };
        diagnostics.schema_version = 2;
        diagnostics.branches = trace.branches.clone();
        diagnostics.attempts.extend(trace.attempts.clone());
        for (ordinal, attempt) in diagnostics.attempts.iter_mut().enumerate() {
            attempt.ordinal = ordinal;
        }
        if trace.selected.is_some() {
            diagnostics.selected = trace.selected.clone();
        }
    }

    fn absorb_branch_metrics(&mut self) {
        let branch_metrics = std::mem::take(&mut *self.branch_metrics.borrow_mut());
        for attempt in branch_metrics.metric_attempts() {
            self.upstream_attempts.push_metric(*attempt);
        }
    }

    fn set_response_source(&mut self, source: ResponseSource) {
        if !matches!(&source, ResponseSource::Upstream(_)) {
            self.selected_peer = None;
            if self.capture_audit_details {
                if let Some(diagnostics) = &mut self.upstream_diagnostics {
                    diagnostics.selected = None;
                }
                if let Some(trace) = &self.policy_trace {
                    trace.borrow_mut().clear_selection(None);
                }
            }
        } else if self.capture_audit_details {
            if let Some(peer) = self.policy_trace.as_ref().and_then(|trace| {
                trace
                    .borrow()
                    .selected
                    .as_ref()
                    .map(|selected| selected.peer)
            }) {
                self.selected_peer = Some(peer);
            }
        }
        if self.capture_audit_details {
            self.response_source = Some(source);
        }
    }

    /// Records the actual executed named sequence. A synthetic inline scope
    /// never reaches here, so the position is always a real configured name.
    fn note_origin(&mut self, origin: Option<SequenceId>) {
        self.current_origin = origin;
        if !self.capture_audit_details {
            return;
        }
        if let Some(sequence) = origin.and_then(|id| self.config.program.sequence(id)) {
            if !sequence.synthetic {
                self.final_sequence = Some(sequence.name.clone());
            }
        }
    }

    fn note_routing(&mut self, state: &ExecutionState) {
        if self.routing != state.routing {
            self.routing = state.routing.clone();
            self.routing_changed_since_response = true;
            self.routing_origin = self.current_origin;
        }
    }

    fn note_response(&mut self, state: &ExecutionState) {
        if self.last_response_generation == state.response_generation() {
            return;
        }
        self.response_routing = match &state.response {
            MachineResponseState::None => None,
            MachineResponseState::Raw(_) | MachineResponseState::Synthesized(_) => {
                if matches!(&state.response, MachineResponseState::Synthesized(_)) {
                    self.response_source = Some(ResponseSource::Local);
                }
                Some(
                    if self.routing_changed_since_response
                        && self.routing_origin == self.current_origin
                    {
                        self.routing.clone()
                    } else {
                        RoutingState::default()
                    },
                )
            }
        };
        self.last_response_generation = state.response_generation();
        self.routing_changed_since_response = false;
    }

    fn set_failure_provenance(&mut self, provenance: FailureProvenance) {
        if self.capture_audit_details {
            self.failure_provenance = Some(provenance);
        }
    }

    fn record_policy_failure(&mut self, error: ExecutionError) {
        self.policy_failure = Some(error);
        self.set_failure_provenance(FailureProvenance::LocalFailure(
            LocalFailureKind::NoUsableUpstreamResponse,
        ));
    }

    fn clear_policy_failure(&mut self) {
        self.policy_failure = None;
        if matches!(
            self.failure_provenance,
            Some(FailureProvenance::LocalFailure(
                LocalFailureKind::NoUsableUpstreamResponse
            ))
        ) {
            self.failure_provenance = None;
        }
    }

    fn record_upstream_response(&mut self, upstream: String) {
        if self.capture_audit_details {
            self.upstream_attempts.push(UpstreamAttemptRecord {
                upstream: upstream.clone(),
                outcome: UpstreamAttemptOutcome::Response,
            });
            self.response_source = Some(ResponseSource::Upstream(upstream));
        } else {
            // Compatibility fallback for legacy test/embedding executors that
            // do not expose the native entry ledger. Native catalog calls use
            // record_invocation_attempt and stay ID-only when audit is off.
            self.upstream_attempts.push(UpstreamAttemptRecord {
                upstream,
                outcome: UpstreamAttemptOutcome::Response,
            });
        }
    }

    fn record_invocation_attempt(
        &mut self,
        executable: ExecutableId,
        entry_index: usize,
        upstream: Option<String>,
        peer: Option<SocketAddr>,
        transport: Option<UpstreamTransport>,
        outcome: UpstreamAttemptOutcome,
    ) {
        self.upstream_attempts.push_metric(UpstreamMetricAttempt {
            executable,
            entry_index,
            outcome,
        });
        if let Some(upstream) = upstream {
            if self.capture_audit_details {
                self.upstream_attempts.push(UpstreamAttemptRecord {
                    upstream: upstream.clone(),
                    outcome,
                });
                if self.policy_trace.is_none()
                    && let Some(diagnostics) = &mut self.upstream_diagnostics
                {
                    diagnostics.attempts.push(UpstreamDiagnosticAttempt {
                        ordinal: diagnostics.attempts.len(),
                        branch_id: None,
                        qtype: None,
                        entry: upstream,
                        peer,
                        transport,
                        outcome,
                    });
                }
            }
        }
    }

    fn record_invocation_failure(&mut self, upstream: &str, outcome: UpstreamAttemptOutcome) {
        if self.capture_audit_details {
            self.failure_provenance = Some(if outcome == UpstreamAttemptOutcome::TimedOut {
                FailureProvenance::UpstreamTimeout {
                    upstream: upstream.to_owned(),
                }
            } else {
                FailureProvenance::UpstreamFailure {
                    upstream: upstream.to_owned(),
                }
            });
            self.response_source = Some(ResponseSource::Local);
        }
    }

    fn select_invocation_attempt(
        &mut self,
        upstream: String,
        peer: Option<SocketAddr>,
        transport: Option<UpstreamTransport>,
    ) {
        self.selected_peer = peer;
        if self.policy_trace.is_some() {
            return;
        }
        if let (Some(peer), Some(transport), Some(diagnostics)) =
            (peer, transport, &mut self.upstream_diagnostics)
        {
            diagnostics.selected = Some(UpstreamDiagnosticSelected {
                branch_id: None,
                entry: upstream,
                peer,
                transport,
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
        }
        self.upstream_attempts
            .push(UpstreamAttemptRecord { upstream, outcome });
    }
}

impl Drop for ExecutionFacts<'_> {
    fn drop(&mut self) {
        // Branch entry work may still be live when the root future is dropped.
        // Absorb the shared collector before taking the terminal checkpoint so
        // forced-drop evidence is not lost with the branch driver.
        self.absorb_branch_metrics();
        if self.completed {
            return;
        }
        if let Some(trace) = &self.policy_trace {
            trace.borrow_mut().selected = None;
        }
        let ledger = self.checkpoint.attempt_ledger_snapshot();
        let mut ledger_had_entries = false;
        if let Some(executable) = self.in_flight_executable {
            for slot in ledger.slots() {
                ledger_had_entries = true;
                let upstream = self
                    .capture_audit_details
                    .then(|| invocation_identity(self.config, executable, slot.entry_index))
                    .flatten();
                let outcome = slot.outcome.unwrap_or_else(|| {
                    if self.cancellation.is_cancelled() {
                        UpstreamAttemptOutcome::Canceled
                    } else {
                        UpstreamAttemptOutcome::Interrupted
                    }
                });
                self.upstream_attempts.push_metric(UpstreamMetricAttempt {
                    executable,
                    entry_index: slot.entry_index,
                    outcome,
                });
                if self.capture_audit_details {
                    let trace_entry = upstream.clone().unwrap_or_else(|| {
                        format!("executable:{:?}:{}", executable, slot.entry_index)
                    });
                    if let Some(trace) = &self.policy_trace {
                        let mut trace = trace.borrow_mut();
                        if let Some(trace_slot) = slot.trace_slot {
                            trace.complete_attempt(
                                trace_slot,
                                trace_entry,
                                slot.peer,
                                slot.transport,
                                outcome,
                            );
                        } else {
                            let qtype = trace
                                .branches
                                .first()
                                .map(|branch| branch.qtype)
                                .unwrap_or_default();
                            trace.record_attempt(
                                Some(0),
                                qtype,
                                trace_entry,
                                slot.peer,
                                slot.transport,
                                outcome,
                            );
                        }
                    }
                    if let Some(upstream) = upstream {
                        self.upstream_attempts.push(UpstreamAttemptRecord {
                            upstream: upstream.clone(),
                            outcome,
                        });
                        if self.policy_trace.is_none()
                            && let Some(diagnostics) = &mut self.upstream_diagnostics
                        {
                            diagnostics.attempts.push(UpstreamDiagnosticAttempt {
                                ordinal: diagnostics.attempts.len(),
                                branch_id: None,
                                qtype: None,
                                entry: upstream,
                                peer: slot.peer,
                                transport: slot.transport,
                                outcome,
                            });
                        }
                    }
                    if let Some(diagnostics) = &mut self.upstream_diagnostics {
                        diagnostics.selected = None;
                    }
                }
            }
        }
        self.finalize_policy_trace();
        self.checkpoint.capture_partial(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                // A response observed by an earlier leg is not necessarily
                // the final response. Only result_from_wire can establish
                // final response provenance, so an unfinished execution
                // must not publish an intermediate W3 answer here.
                response: ObservedResponseState::NoResponse,
                response_details: ResponseDetails::no_response(),
                cache_status: self.cache_status,
                final_sequence: self
                    .routing
                    .final_sequence
                    .clone()
                    .or_else(|| self.final_sequence.clone()),
                matched_group: self.routing.matched_group.clone(),
                domain_set: self.routing.domain_set.clone(),
                effective_tag: self.routing.effective_tag.clone(),
                matched_rule_source: self.routing.matched_rule_source.clone(),
                final_upstream: self.routing.final_upstream.clone(),
                upstream_targets: self.routing.final_upstream_targets.clone(),
                selected_upstream: None,
                upstream_attempts: self.upstream_attempts.clone(),
                upstream_diagnostics: self.upstream_diagnostics.clone(),
                failure_provenance: self.failure_provenance.clone(),
                elapsed: std::time::Duration::ZERO,
            },
            self.in_flight_executable
                .filter(|_| !ledger_had_entries)
                .filter(|_| self.capture_audit_details)
                .and_then(|executable| upstream_identity(self.config, executable)),
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
        upstream_attempts: UpstreamAttemptList::with_capacity_hint(if multi_forward {
            config.forwards.len()
        } else {
            0
        }),
        selected_peer: None,
        upstream_diagnostics: capture_audit_details.then(|| UpstreamDiagnostics {
            schema_version: 1,
            branches: Vec::new(),
            selected: None,
            attempts: Vec::new(),
        }),
        policy_trace: None,
        branch_metrics: Rc::new(RefCell::new(UpstreamAttemptList::with_capacity_hint(
            if multi_forward {
                config.forwards.len()
            } else {
                0
            },
        ))),
        failure_provenance: None,
        policy_failure: None,
        // No entry-tag backfill: the field carries the real executing position
        // recorded during execution, or nothing when none was observed.
        final_sequence: None,
        routing: RoutingState::default(),
        response_routing: None,
        last_response_generation: 0,
        routing_changed_since_response: false,
        current_origin: None,
        routing_origin: None,
        config,
        checkpoint,
        cancellation: request_shutdown.clone(),
        in_flight_executable: None,
        completed: false,
    };
    let state = ExecutionState::new(header, question.clone());
    // Every external leg shares this one request-owned absolute budget.
    let request_deadline = options
        .admission_deadline
        .unwrap_or_else(|| Instant::now() + options.request_deadline);
    let root_control = ExecutionControl::with_shared_budget(
        RootFuelHandle::new(DEFAULT_FUEL),
        CancellationToken::new(),
    );
    let mut machine = match config.new_machine(state, root_control) {
        Ok(machine) => machine,
        Err(_) => {
            facts.cache_status = CacheStatus::NotApplicable;
            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution,
            ));
            return result_from_wire(protocol_error(&header, &question, SERVFAIL), facts);
        }
    };

    // One request-owned cache access and publication token. A second dynamic
    // dispatch fails closed even if the first access hit or its miss was
    // already published at a child-scope boundary.
    let mut cache_accessed = false;
    let mut pending_store: Option<PendingStore> = None;
    let mut upstream_response = false;
    let mut attempted = false;
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
        facts.note_origin(machine.last_origin());
        facts.note_routing(machine.state());
        facts.note_response(machine.state());
        match step {
            MachineStep::Complete(_) => {
                // Publication is owned by the cache's successor boundary, so a
                // token still held here is an unpublished miss and must drop.
                return result_from_state(&machine, &header, &question, facts);
            }
            MachineStep::ScopeComplete(completion) => {
                // The cache's enclosing scope finished naturally. This is the
                // successor completion point: the response now in state is
                // what that successor produced, before the caller's remaining
                // rules can rewrite it.
                facts.note_response(machine.state());
                publish_successor(
                    &mut pending_store,
                    &machine,
                    request_shutdown.is_cancelled(),
                    publication_deadline,
                );
                // A parent may replace the child response without adding any
                // routing fields of its own. Keep the child facts in the
                // response-owned snapshot, but start the parent with an
                // empty audit-routing candidate so stale child provenance
                // cannot become the parent's final decision.
                machine.state_mut().routing = RoutingState::default();
                facts.routing = RoutingState::default();
                facts.routing_changed_since_response = false;
                step = match machine.resume_scope_completion(completion.executable()) {
                    Ok(step) => step,
                    Err(_) => {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        return result_from_state(&machine, &header, &question, facts);
                    }
                };
            }
            MachineStep::Dispatch(dispatch) => {
                if let Some(policy) = config
                    .fallbacks
                    .iter()
                    .find(|policy| policy.executable == dispatch.executable())
                {
                    let trace = facts.enable_policy_trace(question.qtype);
                    let context = BranchContext {
                        config,
                        cache,
                        executor,
                        raw: Rc::new(raw.to_vec()),
                        header,
                        question: question.clone(),
                        deadline: request_deadline,
                        root_cancellation: request_shutdown.clone(),
                        branch_cancellation: request_shutdown.child_token(),
                        trace,
                        branch_metrics: facts.branch_metrics.clone(),
                        cache_accessed: Rc::new(Cell::new(false)),
                        allow_empty_response: false,
                        branch_id: Some(0),
                    };
                    let successor = match machine.fork_successor(CancellationToken::new()) {
                        Ok(successor) => successor,
                        Err(_) => {
                            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                LocalFailureKind::InternalExecution,
                            ));
                            set_servfail(&mut machine);
                            facts.set_response_source(ResponseSource::Local);
                            return result_from_state(&machine, &header, &question, facts);
                        }
                    };
                    let inherited_source = match facts.response_source.as_ref() {
                        Some(ResponseSource::Upstream(source)) => Some(source.clone()),
                        _ => None,
                    };
                    let outcome =
                        run_fallback(policy.clone(), successor, context.clone(), inherited_source)
                            .await;
                    absorb_branch_cache(&context, &outcome);
                    cache_accessed |= context.cache_accessed.get();
                    if let Some(error) = outcome.error {
                        facts.record_policy_failure(error.clone());
                        // The captured successor has already been driven by
                        // the branch. Commit its state, then return the typed
                        // policy failure through the pending dispatch. The
                        // sequence core, rather than a synthetic Accept,
                        // owns the caller/try failure boundary.
                        *machine.state_mut() = outcome.state;
                        let core_error = match policy_failure_for_core(&error) {
                            Ok(core_error) => core_error,
                            Err(terminal) => {
                                return terminal_policy_failure(terminal, facts);
                            }
                        };
                        match machine.resume(dispatch.executable(), Err(core_error)) {
                            Ok(next) => {
                                facts.clear_policy_failure();
                                step = next;
                                continue;
                            }
                            Err(_) => {
                                return result_from_state(&machine, &header, &question, facts);
                            }
                        }
                    }
                    if let Some(source) = outcome.source.clone() {
                        facts.set_response_source(ResponseSource::Upstream(source));
                    } else {
                        facts.set_response_source(ResponseSource::Local);
                    }
                    *machine.state_mut() = outcome.state;
                    step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
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
                if let Some(preference) = config
                    .preferences
                    .iter()
                    .find(|preference| preference.executable == dispatch.executable())
                {
                    let trace = facts.enable_policy_trace(question.qtype);
                    let context = BranchContext {
                        config,
                        cache,
                        executor,
                        raw: Rc::new(raw.to_vec()),
                        header,
                        question: question.clone(),
                        deadline: request_deadline,
                        root_cancellation: request_shutdown.clone(),
                        branch_cancellation: request_shutdown.child_token(),
                        trace,
                        branch_metrics: facts.branch_metrics.clone(),
                        cache_accessed: Rc::new(Cell::new(false)),
                        allow_empty_response: false,
                        branch_id: Some(0),
                    };
                    let successor = match machine.fork_successor(CancellationToken::new()) {
                        Ok(successor) => successor,
                        Err(_) => {
                            facts.set_failure_provenance(FailureProvenance::LocalFailure(
                                LocalFailureKind::InternalExecution,
                            ));
                            set_servfail(&mut machine);
                            facts.set_response_source(ResponseSource::Local);
                            return result_from_state(&machine, &header, &question, facts);
                        }
                    };
                    let inherited_source = match facts.response_source.as_ref() {
                        Some(ResponseSource::Upstream(source)) => Some(source.clone()),
                        _ => None,
                    };
                    let outcome = run_preference(
                        preference.clone(),
                        successor,
                        context.clone(),
                        inherited_source,
                    )
                    .await;
                    absorb_branch_cache(&context, &outcome);
                    cache_accessed |= context.cache_accessed.get();
                    if let Some(error) = outcome.error {
                        facts.record_policy_failure(error.clone());
                        *machine.state_mut() = outcome.state;
                        let core_error = match policy_failure_for_core(&error) {
                            Ok(core_error) => core_error,
                            Err(terminal) => {
                                return terminal_policy_failure(terminal, facts);
                            }
                        };
                        match machine.resume(dispatch.executable(), Err(core_error)) {
                            Ok(next) => {
                                facts.clear_policy_failure();
                                step = next;
                                continue;
                            }
                            Err(_) => {
                                return result_from_state(&machine, &header, &question, facts);
                            }
                        }
                    }
                    if let Some(source) = outcome.source.clone() {
                        facts.set_response_source(ResponseSource::Upstream(source));
                    } else {
                        facts.set_response_source(ResponseSource::Local);
                    }
                    *machine.state_mut() = outcome.state;
                    step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
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
                if config
                    .cache
                    .as_ref()
                    .is_some_and(|cache_config| cache_config.executable == dispatch.executable())
                {
                    if cache_accessed {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        return result_from_state(&machine, &header, &question, facts);
                    }
                    cache_accessed = true;
                    let lookup = cache.lookup(raw).ok().flatten();
                    if let Some(wire) = lookup {
                        facts.cache_status = CacheStatus::Hit;
                        facts.set_response_source(ResponseSource::Cache);
                        facts.failure_provenance = None;
                        machine.state_mut().set_raw_response(wire);
                        // A hit completes the successor chain that contains the
                        // cache; the caller's later rules still run.
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
                    // The miss is published when the cache's own successor
                    // chain completes, not by the query's final state.
                    if machine
                        .watch_enclosing_scope(dispatch.executable())
                        .is_err()
                    {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        return result_from_state(&machine, &header, &question, facts);
                    }
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
                // A later leg must not start on a budget that has already
                // expired. The first leg always runs so a single-forward graph
                // keeps its previous response behavior; from the second leg on,
                // the shared deadline decides, independent of forward count.
                if (upstream_response || attempted) && Instant::now() >= request_deadline {
                    set_servfail(&mut machine);
                    facts.set_response_source(ResponseSource::Local);
                    facts.set_failure_provenance(FailureProvenance::LocalFailure(
                        LocalFailureKind::NoUsableUpstreamResponse,
                    ));
                    return result_from_state(&machine, &header, &question, facts);
                }
                publication_deadline = Some(request_deadline);
                attempted = true;
                facts.in_flight_executable = Some(dispatch.executable());
                facts.checkpoint.begin_attempt_ledger();
                facts.install_root_attempt_trace(dispatch.executable(), question.qtype);
                let exchange = executor
                    .exchange_invocation(
                        dispatch.executable(),
                        raw,
                        request_deadline,
                        request_shutdown.clone(),
                        facts.checkpoint.attempt_ledger_handle(),
                    )
                    .await;
                let attempt_ledger = facts.checkpoint.take_attempt_ledger();
                let in_flight_executable = facts.in_flight_executable.take();
                if request_shutdown.is_cancelled() {
                    record_invocation_ledger(
                        config,
                        in_flight_executable,
                        &attempt_ledger,
                        None,
                        question.qtype,
                        &mut facts,
                    );
                    return canceled_execution(facts);
                }
                // One forward-count-independent policy decides whether a leg's
                // response is usable and what a rejection means.
                match exchange {
                    Ok(batch) => {
                        match qualify_response(batch.response.wire(), header.id, &question) {
                            Some(wire) => {
                                if let Some(selected_entry) = batch.selected_entry {
                                    facts.selected_peer = batch.selected_peer;
                                    record_invocation_ledger(
                                        config,
                                        Some(dispatch.executable()),
                                        &attempt_ledger,
                                        Some(selected_entry),
                                        question.qtype,
                                        &mut facts,
                                    );
                                } else if let Some(upstream) = in_flight_executable
                                    .and_then(|executable| upstream_identity(config, executable))
                                {
                                    facts.record_upstream_response(upstream);
                                }
                                facts.failure_provenance = None;
                                upstream_response = true;
                                machine.state_mut().set_raw_response(wire);
                            }
                            None => {
                                if !attempt_ledger.slots().is_empty() {
                                    record_invocation_ledger(
                                        config,
                                        Some(dispatch.executable()),
                                        &attempt_ledger,
                                        None,
                                        question.qtype,
                                        &mut facts,
                                    );
                                } else if let Some(upstream) = in_flight_executable
                                    .and_then(|executable| upstream_identity(config, executable))
                                {
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
                                set_servfail(&mut machine);
                                facts.set_response_source(ResponseSource::Local);
                                return result_from_state(&machine, &header, &question, facts);
                            }
                        }
                    }
                    Err(ExchangeError::UnknownExecutable(_)) => {
                        facts.set_failure_provenance(FailureProvenance::LocalFailure(
                            LocalFailureKind::InternalExecution,
                        ));
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        return result_from_state(&machine, &header, &question, facts);
                    }
                    Err(error @ ExchangeError::Upstream(_)) => {
                        if !attempt_ledger.slots().is_empty() {
                            record_invocation_ledger(
                                config,
                                in_flight_executable,
                                &attempt_ledger,
                                None,
                                question.qtype,
                                &mut facts,
                            );
                        } else if let Some(upstream) = in_flight_executable
                            .and_then(|executable| upstream_identity(config, executable))
                        {
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
                        set_servfail(&mut machine);
                        facts.set_response_source(ResponseSource::Local);
                        return result_from_state(&machine, &header, &question, facts);
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

/// Publishes the response that the cache's own successor chain produced.
///
/// The token is consumed only at this boundary, so the caller's later rules
/// cannot pollute an already-completed successor result. Cancellation, an
/// expired publication budget, or a response the cache declines to store
/// leaves the token to drop without publishing; none of those is a host
/// failure, so no failure provenance is recorded here.
fn publish_successor(
    pending_store: &mut Option<PendingStore>,
    machine: &ExecutionMachine<'_>,
    canceled: bool,
    publication_deadline: Option<Instant>,
) {
    if canceled {
        return;
    }
    if publication_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return;
    }
    let Some(token) = pending_store.take() else {
        return;
    };
    let _ = token.publish(match &machine.state().response {
        MachineResponseState::Raw(wire) => wire.as_bytes(),
        MachineResponseState::None | MachineResponseState::Synthesized(_) => &[],
    });
}

fn drive_branch<'a, E: ExchangeExecutor + ?Sized>(
    machine: ExecutionMachine<'a>,
    context: BranchContext<'a, E>,
    source: Option<String>,
) -> Pin<Box<dyn Future<Output = BranchOutcome> + 'a>> {
    let cache_accessed = Rc::clone(&context.cache_accessed);
    Box::pin(async move {
        let mut outcome = drive_branch_inner(machine, context, source).await;
        outcome.cache_accessed = cache_accessed.get();
        outcome
    })
}

async fn drive_branch_inner<'a, E: ExchangeExecutor + ?Sized>(
    mut machine: ExecutionMachine<'a>,
    context: BranchContext<'a, E>,
    mut source: Option<String>,
) -> BranchOutcome {
    if let Err(error) = ensure_branch_alive(&machine, &context) {
        return BranchOutcome::failure(machine.state().clone(), error);
    }
    let mut pending_store = None;
    let mut publication_deadline = None;
    let mut step = match machine.step() {
        Ok(step) => step,
        Err(error) => return BranchOutcome::failure(machine.state().clone(), error),
    };
    loop {
        if let Err(error) = ensure_branch_alive(&machine, &context) {
            return BranchOutcome::failure(machine.state().clone(), error);
        }
        match step {
            MachineStep::Complete(_) => {
                if matches!(machine.state().response, MachineResponseState::None) {
                    if context.allow_empty_response {
                        return BranchOutcome::success(machine.state().clone(), source);
                    }
                    return BranchOutcome::failure(
                        machine.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "branch completed without a response",
                        )),
                    );
                }
                return BranchOutcome::success(machine.state().clone(), source);
            }
            MachineStep::ScopeComplete(completion) => {
                publish_successor(
                    &mut pending_store,
                    &machine,
                    context.root_cancellation.is_cancelled(),
                    publication_deadline,
                );
                step = match machine.resume_scope_completion(completion.executable()) {
                    Ok(step) => step,
                    Err(error) => {
                        return BranchOutcome::failure(machine.state().clone(), error);
                    }
                };
            }
            MachineStep::Dispatch(dispatch) => {
                if let Some(policy) = context
                    .config
                    .fallbacks
                    .iter()
                    .find(|policy| policy.executable == dispatch.executable())
                {
                    let successor = match machine.fork_successor(CancellationToken::new()) {
                        Ok(successor) => successor,
                        Err(error) => {
                            return BranchOutcome::failure(machine.state().clone(), error);
                        }
                    };
                    let outcome =
                        run_fallback(policy.clone(), successor, context.clone(), source.clone())
                            .await;
                    absorb_branch_cache(&context, &outcome);
                    if let Some(error) = outcome.error {
                        *machine.state_mut() = outcome.state;
                        let core_error = match policy_failure_for_core(&error) {
                            Ok(core_error) => core_error,
                            Err(error) => {
                                return BranchOutcome::failure(machine.state().clone(), error);
                            }
                        };
                        match machine.resume(dispatch.executable(), Err(core_error)) {
                            Ok(next) => {
                                step = next;
                                continue;
                            }
                            Err(error) => {
                                return BranchOutcome::failure(machine.state().clone(), error);
                            }
                        }
                    }
                    source = outcome.source;
                    *machine.state_mut() = outcome.state;
                    step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
                    {
                        Ok(step) => step,
                        Err(error) => {
                            return BranchOutcome::failure(machine.state().clone(), error);
                        }
                    };
                    continue;
                }
                if let Some(preference) = context
                    .config
                    .preferences
                    .iter()
                    .find(|preference| preference.executable == dispatch.executable())
                {
                    let successor = match machine.fork_successor(CancellationToken::new()) {
                        Ok(successor) => successor,
                        Err(error) => {
                            return BranchOutcome::failure(machine.state().clone(), error);
                        }
                    };
                    let outcome = run_preference(
                        preference.clone(),
                        successor,
                        context.clone(),
                        source.clone(),
                    )
                    .await;
                    absorb_branch_cache(&context, &outcome);
                    if let Some(error) = outcome.error {
                        *machine.state_mut() = outcome.state;
                        let core_error = match policy_failure_for_core(&error) {
                            Ok(core_error) => core_error,
                            Err(error) => {
                                return BranchOutcome::failure(machine.state().clone(), error);
                            }
                        };
                        match machine.resume(dispatch.executable(), Err(core_error)) {
                            Ok(next) => {
                                step = next;
                                continue;
                            }
                            Err(error) => {
                                return BranchOutcome::failure(machine.state().clone(), error);
                            }
                        }
                    }
                    source = outcome.source;
                    *machine.state_mut() = outcome.state;
                    step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
                    {
                        Ok(step) => step,
                        Err(error) => {
                            return BranchOutcome::failure(machine.state().clone(), error);
                        }
                    };
                    continue;
                }
                if context
                    .config
                    .cache
                    .as_ref()
                    .is_some_and(|cache| cache.executable == dispatch.executable())
                {
                    if context.cache_accessed.get() {
                        return BranchOutcome::failure(
                            machine.state().clone(),
                            ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                                "duplicate cache access in one branch",
                            )),
                        );
                    }
                    context.cache_accessed.set(true);
                    if let Ok(Some(wire)) = context.cache.lookup(context.raw()) {
                        machine.state_mut().set_raw_response(wire);
                        source = None;
                        step = match machine
                            .resume(dispatch.executable(), Ok(ExecutorOutcome::Accept))
                        {
                            Ok(step) => step,
                            Err(error) => {
                                return BranchOutcome::failure(machine.state().clone(), error);
                            }
                        };
                        continue;
                    }
                    pending_store = context.cache.begin_store(context.raw()).ok().flatten();
                    publication_deadline = Some(context.deadline);
                    if let Err(error) = machine.watch_enclosing_scope(dispatch.executable()) {
                        return BranchOutcome::failure(machine.state().clone(), error);
                    }
                    step = match machine
                        .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
                    {
                        Ok(step) => step,
                        Err(error) => {
                            return BranchOutcome::failure(machine.state().clone(), error);
                        }
                    };
                    continue;
                }
                if context.root_cancellation.is_cancelled() {
                    return BranchOutcome::failure(
                        machine.state().clone(),
                        ExecutionError::Cancelled,
                    );
                }
                if Instant::now() >= context.deadline {
                    return BranchOutcome::failure(
                        machine.state().clone(),
                        ExecutionError::BudgetExceeded,
                    );
                }
                let branch_cancellation = context.branch_cancellation.child_token();
                let ledger = new_branch_ledger(&context, dispatch.executable());
                let mut ledger_guard =
                    BranchLedgerGuard::new(&context, dispatch.executable(), ledger.clone(), None);
                let exchange = context
                    .executor
                    .exchange_invocation(
                        dispatch.executable(),
                        context.raw(),
                        context.deadline,
                        branch_cancellation,
                        ledger.clone(),
                    )
                    .await;
                let selected_entry = exchange
                    .as_ref()
                    .ok()
                    .and_then(|batch| batch.selected_entry);
                let response_transport = exchange
                    .as_ref()
                    .ok()
                    .map(|batch| branch_transport(batch.response.transport()));
                let response = match &exchange {
                    Ok(batch) => qualify_response(
                        batch.response.wire(),
                        context.header.id,
                        &context.question,
                    )
                    .map(|wire| {
                        let identity = batch
                            .selected_entry
                            .and_then(|entry| {
                                invocation_identity(context.config, dispatch.executable(), entry)
                            })
                            .or_else(|| upstream_identity(context.config, dispatch.executable()));
                        (wire, identity)
                    }),
                    Err(_) => None,
                };
                ledger_guard.record(
                    selected_entry,
                    response_transport,
                    if response.is_some() {
                        UpstreamAttemptOutcome::Response
                    } else {
                        UpstreamAttemptOutcome::Failed
                    },
                );
                let Some((wire, identity)) = response else {
                    return BranchOutcome::failure(
                        machine.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "branch upstream exchange failed",
                        )),
                    );
                };
                machine.state_mut().set_raw_response(wire);
                source = identity;
                step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Continue)) {
                    Ok(step) => step,
                    Err(error) => {
                        return BranchOutcome::failure(machine.state().clone(), error);
                    }
                };
            }
        }
    }
}

fn run_target<'a, E: ExchangeExecutor + ?Sized>(
    target: NativeTarget,
    mut successor: ExecutionMachine<'a>,
    context: BranchContext<'a, E>,
    source: Option<String>,
) -> Pin<Box<dyn Future<Output = BranchOutcome> + 'a>> {
    let cache_accessed = Rc::clone(&context.cache_accessed);
    Box::pin(async move {
        let mut outcome = match target {
            NativeTarget::Sequence(sequence) => {
                let control = successor.control().fork_child(CancellationToken::new());
                let target_machine = match ExecutionMachine::new(
                    &context.config.program,
                    sequence,
                    successor.state().clone(),
                    control,
                ) {
                    Ok(machine) => machine,
                    Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
                };
                let result = drive_branch(
                    target_machine,
                    context.clone().with_empty_response_allowed(),
                    source,
                )
                .await;
                if let Some(error) = result.error {
                    return BranchOutcome::failure(result.state, error);
                }
                *successor.state_mut() = result.state;
                drive_branch(successor, context, result.source).await
            }
            NativeTarget::Fixture(fixture) => {
                if let Err(error) = consume_external_target(&mut successor, &context) {
                    return BranchOutcome::failure(successor.state().clone(), error);
                }
                let outcome = match context.config.program.fixture(fixture) {
                    Some(fixture) => fixture.executable.execute(successor.state_mut()),
                    None => Err(mosdns_sequence_core::ExecutorError::new(
                        "branch fixture is missing",
                    )),
                };
                match outcome {
                    Ok(ExecutorOutcome::Continue | ExecutorOutcome::Return) => {
                        drive_branch(successor, context, source).await
                    }
                    Ok(ExecutorOutcome::Accept) => {
                        BranchOutcome::success(successor.state().clone(), source)
                    }
                    Ok(ExecutorOutcome::Reject { rcode }) => {
                        if successor
                            .state_mut()
                            .set_synthesized_response(rcode)
                            .is_err()
                        {
                            return BranchOutcome::failure(
                                successor.state().clone(),
                                ExecutionError::Executor(
                                    mosdns_sequence_core::ExecutorError::InvalidRcode(rcode),
                                ),
                            );
                        }
                        BranchOutcome::success(successor.state().clone(), None)
                    }
                    Ok(ExecutorOutcome::Exit) => {
                        if matches!(successor.state().response, MachineResponseState::None) {
                            BranchOutcome::failure(
                                successor.state().clone(),
                                ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                                    "branch exited without a response",
                                )),
                            )
                        } else {
                            BranchOutcome::success(successor.state().clone(), source)
                        }
                    }
                    Err(error) => BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::Executor(error),
                    ),
                }
            }
            NativeTarget::External(executable) => {
                if let Err(error) = consume_external_target(&mut successor, &context) {
                    return BranchOutcome::failure(successor.state().clone(), error);
                }
                if let Some(policy) = context
                    .config
                    .fallbacks
                    .iter()
                    .find(|policy| policy.executable == executable)
                {
                    return run_fallback(policy.clone(), successor, context, source).await;
                }
                if let Some(preference) = context
                    .config
                    .preferences
                    .iter()
                    .find(|preference| preference.executable == executable)
                {
                    return run_preference(preference.clone(), successor, context, source).await;
                }
                if context
                    .config
                    .cache
                    .as_ref()
                    .is_some_and(|cache| cache.executable == executable)
                {
                    if context.cache_accessed.replace(true) {
                        return BranchOutcome::failure(
                            successor.state().clone(),
                            ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                                "duplicate cache access in one branch",
                            )),
                        );
                    }
                    if let Ok(Some(wire)) = context.cache.lookup(context.raw()) {
                        successor.state_mut().set_raw_response(wire);
                        return BranchOutcome::success(successor.state().clone(), None);
                    }
                    let pending_store = context.cache.begin_store(context.raw()).ok().flatten();
                    let result = drive_branch(successor, context.clone(), source).await;
                    if result.is_success()
                        && !context.root_cancellation.is_cancelled()
                        && Instant::now() < context.deadline
                    {
                        if let Some(token) = pending_store {
                            if let MachineResponseState::Raw(wire) = &result.state.response {
                                let _ = token.publish(wire.as_bytes());
                            }
                        }
                    }
                    return result;
                }
                let cancellation = context.branch_cancellation.child_token();
                let ledger = new_branch_ledger(&context, executable);
                let mut ledger_guard =
                    BranchLedgerGuard::new(&context, executable, ledger.clone(), None);
                let exchange = context
                    .executor
                    .exchange_invocation(
                        executable,
                        context.raw(),
                        context.deadline,
                        cancellation,
                        ledger.clone(),
                    )
                    .await;
                let selected_entry = exchange
                    .as_ref()
                    .ok()
                    .and_then(|batch| batch.selected_entry);
                let response_transport = exchange
                    .as_ref()
                    .ok()
                    .map(|batch| branch_transport(batch.response.transport()));
                let wire = exchange.as_ref().ok().and_then(|batch| {
                    qualify_response(batch.response.wire(), context.header.id, &context.question)
                });
                ledger_guard.record(
                    selected_entry,
                    response_transport,
                    if wire.is_some() {
                        UpstreamAttemptOutcome::Response
                    } else {
                        UpstreamAttemptOutcome::Failed
                    },
                );
                let Some(wire) = wire else {
                    return BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "branch external exchange failed",
                        )),
                    );
                };
                let source = exchange.ok().and_then(|batch| {
                    batch
                        .selected_entry
                        .and_then(|entry| invocation_identity(context.config, executable, entry))
                        .or_else(|| upstream_identity(context.config, executable))
                });
                successor.state_mut().set_raw_response(wire);
                drive_branch(successor, context, source).await
            }
        };
        outcome.cache_accessed |= cache_accessed.get();
        outcome
    })
}

fn run_fallback<'a, E: ExchangeExecutor + ?Sized>(
    policy: FallbackConfig,
    successor: ExecutionMachine<'a>,
    context: BranchContext<'a, E>,
    source: Option<String>,
) -> Pin<Box<dyn Future<Output = BranchOutcome> + 'a>> {
    Box::pin(async move {
        let started = Instant::now();
        let remaining = context.deadline.saturating_duration_since(started);
        let threshold = policy.threshold.min(remaining);
        let primary_cancel = context.branch_cancellation.child_token();
        let secondary_cancel = context.branch_cancellation.child_token();
        let primary_core = CancellationToken::new();
        let secondary_core = CancellationToken::new();
        let primary_core_cancel = primary_core.clone();
        let secondary_core_cancel = secondary_core.clone();
        let primary_machine = match successor.fork_branch(primary_core) {
            Ok(machine) => machine,
            Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
        };
        let secondary_machine = match successor.fork_branch(secondary_core) {
            Ok(machine) => machine,
            Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
        };
        let (primary_id, secondary_id) = if let Some(trace) = &context.trace {
            let mut trace = trace.borrow_mut();
            let primary_id = trace.add_branch(
                context.branch_id,
                "primary",
                "fallback",
                context.question.qtype,
            );
            let secondary_id = trace.add_branch(
                context.branch_id,
                "secondary",
                "fallback",
                context.question.qtype,
            );
            trace.start(Some(primary_id));
            (Some(primary_id), Some(secondary_id))
        } else {
            (None, None)
        };
        let primary_context = context
            .with_branch(primary_id)
            .with_transport_cancellation(primary_cancel.clone());
        let primary_future = run_target(
            policy.primary,
            primary_machine,
            primary_context,
            source.clone(),
        );
        if policy.always_standby || threshold.is_zero() {
            let secondary_future = run_target(
                policy.secondary,
                secondary_machine,
                context
                    .with_branch(secondary_id)
                    .with_transport_cancellation(secondary_cancel.clone()),
                source.clone(),
            );
            let mut primary = Box::pin(primary_future);
            let mut secondary = Box::pin(secondary_future);
            let mut release = Box::pin(tokio::time::sleep(threshold));
            let mut primary_result: Option<BranchOutcome> = None;
            let mut secondary_result: Option<BranchOutcome> = None;
            let mut released = threshold.is_zero();
            loop {
                if let Some(result) = primary_result.take() {
                    // A primary success is immediately eligible. Only a
                    // secondary success is buffered until threshold release
                    // (or primary failure), so normal fallback never delays
                    // an already valid primary answer.
                    if result.is_success() {
                        secondary_cancel.cancel();
                        secondary_core_cancel.cancel();
                        if secondary_result.is_none() {
                            let _ = secondary.as_mut().await;
                        }
                        trace_outcome(&context, primary_id, &result, true);
                        if let Some(secondary_result) = secondary_result.as_ref() {
                            trace_outcome(&context, secondary_id, secondary_result, false);
                        } else if secondary_id.is_some() {
                            if let Some(trace) = &context.trace {
                                trace.borrow_mut().mark(secondary_id, "canceled");
                            }
                        }
                        return commit_branch_winner(result, &context, successor.state());
                    }
                    primary_result = Some(result);
                }
                if let Some(result) = secondary_result.take() {
                    let primary_failed = primary_result
                        .as_ref()
                        .is_some_and(|primary| !primary.is_success());
                    if result.is_success() && (released || primary_failed) {
                        primary_cancel.cancel();
                        primary_core_cancel.cancel();
                        if primary_result.is_none() {
                            let _ = primary.as_mut().await;
                        }
                        trace_outcome(&context, secondary_id, &result, true);
                        if let Some(primary_result) = primary_result.as_ref() {
                            trace_outcome(&context, primary_id, primary_result, false);
                        } else if primary_id.is_some() {
                            if let Some(trace) = &context.trace {
                                trace.borrow_mut().mark(primary_id, "canceled");
                            }
                        }
                        return commit_branch_winner(result, &context, successor.state());
                    }
                    secondary_result = Some(result);
                }
                if primary_result.is_some() && secondary_result.is_some() {
                    if let Some(result) = primary_result.as_ref() {
                        trace_outcome(&context, primary_id, result, false);
                    }
                    if let Some(result) = secondary_result.as_ref() {
                        trace_outcome(&context, secondary_id, result, false);
                    }
                    return BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "fallback branches produced no usable response",
                        )),
                    );
                }
                // Poll primary first so synchronously-ready ties retain the
                // frozen primary-first contract. If primary is pending,
                // select immediately polls secondary in the same turn.
                tokio::select! {
                    biased;
                    result = &mut primary, if primary_result.is_none() => primary_result = Some(result),
                    result = &mut secondary, if secondary_result.is_none() => secondary_result = Some(result),
                    _ = &mut release, if !released => released = true,
                }
            }
        }
        let mut primary = Box::pin(primary_future);
        let mut release = Box::pin(tokio::time::sleep(threshold));
        tokio::select! {
            biased;
            result = &mut primary => {
                if result.is_success() {
                    secondary_cancel.cancel();
                    secondary_core_cancel.cancel();
                    trace_outcome(&context, primary_id, &result, true);
                    return commit_branch_winner(result, &context, successor.state());
                }
                trace_outcome(&context, primary_id, &result, false);
                trace_start(&context, secondary_id);
                let secondary = run_target(
                    policy.secondary,
                    secondary_machine,
                    context
                        .with_branch(secondary_id)
                        .with_transport_cancellation(secondary_cancel.clone()),
                    source.clone(),
                );
                let secondary = Box::pin(secondary);
                let result = secondary.await;
                if result.is_success() {
                    trace_outcome(&context, secondary_id, &result, true);
                    return commit_branch_winner(result, &context, successor.state());
                }
                trace_outcome(&context, secondary_id, &result, false);
                BranchOutcome::failure(
                    successor.state().clone(),
                    ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                        "fallback branches produced no usable response",
                    )),
                )
            }
            _ = &mut release => {
                trace_start(&context, secondary_id);
                let secondary_future = run_target(
                    policy.secondary,
                    secondary_machine,
                    context
                        .with_branch(secondary_id)
                        .with_transport_cancellation(secondary_cancel.clone()),
                    source,
                );
                let mut secondary = Box::pin(secondary_future);
                tokio::select! {
                    biased;
                    result = &mut primary => {
                        if result.is_success() {
                            secondary_cancel.cancel();
                            secondary_core_cancel.cancel();
                            let _ = secondary.as_mut().await;
                            trace_outcome(&context, primary_id, &result, true);
                            if secondary_id.is_some() {
                                if let Some(trace) = &context.trace {
                                    trace.borrow_mut().mark(secondary_id, "canceled");
                                }
                            }
                            commit_branch_winner(result, &context, successor.state())
                        } else {
                            trace_outcome(&context, primary_id, &result, false);
                            let result = secondary.await;
                            if result.is_success() {
                                trace_outcome(&context, secondary_id, &result, true);
                                commit_branch_winner(result, &context, successor.state())
                            } else {
                                trace_outcome(&context, secondary_id, &result, false);
                                BranchOutcome::failure(successor.state().clone(), ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new("fallback branches produced no usable response")))
                            }
                        }
                    }
                    result = &mut secondary => {
                        if result.is_success() {
                            primary_cancel.cancel();
                            primary_core_cancel.cancel();
                            let _ = primary.as_mut().await;
                            trace_outcome(&context, secondary_id, &result, true);
                            if primary_id.is_some() {
                                if let Some(trace) = &context.trace {
                                    trace.borrow_mut().mark(primary_id, "canceled");
                                }
                            }
                            commit_branch_winner(result, &context, successor.state())
                        } else {
                            trace_outcome(&context, secondary_id, &result, false);
                            let result = primary.await;
                            if result.is_success() {
                                trace_outcome(&context, primary_id, &result, true);
                                commit_branch_winner(result, &context, successor.state())
                            } else {
                                trace_outcome(&context, primary_id, &result, false);
                                BranchOutcome::failure(successor.state().clone(), ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new("fallback branches produced no usable response")))
                            }
                        }
                    }
                }
            }
        }
    })
}

fn run_preference<'a, E: ExchangeExecutor + ?Sized>(
    policy: PreferenceConfig,
    mut successor: ExecutionMachine<'a>,
    context: BranchContext<'a, E>,
    source: Option<String>,
) -> Pin<Box<dyn Future<Output = BranchOutcome> + 'a>> {
    Box::pin(async move {
        let preferred_qtype = match policy.family {
            crate::config::PreferenceFamily::Ipv4 => 1,
            crate::config::PreferenceFamily::Ipv6 => 28,
        };
        let policy_name = match policy.family {
            crate::config::PreferenceFamily::Ipv4 => "prefer_ipv4",
            crate::config::PreferenceFamily::Ipv6 => "prefer_ipv6",
        };
        if context.question.qtype != 1 && context.question.qtype != 28 {
            return drive_branch(successor, context, source).await;
        }
        let key = preference_cache_key(&context.question);
        let now = policy.clock.now();
        {
            let mut evidence = policy.evidence.borrow_mut();
            evidence.retain(|_, expiry| *expiry > now);
            if context.question.qtype != preferred_qtype
                && evidence.get(&key).is_some_and(|expiry| *expiry > now)
            {
                if let Some(trace) = &context.trace {
                    let mut trace = trace.borrow_mut();
                    let original_id = trace.add_branch(
                        context.branch_id,
                        "original",
                        policy_name,
                        context.question.qtype,
                    );
                    let reference_id = trace.add_branch(
                        context.branch_id,
                        "reference",
                        policy_name,
                        preferred_qtype,
                    );
                    trace.mark(Some(original_id), "suppressed");
                    trace.mark(Some(reference_id), "skipped");
                }
                if successor.state_mut().set_synthesized_response(0).is_err() {
                    return BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "preference suppression response could not be built",
                        )),
                    );
                }
                return commit_branch_winner(
                    BranchOutcome::success(successor.state().clone(), None),
                    &context,
                    successor.state(),
                );
            }
        }
        if context.question.qtype == preferred_qtype {
            let branch_id = context.trace.as_ref().map(|trace| {
                let id = trace.borrow_mut().add_branch(
                    context.branch_id,
                    "original",
                    policy_name,
                    context.question.qtype,
                );
                trace.borrow_mut().start(Some(id));
                id
            });
            let result = drive_branch(successor, context.with_branch(branch_id), source).await;
            trace_outcome(&context, branch_id, &result, result.is_success());
            if result.is_success()
                && response_raw(&result.state.response)
                    .is_some_and(|wire| response_has_type(wire, preferred_qtype))
            {
                remember_preference(&policy, key);
            }
            return result;
        }

        let original_machine = match successor.fork_branch(CancellationToken::new()) {
            Ok(machine) => machine,
            Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
        };
        let reference_machine = match successor.fork_branch(CancellationToken::new()) {
            Ok(machine) => machine,
            Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
        };
        // The probe changes QTYPE, so an inherited response for the original
        // question is not eligible evidence for the reference branch.
        let mut reference_machine = reference_machine;
        reference_machine
            .state_mut()
            .set_response(MachineResponseState::None);
        let mut reference_question = context.question.clone();
        reference_question.qtype = preferred_qtype;
        let reference_raw = rewrite_query_qtype(context.raw(), preferred_qtype).ok_or_else(|| {
            ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                "preference query rewrite failed",
            ))
        });
        let reference_raw = match reference_raw {
            Ok(raw) => raw,
            Err(error) => return BranchOutcome::failure(successor.state().clone(), error),
        };
        let reference_context = context.with_query(reference_raw, reference_question);
        let original_transport = context.branch_cancellation.child_token();
        let reference_transport = context.branch_cancellation.child_token();
        let original_core_cancel = original_machine.control().cancellation_token();
        let reference_core_cancel = reference_machine.control().cancellation_token();
        let original_context = context.with_transport_cancellation(original_transport.clone());
        let reference_context =
            reference_context.with_transport_cancellation(reference_transport.clone());
        let (original_id, reference_id) = if let Some(trace) = &context.trace {
            let mut trace = trace.borrow_mut();
            let original_id = trace.add_branch(
                context.branch_id,
                "original",
                policy_name,
                context.question.qtype,
            );
            let reference_id =
                trace.add_branch(context.branch_id, "reference", policy_name, preferred_qtype);
            trace.start(Some(original_id));
            (Some(original_id), Some(reference_id))
        } else {
            (None, None)
        };
        trace_start(&context, reference_id);
        let original_context = original_context.with_branch(original_id);
        let reference_context = reference_context.with_branch(reference_id);
        let mut original = Box::pin(drive_branch(original_machine, original_context, source));
        let mut reference = Box::pin(drive_branch(reference_machine, reference_context, None));
        let mut original_result: Option<BranchOutcome> = None;
        let mut reference_result: Option<BranchOutcome> = None;
        let mut wait_timer = Box::pin(tokio::time::sleep(Duration::from_secs(31_536_000)));
        loop {
            if let Some(reference_value) = reference_result.as_ref() {
                if reference_value.is_success()
                    && response_raw(&reference_value.state.response)
                        .is_some_and(|wire| response_has_type(wire, preferred_qtype))
                {
                    if successor.state_mut().set_synthesized_response(0).is_err() {
                        return BranchOutcome::failure(
                            successor.state().clone(),
                            ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                                "preference suppression response could not be built",
                            )),
                        );
                    }
                    original_transport.cancel();
                    original_core_cancel.cancel();
                    if original_result.is_none() {
                        let _ = original.as_mut().await;
                    }
                    if let Some(reference_value) = reference_result.as_ref() {
                        trace_outcome(&context, reference_id, reference_value, false);
                    }
                    if let Some(trace) = &context.trace {
                        trace.borrow_mut().mark(original_id, "suppressed");
                    }
                    remember_preference(&policy, key);
                    return commit_branch_winner(
                        BranchOutcome::success(successor.state().clone(), None),
                        &context,
                        successor.state(),
                    );
                }
            }
            if let Some(original_value) = original_result.take() {
                if reference_result.is_some() {
                    let committed =
                        commit_branch_winner(original_value, &context, successor.state());
                    trace_outcome(&context, original_id, &committed, committed.is_success());
                    if let Some(reference_value) = reference_result.as_ref() {
                        trace_outcome(&context, reference_id, reference_value, false);
                    }
                    return committed;
                }
                original_result = Some(original_value);
            }
            if original_result.is_some() && reference_result.is_some() {
                let original_value = original_result.take().unwrap_or_else(|| {
                    BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::Executor(mosdns_sequence_core::ExecutorError::new(
                            "preference branches completed without an original result",
                        )),
                    )
                });
                let committed = commit_branch_winner(original_value, &context, successor.state());
                trace_outcome(&context, original_id, &committed, committed.is_success());
                if let Some(reference_value) = reference_result.as_ref() {
                    trace_outcome(&context, reference_id, reference_value, false);
                }
                return committed;
            }
            tokio::select! {
                biased;
                result = &mut original, if original_result.is_none() => {
                    original_result = Some(result);
                    let wait = context
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(500));
                    wait_timer
                        .as_mut()
                        .reset(tokio::time::Instant::now() + wait);
                },
                result = &mut reference, if reference_result.is_none() => reference_result = Some(result),
                _ = &mut wait_timer, if original_result.is_some() && reference_result.is_none() => {
                    reference_transport.cancel();
                    reference_core_cancel.cancel();
                    let _ = reference.as_mut().await;
                    let original_value = original_result.take().unwrap_or_else(|| BranchOutcome::failure(
                        successor.state().clone(),
                        ExecutionError::BudgetExceeded,
                    ));
                    let committed =
                        commit_branch_winner(original_value, &context, successor.state());
                    trace_outcome(&context, original_id, &committed, committed.is_success());
                    if reference_id.is_some() {
                        if let Some(trace) = &context.trace {
                            trace.borrow_mut().mark(reference_id, "canceled");
                        }
                    }
                    return committed;
                }
            }
        }
    })
}

fn response_has_type(wire: &[u8], qtype: u16) -> bool {
    observe_answer_records(wire)
        .map(|answers| answers.into_iter().any(|answer| answer.rrtype == qtype))
        .unwrap_or(false)
}

fn response_raw(response: &MachineResponseState) -> Option<&[u8]> {
    match response {
        MachineResponseState::Raw(wire) => Some(wire.as_bytes()),
        MachineResponseState::None | MachineResponseState::Synthesized(_) => None,
    }
}

fn preference_cache_key(question: &QuestionInfo) -> String {
    question
        .qname_wire
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn remember_preference(policy: &PreferenceConfig, key: String) {
    let now = policy.clock.now();
    let mut evidence = policy.evidence.borrow_mut();
    if evidence.len() >= 65_536 && !evidence.contains_key(&key) {
        if let Some(oldest) = evidence
            .iter()
            .min_by_key(|(_, expiry)| **expiry)
            .map(|(key, _)| key.clone())
        {
            evidence.remove(&oldest);
        }
    }
    evidence.insert(key, now + std::time::Duration::from_secs(60 * 60));
}

fn rewrite_query_qtype(raw: &[u8], qtype: u16) -> Option<Vec<u8>> {
    if raw.len() < 12 {
        return None;
    }
    let mut offset = 12;
    loop {
        let length = *raw.get(offset)?;
        if length == 0 {
            offset = offset.checked_add(1)?;
            break;
        }
        if length & 0xc0 == 0xc0 {
            offset = offset.checked_add(2)?;
            break;
        }
        if length & 0xc0 != 0 || length > 63 {
            return None;
        }
        offset = offset.checked_add(1 + usize::from(length))?;
    }
    let end = offset.checked_add(4)?;
    let mut rewritten = raw.to_vec();
    if end > rewritten.len() {
        return None;
    }
    rewritten[offset..offset + 2].copy_from_slice(&qtype.to_be_bytes());
    Some(rewritten)
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
        .or_else(|| {
            let invocation = config
                .forward_invocations
                .iter()
                .find(|invocation| invocation.executable == executable)?;
            let definition = config.forward_definitions.get(invocation.definition)?;
            let entry_index = invocation.entries.first().copied()?;
            definition
                .entries
                .get(entry_index)
                .map(|entry| entry.identity.clone())
        })
}

fn invocation_identity(
    config: &CompiledConfig,
    executable: ExecutableId,
    entry_index: usize,
) -> Option<String> {
    let invocation = config
        .forward_invocations
        .iter()
        .find(|invocation| invocation.executable == executable)?;
    let definition = config.forward_definitions.get(invocation.definition)?;
    definition
        .entries
        .get(entry_index)
        .map(|entry| entry.identity.clone())
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

fn record_invocation_ledger(
    config: &CompiledConfig,
    executable: Option<ExecutableId>,
    ledger: &UpstreamAttemptLedger,
    selected_entry: Option<usize>,
    qtype: u16,
    facts: &mut ExecutionFacts,
) {
    let Some(executable) = executable else {
        return;
    };
    for slot in ledger.slots() {
        let upstream = facts
            .capture_audit_details
            .then(|| invocation_identity(config, executable, slot.entry_index))
            .flatten();
        let outcome = slot.outcome.unwrap_or_else(|| {
            if facts.checkpoint.capture_audit_details() {
                UpstreamAttemptOutcome::Interrupted
            } else {
                UpstreamAttemptOutcome::Canceled
            }
        });
        facts.record_invocation_attempt(
            executable,
            slot.entry_index,
            upstream.clone(),
            slot.peer,
            slot.transport,
            outcome,
        );
        if let Some(trace) = &facts.policy_trace {
            let mut trace = trace.borrow_mut();
            let entry = upstream
                .clone()
                .unwrap_or_else(|| format!("executable:{:?}:{}", executable, slot.entry_index));
            if let Some(trace_slot) = slot.trace_slot {
                trace.complete_attempt(trace_slot, entry, slot.peer, slot.transport, outcome);
            } else {
                trace.record_attempt(Some(0), qtype, entry, slot.peer, slot.transport, outcome);
            }
        }
        if selected_entry == Some(slot.entry_index) && outcome == UpstreamAttemptOutcome::Response {
            if let Some(upstream) = upstream {
                facts.response_source = Some(ResponseSource::Upstream(upstream.clone()));
                if let Some(trace) = &facts.policy_trace {
                    let mut trace = trace.borrow_mut();
                    trace.candidate(Some(0), upstream.clone(), slot.peer, slot.transport);
                    trace.select(Some(0), true);
                }
                facts.select_invocation_attempt(upstream, slot.peer, slot.transport);
            }
        } else if outcome != UpstreamAttemptOutcome::Response {
            if let Some(upstream) = upstream {
                facts.record_invocation_failure(&upstream, outcome);
            }
        }
    }
}

fn branch_transport(transport: mosdns_upstream_core::Transport) -> UpstreamTransport {
    match transport {
        mosdns_upstream_core::Transport::Udp => UpstreamTransport::Udp,
        mosdns_upstream_core::Transport::Tcp => UpstreamTransport::Tcp,
        mosdns_upstream_core::Transport::Quic => UpstreamTransport::Https,
    }
}

struct BranchLedgerRecord<'a> {
    config: &'a CompiledConfig,
    trace: Option<&'a Rc<RefCell<BranchTrace>>>,
    branch_metrics: &'a Rc<RefCell<UpstreamAttemptList>>,
    root_cancelled: bool,
    branch_id: Option<usize>,
    qtype: u16,
    executable: ExecutableId,
    ledger: &'a UpstreamAttemptLedgerHandle,
    trace_token: Option<u64>,
    selected_entry: Option<usize>,
    fallback_transport: Option<UpstreamTransport>,
    fallback_outcome: UpstreamAttemptOutcome,
}

fn record_branch_ledger_data(record: BranchLedgerRecord<'_>) {
    let BranchLedgerRecord {
        config,
        trace,
        branch_metrics,
        root_cancelled,
        branch_id,
        qtype,
        executable,
        ledger,
        trace_token,
        selected_entry,
        fallback_transport,
        fallback_outcome,
    } = record;
    let slots = ledger.borrow().slots().to_vec();
    let trace = trace.cloned();
    if slots.is_empty() {
        let entry = upstream_identity(config, executable)
            .unwrap_or_else(|| format!("executable:{:?}", executable));
        branch_metrics
            .borrow_mut()
            .push_metric(UpstreamMetricAttempt {
                executable,
                entry_index: 0,
                outcome: fallback_outcome,
            });
        if let Some(trace) = trace {
            let mut trace = trace.borrow_mut();
            if let Some(token) = trace_token {
                trace.complete_attempt(
                    token,
                    entry.clone(),
                    None,
                    fallback_transport,
                    fallback_outcome,
                );
            } else {
                trace.record_attempt(
                    branch_id,
                    qtype,
                    entry.clone(),
                    None,
                    fallback_transport,
                    fallback_outcome,
                );
            }
            if fallback_outcome == UpstreamAttemptOutcome::Response {
                trace.candidate(branch_id, entry, None, fallback_transport);
            }
        }
        return;
    }
    for slot in slots {
        let entry = invocation_identity(config, executable, slot.entry_index)
            .unwrap_or_else(|| format!("executable:{:?}:{}", executable, slot.entry_index));
        let outcome = slot.outcome.unwrap_or({
            if root_cancelled {
                UpstreamAttemptOutcome::Canceled
            } else {
                UpstreamAttemptOutcome::Interrupted
            }
        });
        branch_metrics
            .borrow_mut()
            .push_metric(UpstreamMetricAttempt {
                executable,
                entry_index: slot.entry_index,
                outcome,
            });
        if let Some(trace) = &trace {
            let mut trace = trace.borrow_mut();
            if let Some(token) = slot.trace_slot.or(trace_token) {
                trace.complete_attempt(token, entry.clone(), slot.peer, slot.transport, outcome);
            } else {
                trace.record_attempt(
                    branch_id,
                    qtype,
                    entry.clone(),
                    slot.peer,
                    slot.transport,
                    outcome,
                );
            }
            if selected_entry == Some(slot.entry_index)
                && outcome == UpstreamAttemptOutcome::Response
            {
                trace.candidate(branch_id, entry, slot.peer, slot.transport);
            }
        }
    }
}

fn canceled_execution(mut facts: ExecutionFacts) -> ExecutionResult {
    facts.response_source = None;
    facts.selected_peer = None;
    if let Some(diagnostics) = &mut facts.upstream_diagnostics {
        diagnostics.selected = None;
    }
    if let Some(trace) = &facts.policy_trace {
        trace.borrow_mut().clear_selection(Some("canceled"));
    }
    result_from_wire(Vec::new(), facts)
}

fn terminal_policy_failure(error: ExecutionError, mut facts: ExecutionFacts) -> ExecutionResult {
    debug_assert!(matches!(
        error,
        ExecutionError::Cancelled | ExecutionError::BudgetExceeded
    ));
    facts.response_source = None;
    facts.selected_peer = None;
    if let Some(diagnostics) = &mut facts.upstream_diagnostics {
        diagnostics.selected = None;
    }
    let terminal_decision = if matches!(&error, ExecutionError::Cancelled) {
        "canceled"
    } else {
        "interrupted"
    };
    if let Some(trace) = &facts.policy_trace {
        trace.borrow_mut().clear_selection(Some(terminal_decision));
    }
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
    facts.note_routing(machine.state());
    facts.note_response(machine.state());
    let response = response_from_state(machine, header, question);
    result_from_wire(response, facts)
}

fn result_from_wire(response_wire: Vec<u8>, mut facts: ExecutionFacts) -> ExecutionResult {
    facts.absorb_branch_metrics();
    facts.finalize_policy_trace();
    if facts.policy_failure.take().is_some() && facts.failure_provenance.is_none() {
        facts.set_failure_provenance(FailureProvenance::LocalFailure(
            LocalFailureKind::NoUsableUpstreamResponse,
        ));
    }
    let routing = if response_wire.is_empty() {
        RoutingState::default()
    } else {
        facts
            .response_routing
            .take()
            .unwrap_or_else(|| std::mem::take(&mut facts.routing))
    };
    let supplying_identity = facts.response_source.as_ref().and_then(|source| {
        if let ResponseSource::Upstream(upstream) = source {
            Some(upstream.clone())
        } else {
            None
        }
    });
    let derived_final_upstream = supplying_identity.clone();
    let final_sequence = routing
        .final_sequence
        .clone()
        .or_else(|| facts.final_sequence.clone());
    let final_upstream = routing
        .final_upstream
        .clone()
        .or(derived_final_upstream.clone());
    let effective_tag = routing.effective_tag.clone().or_else(|| {
        routing
            .domain_set
            .as_deref()
            .map(|domain_set| {
                compute_effective_tag(
                    domain_set,
                    final_upstream.as_deref(),
                    routing.matched_group.as_deref(),
                    final_sequence.as_deref(),
                )
            })
            .or_else(|| {
                (routing.matched_rule_source.is_none()
                    && routing.domain_set.is_none()
                    && final_sequence.is_some())
                .then(|| "unmatched_rule".to_owned())
            })
    });
    let actual_upstream = (!response_wire.is_empty())
        .then(|| {
            supplying_identity
                .as_deref()
                .and_then(|identity| upstream_endpoint(facts.config, identity))
                .or_else(|| facts.selected_peer.map(|peer| peer.to_string()))
        })
        .flatten();
    let response_source = facts
        .response_source
        .take()
        .unwrap_or(ResponseSource::Local);
    if !matches!(response_source, ResponseSource::Upstream(_)) {
        if let Some(diagnostics) = &mut facts.upstream_diagnostics {
            diagnostics.selected = None;
        }
    }
    let response = observed_response(&response_wire, response_source);
    let response_details = if facts.capture_audit_details {
        diagnose_response_wire(&response_wire)
    } else {
        ResponseDetails::no_response()
    };
    facts.completed = true;
    ExecutionResult {
        response_wire,
        response,
        response_details,
        cache_status: facts.cache_status,
        final_sequence,
        matched_group: routing.matched_group,
        domain_set: routing.domain_set,
        effective_tag,
        matched_rule_source: routing.matched_rule_source,
        final_upstream,
        upstream_targets: routing
            .final_upstream_targets
            .or_else(|| actual_upstream.clone()),
        selected_upstream: actual_upstream,
        upstream_attempts: std::mem::take(&mut facts.upstream_attempts),
        upstream_diagnostics: facts.upstream_diagnostics.take(),
        failure_provenance: facts.failure_provenance.take(),
    }
}

fn upstream_endpoint(config: &CompiledConfig, identity: &str) -> Option<String> {
    config
        .forwards
        .iter()
        .find(|forward| forward.upstream_tag.as_deref().unwrap_or(&forward.tag) == identity)
        .map(|forward| forward.endpoint.address().to_string())
        .or_else(|| {
            config
                .forward_definitions
                .iter()
                .flat_map(|definition| definition.entries.iter())
                .find(|entry| entry.identity == identity)
                .and_then(|entry| entry.target.dial_addr.map(|address| address.to_string()))
        })
}

fn compute_effective_tag(
    domain_set: &str,
    final_upstream: Option<&str>,
    matched_group: Option<&str>,
    final_sequence: Option<&str>,
) -> String {
    let domain_set = domain_set.trim();
    if domain_set.is_empty() || domain_set == "unmatched_rule" {
        return "unmatched_rule".to_owned();
    }
    if let Some(special) =
        normalize_special_tag(matched_group).or_else(|| normalize_special_tag(final_upstream))
    {
        return special;
    }
    let mut tags = Vec::new();
    for tag in domain_set
        .split('|')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    if tags.is_empty() {
        return "unmatched_rule".to_owned();
    }
    if let Some(tag) = tags.iter().find(|tag| tag.starts_with("特殊上游")) {
        return (*tag).to_owned();
    }
    for candidate in [
        "重定向",
        "指定客户端直连",
        "黑名单",
        "广告屏蔽",
        "BANAAAA",
        "BANSOA",
        "BANPTR",
        "BANHTTPS",
        "DDNS域名",
        "stash国内",
        "stash国外",
        "clashmi国内",
        "clashmi国外",
        "sing-box国内",
        "sing-box国外",
    ] {
        if tags.contains(&candidate) {
            return candidate.to_owned();
        }
    }
    let no_v: Vec<_> = ["记忆无V4", "记忆无V6"]
        .into_iter()
        .filter(|tag| tags.contains(tag))
        .collect();
    if tags.contains(&"!CN fakeip filter") {
        return join_effective_tags(&no_v, "!CN fakeip filter");
    }
    let route_kind = match final_upstream.map(str::trim) {
        Some("domestic" | "cnfake") => Some("direct"),
        Some("foreign" | "foreignecs" | "nocnfake") => Some("proxy"),
        _ => None,
    };
    let has_direct_memory = tags.contains(&"记忆直连");
    let has_proxy_memory = tags.contains(&"记忆代理");
    if has_direct_memory || has_proxy_memory {
        let memory = match (has_direct_memory, has_proxy_memory, route_kind) {
            (true, true, Some("proxy")) | (true, false, Some("proxy")) => "记忆代理",
            (true, true, _) | (true, false, _) => "记忆直连",
            (false, true, Some("direct")) => "记忆代理转直连",
            (false, true, _) => "记忆代理",
            _ => "",
        };
        if !memory.is_empty() {
            return join_effective_tags(&no_v, memory);
        }
    }
    if route_kind == Some("proxy")
        && matches!(
            final_sequence.map(str::trim),
            Some("sequence_fakeip_addlist" | "sequence_fakeip_addlist_exit")
        )
    {
        for candidate in ["白名单", "订阅直连补充", "订阅直连", "CN fakeip filter"] {
            if tags.contains(&candidate) {
                return join_effective_tags(&no_v, "直连候选转代理");
            }
        }
    }
    let candidates = match route_kind {
        Some("direct") => [
            "白名单",
            "订阅直连补充",
            "订阅直连",
            "CN fakeip filter",
            "!CN fakeip filter",
        ]
        .as_slice(),
        Some("proxy") => ["灰名单", "订阅代理补充", "订阅代理"].as_slice(),
        _ => [].as_slice(),
    };
    if let Some(tag) = candidates.iter().find(|candidate| tags.contains(candidate)) {
        return join_effective_tags(&no_v, tag);
    }
    tags.join("|")
}

fn normalize_special_tag(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let slot = value
        .strip_prefix("special_upstream_")
        .or_else(|| value.strip_prefix("special_"))?;
    (!slot.is_empty() && slot.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("特殊上游{slot}"))
}

fn join_effective_tags(no_v: &[&str], core: &str) -> String {
    let mut tags = Vec::new();
    for tag in no_v
        .iter()
        .copied()
        .chain((!core.is_empty()).then_some(core))
    {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags.join("|")
}

fn diagnose_response_wire(response_wire: &[u8]) -> ResponseDetails {
    if response_wire.is_empty() {
        return ResponseDetails::no_response();
    }
    let safe_flags = match (response_wire.get(2), response_wire.get(3)) {
        (Some(&high), Some(&low)) => ResponseFlags {
            aa: high & 0x04 != 0,
            tc: high & 0x02 != 0,
            ra: low & 0x80 != 0,
        },
        _ => ResponseFlags::default(),
    };
    let base_rcode = response_wire
        .get(3)
        .map_or(0, |flags| u16::from(flags & 0x0f));
    let metadata = observe_response_metadata(response_wire).ok();
    let rcode = metadata.as_ref().map_or(base_rcode, |value| value.rcode);
    match observe_answer_records(response_wire) {
        Ok(records) => {
            let has_raw = records
                .iter()
                .any(|record| !matches!(record.rrtype, 1 | 2 | 5 | 12 | 15 | 16 | 28));
            ResponseDetails {
                rcode,
                flags: safe_flags,
                answers: records
                    .into_iter()
                    .map(|record| AuditAnswer {
                        rrtype: record.rrtype,
                        ttl: record.ttl,
                        data: record.data,
                    })
                    .collect(),
                answer_details_status: if has_raw {
                    AnswerDetailsStatus::RawRdata
                } else {
                    AnswerDetailsStatus::Complete
                },
                answer_decode_error: None,
            }
        }
        Err(error) => ResponseDetails {
            rcode,
            flags: safe_flags,
            answers: Vec::new(),
            answer_details_status: AnswerDetailsStatus::DecodeError,
            answer_decode_error: Some(answer_decode_error_name(error).to_owned()),
        },
    }
}

fn answer_decode_error_name(error: ResponseError) -> &'static str {
    match error {
        ResponseError::TooShort
        | ResponseError::TruncatedQuestion
        | ResponseError::TruncatedRecord => "truncated_message",
        ResponseError::NotResponse => "invalid_response",
        ResponseError::BadName => "bad_name",
        ResponseError::InvalidRecordData => "invalid_rdata",
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

pub(crate) fn qualify_response(
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

pub(crate) fn response_priority(response: &[u8]) -> u8 {
    match observe_response_metadata(response) {
        Ok(metadata)
            if metadata.answer_count > 0
                && observe_answer_addresses(response)
                    .map(|addresses| !addresses.is_empty())
                    .unwrap_or(false) =>
        {
            0
        }
        Ok(metadata) if metadata.rcode == 0 || metadata.rcode == 3 => 1,
        Ok(_) => 2,
        Err(_) => 3,
    }
}

fn set_servfail(machine: &mut ExecutionMachine<'_>) {
    let _ = machine
        .state_mut()
        .set_synthesized_response(u16::from(SERVFAIL));
}

fn policy_failure_for_core(error: &ExecutionError) -> Result<ExecutorError, ExecutionError> {
    match error {
        ExecutionError::Executor(error) => Ok(error.clone()),
        ExecutionError::Cancelled | ExecutionError::BudgetExceeded => Err(error.clone()),
        other => Ok(ExecutorError::new(format!(
            "native policy failed: {other:?}"
        ))),
    }
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
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{
        DispatchMetadata, ExecutableId, ExecutableSpec, ExecutionError, ExecutorError, ExternalRef,
        ExternalSpec, MatcherSpecInput, ProgramSpec, RuleSpec, SequenceRef, SequenceSpec,
    };
    use mosdns_upstream_core::{
        Endpoint, ExchangeResponse, SideEffectState, Transport, TransportCancellation,
        UpstreamError,
    };

    use super::{
        ExchangeExecutor, ExecutionCheckpoint, ExecutionRequest, compute_effective_tag,
        diagnose_response_wire, execute_request_with_executor, execute_request_with_observation,
        policy_failure_for_core,
    };
    use crate::assembly::{ForwardAdapter, HostOptions};
    use crate::cache::{CacheTestClock, NativeCacheAdapter};
    use crate::config::{
        CachePluginConfig, CompiledConfig, ForwardConfig, ListenerConfig, ListenerKind, LogLevel,
        SequenceConfig, compile_yaml, compile_yaml_with_base,
    };
    use crate::observer::{
        CacheStatus, FailureProvenance, LocalFailureKind, QueryObserver, QueryTerminalOutcome,
        QueryTransport, ResponseSource, ResponseState, TerminalObservation,
    };

    struct MockExchange {
        calls: Rc<Cell<u32>>,
        response: Vec<u8>,
        fail: bool,
    }

    struct PolicyExchange {
        calls: Rc<RefCell<Vec<(ExecutableId, u16)>>>,
        primary: ExecutableId,
        secondary: ExecutableId,
        primary_delay: Duration,
        secondary_delay: Duration,
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

    impl ExchangeExecutor for PolicyExchange {
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
            let (_, question) = parse_query(query).expect("policy query");
            self.calls.borrow_mut().push((executable, question.qtype));
            let delay = if executable == self.primary {
                self.primary_delay
            } else if executable == self.secondary {
                self.secondary_delay
            } else {
                Duration::ZERO
            };
            let response = response_for_qtype(query, question.qtype, [192, 0, 2, 55]);
            Box::pin(async move {
                tokio::time::sleep(delay).await;
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

    fn response_for_qtype(query: &[u8], qtype: u16, address: [u8; 4]) -> Vec<u8> {
        if qtype == 1 {
            return response_with_ip(query, address);
        }
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
        response.extend_from_slice(&qtype.to_be_bytes());
        response.extend_from_slice(&[0, 1]);
        response.extend_from_slice(&[0xc0, 0x0c]);
        response.extend_from_slice(&qtype.to_be_bytes());
        response.extend_from_slice(&[0, 1, 0, 0, 0, 10, 0, 16]);
        response.extend_from_slice(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, address[0], address[1], address[2], address[3],
        ]);
        response
    }

    #[test]
    fn final_response_flags_read_ra_from_the_low_dns_header_byte() {
        let query = query(91);
        let mut no_ra = response(&query);
        no_ra[3] &= !0x80;
        let no_ra_flags = diagnose_response_wire(&no_ra).flags;
        assert!(!no_ra_flags.ra);
        assert!(!no_ra_flags.tc, "RCODE bit 1 is not the TC flag");

        let mut with_ra = response(&query);
        with_ra[3] |= 0x80;
        assert!(diagnose_response_wire(&with_ra).flags.ra);
    }

    fn query_name(id: u16, name: &str) -> Vec<u8> {
        query_type(id, name, 1)
    }

    fn query_type(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut query = vec![(id >> 8) as u8, id as u8, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.trim_end_matches('.').split('.') {
            query.push(u8::try_from(label.len()).expect("test label length"));
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0]);
        query.extend_from_slice(&qtype.to_be_bytes());
        query.extend_from_slice(&[0, 1]);
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
        let observer = Arc::new(QueryObserver::new(false, ["phase5a_forward".to_owned()], 8));
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
        assert!(result.failure_provenance.is_none());
        assert_eq!(result.upstream_attempts.len(), 1);
        assert_eq!(result.upstream_attempts[0].upstream, "phase5a_forward");
        assert!(result.upstream_attempts.is_inline());

        admitted.capture_execution(TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: result.response,
            cache_status: result.cache_status,
            final_sequence: result.final_sequence,
            matched_group: result.matched_group,
            final_upstream: result.final_upstream,
            upstream_attempts: result.upstream_attempts,
            failure_provenance: result.failure_provenance,
            elapsed: Duration::ZERO,
            ..Default::default()
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
        assert_eq!(direct.upstream_attempts.len(), 1);
        assert!(direct.upstream_attempts.is_inline());
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
        assert!(matches!(
            cold.response,
            ResponseState::Dns {
                source: ResponseSource::Upstream(ref upstream),
                ..
            } if upstream == "forward"
        ));
        assert_eq!(warm.cache_status, CacheStatus::Hit);
        assert_eq!(
            warm.response,
            ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Cache
            }
        );
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
        let observer = std::sync::Arc::new(QueryObserver::new(true, ["forward".to_owned()], 2));
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
        assert_eq!(record.selected_upstream, None);
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
    fn dropped_flow_setter_execution_preserves_configured_metadata_precedence() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let local = tokio::task::LocalSet::new();
        let observer = std::sync::Arc::new(QueryObserver::new(true, ["forward".to_owned()], 2));
        let entered = Rc::new(Cell::new(false));
        let config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: $setter
      - exec: $forward
  - tag: setter
    type: flow_setter
    args: { matched_group: configured_group, final_sequence: configured_sequence, final_upstream: configured_upstream }
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53053", enable_audit: true }
"#,
        )
        .expect("flow setter config");

        local.block_on(&runtime, async {
            let task_observer = std::sync::Arc::clone(&observer);
            let task_entered = Rc::clone(&entered);
            let runner = tokio::task::spawn_local(async move {
                let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
                let options = HostOptions::default();
                let raw = query_name(86, "dropped-flow-setter.test");
                let (header, question) = parse_query(&raw).expect("query");
                let cancellation = TransportCancellation::new();
                let mut admitted = task_observer.admit(
                    "192.0.2.86:53000".parse().expect("client address"),
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
                panic!("pending flow setter exchange unexpectedly returned");
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
        assert_eq!(record.matched_group.as_deref(), Some("configured_group"));
        assert_eq!(
            record.final_sequence.as_deref(),
            Some("configured_sequence")
        );
        assert_eq!(
            record.final_upstream.as_deref(),
            Some("configured_upstream")
        );
        assert_eq!(record.response, ResponseState::NoResponse);
        assert_eq!(record.selected_upstream, None);
        assert_eq!(record.upstream_attempts.len(), 1);
        assert_eq!(record.upstream_attempts[0].upstream, "forward");
        assert_eq!(
            record.upstream_attempts[0].outcome,
            super::UpstreamAttemptOutcome::Interrupted
        );
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
            2,
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
        let route_a_call = config
            .forward_invocations
            .iter()
            .filter_map(|invocation| {
                let definition = config.forward_definitions.get(invocation.definition)?;
                (definition.tag == "phase5a_route_a").then_some(invocation.executable)
            })
            .nth(1)
            .expect("second route A call-site executable");
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
        assert!(matches!(
            direct.response,
            ResponseState::Dns {
                source: ResponseSource::Upstream(ref upstream),
                ..
            } if upstream == "route_a"
        ));

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
            &[b.executable, route_a_call]
        );
        assert_eq!(
            route_a
                .upstream_attempts
                .iter()
                .map(|attempt| attempt.upstream.as_str())
                .collect::<Vec<_>>(),
            ["route_b", "route_a"]
        );
        assert!(matches!(
            route_a.response,
            ResponseState::Dns {
                source: ResponseSource::Upstream(ref upstream),
                ..
            } if upstream == "route_a"
        ));

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
        assert!(matches!(
            route_c.response,
            ResponseState::Dns {
                source: ResponseSource::Upstream(ref upstream),
                ..
            } if upstream == "route_c"
        ));

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
            forward: Some(ForwardConfig {
                tag: forward,
                upstream_tag: None,
                endpoint,
                executable: forward_id,
            }),
            forwards: vec![ForwardConfig {
                tag: "forward".to_owned(),
                upstream_tag: None,
                endpoint,
                executable: forward_id,
            }],
            forward_definitions: Vec::new(),
            forward_invocations: Vec::new(),
            cache: Some(CachePluginConfig {
                tag: cache,
                executable: cache_id,
                capacity: 64,
            }),
            fallbacks: Vec::new(),
            preferences: Vec::new(),
            sequence: SequenceConfig {
                tag: "root".to_owned(),
                sequence: program.sequence_id("root").expect("root"),
                forward_executable: Some(forward_id),
            },
            listener: ListenerConfig {
                tag: "listener".to_owned(),
                kind: ListenerKind::Udp,
                entry: "root".to_owned(),
                listen: "127.0.0.1:1".parse().expect("listen"),
                enable_audit: false,
                idle_timeout: None,
            },
            api: None,
            domain_sets: Vec::new(),
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
                forward: Some(ForwardConfig {
                    tag: a,
                    upstream_tag: None,
                    endpoint,
                    executable: a_id,
                }),
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
                forward_definitions: Vec::new(),
                forward_invocations: Vec::new(),
                cache: None,
                fallbacks: Vec::new(),
                preferences: Vec::new(),
                sequence: SequenceConfig {
                    tag: "root".to_owned(),
                    sequence: program.sequence_id("root").expect("root"),
                    forward_executable: Some(a_id),
                },
                listener: ListenerConfig {
                    tag: "listener".to_owned(),
                    kind: ListenerKind::Udp,
                    entry: "root".to_owned(),
                    listen: "127.0.0.1:1".parse().expect("listen"),
                    enable_audit: false,
                    idle_timeout: None,
                },
                api: None,
                domain_sets: Vec::new(),
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

    /// A child sequence that runs the cache and then the given forward, and a
    /// parent that calls it and can overwrite the response afterwards. This is
    /// the shape R3 must keep separate: the child's completed successor result
    /// is cacheable even when the parent later replaces the final response.
    fn cache_child_then_parent_config(
        parent_tail: Vec<ExecutableSpec>,
    ) -> (CompiledConfig, ExecutableId, ExecutableId) {
        let child_forward = "child_forward".to_owned();
        let parent_forward = "parent_forward".to_owned();
        let cache = "cache".to_owned();
        let child_rules = vec![RuleSpec::unconditional(Some(vec![
            ExecutableSpec::External {
                target: ExternalRef::new(cache.clone()),
            },
            ExecutableSpec::External {
                target: ExternalRef::new(child_forward.clone()),
            },
        ]))];
        let mut parent_rules = vec![RuleSpec::unconditional(Some(vec![ExecutableSpec::Call {
            target: SequenceRef::new("child"),
        }]))];
        parent_rules.push(RuleSpec::unconditional(Some(parent_tail)));
        let program = ProgramSpec::new(
            vec![
                SequenceSpec::new("root", parent_rules),
                SequenceSpec::new("child", child_rules),
            ],
            Vec::new(),
        )
        .with_externals(vec![
            ExternalSpec::new(cache.clone()),
            ExternalSpec::new(child_forward.clone()),
            ExternalSpec::new(parent_forward.clone()),
        ])
        .validate()
        .expect("program");
        let cache_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == cache).then_some(*id))
            .expect("cache id");
        let child_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == child_forward).then_some(*id))
            .expect("child forward id");
        let parent_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == parent_forward).then_some(*id))
            .expect("parent forward id");
        let endpoint = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        (
            CompiledConfig {
                log_level: LogLevel::Error,
                forward: Some(ForwardConfig {
                    tag: child_forward.clone(),
                    upstream_tag: None,
                    endpoint,
                    executable: child_id,
                }),
                forwards: vec![
                    ForwardConfig {
                        tag: child_forward.clone(),
                        upstream_tag: None,
                        endpoint,
                        executable: child_id,
                    },
                    ForwardConfig {
                        tag: parent_forward.clone(),
                        upstream_tag: None,
                        endpoint,
                        executable: parent_id,
                    },
                ],
                forward_definitions: Vec::new(),
                forward_invocations: Vec::new(),
                cache: Some(CachePluginConfig {
                    tag: cache,
                    executable: cache_id,
                    capacity: 64,
                }),
                fallbacks: Vec::new(),
                preferences: Vec::new(),
                sequence: SequenceConfig {
                    tag: "root".to_owned(),
                    sequence: program.sequence_id("root").expect("root"),
                    forward_executable: Some(child_id),
                },
                listener: ListenerConfig {
                    tag: "listener".to_owned(),
                    kind: ListenerKind::Udp,
                    entry: "root".to_owned(),
                    listen: "127.0.0.1:1".parse().expect("listen"),
                    enable_audit: false,
                    idle_timeout: None,
                },
                api: None,
                domain_sets: Vec::new(),
                program,
            },
            child_id,
            parent_id,
        )
    }

    /// Answers each executable with a distinct address so a stored value can
    /// be attributed to the leg that produced it.
    struct PerExecutableExchange {
        calls: Rc<RefCell<Vec<ExecutableId>>>,
        answers: Vec<(ExecutableId, [u8; 4])>,
    }

    impl ExchangeExecutor for PerExecutableExchange {
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
            let address = self
                .answers
                .iter()
                .find_map(|(id, address)| (*id == executable).then_some(*address))
                .expect("every dispatched executable has a configured answer");
            let response = response_with_ip(query, address);
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

    #[test]
    fn a_child_cache_keeps_its_successor_result_when_the_parent_overwrites_the_response() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        // The parent overwrites the child's response with a second forward.
        let (config, child_id, parent_id) =
            cache_child_then_parent_config(vec![ExecutableSpec::External {
                target: ExternalRef::new("parent_forward"),
            }]);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = PerExecutableExchange {
            calls: Rc::clone(&calls),
            answers: vec![(child_id, [192, 0, 2, 31]), (parent_id, [192, 0, 2, 41])],
        };
        let first_request = query(21);
        let first = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &first_request,
            &executor,
        );
        assert_eq!(calls.borrow().as_slice(), &[child_id, parent_id]);
        // The caller's final response is the parent's overwrite.
        assert!(
            matches!(
                &first.response,
                ResponseState::Dns { source: ResponseSource::Upstream(upstream), .. }
                    if upstream == "parent_forward"
            ),
            "{:?}",
            first.response
        );
        let stored = cache
            .lookup(&first_request)
            .expect("lookup")
            .expect("the child successor result must be cached");
        let stored_addresses = mosdns_dns_core::observe_answer_addresses(&stored).expect("stored");
        assert_eq!(
            stored_addresses,
            vec![std::net::IpAddr::V4("192.0.2.31".parse().expect("address"))],
            "the cache must hold the child's own successor response, not the parent's overwrite"
        );

        // A second query is served from that child result without any upstream.
        let before = calls.borrow().len();
        let second_request = query(22);
        let second = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &second_request,
            &executor,
        );
        // The parent still overwrites, so the wire keeps the parent address,
        // but the child chain never runs again.
        assert!(
            matches!(
                &second.response,
                ResponseState::Dns { source: ResponseSource::Upstream(upstream), .. }
                    if upstream == "parent_forward"
            ),
            "{:?}",
            second.response
        );
        assert_eq!(
            calls.borrow().len(),
            before + 1,
            "a cache hit must skip the child forward and run only the parent's"
        );
    }

    #[test]
    fn reverse_qname_and_parent_replacement_do_not_publish_stale_route_provenance() {
        let reverse_config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - matches: "!qname $rules"
        exec: $forward
  - tag: rules
    type: domain_set
    args:
      exps: ["full:matched.test"]
  - tag: forward
    type: forward
    args: { upstreams: [ { tag: peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53053", enable_audit: true }
"#,
        )
        .expect("reverse qname config");
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100)).expect("cache");
        let reverse_query = query_name(92, "other.test.");
        let reverse_result = execute_observed(
            &reverse_config,
            &cache,
            &HostOptions::default(),
            &reverse_query,
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: response(&reverse_query),
                fail: false,
            },
        );
        assert_eq!(
            reverse_result.matched_rule_source.as_deref(),
            Some("inline:entry#0")
        );
        assert_eq!(reverse_result.domain_set, None);
        assert_eq!(reverse_result.effective_tag, None);

        let inline_config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - matches: qname inline.test.
        exec: $forward
  - tag: forward
    type: forward
    args: { upstreams: [ { tag: peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53056", enable_audit: true }
"#,
        )
        .expect("inline qname config");
        let inline_query = query_name(95, "inline.test.");
        let inline_result = execute_observed(
            &inline_config,
            &cache,
            &HostOptions::default(),
            &inline_query,
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: response(&inline_query),
                fail: false,
            },
        );
        assert_eq!(
            inline_result.matched_rule_source.as_deref(),
            Some("inline:entry#0")
        );

        let default_config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: $forward
  - tag: forward
    type: forward
    args: { upstreams: [ { tag: peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53057", enable_audit: true }
"#,
        )
        .expect("default route config");
        let default_query = query_name(96, "default.test.");
        let default_result = execute_observed(
            &default_config,
            &cache,
            &HostOptions::default(),
            &default_query,
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: response(&default_query),
                fail: false,
            },
        );
        assert_eq!(default_result.matched_rule_source, None);
        assert_eq!(
            default_result.effective_tag.as_deref(),
            Some("unmatched_rule")
        );

        let local_default_config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: reject 3
  - tag: unused_forward
    type: forward
    args: { upstreams: [ { tag: unused_peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53058", enable_audit: true }
"#,
        )
        .expect("local default route config");
        let local_default_result = execute_observed(
            &local_default_config,
            &cache,
            &HostOptions::default(),
            &query_name(97, "local-default.test."),
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: Vec::new(),
                fail: false,
            },
        );
        assert_eq!(local_default_result.selected_upstream, None);
        assert_eq!(
            local_default_result.effective_tag.as_deref(),
            Some("unmatched_rule")
        );

        let configured_default = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: flow_setter upstream=configured_default_upstream
      - exec: reject 3
  - tag: unused_forward
    type: forward
    args: { upstreams: [ { tag: unused_peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53059", enable_audit: true }
"#,
        )
        .expect("configured matcher-less default route config");
        let configured_default_result = execute_observed(
            &configured_default,
            &cache,
            &HostOptions::default(),
            &query_name(98, "configured-local-default.test."),
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: Vec::new(),
                fail: false,
            },
        );
        assert_eq!(configured_default_result.selected_upstream, None);
        assert_eq!(
            configured_default_result.final_upstream.as_deref(),
            Some("configured_default_upstream")
        );
        assert_eq!(
            configured_default_result.effective_tag.as_deref(),
            Some("unmatched_rule")
        );

        let parent_config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: $child
      - exec: reject 3
  - tag: child
    type: sequence
    args:
      - matches: qname $child_rules
        exec: $forward
  - tag: child_rules
    type: domain_set
    args:
      exps: ["full:child.test"]
  - tag: forward
    type: forward
    args: { upstreams: [ { tag: child_peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53054", enable_audit: true }
"#,
        )
        .expect("parent replacement config");
        let parent_query = query_name(93, "child.test.");
        let parent_result = execute_observed(
            &parent_config,
            &cache,
            &HostOptions::default(),
            &parent_query,
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: response(&parent_query),
                fail: false,
            },
        );
        assert_eq!(parent_result.response_wire[3] & 0x0f, 3);
        assert_eq!(parent_result.domain_set, None);
        assert_eq!(parent_result.matched_rule_source, None);
        assert_eq!(
            parent_result.effective_tag.as_deref(),
            Some("unmatched_rule")
        );
        assert_eq!(parent_result.selected_upstream, None);
    }

    #[test]
    fn byte_identical_parent_response_replacement_drops_child_provenance() {
        let config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: $child
      - exec: $parent_forward
  - tag: child
    type: sequence
    args:
      - matches: qname $child_rules
        exec: $child_forward
  - tag: child_rules
    type: domain_set
    args:
      exps: ["full:child.test"]
  - tag: child_forward
    type: forward
    args: { upstreams: [ { tag: child_peer, addr: "udp://127.0.0.1:1" } ] }
  - tag: parent_forward
    type: forward
    args: { upstreams: [ { tag: parent_peer, addr: "udp://127.0.0.1:2" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:53055", enable_audit: true }
"#,
        )
        .expect("byte-identical parent replacement config");
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100)).expect("cache");
        let request = query_name(94, "child.test.");
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &request,
            &MockExchange {
                calls: Rc::new(Cell::new(0)),
                response: response(&request),
                fail: false,
            },
        );
        assert_eq!(result.domain_set, None);
        assert_eq!(result.matched_rule_source, None);
        assert_eq!(result.selected_upstream, Some("127.0.0.1:2".to_owned()));
    }

    #[test]
    fn an_entry_cache_wraps_its_successor_and_caches_that_successor_result() {
        // The cache sits directly in the entry sequence ahead of the forward:
        // its successor is the forward, and that is what must be stored.
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let config = config();
        let request = query(23);
        let calls = Rc::new(Cell::new(0));
        let mock = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
            fail: false,
        };
        let first = execute_observed(&config, &cache, &HostOptions::default(), &request, &mock);
        assert_eq!(first.cache_status, CacheStatus::Miss);
        assert_eq!(calls.get(), 1);
        assert!(
            cache.lookup(&request).expect("lookup").is_some(),
            "an entry cache must store its successor's response"
        );
        let second_request = query(24);
        let second = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &second_request,
            &mock,
        );
        assert_eq!(second.cache_status, CacheStatus::Hit);
        assert_eq!(calls.get(), 1, "the warm hit must not dispatch upstream");
    }

    /// The representative chain, compiled from the same text as
    /// `tests/slice3_composition.rs`, so the routing decisions can be checked
    /// in-process without a listener.
    fn representative_chain(dir: &std::path::Path) -> CompiledConfig {
        std::fs::create_dir_all(dir.join("sub_config")).expect("fixture sub_config");
        std::fs::write(
            dir.join("sub_config/rules.txt"),
            "domain:local.test\nfull:local.only.test\n",
        )
        .expect("fixture rules");
        std::fs::write(
            dir.join("sub_config/routes.yaml"),
            format!(
                r#"
plugins:
  - tag: sequence_routed
    type: sequence
    args:
      - matches: qname $local_domains
        exec: $sequence_local
  - tag: sequence_local
    type: sequence
    args:
      - exec: $cache_main
      - exec: $local_forward
  - tag: sequence_default
    type: sequence
    args:
      - exec: $default_forward
  - tag: cache_main
    type: cache
    args:
      size: 64
      lazy_cache_ttl: 0
  - tag: local_forward
    type: forward
    args:
      upstreams:
        - tag: local_peer
          addr: "udp://127.0.0.1:26361"
  - tag: default_forward
    type: forward
    args:
      upstreams:
        - tag: default_peer
          addr: "udp://127.0.0.1:26362"
  - tag: blocked
    type: domain_set
    args:
      exps:
        - full:blocked.test
        - full:another-blocked.test
  - tag: local_domains
    type: domain_set
    args:
      files:
        - "{}"
"#,
                dir.join("sub_config/rules.txt").display()
            ),
        )
        .expect("fixture routes");
        std::fs::write(
            dir.join("config.yaml"),
            r#"
log:
  level: error
include:
  - sub_config/routes.yaml
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qtype 65
        exec: reject 0
      - matches: qname $blocked
        exec: reject 3
      - exec: $sequence_routed
      - matches: has_resp
        exec: accept
      - exec: $sequence_default
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:26353"
      enable_audit: true
"#,
        )
        .expect("fixture config");
        compile_yaml_with_base(
            &std::fs::read_to_string(dir.join("config.yaml")).expect("config"),
            dir,
        )
        .expect("representative chain must compile")
    }

    #[test]
    fn the_representative_chain_blocks_rejects_and_routes_without_a_listener() {
        let dir = std::env::temp_dir().join(format!("phase5b-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = representative_chain(&dir);
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100)).expect("cache");
        let local = config
            .forwards
            .iter()
            .find(|forward| forward.tag == "local_forward")
            .expect("local forward")
            .executable;
        let default = config
            .forwards
            .iter()
            .find(|forward| forward.tag == "default_forward")
            .expect("default forward")
            .executable;
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = RecordingNamedExchange {
            calls: Rc::clone(&calls),
            local,
            default,
        };

        // A blocked name is refused locally without any upstream call.
        let blocked = query_name(31, "blocked.test.");
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &blocked,
            &executor,
        );
        assert_eq!(
            result.response_wire[3] & 0x0f,
            3,
            "blocked.test is NXDOMAIN"
        );
        assert!(result.upstream_attempts.is_empty());
        assert!(calls.borrow().is_empty());

        // A qtype-65 query is answered with RCODE 0 locally.
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_type(33, "other.test.", 65),
            &executor,
        );
        assert_eq!(result.response_wire[3] & 0x0f, 0, "qtype 65 rejects with 0");
        assert!(calls.borrow().is_empty(), "no upstream for a qtype reject");

        // A local-suffix name goes through the child call to the local peer
        // and is cached by the child's own cache.
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(34, "a.local.test."),
            &executor,
        );
        assert_eq!(result.response_wire[3] & 0x0f, 0);
        assert_eq!(calls.borrow().as_slice(), &["local_forward"]);
        assert_eq!(result.cache_status, CacheStatus::Miss);
        assert_eq!(result.final_sequence.as_deref(), Some("sequence_main"));

        // A routed miss falls through the parent to the default peer.
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &query_name(35, "unmatched.test."),
            &executor,
        );
        assert_eq!(result.response_wire[3] & 0x0f, 0);
        assert_eq!(
            calls.borrow().as_slice(),
            &["local_forward", "default_forward"]
        );
        assert!(
            matches!(
                &result.response,
                ResponseState::Dns { source: ResponseSource::Upstream(upstream), .. }
                    if upstream == "default_peer"
            ),
            "{:?}",
            result.response
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct RecordingNamedExchange {
        calls: Rc<RefCell<Vec<&'static str>>>,
        local: ExecutableId,
        default: ExecutableId,
    }

    impl ExchangeExecutor for RecordingNamedExchange {
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
            let (name, address) = if executable == self.local {
                ("local_forward", [192, 0, 2, 21])
            } else if executable == self.default {
                ("default_forward", [192, 0, 2, 22])
            } else {
                panic!("unexpected executable {executable:?}");
            };
            self.calls.borrow_mut().push(name);
            let response = response_with_ip(query, address);
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

    #[test]
    fn a_cancelled_query_never_publishes_a_partial_child_result() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let (config, child_id, _parent_id) = cache_child_then_parent_config(Vec::new());
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = PerExecutableExchange {
            calls: Rc::clone(&calls),
            answers: vec![(child_id, [192, 0, 2, 31])],
        };
        let request = query(25);
        let (header, question) = parse_query(&request).expect("query");
        let cancellation = TransportCancellation::new();
        cancellation.cancel();
        let result = futures_like_block_on(execute_request_with_observation(
            super::ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &HostOptions::default(),
                raw: &request,
                header,
                question,
            },
            &executor,
            cancellation,
            &mut ExecutionCheckpoint::new(true),
        ));
        assert!(
            result.response_wire.is_empty(),
            "cancellation must not publish"
        );
        assert!(
            cache.is_empty(),
            "a cancelled query must not leave a cached child result"
        );
    }

    #[test]
    fn a_second_dynamic_cache_access_fails_closed_without_overwriting_the_token() {
        // A query that reaches the same cache twice cannot hand it two tokens.
        // The second visit is a controlled failure, and the first token must
        // not publish as if the second visit had succeeded.
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let forward = "forward".to_owned();
        let cache_tag = "cache".to_owned();
        // The cache is reached on both a first rule and, after a miss, the
        // forward, then a second rule that reaches it again.
        let program = ProgramSpec::new(
            vec![SequenceSpec::new(
                "root",
                vec![RuleSpec::unconditional(Some(vec![
                    ExecutableSpec::External {
                        target: ExternalRef::new(cache_tag.clone()),
                    },
                    ExecutableSpec::External {
                        target: ExternalRef::new(forward.clone()),
                    },
                    ExecutableSpec::External {
                        target: ExternalRef::new(cache_tag.clone()),
                    },
                ]))],
            )],
            Vec::new(),
        )
        .with_externals(vec![
            ExternalSpec::new(cache_tag.clone()),
            ExternalSpec::new(forward.clone()),
        ])
        .validate()
        .expect("program");
        let cache_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == cache_tag).then_some(*id))
            .expect("cache id");
        let forward_id = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == forward).then_some(*id))
            .expect("forward id");
        let endpoint = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        let config = CompiledConfig {
            log_level: LogLevel::Error,
            forward: Some(ForwardConfig {
                tag: forward.clone(),
                upstream_tag: None,
                endpoint,
                executable: forward_id,
            }),
            forwards: vec![ForwardConfig {
                tag: forward,
                upstream_tag: None,
                endpoint,
                executable: forward_id,
            }],
            forward_definitions: Vec::new(),
            forward_invocations: Vec::new(),
            cache: Some(CachePluginConfig {
                tag: cache_tag,
                executable: cache_id,
                capacity: 64,
            }),
            fallbacks: Vec::new(),
            preferences: Vec::new(),
            sequence: SequenceConfig {
                tag: "root".to_owned(),
                sequence: program.sequence_id("root").expect("root"),
                forward_executable: Some(forward_id),
            },
            listener: ListenerConfig {
                tag: "listener".to_owned(),
                kind: ListenerKind::Udp,
                entry: "root".to_owned(),
                listen: "127.0.0.1:1".parse().expect("listen"),
                enable_audit: false,
                idle_timeout: None,
            },
            api: None,
            domain_sets: Vec::new(),
            program,
        };
        let request = query(26);
        let calls = Rc::new(Cell::new(0));
        let mock = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
            fail: false,
        };
        let execution = execute_observed(&config, &cache, &HostOptions::default(), &request, &mock);
        assert_eq!(
            execution.response_wire[3] & 0x0f,
            super::SERVFAIL,
            "a repeated dynamic cache access must fail closed"
        );
        assert_eq!(
            execution.failure_provenance,
            Some(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution
            ))
        );
        assert_eq!(calls.get(), 1, "the first miss still reaches its forward");
        assert!(cache.is_empty(), "no token may publish after the failure");
    }

    #[test]
    fn a_second_cache_access_after_a_hit_fails_closed() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let (prime_config, _child_id, _parent_id) = cache_child_then_parent_config(Vec::new());
        let (repeat_config, _child_id, _parent_id) =
            cache_child_then_parent_config(vec![ExecutableSpec::Call {
                target: SequenceRef::new("child"),
            }]);
        let request = query(27);
        let prime_calls = Rc::new(Cell::new(0));
        let prime_exchange = MockExchange {
            calls: Rc::clone(&prime_calls),
            response: response(&request),
            fail: false,
        };
        let primed = execute_observed(
            &prime_config,
            &cache,
            &HostOptions::default(),
            &request,
            &prime_exchange,
        );
        assert_eq!(primed.cache_status, CacheStatus::Miss);
        assert_eq!(prime_calls.get(), 1);
        assert!(cache.lookup(&request).expect("lookup").is_some());

        let calls = Rc::new(Cell::new(0));
        let exchange = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
            fail: false,
        };
        let repeated = execute_observed(
            &repeat_config,
            &cache,
            &HostOptions::default(),
            &request,
            &exchange,
        );
        assert_eq!(repeated.response_wire[3] & 0x0f, super::SERVFAIL);
        assert_eq!(repeated.cache_status, CacheStatus::Hit);
        assert_eq!(
            repeated.failure_provenance,
            Some(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution
            ))
        );
        assert_eq!(calls.get(), 0, "a hit and repeat must skip the forward");
    }

    #[test]
    fn a_second_cache_access_after_miss_publication_fails_closed() {
        let clock = CacheTestClock::new(100);
        let cache = NativeCacheAdapter::for_test(clock).expect("cache");
        let (config, _child_id, _parent_id) =
            cache_child_then_parent_config(vec![ExecutableSpec::Call {
                target: SequenceRef::new("child"),
            }]);
        let request = query(28);
        let calls = Rc::new(Cell::new(0));
        let exchange = MockExchange {
            calls: Rc::clone(&calls),
            response: response(&request),
            fail: false,
        };
        let repeated = execute_observed(
            &config,
            &cache,
            &HostOptions::default(),
            &request,
            &exchange,
        );
        assert_eq!(repeated.response_wire[3] & 0x0f, super::SERVFAIL);
        assert_eq!(repeated.cache_status, CacheStatus::Miss);
        assert_eq!(
            repeated.failure_provenance,
            Some(FailureProvenance::LocalFailure(
                LocalFailureKind::InternalExecution
            ))
        );
        assert_eq!(calls.get(), 1, "only the first miss reaches its forward");
        assert!(
            cache.lookup(&request).expect("lookup").is_some(),
            "the first successor had already published before the repeated dispatch"
        );
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

    #[test]
    fn effective_tag_normalization_keeps_special_and_memory_precedence() {
        assert_eq!(
            compute_effective_tag(
                "记忆无V6|订阅直连",
                Some("foreign"),
                None,
                Some("sequence_google")
            ),
            "记忆无V6|订阅直连"
        );
        assert_eq!(
            compute_effective_tag(
                "订阅直连",
                Some("foreign"),
                None,
                Some("sequence_fakeip_addlist")
            ),
            "直连候选转代理"
        );
        assert_eq!(
            compute_effective_tag("anything", Some("special_upstream_7"), None, None),
            "特殊上游7"
        );
        assert_eq!(
            compute_effective_tag("unmatched_rule", Some("foreign"), None, None),
            "unmatched_rule"
        );
        assert_eq!(
            compute_effective_tag("foo|foo|bar|foo", None, None, None),
            "foo|bar"
        );
    }

    #[test]
    fn fallback_zero_starts_both_branches_and_reports_schema_two() {
        let config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args: [ { exec: "$fallback" } ]
  - tag: primary
    type: sequence
    args: [ { exec: "$primary_forward" } ]
  - tag: secondary
    type: sequence
    args: [ { exec: "$secondary_forward" } ]
  - tag: fallback
    type: fallback
    args: { primary: "$primary", secondary: "$secondary", threshold: 0 }
  - tag: primary_forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
  - tag: secondary_forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15454" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: true }
"#,
        )
        .expect("fallback config");
        let primary_definition = config
            .forward_definitions
            .iter()
            .position(|definition| definition.tag == "primary_forward")
            .expect("primary definition");
        let secondary_definition = config
            .forward_definitions
            .iter()
            .position(|definition| definition.tag == "secondary_forward")
            .expect("secondary definition");
        let primary = config
            .forward_invocations
            .iter()
            .find(|invocation| invocation.definition == primary_definition)
            .expect("primary invocation")
            .executable;
        let secondary = config
            .forward_invocations
            .iter()
            .find(|invocation| invocation.definition == secondary_definition)
            .expect("secondary invocation")
            .executable;
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = PolicyExchange {
            calls: Rc::clone(&calls),
            primary,
            secondary,
            primary_delay: Duration::from_millis(20),
            secondary_delay: Duration::from_millis(5),
        };
        let request = query_name(401, "fallback.test");
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::with_deadline(Duration::from_secs(2)),
            &request,
            &executor,
        );
        validate_response(&result.response_wire).expect("fallback response");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 2, "threshold zero must start both forwards");
        assert_eq!(calls[0].0, primary, "threshold ties poll primary first");
        assert_eq!(
            result.upstream_attempts.metric_attempts().len(),
            2,
            "schema-2 tracing must retain canonical upstream metrics"
        );
        let diagnostics = result.upstream_diagnostics.expect("schema-2 diagnostics");
        assert_eq!(diagnostics.schema_version, 2);
        assert!(
            diagnostics
                .branches
                .iter()
                .any(|branch| branch.role == "primary")
        );
        assert!(
            diagnostics
                .branches
                .iter()
                .any(|branch| branch.role == "secondary")
        );
        assert!(
            diagnostics
                .attempts
                .iter()
                .all(|attempt| attempt.branch_id.is_some())
        );
    }

    #[test]
    fn prefer_ipv4_rewrites_reference_qtype_and_suppresses_original_answer() {
        let config = compile_yaml(
            r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args: [ { exec: prefer_ipv4 }, { exec: "$forward" } ]
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: true }
"#,
        )
        .expect("preference config");
        let forward = config.forward_invocations[0].executable;
        let calls = Rc::new(RefCell::new(Vec::new()));
        let executor = PolicyExchange {
            calls: Rc::clone(&calls),
            primary: forward,
            secondary: forward,
            primary_delay: Duration::ZERO,
            secondary_delay: Duration::ZERO,
        };
        let request = query_type(402, "preference.test", 28);
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).expect("cache");
        let result = execute_observed(
            &config,
            &cache,
            &HostOptions::with_deadline(Duration::from_secs(2)),
            &request,
            &executor,
        );
        let details = diagnose_response_wire(&result.response_wire);
        assert_eq!(details.rcode, 0);
        assert!(
            details.answers.is_empty(),
            "preferred evidence suppresses AAAA wire"
        );
        let calls = calls.borrow();
        assert!(calls.iter().any(|(_, qtype)| *qtype == 28));
        assert!(calls.iter().any(|(_, qtype)| *qtype == 1));
        let diagnostics = result.upstream_diagnostics.expect("schema-2 diagnostics");
        assert_eq!(diagnostics.schema_version, 2);
        assert!(
            diagnostics
                .branches
                .iter()
                .any(|branch| branch.decision == "suppressed")
        );
        assert!(
            diagnostics.selected.is_none(),
            "probe must not be selected supplier"
        );
    }

    #[test]
    fn policy_root_terminals_are_not_recoverable_executor_errors() {
        assert!(matches!(
            policy_failure_for_core(&ExecutionError::Cancelled),
            Err(ExecutionError::Cancelled)
        ));
        assert!(matches!(
            policy_failure_for_core(&ExecutionError::BudgetExceeded),
            Err(ExecutionError::BudgetExceeded)
        ));
        assert!(matches!(
            policy_failure_for_core(&ExecutionError::Executor(ExecutorError::new(
                "ordinary policy failure",
            ))),
            Ok(ExecutorError::Failed(message)) if message == "ordinary policy failure"
        ));
    }

    fn futures_like_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }
}
