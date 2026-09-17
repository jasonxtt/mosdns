//! The bounded bootstrap UDP exchange.
//!
//! One leader binds a fresh ephemeral socket in the bootstrap endpoint's
//! address family, connects it to the numeric peer, sends an encoded query
//! immediately, and then retransmits the same query at the policy interval
//! until one correlated reply arrives or the caller's own absolute deadline,
//! caller cancellation, or owner shutdown wins.
//!
//! The exchange runs entirely on the caller's runtime: no runtime, executor,
//! thread, timer, system resolver, TCP fallback, or hidden background task is
//! created. No private timeout is invented and no deadline is extended.
//!
//! Only the configured peer can deliver a reply, because the socket is
//! connected. Datagrams with a wrong peer or a wrong transaction ID are ignored
//! while the caller's budget remains; a correlated terminal DNS failure is
//! reported deterministically.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;

use mosdns_dns_core::{QueryIdSource, build_resolver_query, parse_resolver_response};
use tokio::net::UdpSocket;

use super::{AddressFamily, BootstrapEndpoint, ResolutionPolicy, ResolverError};
use crate::{ExchangeControl, SideEffectState, UpstreamError};

/// The full legal datagram capacity, never Go's silent 4095-byte buffer.
const RECV_BUFFER_BYTES: usize = 65_535;

/// A per-exchange unpredictable DNS transaction ID source.
///
/// The slice deliberately keeps the source injectable: production can supply a
/// cryptographically unpredictable implementation without changing this
/// module, and tests can pin an ID without touching the exchange.
pub trait ResolutionIdSource: Send + Sync {
    /// Draws the transaction ID for the next bootstrap query.
    ///
    /// A source that cannot produce an unpredictable ID returns a typed error
    /// rather than substituting a guess, so a failed draw can never be mistaken
    /// for a valid correlation value.
    fn next_id(&self) -> Result<u16, ResolverError>;

    /// Whether this source's IDs are unpredictable.
    ///
    /// The production construction path refuses to use a source that reports
    /// `false`, so a predictable source can never be selected by accident.
    fn is_unpredictable(&self) -> bool;
}

/// The production ID source: unpredictable transaction IDs from the operating
/// system.
///
/// A DNS transaction ID is part of the anti-spoofing correlation set (RFC 5452),
/// so a bootstrap query must not use a guessable sequence. This is the only
/// source the default construction path may select.
#[derive(Debug, Default)]
pub struct OsIdSource;

impl OsIdSource {
    /// Whether this host can actually supply unpredictable bytes.
    ///
    /// One probe draw is taken here, so a host without entropy fails at
    /// construction rather than degrading silently for the resolver's lifetime.
    #[must_use]
    pub fn is_available(&self) -> bool {
        let mut bytes = [0u8; 2];
        getrandom::fill(&mut bytes).is_ok()
    }
}

impl ResolutionIdSource for OsIdSource {
    fn next_id(&self) -> Result<u16, ResolverError> {
        // A failed draw is surfaced, never papered over. Substituting a value
        // derived from clock or hash state would silently weaken the RFC 5452
        // correlation this ID exists to provide, so the exchange instead fails
        // closed with a typed error.
        let mut bytes = [0u8; 2];
        match getrandom::fill(&mut bytes) {
            Ok(()) => Ok(u16::from_ne_bytes(bytes)),
            Err(_) => Err(ResolverError::UnpredictableIdsUnavailable),
        }
    }

    fn is_unpredictable(&self) -> bool {
        true
    }
}

/// An ID source that never draws, used only to prove the failure contract.
///
/// It exists so the exchange's typed handling of an undrawable source is
/// testable without an entropy-free host.
#[derive(Debug, Default)]
pub struct FailingIdSource;

impl ResolutionIdSource for FailingIdSource {
    fn next_id(&self) -> Result<u16, ResolverError> {
        Err(ResolverError::UnpredictableIdsUnavailable)
    }

    fn is_unpredictable(&self) -> bool {
        false
    }
}

/// A monotonically stepping, non-cryptographic ID source.
///
/// This source is **not** unpredictable and exists only so tests can pin IDs
/// without touching the exchange. It is reachable only through
/// [`super::BootstrapResolver::with_deterministic_ids_for_tests`]; the
/// production construction path never selects it.
#[derive(Debug, Default)]
pub struct SteppingIdSource(AtomicU16);

impl SteppingIdSource {
    /// Creates a stepping source starting at one.
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU16::new(1))
    }
}

