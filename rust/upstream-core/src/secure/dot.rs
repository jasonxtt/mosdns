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
    truncated: bool,
}

impl SecureResponse {
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
        })
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
    race_control(&control, SideEffectState::MaybeSent, deadline, async {
        flush_bytes(&mut tls, SideEffectState::MaybeSent)
            .await
            .map_err(SecureError::from)
    })
    .await?;

    // The full frame is flushed, so the query is now `Sent`. Every control
    // error from here is reported with `Sent`, because a transmitted request
    // cannot be un-sent.
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
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;

    // Owner close wins the admission gate over an in-flight registration, so a
    // committed response keeps its registration until this guard drops.
    debug_assert_eq!(prepared.lifecycle.state(), LifecycleState::Open);

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
