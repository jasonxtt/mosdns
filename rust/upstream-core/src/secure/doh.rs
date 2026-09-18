//! One-exchange DNS-over-HTTPS primitive over HTTP/1.1 and HTTP/2 (Phase 4 Slice3).
//!
//! [`DohUpstream`] owns a [`DohEndpoint`] and an explicit [`TlsPolicy`]. Each
//! exchange opens exactly one fresh TCP connection to the endpoint's **numeric**
//! dial address, authenticates it with a TLS handshake against the **service
//! URL identity**, and then performs exactly one HTTPS `GET` whose `dns`
//! parameter carries the query.
//!
//! Ordering is the contract, not an implementation detail:
//!
//! 1. Numeric connect. The caller supplied a `SocketAddr`; the service identity
//!    never selects the destination.
//! 2. Authenticated TLS handshake against the service URL host. ALPN selects
//!    HTTP/2 or HTTP/1.1; absent ALPN means HTTP/1.1 on the established stream.
//! 3. Exactly one `GET`, with no request body, no `User-Agent`, and no
//!    `Content-Encoding`.
//! 4. The response must be `200`, `application/dns-message`, identity-encoded,
//!    complete, and at most 65535 bytes. It is then validated by `dns-core`.
//!
//! Consequently a handshake failure is a typed TLS error with
//! `SideEffectState::NotSent`, and no path exists that sends the query in
//! plaintext, follows a redirect, retries, pools the connection, or falls back
//! to another protocol.
//!
//! This slice uses Hyper's low-level connection APIs. HTTP/1.1 is polled inline;
//! HTTP/2 receives a sealed, tracked executor so every child future is owned and
//! drained, with no pool or hidden retry.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use hyper::body::Incoming;
use hyper::header::{ACCEPT, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use mosdns_dns_core::{inspect_response_header, validate_response};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::secure::dot::{SecureHttpVersion, SecureResponse};
use crate::secure::endpoint::DohEndpoint;
use crate::secure::error::{DohProtocolError, SecureError};
use crate::secure::tls::{TlsPolicy, classify_handshake_error, server_name_for};
use crate::tcp::race_control;
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl,
    ExchangeRequest, Lifecycle, LifecycleState, SharedInFlightGuard, SideEffectState,
    TransportCancellation, UpstreamError,
};

/// The `application/dns-message` media type, compared case-insensitively.
const DNS_MEDIA_TYPE: &str = "application/dns-message";

/// The largest DNS message a DoH response may carry: 65535 bytes.
const MAX_DNS_BODY: usize = 65_535;

/// The largest response header block this client will accept: 16 KiB.
///
/// This is a bound on the *bytes* of the response head, not on the number of
/// headers. A peer could otherwise send a small number of enormous headers, or
/// many small ones, and still make the client hold far more than the DNS
/// contract needs. The bound is applied to the parsed head, so it holds however
/// the peer chose to distribute those bytes across headers.
const MAX_RESPONSE_HEADER_BYTES: usize = 16 * 1024;

/// The largest number of individual response headers this client will accept.
///
/// This complements the byte bound: it stops a peer from making the client
/// allocate a huge number of tiny header entries that individually stay well
/// under the byte ceiling. The DNS contract needs only a handful.
const MAX_RESPONSE_HEADERS: usize = 64;

/// The Hyper HTTP/1.1 parser/read-buffer ceiling: the same 16 KiB the contract
/// allows for a response head.
///
/// This is the *raw wire* bound. Hyper aborts the connection once the bytes it
/// has buffered for a single head reach this limit, so the parser itself cannot
/// hold more than the contract permits. It is deliberately not larger than
/// [`MAX_RESPONSE_HEADER_BYTES`]: a bigger buffer would let a peer make the
/// parser hold more head bytes than the contract accepts, which a post-parse
/// check could only detect after the allocation had already happened.
///
/// Hyper requires at least 8192, which is well below this value.
const MAX_HTTP1_BUFFER: usize = MAX_RESPONSE_HEADER_BYTES;

/// The ALPN protocol list offered for DoH, in preference order.
///
/// HTTP/2 is selected only when the scoped driver below is able to account for
/// every future Hyper submits to its executor. HTTP/1.1 remains the fallback
/// when it is selected or when the peer omits ALPN.
const DOH_ALPN: &[&[u8]] = &[b"h2", b"http/1.1"];

/// Shared state for the futures Hyper submits while building an HTTP/2
/// connection. Hyper's HTTP/2 connection future is a dispatcher: its actual
/// connection, request-send, and body-pipe futures are handed to the supplied
/// executor. This registry makes those otherwise hidden children owned work.
struct H2ChildState {
    children: Mutex<H2Children>,
    drained: tokio::sync::Notify,
    /// The owner registration this scope keeps alive, when the scope is bounded
    /// by one exchange.
    ///
    /// The fresh path holds it so an aborted caller cannot release the owner's
    /// registration while tracked children still exist. A **pooled** scope
    /// deliberately holds none: it outlives individual exchanges, so holding a
    /// registration for its whole life would keep the owner permanently
    /// non-drained and `close()` could never converge. A pooled session's
    /// children are instead cleaned when the session is dropped, which seals and
    /// aborts them.
    _liveness: Option<Arc<SharedInFlightGuard>>,
    scope_cancellation: TransportCancellation,
    owner_cancellation: TransportCancellation,
    /// The caller cancellation this scope's children are raced against.
    ///
    /// A **pooled** scope always receives a fresh token that no caller can
    /// cancel. A pooled session outlives the request that opened it, so capturing
    /// that request's caller token would tie the long-lived connection driver to
    /// one caller's lifetime: the driver is itself a tracked child, so a later
    /// cancellation of the *first* caller's token would kill the driver and break
    /// every subsequent reuse.
    ///
    /// Nothing is lost by not wiring it to a caller. Cancellation for the
    /// exchange actually in progress is enforced by that exchange's own
    /// [`ExchangeControl`](crate::ExchangeControl) inside
    /// [`run_pooled_exchange`], and a failure there hands the session back as a
    /// discard, which seals this scope and aborts its children.
    caller_cancellation: TransportCancellation,
    /// Deterministic barrier used by tests to hold a drain open.
    ///
    /// It lives on the shared child state rather than on [`H2ScopeLease`] so it
    /// still applies when the scope's owner was dropped and only the surviving
    /// [`Arc<H2ChildState>`] handle is left to drain — which is exactly the
    /// aborted-exchange path the pooled-close regression exercises.
    #[cfg(test)]
    teardown_pause: Mutex<Option<Arc<H2TeardownPause>>>,
}

struct H2Children {
    sealed: bool,
    next_id: u64,
    active: usize,
    aborts: HashMap<u64, tokio::task::AbortHandle>,
}

/// Executor supplied to Hyper's low-level HTTP/2 handshake.
#[derive(Clone)]
struct TrackedH2Executor {
    state: Arc<H2ChildState>,
}

/// An exchange-held lease whose synchronous drop path seals admission and
/// aborts every child. The async finish path additionally waits for every
/// child guard to drop.
///
/// The lease is deliberately **not** the thing that drains. Its [`Drop`] can
/// only seal and abort, because dropping cannot await, and a caller that aborts
/// an exchange drops the lease while the future is still in flight. Draining is
/// therefore owned by the *resource* that spawned the children — for a pooled
/// session, the session itself, which exposes [`H2DrainHandle`] at connect time
/// so the owner can drain a scope whose lease has already been dropped.
pub(crate) struct H2ScopeLease {
    state: Arc<H2ChildState>,
}

/// A standalone handle that can seal and drain a pooled HTTP/2 child scope.
///
/// It clones the shared child state, **not** the lease, so it keeps working
/// after the exchange's `H2ScopeLease` has been dropped by an abort or an
/// explicit drop. The owner holds one for the whole lifetime of a pooled HTTP/2
/// session and uses it to finish teardown on every path that ends the session,
/// including the one where the exchange future never ran to completion.
///
/// The handle is `Clone` so an owner can start a drain **without removing the
/// registered scope**: if the attempt performing that drain is itself aborted
/// mid-await, the owner's scope registry still names the scope and `close()` can
/// still await it. A clone shares the same [`H2ChildState`], so draining through
/// any clone is the same teardown.
///
/// Draining is idempotent: sealing and aborting twice is harmless, and a second
/// drain simply observes an already-empty child set.
#[derive(Clone)]
pub(crate) struct H2DrainHandle {
    state: Arc<H2ChildState>,
}

impl H2DrainHandle {
    /// Seals admission, aborts every tracked child, and waits for the child
    /// count to reach zero.
    pub(crate) async fn finish(&self) {
        let lease = H2ScopeLease {
            state: Arc::clone(&self.state),
        };
        lease.finish().await;
    }

    /// The number of tracked children that have not finished yet.
    #[cfg(test)]
    pub(crate) fn active_children(&self) -> usize {
        self.state
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
    }

    /// Whether `other` names the same child scope as `self`.
    ///
    /// The owner keeps scopes in a shared list and must remove exactly the entry
    /// it drained, so identity is compared by pointer rather than by value.
    pub(crate) fn is_same_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    /// Installs the deterministic teardown barrier for this scope.
    #[cfg(test)]
    pub(crate) fn install_teardown_pause(&self, pause: Arc<H2TeardownPause>) {
        *self
            .state
            .teardown_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }
}

struct H2ChildGuard {
    state: Arc<H2ChildState>,
    id: u64,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct H2TeardownPause {
    /// Number of drains that have reached the barrier so far.
    arrivals: std::sync::Mutex<usize>,
    arrived_notify: tokio::sync::Notify,
    /// Set once the test releases the barrier. It is a flag rather than a single
    /// notification because the barrier must stay open: with the per-caller
    /// teardown, several `close()` calls drain the same scope, and every one of
    /// them parks here. A one-shot release would leave the later ones waiting.
    released: std::sync::atomic::AtomicBool,
    released_notify: tokio::sync::Notify,
}

#[cfg(test)]
impl H2TeardownPause {
    pub(crate) fn new() -> Self {
        Self {
            arrivals: std::sync::Mutex::new(0),
            arrived_notify: tokio::sync::Notify::new(),
            released: std::sync::atomic::AtomicBool::new(false),
            released_notify: tokio::sync::Notify::new(),
        }
    }

    async fn wait_until_released(&self) {
        // Register interest before re-checking, so a release between the two
        // cannot be missed; return immediately once the barrier has opened.
        let released = self.released_notify.notified();
        tokio::pin!(released);
        released.as_mut().enable();

        {
            let mut arrivals = self
                .arrivals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *arrivals += 1;
        }
        self.arrived_notify.notify_waiters();

        if self.released.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        released.await;
    }