impl ResolutionIdSource for SteppingIdSource {
    fn next_id(&self) -> Result<u16, ResolverError> {
        Ok(self.0.fetch_add(1, Ordering::Relaxed))
    }

    fn is_unpredictable(&self) -> bool {
        false
    }
}

/// Bridges a single already-drawn identifier into the wire encoder's borrowed
/// [`QueryIdSource`] interface.
///
/// The encoder only needs `&mut impl QueryIdSource`, while the draw itself is
/// fallible and belongs to the resolver. The exchange therefore resolves the ID
/// first — failing closed on an undrawable source — and hands the encoder this
/// one-shot carrier, so a query is never built around a guessed value.
struct IdSourceAdapter {
    drawn: Option<u16>,
}

impl QueryIdSource for IdSourceAdapter {
    fn next_id(&mut self) -> u16 {
        // Reached only with the ID the exchange already accepted; the encoder
        // uses it exactly once.
        self.drawn
            .take()
            .expect("the exchange draws exactly one id before encoding")
    }
}

/// The typed outcome of one bootstrap exchange, before publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapAnswer {
    pub(crate) address: std::net::IpAddr,
    pub(crate) ttl_secs: u32,
}

/// Maps a transport control failure onto the resolver's typed vocabulary.
fn control_error(error: UpstreamError) -> ResolverError {
    match error {
        UpstreamError::Closed(_) => ResolverError::Closed,
        UpstreamError::Cancelled(_) => ResolverError::Cancelled,
        UpstreamError::DeadlineExceeded(_) => ResolverError::BootstrapTimeout,
        _ => ResolverError::BootstrapReceive,
    }
}

/// Maps a DNS wire failure onto the resolver's typed vocabulary, preserving the
/// terminal rcode when the reply was correlated but negative and the truncation
/// observation when the reply carried TC.
///
/// A correlated TC=1 reply is a legitimate DNS observation, not a malformed
/// message, so it keeps its own typed error instead of collapsing into
/// [`ResolverError::MalformedBootstrapResponse`]. This foundation performs no
/// TCP bootstrap fallback, so that error is terminal.
fn wire_error(error: mosdns_dns_core::ResolverWireError) -> ResolverError {
    use mosdns_dns_core::ResolverWireError as Wire;
    match error {
        Wire::Rcode(code) => ResolverError::BootstrapRcode(code),
        Wire::NoUsableAnswer => ResolverError::NoUsableAddress,
        Wire::Truncated => ResolverError::Truncated,
        _ => ResolverError::MalformedBootstrapResponse,
    }
}

