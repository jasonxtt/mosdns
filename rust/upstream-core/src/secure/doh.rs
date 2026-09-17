//! One-exchange DNS-over-HTTPS primitive over HTTP/1.1 (Phase 4 Slice2).
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
//! 2. Authenticated TLS handshake against the service URL host. Absent ALPN
//!    means HTTP/1.1 on the already-established stream; an unexpected ALPN is
//!    terminal.
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
//! This slice deliberately uses Hyper's **low-level** HTTP/1.1 connection API.
//! Hyper's `http1::Connection` is itself a `Future` that the exchange polls
//! inline, so no background task or executor owns any part of the exchange:
//! there is no detached driver to reap and no library retry to prevent.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use hyper::body::Incoming;
use hyper::client::conn::http1::SendRequest;
use hyper::header::{ACCEPT, CONTENT_ENCODING, CONTENT_TYPE, HOST};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use mosdns_dns_core::{inspect_response_header, patch_response_id_ra, validate_response};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::secure::dot::SecureResponse;
use crate::secure::endpoint::DohEndpoint;
use crate::secure::error::{DohProtocolError, SecureError};
use crate::secure::tls::{TlsPolicy, classify_handshake_error, server_name_for};
use crate::tcp::race_control;
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl,
    ExchangeRequest, Lifecycle, LifecycleState, SideEffectState, TransportCancellation,
    UpstreamError,
};

/// The `application/dns-message` media type, compared case-insensitively.
const DNS_MEDIA_TYPE: &str = "application/dns-message";

/// The largest DNS message a DoH response may carry: 65535 bytes.
const MAX_DNS_BODY: usize = 65_535;

/// The largest response header block this client will read.
///
/// The DNS contract needs only `Content-Type`, `Content-Encoding`,
/// `Content-Length` and a few framing headers, so a generous but fixed ceiling
/// keeps a hostile peer from making the client buffer an unbounded header set.
const MAX_RESPONSE_HEADERS: usize = 64;

/// The HTTP/1.1 read-buffer ceiling.
///
/// This bounds the header block the connection will assemble. It must stay
/// above the largest legal header block this contract accepts (16 KiB per the
/// design) while remaining far below the point where a peer could make the
/// client allocate unbounded memory; Hyper requires at least 8192.
const MAX_HTTP1_BUFFER: usize = 32 * 1024;

/// The ALPN protocol list offered for DoH, in preference order.
///
/// Only HTTP/1.1 is implemented in this slice. HTTP/2 is deliberately absent so
/// a peer cannot negotiate a protocol this client does not drive; advertising it
/// would risk a silent, unimplemented downgrade. Slice3 adds `h2` together with
/// the scoped HTTP/2 driver.
const DOH_ALPN: &[&[u8]] = &[b"http/1.1"];

/// A pure Rust DNS-over-HTTPS owner over HTTP/1.1.
///
/// The owner reuses the same [`Lifecycle`] admission/drain gate as the DoT and
/// plain transports: registration is serialized with `Open -> Closing`, and
/// [`Self::close`] refuses new exchanges and returns only after every
/// registered exchange has released its guard.
pub struct DohUpstream {
    endpoint: DohEndpoint,
    tls: TlsPolicy,
    lifecycle: Lifecycle,
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
            lifecycle: Lifecycle::new(),
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

    /// Performs one bounded authenticated DoH exchange over HTTP/1.1.
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
        exchange_inner(&prepared).await
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
            lifecycle: &self.lifecycle,
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
    lifecycle: &'a Lifecycle,
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
    // Only HTTP/1.1 is offered in this slice. Advertising `h2` here without a
    // scoped HTTP/2 driver would let a peer negotiate a protocol this client
    // cannot drive, which is exactly the silent fallback this design forbids.
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

    check_negotiated_alpn(&tls)?;

    // The handshake succeeded, so the peer is authenticated. No DNS byte has
    // been sent yet, so this check is still `NotSent`.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    // Build the one GET this exchange is allowed to send. There is no body, no
    // `User-Agent`, and no request `Content-Encoding`.
    let request = build_get_request(&prepared.target, &prepared.authority)?;

    prepared.reach(DohPhase::BeforeRequest).await;

    // Phase 3: one request, driven inline. `http1::Connection` is itself a
    // future, so the connection is polled by this exchange rather than by a
    // detached task; there is nothing to reap and no background retry to stop.
    let io = TokioIo::new(tls);
    let (mut sender, connection) =
        race_control(&control, SideEffectState::NotSent, deadline, async {
            hyper::client::conn::http1::Builder::new()
                .max_headers(MAX_RESPONSE_HEADERS)
                // Hyper requires at least 8192; this is well above the largest
                // legal header block this contract accepts.
                .max_buf_size(MAX_HTTP1_BUFFER)
                .handshake::<_, EmptyBody>(io)
                .await
                .map_err(|_| SecureError::from(UpstreamError::Connect))
        })
        .await?;

    tokio::pin!(connection);

    let response = send_request(&control, deadline, &mut sender, &mut connection, request).await?;

