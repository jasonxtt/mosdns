//! Slice 0 QUIC endpoint construction, ALPN singletons, and DoQ byte-shape
//! helpers, plus the Slice 1 one-shot DoQ exchange and the Slice 2 one-shot
//! DNS-over-HTTP/3 exchange (Phase 4 QUIC task).
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
//! lifecycle linearization point. Slice 2 adds [`Doh3Upstream`], the same
//! one-shot shape over HTTP/3: one fresh QUIC connection, one `GET` encoded by
//! the existing `DohEndpoint`, and the frozen DoH response contract (status,
//! media type, encoding, header bounds, and complete bounded body) whose
//! validation is shared with the HTTP/1.1 and HTTP/2 paths instead of being
//! reimplemented.
//!
//! The two-byte length prefix is not reimplemented: the outbound frame reuses
//! the frozen `dns-core` Stream framing helper through
//! [`crate::tcp::write_frame`], the same one the plain-TCP and DoT paths use.
//! No second framing codec exists. Pooling/reuse, retry/fallback, and 0-RTT /
//! resumption do not live here. Once the bidirectional stream is open, a local
//! control decision that wins the outbound write, the request-side `finish`, or
//! the response read calls `RecvStream::stop` with RFC 9250 §4.3
//! `DOQ_REQUEST_CANCELLED` and then returns the original typed local control
//! error unchanged. That call is best-effort local cancellation: Quinn 0.11.7
//! exposes no awaitable `STOP_SENDING` flush or peer-acknowledgement future, so
//! production makes no claim that the frame has reached, or was observed by,
//! the peer. A debug-only test seam (`DoqStopPause`) lets an independent
//! loopback test park the exchange after that `stop` so the real Quinn driver
//! can transmit it before the peer is inspected; the seam is a test observation
//! device, never a production flush mechanism.
//!
//! The HTTP/3 driver never detaches: the h3 connection it must poll continuously
//! runs as a tracked child of the exchange scope (the same seal/abort/drain
//! ownership the HTTP/2 driver uses), and the final lifecycle commit happens
//! only after that scope has drained.
//!
//! Slice 3 closes the control and error-mapping contract. Every phase of both
//! transports runs inside the shared [`race_control`] race against the caller's
//! one absolute deadline and the owner/caller tokens, so connect, handshake,
//! stream open, write, and read all share the same deadline and the fixed owner
//! close → caller cancellation → deadline → commit precedence; no private timer
//! exists. A local control decision that wins after the h3 request stream exists
//! cannot safely send `H3_REQUEST_CANCELLED` through the pinned h3 0.0.8 /
//! h3-quinn 0.0.10 API (a cancelled read leaves h3-quinn's inner stream `None`
//! and `stop_sending` then panics), so the typed control error is returned
//! unchanged - `Closed`, `Cancelled`, or `DeadlineExceeded` - and is never
//! disguised as a peer failure; the boundary and its evidence are documented on
//! [`run_h3_request`]. Peer h3 stream terminations are mapped structurally
//! (never by parsing a reason string) to [`DohProtocolError::PeerStreamTerminated`]
//! with a closed [`PeerStreamError`] category from the RFC 9114 §8.1 HTTP/3 code
//! space, classified by an explicit request/response-stream context allowlist
//! with RFC 9114 §8's unexpected/unknown handling applied; a DoQ nonzero
//! peer wire ID is the typed [`SecureError::DoqProtocolNonzeroResponseId`]
//! protocol error, and a DoQ reset with any RFC 9250 code remains the terminal
//! missing-response-FIN error. An ordinary h3 response-head failure after the
//! request send side finished is a `Sent` receive failure, never the weaker
//! `MaybeSent` of the HTTP/1.1 and HTTP/2 head-not-received variant. None of
//! these ever commits.

use std::future::poll_fn;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use hyper::body::Buf as _;
use mosdns_dns_core::{inspect_response_header, validate_response};
use quinn::crypto::rustls::QuicClientConfig;

