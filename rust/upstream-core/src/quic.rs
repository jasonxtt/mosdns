//! Slice 0 QUIC endpoint construction, ALPN singletons, and DoQ byte-shape
//! helpers (Phase 4 QUIC task).
//!
//! This module is pre-I/O only in Slice 0: endpoint validation, exact ALPN
//! offers, and the outbound ID-zeroing byte shape. No socket, no QUIC
//! handshake, no stream I/O, and no H3 driver live here yet — those arrive in
//! Slices 1-2. The length-prefix encode itself is not reimplemented: callers
//! use the frozen `dns-core` Stream framing helper, the same one
//! [`crate::tcp::encode_frame`] uses.

use std::net::SocketAddr;

use crate::{SecureError, ServerIdentity};

/// The exact ALPN offer for DNS-over-QUIC (RFC 9250 §3).
pub const DOQ_ALPN: &[u8] = b"doq";
/// The exact ALPN offer for DNS-over-HTTP/3 (RFC 9114).
pub const H3_ALPN: &[u8] = b"h3";

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