    // Phase 4: read the response body to a complete end under the same control.
    prepared.reach(DohPhase::BeforeBody).await;
    let body = read_body(&control, deadline, response, &mut connection).await?;

    // A body shorter than the DNS header cannot be a response at all.
    if body.len() < 12 {
        return Err(SecureError::DohProtocol(DohProtocolError::IncompleteBody));
    }
    let header = inspect_response_header(&body)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    // The DoH contract associates a response with its HTTP stream, not with a
    // DNS transaction ID, so a remote ID of 0 is normal. The caller's original
    // ID is restored into the owned wire before it is returned.
    let restored = patch_response_id_ra(&body, request_id)
        .map_err(|_| SecureError::from(UpstreamError::MalformedResponse))?;
    // Only a complete, dns-core-valid response may be returned. A DoH response
    // with TC set is still returned to the caller rather than retried.
    if validate_response(&restored).is_err() {
        return Err(SecureError::from(UpstreamError::MalformedResponse));
    }

    // Phase 5: the final control-aware commit is the single linearization point
    // against owner close, caller cancellation, and the original absolute
    // deadline. Priority is owner, then caller, then deadline, then success.
    //
    // Nothing may be asserted about owner state after this returns: the commit
    // wins under the lifecycle mutex and an owner close may legally begin
    // immediately afterwards.
    prepared.reach(DohPhase::BeforeCommit).await;
    prepared.lifecycle.commit_final_response(
        &prepared.caller_cancellation,
        deadline,
        SideEffectState::Sent,
    )?;
    prepared.reach(DohPhase::AfterCommit).await;

    Ok(SecureResponse::doh(restored, request_id, header.truncated))
}

/// Rejects a negotiated ALPN protocol other than HTTP/1.1.
///
/// Absent ALPN is accepted: the connection already carries HTTP/1.1, which is
/// what this slice implements. An explicitly negotiated protocol that is not
/// HTTP/1.1 is terminal rather than silently reinterpreted.
fn check_negotiated_alpn(tls: &TlsStream<TcpStream>) -> Result<(), SecureError> {
    let (_, session) = tls.get_ref();
    match session.alpn_protocol() {
        None => Ok(()),
        Some(protocol) if protocol == b"http/1.1" => Ok(()),
        // A negotiated protocol this client cannot drive is terminal; the
        // connection is abandoned rather than replayed with another protocol.
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

/// Sends the request and reads the response head.
///
/// The connection future is polled alongside the request, so the exchange owns
/// the whole HTTP/1.1 machine. The status and headers are validated here; the
/// body is read separately so its incremental size can be bounded.
async fn send_request(
    control: &ExchangeControl,
    deadline: Instant,
    sender: &mut SendRequest<EmptyBody>,
    connection: &mut Pin<&mut impl Future<Output = hyper::Result<()>>>,
    request: Request<EmptyBody>,
) -> Result<Response<Incoming>, SecureError> {
    // No control check happens between building and sending: the request has
    // been constructed but nothing is on the wire yet, so a control error here
    // is still `NotSent`.
    let response = race_control(control, SideEffectState::MaybeSent, deadline, async {
        // Poll the connection and the request together: the request future only
        // completes while the connection is being driven. Both outcomes are
        // terminal for this select, so it is not a loop.
        let request_future = sender.send_request(request);
        tokio::pin!(request_future);
        tokio::select! {
            result = &mut request_future => result.map_err(|_| {
                // A failure after the request was handed to the driver may
                // already have transmitted part of it.
                SecureError::Transport(UpstreamError::Send(SideEffectState::MaybeSent))
            }),
            result = connection.as_mut() => Err(match result {
                // The connection ended before a response arrived, so no
                // response headers were ever observed.
                Ok(()) => SecureError::DohProtocol(DohProtocolError::IncompleteBody),
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

/// Validates the status and the headers this contract depends on.
fn validate_response_head(response: &Response<Incoming>) -> Result<(), SecureError> {
    if response.status() != hyper::StatusCode::OK {
        return Err(SecureError::DohProtocol(
            DohProtocolError::UnexpectedStatus {
                status: response.status().as_u16(),
            },
        ));
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
async fn read_body(
    control: &ExchangeControl,
    deadline: Instant,
    response: Response<Incoming>,
    connection: &mut Pin<&mut impl Future<Output = hyper::Result<()>>>,
) -> Result<Vec<u8>, SecureError> {
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
struct EmptyBody;

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

    use super::{DohPhase, DohUpstream};
    use crate::secure::endpoint::DohEndpoint;
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
    const MATRIX_PHASES: [DohPhase; 5] = [
        DohPhase::BeforeConnect,
        DohPhase::BeforeHandshake,
        DohPhase::BeforeRequest,
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
                DohPhase::BeforeHandshake | DohPhase::BeforeRequest => {
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
        assert_eq!(seen.len(), 5, "connect, handshake, request, body, commit");
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

    /// The context used by the success-path tests.
    fn open_context() -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            TransportCancellation::new(),
        )
    }
}