use crate::secure::{
    DNS_MEDIA_TYPE, DohEndpoint, DohProtocolError, DohRequestError, H2ScopeLease, IdentityError,
    MAX_DNS_BODY, MAX_RESPONSE_HEADER_BYTES, MAX_RESPONSE_HEADERS, PeerStreamError, SecureError,
    SecureResponse, ServerIdentity, TlsConfigError, TlsPolicy, parsed_head_bytes,
    restore_request_id, validate_doh_head_parts,
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

/// Debug-only deterministic observation pause for the post-`stop` DoQ
/// cancellation seam.
///
/// **This is a test observation device, not a production flush mechanism.**
/// Production never waits for `STOP_SENDING` to reach the peer: Quinn 0.11.7
/// exposes no awaitable stop-flush or peer-acknowledgement future, and
/// [`DoqUpstream::exchange`] returns its typed local control error as soon as
/// [`quinn::RecvStream::stop`] has been called. The pause exists so an
/// independent loopback integration test can keep the exchange future pending
/// after that `stop` while the real Quinn driver runs on the caller's runtime,
/// observe the peer's [`quinn::SendStream::stopped`] evidence, and only then
/// release the exchange. It adds no sleep, no scheduler yield, and no timeout
/// of its own.
///
/// The whole seam is compiled only under `debug_assertions`, so a release
/// library contains no installer and no pause path.
#[cfg(debug_assertions)]
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct DoqStopPause {
    arrived: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

#[cfg(debug_assertions)]
impl DoqStopPause {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Signals that the exchange parked after `stop`, then waits until the test
    /// releases it. Interest in the release is registered before the arrival is
    /// announced, so a release can never be missed.
    async fn pause(&self) {
        let released = self.released.notified();
        tokio::pin!(released);
        released.as_mut().enable();
        self.arrived.notify_one();
        released.await;
    }

    /// Waits until an exchange has parked on this pause.
    pub async fn arrived(&self) {
        self.arrived.notified().await;
    }

    /// Releases a parked exchange.
    pub fn release(&self) {
        self.released.notify_one();
    }
}

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
    /// Debug-only test observation seam: installed once, consumed by the next
    /// exchange. Absent from release builds.
    #[cfg(debug_assertions)]
    stop_pause: std::sync::Mutex<Option<Arc<DoqStopPause>>>,
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
            #[cfg(debug_assertions)]
            stop_pause: std::sync::Mutex::new(None),
        })
    }

    /// Installs the debug-only deterministic post-`stop` observation pause for
    /// an independent loopback test.
    ///
    /// This is a **test observation seam**, not a production flush mechanism.
    /// The first install wins and is consumed by the next exchange; later
    /// installs are ignored. The method and the pause itself exist only under
    /// `debug_assertions`, so a release library has no way to pause an
    /// exchange waiting for a peer to observe `STOP_SENDING`. A poisoned lock
    /// is recovered rather than propagated, so this cannot panic.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn install_stop_pause(&self, pause: Arc<DoqStopPause>) {
        let mut slot = self
            .stop_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(pause);
        }
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
    /// failures. A response stream that ends without a normal STREAM FIN -
    /// aborted by the peer or lost with the connection - is rejected with
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
            #[cfg(debug_assertions)]
            stop_pause: self
                .stop_pause
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
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
    /// Debug-only test observation seam consumed by this exchange. Release
    /// builds have no such field.
    #[cfg(debug_assertions)]
    stop_pause: Option<Arc<DoqStopPause>>,
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

    /// Applies the shared post-`open_bi` cancellation to one phase result.
    ///
    /// Once `open_bi` has succeeded a `RecvStream` exists, so a local control
    /// decision (owner close, caller cancellation, or the shared absolute
    /// deadline) that wins the outbound write, the request-side `finish`, or the
    /// response read calls `RecvStream::stop` with RFC 9250 §4.3
    /// `DOQ_REQUEST_CANCELLED` before the original typed control error is
    /// returned. Putting the check here keeps those three phases from bypassing
    /// it.
    ///
    /// The `stop` return value is deliberately discarded: it only reports
    /// whether the stream was already closed, and a `ClosedStream` must never
    /// replace the original typed control error or change its `SideEffectState`.
    /// The call is best-effort local cancellation only. Quinn 0.11.7 exposes no
    /// awaitable `STOP_SENDING` flush or peer-acknowledgement future, so this
    /// method does not - and must not - wait for the peer to observe the stop:
    /// no `yield_now`, no sleep, no short timeout, no polling loop, and no
    /// connection/endpoint close plus `wait_idle` stand-in for a barrier. The
    /// control error is returned immediately, and no `commit_final_response`
    /// gate is bypassed.
    ///
    /// The error is returned unchanged, preserving its original
    /// `SideEffectState`: no string conversion, no new generic receive error,
    /// and no protocol or ordinary I/O failure is masked by a stop.
    async fn settle_after_open<T>(
        &self,
        recv: &mut quinn::RecvStream,
        result: Result<T, SecureError>,
    ) -> Result<T, SecureError> {
        if let Err(error) = &result {
            if is_local_control_error(error) {
                let _ = recv.stop(quinn::VarInt::from_u32(DOQ_REQUEST_CANCELLED));
                #[cfg(debug_assertions)]
                if let Some(pause) = self.stop_pause.as_deref() {
                    // Debug-only test observation seam: park the exchange so
                    // the independent loopback test can watch the real Quinn
                    // driver transmit the stop before it releases this
                    // exchange. It is not part of the production cancellation
                    // contract.
                    pause.pause().await;
                }
            }
        }
        result
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
    // conservatively `MaybeSent`. Once `open_bi` has succeeded, a local control
    // decision must stop the receive side even though no request byte may have
    // been written yet.
    let mut outbound = query.to_vec();
    zero_outbound_query_id(&mut outbound);
    let written = race_control(&control, SideEffectState::MaybeSent, deadline, async {
        write_frame(&mut send, &outbound)
            .await
            .map_err(SecureError::from)
    })
    .await;
    prepared.settle_after_open(&mut recv, written).await?;

    // Request-side STREAM FIN: no more request bytes will be written. This is
    // another post-`open_bi` phase, so a local control decision here stops the
    // receive side too.
    let finished = race_control(&control, SideEffectState::MaybeSent, deadline, async {
        send.finish()
            .map_err(|_| SecureError::from(UpstreamError::Send(SideEffectState::MaybeSent)))
    })
    .await;
    prepared.settle_after_open(&mut recv, finished).await?;

    // Phase 4: read the one response up to the peer response-side STREAM FIN.
    // `read_to_end` only returns `Ok` once the FIN is observed, so its success
    // is the FIN evidence; a peer reset or a lost connection aborts the read
    // instead and `classify_read_error` turns either into the typed missing-FIN
    // protocol error. The request frame was fully written, so a control error
    // while waiting is `Sent`.
    let read = race_control(&control, SideEffectState::Sent, deadline, async {
        recv.read_to_end(MAX_DOQ_MESSAGE)
            .await
            .map_err(classify_doq_read_error)
    })
    .await;
    let complete = prepared.settle_after_open(&mut recv, read).await?;

    let (body, truncated) = validate_doq_payload(&complete, request_id)?;

    // Phase 5: the single control-aware commit. Priority is owner, then caller,
    // then the original absolute deadline, then success; a committed response
    // can never be reversed by a later close, cancellation, or deadline.
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;

    // One-shot teardown: close the connection and the endpoint so no socket or
    // connection survives the exchange. No pooling, no idle set, no reuse. This
    // runs only after the response was committed, so it is not - and must not
    // be used as - a cancellation-flush barrier for `STOP_SENDING`.
    connection.close(0u32.into(), b"");
    endpoint.close(0u32.into(), b"");
    endpoint.wait_idle().await;

    Ok(SecureResponse::doq(body, request_id, truncated))
}

