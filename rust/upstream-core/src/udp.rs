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
    ExchangeResponse, IgnoredDatagrams, PreparedExchange, ResponseCommit, SideEffectState,
    TerminalError, Transport, UpstreamError,
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
    commit: &ResponseCommit<'_>,
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
            // The TC response has passed every header/ID step; the commit gate
            // is the final decision before the owned observation is returned.
            commit.before_commit().await;
            commit.commit().map_err(|error| diagnosed(error, ignored))?;
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
        // Validation is complete; the commit gate is the single linearization
        // point against owner close, immediately before the owned response.
        commit.before_commit().await;
        commit.commit().map_err(|error| diagnosed(error, ignored))?;
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
    use std::future::Future;
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::task::{JoinHandle, spawn_blocking};
    use tokio::time::timeout;

    use super::bind_error;
    use crate::{
        CloseResult, CloseTransition, CommitPause, Endpoint, ExchangeContext, ExchangeRequest,
        LifecycleState, SideEffectState, Transport, TransportCancellation, Upstream, UpstreamError,
    };

    /// The largest legal IPv4 UDP payload, matching the production receive
    /// buffer and the Slice1 integration fixtures.
    const LEGAL_UDP_PAYLOAD: usize = 65_507;

    /// Bounds every exchange so a missing or broken socket path cannot hang CI.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

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
        wire.extend_from_slice(&[0x01, 0x00]); // RD=1, QR=0, opcode QUERY
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
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
        wire
    }

    /// A truncated (TC=1) response header that the TC path accepts without a
    /// complete verified body and without any TCP fallback.
    fn truncated_response_wire(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x83, 0x80]); // QR=1, TC=1, RD=1, RA=1
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire
    }

    fn bind_ipv4() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
        let address = socket.local_addr().expect("local address");
        (socket, address)
    }

    fn udp_endpoint(address: SocketAddr) -> Endpoint {
        Endpoint::new(address, Transport::Udp).expect("numeric udp endpoint")
    }

    fn open_context() -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            TransportCancellation::new(),
        )
    }

    /// Receives one datagram and sends `reply` back to its source.
    fn reply_once(socket: UdpSocket, reply: Vec<u8>) -> JoinHandle<()> {
        spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = socket.recv_from(&mut buffer).expect("receive query");
            socket.send_to(&reply, peer).expect("send reply");
        })
    }

    /// Installs the deterministic pre-commit pause seam and returns the test's
    /// handle to it.
    fn install_pause(upstream: &Upstream) -> Arc<CommitPause> {
        let pause = Arc::new(CommitPause::new());
        upstream.install_commit_pause(Arc::clone(&pause));
        pause
    }

    /// Spawns one exchange on the current-thread runtime.
    fn spawn_exchange(
        upstream: &Arc<Upstream>,
        query: Vec<u8>,
    ) -> tokio::task::JoinHandle<Result<crate::ExchangeResponse, UpstreamError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, open_context()).await
        })
    }

    #[test]
    fn owner_close_wins_when_it_reaches_the_commit_gate_first() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe001;
            let server_task = reply_once(server, response_wire(id, 40));

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            // The datagram is received and validated, but the response commit
            // has not run yet: the exchange is parked on the test seam.
            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the pre-commit gate");
            assert_eq!(upstream.in_flight_exchanges(), 1);

            // Owner close reaches the shared gate first.
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closing);

            // Releasing the seam lets the commit observe Closing and lose.
            pause.release();
            let outcome = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome.err().expect("owner close wins the commit gate"),
                UpstreamError::Closed(SideEffectState::Sent)
            );

            // The already-begun async close drains and reaches Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);

            server_task.await.expect("server task joined");
        });
    }

    #[test]
    fn owner_close_wins_the_commit_gate_for_a_truncated_response() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe002;
            let server_task = reply_once(server, truncated_response_wire(id));

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("TC exchange reaches the pre-commit gate");
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            pause.release();

            let outcome = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome
                    .err()
                    .expect("owner close wins the commit gate for a TC response"),
                UpstreamError::Closed(SideEffectState::Sent)
            );
            assert_eq!(upstream.close().await, CloseResult::Closed);

            server_task.await.expect("server task joined");
        });
    }

    #[test]
    fn response_commit_before_owner_close_is_not_reversed() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe003;
            let expected = response_wire(id, 41);
            let server_task = reply_once(server, expected.clone());

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the pre-commit gate");
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Open);

            // The commit reaches the gate while the owner is Open and wins.
            pause.release();
            let response = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined")
                .expect("the response commits while the owner is Open");
            assert_eq!(response.response_id(), id);
            assert_eq!(response.wire(), expected.as_slice());
            assert!(!response.truncated());

            // A close that starts afterwards cannot reverse the committed
            // response and still drains the released registration to Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);

            server_task.await.expect("server task joined");
        });
    }

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
