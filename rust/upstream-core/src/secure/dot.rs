//! One-exchange DNS-over-TLS primitive (Phase 4 Slice1).
//!
//! [`DotUpstream`] owns a [`DotEndpoint`] and an explicit [`TlsPolicy`]. Each
//! exchange opens exactly one fresh TCP connection to the endpoint's **numeric**
//! dial address, authenticates it with a TLS handshake against the endpoint's
//! separate **service identity**, and only then writes one two-byte-length
//! prefixed DNS query and reads one complete framed response.
//!
//! The ordering is the contract, not an implementation detail:
//!
//! 1. Numeric connect. No name resolution happens here: the caller supplied a
//!    `SocketAddr`, and the service identity never selects the destination.
//! 2. Authenticated TLS handshake. A verified [`TlsPolicy`] validates the chain
//!    against the caller's roots, the service identity, and the validity window.
//! 3. Only after a successful handshake is the DNS query framed and written.
//!
//! Consequently a handshake failure is a typed TLS error with
//! `SideEffectState::NotSent`, and no path exists that sends the query in
//! plaintext or retries with verification disabled.
//!
//! Framing is not reimplemented here: [`crate::tcp::write_frame`] and
//! [`crate::tcp::read_frame`] remain the single framing implementation, shared
//! with the plain-TCP primitive. TLS adds a record buffer, so a complete send
//! additionally requires the explicit flush in [`crate::tcp::flush_bytes`].

use std::time::Instant;

use mosdns_dns_core::{inspect_response_header, validate_response};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::secure::endpoint::DotEndpoint;
use crate::secure::error::{SecureError, TlsHandshakeFailure};
use crate::secure::tls::{TlsPolicy, classify_handshake_error, server_name_for};
use crate::tcp::{flush_bytes, race_control, read_frame, write_frame};
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl,
    ExchangeRequest, Lifecycle, LifecycleState, SideEffectState, TransportCancellation,
    UpstreamError,
};

/// The transport a secure exchange reports on its response.
///
/// Deliberately distinct from [`crate::Transport`]: a DoT exchange must never
/// report itself as plain TCP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureTransport {
    /// DNS-over-TLS on a fresh authenticated connection.
    Dot,
    /// DNS-over-HTTPS on a fresh authenticated connection. The negotiated
    /// HTTP version is exposed by [`SecureResponse::http_version`].
    Doh,
}

/// The HTTP protocol selected for a DoH exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureHttpVersion {
    /// HTTP/1.1, including the no-ALPN case permitted by the DoH contract.
    Http1,
    /// HTTP/2 selected through ALPN `h2`.
    Http2,
}

/// The complete DNS wire returned by one secure exchange.
///
/// It owns the response bytes and records the original request ID and the
/// response's own ID separately, so a caller cannot confuse the ID it asked
/// with the ID it received.
#[derive(Debug)]
pub struct SecureResponse {
    wire: Vec<u8>,
    request_id: u16,
    response_id: u16,
    transport: SecureTransport,
    http_version: Option<SecureHttpVersion>,
    truncated: bool,
}

impl SecureResponse {
    /// Builds a DoH response.
    ///
    /// A DoH response is associated with its HTTP stream rather than a DNS
    /// transaction ID, so the upstream's own ID is not a routing key and may be
    /// anything. The returned wire therefore carries the caller's restored ID,
    /// and `response_id` reports the ID actually present in that wire so the
    /// metadata and the bytes can never disagree.
    #[must_use]
    pub(crate) fn doh(
        wire: Vec<u8>,
        request_id: u16,
        http_version: SecureHttpVersion,
        truncated: bool,
    ) -> Self {
        debug_assert_eq!(
            u16::from_be_bytes([wire[0], wire[1]]),
            request_id,
            "the returned DoH wire must carry the caller's restored ID"
        );
        Self {
            wire,
            request_id,
            response_id: request_id,
            transport: SecureTransport::Doh,
            http_version: Some(http_version),
            truncated,
        }
    }

