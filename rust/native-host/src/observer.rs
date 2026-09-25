use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const DURATION_BUCKET_UPPER_BOUNDS_MICROS: [u64; 15] = [
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000,
    1_000_000, 2_500_000,
];

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

impl DurationHistogramSnapshot {
    #[cfg_attr(not(test), allow(dead_code))] // Listener terminalization is wired in Slice 2.
    fn observe(&mut self, elapsed: Duration) {
        let micros = elapsed.as_micros();
        for bucket in &mut self.buckets {
            if bucket
                .upper_bound_micros
                .is_none_or(|upper_bound| micros <= u128::from(upper_bound))
            {
                bucket.cumulative_count = bucket.cumulative_count.saturating_add(1);
            }
        }
        self.count = self.count.saturating_add(1);
        self.sum_micros = self.sum_micros.saturating_add(micros);
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
    pub upstream_attempts: Vec<UpstreamAttemptRecord>,
    pub elapsed: Duration,
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
    forward_attempts_by_upstream: BTreeMap<String, UpstreamAttemptMetricsSnapshot>,
    unknown_forward_attempts_total: u64,
    duration: DurationHistogramSnapshot,
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
            forward_attempts_by_upstream: self.forward_attempts_by_upstream.clone(),
            unknown_forward_attempts_total: self.unknown_forward_attempts_total,
            duration: self.duration.clone(),
        }
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
        Self {
            audit_enabled,
            audit_capacity,
            state: Mutex::new(ObserverState {
                metrics: MetricsState {
                    forward_attempts_by_upstream,
                    ..MetricsState::default()
                },
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
        make_audit_record: impl FnOnce(&TerminalObservation) -> AuditRecord,
    ) {
        // Construct sensitive details only when the sole listener enabled capture,
        // and keep allocation outside the mutex-protected metrics update.
        let audit_record = self.audit_enabled.then(|| make_audit_record(&observation));
        let mut state = self.lock();
        let metrics = &mut state.metrics;
        metrics.completed_total = metrics.completed_total.saturating_add(1);
        metrics.in_flight = metrics
            .in_flight
            .checked_sub(1)
            .expect("terminal query must have a matching admission");
        match observation.outcome {
            QueryTerminalOutcome::SendSucceeded => {
                metrics.send_succeeded_total = metrics.send_succeeded_total.saturating_add(1);
            }
            QueryTerminalOutcome::SendFailed => {
                metrics.send_failed_total = metrics.send_failed_total.saturating_add(1);
            }
            QueryTerminalOutcome::Canceled => {
                metrics.canceled_total = metrics.canceled_total.saturating_add(1);
            }
            QueryTerminalOutcome::NoResponse => {
                metrics.no_response_total = metrics.no_response_total.saturating_add(1);
            }
        }
        if let ResponseState::Dns { rcode, .. } = &observation.response {
            let total = metrics.response_code_totals.entry(*rcode).or_default();
            *total = total.saturating_add(1);
        }
        match observation.cache_status {
            CacheStatus::NotApplicable => {
                metrics.cache_not_applicable_total =
                    metrics.cache_not_applicable_total.saturating_add(1);
            }
            CacheStatus::Hit => {
                metrics.cache_hits_total = metrics.cache_hits_total.saturating_add(1);
            }
            CacheStatus::Miss => {
                metrics.cache_misses_total = metrics.cache_misses_total.saturating_add(1);
            }
        }
        for attempt in &observation.upstream_attempts {
            if let Some(counters) = metrics
                .forward_attempts_by_upstream
                .get_mut(&attempt.upstream)
            {
                counters.attempts_total = counters.attempts_total.saturating_add(1);
                let counter = match attempt.outcome {
                    UpstreamAttemptOutcome::Response => &mut counters.responses_total,
                    UpstreamAttemptOutcome::Failed => &mut counters.failures_total,
                    UpstreamAttemptOutcome::TimedOut => &mut counters.timeouts_total,
                };
                *counter = counter.saturating_add(1);
            } else {
                metrics.unknown_forward_attempts_total =
                    metrics.unknown_forward_attempts_total.saturating_add(1);
            }
        }
        metrics.duration.observe(observation.elapsed);

        if let Some(record) = audit_record {
            if state.audit_records.len() == self.audit_capacity.get() {
                state.audit_records.pop_front();
                state.evicted_total = state.evicted_total.saturating_add(1);
            }
            state.audit_records.push_back(record);
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::net::SocketAddr;
    use std::num::NonZeroUsize;
    use std::time::{Duration, SystemTime};

    use super::{
        AuditRecord, CacheStatus, FailureProvenance, LocalFailureKind, QueryObserver,
        QueryTerminalOutcome, QueryTransport, ResponseSource, ResponseState, TerminalObservation,
        UpstreamAttemptOutcome, UpstreamAttemptRecord,
    };

    fn observer(audit_enabled: bool, capacity: usize) -> QueryObserver {
        QueryObserver::new(
            audit_enabled,
            ["route-a".to_owned(), "route-b".to_owned()],
            NonZeroUsize::new(capacity).expect("non-zero capacity"),
        )
    }

    fn observation(elapsed: Duration) -> TerminalObservation {
        TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Upstream("route-a".to_owned()),
            },
            cache_status: CacheStatus::Miss,
            upstream_attempts: vec![UpstreamAttemptRecord {
                upstream: "route-a".to_owned(),
                outcome: UpstreamAttemptOutcome::Response,
            }],
            elapsed,
        }
    }

    fn audit_record(observation: &TerminalObservation, qname: &str) -> AuditRecord {
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
            response: observation.response.clone(),
            cache_status: observation.cache_status,
            final_sequence: Some("entry".to_owned()),
            final_upstream: Some("route-a".to_owned()),
            upstream_attempts: observation.upstream_attempts.clone(),
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
                    upstream_attempts,
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
