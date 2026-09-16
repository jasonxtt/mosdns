#![forbid(unsafe_code)]

//! Pure Rust contracts for future DNS upstream transports.

#![allow(clippy::pedantic)]

mod udp;

use std::fmt;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Instant;

use mosdns_dns_core::parse_query;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// The direct transport kinds covered by the first Phase 4 boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Udp,
    Tcp,
}

/// A validated numeric upstream endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    address: SocketAddr,
    transport: Transport,
}

impl Endpoint {
    /// Creates an endpoint without hostname resolution or socket setup.
    pub fn new(address: SocketAddr, transport: Transport) -> Result<Self, UpstreamError> {
        if address.port() == 0 {
            return Err(UpstreamError::InvalidEndpoint(EndpointError::ZeroPort));
        }
        Ok(Self { address, transport })
    }

    #[must_use]
    pub fn address(self) -> SocketAddr {
        self.address
    }

    #[must_use]
    pub const fn transport(self) -> Transport {
        self.transport
    }
}

/// Endpoint defects that can be identified before any socket operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointError {
    ZeroPort,
}

/// Query defects identified by the pure request boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    Empty,
    Malformed,
}

/// A borrowed, read-only DNS query with its original transaction ID recorded.
pub struct ExchangeRequest<'q> {
    query: &'q [u8],
    request_id: u16,
}

impl<'q> ExchangeRequest<'q> {
    /// Validates the query without retaining or modifying caller-owned bytes.
    pub fn new(query: &'q [u8]) -> Result<Self, UpstreamError> {
        if query.is_empty() {
            return Err(UpstreamError::InvalidRequest(RequestError::Empty));
        }
        let (header, _) = parse_query(query)
            .map_err(|_| UpstreamError::InvalidRequest(RequestError::Malformed))?;
        Ok(Self {
            query,
            request_id: header.id,
        })
    }

    #[must_use]
    pub const fn query(&self) -> &'q [u8] {
        self.query
    }

    #[must_use]
    pub const fn request_id(&self) -> u16 {
        self.request_id
    }
}

/// A transport cancellation token owned by upstream-core rather than
/// sequence-core. Child cancellation is local; parent cancellation propagates
/// to all descendants. The token exposes an async wake primitive for future
/// socket/timer select code; Slice0 never creates or owns a Tokio runtime.
#[derive(Clone)]
pub struct TransportCancellation(CancellationToken);

impl TransportCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self(CancellationToken::new())
    }

    #[must_use]
    pub fn child_token(&self) -> Self {
        Self(self.0.child_token())
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    /// Waits until this token is cancelled. Future transport I/O can select
    /// this future alongside the caller token, owner token, and deadline.
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }
}

impl Default for TransportCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// One absolute deadline and one transport cancellation scope for an exchange.
#[derive(Clone)]
pub struct ExchangeContext {
    deadline: Instant,
    cancellation: TransportCancellation,
}

impl ExchangeContext {
    #[must_use]
    pub const fn new(deadline: Instant, cancellation: TransportCancellation) -> Self {
        Self {
            deadline,
            cancellation,
        }
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    #[must_use]
    pub fn cancellation(&self) -> TransportCancellation {
        self.cancellation.clone()
    }

    /// Checks terminal readiness. Cancellation is intentionally tested first,
    /// so it wins when cancellation and the deadline become ready together.
    pub fn check_at(
        &self,
        now: Instant,
        side_effect: SideEffectState,
    ) -> Result<(), UpstreamError> {
        if self.cancellation.is_cancelled() {
            return Err(UpstreamError::Cancelled(side_effect));
        }
        if now >= self.deadline {
            return Err(UpstreamError::DeadlineExceeded(side_effect));
        }
        Ok(())
    }
}

/// Distinguishes caller cancellation from owner shutdown for a prepared
/// exchange. The two tokens remain separate so a future transport operation
/// can select both wake futures and report the correct typed error.
#[derive(Clone)]
pub struct ExchangeControl {
    context: ExchangeContext,
    owner_cancellation: TransportCancellation,
}

impl ExchangeControl {
    #[must_use]
    pub fn new(context: ExchangeContext, owner_cancellation: TransportCancellation) -> Self {
        Self {
            context,
            owner_cancellation,
        }
    }

    #[must_use]
    pub const fn context(&self) -> &ExchangeContext {
        &self.context
    }

    #[must_use]
    pub fn caller_cancellation(&self) -> TransportCancellation {
        self.context.cancellation()
    }