    #[must_use]
    fn new(
        wire: Vec<u8>,
        request_id: u16,
        response_id: u16,
        transport: SecureTransport,
        truncated: bool,
    ) -> Self {
        Self {
            wire,
            request_id,
            response_id,
            transport,
            http_version: None,
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

    /// The original caller-supplied request ID, preserved unchanged.
    #[must_use]
    pub const fn request_id(&self) -> u16 {
        self.request_id
    }

    /// The response's own header ID, which the DoT contract requires to equal
    /// the request ID.
    #[must_use]
    pub const fn response_id(&self) -> u16 {
        self.response_id
    }

    #[must_use]
    pub const fn transport(&self) -> SecureTransport {
        self.transport
    }

    /// The negotiated DoH HTTP version, or `None` for a DoT response.
    #[must_use]
    pub const fn http_version(&self) -> Option<SecureHttpVersion> {
        self.http_version
    }

    /// Whether the complete response carried `TC=1`.
    ///
    /// A truncated DoT response is still returned to the caller; it never
    /// triggers a plaintext fallback or a second connection.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// A pure Rust DNS-over-TLS owner.
///
/// The owner reuses the same [`Lifecycle`] admission/drain gate as the plain
/// transports: registration is serialized with `Open -> Closing`, and
/// [`Self::close`] refuses new exchanges and returns only after every
/// registered exchange has released its guard.
pub struct DotUpstream {
    endpoint: DotEndpoint,
    tls: TlsPolicy,
    lifecycle: Lifecycle,
    cancellation: TransportCancellation,
    /// Test-only phase seam, absent from a production build.
    #[cfg(test)]
    pause: std::sync::Mutex<Option<std::sync::Arc<DotPause>>>,
}

/// A named phase of the DoT exchange.
///
/// The variants exist so a deterministic test can park an exchange at an exact
/// boundary and prove which phase a control terminated, without relying on
/// timing. The enum is always defined so the phase call sites compile in every
/// build; only the test seam that acts on it is test-only, and in a production
/// build every phase marker is a no-op.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DotPhase {
    /// Immediately before the numeric connect is attempted.
    BeforeConnect,
    /// Immediately before the TLS handshake is attempted.
    BeforeHandshake,
    /// Immediately before the query frame is written.
    BeforeWrite,
    /// Immediately before the TLS output is flushed.
    BeforeFlush,
    /// Immediately before the response frame is read.
    BeforeRead,
    /// Immediately before the final control-aware commit.
    BeforeCommit,
    /// Immediately after the final commit has succeeded and before the owned
    /// response is constructed.
    ///
    /// This is the window in which an owner close may legally begin after the
    /// commit has already won; nothing in the exchange may assert that the
    /// owner is still `Open` there.
    AfterCommit,
}

/// Deterministic test seam that parks an exchange at one chosen phase.
///
/// This mirrors the existing `CommitPause` seam used by the plain transports.
/// It carries no response bytes, socket, or parser state, and exists solely so
/// phase-ordering tests can prove an outcome without sleeps. The phase it
/// watches is fixed at construction.
///
/// It is compiled only for in-crate tests: [`DotUpstream::take_pause`] and the
/// prepared-exchange field are `cfg(test)` as well, so a production build
/// contains neither the seam nor a phase marker that could park an exchange.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct DotPause {
    phase: std::sync::Mutex<Option<DotPhase>>,
    arrived: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

#[cfg(test)]
impl DotPause {
    #[must_use]
    pub(crate) fn new(phase: DotPhase) -> Self {
        Self {
            arrived: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
            phase: std::sync::Mutex::new(Some(phase)),
        }
    }

    /// Parks if `phase` is the watched phase, then waits for [`Self::release`].
    ///
    /// Comparison must not consume the watched phase: the exchange visits every
    /// phase in order, so a non-match (an earlier phase) has to leave the
    /// watched phase armed for the call that does match.
    async fn reach(&self, phase: DotPhase) {
        if self.watched_phase() != Some(phase) {
            return;
        }
        let released = self.released.notified();
        tokio::pin!(released);
        // Interest is registered before the arrival is announced, so a release
        // that happens immediately afterwards cannot be missed.
        released.as_mut().enable();
        // `notify_one` stores a permit when no waiter is registered yet, so a
        // test that awaits `arrived()` after the exchange parks still observes
        // it; `notify_waiters` would drop that signal and make the handshake
        // racy.
        // The watched phase is cleared only after it has parked, so the first
        // matching visit parks exactly once.
        self.clear_phase();
        self.arrived.notify_one();
        released.await;
    }

    /// The phase this seam watches, if it has not already parked once.
    fn watched_phase(&self) -> Option<DotPhase> {
        *self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Clears the watched phase once it has parked, so a later visit to the
    /// same phase (which cannot happen for one exchange) would not park again.
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
    ///
    /// A permit is stored if the exchange has not yet awaited the release, so a
    /// release that arrives early is never lost.
    pub(crate) fn release(&self) {
        self.released.notify_one();
    }
}

impl DotUpstream {
    /// Creates a DoT owner from a validated endpoint and an explicit policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] when the policy cannot produce a
    /// usable client configuration. The endpoint was validated when it was
    /// constructed, so no socket, resolver, or handshake work happens here.
    pub fn new(endpoint: DotEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        // Reject an unusable policy at construction instead of on the first
        // exchange, so it is never silently accepted.
        tls.client_config()?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Lifecycle::new(),
            cancellation: TransportCancellation::new(),
            #[cfg(test)]
            pause: std::sync::Mutex::new(None),
        })
    }

    /// Installs the deterministic phase seam for in-crate tests.
    #[cfg(test)]
    pub(crate) fn install_pause(&self, pause: std::sync::Arc<DotPause>) {
        *self
            .pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }

    /// Takes the installed phase seam for one exchange, if any.
    ///
    /// The seam is claimed per exchange so a later exchange in the same owner
    /// is unaffected, and the claimed handle is carried by the prepared
    /// exchange rather than consulted from the owner on every phase.
    #[cfg(test)]
    fn take_pause(&self) -> Option<std::sync::Arc<DotPause>> {
        self.pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    #[must_use]
    pub const fn endpoint(&self) -> &DotEndpoint {
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
    ///
    /// Serialized with exchange registration, so no new exchange can register
    /// after this returns and registered exchanges drain through their guards.
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

    /// Performs one bounded authenticated DoT exchange.
    ///
    /// The exchange registers as in-flight under the same gate that serializes
    /// `Open -> Closing`, so close can never observe a zero registration count
    /// while this exchange is admitting itself. The RAII guard is held until
    /// this future returns or is dropped, covering success, every terminal
    /// error, cancellation/deadline, owner close, and an aborted future.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::Tls`] when the handshake fails (always
    /// `NotSent`), and [`SecureError::Transport`] wrapping the exact typed
    /// [`UpstreamError`] for connect, control, send, flush, receive, and
    /// DNS-response failures.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        let prepared = self.prepare_exchange(request, context)?;
        exchange_inner(&prepared).await
    }

    /// Registers and validates one exchange before any socket action.
    fn prepare_exchange<'a>(
        &'a self,
        request: ExchangeRequest<'a>,
        context: ExchangeContext,
    ) -> Result<PreparedDot<'a>, SecureError> {
        let in_flight = self.lifecycle.register()?;
        // The DoT frame carries a two-byte length prefix, so the same
        // pre-connect outbound-size gate as the plain TCP path applies.
        if request.query().len() > usize::from(u16::MAX) {
            return Err(UpstreamError::FrameTooLarge.into());
        }
        context.check_at(Instant::now(), SideEffectState::NotSent)?;
        Ok(PreparedDot {
            endpoint: &self.endpoint,
            tls: &self.tls,
            lifecycle: &self.lifecycle,
            request,
            deadline: context.deadline(),
            caller_cancellation: context.cancellation(),
            owner_cancellation: self.cancellation.clone(),
            #[cfg(test)]
            pause: self.take_pause(),
            _in_flight: in_flight,
        })
    }
}

