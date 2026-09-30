use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::ops::Index;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::{Duration, SystemTime};

use mosdns_upstream_core::TransportCancellation;

/// One final-wire Answer record retained for diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditAnswer {
    pub rrtype: u16,
    pub ttl: u32,
    pub data: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnswerDetailsStatus {
    Complete,
    RawRdata,
    DecodeError,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResponseFlags {
    pub aa: bool,
    pub tc: bool,
    pub ra: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseDetails {
    pub rcode: u16,
    pub flags: ResponseFlags,
    pub answers: Vec<AuditAnswer>,
    pub answer_details_status: AnswerDetailsStatus,
    pub answer_decode_error: Option<String>,
}

impl ResponseDetails {
    pub fn no_response() -> Self {
        Self {
            rcode: 0,
            flags: ResponseFlags::default(),
            answers: Vec::new(),
            answer_details_status: AnswerDetailsStatus::Complete,
            answer_decode_error: None,
        }
    }
}

const DURATION_BUCKET_UPPER_BOUNDS_MICROS: [u64; 15] = [
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000,
    1_000_000, 2_500_000,
];
const DURATION_HISTOGRAM_BUCKET_COUNT: usize = DURATION_BUCKET_UPPER_BOUNDS_MICROS.len() + 1;
const INITIAL_AUDIT_RECORD_CAPACITY: usize = 1_024;
const MAX_NATIVE_REQUEST_ID: u64 = u64::MAX;

struct NativeRequestIdAllocator {
    nonce: [u8; 16],
    next: AtomicU64,
}

impl NativeRequestIdAllocator {
    fn from_nonce(nonce: [u8; 16]) -> Self {
        Self {
            nonce,
            next: AtomicU64::new(1),
        }
    }

    fn random() -> Result<Self, AdmissionError> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| AdmissionError::RequestIdExhausted)?;
        Ok(Self::from_nonce(nonce))
    }

    fn allocate(&self) -> Result<String, AdmissionError> {
        let counter = self
            .next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current == 0 {
                    None
                } else if current == MAX_NATIVE_REQUEST_ID {
                    Some(0)
                } else {
                    Some(current + 1)
                }
            })
            .map_err(|_| AdmissionError::RequestIdExhausted)?;
        let mut nonce_hex = String::with_capacity(32);
        for byte in self.nonce {
            use std::fmt::Write;
            let _ = write!(nonce_hex, "{byte:02x}");
        }
        Ok(format!("n-{nonce_hex}-{counter:016x}"))
    }
}

/// Wall-clock source shared by audit admission and window projections.
pub trait AuditClock: Send + Sync {
    /// Returns the current wall-clock instant.
    fn now(&self) -> SystemTime;
}

#[derive(Default)]
struct SystemAuditClock;

impl AuditClock for SystemAuditClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub(crate) fn default_audit_clock() -> Arc<dyn AuditClock> {
    Arc::new(SystemAuditClock)
}

/// Deterministic wall clock for native audit API tests.
#[derive(Clone)]
pub struct AuditTestClock {
    now: Arc<Mutex<SystemTime>>,
}

impl AuditTestClock {
    /// Creates a test clock at one fixed instant.
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    /// Sets the current test instant.
    pub fn set(&self, now: SystemTime) {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
    }

    /// Advances the current test instant.
    pub fn advance(&self, duration: Duration) {
        let mut now = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *now = now.checked_add(duration).expect("test clock overflow");
    }
}

impl AuditClock for AuditTestClock {
    fn now(&self) -> SystemTime {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    RequestIdExhausted,
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestIdExhausted => formatter.write_str("native request ID counter exhausted"),
        }
    }
}

impl std::error::Error for AdmissionError {}

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum UpstreamAttemptStorage {
    #[default]
    Empty,
    One(UpstreamAttemptRecord),
    Many(Vec<UpstreamAttemptRecord>),
}

/// Keeps the common zero-or-one upstream attempt inline and allocates only
/// when execution records a second leg.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct UpstreamAttemptList {
    storage: UpstreamAttemptStorage,
    capacity_hint: usize,
}

impl UpstreamAttemptList {
    pub(crate) fn with_capacity_hint(capacity_hint: usize) -> Self {
        Self {
            storage: UpstreamAttemptStorage::Empty,
            capacity_hint,
        }
    }