/// Whether a secure error is one of the three local control outcomes that can
/// win a post-`open_bi` race: owner close, caller cancellation, or the shared
/// absolute deadline.
///
/// The match is structural, so no peer-supplied value is parsed and no protocol
/// or ordinary I/O failure is ever classified as a local cancellation.
fn is_local_control_error(error: &SecureError) -> bool {
    matches!(
        error,
        SecureError::Transport(
            UpstreamError::Closed(_)
                | UpstreamError::Cancelled(_)
                | UpstreamError::DeadlineExceeded(_)
        )
    )
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
/// STREAM FIN, and a lost connection proves the response stream/connection
/// terminated before that FIN, so both are the typed
/// [`SecureError::DoqProtocolMissingResponseFin`] protocol error; every other
/// read failure is a terminal receive failure with the request already sent.
/// The outcomes are matched structurally, so no peer-supplied error code or
/// connection-close text is ever parsed.
///
/// For the reset case the RFC 9250 §4.3 application error code does not change
/// the typed outcome: the violation is the missing response-side FIN, which is
/// required for completion regardless of why the peer aborted. `NO_ERROR`
/// (`0x0`) is therefore *not* benign here - it is only benign once a response
/// has completed, and a completed response arrives as a FIN, not a reset. The
/// same holds for `INTERNAL_ERROR` (`0x1`), `PROTOCOL_ERROR` (`0x2`), and
/// `REQUEST_CANCELLED` (`0x3`): all are terminal and never committed. The
/// `Slice3` suite exercises each code on the wire.
pub(crate) fn classify_doq_read_error(error: quinn::ReadToEndError) -> SecureError {
    match error {
        quinn::ReadToEndError::TooLong => SecureError::from(UpstreamError::FrameTooLarge),
        quinn::ReadToEndError::Read(
            quinn::ReadError::Reset(_) | quinn::ReadError::ConnectionLost(_),
        ) => SecureError::DoqProtocolMissingResponseFin,
        quinn::ReadToEndError::Read(_) => {
            SecureError::from(UpstreamError::Receive(SideEffectState::Sent))
        }
    }
}

/// Validates one complete DoQ response stream and restores its caller ID.
///
/// Both the one-shot and shared-connection DoQ paths use this helper so the
/// RFC 9250 framing, zero wire-ID, DNS validation, and original-ID contract
/// cannot drift between transports.
pub(crate) fn validate_doq_payload(
    complete: &[u8],
    request_id: u16,
) -> Result<(Vec<u8>, bool), SecureError> {
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
    if complete.len() != 2 + length {
        return Err(SecureError::DoqProtocolTrailingResponse);
    }
    let mut body = framed_body.to_vec();
    if body.len() < 2 {
        return Err(UpstreamError::MalformedResponse.into());
    }
    if u16::from_be_bytes([body[0], body[1]]) != 0 {
        return Err(SecureError::DoqProtocolNonzeroResponseId);
    }
    body[0..2].copy_from_slice(&request_id.to_be_bytes());
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    if header.id != request_id {
        return Err(UpstreamError::ResponseMismatch.into());
    }
    if validate_response(&body).is_err() {
        return Err(UpstreamError::MalformedResponse.into());
    }
    Ok((body, header.truncated))
}

// ---------------------------------------------------------------------------
// Slice 2: one-shot DNS-over-HTTP/3
// ---------------------------------------------------------------------------

/// The byte buffer type the h3/h3-quinn stack is driven with.
///
/// `hyper::body::Bytes` is a re-export of the exact `bytes::Bytes` the resolved
/// h3/h3-quinn graph uses, so naming it here pins h3's body parameter without a
/// manifest entry: `bytes` is not a direct dependency of this crate, this slice
/// introduces no new dependency, and the resolved graph already holds exactly
/// one `bytes` 1.x.
pub(crate) type H3Body = hyper::body::Bytes;

/// The h3 client connection driver this exchange polls as a tracked child.
pub(crate) type H3Driver = h3::client::Connection<h3_quinn::Connection, H3Body>;
/// The h3 request sender that owns the one `GET`.
pub(crate) type H3Sender = h3::client::SendRequest<h3_quinn::OpenStreams, H3Body>;
/// The one h3 request stream the response is read from.
pub(crate) type H3Stream = h3::client::RequestStream<h3_quinn::BidiStream<H3Body>, H3Body>;

/// A pure Rust one-shot DNS-over-HTTP/3 owner.
///
/// The owner reuses the same [`Lifecycle`] admission/drain gate as the plain,
/// DoT, and DoH transports: registration is serialized with `Open -> Closing`,
/// and [`Self::close`] refuses new exchanges and returns only after every
/// registered exchange has released its guard.
///
/// Every exchange opens exactly one fresh QUIC connection on the caller's
/// runtime, authenticates the endpoint's service identity with a
/// [`TlsPolicy`]-derived configuration offering exactly `h3`, performs exactly
/// one HTTPS `GET` by the existing `DohEndpoint` encoder, and closes both the
/// QUIC connection and the client endpoint before returning. Nothing is pooled,
/// reused, retried, or replayed, and no path falls back to DoH, HTTP/2, or
/// HTTP/1.1.
pub struct Doh3Upstream {
    endpoint: DohEndpoint,
    tls: TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
}

impl Doh3Upstream {
    /// Creates a DoH3 owner from a validated endpoint and an explicit policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] when the policy cannot produce a
    /// usable QUIC-compatible client configuration. The endpoint was validated
    /// when it was constructed, so no socket, resolver, or handshake work
    /// happens here.
    pub fn new(endpoint: DohEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        // Rebuild the exact ALPN-bearing configuration once at construction so
        // an unusable policy is rejected before the first exchange.
        tls.client_config_with_alpn(&[H3_ALPN])?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
        })
    }

    /// The validated DoH endpoint whose authority and encoder this owner reuses.
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

    /// Performs one bounded, fresh-connection DoH3 exchange.
    ///
    /// The exchange registers as in-flight under the same gate that serializes
    /// `Open -> Closing`, so close can never observe a zero registration count
    /// while this exchange is admitting itself. The RAII guard is held until
    /// this future returns or is dropped, covering success, every terminal
    /// error, cancellation/deadline, owner close, and an aborted future. The
    /// tracked h3 driver additionally holds a shared registration, so an aborted
    /// caller cannot release the owner while the driver still exists.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::DohRequest`] for a pre-I/O request defect,
    /// [`SecureError::Tls`] when the QUIC/TLS handshake fails (always
    /// `NotSent`), [`SecureError::DohProtocol`] for an unacceptable HTTP reply,
    /// and [`SecureError::Transport`] wrapping the exact typed
    /// [`UpstreamError`] for connect, control, send, and DNS-response failures.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        // The request target is built before any socket work, so an unframeable
        // query or an over-long target fails as a pre-I/O `NotSent` defect. The
        // encoder is reused verbatim: there is no second `dns` parameter codec.
        let target = self.endpoint.get_request_target(request)?;
        let authority = self.endpoint.authority();
        let prepared = self.prepare_exchange(request, context, target, authority)?;
        Box::pin(exchange_doh3(&prepared)).await
    }

    /// Registers and validates one exchange before any socket action.
    fn prepare_exchange<'a>(
        &'a self,
        request: ExchangeRequest<'a>,
        context: ExchangeContext,
        target: String,
        authority: String,
    ) -> Result<PreparedDoh3<'a>, SecureError> {
        let in_flight = self.lifecycle.register()?;
        context.check_at(Instant::now(), SideEffectState::NotSent)?;
        Ok(PreparedDoh3 {
            endpoint: &self.endpoint,
            tls: &self.tls,
            lifecycle: Arc::clone(&self.lifecycle),
            request,
            target,
            authority,
            deadline: context.deadline(),
            caller_cancellation: context.cancellation(),
            owner_cancellation: self.cancellation.clone(),
            _in_flight: in_flight,
        })
    }
}

