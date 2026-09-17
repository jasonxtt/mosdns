//! The resolver owner: one target/bootstrap tuple, one absolute deadline, and
//! one single-flight publication.
//!
//! The owner holds exactly one resolution key (the target/bootstrap tuple), so
//! no global cache exists and the tuple itself is the cache key. Concurrent
//! callers for the same unresolved or expired name attach to one leader rather
//! than creating unbounded query fan-out.
//!
//! Everything here runs on the caller's runtime. Close prevents new work, drains
//! registered work, and is idempotent; it reuses the reviewed upstream-core
//! [`crate::Lifecycle`] and [`crate::TransportCancellation`] vocabulary rather
//! than duplicating those state machines.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::Notify;

use super::bootstrap::{OsIdSource, ResolutionIdSource, SteppingIdSource};
use super::{
    BootstrapEndpoint, Clock, PublishedTarget, ResolutionPolicy, ResolutionTarget,
    ResolvedDestination, ResolverError, ResolverState,
};
use crate::secure::{DohEndpoint, DotEndpoint};
use crate::{
    CloseCompletion, CloseResult, CloseTransition, Endpoint, ExchangeContext, Lifecycle,
    LifecycleState, Transport, TransportCancellation, Upstream,
};

/// A monotonic identity for one refresh generation.
///
/// Waiters attach to a specific token, so a waiter from generation *N* can never
/// observe generation *N+1*'s in-progress state or its result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GenerationId(u64);

/// The lifecycle of the single refresh generation the owner may run.
#[derive(Debug, Default)]
struct Generation {
    /// The identity of the current or most recent generation.
    id: u64,
    /// Whether one leader is currently running the generation `id`.
    running: bool,
    /// The complete result the generation `id` committed, if any.
    result: Option<Result<PublishedTarget, ResolverError>>,
}

/// Single-flight generation state shared by leader and waiters.
///
/// One short synchronous mutex holds the generation flag and its result, and one
/// [`Notify`] wakes waiters. No await ever happens while the lock is held, so the
/// owner cannot deadlock on its own state.
#[derive(Debug)]
struct SingleFlight {
    inner: Mutex<Generation>,
    completed: Notify,
}

impl SingleFlight {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Generation::default()),
            completed: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Generation> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Attempts to become the leader of a fresh generation.
    ///
    /// Returns the new generation's token when this caller owns it, or `None`
    /// when another caller is already leading, in which case the caller becomes
    /// a bounded waiter attached to that in-flight generation.
    fn try_lead(&self) -> Option<GenerationId> {
        let mut generation = self.lock();
        if generation.running {
            return None;
        }
        generation.id = generation.id.wrapping_add(1);
        generation.running = true;
        generation.result = None;
        Some(GenerationId(generation.id))
    }

    /// The identity of the generation a new waiter must attach to, if one is
    /// currently running or has an unread result.
    fn current_generation(&self) -> Option<GenerationId> {
        let generation = self.lock();
        if generation.running || generation.result.is_some() {
            return Some(GenerationId(generation.id));
        }
        None
    }

    /// The outcome a dropped leader publishes, so waiters can never deadlock on
    /// a generation whose leader future was abandoned or aborted.
    const ABANDONED: Result<PublishedTarget, ResolverError> = Err(ResolverError::Cancelled);

    /// Commits the leader's outcome for `token` and wakes every waiter.
    ///
    /// A completion for a superseded generation is ignored, so a stale leader
    /// can never overwrite a newer generation's state.
    fn complete(&self, token: GenerationId, result: Result<PublishedTarget, ResolverError>) {
        let mut generation = self.lock();
        if GenerationId(generation.id) != token || !generation.running {
            return;
        }
        generation.running = false;
        generation.result = Some(result);
        drop(generation);
        self.completed.notify_waiters();
    }

    /// Awaits the specific generation `token` and returns its committed outcome.
    ///
    /// The waiter only ever reads the state of the generation it attached to. If
    /// that generation has been superseded, this waiter does not silently adopt
    /// the newer one: it reports [`ResolverError::AlreadyResolving`], which is
    /// what makes a waiter from a finished generation unable to observe a later
    /// leader's half-built state.
    async fn wait(&self, token: GenerationId) -> Result<PublishedTarget, ResolverError> {
        loop {
            let notified = self.completed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let generation = self.lock();
                if GenerationId(generation.id) != token {
                    // A newer generation owns the slot; this waiter's own
                    // generation is gone and must not be confused with it.
                    return Err(ResolverError::AlreadyResolving);
                }
                if !generation.running {
                    return match &generation.result {
                        Some(result) => result.clone(),
                        None => Err(ResolverError::AlreadyResolving),
                    };
                }
            }
            notified.await;
        }
    }
}

