//! Slice 3 UDP TC-to-TCP composite policy.
//!
//! [`UdpTcpPolicy`] sits above the reviewed UDP and TCP primitives. It never
//! reimplements framing, validation, socket ownership, lifecycle, or
//! cancellation: each leg is exactly one [`Upstream::exchange`] on the
//! caller's runtime, and the TCP owner targets the same numeric `SocketAddr`
//! as the UDP owner.
//!
//! The policy performs the minimum reviewed protocol fallback:
//!
//! 1. one UDP exchange;
//! 2. a complete UDP response is returned unchanged with no TCP work;
//! 3. a validated TC=1 UDP observation is not a final answer, so the caller
//!    context is checked at the current instant before any TCP work;
//! 4. exactly one fresh TCP exchange then receives the same borrowed query,
//!    the same original DNS ID, and the same cloned absolute deadline.
//!
//! A malformed or undersized UDP error is returned unchanged and is never
//! reclassified as a fallback trigger. There is no retransmission, retry,
//! pooling, reuse, pipelining, Go re-entry, or hidden runtime.

use std::time::Instant;

use crate::{
    Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse, SideEffectState,
    TcpFallbackContext, Transport, Upstream, UpstreamError,
};

/// Reviewed UDP-first composite policy with at most one fresh TCP fallback.
///
/// The two legs are independent single-transport owners so each keeps the
/// existing lifecycle, registration, and cancellation behavior. The policy
/// holds no socket, timer, or runtime of its own.
pub struct UdpTcpPolicy {
    udp: Upstream,
    tcp: Upstream,
}

impl UdpTcpPolicy {
    /// Creates the composite policy over the reviewed numeric UDP endpoint.
    ///
    /// The TCP fallback reuses the exact same `SocketAddr` with
    /// [`Transport::Tcp`], so a caller cannot supply a second, divergent
    /// fallback target. The address was already validated when the UDP
    /// endpoint was constructed.
    #[must_use]
    pub fn new(udp_endpoint: Endpoint) -> Self {
        debug_assert_eq!(udp_endpoint.transport(), Transport::Udp);
        let tcp_endpoint = Endpoint::new(udp_endpoint.address(), Transport::Tcp)
            .expect("a validated numeric UDP endpoint yields a numeric TCP endpoint");
        Self {
            udp: Upstream::new(udp_endpoint),
            tcp: Upstream::new(tcp_endpoint),
        }
    }

    /// Runs one UDP-first exchange with at most one TCP fallback.
    ///
    /// The request is validated once by the UDP owner and the same borrowed
    /// request value is passed to the TCP owner, so the original query wire
    /// and DNS ID reach TCP byte-for-byte and are never revalidated. Both legs
    /// receive a clone of the same [`ExchangeContext`], so the absolute
    /// deadline instant and cancellation token are shared rather than reset.
    ///
    /// # Errors
    ///
    /// A UDP failure is returned unchanged: an undersized or otherwise
    /// malformed datagram is terminal and never triggers TCP. After a TC
    /// observation, the caller context is checked at the current instant with
    /// the overall [`SideEffectState::Sent`] state; an already-effective
    /// cancellation or deadline is returned as its typed error without
    /// entering TCP. Otherwise the single TCP leg runs exactly once, and its
    /// typed failure is returned as [`UpstreamError::TcpFallback`] carrying the
    /// prior [`TcpFallbackContext`] and the original nested cause.
    pub async fn exchange<'q>(
        &self,
        request: ExchangeRequest<'q>,
        context: ExchangeContext,
    ) -> Result<ExchangeResponse, UpstreamError> {
        // The UDP leg validates the request once and owns one registration.
        // The response is a complete answer or a TC observation.
        let udp_response = self.udp.exchange(request, context.clone()).await?;
        if !udp_response.truncated() {
            return Ok(udp_response);
        }

        // Retain the structured prior TC observation before the response is
        // consumed and before the single TCP attempt can produce a typed
        // failure. It records only the original request ID, the matching UDP
        // response ID, the TC flag, and the overall prior Sent side effect.
        let prior = TcpFallbackContext::new(
            udp_response.request_id(),
            udp_response.response_id(),
            udp_response.truncated(),
            SideEffectState::Sent,
        );

        // A TC observation is not the final answer. Before any TCP work, the
        // already-existing caller context is re-checked at the current instant
        // with the overall Sent state; cancellation wins a tie with the
        // deadline. This check starts no timer and resets no deadline.
        context.check_at(Instant::now(), SideEffectState::Sent)?;

        // Exactly one fresh TCP exchange with the same borrowed request and the
        // same context clone (same absolute deadline, same cancellation token).
        // A failure keeps the original typed TCP cause nested under the prior
        // TC context; it is never stringified, relabelled, or retried.
        match self.tcp.exchange(request, context).await {
            Ok(response) => Ok(response),
            Err(cause) => Err(UpstreamError::TcpFallback {
                prior,
                cause: Box::new(cause),
            }),
        }
    }
}