    #[must_use]
    pub fn owner_cancellation(&self) -> TransportCancellation {
        self.owner_cancellation.clone()
    }

    /// Checks owner shutdown before caller cancellation, then the shared
    /// absolute deadline. Owner shutdown is terminal `Closed`, while explicit
    /// caller cancellation remains `Cancelled`.
    pub fn check_at(
        &self,
        now: Instant,
        side_effect: SideEffectState,
    ) -> Result<(), UpstreamError> {
        if self.owner_cancellation.is_cancelled() {
            return Err(UpstreamError::Closed(side_effect));
        }
        self.context.check_at(now, side_effect)
    }
}

/// The closed side-effect vocabulary used by all terminal transport errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideEffectState {
    NotSent,
    MaybeSent,
    Sent,
}

/// Typed transport failure categories. No error variant carries an unknown
/// side-effect state; runtime failures retain the last tracked state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamError {
    InvalidRequest(RequestError),
    InvalidEndpoint(EndpointError),
    Cancelled(SideEffectState),
    DeadlineExceeded(SideEffectState),
    Connect,
    Send(SideEffectState),
    Receive(SideEffectState),
    MalformedResponse,
    UnexpectedPeer,
    ResponseMismatch,
    TruncatedFrame,
    FrameTooLarge,
    Closed(SideEffectState),
    Runtime(SideEffectState),
}

impl UpstreamError {
    #[must_use]
    pub const fn side_effect(self) -> SideEffectState {
        match self {
            Self::InvalidRequest(_)
            | Self::InvalidEndpoint(_)
            | Self::Connect
            | Self::FrameTooLarge => SideEffectState::NotSent,
            Self::Cancelled(state)
            | Self::DeadlineExceeded(state)
            | Self::Send(state)
            | Self::Receive(state)
            | Self::Closed(state)
            | Self::Runtime(state) => state,
            Self::MalformedResponse
            | Self::UnexpectedPeer
            | Self::ResponseMismatch
            | Self::TruncatedFrame => SideEffectState::Sent,
        }
    }
}

impl fmt::Display for UpstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidRequest(_) => "invalid request",
            Self::InvalidEndpoint(_) => "invalid endpoint",
            Self::Cancelled(_) => "cancelled",
            Self::DeadlineExceeded(_) => "deadline exceeded",
            Self::Connect => "connect failure",
            Self::Send(_) => "send failure",
            Self::Receive(_) => "receive failure",
            Self::MalformedResponse => "malformed response",
            Self::UnexpectedPeer => "unexpected peer",
            Self::ResponseMismatch => "response mismatch",
            Self::TruncatedFrame => "truncated frame",
            Self::FrameTooLarge => "frame too large",
            Self::Closed(_) => "closed",
            Self::Runtime(_) => "runtime failure",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for UpstreamError {}

/// A response owns the complete wire returned by a future transport.
pub struct ExchangeResponse {
    wire: Vec<u8>,
    request_id: u16,
    response_id: u16,
    transport: Transport,
    truncated: bool,
}

impl ExchangeResponse {
    #[must_use]
    pub fn new(
        wire: Vec<u8>,
        request_id: u16,
        response_id: u16,
        transport: Transport,
        truncated: bool,
    ) -> Self {
        Self {
            wire,
            request_id,
            response_id,
            transport,
            truncated,
        }
    }

    #[must_use]
    pub fn wire(&self) -> &[u8] {
        &self.wire
    }

    #[must_use]
    pub fn into_wire(self) -> Vec<u8> {
        self.wire
    }

    #[must_use]
    pub const fn request_id(&self) -> u16 {
        self.request_id
    }

    #[must_use]
    pub const fn response_id(&self) -> u16 {
        self.response_id
    }