/// A resolver configured for one target/bootstrap tuple.
///
/// Numeric targets bypass DNS entirely: the numeric destination is published
/// immediately and no bootstrap socket is opened.
pub struct BootstrapResolver {
    target: ResolutionTarget,
    bootstrap: BootstrapEndpoint,
    policy: ResolutionPolicy,
    clock: Arc<dyn Clock>,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    state: Arc<ResolverState>,
    flight: SingleFlight,
    ids: Arc<dyn ResolutionIdSource>,
}

impl BootstrapResolver {
    /// Builds a production resolver for one validated tuple.
    ///
    /// The transaction IDs come from [`OsIdSource`], the unpredictable
    /// operating-system source. A DNS transaction ID is part of the
    /// anti-spoofing correlation set, so this path never falls back to a
    /// predictable sequence; on a host with no usable entropy source it returns
    /// [`ResolverError::UnpredictableIdsUnavailable`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::BootstrapFamilyMismatch`] when the numeric
    /// bootstrap peer's family cannot serve the target's selected family, and
    /// [`ResolverError::UnpredictableIdsUnavailable`] when no unpredictable ID
    /// source is available.
    pub fn new(
        target: ResolutionTarget,
        bootstrap: BootstrapEndpoint,
        policy: ResolutionPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ResolverError> {
        if !OsIdSource.is_available() {
            return Err(ResolverError::UnpredictableIdsUnavailable);
        }
        Self::with_id_source(target, bootstrap, policy, clock, Arc::new(OsIdSource))
    }

    /// Builds a resolver with deterministic, **predictable** transaction IDs.
    ///
    /// This exists only so tests can pin IDs without touching the exchange. It
    /// is not a production path: it must never be reachable from the default
    /// construction, which is why it is named for its one legitimate caller.
    ///
    /// # Errors
    ///
    /// As [`Self::with_id_source`].
    pub fn with_deterministic_ids_for_tests(
        target: ResolutionTarget,
        bootstrap: BootstrapEndpoint,
        policy: ResolutionPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ResolverError> {
        Self::with_id_source(
            target,
            bootstrap,
            policy,
            clock,
            Arc::new(SteppingIdSource::new()),
        )
    }

    /// Builds a resolver with an injected query-ID source.
    ///
    /// A production caller supplies an unpredictable source here; the default
    /// stepping source exists so the exchange stays deterministic in tests.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn with_id_source(
        target: ResolutionTarget,
        bootstrap: BootstrapEndpoint,
        policy: ResolutionPolicy,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn ResolutionIdSource>,
    ) -> Result<Self, ResolverError> {
        if target.family() != bootstrap.family() {
            return Err(ResolverError::BootstrapFamilyMismatch);
        }
        Ok(Self {
            target,
            bootstrap,
            policy,
            clock,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            state: Arc::new(ResolverState::new()),
            flight: SingleFlight::new(),
            ids,
        })
    }

    /// The validated target this resolver serves.
    #[must_use]
    pub const fn target(&self) -> &ResolutionTarget {
        &self.target
    }

    /// The numeric UDP bootstrap peer.
    #[must_use]
    pub const fn bootstrap(&self) -> BootstrapEndpoint {
        self.bootstrap
    }

    /// The reviewed TTL/retransmit policy.
    #[must_use]
    pub const fn policy(&self) -> &ResolutionPolicy {
        &self.policy
    }

    /// The publication state, shared so callers can observe diagnostics.
    #[must_use]
    pub fn state(&self) -> Arc<ResolverState> {
        Arc::clone(&self.state)
    }

    /// Whether this resolver draws unpredictable transaction IDs.
    ///
    /// An observability hook proving the default production path did not select
    /// the deterministic test source; it exposes no ID values.
    #[must_use]
    pub fn uses_unpredictable_ids(&self) -> bool {
        self.ids.is_unpredictable()
    }

    /// The owner lifecycle state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// The number of resolutions currently registered as in-flight.
    #[must_use]
    pub fn in_flight_resolutions(&self) -> usize {
        self.lifecycle.in_flight()
    }

    /// Begins owner shutdown and cancels owned work.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
        }
        transition
    }

    /// Begins close, drains every in-flight resolution on the caller's runtime,
    /// then completes the guarded transition. Idempotent.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        self.lifecycle.drain().await;
        match self.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    /// Resolves the target, or returns the still-fresh published result.
    ///
    /// The caller's `context` supplies the original absolute deadline and the
    /// caller cancellation token; owner shutdown is observed through this
    /// resolver's own lifecycle. Resolution never resets or extends the
    /// deadline before a later dial.
    ///
    /// # Errors
    ///
    /// Returns the typed [`ResolverError`] for a malformed reply, a terminal DNS
    /// rcode, no usable address, a deadline, cancellation, or shutdown.
    pub async fn resolve(
        &self,
        context: ExchangeContext,
    ) -> Result<PublishedTarget, ResolverError> {
        self.lifecycle
            .ensure_open()
            .map_err(|_| ResolverError::Closed)?;

        // Every path that can publish first applies the same terminal controls:
        // owner shutdown, caller cancellation, then the caller's one absolute
        // deadline. A numeric target bypasses DNS, but not the caller's budget.
        self.check_control(&context, crate::SideEffectState::NotSent)?;

        // A numeric target never touches the network.
        if let Some(address) = self.target.numeric_address() {
            let published = super::resolve_numeric(address)?;
            // Publishing is a success, so it goes through the same lifecycle
            // linearization gate as a DNS result: a close that wins the gate
            // prevents publication, and a publication that wins cannot be
            // reversed by a later close.
            self.commit_or_closed(&context)?;
            self.state.publish(published.clone());
            return Ok(published);
        }

        let now = self.clock.now();
        if let Some(fresh) = self.state.serve_fresh(now) {
            return Ok(fresh);
        }

        match self.flight.try_lead() {
            Some(token) => {
                // Leader: run one generation, then publish its committed outcome.
                // The guard completes the generation even if this future is
                // dropped, so waiters can never deadlock on an abandoned leader.
                let guard = LeaderGuard {
                    flight: &self.flight,
                    token,
                    completed: false,
                };
                let outcome = self.run_leader(&context).await;
                // Publication is a success and goes through the same lifecycle
                // linearization gate as a numeric publish: a close that wins the
                // gate turns the completed result into `Closed` instead of
                // publishing, so no value can be committed after close wins.
                let outcome = match outcome {
                    Ok(published) => match self.commit_or_closed(&context) {
                        Ok(()) => {
                            self.state.publish(published.clone());
                            Ok(published)
                        }
                        Err(error) => {
                            self.state.record_refresh_failure(error.clone());
                            Err(error)
                        }
                    },
                    Err(error) => {
                        self.state.record_refresh_failure(error.clone());
                        Err(error)
                    }
                };
                guard.complete(outcome.clone());
                outcome
            }
            None => {
                // Waiter: attach to the specific in-flight generation and
                // observe this caller's own cancellation, owner shutdown, and
                // deadline while waiting, so a stalled leader cannot pin a
                // waiter forever and a superseded generation is never adopted.
                match self.flight.current_generation() {
                    Some(token) => self.wait_for_generation(&context, token).await,
                    // The generation finished between the two lock acquisitions;
                    // this caller may simply lead its own.
                    None => Err(ResolverError::AlreadyResolving),
                }
            }
        }
    }

    /// Completes the lifecycle linearization gate for a publication.
    ///
    /// Owner close, caller cancellation, and the caller's original absolute
    /// deadline are evaluated under the same lifecycle lock as registration and
    /// `begin_close`, so a close that wins the gate always prevents the
    /// publication and a committed publication can never be reversed by a later
    /// close.
    fn commit_or_closed(&self, context: &ExchangeContext) -> Result<(), ResolverError> {
        self.lifecycle
            .commit_final_response(
                &context.cancellation(),
                context.deadline(),
                crate::SideEffectState::Sent,
            )
            .map_err(|error| match error {
                crate::UpstreamError::Closed(_) => ResolverError::Closed,
                crate::UpstreamError::Cancelled(_) => ResolverError::Cancelled,
                _ => ResolverError::BootstrapTimeout,
            })
    }

    /// Waits on the current generation while independently honoring this
    /// caller's cancellation and absolute deadline and the owner's shutdown.
    async fn wait_for_generation(
        &self,
        context: &ExchangeContext,
        token: GenerationId,
    ) -> Result<PublishedTarget, ResolverError> {
        let caller_token = context.cancellation();
        let owner = self.cancellation.cancelled();
        let caller = caller_token.cancelled();
        tokio::pin!(owner);
        tokio::pin!(caller);

        let timer = tokio::time::sleep_until(tokio::time::Instant::from_std(context.deadline()));
        tokio::pin!(timer);

        // Register a liveness hold so an owner close drains this waiter too.
        let _guard = self
            .lifecycle
            .register_owned()
            .map_err(|_| ResolverError::Closed)?;

        tokio::select! {
            biased;
            () = &mut owner => Err(ResolverError::Closed),
            () = &mut caller => Err(ResolverError::Cancelled),
            () = &mut timer => Err(ResolverError::BootstrapTimeout),
            result = self.flight.wait(token) => result,
        }
    }

    /// Applies owner shutdown, caller cancellation, then the absolute deadline.
    fn check_control(
        &self,
        context: &ExchangeContext,
        side_effect: crate::SideEffectState,
    ) -> Result<(), ResolverError> {
        if self.cancellation.is_cancelled() {
            return Err(ResolverError::Closed);
        }
        context
            .check_at(Instant::now(), side_effect)
            .map_err(|error| match error {
                crate::UpstreamError::Cancelled(_) => ResolverError::Cancelled,
                crate::UpstreamError::Closed(_) => ResolverError::Closed,
                _ => ResolverError::BootstrapTimeout,
            })
    }

    /// Runs one leader generation: the bounded bootstrap exchange, then the
    /// atomic publication of a complete validated result.
    async fn run_leader(
        &self,
        context: &ExchangeContext,
    ) -> Result<PublishedTarget, ResolverError> {
        // Register with the lifecycle so an owner close drains this resolution.
        let _guard = self
            .lifecycle
            .register_owned()
            .map_err(|_| ResolverError::Closed)?;

        let control = crate::ExchangeControl::new(context.clone(), self.cancellation.child_token());
        let answer = super::bootstrap::exchange(
            self.target.host(),
            self.target.family(),
            self.bootstrap,
            &self.policy,
            &control,
            self.ids.as_ref(),
        )
        .await?;

        // Final control check before the completed result is handed back: no
        // post-terminal result, and owner close wins. The publication itself
        // goes through the lifecycle gate in `resolve`.
        self.check_control(context, crate::SideEffectState::Sent)?;

        let ttl = self.policy.clamp_ttl_secs(answer.ttl_secs);
        let destination = ResolvedDestination::new(
            answer.address,
            self.target.family(),
            u32::try_from(ttl.as_secs()).map_err(|_| ResolverError::InvalidTtl)?,
            self.clock.now(),
        )?;

        Ok(PublishedTarget::new(self.target.clone(), destination))
    }
}

