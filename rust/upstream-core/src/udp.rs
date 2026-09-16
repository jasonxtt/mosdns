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

use crate::{
    ExchangeResponse, IgnoredDatagrams, PreparedExchange, SideEffectState, TerminalError,
    Transport, UpstreamError,
};

/// The largest possible DNS wire size. Slice1 receives with this full legal
/// datagram capacity and must never inherit Go's 4095-byte implementation
/// buffer, which silently truncated legal UDP responses.
const RECV_BUFFER_BYTES: usize = 65_535;

/// Classifies an ordinary local socket bind/setup failure.
///
/// Local setup has not sent a DNS payload, so it maps to the settled `NotSent`
/// [`UpstreamError::Connect`] category. This tiny seam keeps the mapping
/// unit-testable without injecting a socket factory.
fn bind_error(_error: &std::io::Error) -> UpstreamError {
    UpstreamError::Connect
}

/// Attaches retained ignored-datagram diagnostics to a terminal transport
/// error.
///
/// The primary cause is preserved verbatim and never relabelled; when nothing
/// was ignored the plain error is returned unchanged.
fn diagnosed(error: UpstreamError, ignored: IgnoredDatagrams) -> UpstreamError {
    if !ignored.any() {
        return error;
    }
    let cause = match error {
        UpstreamError::DeadlineExceeded(state) => TerminalError::DeadlineExceeded(state),
        UpstreamError::Cancelled(state) => TerminalError::Cancelled(state),
        UpstreamError::Closed(state) => TerminalError::Closed(state),
        UpstreamError::Receive(state) => TerminalError::Receive(state),
        other => return other,
    };
    UpstreamError::Diagnosed { cause, ignored }
}

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
    // A local bind/setup failure is a `Connect` runtime failure that has sent
    // nothing; `Connect.side_effect()` is already `NotSent`.
    let socket = UdpSocket::bind(bind_address)
        .await
        .map_err(|error| bind_error(&error))?;

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
    // Ignored datagrams are retained only as diagnostics on the terminal error;
    // they never replace the primary cause or the side-effect state.
    let mut ignored = IgnoredDatagrams::NONE;
    loop {
        // After every wake, re-apply owner close, caller cancellation, and the
        // shared absolute deadline before touching the socket again.
        prepared
            .check_at(Instant::now(), SideEffectState::Sent)
            .map_err(|error| diagnosed(error, ignored))?;

        let received = tokio::select! {
            biased;
            () = &mut owner_cancelled => {
                return Err(diagnosed(UpstreamError::Closed(SideEffectState::Sent), ignored))
            }
            () = &mut caller_cancelled => {
                return Err(diagnosed(UpstreamError::Cancelled(SideEffectState::Sent), ignored))
            }
            () = &mut deadline => {
                return Err(diagnosed(
                    UpstreamError::DeadlineExceeded(SideEffectState::Sent),
                    ignored,
                ))
            }
            result = socket.recv_from(&mut buffer) => result,
        };
        let (length, peer) = received
            .map_err(|_| diagnosed(UpstreamError::Receive(SideEffectState::Sent), ignored))?;

        // Only the configured numeric peer address (including port) may supply
        // the response; other datagrams are ignored while the exchange lives.
        if peer != endpoint {
            ignored.record_unexpected_peer();
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
            ignored.record_response_id_mismatch();
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

#[cfg(test)]
mod tests {
    use super::{SideEffectState, UpstreamError, bind_error};

    #[test]
    fn local_bind_failure_maps_to_connect_without_side_effects() {
        let error = bind_error(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "bind denied",
        ));
        assert_eq!(error, UpstreamError::Connect);
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
    }
}
