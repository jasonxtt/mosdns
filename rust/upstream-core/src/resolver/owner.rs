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

/// The outcome of one admission decision, taken under a single lock.
///
/// The generation handle is always the one the caller actually observed, so a
/// caller can never be admitted against a generation it did not see.
enum Admission {
    /// This caller owns a fresh generation and must run it.
    Leader(Arc<GenerationState>),
    /// A generation is already running; wait on exactly this generation.
    Waiter(Arc<GenerationState>),
    /// A generation already finished with an unclaimed result to serve.
    Result(Result<PublishedTarget, ResolverError>),
}

/// The owned state of one refresh generation.
///
/// Every participant holds an [`Arc`] to this rather than reading a shared slot,
/// which is what makes a registered waiter's guarantee unconditional: the result
/// it is entitled to lives inside the handle it already owns, so no later caller,
/// eviction, or generation advance can take it away. PRD 5.4 requires exactly
/// that every caller admitted to a generation waits on *that* generation.
///
/// Generation identity is the allocation itself: `admit` hands out one
/// [`Arc`] per generation and `complete` compares by pointer, so a handle can
/// never be confused with another generation's, and no separate token needs to
/// be kept in step with it.
#[derive(Debug)]
struct GenerationState {
    /// The leader's outcome, written exactly once while the state lock is held.
    /// Read only after [`Self::done`] fires, so a waiter never blocks on the lock
    /// to observe it.
    result: std::sync::OnceLock<Result<PublishedTarget, ResolverError>>,
    /// Fires once when `result` has been written.
    done: Notify,
}

impl GenerationState {
    fn new() -> Self {
        Self {
            result: std::sync::OnceLock::new(),
            done: Notify::new(),
        }
    }

    /// The committed outcome, if the leader has completed.
    fn committed(&self) -> Option<Result<PublishedTarget, ResolverError>> {
        self.result.get().cloned()
    }
}

/// The owner's admission state for the single refresh generation.
#[derive(Debug, Default)]
struct Generation {
    /// The generation a leader is currently running, if any.
    live: Option<Arc<GenerationState>>,
    /// The most recently completed generation, retained for a late caller (and
    /// for any waiter still holding its handle).
    completed: Option<Arc<GenerationState>>,
    /// Whether a late caller has already been served `completed`'s result. This
    /// only stops a finished result from being handed out *forever*; it never
    /// removes the result from the handle its registered waiters already hold.
    completed_claimed: bool,
}

/// Single-flight generation state shared by leader and waiters.
///
/// One short synchronous mutex guards admission, completion, and the
/// live/completed handles. No await ever happens while it is held, so the owner
/// cannot deadlock on its own state.
#[derive(Debug)]
struct SingleFlight {
    inner: Mutex<Generation>,
}

impl SingleFlight {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Generation::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Generation> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Admits a caller as the leader of a fresh generation, or as a waiter on the
    /// live one, or as a one-time consumer of the last completed result — all in
    /// a single lock acquisition.
    ///
    /// Deciding the role and handing back the generation handle happen under one
    /// lock, so the handle a caller receives always names the generation it
    /// observed. Splitting them is the race this prevents: a caller could see a
    /// live generation, lose the lock while that generation completed and a new
    /// one started, and then attach to the *new* generation.
    fn admit(&self) -> Admission {
        let mut state = self.lock();
        if let Some(live) = &state.live {
            // A live generation exists: hand back its exact handle.
            return Admission::Waiter(Arc::clone(live));
        }
        // A finished generation still holds its result inside its own handle, so
        // a late caller may be served it once. This is only a courtesy to a
        // caller that arrives after the fact; it never affects a waiter that
        // already holds the handle, because the result is not removed.
        if !state.completed_claimed {
            if let Some(completed) = &state.completed {
                if let Some(result) = completed.committed() {
                    state.completed_claimed = true;
                    return Admission::Result(result);
                }
            }
        }
        // Nothing live and nothing unclaimed to serve: this caller leads a fresh
        // generation, which is the only place the previous handle is retired.
        let generation = Arc::new(GenerationState::new());
        state.live = Some(Arc::clone(&generation));
        state.completed = None;
        state.completed_claimed = false;
        Admission::Leader(generation)
    }

    /// The outcome a dropped leader publishes, so waiters can never deadlock on
    /// a generation whose leader future was abandoned or aborted.
    const ABANDONED: Result<PublishedTarget, ResolverError> = Err(ResolverError::Cancelled);