    /// Waits until `count` drains have parked on this barrier.
    pub(crate) async fn wait_for_arrivals(&self, count: usize) {
        loop {
            let notified = self.arrived_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let seen = *self
                .arrivals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if seen >= count {
                return;
            }
            notified.await;
        }
    }

    /// Opens the barrier, releasing every parked drain and every later one.
    pub(crate) fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
        self.released_notify.notify_waiters();
    }
}

impl H2ScopeLease {
    /// Creates a scope bounded by one exchange, holding an owner registration.
    fn new(
        liveness: Arc<SharedInFlightGuard>,
        owner_cancellation: TransportCancellation,
        caller_cancellation: TransportCancellation,
    ) -> Self {
        Self::with_liveness(Some(liveness), owner_cancellation, caller_cancellation)
    }

    /// Creates a scope for a pooled session, holding no owner registration and no
    /// caller cancellation.
    ///
    /// A pooled session outlives individual exchanges, so it must not keep a
    /// registration that would prevent `close()` from draining, and it must not
    /// capture any single request's caller token — see the field docs on
    /// [`H2ChildState::caller_cancellation`]. It gets a fresh token no caller can
    /// cancel, so a later cancellation of the request that opened the session
    /// cannot kill the retained driver. Its children are still tracked and are
    /// sealed and aborted when the session is dropped or discarded.
    pub(crate) fn pooled(owner_cancellation: TransportCancellation) -> Self {
        Self::with_liveness(None, owner_cancellation, TransportCancellation::new())
    }

    fn with_liveness(
        liveness: Option<Arc<SharedInFlightGuard>>,
        owner_cancellation: TransportCancellation,
        caller_cancellation: TransportCancellation,
    ) -> Self {
        Self {
            state: Arc::new(H2ChildState {
                children: Mutex::new(H2Children {
                    sealed: false,
                    next_id: 0,
                    active: 0,
                    aborts: HashMap::new(),
                }),
                drained: tokio::sync::Notify::new(),
                _liveness: liveness,
                scope_cancellation: TransportCancellation::new(),
                owner_cancellation,
                caller_cancellation,
                #[cfg(test)]
                teardown_pause: Mutex::new(None),
            }),
        }
    }

    #[cfg(test)]
    fn install_teardown_pause(&self, pause: Arc<H2TeardownPause>) {
        *self
            .state
            .teardown_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }

    /// A standalone drain handle for this scope.
    fn drain_handle(&self) -> H2DrainHandle {
        H2DrainHandle {
            state: Arc::clone(&self.state),
        }
    }

    fn executor(&self) -> TrackedH2Executor {
        TrackedH2Executor {
            state: Arc::clone(&self.state),
        }
    }

    #[cfg(test)]
    fn active_children(&self) -> usize {
        self.state
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
    }

    fn seal_and_abort(&self) {
        self.state.scope_cancellation.cancel();
        let mut children = self
            .state
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        children.sealed = true;
        for handle in children.aborts.values() {
            handle.abort();
        }
    }

    async fn finish(&self) {
        self.seal_and_abort();
        #[cfg(test)]
        {
            // Clone rather than take: with the per-caller teardown, every
            // concurrent drain of the same scope should park on the same barrier,
            // so the test can observe that all of them wait.
            let teardown_pause = self
                .state
                .teardown_pause
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(pause) = teardown_pause {
                pause.wait_until_released().await;
            }
        }
        loop {
            let drained = {
                let children = self
                    .state
                    .children
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                children.active == 0
            };
            if drained {
                return;
            }
            let notified = self.state.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let still_drained = {
                let children = self
                    .state
                    .children
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                children.active == 0
            };
            if still_drained {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for H2ScopeLease {
    fn drop(&mut self) {
        self.seal_and_abort();
    }
}

impl H2ChildState {
    fn execute<F>(self: &Arc<Self>, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut children = self
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if children.sealed {
            return;
        }

        // Registration and tokio::spawn happen under the same mutex. A close
        // cannot seal the scope between accounting a child and giving Tokio
        // ownership of it.
        let id = children.next_id;
        children.next_id = children.next_id.wrapping_add(1);
        children.active += 1;
        let guard = H2ChildGuard {
            state: Arc::clone(self),
            id,
        };
        let scope_cancellation = self.scope_cancellation.clone();
        let owner_cancellation = self.owner_cancellation.clone();
        let caller_cancellation = self.caller_cancellation.clone();
        let join = tokio::spawn(async move {
            let _guard = guard;
            tokio::pin!(future);
            tokio::select! {
                biased;
                () = scope_cancellation.cancelled() => {},
                () = owner_cancellation.cancelled() => {},
                () = caller_cancellation.cancelled() => {},
                () = &mut future => {},
            }
        });
        children.aborts.insert(id, join.abort_handle());
    }
}

impl Drop for H2ChildGuard {
    fn drop(&mut self) {
        let mut children = self
            .state
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if children.aborts.remove(&self.id).is_some() {
            children.active = children.active.saturating_sub(1);
            if children.active == 0 {
                self.state.drained.notify_waiters();
            }
        }
    }
}

impl<F> hyper::rt::Executor<F> for TrackedH2Executor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.state.execute(future);
    }
}

/// A pure Rust DNS-over-HTTPS owner over HTTP/1.1 or HTTP/2.
///
/// The owner reuses the same [`Lifecycle`] admission/drain gate as the DoT and
/// plain transports: registration is serialized with `Open -> Closing`, and
/// [`Self::close`] refuses new exchanges and returns only after every
/// registered exchange has released its guard.
pub struct DohUpstream {
    endpoint: DohEndpoint,
    tls: TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    #[cfg(test)]
    pause: std::sync::Mutex<Option<std::sync::Arc<DohPause>>>,
}

impl DohUpstream {
    /// Creates a DoH owner from a validated endpoint and an explicit policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] when the policy cannot produce a
    /// usable client configuration. The endpoint was validated when it was
    /// constructed, so no socket, resolver, or handshake work happens here.
    pub fn new(endpoint: DohEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        // Reject an unusable policy at construction rather than on the first
        // exchange, so it is never silently accepted.
        tls.client_config()?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            #[cfg(test)]
            pause: std::sync::Mutex::new(None),
        })
    }

    #[must_use]
    pub const fn endpoint(&self) -> &DohEndpoint {
        &self.endpoint
    }

    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// Number of exchanges currently registered as in-flight.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }

    /// Begins owner shutdown and cancels the owner token.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
        }
        transition
    }

    /// Begins close, drains every in-flight exchange, then completes shutdown.
    ///
    /// No lock is held across the drain await, and no peer-dependent
    /// `close_notify` wait is performed: dropping the TLS stream is sufficient
    /// and cannot hold shutdown open.
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

    /// Performs one bounded authenticated DoH exchange over HTTP/1.1 or HTTP/2.
    ///
    /// The exchange registers as in-flight under the same gate that serializes
    /// `Open -> Closing`, so close can never observe a zero registration count
    /// while this exchange is admitting itself. The RAII guard is held until
    /// this future returns or is dropped, covering success, every terminal
    /// error, cancellation/deadline, owner close, and an aborted future.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::DohRequest`] for a pre-I/O request defect,
    /// [`SecureError::Tls`] when the handshake fails (always `NotSent`),
    /// [`SecureError::DohProtocol`] for an unacceptable HTTP reply, and
    /// [`SecureError::Transport`] wrapping the exact typed [`UpstreamError`]
    /// for connect, control, DNS-response, and body-read failures.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        // The request target is built before any socket work, so an unframeable
        // query or an over-long target fails as a pre-I/O `NotSent` defect.
        let target = self.endpoint.get_request_target(request)?;
        let authority = self.endpoint.authority();
        let prepared = self.prepare_exchange(request, context, target, authority)?;
        // Keep the public exchange future small even as the protocol-specific
        // HTTP/1.1 and tracked HTTP/2 state machines grow independently.
        Box::pin(exchange_inner(&prepared)).await
    }

    /// Registers and validates one exchange before any socket action.
    fn prepare_exchange<'a>(
        &'a self,
        request: ExchangeRequest<'a>,
        context: ExchangeContext,
        target: String,
        authority: String,
    ) -> Result<PreparedDoh<'a>, SecureError> {
        let in_flight = self.lifecycle.register()?;
        context.check_at(Instant::now(), SideEffectState::NotSent)?;
        Ok(PreparedDoh {
            endpoint: &self.endpoint,
            tls: &self.tls,
            lifecycle: Arc::clone(&self.lifecycle),
            request,
            target,
            authority,
            deadline: context.deadline(),
            caller_cancellation: context.cancellation(),
            owner_cancellation: self.cancellation.clone(),
            #[cfg(test)]
            pause: self.take_pause(),
            _in_flight: in_flight,
        })
    }

    /// Installs the deterministic phase seam for in-crate tests.
    #[cfg(test)]
    pub(crate) fn install_pause(&self, pause: std::sync::Arc<DohPause>) {
        *self
            .pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }

    /// Takes the installed phase seam for one exchange, if any.
    #[cfg(test)]
    fn take_pause(&self) -> Option<std::sync::Arc<DohPause>> {
        self.pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// A named phase of the DoH exchange.
///
/// Mirrors [`crate::secure::dot::DotPhase`]: the enum always exists so the phase
/// call sites compile in every build, while the seam that acts on it is
/// `cfg(test)` only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DohPhase {
    /// Immediately before the numeric connect is attempted.
    BeforeConnect,
    /// Immediately before the TLS handshake is attempted.
    BeforeHandshake,
    /// Immediately before the request is handed to the connection driver.
    BeforeRequest,
    /// After the request has been handed to the connection driver and before
    /// any response head has been observed.
    ///
    /// This is the window in which a request may or may not have reached the
    /// peer, so the side-effect state is conservatively `MaybeSent`.
    AfterRequestSent,
    /// Immediately before the response body is read to its end.
    BeforeBody,
    /// Immediately before the final control-aware commit.
    BeforeCommit,
    /// Immediately after the final commit has succeeded.
    AfterCommit,
}

/// Deterministic test seam that parks an exchange at one chosen phase.
///
/// Compiled only for in-crate tests. It carries no response bytes, socket, or
/// parser state, and exists so phase-ordering tests can prove an outcome
/// without sleeps.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct DohPause {
    phase: std::sync::Mutex<Option<DohPhase>>,
    arrived: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

