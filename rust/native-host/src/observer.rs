use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::{Duration, SystemTime};

use mosdns_upstream_core::TransportCancellation;

const DURATION_BUCKET_UPPER_BOUNDS_MICROS: [u64; 15] = [
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000,
    1_000_000, 2_500_000,
];
const DURATION_HISTOGRAM_BUCKET_COUNT: usize = DURATION_BUCKET_UPPER_BOUNDS_MICROS.len() + 1;
const INITIAL_AUDIT_RECORD_CAPACITY: usize = 1_024;

/// The lifecycle result of an admitted query at the existing transport boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryTerminalOutcome {
    /// The transport operation completed successfully; client delivery is not implied.
    SendSucceeded,
    /// A non-cancellation transport error prevented successful send completion.
    SendFailed,
    /// Cancellation won before successful send completion.
    Canceled,
    /// Execution ended without a DNS response and without cancellation.
    NoResponse,
}

/// The listener transport that admitted a query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryTransport {
    /// A UDP datagram listener.
    Udp,
    /// A TCP stream listener.
    Tcp,
}

/// Cache disposition for one admitted query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheStatus {
    /// Execution ended before cache disposition was established.
    Undetermined,
    /// The graph does not contain a cache dispatch.
    NotApplicable,
    /// The cache supplied the final response.
    Hit,
    /// The cache was consulted and execution continued to a forward.
    Miss,
}

/// Origin of a formed DNS response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseSource {
    /// The final response was supplied by the cache.
    Cache,
    /// The native host formed the response locally.
    Local,
    /// The named configured upstream supplied the final response.
    Upstream(String),
}

/// Whether execution formed a DNS response, with its code and source when present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseState {
    /// A response was formed; this does not imply that the client received it.
    Dns { rcode: u16, source: ResponseSource },
    /// Execution ended without forming a DNS response.
    NoResponse,
}

/// Result of one upstream network attempt, in execution order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamAttemptOutcome {
    /// The upstream supplied a DNS response.
    Response,
    /// The attempt expired at its upstream deadline.
    TimedOut,
    /// The attempt failed without a DNS response.
    Failed,
    /// The request was canceled while the attempt was still in progress.
    Canceled,
    /// The request was dropped while the attempt was still in progress.
    Interrupted,
}

/// One actual upstream attempt made by the request execution path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpstreamAttemptRecord {
    /// Configured upstream identity; never a query or client value.
    pub upstream: String,
    /// Outcome observed for this attempt.
    pub outcome: UpstreamAttemptOutcome,
}

/// Classification for a local failure that produced or prevented a response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalFailureKind {
    /// No upstream leg produced a usable response.
    NoUsableUpstreamResponse,
    /// The native host could not form a response from execution results.
    ResponseConstruction,
    /// Execution stopped because of an internal host error.
    InternalExecution,
}

/// Request-level provenance for a final upstream or local failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureProvenance {
    /// An upstream leg timed out.
    UpstreamTimeout { upstream: String },
    /// An upstream leg failed without timing out.
    UpstreamFailure { upstream: String },
    /// The native host failed locally.
    LocalFailure(LocalFailureKind),
}

/// One bounded typed record retained when detailed audit capture is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    /// Wall-clock time at query admission.
    pub timestamp: SystemTime,
    /// Client socket address observed by the listener.
    pub client_addr: SocketAddr,
    /// UDP or TCP listener that admitted the query.
    pub transport: QueryTransport,
    /// Parsed question name.
    pub qname: String,
    /// Parsed question type.
    pub qtype: u16,
    /// Parsed question class.
    pub qclass: u16,
    /// Monotonic elapsed time from admission through terminalization.
    pub elapsed: Duration,
    /// Mutually exclusive lifecycle terminal outcome.
    pub terminal_outcome: QueryTerminalOutcome,
    /// Final response state and source, independent from transport outcome.
    pub response: ResponseState,
    /// Cache result for this query.
    pub cache_status: CacheStatus,
    /// Final executed sequence tag, if established.
    pub final_sequence: Option<String>,
    /// Upstream that supplied the final response, if any.
    pub final_upstream: Option<String>,
    /// Actual upstream attempts in execution order.
    pub upstream_attempts: Vec<UpstreamAttemptRecord>,
    /// Distinct failure provenance when execution establishes one.
    pub failure_provenance: Option<FailureProvenance>,
}

/// Cumulative histogram bucket; `None` is the positive-infinity bucket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurationHistogramBucket {
    /// Inclusive upper bound in microseconds, or `None` for positive infinity.
    pub upper_bound_micros: Option<u64>,
    /// Number of observations less than or equal to this bucket's bound.
    pub cumulative_count: u64,
}

/// Fixed-bucket cumulative latency distribution for completed admitted queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurationHistogramSnapshot {
    /// Finite bounds followed by a positive-infinity bucket.
    pub buckets: Vec<DurationHistogramBucket>,
    /// Number of recorded admitted-query observations.
    pub count: u64,
    /// Sum of observed microseconds, retained for later 5C snapshot consumers.
    pub sum_micros: u128,
}