    #[must_use]
    pub const fn transport(&self) -> Transport {
        self.transport
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Lifecycle state for an upstream owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    Open,
    Closing,
    Closed,
}

/// Serialized owner state plus the in-flight exchange registration count.
///
/// A single `std::sync::Mutex` protects both the `Open -> Closing` admission
/// transition and exchange registration, so close can never observe a zero
/// registration count while a new exchange is registering. The lock is only
/// ever held for short synchronous critical sections; no await happens while
/// it is held.
#[derive(Debug)]
struct LifecycleInner {
    state: LifecycleState,
    in_flight: usize,
}

/// Lifecycle gate for an upstream owner.
///
/// The owner state machine and the in-flight registration count share one
/// mutex so admission (`Open -> Closing`) is atomic with registration. The
/// paired [`Notify`] wakes async drain waiters on the caller's runtime when the
/// registration count reaches zero.
#[derive(Debug)]
pub struct Lifecycle {
    inner: Mutex<LifecycleInner>,
    drained: Notify,
}

impl Lifecycle {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(LifecycleInner {
                state: LifecycleState::Open,
                in_flight: 0,
            }),
            drained: Notify::const_new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LifecycleInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn state(&self) -> LifecycleState {
        self.lock().state
    }

    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let mut inner = self.lock();
        match inner.state {
            LifecycleState::Open => {
                inner.state = LifecycleState::Closing;
                CloseTransition::BeganClosing
            }
            LifecycleState::Closing => CloseTransition::AlreadyClosing,
            LifecycleState::Closed => CloseTransition::AlreadyClosed,
        }
    }

    #[must_use]
    pub fn finish_close(&self) -> CloseCompletion {
        let mut inner = self.lock();
        match inner.state {
            LifecycleState::Open => CloseCompletion::NotClosing,
            LifecycleState::Closed => CloseCompletion::AlreadyClosed,
            // The lifecycle must never expose `Closed` while any exchange is
            // still registered.
            LifecycleState::Closing if inner.in_flight > 0 => CloseCompletion::InFlight,
            LifecycleState::Closing => {
                inner.state = LifecycleState::Closed;
                CloseCompletion::Closed
            }
        }
    }

    pub fn ensure_open(&self) -> Result<(), UpstreamError> {
        if self.state() == LifecycleState::Open {
            Ok(())
        } else {
            Err(UpstreamError::Closed(SideEffectState::NotSent))
        }
    }

    /// Registers one in-flight exchange under the same lock that gates
    /// `Open -> Closing`. A rejected registration leaves the count untouched.
    fn register(&self) -> Result<InFlightGuard<'_>, UpstreamError> {
        let mut inner = self.lock();
        if inner.state != LifecycleState::Open {
            return Err(UpstreamError::Closed(SideEffectState::NotSent));
        }
        inner.in_flight += 1;
        Ok(InFlightGuard { lifecycle: self })
    }

    /// Releases one registration. The matching guard calls this exactly once.
    fn release(&self) {
        let mut inner = self.lock();
        debug_assert!(inner.in_flight > 0, "in-flight registration underflow");
        inner.in_flight -= 1;
        let drained = inner.in_flight == 0;
        drop(inner);
        if drained {
            self.drained.notify_waiters();
        }
    }

    #[must_use]
    fn in_flight(&self) -> usize {
        self.lock().in_flight
    }

    /// Awaits the registration count reaching zero on the caller's runtime.
    ///
    /// Interest is registered with the [`Notify`] before the count is checked,
    /// so a release between the check and the await cannot be missed.
    async fn drain(&self) {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.in_flight() == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII registration guard. Any exchange that holds one exposes the owner as
/// draining until the guard is dropped, on every success, error, cancellation,
/// or aborted-future path.
struct InFlightGuard<'a> {
    lifecycle: &'a Lifecycle,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.lifecycle.release();
    }
}

/// Result of attempting to start owner shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseTransition {
    BeganClosing,
    AlreadyClosing,
    AlreadyClosed,
}

/// Result of attempting to complete owner shutdown. Completion only succeeds
/// from `Closing` after the in-flight registration count has reached zero; an
/// open owner cannot skip the observable Closing state, and a draining owner
/// reports `InFlight` instead of `Closed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseCompletion {
    Closed,
    NotClosing,
    InFlight,
    AlreadyClosed,
}

/// Result of a complete awaitable close. `Closed` means this call observed the
/// owner reach `Closed` after draining; `AlreadyClosed` means it was already
/// `Closed` when the call began.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseResult {
    Closed,
    AlreadyClosing,
    AlreadyClosed,
}

/// Pure Rust upstream owner. Slice1 performs only the reviewed UDP exchange
/// primitive on the caller's runtime; TCP and all production wiring remain
/// outside the current slice.
pub struct Upstream {
    endpoint: Endpoint,
    lifecycle: Lifecycle,
    cancellation: TransportCancellation,
}