/// Runs the bounded bootstrap exchange for one already-validated tuple.
///
/// # Errors
///
/// Every failure is a typed [`ResolverError`]; no partial or stale answer is
/// produced, and no retry extends the caller's deadline.
pub(crate) async fn exchange(
    target_host: &str,
    family: AddressFamily,
    bootstrap: BootstrapEndpoint,
    policy: &ResolutionPolicy,
    control: &ExchangeControl,
    ids: &dyn ResolutionIdSource,
) -> Result<BootstrapAnswer, ResolverError> {
    let peer = bootstrap.address();
    let deadline = control.context().deadline();

    // Before bind: owner close, caller cancellation, then the deadline.
    control
        .check_at(Instant::now(), SideEffectState::NotSent)
        .map_err(control_error)?;

    let bind_address: SocketAddr = match peer {
        SocketAddr::V4(_) => SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)),
    };
    let socket = UdpSocket::bind(bind_address)
        .await
        .map_err(|_| ResolverError::BootstrapConnect)?;
    // Connecting the socket makes the filter part of the socket itself, so only
    // the configured numeric peer can deliver a datagram.
    socket
        .connect(peer)
        .await
        .map_err(|_| ResolverError::BootstrapConnect)?;

    control
        .check_at(Instant::now(), SideEffectState::NotSent)
        .map_err(control_error)?;

    // Draw the transaction ID first: if the source cannot supply an
    // unpredictable value, the exchange fails closed before encoding anything.
    let request_id = ids.next_id()?;
    // Encode one query and retransmit the identical bytes; the ID is drawn once,
    // so a retransmission can never be mistaken for a new question.
    let mut source = IdSourceAdapter {
        drawn: Some(request_id),
    };
    let query = build_resolver_query(target_host, family, &mut source)
        .map_err(|_| ResolverError::InvalidHostname)?;
    let qname_wire = question_name(&query).ok_or(ResolverError::MalformedBootstrapResponse)?;
    debug_assert_eq!(u16::from_be_bytes([query[0], query[1]]), request_id);
    // The owner's own bounds must govern the wire parse, not dns-core's default
    // policy: dns-core clamps the effective TTL it reports, so parsing under its
    // defaults would silently discard a caller's custom floor or ceiling.
    let cname_policy = policy.dns_core_policy()?;
    let owner = control.owner_cancellation();
    let caller = control.caller_cancellation();
    let owner_cancelled = owner.cancelled();
    let caller_cancelled = caller.cancelled();
    tokio::pin!(owner_cancelled);
    tokio::pin!(caller_cancelled);

    // One absolute deadline, created once from the caller's own context. No
    // later phase resets or extends it.
    let timer = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
    tokio::pin!(timer);

    let mut buffer = vec![0u8; RECV_BUFFER_BYTES];
    let mut sent = false;
    let mut next_send = Instant::now();

    loop {
        let now = Instant::now();
        let side_effect = if sent {
            SideEffectState::Sent
        } else {
            SideEffectState::NotSent
        };
        control.check_at(now, side_effect).map_err(control_error)?;

        // Send immediately on entry and again at each policy interval, racing
        // the control tokens and the single absolute deadline.
        let wait_until_send = next_send.saturating_duration_since(now);
        let send_timer = tokio::time::sleep(wait_until_send);
        tokio::pin!(send_timer);

        let received = if sent {
            tokio::select! {
                biased;
                () = &mut owner_cancelled => return Err(ResolverError::Closed),
                () = &mut caller_cancelled => return Err(ResolverError::Cancelled),
                () = &mut timer => return Err(ResolverError::BootstrapTimeout),
                result = socket.recv(&mut buffer) => Some(result),
                () = &mut send_timer => None,
            }
        } else {
            tokio::select! {
                biased;
                () = &mut owner_cancelled => return Err(ResolverError::Closed),
                () = &mut caller_cancelled => return Err(ResolverError::Cancelled),
                () = &mut timer => return Err(ResolverError::BootstrapTimeout),
                () = &mut send_timer => None,
            }
        };

        match received {
            // The retransmit interval elapsed: resend the identical query.
            None => {
                let written = tokio::select! {
                    biased;
                    () = &mut owner_cancelled => return Err(ResolverError::Closed),
                    () = &mut caller_cancelled => return Err(ResolverError::Cancelled),
                    () = &mut timer => return Err(ResolverError::BootstrapTimeout),
                    result = socket.send(&query) => result,
                }
                .map_err(|_| ResolverError::BootstrapSend)?;
                if written != query.len() {
                    return Err(ResolverError::BootstrapSend);
                }
                sent = true;
                next_send = Instant::now() + policy.retransmit_interval();
            }
            Some(Err(_)) => return Err(ResolverError::BootstrapReceive),
            Some(Ok(length)) => {
                let datagram = &buffer[..length];
                match parse_resolver_response(
                    datagram,
                    family,
                    &qname_wire,
                    request_id,
                    &cname_policy,
                ) {
                    Ok(selected) => {
                        return Ok(BootstrapAnswer {
                            address: selected.address,
                            ttl_secs: selected.ttl,
                        });
                    }
                    // A datagram from the configured peer that is not a
                    // correlated reply is ignored while budget remains: the
                    // socket is connected, so this can only be the peer's own
                    // traffic and a later valid reply must still be able to win.
                    Err(mosdns_dns_core::ResolverWireError::MismatchedId)
                    | Err(mosdns_dns_core::ResolverWireError::QuestionMismatch)
                    | Err(mosdns_dns_core::ResolverWireError::NotAResponse)
                    | Err(mosdns_dns_core::ResolverWireError::UnexpectedOpcode(_)) => continue,
                    Err(error) => return Err(wire_error(error)),
                }
            }
        }
    }
}

/// Copies the question name out of an encoded query in uncompressed wire form.
fn question_name(query: &[u8]) -> Option<Vec<u8>> {
    let parsed = mosdns_dns_core::parse_query(query).ok()?;
    Some(parsed.1.qname_wire)
}

#[cfg(test)]
mod tests {
    use super::{ResolutionIdSource, SteppingIdSource};

    #[test]
    fn stepping_id_source_advances() {
        let source = SteppingIdSource::new();
        assert_eq!(source.next_id(), Ok(1));
        assert_eq!(source.next_id(), Ok(2));
        assert_eq!(source.next_id(), Ok(3));
    }
}