impl Default for DurationHistogramSnapshot {
    fn default() -> Self {
        let mut buckets = DURATION_BUCKET_UPPER_BOUNDS_MICROS
            .iter()
            .map(|upper_bound_micros| DurationHistogramBucket {
                upper_bound_micros: Some(*upper_bound_micros),
                cumulative_count: 0,
            })
            .collect::<Vec<_>>();
        buckets.push(DurationHistogramBucket {
            upper_bound_micros: None,
            cumulative_count: 0,
        });
        Self {
            buckets,
            count: 0,
            sum_micros: 0,
        }
    }
}

/// Non-cumulative counters used on the per-query hot path. The public
/// cumulative view is constructed only when a metrics snapshot is requested.
#[derive(Clone, Debug, Default)]
struct DurationHistogramState {
    bucket_counts: [u64; DURATION_HISTOGRAM_BUCKET_COUNT],
    count: u64,
    sum_micros: u128,
}

impl DurationHistogramState {
    fn observe(&mut self, elapsed: Duration) {
        let micros = elapsed.as_micros();
        let bucket_index = DURATION_BUCKET_UPPER_BOUNDS_MICROS
            .partition_point(|upper_bound| micros > u128::from(*upper_bound));
        self.bucket_counts[bucket_index] = self.bucket_counts[bucket_index].saturating_add(1);
        self.count = self.count.saturating_add(1);
        self.sum_micros = self.sum_micros.saturating_add(micros);
    }

    fn snapshot(&self) -> DurationHistogramSnapshot {
        let mut cumulative_count = 0_u64;
        let mut buckets = Vec::with_capacity(DURATION_HISTOGRAM_BUCKET_COUNT);
        for (index, upper_bound_micros) in DURATION_BUCKET_UPPER_BOUNDS_MICROS.iter().enumerate() {
            cumulative_count = cumulative_count.saturating_add(self.bucket_counts[index]);
            buckets.push(DurationHistogramBucket {
                upper_bound_micros: Some(*upper_bound_micros),
                cumulative_count,
            });
        }
        cumulative_count = cumulative_count
            .saturating_add(self.bucket_counts[DURATION_HISTOGRAM_BUCKET_COUNT - 1]);
        buckets.push(DurationHistogramBucket {
            upper_bound_micros: None,
            cumulative_count,
        });
        DurationHistogramSnapshot {
            buckets,
            count: self.count,
            sum_micros: self.sum_micros,
        }
    }
}

/// Per-configured-upstream network attempt outcomes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpstreamAttemptMetricsSnapshot {
    /// Actual network attempts made for this upstream.
    pub attempts_total: u64,
    /// Attempts that produced a DNS response.
    pub responses_total: u64,
    /// Attempts that failed without timing out.
    pub failures_total: u64,
    /// Attempts that reached their deadline.
    pub timeouts_total: u64,
    /// Attempts interrupted by request cancellation.
    pub canceled_total: u64,
    /// Attempts interrupted by a non-cancellation drop or unwind.
    pub interrupted_total: u64,
}

/// Read-only point-in-time copy of the host's basic metrics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricsSnapshot {
    /// Queries parsed and admitted by a listener.
    pub admitted_total: u64,
    /// Admitted queries that reached one lifecycle terminal outcome.
    pub completed_total: u64,
    /// Admitted queries that have not yet reached a terminal outcome.
    pub in_flight: u64,
    /// Malformed listener input excluded from admitted and completed totals.
    pub malformed_total: u64,
    /// Queries whose transport send completed successfully.
    pub send_succeeded_total: u64,
    /// Queries whose non-cancellation transport send failed.
    pub send_failed_total: u64,
    /// Queries where cancellation won before successful send completion.
    pub canceled_total: u64,
    /// Queries that ended without a response and without cancellation.
    pub no_response_total: u64,
    /// Formed DNS response totals by response code.
    pub response_code_totals: BTreeMap<u16, u64>,
    /// Cache hit queries.
    pub cache_hits_total: u64,
    /// Cache miss queries.
    pub cache_misses_total: u64,
    /// Queries whose graph has no cache dispatch.
    pub cache_not_applicable_total: u64,
    /// Queries terminalized before execution established cache disposition.
    pub cache_undetermined_total: u64,
    /// Forward-attempt outcomes keyed only by configured upstream identity.
    pub forward_attempts_by_upstream: BTreeMap<String, UpstreamAttemptMetricsSnapshot>,
    /// Forward attempts whose identity was absent from the compiled catalog.
    pub unknown_forward_attempts_total: u64,
    /// Fixed cumulative end-to-end latency distribution.
    pub duration: DurationHistogramSnapshot,
}

/// Read-only snapshot of bounded detailed audit retention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditSnapshot {
    /// Retained records in oldest-to-newest order.
    pub records: Vec<AuditRecord>,
    /// Records evicted from the front of the configured retention ring.
    pub evicted_total: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