impl Upstream {
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            lifecycle: Lifecycle::new(),
            cancellation: TransportCancellation::new(),
        }
    }

    #[must_use]
    pub const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// Number of exchanges currently registered as in-flight.
    ///
    /// This is a deterministic observability hook for the close/drain contract;
    /// it never blocks on socket I/O.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }

    /// Begins owner shutdown and cancels the owner token.
    ///
    /// The `Open -> Closing` admission transition is serialized with exchange
    /// registration, so after this returns no new exchange can register. An
    /// already-registered exchange keeps draining until its RAII guard drops.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
        }
        transition
    }

    /// Performs only the guarded `Closing -> Closed` transition.
    ///
    /// Returns [`CloseCompletion::InFlight`] while any registration is
    /// outstanding, so the lifecycle never exposes `Closed` with a non-zero
    /// registration count.
    #[must_use]
    pub fn finish_close(&self) -> CloseCompletion {
        self.lifecycle.finish_close()
    }

    /// Begins close, drains every in-flight exchange on the caller's runtime,
    /// then performs the guarded `Closing -> Closed` transition.
    ///
    /// No lock is held across the drain await; repeated and concurrent calls
    /// converge on the same `Closed` owner without deadlock. No runtime,
    /// executor, task, or blocking wait is created here.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            // Already terminal when the call began.
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        self.lifecycle.drain().await;
        match self.finish_close() {
            // Another close call may have completed the guarded transition
            // while this call was draining; either way the owner is `Closed`.
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    /// Validates and prepares an exchange before any future socket action.
    pub fn prepare_exchange<'q>(
        &self,
        request: ExchangeRequest<'q>,
        context: ExchangeContext,
    ) -> Result<PreparedExchange<'q>, UpstreamError> {
        self.lifecycle.ensure_open()?;
        if self.endpoint.transport == Transport::Tcp && request.query.len() > usize::from(u16::MAX)
        {
            return Err(UpstreamError::FrameTooLarge);
        }
        context.check_at(Instant::now(), SideEffectState::NotSent)?;
        Ok(PreparedExchange {
            endpoint: self.endpoint,
            request,
            control: ExchangeControl::new(context, self.cancellation.clone()),
        })
    }

    /// Performs one bounded exchange over the configured numeric endpoint.
    ///
    /// The exchange registers as in-flight under the same gate that serializes
    /// `Open -> Closing`, so close can never observe a zero registration count
    /// while this exchange is admitting itself. The RAII guard is held until
    /// this future returns or is dropped, covering success, every terminal
    /// error, cancellation/deadline, owner close, the TCP placeholder, and an
    /// aborted future.
    ///
    /// Slice1 implements the reviewed one-exchange/one-socket UDP primitive in
    /// [`udp::exchange`]. Plain TCP is intentionally not implemented in this
    /// entry point: it returns the explicit minimal `Runtime(NotSent)`
    /// placeholder so no TCP path can be reached silently before Slice2.
    ///
    /// # Errors
    ///
    /// Returns the explicit typed transport error for the exchange. The query
    /// is borrowed and never mutated, and any returned response owns its wire.
    pub async fn exchange<'q>(
        &self,
        request: ExchangeRequest<'q>,
        context: ExchangeContext,
    ) -> Result<ExchangeResponse, UpstreamError> {
        // Registration is the admission step: it happens before any transport
        // ownership and is released on every return/drop path by RAII.
        let _in_flight = self.lifecycle.register()?;
        let prepared = self.prepare_exchange(request, context)?;
        match prepared.endpoint().transport() {
            Transport::Udp => udp::exchange(&prepared).await,
            Transport::Tcp => Err(UpstreamError::Runtime(SideEffectState::NotSent)),
        }
    }
}

/// Validated exchange inputs held until the selected transport primitive runs.
pub struct PreparedExchange<'q> {
    endpoint: Endpoint,
    request: ExchangeRequest<'q>,
    control: ExchangeControl,
}

impl<'q> PreparedExchange<'q> {
    #[must_use]
    pub const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    #[must_use]
    pub const fn request(&self) -> &ExchangeRequest<'q> {
        &self.request
    }

    #[must_use]
    pub const fn context(&self) -> &ExchangeContext {
        self.control.context()
    }

    #[must_use]
    pub fn owner_cancellation(&self) -> TransportCancellation {
        self.control.owner_cancellation()
    }

    /// Checks owner shutdown, caller cancellation, and the shared deadline in
    /// the contractually defined order.
    pub fn check_at(
        &self,
        now: Instant,
        side_effect: SideEffectState,
    ) -> Result<(), UpstreamError> {
        self.control.check_at(now, side_effect)
    }
}