/// RAII ownership of one refresh generation.
///
/// A leader holds this guard for the whole generation. If the leader's future is
/// dropped or aborted, the guard still completes the generation with a typed
/// failure and wakes every waiter, so a waiter can never deadlock on a
/// generation that no longer has a leader. A later caller may then lead a fresh
/// one.
struct LeaderGuard<'a> {
    flight: &'a SingleFlight,
    token: GenerationId,
    completed: bool,
}

impl<'a> LeaderGuard<'a> {
    /// Commits this generation exactly once and wakes the waiters.
    fn complete(mut self, result: Result<PublishedTarget, ResolverError>) {
        self.completed = true;
        self.flight.complete(self.token, result);
    }
}

impl Drop for LeaderGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.flight.complete(self.token, SingleFlight::ABANDONED);
        }
    }
}

/// Explicit composition helpers from a published numeric destination into the
/// existing numeric UDP/TCP and secure endpoints.
///
/// Every constructor below takes the numeric dial address from resolution and
/// keeps the service identity supplied by the caller exactly as it was
/// validated: resolution never rewrites TLS SNI or a DoH URL authority.
pub struct ResolverComposition;

impl ResolverComposition {
    /// Builds a plain UDP or TCP endpoint at the resolved numeric address.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] when the published port is zero,
    /// which the numeric [`Endpoint`] constructor also rejects.
    pub fn endpoint(
        published: &PublishedTarget,
        transport: Transport,
    ) -> Result<Endpoint, ResolverError> {
        Endpoint::new(published.dial(), transport).map_err(|_| ResolverError::ZeroPort)
    }

