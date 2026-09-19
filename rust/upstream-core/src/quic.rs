//! Slice 0 QUIC endpoint construction, ALPN singletons, and DoQ byte-shape
//! helpers, plus the Slice 1 one-shot DoQ exchange (Phase 4 QUIC task).
//!
//! Slice 0 is pre-I/O: endpoint validation, exact ALPN offers, and the outbound
//! ID-zeroing byte shape. Slice 1 adds [`DoqUpstream`], a fresh-connection DoQ
//! exchange that runs entirely on the caller's runtime. It connects to the
//! endpoint's numeric dial address, authenticates the separate service identity
//! with a [`TlsPolicy`]-derived configuration offering exactly `doq`, opens one
//! bidirectional QUIC stream, writes one two-byte big-endian length-prefixed
//! query whose wire ID is zeroed, signals request-side STREAM FIN, reads one
//! response up to its peer response-side STREAM FIN, checks the peer wire ID is
//! zero before restoring the caller ID, rejects a response stream that was
//! aborted instead of finished as a typed missing-FIN DoQ protocol error,
//! rejects any trailing bytes after the first declared frame as a typed DoQ
//! protocol error, validates the DNS response, and commits through the existing
//! lifecycle linearization point.
//!
//! The two-byte length prefix is not reimplemented: the outbound frame reuses
//! the frozen `dns-core` Stream framing helper through
//! [`crate::tcp::write_frame`], the same one the plain-TCP and DoT paths use.
//! No second framing codec exists. Pooling/reuse, retry/fallback, 0-RTT,
//! resumption, and H3 do not live here. A local control decision that wins
//! while the response is outstanding actively cancels the receive side with
//! RFC 9250 §4.3 `DOQ_REQUEST_CANCELLED` before the typed local control error
//! is returned.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use mosdns_dns_core::{inspect_response_header, validate_response};
use quinn::crypto::rustls::QuicClientConfig;

use crate::secure::{
    IdentityError, SecureError, SecureResponse, ServerIdentity, TlsConfigError, TlsPolicy,
};
use crate::tcp::{race_control, write_frame};
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl,
    ExchangeRequest, Lifecycle, LifecycleState, SideEffectState, TransportCancellation,
    UpstreamError,
};

/// The exact ALPN offer for DNS-over-QUIC (RFC 9250 §3).
pub const DOQ_ALPN: &[u8] = b"doq";
/// The exact ALPN offer for DNS-over-HTTP/3 (RFC 9114).
pub const H3_ALPN: &[u8] = b"h3";

/// RFC 9250 §4.3 `DOQ_REQUEST_CANCELLED`: the application error code the client
/// sends on the request stream via `STOP_SENDING` when a local control decision
/// (owner close, caller cancellation, or the shared absolute deadline) wins
/// while the response is still outstanding.
pub const DOQ_REQUEST_CANCELLED: u32 = 0x3;

/// The largest complete DoQ stream message: the DNS wire upper bound plus the
/// two-byte big-endian stream length prefix.
const MAX_DOQ_MESSAGE: usize = 65_537;

/// A DNS-over-QUIC destination: a numeric dial address plus a separate TLS
/// service identity.
///
/// Construction mirrors [`DotEndpoint`](super::secure::DotEndpoint): it
/// validates without resolving the identity and without opening a socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoqEndpoint {
    dial: SocketAddr,
    identity: ServerIdentity,
}

impl DoqEndpoint {
    /// Creates a DoQ endpoint without resolving the identity or opening a
    /// socket.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::ZeroDialPort`] when `dial` has port zero.
    pub fn new(dial: SocketAddr, identity: ServerIdentity) -> Result<Self, SecureError> {
        if dial.port() == 0 {
            return Err(SecureError::ZeroDialPort);
        }
        Ok(Self { dial, identity })
    }

    /// The numeric destination a DoQ connection is opened to.
    #[must_use]
    pub const fn dial(&self) -> SocketAddr {
        self.dial
    }

    /// The TLS service identity, independent of [`Self::dial`].
    #[must_use]
    pub const fn identity(&self) -> &ServerIdentity {
        &self.identity
    }
}