/// Validated DoH3 exchange inputs held until the transport primitive runs.
///
/// The struct owns the in-flight registration guard, so dropping a prepared
/// exchange, including through an aborted caller future, releases the
/// registration without an explicit cleanup step.
struct PreparedDoh3<'a> {
    endpoint: &'a DohEndpoint,
    tls: &'a TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    request: ExchangeRequest<'a>,
    target: String,
    authority: String,
    deadline: Instant,
    caller_cancellation: TransportCancellation,
    owner_cancellation: TransportCancellation,
    /// Held only for its RAII release; never read.
    _in_flight: crate::InFlightGuard<'a>,
}

impl PreparedDoh3<'_> {
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

/// A validated response that is not committed yet.
///
/// The candidate is only committed after the tracked h3 driver scope has sealed
/// and drained, mirroring the HTTP/2 final-commit rule.
struct ValidatedDoh3Response {
    wire: Vec<u8>,
    request_id: u16,
    truncated: bool,
}

/// Runs one fresh, authenticated DoH3 exchange for a prepared request.
///
/// The single absolute deadline established by the caller covers every phase:
/// the numeric client-socket bind, QUIC connect and TLS handshake, the h3
/// connection build, the request and its send-side FIN, the response head and
/// body, and the final commit. No phase starts a fresh relative timer.
async fn exchange_doh3(prepared: &PreparedDoh3<'_>) -> Result<SecureResponse, SecureError> {
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let control = prepared.control();
    let dial = prepared.endpoint.dial();
    let identity = prepared.endpoint.identity();
    let deadline = prepared.deadline;

    // The client configuration is rebuilt from the frozen policy for every
    // exchange, with ALPN exactly `h3`, so no mutable per-owner state can flip a
    // verified policy into an insecure one or offer another protocol.
    let rustls_config = prepared.tls.client_config_with_alpn(&[H3_ALPN])?;
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
    // the first HTTP/DNS application byte, so a failure here is a typed TLS
    // error with `NotSent` state. No path retries or downgrades verification.
    let connecting = endpoint
        .connect(dial, identity.as_str())
        .map_err(classify_connect_error)?;
    let connection = race_control(&control, SideEffectState::NotSent, deadline, async {
        connecting.await.map_err(classify_handshake_failure)
    })
    .await?;

    // The handshake succeeded, so the peer is authenticated. No request byte has
    // been sent yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // The tracked driver scope is created before any request byte exists. The
    // shared registration keeps the owner non-drained for as long as the driver
    // child lives, so an aborted caller cannot release the exchange while the h3
    // driver still runs.
    let liveness = Arc::new(prepared.lifecycle.register_shared()?);
    let scope = H2ScopeLease::new(
        liveness,
        prepared.owner_cancellation.clone(),
        prepared.caller_cancellation.clone(),
    );

    // Phase 3: build the h3 connection on the caller's runtime. This opens the
    // control and QPACK streams but sends no DNS request byte, so a failure is
    // a typed connect failure with `NotSent`.
    let h3_connection = h3_quinn::Connection::new(connection.clone());
    let mut builder = h3::client::builder();
    builder.max_field_section_size(u64::try_from(MAX_RESPONSE_HEADER_BYTES).unwrap_or(u64::MAX));
    let (driver, mut send_request) =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            builder
                .build::<_, _, H3Body>(h3_connection)
                .await
                .map_err(classify_h3_setup_error)
        })
        .await?;

    // The driver must be polled continuously, so it is registered as a tracked
    // child before the first request byte is sent. Teardown seals admission,
    // aborts it, and drains to guard drop; it is never detached.
    scope.spawn(async move {
        let _ = drive_h3_connection(driver).await;
    });

    let candidate = match run_h3_request(prepared, &control, deadline, &mut send_request).await {
        Ok(candidate) => candidate,
        Err(error) => {
            scope.finish().await;
            return Err(error);
        }
    };

    // Seal admission, abort the tracked driver, and drain until its guard drops.
    // Only then is the response allowed to commit, so cancellation or owner
    // close during teardown cannot become a late success.
    scope.finish().await;
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;

    // One-shot teardown of the QUIC connection and client endpoint, after the
    // commit. No pooling, no idle set, no reuse. This is not - and must not be
    // used as - a cancellation-flush barrier: a control decision always returns
    // through the typed error path above without reaching this code.
    connection.close(0u32.into(), b"");
    endpoint.close(0u32.into(), b"");
    endpoint.wait_idle().await;

    Ok(SecureResponse::doh3(
        candidate.wire,
        candidate.request_id,
        candidate.truncated,
    ))
}

