//! Secure upstream endpoint construction and the bounded secure transports
//! (Phase 4 Slices 0-2).
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
//!   HTTPS `GET` over HTTP/1.1.
//!
//! HTTP/2, connection pooling/reuse, resolver/bootstrap, and listener or host
//! composition remain outside this slice.

mod doh;
mod dot;
mod endpoint;
mod error;
mod tls;

pub use doh::DohUpstream;
pub use dot::{DotUpstream, SecureResponse, SecureTransport};
pub use endpoint::{DohEndpoint, DotEndpoint, ServerIdentity};
pub use error::{
    CertificateRejection, DohProtocolError, DohRequestError, IdentityError, SecureError,
    ServiceUrlError, TlsConfigError, TlsHandshakeFailure,
};
pub use tls::TlsPolicy;