/// Zeroes the DNS transaction ID bytes of an owned outbound DoQ copy
/// (RFC 9250 §4.2.1).
///
/// The caller's borrowed query bytes are never touched: the caller copies
/// first, then zeroes the copy. `STREAM FIN` signalling is stream-transport
/// behavior and is not part of this byte helper; it is proven by the Slice 1
/// loopback fixture.
pub fn zero_outbound_query_id(outbound: &mut [u8]) {
    if outbound.len() >= 2 {
        outbound[0] = 0;
        outbound[1] = 0;
    }
}

/// A pure Rust one-shot DNS-over-QUIC owner.
///
/// The owner reuses the same [`Lifecycle`] admission/drain gate as the plain
/// and secure transports: registration is serialized with `Open -> Closing`,
/// and [`Self::close`] refuses new exchanges and returns only after every
/// registered exchange has released its guard. Every exchange opens exactly one
/// fresh QUIC connection on the caller's runtime; the connection is closed and
/// dropped before the exchange returns, so nothing is pooled or reused.
pub struct DoqUpstream {
    endpoint: DoqEndpoint,
    tls: TlsPolicy,
    lifecycle: Lifecycle,
    cancellation: TransportCancellation,
}

impl DoqUpstream {
    /// Creates a DoQ owner from a validated endpoint and an explicit policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] when the policy cannot produce a
    /// usable QUIC-compatible client configuration. The endpoint was validated
    /// when it was constructed, so no socket, resolver, or handshake work
    /// happens here.
    pub fn new(endpoint: DoqEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        // Rebuild the exact ALPN-bearing configuration once at construction so
        // an unusable policy is rejected before the first exchange instead of
        // being silently accepted.
        tls.client_config_with_alpn(&[DOQ_ALPN])?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Lifecycle::new(),
            cancellation: TransportCancellation::new(),
        })
    }

    /// The validated DoQ endpoint (numeric dial plus service identity).
    #[must_use]
    pub const fn endpoint(&self) -> &DoqEndpoint {
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

    /// Performs one bounded, fresh-connection DoQ exchange.
    ///
    /// The exchange registers as in-flight under the same gate that serializes
    /// `Open -> Closing`, so close can never observe a zero registration count
    /// while this exchange is admitting itself. The RAII guard is held until
    /// this future returns or is dropped, covering success, every terminal
    /// error, cancellation/deadline, owner close, and an aborted future.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::Tls`] when the QUIC handshake fails (always
    /// `NotSent`), and [`SecureError::Transport`] wrapping the exact typed
    /// [`UpstreamError`] for connect, control, send, receive, and DNS-response
    /// failures. A response stream that was aborted instead of completing with
    /// a normal STREAM FIN is rejected with
    /// [`SecureError::DoqProtocolMissingResponseFin`] (`Sent`), and a stream
    /// that carries a trailing second response after the first declared frame
    /// is rejected with [`SecureError::DoqProtocolTrailingResponse`] (`Sent`);
    /// neither is ever committed.
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
    ) -> Result<PreparedDoq<'a>, SecureError> {
        let in_flight = self.lifecycle.register()?;
        // The DoQ stream carries a two-byte length prefix, so the same
        // pre-connect outbound-size gate as the plain TCP and DoT paths applies.
        if request.query().len() > usize::from(u16::MAX) {
            return Err(UpstreamError::FrameTooLarge.into());
        }
        context.check_at(Instant::now(), SideEffectState::NotSent)?;
        Ok(PreparedDoq {
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

/// Validated DoQ exchange inputs held until the transport primitive runs.
///
/// The struct owns the in-flight registration guard, so dropping a prepared
/// exchange, including through an aborted caller future, releases the
/// registration without an explicit cleanup step.
struct PreparedDoq<'a> {
    endpoint: &'a DoqEndpoint,
    tls: &'a TlsPolicy,
    lifecycle: &'a Lifecycle,
    request: ExchangeRequest<'a>,
    deadline: Instant,
    caller_cancellation: TransportCancellation,
    owner_cancellation: TransportCancellation,
    /// Held only for its RAII release; never read.
    _in_flight: crate::InFlightGuard<'a>,
}

impl PreparedDoq<'_> {
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

/// Runs one fresh, authenticated DoQ exchange for a prepared request.
///
/// The single absolute deadline established by the caller covers every phase:
/// the numeric client-socket bind, QUIC connect and TLS handshake, the
/// bidirectional stream open, the framed write plus request-side FIN, the exact
/// read up to the peer response-side FIN, and the final commit. No phase starts
/// a fresh relative timeout or subtracts elapsed time.
async fn exchange_inner(prepared: &PreparedDoq<'_>) -> Result<SecureResponse, SecureError> {
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let control = prepared.control();
    let dial = prepared.endpoint.dial();
    let identity = prepared.endpoint.identity();
    let request_id = prepared.request.request_id();
    let query = prepared.request.query();
    let deadline = prepared.deadline;

    // The client configuration is rebuilt from the frozen policy for every
    // exchange, with ALPN exactly `doq`, so no mutable per-owner state can flip
    // a verified policy into an insecure one between exchanges.
    let rustls_config = prepared.tls.client_config_with_alpn(&[DOQ_ALPN])?;
    let quic_crypto = QuicClientConfig::try_from(rustls_config)
        .map_err(|_| SecureError::TlsConfig(TlsConfigError::Provider))?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));

    // Phase 1: a fresh client endpoint bound to an ephemeral local port of the
    // dial's address family. A numeric dial never resolves a name; the service
    // identity cannot select the destination.
    let local = if dial.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let mut endpoint =
        quinn::Endpoint::client(local).map_err(|_| SecureError::from(UpstreamError::Connect))?;
    endpoint.set_default_client_config(client_config);

    // Phase 2: QUIC connect and TLS handshake. The handshake completes before
    // the first DNS application byte, so a failure here is a typed TLS error
    // with `NotSent` state. No path retries or downgrades verification.
    let connecting = endpoint
        .connect(dial, identity.as_str())
        .map_err(classify_connect_error)?;
    let connection = race_control(&control, SideEffectState::NotSent, deadline, async {
        connecting.await.map_err(classify_handshake_failure)
    })
    .await?;

    // The handshake succeeded, so the peer is authenticated. No DNS byte has
    // been sent yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Phase 3: exactly one bidirectional stream: one query to one response.
    let (mut send, mut recv) = race_control(&control, SideEffectState::NotSent, deadline, async {
        connection
            .open_bi()
            .await
            .map_err(|_| SecureError::from(UpstreamError::Connect))
    })
    .await?;

    // The outbound copy (not the caller's borrowed bytes) has its wire ID
    // zeroed; the two-byte prefix comes from the shared framing helper inside
    // `write_frame`. A control error during a potentially partial write is
    // conservatively `MaybeSent`.
    let mut outbound = query.to_vec();
    zero_outbound_query_id(&mut outbound);
    race_control(&control, SideEffectState::MaybeSent, deadline, async {
        write_frame(&mut send, &outbound)
            .await
            .map_err(SecureError::from)
    })
    .await?;

    // Request-side STREAM FIN: no more request bytes will be written.
    race_control(&control, SideEffectState::MaybeSent, deadline, async {
        send.finish()
            .map_err(|_| SecureError::from(UpstreamError::Send(SideEffectState::MaybeSent)))
    })
    .await?;

    // Phase 4: read the one response up to the peer response-side STREAM FIN.
    // `read_to_end` only returns `Ok` once the FIN is observed, so its success
    // is the FIN evidence; a peer reset aborts the read instead and
    // `classify_read_error` turns that into the typed missing-FIN protocol
    // error. The request frame was fully written, so a control error while
    // waiting is `Sent`.
    let read = race_control(&control, SideEffectState::Sent, deadline, async {
        recv.read_to_end(MAX_DOQ_MESSAGE)
            .await
            .map_err(classify_read_error)
    })
    .await;
    let complete = match read {
        Ok(complete) => complete,
        Err(error) => {
            // RFC 9250 §4.3: a local control decision that wins while the
            // response is outstanding (owner close, caller cancellation, or the
            // shared absolute deadline) must actively cancel the receive side
            // with `DOQ_REQUEST_CANCELLED` before the original typed control
            // error is returned. The error itself is returned unchanged: no
            // string conversion, no new generic receive error, and no protocol
            // read failure is masked by a stop.
            if matches!(
                error,
                SecureError::Transport(
                    UpstreamError::Closed(_)
                        | UpstreamError::Cancelled(_)
                        | UpstreamError::DeadlineExceeded(_)
                )
            ) {
                let _ = recv.stop(quinn::VarInt::from_u32(DOQ_REQUEST_CANCELLED));
                // The stop frame is queued on the connection; yield once so the
                // transport driver transmits it before this exchange tears its
                // endpoint down, since an unflushed cancellation is invisible to
                // the peer. This is a scheduler yield to the I/O driver, not a
                // sleep or a retry, and it adds no timeout of its own.
                tokio::task::yield_now().await;
            }
            return Err(error);
        }
    };

    // A response shorter than a prefix cannot be framed; a zero-length body is
    // malformed. The peer's wire ID MUST be zero (RFC 9250 §4.2.1), checked
    // before the caller's original ID is restored.
    if complete.len() < 2 {
        return Err(UpstreamError::TruncatedFrame.into());
    }
    let length = usize::from(u16::from_be_bytes([complete[0], complete[1]]));
    if length == 0 {
        return Err(UpstreamError::MalformedResponse.into());
    }
    let Some(framed_body) = complete.get(2..2 + length) else {
        return Err(UpstreamError::TruncatedFrame.into());
    };
    // RFC 9250 §4.2 permits exactly one response per stream. The whole stream
    // was read up to the peer response-side FIN, so any byte after the first
    // declared frame — a second complete response or a partial trailing frame —
    // is a terminal protocol violation, never silently ignored.
    if complete.len() != 2 + length {
        return Err(SecureError::DoqProtocolTrailingResponse);
    }
    let mut body = framed_body.to_vec();
    if body.len() < 2 {
        return Err(UpstreamError::MalformedResponse.into());
    }
    if u16::from_be_bytes([body[0], body[1]]) != 0 {
        return Err(UpstreamError::ResponseMismatch.into());
    }

    // Restore the caller's original ID into the owned response copy before any
    // dns-core validation, so the committed wire carries the caller's ID.
    body[0..2].copy_from_slice(&request_id.to_be_bytes());

    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    if header.id != request_id {
        return Err(UpstreamError::ResponseMismatch.into());
    }
    if validate_response(&body).is_err() {
        return Err(UpstreamError::MalformedResponse.into());
    }

    // Phase 5: the single control-aware commit. Priority is owner, then caller,
    // then the original absolute deadline, then success; a committed response
    // can never be reversed by a later close, cancellation, or deadline.
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;

    // One-shot teardown: close the connection and the endpoint so no socket or
    // connection survives the exchange. No pooling, no idle set, no reuse.
    connection.close(0u32.into(), b"");
    endpoint.close(0u32.into(), b"");
    endpoint.wait_idle().await;

    Ok(SecureResponse::doq(body, request_id, header.truncated))
}