#[cfg(test)]
impl DohPause {
    #[must_use]
    pub(crate) fn new(phase: DohPhase) -> Self {
        Self {
            arrived: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
            phase: std::sync::Mutex::new(Some(phase)),
        }
    }

    /// Parks if `phase` is the watched phase, then waits for [`Self::release`].
    ///
    /// The comparison must not consume the watched phase: the exchange visits
    /// every phase in order, so an earlier non-match has to leave the watched
    /// phase armed for the call that does match.
    async fn reach(&self, phase: DohPhase) {
        if self.watched_phase() != Some(phase) {
            return;
        }
        let released = self.released.notified();
        tokio::pin!(released);
        // Interest is registered before the arrival is announced, so a release
        // that arrives immediately afterwards cannot be missed.
        released.as_mut().enable();
        self.clear_phase();
        self.arrived.notify_one();
        released.await;
    }

    fn watched_phase(&self) -> Option<DohPhase> {
        *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn clear_phase(&self) {
        *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// Waits until an exchange has parked on this seam.
    pub(crate) async fn arrived(&self) {
        self.arrived.notified().await;
    }

    /// Releases a parked exchange past the seam.
    pub(crate) fn release(&self) {
        self.released.notify_one();
    }
}

/// Validated DoH exchange inputs held until the transport primitive runs.
struct PreparedDoh<'a> {
    endpoint: &'a DohEndpoint,
    tls: &'a TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    request: ExchangeRequest<'a>,
    target: String,
    authority: String,
    deadline: Instant,
    caller_cancellation: TransportCancellation,
    owner_cancellation: TransportCancellation,
    #[cfg(test)]
    pause: Option<std::sync::Arc<DohPause>>,
    /// Held only for its RAII release; never read.
    _in_flight: crate::InFlightGuard<'a>,
}

impl PreparedDoh<'_> {
    /// Checks owner shutdown, caller cancellation, then the shared absolute
    /// deadline, in the contractually fixed order.
    fn check_at(&self, now: Instant, side_effect: SideEffectState) -> Result<(), UpstreamError> {
        if self.owner_cancellation.is_cancelled() {
            return Err(UpstreamError::Closed(side_effect));
        }
        if self.caller_cancellation.is_cancelled() {
            return Err(UpstreamError::Cancelled(side_effect));
        }
        if now >= self.deadline {
            return Err(UpstreamError::DeadlineExceeded(side_effect));
        }
        Ok(())
    }

    /// The owner/caller/deadline control view raced by the shared helper.
    fn control(&self) -> ExchangeControl {
        ExchangeControl::new(
            ExchangeContext::new(self.deadline, self.caller_cancellation.clone()),
            self.owner_cancellation.clone(),
        )
    }

    /// Parks at `phase` when this exchange carries a seam watching it.
    async fn reach(&self, phase: DohPhase) {
        #[cfg(test)]
        {
            if let Some(pause) = &self.pause {
                pause.reach(phase).await;
            }
        }
        #[cfg(not(test))]
        {
            let _ = phase;
        }
    }
}

/// Runs one fresh authenticated DoH exchange for a prepared request.
///
/// The single absolute deadline established by the caller covers every phase:
/// numeric connect, TLS handshake, sending the request, reading the response
/// head and body, and the final commit. No phase starts a fresh relative timer.
async fn exchange_inner(prepared: &PreparedDoh<'_>) -> Result<SecureResponse, SecureError> {
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let control = prepared.control();
    let dial = prepared.endpoint.dial();
    let request_id = prepared.request.request_id();
    let deadline = prepared.deadline;

    // The client configuration is rebuilt from the frozen policy for every
    // exchange, so no mutable per-owner state can flip a verified policy into an
    // insecure one between exchanges.
    let config = Arc::new(prepared.tls.client_config_with_alpn(DOH_ALPN)?);
    let server_name = server_name_for(prepared.endpoint.identity())?;

    // Phase 1: numeric dial. No name resolution happens here.
    prepared.reach(DohPhase::BeforeConnect).await;
    let stream: TcpStream = race_control(&control, SideEffectState::NotSent, deadline, async {
        TcpStream::connect(dial)
            .await
            .map_err(|_| SecureError::from(UpstreamError::Connect))
    })
    .await?;

    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Phase 2: authenticated handshake. A `TlsPolicy::verified` failure is
    // terminal: no insecure retry, no plaintext continuation, no second
    // connection. An absent ALPN still permits HTTP/1.1 on this stream, but an
    // unexpected negotiated protocol is terminal.
    prepared.reach(DohPhase::BeforeHandshake).await;
    let connector = TlsConnector::from(config);
    let tls: TlsStream<TcpStream> =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            connector
                .connect(server_name, stream)
                .await
                .map_err(|error| SecureError::Tls(classify_handshake_io_error(&error)))
        })
        .await?;

    let protocol = negotiated_protocol(&tls)?;

    // The handshake succeeded, so the peer is authenticated. No DNS byte has
    // been sent yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Build the one GET this exchange is allowed to send. There is no body, no
    // `User-Agent`, and no request `Content-Encoding`.
    let request = build_get_request(&prepared.target, &prepared.authority)?;

    match protocol {
        DohProtocol::Http1 => {
            exchange_http1(prepared, tls, request, request_id, control, deadline).await
        }
        DohProtocol::Http2 => {
            exchange_http2(prepared, tls, request, request_id, control, deadline).await
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DohProtocol {
    Http1,
    Http2,
}

/// Runs the already-authenticated HTTP/1.1 leg. The connection future is
/// polled inline, preserving Slice2's no-background-driver ownership contract.
async fn exchange_http1(
    prepared: &PreparedDoh<'_>,
    tls: TlsStream<TcpStream>,
    request: hyper::Request<EmptyBody>,
    request_id: u16,
    control: ExchangeControl,
    deadline: Instant,
) -> Result<SecureResponse, SecureError> {
    prepared.reach(DohPhase::BeforeRequest).await;
    let io = TokioIo::new(tls);
    let (mut sender, connection) =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            hyper::client::conn::http1::Builder::new()
                .max_headers(MAX_RESPONSE_HEADERS)
                .max_buf_size(MAX_HTTP1_BUFFER)
                .handshake::<_, EmptyBody>(io)
                .await
                .map_err(|_| SecureError::from(UpstreamError::Connect))
        })
        .await?;
    tokio::pin!(connection);
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;
    let request_future = sender.send_request(request);
    tokio::pin!(request_future);
    prepared.reach(DohPhase::AfterRequestSent).await;
    let response = send_request(&control, deadline, &mut request_future, &mut connection).await?;
    let response = validate_doh_response(
        prepared,
        request_id,
        SecureHttpVersion::Http1,
        control,
        deadline,
        response,
        &mut connection,
    )
    .await?;
    commit_doh_response(prepared, response, deadline).await
}

/// Runs the already-authenticated HTTP/2 leg with a tracked executor. Hyper
/// submits the socket driver and request/body futures to this executor; the
/// scope seals new submissions and aborts/drains every registered child on all
/// normal, error, cancellation, and dropped-future paths.
async fn exchange_http2(
    prepared: &PreparedDoh<'_>,
    tls: TlsStream<TcpStream>,
    request: hyper::Request<EmptyBody>,
    request_id: u16,
    control: ExchangeControl,
    deadline: Instant,
) -> Result<SecureResponse, SecureError> {
    let liveness = Arc::new(prepared.lifecycle.register_shared()?);
    let scope = H2ScopeLease::new(
        liveness,
        prepared.owner_cancellation.clone(),
        prepared.caller_cancellation.clone(),
    );
    let result = exchange_http2_scoped(
        prepared, tls, request, request_id, control, deadline, &scope,
    )
    .await;
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            scope.finish().await;
            return Err(error);
        }
    };
    finalize_h2_response(prepared, response, &scope, deadline).await
}

async fn exchange_http2_scoped(
    prepared: &PreparedDoh<'_>,
    tls: TlsStream<TcpStream>,
    request: hyper::Request<EmptyBody>,
    request_id: u16,
    control: ExchangeControl,
    deadline: Instant,
    scope: &H2ScopeLease,
) -> Result<ValidatedDohResponse, SecureError> {
    prepared.reach(DohPhase::BeforeRequest).await;
    let io = TokioIo::new(tls);
    let executor = scope.executor();
    let (mut sender, connection) =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            let mut builder = hyper::client::conn::http2::Builder::new(executor);
            builder.max_header_list_size(MAX_RESPONSE_HEADER_BYTES as u32);
            builder
                .handshake::<_, EmptyBody>(io)
                .await
                .map_err(|_| SecureError::from(UpstreamError::Connect))
        })
        .await?;
    tokio::pin!(connection);
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;
    let request_future = sender.send_request(request);
    tokio::pin!(request_future);
    prepared.reach(DohPhase::AfterRequestSent).await;
    let response = send_request(&control, deadline, &mut request_future, &mut connection).await?;
    validate_doh_response(
        prepared,
        request_id,
        SecureHttpVersion::Http2,
        control,
        deadline,
        response,
        &mut connection,
    )
    .await
}