    pub(crate) fn push(&mut self, attempt: UpstreamAttemptRecord) {
        self.storage = match std::mem::take(&mut self.storage) {
            UpstreamAttemptStorage::Empty => UpstreamAttemptStorage::One(attempt),
            UpstreamAttemptStorage::One(first) => {
                let mut attempts = Vec::with_capacity(self.capacity_hint.max(2));
                attempts.push(first);
                attempts.push(attempt);
                UpstreamAttemptStorage::Many(attempts)
            }
            UpstreamAttemptStorage::Many(mut attempts) => {
                attempts.push(attempt);
                UpstreamAttemptStorage::Many(attempts)
            }
        };
    }

    pub(crate) fn as_slice(&self) -> &[UpstreamAttemptRecord] {
        match &self.storage {
            UpstreamAttemptStorage::Empty => &[],
            UpstreamAttemptStorage::One(attempt) => std::slice::from_ref(attempt),
            UpstreamAttemptStorage::Many(attempts) => attempts,
        }
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, UpstreamAttemptRecord> {
        self.as_slice().iter()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    pub(crate) fn into_vec(self) -> Vec<UpstreamAttemptRecord> {
        match self.storage {
            UpstreamAttemptStorage::Empty => Vec::new(),
            UpstreamAttemptStorage::One(attempt) => vec![attempt],
            UpstreamAttemptStorage::Many(attempts) => attempts,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_inline(&self) -> bool {
        !matches!(self.storage, UpstreamAttemptStorage::Many(_))
    }
}

impl From<Vec<UpstreamAttemptRecord>> for UpstreamAttemptList {
    fn from(mut attempts: Vec<UpstreamAttemptRecord>) -> Self {
        match attempts.len() {
            0 => Self::default(),
            1 => Self {
                storage: UpstreamAttemptStorage::One(attempts.remove(0)),
                capacity_hint: 0,
            },
            _ => Self {
                storage: UpstreamAttemptStorage::Many(attempts),
                capacity_hint: 0,
            },
        }
    }
}

impl From<UpstreamAttemptRecord> for UpstreamAttemptList {
    fn from(attempt: UpstreamAttemptRecord) -> Self {
        Self {
            storage: UpstreamAttemptStorage::One(attempt),
            capacity_hint: 0,
        }
    }
}

impl Index<usize> for UpstreamAttemptList {
    type Output = UpstreamAttemptRecord;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
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
    /// Native admission identifier, independent of the DNS transaction ID.
    pub trace_id: String,
    /// Monotonic elapsed time from admission through terminalization.
    pub elapsed: Duration,
    /// Mutually exclusive lifecycle terminal outcome.
    pub terminal_outcome: QueryTerminalOutcome,
    /// Final response state and source, independent from transport outcome.
    pub response: ResponseState,
    /// Safe flags and all-or-none final Answer projection.
    pub response_details: ResponseDetails,
    /// Cache result for this query.
    pub cache_status: CacheStatus,
    /// Final executed sequence tag, if established.
    pub final_sequence: Option<String>,
    /// Configured matched group, when the sequence set one.
    pub matched_group: Option<String>,
    /// Provider or inline rule identity that established the route.
    pub domain_set: Option<String>,
    /// Effective product routing label, when provenance is known.
    pub effective_tag: Option<String>,
    /// Configuration-owned source descriptor, when established.
    pub matched_rule_source: Option<String>,
    /// Upstream that supplied the final response, if any.
    pub final_upstream: Option<String>,
    /// Configured targets belonging to the final supplying leg.
    pub upstream_targets: Option<String>,
    /// Actual numeric endpoint that supplied the final wire.
    pub selected_upstream: Option<String>,
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

#[derive(Clone, Copy)]
pub(crate) struct AuditTimingSnapshot {
    pub timestamp: SystemTime,
    pub elapsed: Duration,
}

pub(crate) struct AuditStatsSnapshot {
    pub total_queries: usize,
    pub elapsed_micros: u128,
}

pub(crate) struct AuditReadSnapshot {
    pub records: Vec<Arc<AuditRecord>>,
    pub slowest: Vec<Arc<AuditRecord>>,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
pub(crate) struct TerminalObservation {
    pub outcome: QueryTerminalOutcome,
    pub response: ResponseState,
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
    pub failure_provenance: Option<FailureProvenance>,
    pub elapsed: Duration,
}

impl Default for TerminalObservation {
    fn default() -> Self {
        Self {
            outcome: QueryTerminalOutcome::NoResponse,
            response: ResponseState::NoResponse,
            response_details: ResponseDetails::no_response(),
            cache_status: CacheStatus::Undetermined,
            final_sequence: None,
            matched_group: None,
            domain_set: None,
            effective_tag: None,
            matched_rule_source: None,
            final_upstream: None,
            upstream_targets: None,
            selected_upstream: None,
            upstream_attempts: UpstreamAttemptList::default(),
            failure_provenance: None,
            elapsed: Duration::ZERO,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExecutionCheckpoint {
    capture_audit_details: bool,
    response: ResponseState,
    response_details: ResponseDetails,
    cache_status: CacheStatus,
    final_sequence: Option<String>,
    matched_group: Option<String>,
    domain_set: Option<String>,
    effective_tag: Option<String>,
    matched_rule_source: Option<String>,
    final_upstream: Option<String>,
    upstream_targets: Option<String>,
    selected_upstream: Option<String>,
    upstream_attempts: UpstreamAttemptList,
    failure_provenance: Option<FailureProvenance>,
    in_flight_upstream: Option<String>,
    completed_observation: Option<TerminalObservation>,
}

impl ExecutionCheckpoint {
    pub(crate) fn new(capture_audit_details: bool) -> Self {
        Self {
            capture_audit_details,
            response: ResponseState::NoResponse,
            response_details: ResponseDetails::no_response(),
            cache_status: CacheStatus::Undetermined,
            final_sequence: None,
            matched_group: None,
            domain_set: None,
            effective_tag: None,
            matched_rule_source: None,
            final_upstream: None,
            upstream_targets: None,
            selected_upstream: None,
            upstream_attempts: UpstreamAttemptList::default(),
            failure_provenance: None,
            in_flight_upstream: None,
            completed_observation: None,
        }
    }

    pub(crate) fn capture_audit_details(&self) -> bool {
        self.capture_audit_details
    }

    pub(crate) fn capture_partial(
        &mut self,
        observation: &TerminalObservation,
        in_flight_upstream: Option<String>,
    ) {
        self.response = observation.response.clone();
        self.response_details = observation.response_details.clone();
        self.cache_status = observation.cache_status;
        self.final_sequence.clone_from(&observation.final_sequence);
        self.matched_group.clone_from(&observation.matched_group);
        self.domain_set.clone_from(&observation.domain_set);
        self.effective_tag.clone_from(&observation.effective_tag);
        self.matched_rule_source
            .clone_from(&observation.matched_rule_source);
        self.final_upstream.clone_from(&observation.final_upstream);
        self.upstream_targets
            .clone_from(&observation.upstream_targets);
        self.selected_upstream
            .clone_from(&observation.selected_upstream);
        self.upstream_attempts
            .clone_from(&observation.upstream_attempts);
        self.failure_provenance
            .clone_from(&observation.failure_provenance);
        self.in_flight_upstream = in_flight_upstream;
    }

    fn terminal_observation(
        &self,
        outcome: QueryTerminalOutcome,
        canceled: bool,
        elapsed: Duration,
    ) -> TerminalObservation {
        let mut upstream_attempts = self.upstream_attempts.clone();
        if let Some(upstream) = &self.in_flight_upstream {
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
            response: self.response.clone(),
            response_details: self.response_details.clone(),
            cache_status: self.cache_status,
            final_sequence: self.final_sequence.clone(),
            matched_group: self.matched_group.clone(),
            domain_set: self.domain_set.clone(),
            effective_tag: self.effective_tag.clone(),
            matched_rule_source: self.matched_rule_source.clone(),
            final_upstream: self.final_upstream.clone(),
            upstream_targets: self.upstream_targets.clone(),
            selected_upstream: self.selected_upstream.clone(),
            upstream_attempts,
            failure_provenance: self.failure_provenance.clone(),
            elapsed,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct MetricsState {
    completed_total: u64,
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
    fn snapshot(&self, in_flight: u64) -> MetricsSnapshot {
        MetricsSnapshot {
            admitted_total: self.completed_total.saturating_add(in_flight),
            completed_total: self.completed_total,
            in_flight,
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
    audit_records: VecDeque<Arc<AuditRecord>>,
    slowest_records: Vec<Arc<AuditRecord>>,
    audit_timings: VecDeque<AuditTimingSnapshot>,
    audit_elapsed_micros: u128,
    audit_capacity: usize,
    capturing: bool,
    evicted_total: u64,
}

/// Thread-safe observer state owned by one host assembly.
#[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
pub(crate) struct QueryObserver {
    audit_enabled: bool,
    audit_clock: Arc<dyn AuditClock>,
    request_ids: NativeRequestIdAllocator,
    in_flight: AtomicU64,
    state: Mutex<ObserverState>,
}

impl QueryObserver {
    #[cfg(test)]
    pub(crate) fn new(
        audit_enabled: bool,
        upstream_identities: impl IntoIterator<Item = String>,
        audit_capacity: usize,
    ) -> Self {
        Self::with_clock(
            audit_enabled,
            upstream_identities,
            audit_capacity,
            Arc::new(SystemAuditClock),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_clock(
        audit_enabled: bool,
        upstream_identities: impl IntoIterator<Item = String>,
        audit_capacity: usize,
        audit_clock: Arc<dyn AuditClock>,
    ) -> Self {
        Self::with_parts(
            audit_enabled,
            upstream_identities,
            audit_capacity,
            audit_clock,
            NativeRequestIdAllocator::from_nonce([0x11; 16]),
        )
    }

    pub(crate) fn try_with_clock(
        audit_enabled: bool,
        upstream_identities: impl IntoIterator<Item = String>,
        audit_capacity: usize,
        audit_clock: Arc<dyn AuditClock>,
    ) -> Result<Self, AdmissionError> {
        let request_ids = NativeRequestIdAllocator::random()?;
        let observer = Self::with_parts(
            audit_enabled,
            upstream_identities,
            audit_capacity,
            audit_clock,
            request_ids,
        );
        Ok(observer)
    }

    fn with_parts(
        audit_enabled: bool,
        upstream_identities: impl IntoIterator<Item = String>,
        audit_capacity: usize,
        audit_clock: Arc<dyn AuditClock>,
        request_ids: NativeRequestIdAllocator,
    ) -> Self {
        let forward_attempts_by_upstream = upstream_identities
            .into_iter()
            .map(|identity| (identity, UpstreamAttemptMetricsSnapshot::default()))
            .collect();
        let audit_records = VecDeque::with_capacity(if audit_enabled {
            audit_capacity.min(INITIAL_AUDIT_RECORD_CAPACITY)
        } else {
            0
        });
        Self {
            audit_enabled,
            audit_clock,
            request_ids,
            in_flight: AtomicU64::new(0),
            state: Mutex::new(ObserverState {
                metrics: MetricsState {
                    forward_attempts_by_upstream,
                    ..MetricsState::default()
                },
                audit_records,
                slowest_records: Vec::new(),
                audit_timings: VecDeque::with_capacity(if audit_enabled {
                    audit_capacity.min(INITIAL_AUDIT_RECORD_CAPACITY)
                } else {
                    0
                }),
                audit_elapsed_micros: 0,
                audit_capacity,
                capturing: audit_enabled,
                ..ObserverState::default()
            }),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // Listener admission is wired in Slice 2.
    pub(crate) fn admit_query(&self) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
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
        let capture_at_terminal = self.audit_enabled && self.lock().capturing;
        if capture_at_terminal {
            let mut state = self.lock();
            let can_retain = if state.audit_capacity == 0 {
                true
            } else {
                let ring_ready = state.audit_records.len() == state.audit_capacity
                    || state.audit_records.try_reserve(1).is_ok();
                let timing_ready = state.audit_timings.len() == state.audit_capacity
                    || state.audit_timings.try_reserve(1).is_ok();
                ring_ready && timing_ready && state.slowest_records.try_reserve(1).is_ok()
            };
            if !can_retain {
                // Retention is best-effort at the configured boundary: a
                // variable-detail allocation failure must not lose lifetime
                // metrics or the admitted-query terminalization. Read-side
                // projections use the same explicit 500 policy.
                state.metrics.record_terminal(
                    observation.outcome,
                    &observation.response,
                    observation.cache_status,
                    observation.upstream_attempts.as_slice(),
                    observation.elapsed,
                );
                self.decrement_in_flight();
                return;
            }
            let record = Arc::new(make_audit_record(observation));
            state.metrics.record_terminal(
                record.terminal_outcome,
                &record.response,
                record.cache_status,
                &record.upstream_attempts,
                record.elapsed,
            );
            self.decrement_in_flight();
            if state.audit_capacity == 0 {
                return;
            }
            state.slowest_records.push(Arc::clone(&record));
            state.slowest_records.sort_by(|left, right| {
                right
                    .elapsed
                    .cmp(&left.elapsed)
                    .then_with(|| right.trace_id.cmp(&left.trace_id))
            });
            state.slowest_records.truncate(300);
            if state.audit_records.len() == state.audit_capacity {
                state.audit_records.pop_front();
                if let Some(timing) = state.audit_timings.pop_front() {
                    state.audit_elapsed_micros = state
                        .audit_elapsed_micros
                        .saturating_sub(timing.elapsed.as_micros());
                }
                state.evicted_total = state.evicted_total.saturating_add(1);
            }
            state.audit_elapsed_micros = state
                .audit_elapsed_micros
                .saturating_add(record.elapsed.as_micros());
            state.audit_timings.push_back(AuditTimingSnapshot {
                timestamp: record.timestamp,
                elapsed: record.elapsed,
            });
            state.audit_records.push_back(record);
        } else {
            let mut state = self.lock();
            state.metrics.record_terminal(
                observation.outcome,
                &observation.response,
                observation.cache_status,
                observation.upstream_attempts.as_slice(),
                observation.elapsed,
            );
            self.decrement_in_flight();
        }
    }

    pub(crate) fn start_capture(&self) -> bool {
        let mut state = self.lock();
        if !self.audit_enabled {
            return false;
        }
        state.capturing = true;
        true
    }

    pub(crate) fn stop_capture(&self) -> bool {
        let mut state = self.lock();
        if !self.audit_enabled {
            return false;
        }
        state.capturing = false;
        true
    }

    pub(crate) fn clear_audit(&self) {
        let mut state = self.lock();
        state.audit_records.clear();
        state.slowest_records.clear();
        state.audit_timings.clear();
        state.audit_elapsed_micros = 0;
        state.evicted_total = 0;
    }

    pub(crate) fn set_audit_capacity(&self, capacity: usize) {
        let mut state = self.lock();
        state.audit_capacity = capacity;
        state.audit_records.clear();
        state.slowest_records.clear();
        state.audit_timings.clear();
        state.audit_elapsed_micros = 0;
        state.evicted_total = 0;
    }

    pub(crate) fn audit_capacity(&self) -> usize {
        self.lock().audit_capacity
    }

    pub(crate) fn is_capturing(&self) -> bool {
        self.lock().capturing
    }

    #[cfg(test)]
    fn audit_capturing(&self) -> bool {
        self.lock().capturing
    }

    fn decrement_in_flight(&self) {
        self.in_flight
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_sub(1)
            })
            .expect("terminal query must have a matching admission");
    }

    pub(crate) fn try_admit(
        self: &std::sync::Arc<Self>,
        client_addr: SocketAddr,
        transport: QueryTransport,
        question: &mosdns_dns_core::QuestionInfo,
        cancellation: TransportCancellation,
    ) -> Result<AdmittedQueryGuard, AdmissionError> {
        let admitted_at = Instant::now();
        let trace_id = self.request_ids.allocate()?;
        let audit_context = self.audit_enabled.then(|| AuditContext {
            timestamp: self.audit_clock.now(),
            client_addr,
            transport,
            qname: render_qname(&question.qname_wire),
            qtype: question.qtype,
            qclass: question.qclass,
            trace_id,
        });
        self.admit_query();
        Ok(AdmittedQueryGuard {
            observer: std::sync::Arc::clone(self),
            admitted_at,
            audit_context,
            cancellation,
            execution_checkpoint: Box::new(ExecutionCheckpoint::new(self.audit_enabled)),
            finalized: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn admit(
        self: &std::sync::Arc<Self>,
        client_addr: SocketAddr,
        transport: QueryTransport,
        question: &mosdns_dns_core::QuestionInfo,
        cancellation: TransportCancellation,
    ) -> AdmittedQueryGuard {
        self.try_admit(client_addr, transport, question, cancellation)
            .expect("test request ID allocation")
    }

    pub(crate) fn metrics_snapshot(&self) -> MetricsSnapshot {
        let state = self.lock();
        state
            .metrics
            .snapshot(self.in_flight.load(Ordering::SeqCst))
    }

    pub(crate) fn audit_snapshot(&self) -> AuditSnapshot {
        let state = self.lock();
        AuditSnapshot {
            records: state
                .audit_records
                .iter()
                .map(|record| record.as_ref().clone())
                .collect(),
            evicted_total: state.evicted_total,
        }
    }

    pub(crate) fn audit_read_snapshot(&self) -> AuditReadSnapshot {
        let state = self.lock();
        AuditReadSnapshot {
            records: state.audit_records.iter().cloned().collect(),
            slowest: state.slowest_records.clone(),
        }
    }

    pub(crate) fn audit_stats_snapshot(&self) -> AuditStatsSnapshot {
        let state = self.lock();
        AuditStatsSnapshot {
            total_queries: state.audit_records.len(),
            elapsed_micros: state.audit_elapsed_micros,
        }
    }

    pub(crate) fn audit_timing_snapshot(&self) -> Vec<AuditTimingSnapshot> {
        self.lock().audit_timings.iter().copied().collect()
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
    trace_id: String,
}

/// Finalizes one admitted request. Dropping an unfinished request records one
/// canceled terminal event if its cancellation scope fired, or a no-response
/// event for another unwind path.
pub(crate) struct AdmittedQueryGuard {
    observer: std::sync::Arc<QueryObserver>,
    admitted_at: Instant,
    audit_context: Option<AuditContext>,
    cancellation: TransportCancellation,
    execution_checkpoint: Box<ExecutionCheckpoint>,
    finalized: bool,
}

impl AdmittedQueryGuard {
    pub(crate) fn execution_checkpoint(&mut self) -> &mut ExecutionCheckpoint {
        &mut self.execution_checkpoint
    }

    pub(crate) fn capture_execution(&mut self, observation: TerminalObservation) {
        self.execution_checkpoint.completed_observation = Some(observation);
    }

    pub(crate) fn finish(mut self, outcome: QueryTerminalOutcome) {
        let mut observation = self
            .execution_checkpoint
            .completed_observation
            .take()
            .expect("terminal query must carry completed execution facts");
        observation.outcome = outcome;
        observation.elapsed = self.admitted_at.elapsed();
        self.record(observation);
    }

    fn record(&mut self, observation: TerminalObservation) {
        self.finalized = true;
        let audit_context = self.audit_context.take();
        self.observer.record_terminal(observation, |observation| {
            let context = audit_context.expect("audit context exists when capture is enabled");
            let derived_final_upstream = match &observation.response {
                ResponseState::Dns {
                    source: ResponseSource::Upstream(upstream),
                    ..
                } => Some(upstream.clone()),
                ResponseState::Dns { .. } | ResponseState::NoResponse => None,
            };
            let final_upstream = observation.final_upstream.or(derived_final_upstream);
            AuditRecord {
                timestamp: context.timestamp,
                client_addr: context.client_addr,
                transport: context.transport,
                qname: context.qname,
                qtype: context.qtype,
                qclass: context.qclass,
                trace_id: context.trace_id,
                elapsed: observation.elapsed,
                terminal_outcome: observation.outcome,
                response: observation.response,
                response_details: observation.response_details,
                cache_status: observation.cache_status,
                final_sequence: observation.final_sequence,
                matched_group: observation.matched_group,
                domain_set: observation.domain_set,
                effective_tag: observation.effective_tag,
                matched_rule_source: observation.matched_rule_source,
                final_upstream,
                upstream_targets: observation.upstream_targets,
                selected_upstream: observation.selected_upstream,
                upstream_attempts: observation.upstream_attempts.into_vec(),
                failure_provenance: observation.failure_provenance,
            }
        });
    }
}

impl Drop for AdmittedQueryGuard {
    fn drop(&mut self) {
        if !self.finalized {
            let canceled = self.cancellation.is_cancelled();
            let outcome = if canceled {
                QueryTerminalOutcome::Canceled
            } else {
                QueryTerminalOutcome::NoResponse
            };
            let mut observation = self
                .execution_checkpoint
                .completed_observation
                .take()
                .unwrap_or_else(|| {
                    self.execution_checkpoint.terminal_observation(
                        outcome,
                        canceled,
                        self.admitted_at.elapsed(),
                    )
                });
            observation.outcome = outcome;
            observation.elapsed = self.admitted_at.elapsed();
            self.record(observation);
        }
    }
}

fn render_qname(wire: &[u8]) -> String {
    fn labels(mut wire: &[u8]) -> impl Iterator<Item = &[u8]> {
        std::iter::from_fn(move || {
            let (&length, rest) = wire.split_first()?;
            if length == 0 {
                return None;
            }
            let label = rest.get(..usize::from(length))?;
            wire = &rest[usize::from(length)..];
            Some(label)
        })
    }

    // Two bounded passes over an already parsed DNS name avoid allocating a
    // string per label, a label vector, join output and formatted escapes.
    let length = labels(wire)
        .map(|label| {
            1 + label
                .iter()
                .map(|byte| match *byte {
                    b'.' | b'\\' => 2,
                    0x21..=0x7e => 1,
                    _ => 4,
                })
                .sum::<usize>()
        })
        .sum::<usize>()
        .max(1);
    let mut rendered = String::with_capacity(length);
    for label in labels(wire) {
        for byte in label {
            match *byte {
                b'.' => rendered.push_str("\\."),
                b'\\' => rendered.push_str("\\\\"),
                0x21..=0x7e => rendered.push(char::from(*byte)),
                _ => {
                    rendered.push('\\');
                    rendered.push(char::from(b'0' + byte / 100));
                    rendered.push(char::from(b'0' + (byte / 10) % 10));
                    rendered.push(char::from(b'0' + byte % 10));
                }
            }
        }
        rendered.push('.');
    }
    if rendered.is_empty() {
        rendered.push('.');
    }
    rendered
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::net::SocketAddr;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, SystemTime};

    use mosdns_upstream_core::TransportCancellation;

    use super::{
        AuditRecord, CacheStatus, FailureProvenance, INITIAL_AUDIT_RECORD_CAPACITY,
        LocalFailureKind, MAX_NATIVE_REQUEST_ID, NativeRequestIdAllocator, QueryObserver,
        QueryTerminalOutcome, QueryTransport, ResponseDetails, ResponseSource, ResponseState,
        TerminalObservation, UpstreamAttemptList, UpstreamAttemptOutcome, UpstreamAttemptRecord,
    };

    fn observer(audit_enabled: bool, capacity: usize) -> QueryObserver {
        QueryObserver::new(
            audit_enabled,
            ["route-a".to_owned(), "route-b".to_owned()],
            capacity,
        )
    }

    #[test]
    fn native_request_ids_are_fixed_width_and_fail_closed_at_counter_exhaustion() {
        let allocator = NativeRequestIdAllocator::from_nonce([0xab; 16]);
        assert_eq!(
            allocator.allocate().expect("first request id"),
            "n-abababababababababababababababab-0000000000000001"
        );
        allocator
            .next
            .store(MAX_NATIVE_REQUEST_ID, Ordering::SeqCst);
        assert!(allocator.allocate().is_ok());
        assert!(allocator.allocate().is_err());
    }

    #[test]
    fn runtime_capture_defaults_on_and_static_gate_cannot_be_overridden() {
        let enabled = observer(true, 2);
        assert!(enabled.audit_capturing());
        assert!(enabled.stop_capture());
        assert!(!enabled.audit_capturing());
        assert!(enabled.start_capture());
        assert!(enabled.audit_capturing());

        let disabled = observer(false, 2);
        assert!(!disabled.audit_capturing());
        assert!(!disabled.start_capture());
        assert!(!disabled.stop_capture());
        assert!(!disabled.audit_capturing());
    }

    #[test]
    fn terminal_capture_uses_the_runtime_state_at_terminalization() {
        let observer = std::sync::Arc::new(observer(true, 4));
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'o', b'l', b'd', 0],
            qtype: 1,
            qclass: 1,
        };
        let first = observer.admit(
            "192.0.2.10:53000".parse().expect("client"),
            QueryTransport::Udp,
            &question,
            TransportCancellation::new(),
        );
        observer.stop_capture();
        drop(first);
        assert!(observer.audit_snapshot().records.is_empty());

        let second = observer.admit(
            "192.0.2.11:53000".parse().expect("client"),
            QueryTransport::Udp,
            &question,
            TransportCancellation::new(),
        );
        observer.start_capture();
        drop(second);
        assert_eq!(observer.audit_snapshot().records.len(), 1);
        assert_eq!(observer.metrics_snapshot().completed_total, 2);
    }

    #[test]
    fn zero_capacity_keeps_lifetime_metrics_without_retaining_records() {
        let observer = observer(true, 0);
        for _ in 0..3 {
            observer.admit_query();
            observer.record_terminal(observation(Duration::from_micros(1)), |observation| {
                audit_record(observation, "zero.example")
            });
        }

        assert!(observer.audit_snapshot().records.is_empty());
        assert_eq!(observer.metrics_snapshot().completed_total, 3);
        assert_eq!(observer.metrics_snapshot().duration.count, 3);
        assert_eq!(observer.audit_snapshot().evicted_total, 0);
    }

    #[test]
    fn clear_and_resize_linearize_against_the_retained_ring() {
        let observer = observer(true, 3);
        for name in ["one.example", "two.example", "three.example"] {
            observer.admit_query();
            observer.record_terminal(observation(Duration::from_micros(1)), |observation| {
                audit_record(observation, name)
            });
        }
        observer.set_audit_capacity(1);
        assert!(observer.audit_snapshot().records.is_empty());
        assert_eq!(observer.audit_snapshot().evicted_total, 0);
        observer.clear_audit();
        assert!(observer.audit_snapshot().records.is_empty());
        assert_eq!(observer.metrics_snapshot().completed_total, 3);
    }

    #[test]
    fn audit_admission_renders_escaped_question_in_exact_sized_buffer() {
        let observer = std::sync::Arc::new(observer(true, 2));
        for (wire, expected) in [
            (vec![0], "."),
            (vec![1, b'a', 1, b'b', 0], "a.b."),
            (vec![4, b'.', b'\\', 0, 255, 0], "\\.\\\\\\000\\255."),
        ] {
            let question = mosdns_dns_core::QuestionInfo {
                qname_wire: wire,
                qtype: 1,
                qclass: 1,
            };
            let guard = observer.admit(
                "127.0.0.1:1234".parse().expect("client"),
                QueryTransport::Tcp,
                &question,
                TransportCancellation::new(),
            );
            let name = &guard.audit_context.as_ref().expect("audit enabled").qname;
            assert_eq!(name, expected);
            assert_eq!(name.capacity(), expected.len(), "reserve only final output");
            drop(guard);
        }
        assert_eq!(observer.metrics_snapshot().completed_total, 3);
        let audit = observer.audit_snapshot();
        assert_eq!(audit.records.len(), 2);
        assert_eq!(audit.evicted_total, 1);
        assert_eq!(audit.records[1].qname, "\\.\\\\\\000\\255.");
    }

    #[test]
    fn upstream_attempt_list_keeps_one_attempt_inline_and_promotes_in_order() {
        let mut attempts = UpstreamAttemptList::with_capacity_hint(3);
        assert!(attempts.is_inline());
        attempts.push(UpstreamAttemptRecord {
            upstream: "route-a".to_owned(),
            outcome: UpstreamAttemptOutcome::Response,
        });
        assert!(attempts.is_inline());
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].upstream, "route-a");

        attempts.push(UpstreamAttemptRecord {
            upstream: "route-b".to_owned(),
            outcome: UpstreamAttemptOutcome::Failed,
        });
        assert!(!attempts.is_inline());
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].upstream, "route-a");
        assert_eq!(attempts[1].upstream, "route-b");
        assert_eq!(attempts[1].outcome, UpstreamAttemptOutcome::Failed);
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
    fn dropped_send_preserves_completed_facts_from_boxed_observation() {
        let observer = std::sync::Arc::new(observer(true, 4));
        let cancellation = TransportCancellation::new();
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'w', b'1', 0],
            qtype: 1,
            qclass: 1,
        };
        let mut guard = observer.admit(
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
            matched_group: None,
            final_upstream: None,
            upstream_attempts: UpstreamAttemptList::from(vec![UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Response,
            }]),
            failure_provenance: None,
            elapsed: Duration::ZERO,
            ..Default::default()
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
        let in_flight = observer.metrics_snapshot();
        assert_eq!(in_flight.admitted_total, 1);
        assert_eq!(in_flight.completed_total, 0);
        assert_eq!(in_flight.in_flight, 1);
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
        let mut guard = observer.admit(
            "192.0.2.52:53000".parse().expect("client address"),
            QueryTransport::Udp,
            &question,
            TransportCancellation::new(),
        );
        guard.execution_checkpoint().capture_partial(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::Miss,
                final_sequence: Some("entry".to_owned()),
                matched_group: None,
                final_upstream: None,
                upstream_attempts: UpstreamAttemptList::from(vec![UpstreamAttemptRecord {
                    upstream: "route-a".to_owned(),
                    outcome: UpstreamAttemptOutcome::Response,
                }]),
                failure_provenance: Some(FailureProvenance::UpstreamFailure {
                    upstream: "route-a".to_owned(),
                }),
                elapsed: Duration::ZERO,
                ..Default::default()
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
        let mut guard = observer.admit(
            "192.0.2.53:53000".parse().expect("client address"),
            QueryTransport::Tcp,
            &question,
            cancellation.clone(),
        );
        guard.execution_checkpoint().capture_partial(
            &TerminalObservation {
                outcome: QueryTerminalOutcome::NoResponse,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::Undetermined,
                final_sequence: Some("entry".to_owned()),
                matched_group: None,
                final_upstream: None,
                upstream_attempts: UpstreamAttemptList::default(),
                failure_provenance: None,
                elapsed: Duration::ZERO,
                ..Default::default()
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
            matched_group: None,
            final_upstream: None,
            upstream_attempts: UpstreamAttemptList::from(vec![UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Response,
            }]),
            failure_provenance: None,
            elapsed,
            ..Default::default()
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
            matched_group: None,
            final_upstream: Some("route-a".to_owned()),
            upstream_attempts: observation.upstream_attempts.into_vec(),
            failure_provenance: None,
            trace_id: "n-11111111111111111111111111111111-0000000000000001".to_owned(),
            response_details: ResponseDetails::no_response(),
            domain_set: None,
            effective_tag: None,
            matched_rule_source: None,
            upstream_targets: None,
            selected_upstream: None,
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
                    matched_group: None,
                    final_upstream: None,
                    upstream_attempts: upstream_attempts.into(),
                    failure_provenance: None,
                    elapsed: Duration::from_micros(250),
                    ..Default::default()
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
