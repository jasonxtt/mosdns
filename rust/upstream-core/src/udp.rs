//! Slice 1 one-exchange/one-socket UDP primitive.
//!
//! Each call binds one fresh ephemeral [`UdpSocket`] in the configured
//! endpoint's address family, sends the caller's borrowed query exactly once
//! without rewriting its DNS ID, and receives until a validated response, a
//! terminal cancellation/deadline, or a socket failure. The socket is owned by
//! the exchange and dropped on every terminal path.
//!
//! Slice1 deliberately does not share a socket, demultiplex responses, resend,
//! pool, reuse, pipeline, fall back to TCP, or touch any protocol beyond plain
//! UDP. A valid truncated (TC=1) header is returned as an owned observation for
//! the later composite policy; this module never starts TCP.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Instant;

use mosdns_dns_core::{inspect_response_header, validate_response};
use tokio::net::UdpSocket;

use crate::{ExchangeResponse, PreparedExchange, SideEffectState, Transport, UpstreamError};

/// The largest possible DNS wire size. Slice1 receives with this full legal
/// datagram capacity and must never inherit Go's 4095-byte implementation
/// buffer, which silently truncated legal UDP responses.
const RECV_BUFFER_BYTES: usize = 65_535;

/// Runs the reviewed one-exchange/one-socket UDP primitive for a prepared
/// exchange.
///
/// The caller has already validated the request, transport, and owner
/// lifecycle through [`crate::Upstream::prepare_exchange`]. Every async wake
/// re-applies the owner-close, caller-cancellation, and absolute-deadline
/// precedence from [`crate::ExchangeControl::check_at`]: owner close is
/// `Closed`, caller cancellation is `Cancelled`, and the deadline is
/// `DeadlineExceeded`, with cancellation winning a tie.
pub(crate) async fn exchange(
    prepared: &PreparedExchange<'_>,
) -> Result<ExchangeResponse, UpstreamError> {
    debug_assert_eq!(prepared.endpoint().transport(), Transport::Udp);

    // Before bind: owner close, caller cancellation, then absolute deadline.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let endpoint = prepared.endpoint().address();
    let bind_address = match endpoint {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };
    // A local bind/setup failure is a runtime failure that has sent nothing.
    let socket = UdpSocket::bind(bind_address)
        .await
        .map_err(|_| UpstreamError::Runtime(SideEffectState::NotSent))?;

    // Bind is an async wake: re-apply the full control before any send.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let owner = prepared.owner_cancellation();
    let caller = prepared.context().cancellation();
    let owner_cancelled = owner.cancelled();
    let caller_cancelled = caller.cancelled();
    tokio::pin!(owner_cancelled);
    tokio::pin!(caller_cancelled);

    // One absolute deadline, created once and reused by every select below.
    let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(
        prepared.context().deadline(),
    ));
    tokio::pin!(deadline);

    let request_id = prepared.request().request_id();
    let query = prepared.request().query();

    // Before send: re-apply the control, then send the unchanged query exactly
    // once, racing owner close, caller cancellation, and the deadline.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;
    let sent = tokio::select! {
        biased;
        () = &mut owner_cancelled => Err(UpstreamError::Closed(SideEffectState::NotSent)),
        () = &mut caller_cancelled => Err(UpstreamError::Cancelled(SideEffectState::NotSent)),
        () = &mut deadline => Err(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
        result = socket.send_to(query, endpoint) => {
            result.map_err(|_| UpstreamError::Send(SideEffectState::MaybeSent))
        }
    }?;
    // Only a complete datagram send is `Sent`; a short send is uncertain.
    if sent != query.len() {
        return Err(UpstreamError::Send(SideEffectState::MaybeSent));
    }

    let mut buffer = vec![0u8; RECV_BUFFER_BYTES];
    loop {
        // After every wake, re-apply owner close, caller cancellation, and the
        // shared absolute deadline before touching the socket again.
        prepared.check_at(Instant::now(), SideEffectState::Sent)?;

        let received = tokio::select! {
            biased;
            () = &mut owner_cancelled => return Err(UpstreamError::Closed(SideEffectState::Sent)),
            () = &mut caller_cancelled => return Err(UpstreamError::Cancelled(SideEffectState::Sent)),
            () = &mut deadline => {
                return Err(UpstreamError::DeadlineExceeded(SideEffectState::Sent))
            }
            result = socket.recv_from(&mut buffer) => result,
        };
        let (length, peer) = received.map_err(|_| UpstreamError::Receive(SideEffectState::Sent))?;

        // Only the configured numeric peer address (including port) may supply
        // the response; other datagrams are ignored while the exchange lives.
        if peer != endpoint {
            continue;
        }

        let datagram = &buffer[..length];
        let header = match inspect_response_header(datagram) {
            Ok(header) => header,
            // A QR-clear or too-short expected-peer packet cannot be attributed
            // to this exchange as a response and is terminally malformed.
            Err(_) => return Err(UpstreamError::MalformedResponse),
        };
        // A wrong-ID datagram from the expected peer is ignored until a
        // matching response arrives or the exchange terminates.
        if header.id != request_id {
            continue;
        }
        // A valid matching TC header is a successful UDP observation owned by
        // the response. Slice1 does not fall back to TCP; the composite Slice3
        // policy decides that from this owned truncated observation.
        if header.truncated {
            return Ok(ExchangeResponse::new(
                datagram.to_vec(),
                request_id,
                header.id,
                Transport::Udp,
                true,
            ));
        }
        // Only a complete non-TC wire that passes dns-core validation may be
        // returned as the accepted response.
        if validate_response(datagram).is_err() {
            return Err(UpstreamError::MalformedResponse);
        }
        return Ok(ExchangeResponse::new(
            datagram.to_vec(),
            request_id,
            header.id,
            Transport::Udp,
            false,
        ));
    }
}