/// Validates an HTTP response and restores only the caller's DNS ID.
///
/// The returned candidate is not committed yet. HTTP/2 must first seal and
/// drain its tracked executor children before the separate final-commit step.
async fn validate_doh_response<C>(
    prepared: &PreparedDoh<'_>,
    request_id: u16,
    http_version: SecureHttpVersion,
    control: ExchangeControl,
    deadline: Instant,
    response: Response<Incoming>,
    connection: &mut Pin<&mut C>,
) -> Result<ValidatedDohResponse, SecureError>
where
    C: Future<Output = hyper::Result<()>>,
{
    prepared.reach(DohPhase::BeforeBody).await;
    let body = read_body(&control, deadline, response, connection).await?;
    if body.len() < 12 {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    let restored = restore_request_id(&body, request_id)?;
    if validate_response(&restored).is_err() {
        return Err(SecureError::from(UpstreamError::MalformedResponse));
    }
    Ok(ValidatedDohResponse {
        wire: restored,
        request_id,
        http_version,
        truncated: header.truncated,
    })
}

/// Performs the final owner/caller/deadline commit immediately before success.
///
/// HTTP/1.1 calls this after its inline driver has finished. HTTP/2 calls it
/// only after `H2ScopeLease::finish()` has sealed and drained every executor
/// child, so cancellation or owner close during teardown cannot become a late
/// success.
async fn commit_doh_response(
    prepared: &PreparedDoh<'_>,
    response: ValidatedDohResponse,
    deadline: Instant,
) -> Result<SecureResponse, SecureError> {
    prepared.reach(DohPhase::BeforeCommit).await;
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;
    prepared.reach(DohPhase::AfterCommit).await;
    Ok(SecureResponse::doh(
        response.wire,
        response.request_id,
        response.http_version,
        response.truncated,
    ))
}

/// Drains the HTTP/2 executor before entering the final commit linearization.
async fn finalize_h2_response(
    prepared: &PreparedDoh<'_>,
    response: ValidatedDohResponse,
    scope: &H2ScopeLease,
    deadline: Instant,
) -> Result<SecureResponse, SecureError> {
    scope.finish().await;
    commit_doh_response(prepared, response, deadline).await
}

struct ValidatedDohResponse {
    wire: Vec<u8>,
    request_id: u16,
    http_version: SecureHttpVersion,
    truncated: bool,
}

/// Returns a copy of `body` with only the DNS transaction ID replaced.
///
/// The packet must already be a well-formed response header: at least 12 bytes
/// with QR set. Every other byte — including the RA, TC, RD and RCODE fields —
/// is preserved exactly as the upstream sent it, so the returned wire differs
/// from the upstream's only in bytes 0 and 1.
///
/// This is intentionally narrower than `dns_core::patch_response_id_ra`, which
/// also sets RA for the server response path.
///
/// # Errors
///
/// Returns [`UpstreamError::MalformedResponse`] when the packet is shorter than
/// a DNS header or has QR clear, so a non-response can never be returned to the
/// caller as if it were one.
fn restore_request_id(body: &[u8], request_id: u16) -> Result<Vec<u8>, SecureError> {
    // Validation only: this rejects a short or QR-clear packet before copying.
    inspect_response_header(body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    let mut restored = body.to_vec();
    restored[0..2].copy_from_slice(&request_id.to_be_bytes());
    Ok(restored)
}

/// Selects the one protocol this fresh connection can drive.
///
/// Absent ALPN is accepted as HTTP/1.1 on the already-established stream. Any
/// explicitly negotiated protocol outside the offered h2/http1.1 pair is
/// terminal rather than silently reinterpreted or retried.
fn negotiated_protocol(tls: &TlsStream<TcpStream>) -> Result<DohProtocol, SecureError> {
    let (_, session) = tls.get_ref();
    classify_alpn(session.alpn_protocol())
}

fn classify_alpn(protocol: Option<&[u8]>) -> Result<DohProtocol, SecureError> {
    match protocol {
        None | Some(b"http/1.1") => Ok(DohProtocol::Http1),
        Some(b"h2") => Ok(DohProtocol::Http2),
        Some(_) => Err(SecureError::DohProtocol(DohProtocolError::UnexpectedAlpn)),
    }
}

/// Builds the single `GET` request this exchange may send.
///
/// The target is the origin-form request target produced by the endpoint, so the
/// authority and any numeric dial override cannot leak into it. `Host` is the
/// service authority from the URL.
fn build_get_request(target: &str, authority: &str) -> Result<Request<EmptyBody>, SecureError> {
    let uri = target.parse::<hyper::Uri>().map_err(|_| {
        SecureError::DohRequest(crate::secure::error::DohRequestError::TargetTooLarge)
    })?;
    let mut builder = Request::builder()
        .method(hyper::Method::GET)
        .uri(uri)
        // The DNS media type is the only acceptable response type.
        .header(ACCEPT, DNS_MEDIA_TYPE);
    // `Host` carries the service authority, never the numeric dial address.
    builder = builder.header(HOST, authority);
    builder
        .body(EmptyBody)
        .map_err(|_| SecureError::DohRequest(crate::secure::error::DohRequestError::TargetTooLarge))
}

/// Awaits the response head for an already-dispatched request.
///
/// The caller has already handed `request_future` to the connection driver, so
/// this function is strictly post-handoff: every failure it reports, including a
/// control failure, is `MaybeSent`, because the request may already have reached
/// the peer.
///
/// The connection future is polled alongside the request, so the exchange owns
/// the whole HTTP/1.1 machine. The status and headers are validated here; the
/// body is read separately so its incremental size can be bounded.
async fn send_request(
    control: &ExchangeControl,
    deadline: Instant,
    request_future: &mut Pin<&mut impl Future<Output = hyper::Result<Response<Incoming>>>>,
    connection: &mut Pin<&mut impl Future<Output = hyper::Result<()>>>,
) -> Result<Response<Incoming>, SecureError> {
    let response = race_control(control, SideEffectState::MaybeSent, deadline, async {
        // Poll the connection and the request together: the request future only
        // completes while the connection is being driven. Both outcomes are
        // terminal for this select, so it is not a loop.
        tokio::select! {
            result = request_future.as_mut() => result.map_err(|_| {
                // The request was already handed to the driver, so a failure
                // here may have transmitted part of it.
                SecureError::Transport(UpstreamError::Send(SideEffectState::MaybeSent))
            }),
            result = connection.as_mut() => Err(match result {
                // The connection ended before any response head was observed.
                // Nothing proves the request reached the peer, so this is
                // conservatively MaybeSent rather than a Sent body defect.
                Ok(()) => SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
                Err(_) => SecureError::Transport(UpstreamError::Receive(
                    SideEffectState::MaybeSent,
                )),
            }),
        }
    })
    .await?;

    validate_response_head(&response)?;
    Ok(response)
}

/// Measures the byte size of the parsed response head.
///
/// Hyper decodes the head into a status and a header map, so the original bytes
/// are no longer available; this reconstructs the equivalent wire size from the
/// parsed form. It is therefore a faithful measure of the head's logical size
/// (status line plus every header line plus the terminating blank line), which
/// is what the 16 KiB contract bounds.
fn response_head_bytes(response: &Response<Incoming>) -> usize {
    // "HTTP/1.1 200 OK\r\n" — the version is fixed and the reason phrase is not
    // preserved by the parser, so the status line is measured conservatively
    // from the parts that are.
    let version = match response.version() {
        hyper::Version::HTTP_09 => "HTTP/0.9 ",
        hyper::Version::HTTP_10 => "HTTP/1.0 ",
        _ => "HTTP/1.1 ",
    };
    let mut total = version.len()
        + 3 // status code
        + 1 // SP
        + response.status().canonical_reason().map_or(0, str::len)
        + 2; // CRLF
    for (name, value) in response.headers() {
        // "<name>: <value>\r\n"
        total = total.saturating_add(name.as_str().len() + 2 + value.as_bytes().len() + 2);
    }
    total.saturating_add(2) // the blank line ending the head
}

/// Validates the status, size and headers this contract depends on.
fn validate_response_head(response: &Response<Incoming>) -> Result<(), SecureError> {
    if response.status() != hyper::StatusCode::OK {
        return Err(SecureError::DohProtocol(
            DohProtocolError::UnexpectedStatus {
                status: response.status().as_u16(),
            },
        ));
    }

    // Defense in depth. The raw wire bound is enforced during parsing by
    // `MAX_HTTP1_BUFFER`; this measures the *parsed* head and catches anything
    // the parser accepted, including a case where the reconstruction and the
    // wire disagree.
    //
    // Note that `response_head_bytes` reconstructs the head from parsed fields
    // and therefore cannot see bytes the parser discarded, such as a long
    // non-canonical reason phrase. That is exactly why the raw bound above is
    // the primary control and this is only a secondary check.
    let head_bytes = response_head_bytes(response);
    if head_bytes > MAX_RESPONSE_HEADER_BYTES {
        return Err(SecureError::DohProtocol(
            DohProtocolError::ResponseHeadTooLarge,
        ));
    }

    // A declared body larger than the DNS maximum is rejected here, at the
    // head, before any body byte is read. The incremental bound in `read_body`
    // still applies, so a peer cannot evade the limit by omitting or
    // understating `Content-Length`.
    if let Some(length) = response.headers().get(CONTENT_LENGTH) {
        let length = length
            .to_str()
            .map_err(|_| SecureError::DohProtocol(DohProtocolError::BodyTooLarge))?;
        let declared = length
            .trim()
            .parse::<u64>()
            .map_err(|_| SecureError::DohProtocol(DohProtocolError::BodyTooLarge))?;
        if declared > MAX_DNS_BODY as u64 {
            return Err(SecureError::DohProtocol(DohProtocolError::BodyTooLarge));
        }
    }

    // Content-Encoding must be absent or `identity`; the body is never
    // decompressed, so a compressed payload cannot be read as DNS wire.
    if let Some(encoding) = response.headers().get(CONTENT_ENCODING) {
        let value = encoding.to_str().unwrap_or_default();
        if !value.eq_ignore_ascii_case("identity") {
            return Err(SecureError::DohProtocol(DohProtocolError::ContentEncoding));
        }
    }

    let media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .ok_or(SecureError::DohProtocol(DohProtocolError::MissingMediaType))?;
    let media_type = media_type
        .to_str()
        .map_err(|_| SecureError::DohProtocol(DohProtocolError::WrongMediaType))?;
    // The type is matched case-insensitively and parameters are allowed, so
    // `Application/DNS-Message; charset=binary` is accepted.
    let essence = media_type.split(';').next().unwrap_or_default().trim();
    if !essence.eq_ignore_ascii_case(DNS_MEDIA_TYPE) {
        return Err(SecureError::DohProtocol(DohProtocolError::WrongMediaType));
    }
    Ok(())
}

/// Reads the response body to a complete end, bounded by the DNS maximum.
///
/// The body is accumulated incrementally and the limit is enforced on the bytes
/// actually received, not only on a declared `Content-Length`: a peer cannot
/// evade the bound by omitting the header or by declaring a small length. An
/// early EOF, a `Content-Length`/chunk mismatch, or a malformed chunked encoding
/// is `IncompleteBody`, never a silently accepted prefix.
async fn read_body<C>(
    control: &ExchangeControl,
    deadline: Instant,
    response: Response<Incoming>,
    connection: &mut Pin<&mut C>,
) -> Result<Vec<u8>, SecureError>
where
    C: Future<Output = hyper::Result<()>> + ?Sized,
{
    use http_body_util::BodyExt as _;

    let mut body = response.into_body();
    let collected = race_control(control, SideEffectState::Sent, deadline, async {
        let mut collected: Vec<u8> = Vec::new();
        // The connection is only driven until it finishes; after that the body
        // is read from the frames Hyper has already queued.
        let mut connection_finished = false;
        loop {
            // The body and the connection are polled together: Hyper only
            // produces the next body frame while its connection is being
            // driven, and this exchange owns that connection future inline.
            let frame_future = body.frame();
            tokio::pin!(frame_future);
            let frame = loop {
                if connection_finished {
                    break frame_future.await;
                }
                tokio::select! {
                    biased;
                    frame = &mut frame_future => break frame,
                    result = connection.as_mut() => match result {
                        // A transport error before end-of-body means the body
                        // cannot be trusted to be complete.
                        Err(_) => {
                            return Err(SecureError::DohProtocol(
                                DohProtocolError::IncompleteBody,
                            ));
                        }
                        // The connection ended cleanly. Any body frames it
                        // produced are already queued, so stop driving it and
                        // read them to the end.
                        Ok(()) => connection_finished = true,
                    },
                }
            };
            let Some(frame) = frame else {
                break;
            };
            let frame =
                frame.map_err(|_| SecureError::DohProtocol(DohProtocolError::IncompleteBody))?;
            let data = frame
                .into_data()
                .map_err(|_| SecureError::DohProtocol(DohProtocolError::IncompleteBody))?;
            if collected.len().saturating_add(data.len()) > MAX_DNS_BODY {
                return Err(SecureError::DohProtocol(DohProtocolError::BodyTooLarge));
            }
            collected.extend_from_slice(&data);
        }
        Ok::<Vec<u8>, SecureError>(collected)
    })
    .await?;

    if collected.is_empty() {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    Ok(collected)
}

/// An empty request body.
///
/// A DoH `GET` carries the query in the URL, so the body is always empty. Using
/// a closed type rather than a generic buffer makes "no request body" a property
/// of the type.
#[derive(Debug)]
pub(crate) struct EmptyBody;

impl hyper::body::Body for EmptyBody {
    type Data = hyper::body::Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        // Always immediately at end-of-stream: there is never a body.
        std::task::Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        hyper::body::SizeHint::with_exact(0)
    }
}

/// Classifies a tokio-rustls handshake failure without string parsing.
fn classify_handshake_io_error(
    error: &std::io::Error,
) -> crate::secure::error::TlsHandshakeFailure {
    if let Some(rustls_error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<rustls::Error>())
    {
        return classify_handshake_error(rustls_error);
    }
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => {
            crate::secure::error::TlsHandshakeFailure::UnexpectedEof
        }
        _ => crate::secure::error::TlsHandshakeFailure::Protocol,
    }
}

/// A retained, authenticated DoH session available for one reuse.
///
/// The pooled-session type lives here, beside the protocol code that owns the
/// handshake, request encoding, response validation, and HTTP/2 child tracking,
/// so reuse composes that machinery instead of duplicating a second DoH state
/// machine.
///
/// ## Why the driver is retained
///
/// Hyper's HTTP/1.1 and HTTP/2 clients are *driver* connections: a response head
/// and body are only produced while the connection future is being polled. The
/// fresh path drives that future inline for the duration of one exchange.
/// Reusing the connection therefore requires the driver to stay alive between
/// exchanges, so this session owns it and polls it from inside the caller's own
/// await — never from a detached task.
///
/// ## HTTP/2 children
///
/// An HTTP/2 session also owns the [`H2ScopeLease`] whose executor the driver
/// dispatches its child futures to. Unlike the fresh path — where the scope is
/// per exchange, holds an owner registration, and is sealed and drained when
/// that exchange ends — a pooled scope must survive across exchanges: the
/// connection driver itself is one of its children and stays alive exactly as
/// long as the session is usable. So a pooled exchange performs **no** settle
/// step, because there is nothing to settle down to — waiting for the child
/// count to reach zero would wait for the connection to die.
///
/// Pooled reuse is instead made safe by two other properties: the owner admits
/// at most one exchange at a time on this session
/// ([`MAX_PENDING_PER_CONNECTION`](crate::MAX_PENDING_PER_CONNECTION)), and the
/// session is handed back for retention only after a fully completed exchange.
/// Every terminal failure returns it as a discard. Sealing and aborting happen
/// on the paths that actually end the session: an explicit discard, owner
/// close, or drop.
///
/// The negotiated protocol is recorded by the variant itself and drives the reuse
/// key, so an HTTP/2 session can never be handed to an HTTP/1.1 request or the
/// reverse.
pub(crate) enum PooledDohSession {
    /// An HTTP/1.1 session with its retained driver.
    Http1 {
        sender: hyper::client::conn::http1::SendRequest<EmptyBody>,
        driver: Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>,
    },
    /// An HTTP/2 session with its retained driver and child-tracking scope.
    Http2 {
        sender: hyper::client::conn::http2::SendRequest<EmptyBody>,
        driver: Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>,
        /// The HTTP/2 child-tracking scope whose executor the driver dispatches
        /// to. It is deliberately never read: it is held for its whole pooled
        /// lifetime, and explicit async shutdown or `Drop` seals the scope and
        /// aborts every tracked child. The field exists so the session owns the
        /// scope across exchanges.
        _scope: H2ScopeLease,
    },
}

/// The terminal outcome of one pooled DoH attempt.
///
/// The session is returned so the owner can decide whether to retain it;
/// `rebuildable` is true only when the request was provably never transmitted.
pub(crate) struct PooledDohOutcome {
    pub(crate) error: SecureError,
    pub(crate) session: Option<PooledDohSession>,
    pub(crate) rebuildable: bool,
}

/// A negotiated pooled session.
///
/// The HTTP/2 scope, when present, lives **inside** [`PooledDohSession::Http2`],
/// so the session owns its own child tracking for its whole pooled lifetime and
/// the owner has nothing extra to thread through.
impl PooledDohSession {
    /// Dials, authenticates, and negotiates one DoH session.
    ///
    /// Ordering is the reviewed DoH contract: numeric connect, then the
    /// authenticated handshake against the service URL identity with ALPN
    /// offered, then the HTTP connection handshake. No DNS byte has been sent, so
    /// a failure in any phase is `NotSent`.
    ///
    /// An HTTP/2 session creates its own pooled [`H2ScopeLease`], which holds no
    /// owner registration (see the type docs) and therefore never blocks
    /// `close()` from draining.
    ///
    /// # Errors
    ///
    /// Returns the exact typed [`SecureError`] the fresh DoH path returns for a
    /// connect, handshake, ALPN, or control failure.
    ///
    /// Only the **owner** token is taken, never a caller token: the session this
    /// builds is retained across exchanges, so binding it to one caller's
    /// cancellation would outlive that caller. The exchange in progress still
    /// honors its own caller cancellation through `control`.
    pub(crate) async fn connect(
        endpoint: &DohEndpoint,
        tls: &TlsPolicy,
        owner_cancellation: TransportCancellation,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<Self, SecureError> {
        let config = Arc::new(tls.client_config_with_alpn(DOH_ALPN)?);
        let server_name = server_name_for(endpoint.identity())?;
        let dial = endpoint.dial();

        // Phase 1: numeric dial, so the service identity cannot select the
        // destination.
        let stream: TcpStream = race_control(control, SideEffectState::NotSent, deadline, async {
            TcpStream::connect(dial)
                .await
                .map_err(|_| SecureError::from(UpstreamError::Connect))
        })
        .await?;

        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        // Phase 2: authenticated handshake. A verified policy's failure is
        // terminal: no insecure retry and no plaintext continuation.
        let connector = TlsConnector::from(config);
        let tls_stream: TlsStream<TcpStream> =
            race_control(control, SideEffectState::NotSent, deadline, async {
                connector
                    .connect(server_name, stream)
                    .await
                    .map_err(|error| SecureError::Tls(classify_handshake_io_error(&error)))
            })
            .await?;

        let protocol = negotiated_protocol(&tls_stream)?;

        // The peer is authenticated and still no DNS byte has been sent.
        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        // Phase 3: the HTTP connection handshake on the caller's runtime.
        let io = TokioIo::new(tls_stream);
        match protocol {
            DohProtocol::Http1 => {
                let (sender, connection) =
                    race_control(control, SideEffectState::NotSent, deadline, async {
                        hyper::client::conn::http1::Builder::new()
                            .max_headers(MAX_RESPONSE_HEADERS)
                            .max_buf_size(MAX_HTTP1_BUFFER)
                            .handshake::<_, EmptyBody>(io)
                            .await
                            .map_err(|_| SecureError::from(UpstreamError::Connect))
                    })
                    .await?;
                Ok(Self::Http1 {
                    sender,
                    driver: Box::pin(connection),
                })
            }
            DohProtocol::Http2 => {
                // The pooled scope takes only the owner token: this session
                // outlives the request that opened it, so wiring in *this*
                // request's caller token would let a later cancellation of that
                // token kill the retained driver.
                let scope = H2ScopeLease::pooled(owner_cancellation);
                let executor = scope.executor();
                let (sender, connection) =
                    race_control(control, SideEffectState::NotSent, deadline, async {
                        let mut builder = hyper::client::conn::http2::Builder::new(executor);
                        builder.max_header_list_size(MAX_RESPONSE_HEADER_BYTES as u32);
                        builder
                            .handshake::<_, EmptyBody>(io)
                            .await
                            .map_err(|_| SecureError::from(UpstreamError::Connect))
                    })
                    .await?;
                Ok(Self::Http2 {
                    sender,
                    driver: Box::pin(connection),
                    _scope: scope,
                })
            }
        }
    }

    /// Tears this session down, awaiting every tracked HTTP/2 child.
    ///
    /// [`Drop`] can only seal and abort — it cannot await — so a session dropped
    /// on a discard path would leave its children for the runtime to reap
    /// whenever it next gets to them. That violates the requirement that no
    /// tracked child outlives its connection and that close returns `Closed`
    /// only once draining has finished. This is the explicit async teardown: it
    /// seals admission, aborts the children, and waits for the tracked count to
    /// reach zero.
    ///
    /// HTTP/1.1 has no tracked children, so it has nothing to wait for.
    ///
    /// It cannot deadlock against the serial pool: the only children a pooled
    /// scope ever holds are Hyper's own per-request send/pipe futures and the
    /// retained driver's, none of which wait on this owner.
    pub(crate) async fn shutdown(self) {
        match self {
            Self::Http1 { .. } => {}
            Self::Http2 { _scope, .. } => _scope.finish().await,
        }
    }

    /// Installs a deterministic teardown barrier on a pooled HTTP/2 session's
    /// scope, so a test can prove that owner close waits for child drain.
    ///
    /// Returns `None` for an HTTP/1.1 session, which has no tracked children.
    #[cfg(test)]
    pub(crate) fn install_teardown_pause_for_test(
        &self,
        pause: Arc<H2TeardownPause>,
    ) -> Option<()> {
        match self {
            Self::Http1 { .. } => None,
            Self::Http2 { _scope, .. } => {
                _scope.install_teardown_pause(pause);
                Some(())
            }
        }
    }

    /// The number of live tracked children on a pooled HTTP/2 scope.
    ///
    /// Used by the crate's pooled-close regression to assert the scope really is
    /// drained once owner close has completed. Test-only.
    #[cfg(test)]
    pub(crate) fn active_children_for_test(&self) -> Option<usize> {
        match self {
            Self::Http1 { .. } => None,
            Self::Http2 { _scope, .. } => Some(_scope.active_children()),
        }
    }
    /// The drain handle for this session's HTTP/2 scope, if it has one.
    ///
    /// The owner parks this for the whole lifetime of a pooled HTTP/2 session so
    /// it can finish teardown even when the exchange future was aborted and only
    /// this session's synchronous `Drop` ran.
    #[must_use]
    pub(crate) fn h2_drain_handle(&self) -> Option<H2DrainHandle> {
        match self {
            Self::Http1 { .. } => None,
            Self::Http2 { _scope, .. } => Some(_scope.drain_handle()),
        }
    }

    /// Whether the peer has already closed this session.
    /// `SendRequest::is_closed` alone is **not** sufficient for a *retained*
    /// session: Hyper only learns that the connection ended by polling its
    /// driver, and an idle pooled driver is not being polled between exchanges.
    /// A FIN or RST arriving while the session sits in the pool would therefore
    /// go unnoticed, and the next exchange would hand a request to a dead
    /// connection.
    ///
    /// This probe closes that gap without sending a single DNS byte: it polls
    /// the retained driver once with a no-op waker. A driver that is already
    /// finished — cleanly or with an error — is a dead connection and reports
    /// `true`; a driver still pending is alive and reports `false`, and having
    /// been polled with a no-op waker it simply re-registers when the pooled
    /// exchange polls it properly.
    ///
    /// Because no request has been handed to the driver yet, the caller may treat
    /// an ended session as `NotSent` and perform its single fresh replacement.
    #[must_use]
    pub(crate) fn is_closed(&mut self) -> bool {
        match self {
            Self::Http1 { sender, driver } => sender.is_closed() || driver_ended(driver),
            Self::Http2 { sender, driver, .. } => sender.is_closed() || driver_ended(driver),
        }
    }

    /// The protocol name recorded for the reuse key.
    #[must_use]
    pub(crate) const fn protocol_name(&self) -> &'static str {
        match self {
            Self::Http1 { .. } => "http/1.1",
            Self::Http2 { .. } => "h2",
        }
    }

    /// Runs one DoH `GET` on this established session.
    ///
    /// The session is always returned so the owner can decide whether to retain
    /// it. All request encoding, response-head validation, body reading, and ID
    /// restoration come from the same helpers the fresh path uses, so this adds
    /// no second DoH protocol state machine — each variant only wires its sender
    /// and its retained driver to those helpers.
    pub(crate) async fn exchange(
        mut self,
        target: &str,
        authority: &str,
        request_id: u16,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<(SecureResponse, Self), PooledDohOutcome> {
        let request = match build_get_request(target, authority) {
            Ok(request) => request,
            Err(error) => {
                // A rejected request target never reached the wire.
                return Err(PooledDohOutcome {
                    error,
                    session: Some(self),
                    rebuildable: true,
                });
            }
        };

        let (http_version, outcome) = match &mut self {
            Self::Http1 { sender, driver } => (
                SecureHttpVersion::Http1,
                run_pooled_exchange(
                    sender,
                    driver,
                    request,
                    request_id,
                    control,
                    deadline,
                    SecureHttpVersion::Http1,
                )
                .await,
            ),
            Self::Http2 { sender, driver, .. } => {
                // The `scope` field is deliberately not bound here: it must stay
                // owned by the session across exchanges, and this exchange does
                // not touch it. It seals and aborts only when the session drops.
                let outcome = run_pooled_exchange(
                    sender,
                    driver,
                    request,
                    request_id,
                    control,
                    deadline,
                    SecureHttpVersion::Http2,
                )
                .await;
                // No settle step here. A pooled HTTP/2 session keeps its
                // connection driver alive for the next exchange, and the driver
                // itself is dispatched to the tracked executor, so the scope
                // always has a live child while the session is usable. Waiting
                // for `active == 0` before reuse would therefore wait forever.
                //
                // Safety instead comes from the two properties that make the
                // reuse sound: the owner admits exactly one exchange at a time
                // for this session (`MAX_PENDING_PER_CONNECTION`), and *every*
                // terminal failure return below hands the session back to the
                // owner as a discard, so a session is only retained after a
                // fully completed exchange. The scope is sealed and its children
                // aborted when the session is dropped or the owner closes.
                (SecureHttpVersion::Http2, outcome)
            }
        };

        match outcome {
            Ok(validated) => {
                let response = SecureResponse::doh(
                    validated.wire,
                    validated.request_id,
                    http_version,
                    validated.truncated,
                );
                Ok((response, self))
            }
            Err(error) => Err(PooledDohOutcome {
                // A failure after the request was handed to the driver may have
                // transmitted part of it, so it is never rebuildable unless the
                // error itself says nothing was sent.
                rebuildable: matches!(error.side_effect(), SideEffectState::NotSent),
                error,
                session: Some(self),
            }),
        }
    }
}

/// Polls a retained connection driver once to learn whether it has already
/// finished.
///
/// The driver is polled with a no-op waker, so this never blocks, never yields
/// to the runtime, and never registers a real interest. A driver that is still
/// running returns [`Poll::Pending`] and is untouched by the poll; a driver that
/// has completed — cleanly or with a transport error — returns `Ready`, which is
/// exactly the "the retained connection is gone" signal the pool needs.
fn driver_ended(driver: &mut Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>) -> bool {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    matches!(
        driver.as_mut().poll(&mut context),
        std::task::Poll::Ready(_)
    )
}

/// A sender from either negotiated HTTP version.
/// Hyper's HTTP/1.1 and HTTP/2 senders expose the same `send_request` shape but
/// share no trait, so this thin adapter lets the pooled path drive either one
/// through the *same* validation helpers instead of duplicating the request
/// flow per protocol.
trait DohSender {
    fn send(
        &mut self,
        request: Request<EmptyBody>,
    ) -> impl Future<Output = hyper::Result<Response<Incoming>>>;
}

impl DohSender for hyper::client::conn::http1::SendRequest<EmptyBody> {
    fn send(
        &mut self,
        request: Request<EmptyBody>,
    ) -> impl Future<Output = hyper::Result<Response<Incoming>>> {
        self.send_request(request)
    }
}

impl DohSender for hyper::client::conn::http2::SendRequest<EmptyBody> {
    fn send(
        &mut self,
        request: Request<EmptyBody>,
    ) -> impl Future<Output = hyper::Result<Response<Incoming>>> {
        self.send_request(request)
    }
}

/// Sends one request on a retained session and reads its complete response.
///
/// The retained driver is polled alongside the request and body, exactly as the
/// fresh path does, so Hyper can produce the frames. Nothing here is a detached
/// task: the driver is driven from inside the caller's own await.
async fn run_pooled_exchange<S>(
    sender: &mut S,
    driver: &mut Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>,
    request: Request<EmptyBody>,
    request_id: u16,
    control: &ExchangeControl,
    deadline: Instant,
    http_version: SecureHttpVersion,
) -> Result<ValidatedDohResponse, SecureError>
where
    S: DohSender,
{
    let request_future = sender.send(request);
    tokio::pin!(request_future);
    let mut driver = driver.as_mut();

    let response = race_control(control, SideEffectState::MaybeSent, deadline, async {
        tokio::select! {
            result = request_future.as_mut() => result.map_err(|_| {
                SecureError::Transport(UpstreamError::Send(SideEffectState::MaybeSent))
            }),
            result = driver.as_mut() => Err(match result {
                Ok(()) => SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
                Err(_) => SecureError::Transport(UpstreamError::Receive(
                    SideEffectState::MaybeSent,
                )),
            }),
        }
    })
    .await?;

    validate_response_head(&response)?;

    let body = read_body(control, deadline, response, &mut driver).await?;
    if body.len() < 12 {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    let restored = restore_request_id(&body, request_id)?;
    if validate_response(&restored).is_err() {
        return Err(SecureError::from(UpstreamError::MalformedResponse));
    }
    Ok(ValidatedDohResponse {
        wire: restored,
        request_id,
        http_version,
        truncated: header.truncated,
    })
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        KeyUsagePurpose, SanType, date_time_ymd,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};
    use tokio::time::timeout;
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::server::TlsStream;

    use super::{
        DohPhase, DohProtocol, DohUpstream, H2ScopeLease, H2TeardownPause, ValidatedDohResponse,
        classify_alpn, finalize_h2_response,
    };
    use crate::secure::endpoint::DohEndpoint;
    use crate::secure::error::{DohProtocolError, SecureError};
    use crate::secure::tls::TlsPolicy;
    use crate::{
        CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, Lifecycle, SideEffectState,
        TransportCancellation, UpstreamError,
    };

    /// Bounds every control wait so a broken path fails instead of hanging.
    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    /// Runs one bounded current-thread runtime for a single test.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread test runtime")
            .block_on(future)
    }

    /// A valid query with a caller-chosen ID.
    fn query_wire(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x01, 0x00]);
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        wire
    }

    /// A complete, dns-core-valid response with a one-byte answer marker.
    fn response_wire(id: u16, marker: u8) -> Vec<u8> {
        let mut wire = query_wire(id);
        wire[2] = 0x81;
        wire[3] = 0x80;
        wire[6..8].copy_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
        wire
    }

    /// A generated trust anchor plus a server leaf that chains to it.
    struct Identity {
        ca_der: rustls::pki_types::CertificateDer<'static>,
        leaf_der: rustls::pki_types::CertificateDer<'static>,
        leaf_key: KeyPair,
    }

    /// Generates a fresh CA and a `dns.example` leaf signed by it.
    fn generate_identity() -> Identity {
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("generate a synthetic CA key");
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "mosdns-doh-test-root");
        ca_params.not_before = date_time_ymd(2024, 1, 1);
        ca_params.not_after = date_time_ymd(2036, 1, 1);
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("self-sign the synthetic CA");

        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("generate a synthetic leaf key");
        let mut leaf_params = CertificateParams::default();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "dns.example");
        leaf_params.subject_alt_names = vec![SanType::DnsName(
            "dns.example".try_into().expect("valid synthetic DNS name"),
        )];
        leaf_params.not_before = date_time_ymd(2024, 1, 1);
        leaf_params.not_after = date_time_ymd(2036, 1, 1);
        leaf_params.is_ca = IsCa::ExplicitNoCa;
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

        let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
        let leaf_cert = leaf_params
            .signed_by(&leaf_key, &issuer)
            .expect("sign the synthetic leaf");

        Identity {
            ca_der: ca_cert.der().clone(),
            leaf_der: leaf_cert.der().clone(),
            leaf_key,
        }
    }

    /// Builds the client policy trusting only the generated CA.
    fn client_policy(identity: &Identity) -> TlsPolicy {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(identity.ca_der.clone())
            .expect("the generated CA parses as a trust anchor");
        TlsPolicy::verified(roots).expect("verified policy")
    }

    /// Builds a server configuration presenting the leaf and its CA.
    fn server_config(identity: &Identity) -> Arc<rustls::ServerConfig> {
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.leaf_der.clone(), identity.ca_der.clone()],
            rustls::pki_types::PrivateKeyDer::try_from(identity.leaf_key.serialize_der())
                .expect("generated key is valid PKCS#8"),
        )
        .map(Arc::new)
        .expect("generated certificate and key are consistent")
    }

    /// Binds a fresh loopback listener and returns its address.
    fn bind() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        let address = listener.local_addr().expect("local address");
        (listener, address)
    }

    /// Builds a DoH owner for `address` authenticated as `dns.example`.
    fn owner_for(address: SocketAddr, identity: &Identity) -> DohUpstream {
        DohUpstream::new(
            DohEndpoint::new("https://dns.example/dns-query", address).expect("valid DoH endpoint"),
            client_policy(identity),
        )
        .expect("owner")
    }

    /// A scripted HTTPS server that counts the requests it receives.
    ///
    /// The count is the evidence that a terminal failure never sends a second
    /// GET, and it is also the "one request" observation for the success path.
    struct CountingServer {
        address: SocketAddr,
        handle: std::thread::JoinHandle<usize>,
    }

    impl CountingServer {
        /// Starts a server that answers each connection, and reports how many
        /// requests it received in total.
        fn start(identity: &Identity) -> Self {
            Self::start_with(identity, true)
        }

        /// Starts a server that either answers or holds, counting requests.
        fn start_with(identity: &Identity, answer: bool) -> Self {
            Self::start_scripted(identity, move |_tls| {
                if answer {
                    Some((200, response_wire(0, 7)))
                } else {
                    None
                }
            })
        }

        /// Starts a server that answers every request with `status`.
        ///
        /// Used to produce a fast, peer-driven terminal failure: a forbidden
        /// retry could reconnect here, so the observed request count is a
        /// meaningful test of the no-retry contract.
        fn start_status(identity: &Identity, status: u16) -> Self {
            Self::start_scripted(identity, move |_tls| Some((status, Vec::new())))
        }

        /// Starts a server that counts requests and replies per `script`.
        ///
        /// The listener keeps accepting until it goes idle, so a forbidden
        /// second connection is counted rather than ignored.
        fn start_scripted<F>(identity: &Identity, mut script: F) -> Self
        where
            F: FnMut(&TlsStream<TcpStream>) -> Option<(u16, Vec<u8>)> + Send + 'static,
        {
            let (listener, address) = bind();
            listener
                .set_nonblocking(true)
                .expect("listener non-blocking");
            let config = server_config(identity);
            let handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build server runtime");
                runtime.block_on(async move {
                    let listener =
                        AsyncTcpListener::from_std(listener).expect("adopt listener");
                    let mut requests = 0usize;
                    loop {
                        let Ok(accepted) = timeout(Duration::from_millis(400), listener.accept()).await
                        else {
                            break;
                        };
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        let acceptor = TlsAcceptor::from(Arc::clone(&config));
                        let Ok(Ok(mut tls)) = timeout(TEST_TIMEOUT, acceptor.accept(stream)).await
                        else {
                            continue;
                        };
                        if read_one_request(&mut tls).await {
                            requests += 1;
                        }
                        if let Some((status, body)) = script(&tls) {
                            let reason = if status == 200 { "OK" } else { "Error" };
                            let head = format!(
                                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = tls.write_all(head.as_bytes()).await;
                            let _ = tls.write_all(&body).await;
                            let _ = tls.flush().await;
                        }
                        let mut scratch = [0u8; 64];
                        while let Ok(read) = tls.read(&mut scratch).await {
                            if read == 0 {
                                break;
                            }
                        }
                    }
                    requests
                })
            });
            Self { address, handle }
        }

        fn join(self) -> usize {
            self.handle.join().expect("server thread joined")
        }
    }

    /// Reads one HTTP request head; returns `true` when one arrived.
    async fn read_one_request(tls: &mut TlsStream<TcpStream>) -> bool {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                return true;
            }
            if buffer.len() > 64 * 1024 {
                return false;
            }
            match tls.read(&mut chunk).await {
                Ok(0) | Err(_) => return false,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            }
        }
    }

    /// The four controls the acceptance matrix requires at every pre-result
    /// phase: owner close, caller cancellation, the shared absolute deadline,
    /// and a dropped/aborted caller future.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PhaseControl {
        OwnerClose,
        CallerCancellation,
        AbsoluteDeadline,
        AbortDrop,
    }

    /// Every pre-result phase the control matrix must cover.
    const MATRIX_PHASES: [DohPhase; 6] = [
        DohPhase::BeforeConnect,
        DohPhase::BeforeHandshake,
        DohPhase::BeforeRequest,
        DohPhase::AfterRequestSent,
        DohPhase::BeforeBody,
        DohPhase::BeforeCommit,
    ];

    /// The side-effect state the exchange has provably reached at `phase`.
    ///
    /// This mirrors the reviewed send-state table exactly:
    ///
    /// * before connect and during the handshake no HTTP request exists, and
    ///   the request is still `NotSent` while it is built but not yet handed to
    ///   the driver (the "before the send future is polled" boundary);
    /// * once the response headers have been observed the request was certainly
    ///   transmitted, so the state is `Sent` for the body and commit phases.
    const fn side_effect_at(phase: DohPhase) -> SideEffectState {
        match phase {
            DohPhase::BeforeConnect | DohPhase::BeforeHandshake | DohPhase::BeforeRequest => {
                SideEffectState::NotSent
            }
            // The request is with the driver but no response head exists yet,
            // so delivery is unknowable: this is the conservative state.
            DohPhase::AfterRequestSent => SideEffectState::MaybeSent,
            DohPhase::BeforeBody | DohPhase::BeforeCommit | DohPhase::AfterCommit => {
                SideEffectState::Sent
            }
        }
    }

    /// Installs a phase seam on the owner and returns it to the test.
    fn install(upstream: &DohUpstream, phase: DohPhase) -> Arc<super::DohPause> {
        let pause = Arc::new(super::DohPause::new(phase));
        upstream.install_pause(Arc::clone(&pause));
        pause
    }

    /// Spawns one exchange so the test can apply a control while it is parked.
    fn spawn(
        upstream: &Arc<DohUpstream>,
        query: Vec<u8>,
        context: ExchangeContext,
    ) -> tokio::task::JoinHandle<Result<super::SecureResponse, SecureError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, context).await
        })
    }

    /// Runs one `(phase, control)` matrix cell and asserts its contract.
    fn run_phase_control_case(phase: DohPhase, control: PhaseControl) {
        block_on(async {
            let identity = generate_identity();

            // `BeforeConnect` is decided before any socket exists, so a
            // bound-but-unused listener is enough. Every later phase requires a
            // completed TLS handshake, so it needs a live server; the server
            // deliberately does not answer, leaving the exchange parked until
            // the test's control terminates it.
            let (address, server) = match phase {
                // Decided before any socket exists: a bound-but-unused listener
                // is enough.
                DohPhase::BeforeConnect => {
                    let (listener, address) = bind();
                    listener
                        .set_nonblocking(true)
                        .expect("listener non-blocking");
                    (address, None)
                }
                // Reached once the handshake is complete, but before any
                // response exists: the server holds without answering.
                DohPhase::BeforeHandshake
                | DohPhase::BeforeRequest
                | DohPhase::AfterRequestSent => {
                    let server = CountingServer::start_with(&identity, false);
                    (server.address, Some(server))
                }
                // Reached only after response headers have been observed, so
                // the server must answer. It declares a body and withholds it,
                // keeping the exchange parked in the body phase.
                DohPhase::BeforeBody | DohPhase::BeforeCommit | DohPhase::AfterCommit => {
                    let server = CountingServer::start_with(&identity, true);
                    (server.address, Some(server))
                }
            };

            let upstream = Arc::new(owner_for(address, &identity));
            let pause = install(&upstream, phase);

            let caller = TransportCancellation::new();
            let deadline = Instant::now() + Duration::from_millis(500);
            let context = ExchangeContext::new(deadline, caller.clone());
            let exchange = spawn(&upstream, query_wire(0x6A00), context);

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .unwrap_or_else(|_| panic!("{phase:?}/{control:?}: reaches its phase"));
            assert_eq!(upstream.in_flight_exchanges(), 1, "{phase:?}/{control:?}");

            let expected = side_effect_at(phase);
            match control {
                PhaseControl::OwnerClose => {
                    assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
                    pause.release();
                    expect_error(
                        exchange,
                        SecureError::Transport(UpstreamError::Closed(expected)),
                    )
                    .await;
                    assert_eq!(upstream.close().await, CloseResult::Closed);
                }
                PhaseControl::CallerCancellation => {
                    caller.cancel();
                    pause.release();
                    expect_error(
                        exchange,
                        SecureError::Transport(UpstreamError::Cancelled(expected)),
                    )
                    .await;
                }
                PhaseControl::AbsoluteDeadline => {
                    // Wait for the exchange's own deadline instant; no arbitrary
                    // sleep stands in for the deadline.
                    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                    pause.release();
                    expect_error(
                        exchange,
                        SecureError::Transport(UpstreamError::DeadlineExceeded(expected)),
                    )
                    .await;
                }
                PhaseControl::AbortDrop => {
                    exchange.abort();
                    let _ = exchange.await;
                    assert_eq!(
                        upstream.close().await,
                        CloseResult::Closed,
                        "{phase:?}/{control:?}: the owner drains after the drop"
                    );
                }
            }

            assert_eq!(
                upstream.in_flight_exchanges(),
                0,
                "{phase:?}/{control:?}: every registration is released"
            );
            if let Some(server) = server {
                let requests = server.join();
                // The invariant that matters is that a terminal control never
                // causes a second connection or a retried request.
                assert!(
                    requests <= 1,
                    "{phase:?}/{control:?}: at most one request is ever sent, saw {requests}"
                );
                // Once the response headers were observed the request is
                // provably on the wire, so exactly one must have arrived.
                if side_effect_at(phase) == SideEffectState::Sent {
                    assert_eq!(
                        requests, 1,
                        "{phase:?}/{control:?}: a Sent state implies one delivered request"
                    );
                }
            }
        });
    }

    /// Asserts the exchange failed with the expected typed control error.
    async fn expect_error(
        handle: tokio::task::JoinHandle<Result<super::SecureResponse, SecureError>>,
        expected: SecureError,
    ) {
        let error = timeout(TEST_TIMEOUT, handle)
            .await
            .expect("exchange bounded")
            .expect("exchange task joined")
            .expect_err("the exchange must be terminated by its control");
        assert_eq!(error, expected);
    }

    /// Runs all four controls against one phase.
    fn run_phase_control_matrix(phase: DohPhase) {
        for control in [
            PhaseControl::OwnerClose,
            PhaseControl::CallerCancellation,
            PhaseControl::AbsoluteDeadline,
            PhaseControl::AbortDrop,
        ] {
            run_phase_control_case(phase, control);
        }
    }

    /// The matrix must cover exactly the pre-result phases.
    #[test]
    fn the_control_matrix_covers_every_pre_result_phase_exactly_once() {
        let mut seen: Vec<DohPhase> = MATRIX_PHASES.to_vec();
        seen.sort_by_key(|phase| format!("{phase:?}"));
        seen.dedup();
        assert_eq!(seen.len(), MATRIX_PHASES.len(), "no duplicate phases");
        assert_eq!(
            seen.len(),
            6,
            "connect, handshake, request, request-sent, body, commit"
        );
        assert_eq!(
            side_effect_at(DohPhase::BeforeConnect),
            SideEffectState::NotSent
        );
        assert_eq!(
            side_effect_at(DohPhase::BeforeHandshake),
            SideEffectState::NotSent
        );
        assert_eq!(
            side_effect_at(DohPhase::BeforeRequest),
            SideEffectState::NotSent
        );
        assert_eq!(
            side_effect_at(DohPhase::AfterRequestSent),
            SideEffectState::MaybeSent
        );
        assert_eq!(side_effect_at(DohPhase::BeforeBody), SideEffectState::Sent);
        assert_eq!(
            side_effect_at(DohPhase::BeforeCommit),
            SideEffectState::Sent
        );
    }

    #[test]
    fn control_matrix_at_before_connect() {
        run_phase_control_matrix(DohPhase::BeforeConnect);
    }

    #[test]
    fn control_matrix_at_before_handshake() {
        run_phase_control_matrix(DohPhase::BeforeHandshake);
    }

    #[test]
    fn control_matrix_at_before_request() {
        run_phase_control_matrix(DohPhase::BeforeRequest);
    }

    #[test]
    fn control_matrix_at_after_request_sent() {
        run_phase_control_matrix(DohPhase::AfterRequestSent);
    }

    #[test]
    fn control_matrix_at_before_body() {
        run_phase_control_matrix(DohPhase::BeforeBody);
    }

    #[test]
    fn control_matrix_at_before_commit() {
        run_phase_control_matrix(DohPhase::BeforeCommit);
    }

    #[test]
    fn a_successful_exchange_sends_exactly_one_get_on_one_connection() {
        block_on(async {
            let identity = generate_identity();
            let server = CountingServer::start(&identity);
            let upstream = owner_for(server.address, &identity);
            let query = query_wire(0x7001);
            let request = ExchangeRequest::new(&query).expect("valid query");

            let response = upstream
                .exchange(request, open_context())
                .await
                .expect("a well-formed DoH response succeeds");
            assert_eq!(response.transport(), super::super::SecureTransport::Doh);
            assert_eq!(response.request_id(), 0x7001);

            // Exactly one request reached the server, on one connection.
            assert_eq!(server.join(), 1, "at most one GET per exchange");
            assert_eq!(upstream.in_flight_exchanges(), 0);
        });
    }

    #[test]
    fn a_terminal_protocol_failure_never_sends_a_second_request() {
        block_on(async {
            let identity = generate_identity();
            // The server answers with a non-200 status, which is a fast,
            // peer-driven terminal failure. Unlike a deadline failure, a
            // forbidden retry *could* still reconnect and send again, so the
            // request count below is a real test of the no-retry contract.
            let server = CountingServer::start_status(&identity, 503);
            let upstream = owner_for(server.address, &identity);
            let query = query_wire(0x7002);
            let request = ExchangeRequest::new(&query).expect("valid query");
            let error = upstream
                .exchange(request, open_context())
                .await
                .expect_err("a 503 is terminal");
            assert!(
                matches!(error, SecureError::DohProtocol(_)),
                "got {error:?}"
            );

            // The count proves no hidden retry or protocol fallback happened:
            // a retry would have produced a second connection and request.
            assert_eq!(
                server.join(),
                1,
                "a terminal protocol failure must never produce a second GET"
            );
        });
    }

    #[test]
    fn h2_executor_seals_admission_and_drains_registered_children() {
        block_on(async {
            let lifecycle = Arc::new(Lifecycle::new());
            let liveness = Arc::new(
                lifecycle
                    .register_shared()
                    .expect("liveness registration succeeds while open"),
            );
            let scope = H2ScopeLease::new(
                liveness,
                TransportCancellation::new(),
                TransportCancellation::new(),
            );
            let executor = scope.executor();
            hyper::rt::Executor::execute(&executor, async {
                std::future::pending::<()>().await;
            });
            assert_eq!(scope.active_children(), 1);

            scope.finish().await;
            assert_eq!(scope.active_children(), 0);

            // Hyper submissions that race with teardown are dropped at the
            // sealed boundary rather than becoming untracked Tokio work.
            hyper::rt::Executor::execute(&executor, async {});
            assert_eq!(scope.active_children(), 0);
            drop(executor);
            drop(scope);
            assert_eq!(lifecycle.in_flight(), 0);
        });
    }

    #[test]
    fn h2_teardown_barrier_preserves_liveness_until_scope_release() {
        block_on(async {
            let lifecycle = Arc::new(Lifecycle::new());
            let liveness = Arc::new(
                lifecycle
                    .register_shared()
                    .expect("liveness registration succeeds while open"),
            );
            let scope = Arc::new(H2ScopeLease::new(
                liveness,
                TransportCancellation::new(),
                TransportCancellation::new(),
            ));
            let pause = Arc::new(H2TeardownPause::new());
            scope.install_teardown_pause(Arc::clone(&pause));
            let executor = scope.executor();
            hyper::rt::Executor::execute(&executor, async {
                std::future::pending::<()>().await;
            });

            let finish_scope = Arc::clone(&scope);
            let finish = tokio::spawn(async move {
                finish_scope.finish().await;
            });
            timeout(TEST_TIMEOUT, pause.wait_for_arrivals(1))
                .await
                .expect("teardown reaches the deterministic barrier");

            let mut drain = Box::pin(lifecycle.drain());
            tokio::select! {
                biased;
                () = &mut drain => panic!("owner drain returned while h2 scope liveness was held"),
                () = tokio::task::yield_now() => {},
            }

            pause.release();
            timeout(TEST_TIMEOUT, finish)
                .await
                .expect("h2 teardown finishes")
                .expect("teardown task joins");
            assert_eq!(scope.active_children(), 0);

            drop(executor);
            drop(scope);
            timeout(TEST_TIMEOUT, drain)
                .await
                .expect("owner drain completes only after scope release");
            assert_eq!(lifecycle.in_flight(), 0);
        });
    }

    #[test]
    fn h2_validated_response_cannot_commit_until_teardown_releases() {
        block_on(async {
            let identity = generate_identity();
            let (listener, address) = bind();
            drop(listener);
            let upstream = owner_for(address, &identity);
            let query = query_wire(0x7011);
            let request = ExchangeRequest::new(&query).expect("valid query");
            let target = upstream
                .endpoint()
                .get_request_target(request)
                .expect("request target");
            let authority = upstream.endpoint().authority();
            let prepared = upstream
                .prepare_exchange(request, open_context(), target, authority)
                .expect("prepared exchange");
            let liveness = Arc::new(
                prepared
                    .lifecycle
                    .register_shared()
                    .expect("h2 liveness registration"),
            );
            let scope = H2ScopeLease::new(
                liveness,
                prepared.owner_cancellation.clone(),
                prepared.caller_cancellation.clone(),
            );
            let pause = Arc::new(H2TeardownPause::new());
            scope.install_teardown_pause(Arc::clone(&pause));
            let executor = scope.executor();
            hyper::rt::Executor::execute(&executor, async {
                std::future::pending::<()>().await;
            });
            let response = ValidatedDohResponse {
                wire: response_wire(0, 7),
                request_id: 0x7011,
                http_version: super::SecureHttpVersion::Http2,
                truncated: false,
            };

            let error = {
                let finalization =
                    finalize_h2_response(&prepared, response, &scope, prepared.deadline);
                tokio::pin!(finalization);
                tokio::select! {
                    biased;
                    result = &mut finalization => panic!("h2 response committed before teardown: {result:?}"),
                    () = pause.wait_for_arrivals(1) => {},
                }
                assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
                pause.release();
                finalization
                    .await
                    .expect_err("owner close during teardown must win over success")
            };
            assert_eq!(
                error,
                SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent))
            );

            drop(executor);
            drop(scope);
            drop(prepared);
            assert_eq!(upstream.close().await, CloseResult::Closed);
        });
    }

    #[test]
    fn alpn_dispatch_accepts_only_the_two_driven_protocols() {
        assert_eq!(classify_alpn(None), Ok(DohProtocol::Http1));
        assert_eq!(classify_alpn(Some(b"http/1.1")), Ok(DohProtocol::Http1));
        assert_eq!(classify_alpn(Some(b"h2")), Ok(DohProtocol::Http2));
        assert_eq!(
            classify_alpn(Some(b"acme/1")),
            Err(SecureError::DohProtocol(DohProtocolError::UnexpectedAlpn))
        );
    }

    /// The context used by the success-path tests.
    fn open_context() -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            TransportCancellation::new(),
        )
    }
}