/// Drives one h3 client connection to completion as a tracked child.
///
/// The h3 connection owns the control and QPACK stream state, so it must be
/// polled for the request and response to make progress. It is spawned through
/// the exchange scope, which owns and drains it; production never starts a
/// detached task and never hides a runtime.
pub(crate) async fn drive_h3_connection(driver: H3Driver) -> h3::error::ConnectionError {
    let mut driver = driver;
    poll_fn(|context| driver.poll_close(context)).await
}

/// Sends the one `GET` and returns the validated, not-yet-committed response.
///
/// The `send_request` and request-FIN phases are conservatively `MaybeSent`, so a
/// failure there never claims the request reached the peer. Once the FIN has been
/// accepted, every later phase - the response head, the body, and the trailers -
/// is `Sent`, because the request was fully written and finished. A local control
/// error from any post-send phase is returned unchanged; the cancellation
/// boundary note that follows this function records why no `H3_REQUEST_CANCELLED`
/// stop frame is sent.
async fn run_h3_request(
    prepared: &PreparedDoh3<'_>,
    control: &ExchangeControl,
    deadline: Instant,
    send_request: &mut H3Sender,
) -> Result<ValidatedDoh3Response, SecureError> {
    // Exactly one GET. `:authority` is the service authority and `:path` is the
    // reused encoder output, so the numeric dial can never leak into either.
    let request = build_h3_get_request(&prepared.target, &prepared.authority)?;

    let mut stream = race_control(control, SideEffectState::MaybeSent, deadline, async {
        send_request
            .send_request(request)
            .await
            .map_err(classify_h3_send_error)
    })
    .await?;

    // Request send-side FIN: no request body follows the single GET.
    race_control(control, SideEffectState::MaybeSent, deadline, async {
        stream.finish().await.map_err(classify_h3_send_error)
    })
    .await?;

    // The request head was written and its send side finished, so any failure
    // while waiting for the response is `Sent`. A local control decision returns
    // its typed cause unchanged; see the cancellation boundary note below for
    // why no `H3_REQUEST_CANCELLED` stop frame is attempted.
    let response = race_control(control, SideEffectState::Sent, deadline, async {
        stream.recv_response().await.map_err(classify_h3_head_error)
    })
    .await?;

    let declared = validate_h3_response_head(response.status(), response.headers())?;
    let request_id = prepared.request.request_id();

    // The body is accumulated incrementally under the DNS bound; the declared
    // length, when present, must also match exactly, because the h3 transport
    // performs no `content-length` framing check of its own.
    let body = read_h3_body(control, deadline, &mut stream, declared).await?;
    if body.len() < 12 {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    let restored = restore_request_id(&body, request_id)?;
    if validate_response(&restored).is_err() {
        return Err(SecureError::from(UpstreamError::MalformedResponse));
    }
    Ok(ValidatedDoh3Response {
        wire: restored,
        request_id,
        truncated: header.truncated,
    })
}

// Local-cancellation boundary (Slice 3).
//
// `design.md` §4/§6 asks for an active `H3_REQUEST_CANCELLED` on the receive
// side when a local control decision wins. That is **not safely reachable
// through the pinned h3 0.0.8 / h3-quinn 0.0.10 API** and is therefore
// deliberately not attempted here, with direct evidence:
//
//   * `h3::client::RequestStream::stop_sending` delegates to
//     `h3_quinn::RecvStream::stop_sending`, which does
//     `self.stream.as_mut().unwrap().stop(..)`
//     (`h3-quinn-0.0.10/src/lib.rs:390-397`).
//   * `h3_quinn::RecvStream::poll_data` *takes* that `Option` into the in-flight
//     `read_chunk_fut` and only restores it after the read future completes
//     (`h3-quinn-0.0.10/src/lib.rs:375-387`).
//   * A local control decision wins by dropping that read future, so `stream`
//     is left `None` and an immediate `stop_sending` panics with
//     `called Option::unwrap() on a None value`.
//
// The cancellation is therefore expressed by the unchanged typed control error
// (owner `Closed`, caller `Cancelled`, or `DeadlineExceeded`, each with its own
// `SideEffectState`) plus the connection/endpoint teardown that follows it, and
// a local decision is never disguised as a peer error. The `Slice3` suite
// asserts exactly that outcome for owner close, caller cancellation, and the
// deadline, including after a complete response body has already arrived.

/// Builds the single `GET` this exchange may send.
///
/// The request target is the origin-form target produced by the endpoint, so the
/// numeric dial override cannot leak into it. The authority is carried in the
/// URI, which makes h3 emit exactly one `:authority` pseudo-header; no `Host`
/// header is added alongside it.
pub(crate) fn build_h3_get_request(
    target: &str,
    authority: &str,
) -> Result<hyper::Request<()>, SecureError> {
    let uri = hyper::Uri::builder()
        .scheme("https")
        .authority(authority)
        .path_and_query(target)
        .build()
        .map_err(|_| SecureError::DohRequest(DohRequestError::TargetTooLarge))?;
    hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(uri)
        // The DNS media type is the only acceptable response type.
        .header(hyper::header::ACCEPT, DNS_MEDIA_TYPE)
        .body(())
        .map_err(|_| SecureError::DohRequest(DohRequestError::TargetTooLarge))
}

/// Validates the h3 response head and returns its declared body length.
///
/// The status, head byte bound, declared length, content encoding, and media
/// type checks are the shared DoH contract. The header-count bound is enforced
/// here because, unlike the Hyper HTTP/1.1 and HTTP/2 parsers, the h3 client
/// layer imposes no header-count limit of its own.
pub(crate) fn validate_h3_response_head(
    status: hyper::StatusCode,
    headers: &hyper::HeaderMap,
) -> Result<Option<u64>, SecureError> {
    if headers.len() > MAX_RESPONSE_HEADERS {
        return Err(SecureError::DohProtocol(
            DohProtocolError::ResponseHeadTooLarge,
        ));
    }
    let head_bytes = parsed_head_bytes(hyper::Version::HTTP_3, status, headers);
    validate_doh_head_parts(status, headers, head_bytes)
}

/// Reads the response body to its end, bounded by the DNS maximum.
///
/// The bound is enforced on the bytes actually received rather than only on a
/// declared length, so a peer cannot evade it by omitting or understating the
/// header. An early end of stream, a body that disagrees with `content-length`,
/// or an empty body is `IncompleteBody`, never a silently accepted prefix.
///
/// `recv_data` returning `Ok(None)` is *not* by itself proof that the response
/// completed: in h3 0.0.8 it also returns `None` as soon as a trailing HEADERS
/// frame has been buffered as response trailers. Completion therefore requires
/// one `recv_trailers` step, which waits for the real stream end, validates any
/// trailing field section, and surfaces a reset or an illegal frame after
/// trailers as an error. Only `Ok(None)` - no trailers at all - completes the
/// body. Any trailers are [`DohProtocolError::IncompleteBody`], matching the
/// HTTP/1.1 and HTTP/2 paths, which never silently accept a response that keeps
/// going past the body they consumed. That step runs inside the same
/// control/deadline race as the body loop, so a local control decision or the
/// absolute deadline still wins over it.
pub(crate) async fn read_h3_body(
    control: &ExchangeControl,
    deadline: Instant,
    stream: &mut H3Stream,
    declared: Option<u64>,
) -> Result<Vec<u8>, SecureError> {
    let collected = race_control(control, SideEffectState::Sent, deadline, async {
        let mut collected: Vec<u8> = Vec::new();
        loop {
            let Some(frame) = stream.recv_data().await.map_err(classify_h3_body_error)? else {
                break;
            };
            let data = frame.chunk();
            if collected.len().saturating_add(data.len()) > MAX_DNS_BODY {
                return Err(SecureError::DohProtocol(DohProtocolError::BodyTooLarge));
            }
            collected.extend_from_slice(data);
        }
        match stream
            .recv_trailers()
            .await
            .map_err(classify_h3_body_error)?
        {
            None => Ok::<Vec<u8>, SecureError>(collected),
            Some(_) => Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody)),
        }
    })
    .await?;

    if collected.is_empty() {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    if let Some(declared) = declared {
        if u64::try_from(collected.len()) != Ok(declared) {
            return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
        }
    }
    Ok(collected)
}