/// Validated DoT exchange inputs held until the transport primitive runs.
///
/// The struct owns the in-flight registration guard, so dropping a prepared
/// exchange, including through an aborted caller future, releases the
/// registration without an explicit cleanup step.
struct PreparedDot<'a> {
    endpoint: &'a DotEndpoint,
    tls: &'a TlsPolicy,
    lifecycle: &'a Lifecycle,
    request: ExchangeRequest<'a>,
    deadline: Instant,
    caller_cancellation: TransportCancellation,
    owner_cancellation: TransportCancellation,
    /// The per-exchange deterministic phase seam, if a test installed one.
    #[cfg(test)]
    pause: Option<std::sync::Arc<DotPause>>,
    /// Held only for its RAII release; never read.
    _in_flight: crate::InFlightGuard<'a>,
}

impl PreparedDot<'_> {
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
    ///
    /// A no-op in production builds and for phases the seam does not watch, so
    /// it cannot affect the transport's real ordering.
    async fn reach(&self, phase: DotPhase) {
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

/// Runs one fresh authenticated DoT exchange for a prepared request.
///
/// The single absolute deadline established by the caller covers every phase:
/// numeric connect, TLS handshake, the framed write, the explicit flush, the
/// exact frame read, and the final commit. No phase starts a fresh relative
/// timeout or subtracts elapsed time.
async fn exchange_inner(prepared: &PreparedDot<'_>) -> Result<SecureResponse, SecureError> {
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let control = prepared.control();
    let dial = prepared.endpoint.dial();
    let request_id = prepared.request.request_id();
    let query = prepared.request.query();
    let deadline = prepared.deadline;

    // The client configuration is rebuilt from the frozen policy for every
    // exchange, so no mutable per-owner state can flip a verified policy into
    // an insecure one between exchanges.
    let config = prepared.tls.client_config()?;
    let server_name = server_name_for(prepared.endpoint.identity())?;

    // Phase 1: numeric dial. `TcpStream::connect` on a `SocketAddr` performs no
    // name resolution, so the service identity cannot influence the
    // destination. A connect failure has sent nothing.
    prepared.reach(DotPhase::BeforeConnect).await;
    let stream: TcpStream = race_control(&control, SideEffectState::NotSent, deadline, async {
        TcpStream::connect(dial)
            .await
            .map_err(|_| SecureError::from(UpstreamError::Connect))
    })
    .await?;

    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Phase 2: authenticated handshake. This is the only point where
    // verification can succeed or fail, and it completes before the first DNS
    // application byte. A verified policy's failure is terminal: no insecure
    // retry, no plaintext continuation, no second connection.
    prepared.reach(DotPhase::BeforeHandshake).await;
    let connector = TlsConnector::from(config);
    let mut tls: TlsStream<TcpStream> =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            connector
                .connect(server_name, stream)
                .await
                .map_err(|error| SecureError::Tls(classify_handshake_io_error(&error)))
        })
        .await?;

    // The handshake succeeded, so the peer is authenticated. No DNS byte has
    // been sent yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Phase 3: exactly one unchanged query frame. `write_frame` reuses the
    // shared framing helper, including its size gate and its handling of a
    // partially accepted frame.
    prepared.reach(DotPhase::BeforeWrite).await;
    race_control(&control, SideEffectState::MaybeSent, deadline, async {
        write_frame(&mut tls, query)
            .await
            .map_err(SecureError::from)
    })
    .await?;

    // Phase 4: explicit flush. A TLS stream buffers plaintext into records, so
    // a successful `write_frame` does not prove the query reached the socket.
    // The flush completes the send; a failure during it is conservatively
    // `MaybeSent` because part of the frame may already have crossed the wire.
    prepared.reach(DotPhase::BeforeFlush).await;
    race_control(&control, SideEffectState::MaybeSent, deadline, async {
        flush_bytes(&mut tls, SideEffectState::MaybeSent)
            .await
            .map_err(SecureError::from)
    })
    .await?;

    // The full frame is flushed, so the query is now `Sent`. Every control
    // error from here is reported with `Sent`, because a transmitted request
    // cannot be un-sent.
    prepared.reach(DotPhase::BeforeRead).await;
    let body = race_control(&control, SideEffectState::Sent, deadline, async {
        read_frame(&mut tls).await.map_err(SecureError::from)
    })
    .await?;

    // A response shorter than the header or with QR clear cannot be attributed
    // to this exchange as a response.
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    // The accepted response must answer this request, preserving the caller's
    // original DNS ID.
    if header.id != request_id {
        return Err(UpstreamError::ResponseMismatch.into());
    }
    // Only a complete, dns-core-valid response may be returned. A complete DoT
    // frame is the authoritative response even when it carries TC=1, so a
    // truncated response is returned rather than retried over plaintext.
    if validate_response(&body).is_err() {
        return Err(UpstreamError::MalformedResponse.into());
    }

    // Phase 5: the final control-aware commit is the single linearization point
    // against owner close, caller cancellation, and the original absolute
    // deadline. Priority is owner, then caller, then deadline, then success,
    // and a committed response can never be reversed by a later close.
    //
    // Nothing may be asserted about the owner's state after this call returns:
    // the commit wins under the lifecycle mutex and an owner close may legally
    // begin immediately afterwards, moving the owner to `Closing` while this
    // response is still the committed outcome. The committed response is
    // returned unchanged, and the registration guard keeps the owner draining
    // until this call's frame drops.
    prepared.reach(DotPhase::BeforeCommit).await;
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;

    // The commit has won. An owner close may begin at any instant from here on;
    // this marker exists so a deterministic test can prove that outcome without
    // asserting anything about the owner's state.
    prepared.reach(DotPhase::AfterCommit).await;

    Ok(SecureResponse::new(
        body,
        request_id,
        header.id,
        SecureTransport::Dot,
        header.truncated,
    ))
}

