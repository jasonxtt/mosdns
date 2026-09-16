//! Secure upstream endpoint construction and one-exchange DoT transport
//! (Phase 4 Slices 0-1).
//!
//! This module covers pre-I/O construction and encoding plus the bounded
//! DNS-over-TLS one-exchange primitive:
//!
//! * validating a DNS-name or IP service identity, keeping the numeric dial
//!   destination separate from that identity, freezing an explicit TLS trust
//!   policy, and building a pure DoH GET origin-form request target;
//! * [`DotUpstream`], which dials one fresh numeric connection, authenticates
//!   it with TLS before any DNS byte, and then performs exactly one framed
//!   query/response exchange.
//!
//! DoH I/O, Hyper drivers, connection pooling/reuse, resolver/bootstrap, and
//! listener or host composition remain outside this slice.

mod dot;
mod endpoint;
mod error;
mod tls;

pub use dot::{DotUpstream, SecureResponse, SecureTransport};
pub use endpoint::{DohEndpoint, DotEndpoint, ServerIdentity};
pub use error::{
    CertificateRejection, DohRequestError, IdentityError, SecureError, ServiceUrlError,
    TlsConfigError, TlsHandshakeFailure,
};
pub use tls::TlsPolicy;