/// Classifies an h3 connection-build failure.
///
/// The build opens only the h3 control and QPACK streams; no DNS request byte
/// exists yet, so the failure is a typed connect failure with `NotSent`.
fn classify_h3_setup_error(_error: h3::error::ConnectionError) -> SecureError {
    SecureError::from(UpstreamError::Connect)
}

/// Classifies an h3 request-send or request-finish failure.
///
/// The request may have been partially written, so the failure is conservatively
/// a typed send failure with `MaybeSent`. A peer `STOP_SENDING` on the request
/// send side surfaces here as [`h3::error::StreamError::RemoteTerminate`]; it is
/// deliberately kept as a send failure rather than a response-stream
/// termination, because the request write is the side that is still in doubt and
/// `MaybeSent` is the truthful state. No branch here retries or replays.
pub(crate) fn classify_h3_send_error(_error: h3::error::StreamError) -> SecureError {
    SecureError::from(UpstreamError::Send(SideEffectState::MaybeSent))
}

/// Classifies an h3 response-head failure.
///
/// A field section above the advertised 16 KiB bound is the head-size violation
/// the DoH contract names. An explicit peer stream termination is mapped
/// structurally to the closed [`PeerStreamError`] category, so no raw code,
/// reason, or payload crosses the boundary.
///
/// Every other head-phase outcome - the h3 stream or connection ending, or an h3
/// message error - is observed only after this exchange wrote the request head
/// and its `stream.finish()` succeeded, so `design.md` §7 classifies it as a
/// completed write followed by a failed read: a typed receive failure with
/// [`SideEffectState::Sent`]. It is deliberately *not*
/// [`DohProtocolError::ResponseHeadNotReceived`], whose conservative `MaybeSent`
/// state exists for the HTTP/1.1 and HTTP/2 drivers, where the request hand-off
/// may still be in doubt.
pub(crate) fn classify_h3_head_error(error: h3::error::StreamError) -> SecureError {
    match error {
        h3::error::StreamError::HeaderTooBig { .. } => {
            SecureError::DohProtocol(DohProtocolError::ResponseHeadTooLarge)
        }
        h3::error::StreamError::RemoteTerminate { code, .. } => {
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated {
                code: classify_peer_stream_code(code),
            })
        }
        _ => SecureError::Transport(UpstreamError::Receive(SideEffectState::Sent)),
    }
}