/// Classifies a tokio-rustls handshake failure without string parsing.
///
/// tokio-rustls transports the underlying `rustls::Error` as the source of an
/// `io::Error`, so the structured cause is recovered by downcast. Recovery is
/// best-effort: an unrecognized cause still yields a typed TLS failure, and no
/// branch echoes certificate bytes or key material.
fn classify_handshake_io_error(error: &std::io::Error) -> TlsHandshakeFailure {
    if let Some(rustls_error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<rustls::Error>())
    {
        return classify_handshake_error(rustls_error);
    }
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => TlsHandshakeFailure::UnexpectedEof,
        _ => TlsHandshakeFailure::Protocol,
    }
}

/// A retained, authenticated DoT session available for one reuse.
///
/// The pooled-session type lives here, beside the protocol code that owns the
/// handshake and framing, so reuse composes the existing `write_frame`/
/// `flush_bytes`/`read_frame` helpers and the same control race instead of
/// duplicating a second DoT state machine.
///
/// The session carries only the established stream; the identity it was
/// authenticated against is part of the reuse key that owns it, never stored
/// here, so a session can never be handed to a different identity.
pub(crate) struct PooledDotSession {
    tls: TlsStream<TcpStream>,
}

impl PooledDotSession {
    /// Dials and authenticates one DoT session for `endpoint` under `tls`.
    ///
    /// Ordering is the reviewed DoT contract: numeric connect first, then the
    /// authenticated handshake against the service identity, and only then is
    /// the session available for a query. A connect or handshake failure has
    /// sent no DNS byte, so it is `NotSent`.
    ///
    /// # Errors
    ///
    /// Returns the exact typed [`SecureError`] the fresh DoT path returns for a
    /// connect, handshake, or control failure.
    pub(crate) async fn connect(
        endpoint: &DotEndpoint,
        tls: &TlsPolicy,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<Self, SecureError> {
        let config = tls.client_config()?;
        let server_name = server_name_for(endpoint.identity())?;
        let dial = endpoint.dial();

        // Phase 1: numeric dial. No name resolution, so the service identity
        // cannot influence the destination.
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

        Ok(Self { tls: tls_stream })
    }

    /// Reports whether the peer has already closed this session.
    ///
    /// A TLS session cannot be probed by a plain socket read without consuming
    /// record bytes, so this asks the underlying TCP stream for readable-at-EOF
    /// without reading TLS data. `WouldBlock` is the healthy idle case.
    #[must_use]
    pub(crate) fn is_peer_closed(&self) -> bool {
        let mut probe = [0u8; 1];
        // `TlsStream::get_ref` yields the underlying stream and the TLS session.
        let (stream, _session) = self.tls.get_ref();
        match stream.try_read(&mut probe) {
            Ok(0) | Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
        }
    }

    /// Runs one framed DoT exchange on this established session.
    ///
    /// Reuses the shared framing helpers and the shared control race, so the
    /// deadline, cancellation, and close semantics are identical to the fresh
    /// path. A failure after the query was flushed is terminal.
    pub(crate) async fn exchange(
        mut self,
        request: &ExchangeRequest<'_>,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<(SecureResponse, Self), PooledDotOutcome> {
        let request_id = request.request_id();
        let query = request.query();

        // One unchanged query frame, then the explicit flush a TLS record
        // buffer requires. A flush failure is conservatively `MaybeSent`.
        if let Err(error) = race_control(control, SideEffectState::MaybeSent, deadline, async {
            write_frame(&mut self.tls, query)
                .await
                .map_err(SecureError::from)
        })
        .await
        {
            return Err(PooledDotOutcome::failed(error, self));
        }

        if let Err(error) = race_control(control, SideEffectState::MaybeSent, deadline, async {
            flush_bytes(&mut self.tls, SideEffectState::MaybeSent)
                .await
                .map_err(SecureError::from)
        })
        .await
        {
            return Err(PooledDotOutcome::failed(error, self));
        }

        // The frame is flushed, so the query is `Sent` from here on.
        let body = match race_control(control, SideEffectState::Sent, deadline, async {
            read_frame(&mut self.tls).await.map_err(SecureError::from)
        })
        .await
        {
            Ok(body) => body,
            Err(error) => return Err(PooledDotOutcome::failed(error, self)),
        };

        let header = match inspect_response_header(&body) {
            Ok(header) => header,
            Err(_) => {
                return Err(PooledDotOutcome::failed(
                    SecureError::from(UpstreamError::MalformedResponse),
                    self,
                ));
            }
        };
        if header.id != request_id {
            return Err(PooledDotOutcome::failed(
                UpstreamError::ResponseMismatch.into(),
                self,
            ));
        }
        if validate_response(&body).is_err() {
            return Err(PooledDotOutcome::failed(
                UpstreamError::MalformedResponse.into(),
                self,
            ));
        }

        Ok((
            SecureResponse::new(
                body,
                request_id,
                header.id,
                SecureTransport::Dot,
                header.truncated,
            ),
            self,
        ))
    }
}

/// The terminal outcome of one pooled DoT attempt.
///
/// The session is always returned so the owner can decide whether to retain it.
/// `rebuildable` is true only when no DNS byte was written, which is the single
/// case in which the owner may dial a replacement session.
pub(crate) struct PooledDotOutcome {
    pub(crate) error: SecureError,
    pub(crate) session: PooledDotSession,
    pub(crate) rebuildable: bool,
}

impl PooledDotOutcome {
    /// Classifies a failure, deciding whether a replacement dial is allowed.
    fn failed(error: SecureError, session: PooledDotSession) -> Self {
        // Only a `NotSent` failure provably left no byte with the peer. An idle
        // session the peer closed fails on the first write with zero bytes
        // accepted, which is exactly that case.
        let rebuildable = matches!(error.side_effect(), SideEffectState::NotSent);
        Self {
            error,
            session,
            rebuildable,
        }
    }
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
    use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::time::timeout;
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::server::TlsStream;