pub(crate) struct TerminalObservation {
    pub outcome: QueryTerminalOutcome,
    pub response: ResponseState,
    pub cache_status: CacheStatus,
    pub final_sequence: Option<String>,
    pub final_upstream: Option<String>,
    pub upstream_attempts: Vec<UpstreamAttemptRecord>,
    pub failure_provenance: Option<FailureProvenance>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
struct ExecutionCheckpoint {
    response: ResponseState,
    cache_status: CacheStatus,
    final_sequence: Option<String>,
    final_upstream: Option<String>,
    upstream_attempts: Vec<UpstreamAttemptRecord>,
    failure_provenance: Option<FailureProvenance>,
    in_flight_upstream: Option<String>,
}

/// Shared progress between an execution future and its listener-owned guard.
/// The execution future publishes its local facts if cancellation drops it.
#[derive(Clone, Debug)]
pub(crate) struct ExecutionProgress(Arc<Mutex<ExecutionCheckpoint>>);

impl ExecutionProgress {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(ExecutionCheckpoint {
            response: ResponseState::NoResponse,
            cache_status: CacheStatus::Undetermined,
            final_sequence: None,
            final_upstream: None,
            upstream_attempts: Vec::new(),
            failure_provenance: None,
            in_flight_upstream: None,
        })))
    }

    pub(crate) fn capture(
        &self,
        observation: &TerminalObservation,
        in_flight_upstream: Option<String>,
    ) {
        let mut checkpoint = self.lock();
        checkpoint.response = observation.response.clone();
        checkpoint.cache_status = observation.cache_status;
        checkpoint
            .final_sequence
            .clone_from(&observation.final_sequence);
        checkpoint
            .final_upstream
            .clone_from(&observation.final_upstream);
        checkpoint
            .upstream_attempts
            .clone_from(&observation.upstream_attempts);
        checkpoint
            .failure_provenance
            .clone_from(&observation.failure_provenance);
        checkpoint.in_flight_upstream = in_flight_upstream;
    }

    pub(crate) fn capture_owned(&self, observation: TerminalObservation) {
        let mut checkpoint = self.lock();
        checkpoint.response = observation.response;
        checkpoint.cache_status = observation.cache_status;
        checkpoint.final_sequence = observation.final_sequence;
        checkpoint.final_upstream = observation.final_upstream;
        checkpoint.upstream_attempts = observation.upstream_attempts;
        checkpoint.failure_provenance = observation.failure_provenance;
        checkpoint.in_flight_upstream = None;
    }

    fn terminal_observation(
        &self,
        outcome: QueryTerminalOutcome,
        canceled: bool,
        elapsed: Duration,
    ) -> TerminalObservation {
        let checkpoint = self.lock();
        let mut upstream_attempts = checkpoint.upstream_attempts.clone();
        if let Some(upstream) = &checkpoint.in_flight_upstream {
            upstream_attempts.push(UpstreamAttemptRecord {
                upstream: upstream.clone(),
                outcome: if canceled {
                    UpstreamAttemptOutcome::Canceled
                } else {
                    UpstreamAttemptOutcome::Interrupted
                },
            });
        }
        TerminalObservation {
            outcome,
            response: checkpoint.response.clone(),
            cache_status: checkpoint.cache_status,
            final_sequence: checkpoint.final_sequence.clone(),
            final_upstream: checkpoint.final_upstream.clone(),
            upstream_attempts,
            failure_provenance: checkpoint.failure_provenance.clone(),
            elapsed,
        }
    }

    fn take_terminal_observation(
        &self,
        outcome: QueryTerminalOutcome,
        elapsed: Duration,
    ) -> TerminalObservation {
        let mut checkpoint = self.lock();
        let mut upstream_attempts = std::mem::take(&mut checkpoint.upstream_attempts);
        if let Some(upstream) = checkpoint.in_flight_upstream.take() {
            upstream_attempts.push(UpstreamAttemptRecord {
                upstream,
                outcome: if outcome == QueryTerminalOutcome::Canceled {
                    UpstreamAttemptOutcome::Canceled
                } else {
                    UpstreamAttemptOutcome::Interrupted
                },
            });
        }
        TerminalObservation {
            outcome,
            response: std::mem::replace(&mut checkpoint.response, ResponseState::NoResponse),
            cache_status: checkpoint.cache_status,
            final_sequence: checkpoint.final_sequence.take(),
            final_upstream: checkpoint.final_upstream.take(),
            upstream_attempts,
            failure_provenance: checkpoint.failure_provenance.take(),
            elapsed,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ExecutionCheckpoint> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Clone, Debug, Default)]
struct MetricsState {
    admitted_total: u64,
    completed_total: u64,
    in_flight: u64,
    malformed_total: u64,
    send_succeeded_total: u64,
    send_failed_total: u64,
    canceled_total: u64,
    no_response_total: u64,
    response_code_totals: BTreeMap<u16, u64>,
    cache_hits_total: u64,
    cache_misses_total: u64,
    cache_not_applicable_total: u64,
    cache_undetermined_total: u64,
    forward_attempts_by_upstream: BTreeMap<String, UpstreamAttemptMetricsSnapshot>,
    unknown_forward_attempts_total: u64,
    duration: DurationHistogramState,
}