/// Classifies an h3 body-read failure.
///
/// The response head was already observed, so the request was transmitted and
/// the state is `Sent`. An explicit peer stream termination keeps its structured
/// code category; every other read failure - a reset through the connection, a
/// connection loss, or a truncated data frame - proves the body did not
/// complete, which is the typed incomplete-body protocol error.
pub(crate) fn classify_h3_body_error(error: h3::error::StreamError) -> SecureError {
    match error {
        h3::error::StreamError::RemoteTerminate { code, .. } => {
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated {
                code: classify_peer_stream_code(code),
            })
        }
        _ => SecureError::DohProtocol(DohProtocolError::IncompleteBody),
    }
}

/// Maps an HTTP/3 peer stream-termination code to the closed
/// [`PeerStreamError`] category.
///
/// The match is on the numeric `h3::error::Code` value, never on a formatted
/// reason, so a peer cannot influence the classification through `Display`. The
/// h3 layer passes the raw QUIC stream error code through unchanged, and a
/// `RemoteTerminate` here can only arrive on the DoH3 *request/response*
/// stream, so the classification is made in that context and separates three
/// cases:
///
/// * The four RFC 9114 §8.1 HTTP/3 codes this client reviews -
///   `H3_NO_ERROR` (`0x100`), `H3_GENERAL_PROTOCOL_ERROR` (`0x101`),
///   `H3_INTERNAL_ERROR` (`0x102`), and `H3_REQUEST_CANCELLED` (`0x10c`) - map
///   to their matching categories.
/// * A defined HTTP/3 or QPACK code whose meaning applies to this
///   request/response stream is a termination this four-category review does
///   not name, so it is reported as the unclassified [`PeerStreamError::Other`]
///   rather than mislabelled. The eligible codes are listed one by one in
///   [`is_request_response_stream_h3_code`]; that covers the request/response
///   codes of RFC 9114 §8.1 (for example `H3_FRAME_UNEXPECTED`, `0x105`) and
///   RFC 9204's `QPACK_DECOMPRESSION_FAILED` (`0x200`), which §6 defines for a
///   failed field-section decode on exactly this request/response stream.
/// * Everything else is an unknown code or a code whose definition is scoped to
///   another context, and RFC 9114 §8 requires *both* to be treated as
///   equivalent to `H3_NO_ERROR`. That deliberately includes RFC 9114 §8.1
///   codes that are defined only for the control stream, a critical stream,
///   connection-level bookkeeping, the creation of a *new* stream, or a
///   `CONNECT` tunnel - `H3_CLOSED_CRITICAL_STREAM` (`0x104`),
///   `H3_ID_ERROR` (`0x108`), `H3_SETTINGS_ERROR` (`0x109`),
///   `H3_MISSING_SETTINGS` (`0x10a`), `H3_STREAM_CREATION_ERROR` (`0x103`), and
///   `H3_CONNECT_ERROR` (`0x10f`) - as well as the whole RFC 9000 §20.1
///   transport code space below `0x100` (the DoQ `0x0`-`0x3` values), the
///   reserved `0x1f * N + 0x21` grease space, and RFC 9204's
///   `QPACK_ENCODER_STREAM_ERROR` (`0x201`) and `QPACK_DECODER_STREAM_ERROR`
///   (`0x202`), whose definitions are scoped to the QPACK encoder and decoder
///   streams. Each maps to [`PeerStreamError::NoError`] and is never
///   reinterpreted as an H3 internal, protocol, or cancellation error.
///
/// A termination never commits regardless of category: completion requires the
/// response to end with an h3 stream FIN, which a reset or `STOP_SENDING` is
/// not. The raw code and any reason text are intentionally discarded.
fn classify_peer_stream_code(code: h3::error::Code) -> PeerStreamError {
    match code.value() {
        0x100 => PeerStreamError::NoError,
        0x101 => PeerStreamError::ProtocolError,
        0x102 => PeerStreamError::InternalError,
        0x10c => PeerStreamError::RequestCancelled,
        value if is_request_response_stream_h3_code(value) => PeerStreamError::Other,
        _ => PeerStreamError::NoError,
    }
}