/// Classifies a QUIC connect/setup error without string parsing.
///
/// A malformed server name is a pre-I/O identity defect; every other setup
/// failure has sent nothing and is reported as a typed `Connect` failure.
fn classify_connect_error(error: quinn::ConnectError) -> SecureError {
    match error {
        quinn::ConnectError::InvalidServerName(_) => {
            SecureError::InvalidIdentity(IdentityError::Malformed)
        }
        _ => SecureError::from(UpstreamError::Connect),
    }
}

/// Classifies a QUIC handshake outcome as a typed TLS failure.
///
/// The handshake (certificate verification, identity check, and ALPN
/// negotiation) completes before the first DNS application byte, so every
/// failure here is `NotSent`. No branch retries, falls back, or downgrades
/// verification.
fn classify_handshake_failure(_error: quinn::ConnectionError) -> SecureError {
    SecureError::Tls(crate::secure::TlsHandshakeFailure::Other)
}

/// Classifies a stream read-to-FIN outcome.
///
/// A message above the bound is [`UpstreamError::FrameTooLarge`]. A peer reset
/// proves the response stream was aborted instead of completing with a normal
/// STREAM FIN, so it is the typed [`SecureError::DoqProtocolMissingResponseFin`]
/// protocol error; every other read failure is a terminal receive failure with
/// the request already sent. The reset outcome is matched structurally, so no
/// peer-supplied error code is ever parsed as text.
fn classify_read_error(error: quinn::ReadToEndError) -> SecureError {
    match error {
        quinn::ReadToEndError::TooLong => SecureError::from(UpstreamError::FrameTooLarge),
        quinn::ReadToEndError::Read(quinn::ReadError::Reset(_)) => {
            SecureError::DoqProtocolMissingResponseFin
        }
        quinn::ReadToEndError::Read(_) => {
            SecureError::from(UpstreamError::Receive(SideEffectState::Sent))
        }
    }
}