    /// Builds a DoT endpoint that dials the resolved numeric address while
    /// keeping the caller's original TLS service identity.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] when the published port is zero.
    pub fn dot_endpoint(
        published: &PublishedTarget,
        identity: &crate::ServerIdentity,
    ) -> Result<DotEndpoint, ResolverError> {
        DotEndpoint::new(published.dial(), identity.clone()).map_err(|_| ResolverError::ZeroPort)
    }

    /// Builds a DoH endpoint that dials the resolved numeric address while
    /// keeping the caller's original service URL, authority, and path.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] when the published port is zero.
    pub fn doh_endpoint(
        published: &PublishedTarget,
        service_url: &str,
    ) -> Result<DohEndpoint, ResolverError> {
        DohEndpoint::new(service_url, published.dial()).map_err(|_| ResolverError::ZeroPort)
    }
}

/// A resolver paired with the plain upstream it resolved for.
///
/// This is the minimum composition the existing transports need: the resolved
/// numeric address feeds their numeric dial boundary, and nothing else about
/// them changes.
pub struct ResolvedUpstream {
    upstream: Upstream,
    published: PublishedTarget,
}

impl ResolvedUpstream {
    /// Wraps a resolved publication in the plain transport it dials.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] for a zero published port.
    pub fn new(published: PublishedTarget, transport: Transport) -> Result<Self, ResolverError> {
        let endpoint = ResolverComposition::endpoint(&published, transport)?;
        Ok(Self {
            upstream: Upstream::new(endpoint),
            published,
        })
    }