impl MetricsState {
    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            admitted_total: self.admitted_total,
            completed_total: self.completed_total,
            in_flight: self.in_flight,
            malformed_total: self.malformed_total,
            send_succeeded_total: self.send_succeeded_total,
            send_failed_total: self.send_failed_total,
            canceled_total: self.canceled_total,
            no_response_total: self.no_response_total,
            response_code_totals: self.response_code_totals.clone(),
            cache_hits_total: self.cache_hits_total,
            cache_misses_total: self.cache_misses_total,
            cache_not_applicable_total: self.cache_not_applicable_total,
            cache_undetermined_total: self.cache_undetermined_total,
            forward_attempts_by_upstream: self.forward_attempts_by_upstream.clone(),
            unknown_forward_attempts_total: self.unknown_forward_attempts_total,
            duration: self.duration.snapshot(),
        }
    }

    fn record_terminal(
        &mut self,
        outcome: QueryTerminalOutcome,
        response: &ResponseState,
        cache_status: CacheStatus,
        upstream_attempts: &[UpstreamAttemptRecord],
        elapsed: Duration,
    ) {
        self.completed_total = self.completed_total.saturating_add(1);
        self.in_flight = self
            .in_flight
            .checked_sub(1)
            .expect("terminal query must have a matching admission");
        match outcome {
            QueryTerminalOutcome::SendSucceeded => {
                self.send_succeeded_total = self.send_succeeded_total.saturating_add(1);
            }
            QueryTerminalOutcome::SendFailed => {
                self.send_failed_total = self.send_failed_total.saturating_add(1);
            }
            QueryTerminalOutcome::Canceled => {
                self.canceled_total = self.canceled_total.saturating_add(1);
            }
            QueryTerminalOutcome::NoResponse => {
                self.no_response_total = self.no_response_total.saturating_add(1);
            }
        }
        if let ResponseState::Dns { rcode, .. } = response {
            let total = self.response_code_totals.entry(*rcode).or_default();
            *total = total.saturating_add(1);
        }
        match cache_status {
            CacheStatus::Undetermined => {
                self.cache_undetermined_total = self.cache_undetermined_total.saturating_add(1);
            }
            CacheStatus::NotApplicable => {
                self.cache_not_applicable_total = self.cache_not_applicable_total.saturating_add(1);
            }
            CacheStatus::Hit => {
                self.cache_hits_total = self.cache_hits_total.saturating_add(1);
            }
            CacheStatus::Miss => {
                self.cache_misses_total = self.cache_misses_total.saturating_add(1);
            }
        }
        for attempt in upstream_attempts {
            if let Some(counters) = self.forward_attempts_by_upstream.get_mut(&attempt.upstream) {
                counters.attempts_total = counters.attempts_total.saturating_add(1);
                let counter = match attempt.outcome {
                    UpstreamAttemptOutcome::Response => &mut counters.responses_total,
                    UpstreamAttemptOutcome::Failed => &mut counters.failures_total,
                    UpstreamAttemptOutcome::TimedOut => &mut counters.timeouts_total,
                    UpstreamAttemptOutcome::Canceled => &mut counters.canceled_total,
                    UpstreamAttemptOutcome::Interrupted => &mut counters.interrupted_total,
                };
                *counter = counter.saturating_add(1);
            } else {
                self.unknown_forward_attempts_total =
                    self.unknown_forward_attempts_total.saturating_add(1);
            }
        }
        self.duration.observe(elapsed);
    }
}

#[derive(Default)]
struct ObserverState {
    metrics: MetricsState,
    audit_records: VecDeque<AuditRecord>,
    evicted_total: u64,
}

/// Thread-safe observer state owned by one host assembly.
#[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
pub(crate) struct QueryObserver {
    audit_enabled: bool,
    audit_capacity: NonZeroUsize,
    state: Mutex<ObserverState>,
}