/// Whether `value` is a defined HTTP/3 or QPACK code whose RFC 9114 §8.1 /
/// RFC 9204 §6 meaning applies to the DoH3 *request/response* stream, so a
/// `RemoteTerminate` carrying it is a *known* error reported as the unclassified
/// [`PeerStreamError::Other`] rather than folded into RFC 9114 §8's
/// unknown/unexpected `H3_NO_ERROR` equivalence.
///
/// This is an explicit per-code allowlist, deliberately **not** a numeric range.
/// RFC 9114 §8: "use of an error code in an unexpected context or receipt of an
/// unknown error code MUST be treated as equivalent to H3_NO_ERROR." Every
/// `0x103..=0x110` code therefore has to be judged against the concrete context
/// of this path - one plain HTTPS `GET` over a client-initiated bidirectional
/// request/response stream - not merely against the numeric block it sits in:
///
/// * `H3_CLOSED_CRITICAL_STREAM` (`0x104`) is the closure of a control or QPACK
///   critical stream (RFC 9114 §6.2.1).
/// * `H3_ID_ERROR` (`0x108`) is connection-level stream-ID/push-ID misuse
///   (RFC 9114 §4.6, §6.2.2, §7.2.5-§7.2.7).
/// * `H3_SETTINGS_ERROR` (`0x109`) and `H3_MISSING_SETTINGS` (`0x10a`) are
///   errors of the SETTINGS frame, which exists only on the control stream
///   (RFC 9114 §6.2.1, §7.2.4).
/// * `H3_STREAM_CREATION_ERROR` (`0x103`) is the endpoint detecting that its
///   peer "created a stream that it will not accept" (RFC 9114 §8.1) - a *new*
///   stream the endpoint refuses, not the termination of an already-accepted
///   request/response stream. Resetting the existing `GET` response stream with
///   it is an error code in an unexpected context.
/// * `H3_CONNECT_ERROR` (`0x10f`) is defined for "the TCP connection established
///   in response to a CONNECT request" (RFC 9114 §4.4/§8.1). This path sends a
///   plain `GET` and never `CONNECT`, so the code is likewise out of context.
///
/// The four codes matched to their own [`PeerStreamError`] categories before
/// this predicate are not repeated. The remaining eligible codes are the
/// request/response errors RFC 9114 §8.1 defines for a request or response
/// exchange - `0x105`, `0x106`, `0x107`, `0x10b`, `0x10d`, `0x10e`, and `0x110`,
/// each describing a condition of the current exchange (a frame not permitted on
/// the current stream, a frame layout/size violation, peer load, request
/// rejection, an incomplete request stream, a malformed HTTP message, and the
/// requested operation not being servable over HTTP/3) - plus RFC 9204 §6's
/// `QPACK_DECOMPRESSION_FAILED` (`0x200`), defined for a failed field-section
/// decode on a request stream.
const fn is_request_response_stream_h3_code(value: u64) -> bool {
    matches!(
        value,
        0x105 // H3_FRAME_UNEXPECTED
            | 0x106 // H3_FRAME_ERROR
            | 0x107 // H3_EXCESSIVE_LOAD
            | 0x10b // H3_REQUEST_REJECTED
            | 0x10d // H3_REQUEST_INCOMPLETE
            | 0x10e // H3_MESSAGE_ERROR
            | 0x110 // H3_VERSION_FALLBACK
            | 0x200 // QPACK_DECOMPRESSION_FAILED (RFC 9204 §6, request stream)
    )
}