    use super::{DotPhase, DotUpstream};
    use crate::secure::endpoint::{DotEndpoint, ServerIdentity};
    use crate::secure::error::SecureError;
    use crate::secure::tls::TlsPolicy;
    use crate::{
        CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, SideEffectState,
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
    ///
    /// The keys exist only for this test process and are never written to disk,
    /// matching the integration-test fixture contract. The leaf is genuinely a
    /// `CA:FALSE` end entity with `serverAuth`, so a real handshake verifies it
    /// rather than failing on a CA certificate used as a leaf.
    struct Identity {
        /// The trust anchor the client is configured with.
        ca_der: rustls::pki_types::CertificateDer<'static>,
        /// The server's leaf certificate.
        leaf_der: rustls::pki_types::CertificateDer<'static>,
        /// The leaf's private key.
        leaf_key: KeyPair,
    }

    /// Generates a fresh CA and a `dns.example` leaf signed by it.
    fn generate_identity() -> Identity {
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("generate a synthetic CA key");
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "mosdns-dot-test-root");
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

    /// A scripted TLS server running on its own thread and runtime.
    ///
    /// `script` runs after a successful handshake; it receives the stream and
    /// the test's `oneshot` sender so the test can gate on the real phase.
    struct Server {
        address: SocketAddr,
        handle: std::thread::JoinHandle<()>,
    }

    impl Server {
        fn start<F, Fut>(
            identity: &Identity,
            consumed: Option<oneshot::Sender<()>>,
            script: F,
        ) -> Self
        where
            F: FnOnce(TlsStream<TcpStream>, Option<oneshot::Sender<()>>) -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send,
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
                    let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
                    let (stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                        .await
                        .expect("a connection arrives")
                        .expect("accept succeeds");
                    let acceptor = TlsAcceptor::from(config);
                    if let Ok(Ok(tls)) = timeout(TEST_TIMEOUT, acceptor.accept(stream)).await {
                        script(tls, consumed).await;
                    }
                });
            });
            Self { address, handle }
        }

        fn join(self) {
            self.handle.join().expect("server thread joined");
        }
    }

    /// Reads exactly one framed DNS message.
    async fn read_framed(stream: &mut TlsStream<TcpStream>) -> Vec<u8> {
        use tokio::io::AsyncReadExt as _;
        let mut prefix = [0u8; 2];
        stream.read_exact(&mut prefix).await.expect("read prefix");
        let length = usize::from(u16::from_be_bytes(prefix));
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.expect("read body");
        body
    }

    /// Reads until the client closes the connection, then returns.
    ///
    /// This is the deterministic counterpart to a "hold the connection open"
    /// sleep: the server returns as soon as the exchange under test drops its
    /// stream, so joining the server thread can never hang, and nothing depends
    /// on elapsed time.
    async fn hold_until_client_closes(stream: &mut TlsStream<TcpStream>) {
        use tokio::io::AsyncReadExt as _;
        let mut scratch = [0u8; 64];
        loop {
            match stream.read(&mut scratch).await {
                // EOF: the client dropped its stream.
                Ok(0) => return,
                Ok(_) => {}
                // Any transport error is also the end of this server's interest.
                Err(_) => return,
            }
        }
    }

    /// Reads one framed DNS message, or `None` if the client left first.
    ///
    /// Unlike [`read_framed`], this tolerates a client that is parked before it
    /// ever writes, which is exactly what the write- and flush-phase cases do.
    async fn read_framed_opt(stream: &mut TlsStream<TcpStream>) -> Option<Vec<u8>> {
        use tokio::io::AsyncReadExt as _;
        let mut prefix = [0u8; 2];
        if stream.read_exact(&mut prefix).await.is_err() {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(prefix));
        if length == 0 {
            return None;
        }
        let mut body = vec![0u8; length];
        match stream.read_exact(&mut body).await {
            Ok(_) => Some(body),
            Err(_) => None,
        }
    }

    /// Writes one framed DNS message, or reports that the client left first.
    async fn write_framed_opt(stream: &mut TlsStream<TcpStream>, body: &[u8]) -> bool {
        use tokio::io::AsyncWriteExt as _;
        let Ok(length) = u16::try_from(body.len()) else {
            return false;
        };
        let mut frame = Vec::with_capacity(body.len() + 2);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(body);
        match stream.write_all(&frame).await {
            Ok(()) => stream.flush().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Writes one framed DNS message.
    async fn write_framed(stream: &mut TlsStream<TcpStream>, body: &[u8]) {
        use tokio::io::AsyncWriteExt as _;
        let length = u16::try_from(body.len()).expect("response fits the u16 prefix");
        let mut frame = Vec::with_capacity(body.len() + 2);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(body);
        stream.write_all(&frame).await.expect("write frame");
        stream.flush().await.expect("flush frame");
    }

    /// Builds an owner plus the endpoint identity for the given server.
    fn owner_for(address: SocketAddr, identity: &Identity) -> DotUpstream {
        DotUpstream::new(
            DotEndpoint::new(
                address,
                ServerIdentity::new("dns.example").expect("valid identity"),
            )
            .expect("valid endpoint"),
            client_policy(identity),
        )
        .expect("owner")
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

    /// The owner/caller/deadline context used by most phase tests.
    fn open_context(caller: &TransportCancellation) -> ExchangeContext {
        ExchangeContext::new(Instant::now() + Duration::from_secs(30), caller.clone())
    }

    /// Spawns one exchange so the test can apply a control while it is parked.
    fn spawn(
        upstream: &Arc<DotUpstream>,
        query: Vec<u8>,
        context: ExchangeContext,
    ) -> tokio::task::JoinHandle<Result<super::SecureResponse, SecureError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, context).await
        })
    }

    /// Installs a phase seam on the owner and returns it to the test.
    fn install(upstream: &DotUpstream, phase: DotPhase) -> Arc<super::DotPause> {
        let pause = Arc::new(super::DotPause::new(phase));
        upstream.install_pause(Arc::clone(&pause));
        pause
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
    const MATRIX_PHASES: [DotPhase; 6] = [
        DotPhase::BeforeConnect,
        DotPhase::BeforeHandshake,
        DotPhase::BeforeWrite,
        DotPhase::BeforeFlush,
        DotPhase::BeforeRead,
        DotPhase::BeforeCommit,
    ];

    /// The side-effect state the exchange has provably reached when parked at
    /// `phase`.
    ///
    /// This mirrors the production layering exactly: no DNS byte exists before
    /// or during the handshake, a written or still-buffered frame is
    /// conservatively `MaybeSent`, and a flushed frame is `Sent`. Every control
    /// at a given phase must report the same state, which is what makes the
    /// matrix a real contract rather than four independent assertions.
    const fn side_effect_at(phase: DotPhase) -> SideEffectState {
        match phase {
            DotPhase::BeforeConnect | DotPhase::BeforeHandshake => SideEffectState::NotSent,
            DotPhase::BeforeWrite | DotPhase::BeforeFlush => SideEffectState::MaybeSent,
            DotPhase::BeforeRead | DotPhase::BeforeCommit | DotPhase::AfterCommit => {
                SideEffectState::Sent
            }
        }
    }

    /// The request ID every matrix case sends.
    const MATRIX_REQUEST_ID: u16 = 0x6A00;

    /// A server that answers the query when it arrives, then holds until the
    /// client closes the connection.
    ///
    /// Every step tolerates the client leaving early, which is what lets one
    /// helper serve every phase in the matrix: a client parked before its write
    /// never sends the query, while a client parked at the read or commit phase
    /// needs the response. Returning on client EOF keeps `Server::join` free of
    /// any timing dependency.
    fn matrix_server(identity: &Identity) -> Server {
        Server::start(identity, None, |mut tls, _| async move {
            if read_framed_opt(&mut tls).await.is_some() {
                let _ = write_framed_opt(&mut tls, &response_wire(MATRIX_REQUEST_ID, 9)).await;
            }
            hold_until_client_closes(&mut tls).await;
        })
    }

    /// Runs one `(phase, control)` matrix cell and asserts its contract.
    ///
    /// The exchange is parked on `phase` by the deterministic seam, so the
    /// control is applied at a known point rather than inferred from elapsed
    /// time. The exchange must have reached the seam before the control is
    /// applied, which is the ordering evidence for every cell.
    fn run_phase_control_case(phase: DotPhase, control: PhaseControl) {
        block_on(async {
            let identity = generate_identity();

            // `BeforeConnect` is decided before any socket exists, so it needs
            // no server: the address is bound but never accepted.
            let (address, server) = if phase == DotPhase::BeforeConnect {
                let (listener, address) = bind();
                listener
                    .set_nonblocking(true)
                    .expect("listener non-blocking");
                (address, None)
            } else {
                let server = matrix_server(&identity);
                (server.address, Some(server))
            };

            let upstream = Arc::new(owner_for(address, &identity));
            let pause = install(&upstream, phase);

            let caller = TransportCancellation::new();
            let deadline = Instant::now() + Duration::from_millis(500);
            let context = ExchangeContext::new(deadline, caller.clone());
            let exchange = spawn(&upstream, query_wire(MATRIX_REQUEST_ID), context);

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .unwrap_or_else(|_| {
                    panic!("{phase:?}/{control:?}: the exchange reaches its phase")
                });
            assert_eq!(
                upstream.in_flight_exchanges(),
                1,
                "{phase:?}/{control:?}: the exchange is registered while parked"
            );

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
                    // Wait for the exchange's own absolute deadline instant; no
                    // arbitrary sleep stands in for the deadline.
                    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                    pause.release();
                    expect_error(
                        exchange,
                        SecureError::Transport(UpstreamError::DeadlineExceeded(expected)),
                    )
                    .await;
                }
                PhaseControl::AbortDrop => {
                    // Aborting the task is what drops the in-flight future; the
                    // RAII registration must be released by that drop alone.
                    exchange.abort();
                    let _ = exchange.await;
                    assert_eq!(
                        upstream.close().await,
                        CloseResult::Closed,
                        "{phase:?}/{control:?}: the owner drains after the future is dropped"
                    );
                }
            }

            assert_eq!(
                upstream.in_flight_exchanges(),
                0,
                "{phase:?}/{control:?}: every registration is released"
            );
            if let Some(server) = server {
                server.join();
            }
        });
    }

    /// Runs all four controls against one phase.
    fn run_phase_control_matrix(phase: DotPhase) {
        for control in [
            PhaseControl::OwnerClose,
            PhaseControl::CallerCancellation,
            PhaseControl::AbsoluteDeadline,
            PhaseControl::AbortDrop,
        ] {
            run_phase_control_case(phase, control);
        }
    }

    /// The phases in the matrix must be exactly the pre-result phases: a phase
    /// silently missing from the const array would otherwise leave a gap in the
    /// coverage this test exists to provide.
    #[test]
    fn the_control_matrix_covers_every_pre_result_phase_exactly_once() {
        let mut seen: Vec<DotPhase> = MATRIX_PHASES.to_vec();
        seen.sort_by_key(|phase| format!("{phase:?}"));
        seen.dedup();
        assert_eq!(seen.len(), MATRIX_PHASES.len(), "no duplicate phases");
        assert_eq!(
            seen.len(),
            6,
            "connect, handshake, write, flush, read and final commit"
        );
        // The side-effect layering is part of the contract, so assert it here
        // rather than only inside the per-phase cases.
        assert_eq!(
            side_effect_at(DotPhase::BeforeConnect),
            SideEffectState::NotSent
        );
        assert_eq!(
            side_effect_at(DotPhase::BeforeHandshake),
            SideEffectState::NotSent
        );
        assert_eq!(
            side_effect_at(DotPhase::BeforeWrite),
            SideEffectState::MaybeSent
        );
        assert_eq!(
            side_effect_at(DotPhase::BeforeFlush),
            SideEffectState::MaybeSent
        );
        assert_eq!(side_effect_at(DotPhase::BeforeRead), SideEffectState::Sent);
        assert_eq!(
            side_effect_at(DotPhase::BeforeCommit),
            SideEffectState::Sent
        );
    }

    #[test]
    fn control_matrix_at_before_connect() {
        run_phase_control_matrix(DotPhase::BeforeConnect);
    }

    #[test]
    fn control_matrix_at_before_handshake() {
        run_phase_control_matrix(DotPhase::BeforeHandshake);
    }

    #[test]
    fn control_matrix_at_before_write() {
        run_phase_control_matrix(DotPhase::BeforeWrite);
    }

    #[test]
    fn control_matrix_at_before_flush() {
        run_phase_control_matrix(DotPhase::BeforeFlush);
    }

    #[test]
    fn control_matrix_at_before_read() {
        run_phase_control_matrix(DotPhase::BeforeRead);
    }

    #[test]
    fn control_matrix_at_before_commit() {
        run_phase_control_matrix(DotPhase::BeforeCommit);
    }

    #[test]
    fn owner_close_immediately_after_a_winning_commit_still_returns_the_response() {
        block_on(async {
            // P1-1 regression: the commit wins the lifecycle gate, and the owner
            // may legally begin closing immediately afterwards. The exchange must
            // return the committed response without panicking or late-failing.
            let id = 0x6007;
            let expected = response_wire(id, 7);
            let reply = expected.clone();
            let identity = generate_identity();
            let server = Server::start(&identity, None, move |mut tls, _| async move {
                let _ = read_framed(&mut tls).await;
                write_framed(&mut tls, &reply).await;
            });
            let upstream = Arc::new(owner_for(server.address, &identity));
            let pause = install(&upstream, DotPhase::AfterCommit);

            let exchange = spawn(
                &upstream,
                query_wire(id),
                open_context(&TransportCancellation::new()),
            );
            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("the exchange reaches the post-commit window");
            // The commit has already won; close now, in the window that used to
            // trip the removed assertion.
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            assert_eq!(
                upstream.lifecycle_state(),
                crate::LifecycleState::Closing,
                "the owner is legally Closing after the commit won"
            );
            pause.release();

            let response = timeout(TEST_TIMEOUT, exchange)
                .await
                .expect("exchange bounded")
                .expect("exchange task joined")
                .expect("a committed response is returned even though close followed");
            assert_eq!(response.wire(), expected.as_slice());
            assert_eq!(response.request_id(), id);

            // The registration is released and the owner drains to Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join();
        });
    }

    #[test]
    fn pre_commit_phase_close_loses_the_gate_with_closed_sent() {
        block_on(async {
            // The complement of the regression above: when the close reaches the
            // gate first, the commit loses and the exchange reports Closed.
            let id = 0x6008;
            let reply = response_wire(id, 8);
            let identity = generate_identity();
            let server = Server::start(&identity, None, move |mut tls, _| async move {
                let _ = read_framed(&mut tls).await;
                write_framed(&mut tls, &reply).await;
            });
            let upstream = Arc::new(owner_for(server.address, &identity));
            let pause = install(&upstream, DotPhase::BeforeCommit);

            let exchange = spawn(
                &upstream,
                query_wire(id),
                open_context(&TransportCancellation::new()),
            );
            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("the exchange reaches the pre-commit gate");
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            pause.release();

            expect_error(
                exchange,
                SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent)),
            )
            .await;
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join();
        });
    }

    #[test]
    fn the_first_bytes_a_peer_receives_are_tls_handshake_never_a_plaintext_query() {
        block_on(async {
            // Deterministic proof that no DNS query can precede the handshake.
            // The peer is a raw TCP listener: whatever the client sends arrives
            // uninspected, so the very first byte tells us whether the client
            // started a TLS handshake (record type 0x16) or leaked the DNS query
            // as plaintext. The query's own first byte is the high byte of its
            // 16-bit length prefix, which is 0x00 for any realistic query, so the
            // two cases cannot be confused.
            let (listener, address) = bind();
            listener.set_nonblocking(true).expect("non-blocking");
            let identity = generate_identity();
            let query = query_wire(0x6300);

            let probe = {
                let query = query.clone();
                tokio::task::spawn_blocking(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build probe runtime");
                    runtime.block_on(async move {
                        let listener =
                            AsyncTcpListener::from_std(listener).expect("adopt listener");
                        let (mut stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                            .await
                            .expect("the client connects")
                            .expect("accept succeeds");
                        use tokio::io::AsyncReadExt as _;
                        let mut buffer = [0u8; 64];
                        // The client's first flight arrives promptly after
                        // connecting; a bounded read just keeps a broken client
                        // from hanging the test.
                        let read = timeout(TEST_TIMEOUT, stream.read(&mut buffer))
                            .await
                            .expect("the client sends its first flight")
                            .expect("read the client's first bytes");
                        assert!(read > 0, "the client must send a TLS ClientHello");
                        // A TLS handshake record, not the DNS query.
                        assert_eq!(
                            buffer[0], 0x16,
                            "the first byte must be a TLS handshake record type"
                        );
                        assert_ne!(
                            &buffer[..read.min(2)],
                            &query[..read.min(2)],
                            "the client must not send the DNS query frame before its handshake"
                        );
                    });
                })
            };

            let upstream = owner_for(address, &identity);
            let caller = TransportCancellation::new();
            let request = ExchangeRequest::new(&query).expect("valid query");
            let error = timeout(
                TEST_TIMEOUT,
                upstream.exchange(request, open_context(&caller)),
            )
            .await
            .expect("exchange bounded")
            .expect_err("a peer that never completes TLS cannot carry a query");
            // The handshake never completed, so nothing was sent as DNS.
            assert_eq!(
                error.side_effect(),
                SideEffectState::NotSent,
                "a stalled handshake must still report NotSent: {error:?}"
            );
            assert_eq!(upstream.in_flight_exchanges(), 0);

            probe.await.expect("probe task joined");
        });
    }

    #[test]
    fn the_handshake_authenticates_the_service_identity_not_the_dial_address() {
        block_on(async {
            // The server's certificate covers `dns.example` and the client dials
            // a loopback address. A verified handshake can only succeed if the
            // client validated the certificate name against the configured
            // service identity, so a successful exchange proves the dial
            // address did not stand in for the identity.
            let identity = generate_identity();
            let server = Server::start(&identity, None, |mut tls, _| async move {
                let _ = read_framed(&mut tls).await;
                write_framed(&mut tls, &response_wire(0x6400, 4)).await;
            });

            let upstream = owner_for(server.address, &identity);
            assert_eq!(
                upstream.endpoint().dial().ip(),
                std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
                "the dial destination is the numeric loopback address"
            );
            assert_eq!(upstream.endpoint().identity().as_str(), "dns.example");

            let query = query_wire(0x6400);
            let request = ExchangeRequest::new(&query).expect("valid query");
            let response = timeout(
                TEST_TIMEOUT,
                upstream.exchange(request, open_context(&TransportCancellation::new())),
            )
            .await
            .expect("exchange bounded")
            .expect("the certificate name matches the service identity, not the dial address");
            assert_eq!(response.request_id(), 0x6400);
            assert_eq!(response.response_id(), 0x6400);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join();
        });
    }

    #[test]
    fn a_certificate_for_a_different_identity_is_rejected_at_handshake() {
        block_on(async {
            // The complement: the same server configured under a different
            // service identity must fail the handshake, proving the identity is
            // actually checked rather than accepted from the certificate.
            let identity = generate_identity();
            let server = Server::start(&identity, None, |_tls, _| async {});

            let upstream = DotUpstream::new(
                DotEndpoint::new(
                    server.address,
                    ServerIdentity::new("other.example").expect("valid identity"),
                )
                .expect("valid endpoint"),
                client_policy(&identity),
            )
            .expect("owner");

            let query = query_wire(0x6401);
            let request = ExchangeRequest::new(&query).expect("valid query");
            let error = timeout(
                TEST_TIMEOUT,
                upstream.exchange(request, open_context(&TransportCancellation::new())),
            )
            .await
            .expect("exchange bounded")
            .expect_err("a name mismatch must fail the handshake");
            assert_eq!(
                error,
                SecureError::Tls(crate::secure::error::TlsHandshakeFailure::Certificate(
                    crate::secure::error::CertificateRejection::NotValidForName
                ))
            );
            assert_eq!(error.side_effect(), SideEffectState::NotSent);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            server.join();
        });
    }
}