impl QueryObserver {
    pub(crate) fn new(
        audit_enabled: bool,
        upstream_identities: impl IntoIterator<Item = String>,
        audit_capacity: NonZeroUsize,
    ) -> Self {
        let forward_attempts_by_upstream = upstream_identities
            .into_iter()
            .map(|identity| (identity, UpstreamAttemptMetricsSnapshot::default()))
            .collect();
        let audit_records = VecDeque::with_capacity(if audit_enabled {
            audit_capacity.get().min(INITIAL_AUDIT_RECORD_CAPACITY)
        } else {
            0
        });
        Self {
            audit_enabled,
            audit_capacity,
            state: Mutex::new(ObserverState {
                metrics: MetricsState {
                    forward_attempts_by_upstream,
                    ..MetricsState::default()
                },
                audit_records,
                ..ObserverState::default()
            }),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // Listener admission is wired in Slice 2.
    pub(crate) fn admit_query(&self) {
        let mut state = self.lock();
        state.metrics.admitted_total = state.metrics.admitted_total.saturating_add(1);
        state.metrics.in_flight = state.metrics.in_flight.saturating_add(1);
    }

    #[cfg_attr(not(test), allow(dead_code))] // Malformed-input accounting is wired in Slice 2.
    pub(crate) fn record_malformed(&self) {
        let mut state = self.lock();
        state.metrics.malformed_total = state.metrics.malformed_total.saturating_add(1);
    }

    #[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
    pub(crate) fn record_terminal(
        &self,
        observation: TerminalObservation,
        make_audit_record: impl FnOnce(TerminalObservation) -> AuditRecord,
    ) {
        if self.audit_enabled {
            let record = make_audit_record(observation);
            let mut state = self.lock();
            state.metrics.record_terminal(
                record.terminal_outcome,
                &record.response,
                record.cache_status,
                &record.upstream_attempts,
                record.elapsed,
            );
            if state.audit_records.len() == self.audit_capacity.get() {
                state.audit_records.pop_front();
                state.evicted_total = state.evicted_total.saturating_add(1);
            }
            state.audit_records.push_back(record);
        } else {
            let mut state = self.lock();
            state.metrics.record_terminal(
                observation.outcome,
                &observation.response,
                observation.cache_status,
                &observation.upstream_attempts,
                observation.elapsed,
            );
        }
    }

    pub(crate) fn admit(
        self: &std::sync::Arc<Self>,
        client_addr: SocketAddr,
        transport: QueryTransport,
        question: &mosdns_dns_core::QuestionInfo,
        cancellation: TransportCancellation,
    ) -> AdmittedQueryGuard {
        let admitted_at = Instant::now();
        let audit_context = self.audit_enabled.then(|| AuditContext {
            timestamp: SystemTime::now(),
            client_addr,
            transport,
            qname: render_qname(&question.qname_wire),
            qtype: question.qtype,
            qclass: question.qclass,
        });
        self.admit_query();
        AdmittedQueryGuard {
            observer: std::sync::Arc::clone(self),
            admitted_at,
            audit_context,
            cancellation,
            execution_progress: ExecutionProgress::new(),
            finalized: false,
        }
    }

    pub(crate) fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.lock().metrics.snapshot()
    }

    pub(crate) fn audit_snapshot(&self) -> AuditSnapshot {
        let state = self.lock();
        AuditSnapshot {
            records: state.audit_records.iter().cloned().collect(),
            evicted_total: state.evicted_total,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ObserverState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct AuditContext {
    timestamp: SystemTime,
    client_addr: SocketAddr,
    transport: QueryTransport,
    qname: String,
    qtype: u16,
    qclass: u16,
}

/// Finalizes one admitted request. Dropping an unfinished request records one
/// canceled terminal event if its cancellation scope fired, or a no-response
/// event for another unwind path.
pub(crate) struct AdmittedQueryGuard {
    observer: std::sync::Arc<QueryObserver>,
    admitted_at: Instant,
    audit_context: Option<AuditContext>,
    cancellation: TransportCancellation,
    execution_progress: ExecutionProgress,
    finalized: bool,
}

impl AdmittedQueryGuard {
    pub(crate) fn execution_progress(&self) -> ExecutionProgress {
        self.execution_progress.clone()
    }

    pub(crate) fn capture_execution(&self, observation: TerminalObservation) {
        self.execution_progress.capture_owned(observation);
    }

    pub(crate) fn finish(mut self, outcome: QueryTerminalOutcome) {
        let observation = self
            .execution_progress
            .take_terminal_observation(outcome, self.admitted_at.elapsed());
        self.record(observation);
    }

    fn record(&mut self, observation: TerminalObservation) {
        self.finalized = true;
        let audit_context = self.audit_context.take();
        self.observer.record_terminal(observation, |observation| {
            let context = audit_context.expect("audit context exists when capture is enabled");
            AuditRecord {
                timestamp: context.timestamp,
                client_addr: context.client_addr,
                transport: context.transport,
                qname: context.qname,
                qtype: context.qtype,
                qclass: context.qclass,
                elapsed: observation.elapsed,
                terminal_outcome: observation.outcome,
                response: observation.response,
                cache_status: observation.cache_status,
                final_sequence: observation.final_sequence,
                final_upstream: observation.final_upstream,
                upstream_attempts: observation.upstream_attempts,
                failure_provenance: observation.failure_provenance,
            }
        });
    }
}

impl Drop for AdmittedQueryGuard {
    fn drop(&mut self) {
        if !self.finalized {
            let canceled = self.cancellation.is_cancelled();
            self.record(self.execution_progress.terminal_observation(
                if canceled {
                    QueryTerminalOutcome::Canceled
                } else {
                    QueryTerminalOutcome::NoResponse
                },
                canceled,
                self.admitted_at.elapsed(),
            ));
        }
    }
}

fn render_qname(wire: &[u8]) -> String {
    let mut labels = Vec::new();
    let mut offset = 0;
    while offset < wire.len() {
        let length = usize::from(wire[offset]);
        offset += 1;
        if length == 0 {
            break;
        }
        let Some(label) = wire.get(offset..offset.saturating_add(length)) else {
            break;
        };
        let mut rendered = String::new();
        for byte in label {
            match *byte {
                b'.' => rendered.push_str("\\."),
                b'\\' => rendered.push_str("\\\\"),
                0x21..=0x7e => rendered.push(char::from(*byte)),
                _ => rendered.push_str(&format!("\\{byte:03}")),
            }
        }
        labels.push(rendered);
        offset += length;
    }
    if labels.is_empty() {
        ".".to_owned()
    } else {
        format!("{}.", labels.join("."))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::net::SocketAddr;
    use std::num::NonZeroUsize;
    use std::time::{Duration, SystemTime};

    use mosdns_upstream_core::TransportCancellation;

    use super::{
        AuditRecord, CacheStatus, FailureProvenance, INITIAL_AUDIT_RECORD_CAPACITY,
        LocalFailureKind, QueryObserver, QueryTerminalOutcome, QueryTransport, ResponseSource,
        ResponseState, TerminalObservation, UpstreamAttemptOutcome, UpstreamAttemptRecord,
    };

    fn observer(audit_enabled: bool, capacity: usize) -> QueryObserver {
        QueryObserver::new(
            audit_enabled,
            ["route-a".to_owned(), "route-b".to_owned()],
            NonZeroUsize::new(capacity).expect("non-zero capacity"),
        )
    }

    #[test]
    fn audit_ring_reserves_a_bounded_initial_capacity_only_when_enabled() {
        let enabled = observer(true, 100_000);
        assert_eq!(
            enabled.lock().audit_records.capacity(),
            INITIAL_AUDIT_RECORD_CAPACITY
        );

        let small = observer(true, 2);
        assert_eq!(small.lock().audit_records.capacity(), 2);

        let disabled = observer(false, 100_000);
        assert_eq!(disabled.lock().audit_records.capacity(), 0);
    }

    #[test]
    fn dropped_send_preserves_completed_facts_from_checkpoint() {
        let observer = std::sync::Arc::new(observer(true, 4));
        let cancellation = TransportCancellation::new();
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'w', b'1', 0],
            qtype: 1,
            qclass: 1,
        };
        let guard = observer.admit(
            "192.0.2.54:53000".parse().expect("client address"),
            QueryTransport::Udp,
            &question,
            cancellation.clone(),
        );
        guard.capture_execution(TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Upstream("route-a".to_owned()),
            },
            cache_status: CacheStatus::Miss,
            final_sequence: Some("w1".to_owned()),
            final_upstream: Some("route-a".to_owned()),
            upstream_attempts: vec![UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Response,
            }],
            failure_provenance: None,
            elapsed: Duration::ZERO,
        });

        cancellation.cancel();
        drop(guard);

        let audit = observer.audit_snapshot();
        assert_eq!(audit.records.len(), 1);
        assert_eq!(
            audit.records[0].terminal_outcome,
            QueryTerminalOutcome::Canceled
        );
        assert_eq!(
            audit.records[0].response,
            ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Upstream("route-a".to_owned()),
            }
        );
        assert_eq!(audit.records[0].cache_status, CacheStatus::Miss);
        assert_eq!(audit.records[0].final_sequence.as_deref(), Some("w1"));
        assert_eq!(audit.records[0].upstream_attempts.len(), 1);
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(metrics.canceled_total, 1);
        assert_eq!(metrics.send_succeeded_total, 0);
        assert_eq!(metrics.cache_misses_total, 1);
    }

    #[test]
    fn unfinished_admission_is_finalized_once_using_its_cancellation_scope() {
        let observer = std::sync::Arc::new(observer(true, 2));
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'a', b'.', b'\\', 0],
            qtype: 1,
            qclass: 1,
        };
        let guard = observer.admit(
            "192.0.2.50:53000".parse().expect("client address"),
            QueryTransport::Udp,
            &question,
            TransportCancellation::new(),
        );
        drop(guard);

        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(metrics.in_flight, 0);
        assert_eq!(metrics.no_response_total, 1);
        assert_eq!(metrics.canceled_total, 0);
        assert_eq!(metrics.duration.count, 1);
        let audit = observer.audit_snapshot();
        assert_eq!(audit.records.len(), 1);
        assert_eq!(audit.records[0].qname, "a\\.\\\\.");
        assert_eq!(audit.records[0].response, ResponseState::NoResponse);
        assert_eq!(
            audit.records[0].terminal_outcome,
            QueryTerminalOutcome::NoResponse
        );

        let cancellation = TransportCancellation::new();
        let guard = observer.admit(
            "192.0.2.51:53000".parse().expect("client address"),
            QueryTransport::Tcp,
            &question,
            cancellation.clone(),
        );
        cancellation.cancel();
        drop(guard);
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 2);
        assert_eq!(metrics.completed_total, 2);
        assert_eq!(metrics.no_response_total, 1);
        assert_eq!(metrics.canceled_total, 1);
        assert_eq!(
            observer.audit_snapshot().records[1].terminal_outcome,
            QueryTerminalOutcome::Canceled
        );
    }

    #[test]
    fn unfinished_execution_preserves_known_facts_and_marks_unresolved_facts() {
        let observer = std::sync::Arc::new(observer(true, 4));
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'w', b'2', 0],
            qtype: 1,
            qclass: 1,
        };
        let guard = observer.admit(
            "192.0.2.52:53000".parse().expect("client address"),
            QueryTransport::Udp,
            &question,
            TransportCancellation::new(),
        );
        let progress = guard.execution_progress();
        progress.capture(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::Miss,
                final_sequence: Some("entry".to_owned()),
                final_upstream: None,
                upstream_attempts: vec![UpstreamAttemptRecord {
                    upstream: "route-a".to_owned(),
                    outcome: UpstreamAttemptOutcome::Response,
                }],
                failure_provenance: Some(FailureProvenance::UpstreamFailure {
                    upstream: "route-a".to_owned(),
                }),
                elapsed: Duration::ZERO,
            },
            Some("route-b".to_owned()),
        );
        drop(guard);

        let audit = observer.audit_snapshot();
        let record = &audit.records[0];
        assert_eq!(record.cache_status, CacheStatus::Miss);
        assert_eq!(record.final_sequence.as_deref(), Some("entry"));
        assert_eq!(record.final_upstream, None);
        assert_eq!(record.upstream_attempts.len(), 2);
        assert_eq!(
            record.upstream_attempts[1],
            UpstreamAttemptRecord {
                upstream: "route-b".to_owned(),
                outcome: UpstreamAttemptOutcome::Interrupted,
            }
        );
        assert_eq!(
            record.failure_provenance,
            Some(FailureProvenance::UpstreamFailure {
                upstream: "route-a".to_owned(),
            })
        );

        let cancellation = TransportCancellation::new();
        let guard = observer.admit(
            "192.0.2.53:53000".parse().expect("client address"),
            QueryTransport::Tcp,
            &question,
            cancellation.clone(),
        );
        let progress = guard.execution_progress();
        progress.capture(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::Undetermined,
                final_sequence: Some("entry".to_owned()),
                final_upstream: None,
                upstream_attempts: Vec::new(),
                failure_provenance: None,
                elapsed: Duration::ZERO,
            },
            Some("route-a".to_owned()),
        );
        cancellation.cancel();
        drop(guard);

        let audit = observer.audit_snapshot();
        assert_eq!(audit.records[1].cache_status, CacheStatus::Undetermined);
        assert_eq!(
            audit.records[1].upstream_attempts,
            [UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Canceled,
            }]
        );
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.cache_misses_total, 1);
        assert_eq!(metrics.cache_not_applicable_total, 0);
        assert_eq!(metrics.cache_undetermined_total, 1);
        assert_eq!(
            metrics
                .forward_attempts_by_upstream
                .get("route-b")
                .expect("route-b metrics")
                .interrupted_total,
            1
        );
        assert_eq!(
            metrics
                .forward_attempts_by_upstream
                .get("route-a")
                .expect("route-a metrics")
                .canceled_total,
            1
        );
    }

    fn observation(elapsed: Duration) -> TerminalObservation {
        TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Upstream("route-a".to_owned()),
            },
            cache_status: CacheStatus::Miss,
            final_sequence: Some("entry".to_owned()),
            final_upstream: Some("route-a".to_owned()),
            upstream_attempts: vec![UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Response,
            }],
            failure_provenance: None,
            elapsed,
        }
    }

    fn audit_record(observation: TerminalObservation, qname: &str) -> AuditRecord {
        AuditRecord {
            timestamp: SystemTime::UNIX_EPOCH,
            client_addr: "192.0.2.99:53000"
                .parse::<SocketAddr>()
                .expect("client address"),
            transport: QueryTransport::Udp,
            qname: qname.to_owned(),
            qtype: 1,
            qclass: 1,
            elapsed: observation.elapsed,
            terminal_outcome: observation.outcome,
            response: observation.response,
            cache_status: observation.cache_status,
            final_sequence: Some("entry".to_owned()),
            final_upstream: Some("route-a".to_owned()),
            upstream_attempts: observation.upstream_attempts,
            failure_provenance: None,
        }
    }

    #[test]
    fn disabled_audit_keeps_basic_metrics_without_building_query_details() {
        let observer = observer(false, 2);
        observer.admit_query();
        let materialized = Cell::new(false);
        let observation = observation(Duration::from_micros(100));
        observer.record_terminal(observation, |observation| {
            materialized.set(true);
            audit_record(observation, "private.example")
        });

        assert!(
            !materialized.get(),
            "audit details are skipped before construction"
        );
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(metrics.in_flight, 0);
        assert_eq!(metrics.send_succeeded_total, 1);
        assert_eq!(metrics.cache_misses_total, 1);
        assert_eq!(metrics.response_code_totals.get(&0), Some(&1));
        let upstream = metrics
            .forward_attempts_by_upstream
            .get("route-a")
            .expect("configured upstream counter");
        assert_eq!(upstream.attempts_total, 1);
        assert_eq!(upstream.responses_total, 1);
        assert_eq!(metrics.duration.count, 1);
        assert!(observer.audit_snapshot().records.is_empty());
    }

    #[test]
    fn enabled_audit_retains_the_newest_records_and_counts_evictions() {
        let observer = observer(true, 2);
        for index in 0..3 {
            observer.admit_query();
            let observation = observation(Duration::from_micros(50));
            let qname = format!("query-{index}.example");
            observer.record_terminal(observation, |observation| audit_record(observation, &qname));
        }

        let audit = observer.audit_snapshot();
        assert_eq!(audit.evicted_total, 1);
        assert_eq!(
            audit
                .records
                .iter()
                .map(|record| record.qname.as_str())
                .collect::<Vec<_>>(),
            ["query-1.example", "query-2.example"]
        );
        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 3);
        assert_eq!(metrics.completed_total, 3);
        assert_eq!(metrics.in_flight, 0);
        assert_eq!(metrics.duration.count, metrics.completed_total);
    }

    #[test]
    fn duration_histogram_uses_inclusive_frozen_bounds_and_positive_infinity() {
        let observer = observer(false, 1);
        let bounds = [
            50_u64, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000,
            500_000, 1_000_000, 2_500_000,
        ];
        for micros in bounds {
            observer.admit_query();
            observer.record_terminal(observation(Duration::from_micros(micros)), |_| {
                unreachable!("disabled audit must not construct a record")
            });
        }
        observer.admit_query();
        observer.record_terminal(observation(Duration::from_micros(2_500_001)), |_| {
            unreachable!("disabled audit must not construct a record")
        });

        let histogram = observer.metrics_snapshot().duration;
        assert_eq!(histogram.count, 16);
        assert_eq!(histogram.buckets.len(), 16);
        for (index, bucket) in histogram.buckets[..15].iter().enumerate() {
            assert_eq!(bucket.upper_bound_micros, Some(bounds[index]));
            assert_eq!(bucket.cumulative_count, u64::try_from(index + 1).unwrap());
        }
        assert_eq!(histogram.buckets[15].upper_bound_micros, None);
        assert_eq!(histogram.buckets[15].cumulative_count, 16);
    }

    #[test]
    fn failure_provenance_has_typed_upstream_and_local_cases() {
        assert_ne!(
            FailureProvenance::UpstreamTimeout {
                upstream: "route-a".to_owned()
            },
            FailureProvenance::LocalFailure(LocalFailureKind::NoUsableUpstreamResponse)
        );
    }

    #[test]
    fn terminal_counters_partition_completion_and_keep_metric_dimensions_bounded() {
        let observer = observer(false, 2);
        let cases = [
            (
                QueryTerminalOutcome::SendSucceeded,
                ResponseState::Dns {
                    rcode: 0,
                    source: ResponseSource::Upstream("route-a".to_owned()),
                },
                CacheStatus::Hit,
                vec![],
            ),
            (
                QueryTerminalOutcome::SendFailed,
                ResponseState::Dns {
                    rcode: 2,
                    source: ResponseSource::Local,
                },
                CacheStatus::Miss,
                vec![UpstreamAttemptRecord {
                    upstream: "route-b".to_owned(),
                    outcome: UpstreamAttemptOutcome::Failed,
                }],
            ),
            (
                QueryTerminalOutcome::Canceled,
                ResponseState::NoResponse,
                CacheStatus::NotApplicable,
                vec![UpstreamAttemptRecord {
                    upstream: "route-b".to_owned(),
                    outcome: UpstreamAttemptOutcome::TimedOut,
                }],
            ),
            (
                QueryTerminalOutcome::NoResponse,
                ResponseState::NoResponse,
                CacheStatus::Miss,
                vec![UpstreamAttemptRecord {
                    upstream: "unconfigured-route".to_owned(),
                    outcome: UpstreamAttemptOutcome::Response,
                }],
            ),
        ];
        for (outcome, response, cache_status, upstream_attempts) in cases {
            observer.admit_query();
            observer.record_terminal(
                TerminalObservation {
                    outcome,
                    response,
                    cache_status,
                    final_sequence: None,
                    final_upstream: None,
                    upstream_attempts,
                    failure_provenance: None,
                    elapsed: Duration::from_micros(250),
                },
                |_| unreachable!("disabled audit must not construct a record"),
            );
        }
        observer.record_malformed();
        observer.admit_query();

        let metrics = observer.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 5);
        assert_eq!(metrics.completed_total, 4);
        assert_eq!(metrics.in_flight, 1);
        assert_eq!(metrics.malformed_total, 1);
        assert_eq!(
            metrics.send_succeeded_total
                + metrics.send_failed_total
                + metrics.canceled_total
                + metrics.no_response_total,
            metrics.completed_total
        );
        assert_eq!(metrics.response_code_totals.get(&0), Some(&1));
        assert_eq!(metrics.response_code_totals.get(&2), Some(&1));
        assert_eq!(metrics.cache_hits_total, 1);
        assert_eq!(metrics.cache_misses_total, 2);
        assert_eq!(metrics.cache_not_applicable_total, 1);
        assert_eq!(metrics.forward_attempts_by_upstream.len(), 2);
        assert_eq!(metrics.unknown_forward_attempts_total, 1);
        assert_eq!(metrics.duration.count, metrics.completed_total);
    }

    #[test]
    fn observer_is_send_and_sync_for_a_future_multicore_host() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<QueryObserver>();
    }
}