    /// The plain transport bound to the resolved numeric address.
    #[must_use]
    pub const fn upstream(&self) -> &Upstream {
        &self.upstream
    }

    /// The publication this upstream was built from.
    #[must_use]
    pub const fn published(&self) -> &PublishedTarget {
        &self.published
    }

    /// The numeric address the transport dials.
    #[must_use]
    pub fn dial(&self) -> SocketAddr {
        self.published.dial()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{BootstrapResolver, ResolverComposition, SingleFlight};
    use crate::resolver::{
        AddressFamily, BootstrapEndpoint, Clock, ConfigVersion, ResolutionPolicy, ResolutionTarget,
        ResolverError,
    };
    use crate::{ServerIdentity, Transport};

    struct FixedClock(Instant);

    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    fn resolver(host: &str) -> BootstrapResolver {
        BootstrapResolver::new(
            ResolutionTarget::new(host, 853, AddressFamily::Ipv4).expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver")
    }

    fn resolver_under_test() -> BootstrapResolver {
        BootstrapResolver::with_deterministic_ids_for_tests(
            ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4).expect("t"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("b"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver")
    }

    #[test]
    fn numeric_target_composes_without_dns() {
        let resolver = resolver("192.0.2.5");
        let published =
            super::super::resolve_numeric("192.0.2.5:853".parse().expect("addr")).expect("literal");
        assert_eq!(resolver.policy().min_ttl(), Duration::from_secs(300));
        let endpoint = ResolverComposition::endpoint(&published, Transport::Udp).expect("endpoint");
        assert_eq!(endpoint.address(), "192.0.2.5:853".parse().expect("addr"));
    }

    #[test]
    fn dot_composition_keeps_the_service_identity() {
        let published =
            super::super::resolve_numeric("192.0.2.5:853".parse().expect("addr")).expect("literal");
        let identity = ServerIdentity::new("bootstrap.example.org").expect("identity");
        let dot = ResolverComposition::dot_endpoint(&published, &identity).expect("dot");
        assert_eq!(dot.dial(), "192.0.2.5:853".parse().expect("addr"));
        assert_eq!(dot.identity().dns_name(), Some("bootstrap.example.org"));
    }

    #[test]
    fn doh_composition_keeps_the_service_url_authority() {
        let published =
            super::super::resolve_numeric("192.0.2.5:443".parse().expect("addr")).expect("literal");
        let doh =
            ResolverComposition::doh_endpoint(&published, "https://doh.example.org/dns-query")
                .expect("doh");
        assert_eq!(doh.dial(), "192.0.2.5:443".parse().expect("addr"));
        assert_eq!(doh.host(), "doh.example.org");
        assert_eq!(doh.path(), "/dns-query");
    }

    #[test]
    fn bootstrap_family_must_match_the_target_family() {
        let error = BootstrapResolver::new(
            ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv6)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .err()
        .expect("family mismatch");
        assert_eq!(error, ResolverError::BootstrapFamilyMismatch);
    }

    /// A waiter that attached to generation N must never observe generation N+1.
    ///
    /// This is the exact interleaving the audit found: a waiter whose own
    /// generation already finished can race a new leader's `try_lead`, see
    /// `running` with no result recorded yet, and adopt the newer generation as
    /// if it were its own. The token makes that impossible.
    #[tokio::test]
    async fn a_waiter_attached_to_a_finished_generation_cannot_adopt_the_next_one() {
        let flight = SingleFlight::new();

        // Generation 1 runs and then fails, so its result is recorded.
        let first = flight.try_lead().expect("leader of generation 1");
        flight.complete(first, Err(ResolverError::BootstrapTimeout));

        // A new leader takes generation 2 and is still running: `running` is
        // true and no result has been recorded. This is exactly the state a
        // token-less waiter would mistake for its own generation.
        let second = flight.try_lead().expect("leader of generation 2");
        assert_ne!(first, second, "generation 2 has its own token");

        // The stale generation-1 waiter must not adopt generation 2's live,
        // result-less state; it reports that its own generation is gone.
        assert_eq!(
            flight.wait(first).await,
            Err(ResolverError::AlreadyResolving),
            "a superseded generation is never adopted by its old waiter"
        );

        // The live token still observes its own committed result.
        let target = super::super::ResolutionTarget::new("192.0.2.1", 853, AddressFamily::Ipv4)
            .expect("target");
        let destination = super::super::ResolvedDestination::new_literal(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1)),
            AddressFamily::Ipv4,
        );
        let published = super::super::PublishedTarget::new(target, destination);
        flight.complete(second, Ok(published.clone()));
        assert_eq!(flight.wait(second).await, Ok(published));
    }

    /// A waiter attached to a generation that finished receives its own result.
    #[tokio::test]
    async fn a_waiter_attached_to_a_finished_generation_receives_its_result() {
        let flight = SingleFlight::new();
        let token = flight.try_lead().expect("leader");
        flight.complete(token, Err(ResolverError::BootstrapTimeout));
        assert_eq!(
            flight.wait(token).await,
            Err(ResolverError::BootstrapTimeout),
            "the waiter sees its own generation's typed failure"
        );
    }

    /// A stale leader's completion cannot overwrite a newer generation.
    #[test]
    fn a_superseded_leader_cannot_complete_a_newer_generation() {
        let flight = SingleFlight::new();
        let first = flight.try_lead().expect("generation 1");
        flight.complete(first, Err(ResolverError::BootstrapTimeout));
        let second = flight.try_lead().expect("generation 2");

        // The generation-1 token is stale; completing it must be a no-op.
        flight.complete(first, Err(ResolverError::Cancelled));
        let generation = flight.lock();
        assert!(generation.running, "generation 2 is still running");
        assert!(generation.result.is_none(), "no stale result was recorded");
        assert_eq!(generation.id, second.0);
    }

    /// The production construction path never selects the predictable source.
    #[test]
    fn the_default_construction_uses_unpredictable_ids() {
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4).expect("t"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("b"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("production resolver");
        assert!(resolver.uses_unpredictable_ids());
        let deterministic = resolver_under_test();
        assert!(!deterministic.uses_unpredictable_ids());
    }

    #[test]
    fn config_version_family_is_reused_by_the_resolver_model() {
        assert_eq!(
            ConfigVersion::from_u8(4).unwrap().family(),
            AddressFamily::Ipv4
        );
    }
}