    /// Commits the leader's outcome into `generation` and wakes its waiters.
    ///
    /// A completion is only recorded if this generation is still the live one, so
    /// a stale leader can never write into a newer generation's handle. The
    /// write goes to the handle itself, which every waiter already holds, and is
    /// published exactly once.
    fn complete(
        &self,
        generation: &Arc<GenerationState>,
        result: Result<PublishedTarget, ResolverError>,
    ) {
        let mut state = self.lock();
        let is_live = state
            .live
            .as_ref()
            .is_some_and(|live| Arc::ptr_eq(live, generation));
        if !is_live {
            return;
        }
        // Publish the outcome while the state lock is still held, so no caller
        // can observe a generation that is no longer live yet has no result.
        // `admit` takes the same lock, so a caller either sees the generation as
        // live (and waits on it) or sees it completed with its result already
        // present — never the empty window between the two.
        let _ = generation.result.set(result);
        state.live = None;
        state.completed = Some(Arc::clone(generation));
        state.completed_claimed = false;
        drop(state);
        // Wake every waiter only after the result is visible under the lock.
        generation.done.notify_waiters();
    }

    /// Awaits the specific generation and returns its committed outcome.
    ///
    /// Because the result lives in the handle this waiter already owns, the
    /// guarantee is unconditional: once the generation completes, every waiter
    /// admitted to it observes that generation's result, no matter how many
    /// later callers were also served it. A waiter never adopts a newer
    /// generation's state.
    async fn wait(
        &self,
        generation: &Arc<GenerationState>,
    ) -> Result<PublishedTarget, ResolverError> {
        loop {
            let notified = generation.done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = generation.committed() {
                return result;
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
    /// The entropy probe is taken only when the target actually needs DNS. A
    /// numeric `dial_addr` target is usable immediately and must not depend on
    /// RNG availability at all, so it is constructed without probing and
    /// without ever drawing an ID.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::UnpredictableIdsUnavailable`] when a hostname
    /// target needs unpredictable IDs and none is available. The bootstrap
    /// peer's transport family and the target's answer family are independent,
    /// so a mismatch between them is not an error.
    pub fn new(
        target: ResolutionTarget,
        bootstrap: BootstrapEndpoint,
        policy: ResolutionPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ResolverError> {
        // Numeric targets bypass DNS entirely and therefore never draw an ID, so
        // they must not be gated on entropy their host may not have.
        if !target.is_numeric() && !OsIdSource.is_available() {
            return Err(ResolverError::UnpredictableIdsUnavailable);
        }
        Self::with_id_source(target, bootstrap, policy, clock, Arc::new(OsIdSource))
    }

    /// Builds a resolver whose ID source always fails to draw.
    ///
    /// Test-only: it proves the exchange surfaces a typed error instead of
    /// substituting a guessed identifier.
    ///
    /// # Errors
    ///
    /// As [`Self::with_id_source`].
    #[doc(hidden)]
    pub fn with_failing_ids_for_tests(
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
            Arc::new(super::bootstrap::FailingIdSource),
        )
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
    #[doc(hidden)]
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
    pub(crate) fn with_id_source(
        target: ResolutionTarget,
        bootstrap: BootstrapEndpoint,
        policy: ResolutionPolicy,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn ResolutionIdSource>,
    ) -> Result<Self, ResolverError> {
        // The bootstrap peer's transport family and the answer family are
        // independent: `bootstrap` is its own numeric UDP endpoint, and
        // `bootstrap_version` only selects whether the query asks A or AAAA. An
        // IPv4 bootstrap server may answer AAAA and an IPv6 one may answer A, so
        // there is deliberately no equality invariant between them. The exchange
        // binds and connects in the peer's own family and passes the target's
        // answer family separately.
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

        // Admission decides leadership and reads the generation token under one
        // lock, so a waiter can never attach to a generation other than the one
        // it observed. A finished-but-unclaimed generation result is handed back
        // here; if that result is a still-fresh success this caller serves it
        // instead of repeating the query, and otherwise this caller tries to lead
        // a fresh generation. The loop is bounded because `admit` consumes the
        // result slot, so the next iteration yields `Leader` or `Waiter`.
        loop {
            match self.flight.admit() {
                Admission::Leader(generation) => {
                    let guard = LeaderGuard {
                        flight: &self.flight,
                        generation,
                        completed: false,
                    };
                    // Re-read the publication now that this caller is the
                    // admitted leader: a generation may have completed and
                    // published between the `serve_fresh` check above and this
                    // admission. Without this recheck the new leader would repeat
                    // a DNS query for a value that is already fresh. The
                    // generation is completed with the observed value so
                    // concurrent waiters receive it too.
                    if let Some(fresh) = self.state.serve_fresh(self.clock.now()) {
                        guard.complete(Ok(fresh.clone()));
                        return Ok(fresh);
                    }
                    let outcome = self.run_leader(&context).await;
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
                    return outcome;
                }
                Admission::Waiter(generation) => {
                    // A live generation owns the work; attach to exactly this
                    // generation and observe this caller's own controls while
                    // waiting. The handle carries the result, so this waiter is
                    // guaranteed to observe its own generation's outcome even if
                    // a later caller is served the same result first.
                    return self.wait_for_generation(&context, &generation).await;
                }
                Admission::Result(result) => {
                    self.check_control(&context, crate::SideEffectState::Sent)?;
                    // A concurrent success is served without re-querying, but only
                    // while it is genuinely fresh: an expired value must never be
                    // returned as success. A failed generation's result is not
                    // returned either, so this caller may retry by leading a new
                    // generation.
                    // Written without a let-chain so this stays valid on the
                    // workspace MSRV: let-chains are not stable until later.
                    let fresh = match &result {
                        Ok(published) => !published.is_expired(self.clock.now()),
                        Err(_) => false,
                    };
                    if fresh {
                        return result;
                    }
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
        generation: &Arc<GenerationState>,
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
            result = self.flight.wait(generation) => result,
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
    generation: Arc<GenerationState>,
    completed: bool,
}

impl<'a> LeaderGuard<'a> {
    /// Commits this generation exactly once and wakes the waiters.
    fn complete(mut self, result: Result<PublishedTarget, ResolverError>) {
        self.completed = true;
        self.flight.complete(&self.generation, result);
    }
}

impl Drop for LeaderGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.flight
                .complete(&self.generation, SingleFlight::ABANDONED);
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

    use super::{Admission, BootstrapResolver, ResolverComposition, SingleFlight};
    use crate::resolver::{
        AddressFamily, BootstrapEndpoint, Clock, ConfigVersion, PublishedTarget, ResolutionPolicy,
        ResolutionTarget, ResolvedDestination, ResolverError,
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
    fn bootstrap_and_target_families_are_independent() {
        // An IPv6 answer family may be requested through an IPv4 bootstrap peer:
        // bootstrap is a numeric UDP endpoint, and `bootstrap_version` only picks
        // A vs AAAA. No equality invariant exists between the two families.
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv6)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("an IPv4 bootstrap may serve an AAAA target");
        assert_eq!(resolver.target().family(), AddressFamily::Ipv6);
        assert_eq!(resolver.bootstrap().family(), AddressFamily::Ipv4);

        // And the mirror case: an IPv6 bootstrap peer serving an A target.
        let mirror = BootstrapResolver::new(
            ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new("::1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("an IPv6 bootstrap may serve an A target");
        assert_eq!(mirror.bootstrap().family(), AddressFamily::Ipv6);
    }

    /// A [`PublishedTarget`] for a loopback-resolvable numeric literal.
    fn published_pair(ip: [u8; 4]) -> PublishedTarget {
        let target = ResolutionTarget::new("192.0.2.1", 853, AddressFamily::Ipv4).expect("target");
        let destination = ResolvedDestination::new_literal(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])),
            AddressFamily::Ipv4,
        );
        PublishedTarget::new(target, destination)
    }

    /// The exact race the controller audit named: a late caller must not be able
    /// to consume a completed generation's result out from under a waiter that
    /// was already admitted to that generation. PRD 5.4 requires every caller
    /// admitted to a generation to wait on *that* generation.
    ///
    /// Sequence, with the lock released exactly where the bug lived:
    ///   W admits -> Waiter(gen 1)   [W is registered on generation 1]
    ///   leader completes gen 1 -> result stored, waiters notified
    ///   a NEW caller admits -> must not strip generation 1's result
    ///   W waits on its own handle -> still observes generation 1's outcome
    ///
    /// This is deterministic: the ordering is fixed by the calls themselves, and
    /// no sleep, timeout, or scheduler timing is involved.
    #[tokio::test]
    async fn a_late_caller_cannot_steal_a_completed_result_from_a_registered_waiter() {
        let flight = SingleFlight::new();

        // Generation 1 is live and W is admitted to it.
        let live = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("the first caller leads generation 1"),
        };
        let waiter = match flight.admit() {
            Admission::Waiter(generation) => generation,
            _ => panic!("the second caller waits on generation 1"),
        };
        assert!(Arc::ptr_eq(&live, &waiter), "W is bound to generation 1");

        // Generation 1 completes with a real success.
        let expected = published_pair([192, 0, 2, 71]);
        flight.complete(&live, Ok(expected.clone()));

        // A brand-new caller arrives after the notification and is served the
        // result. Under the previous destructive `take()`, this is what emptied
        // the slot the registered waiter was about to read.
        match flight.admit() {
            Admission::Result(result) => assert_eq!(result, Ok(expected.clone())),
            _ => panic!("the late caller is served the finished result"),
        }

        // The registered waiter must STILL observe generation 1's own result.
        assert_eq!(
            flight.wait(&waiter).await,
            Ok(expected),
            "a registered waiter keeps its generation's result after a late caller is served it"
        );
    }

    /// The same interleaving with a failed generation: the registered waiter
    /// still sees its own typed failure, and because a failure is never handed
    /// out as a stand-in success, a later caller can lead a fresh generation and
    /// retry rather than being told it already succeeded.
    #[tokio::test]
    async fn a_failed_generation_still_serves_its_waiter_and_still_allows_retry() {
        let flight = SingleFlight::new();

        let live = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("leader"),
        };
        let waiter = match flight.admit() {
            Admission::Waiter(generation) => generation,
            _ => panic!("waiter"),
        };
        flight.complete(&live, Err(ResolverError::BootstrapTimeout));

        // A late caller is handed this generation's typed failure ...
        match flight.admit() {
            Admission::Result(result) => {
                assert_eq!(result, Err(ResolverError::BootstrapTimeout));
            }
            _ => panic!("the failure is this generation's outcome"),
        }

        // ... the registered waiter still observes its own generation ...
        assert_eq!(
            flight.wait(&waiter).await,
            Err(ResolverError::BootstrapTimeout)
        );

        // ... and a later caller can still lead a new generation to retry.
        let retry = match flight.admit() {
            Admission::Leader(generation) => generation,
            Admission::Waiter(_) | Admission::Result(_) => {
                panic!("a failed generation must not block a retry")
            }
        };
        let recovered = published_pair([192, 0, 2, 72]);
        flight.complete(&retry, Ok(recovered.clone()));
        assert_eq!(flight.wait(&retry).await, Ok(recovered));
    }

    /// Admission decides leadership and hands back the generation handle in one
    /// observation, so a caller can never attach to a generation it did not see.
    #[tokio::test]
    async fn admission_binds_a_waiter_to_the_generation_it_observed() {
        let flight = SingleFlight::new();

        let gen1 = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("the first caller leads generation 1"),
        };
        let waiter = match flight.admit() {
            Admission::Waiter(generation) => generation,
            _ => panic!("the second caller waits on generation 1"),
        };
        assert!(
            Arc::ptr_eq(&waiter, &gen1),
            "the waiter observed generation 1"
        );

        flight.complete(&gen1, Err(ResolverError::BootstrapTimeout));

        // A later caller leads generation 2, which retires generation 1's handle
        // from the owner's slot but cannot invalidate the waiter's own handle.
        let _ = flight.admit();
        let gen2 = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("generation 2 is led"),
        };
        assert!(
            !Arc::ptr_eq(&gen2, &gen1),
            "generation 2 is a distinct handle"
        );

        // The generation-1 waiter keeps its own outcome and never adopts
        // generation 2's state.
        assert_eq!(
            flight.wait(&waiter).await,
            Err(ResolverError::BootstrapTimeout),
            "the waiter keeps its own generation's outcome"
        );

        let gen2_result = published_pair([192, 0, 2, 73]);
        flight.complete(&gen2, Ok(gen2_result.clone()));
        assert_eq!(flight.wait(&gen2).await, Ok(gen2_result));
    }

    /// A caller arriving after a generation already completed adopts that
    /// generation's result instead of starting a duplicate query.
    #[tokio::test]
    async fn admission_serves_an_unclaimed_result_instead_of_requerying() {
        let flight = SingleFlight::new();
        let published = published_pair([192, 0, 2, 74]);

        let leader = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("the first caller leads"),
        };
        flight.complete(&leader, Ok(published.clone()));

        // A caller that arrives now must be handed the finished result, not
        // admitted as a leader that would repeat the query.
        match flight.admit() {
            Admission::Result(result) => assert_eq!(result, Ok(published)),
            Admission::Leader(_) => panic!("a completed success must not start a new query"),
            Admission::Waiter(_) => panic!("no generation is running"),
        }
    }

    /// `complete` publishes the result and retires the generation under one lock
    /// hold, so no caller can observe a generation that is neither live nor
    /// finished, and therefore no waiter can miss a result that was announced.
    #[test]
    fn completion_never_exposes_a_generation_without_a_result() {
        let flight = SingleFlight::new();
        let leader = match flight.admit() {
            Admission::Leader(generation) => generation,
            _ => panic!("leader"),
        };
        flight.complete(&leader, Err(ResolverError::BootstrapTimeout));

        // After completion the generation is finished AND its result is present:
        // both were done inside the same critical section.
        let state = flight.lock();
        assert!(state.live.is_none(), "the generation is no longer live");
        assert!(
            state.completed.is_some(),
            "the completed generation is retained"
        );
        assert!(
            state
                .completed
                .as_ref()
                .expect("completed")
                .committed()
                .is_some(),
            "the result is stored in the same lock hold that retired the generation"
        );
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
