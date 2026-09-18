//! Secure upstream endpoint construction and the bounded secure transports
//! (Phase 4 Slices 0-3).
//!
//! This module covers pre-I/O construction and encoding plus two one-exchange
//! secure primitives:
//!
//! * validating a DNS-name or IP service identity, keeping the numeric dial
//!   destination separate from that identity, freezing an explicit TLS trust
//!   policy, and building a pure DoH GET origin-form request target;
//! * [`DotUpstream`], which dials one fresh numeric connection, authenticates
//!   it with TLS before any DNS byte, and then performs exactly one framed
//!   query/response exchange;
//! * [`DohUpstream`], which dials one fresh numeric connection, authenticates
//!   it with TLS against the service URL identity, and then performs exactly one
//!   HTTPS `GET` over HTTP/1.1 or HTTP/2 with scoped child-task ownership.
//!
//! Connection pooling/reuse, resolver/bootstrap, HTTP/3, and listener or host
//! composition remain outside this slice.

mod doh;
mod dot;
mod endpoint;
mod error;
mod tls;

pub use doh::DohUpstream;
pub use dot::{DotUpstream, SecureHttpVersion, SecureResponse, SecureTransport};
pub use endpoint::{DohEndpoint, DotEndpoint, ServerIdentity};
pub use error::{
    CertificateRejection, DohProtocolError, DohRequestError, IdentityError, SecureError,
    ServiceUrlError, TlsConfigError, TlsHandshakeFailure,
};
pub use tls::TlsPolicy;

#[cfg(test)]
pub(crate) use doh::H2TeardownPause;
/// Crate-internal reuse surface for the secure transports.
///
/// The pooled-session types live beside the protocol code that owns their
/// private state, so connection reuse reuses the existing framing, control race,
/// and HTTP/2 child tracking instead of duplicating a second state machine.
/// Nothing here is re-exported from the crate root.
pub(crate) use doh::{H2DrainHandle, PooledDohSession};
pub(crate) use dot::PooledDotSession;
