//! Secure upstream endpoint construction (Phase 4 Slice0).
//!
//! This module currently covers only pre-I/O construction: validating a
//! DNS-name or IP service identity, keeping the numeric dial destination
//! separate from that identity, freezing an explicit TLS trust policy, and
//! reporting typed construction errors. TLS sockets/handshakes, DoT/DoH I/O,
//! request encoding, and lifecycle ownership are deliberately absent from this
//! slice.

mod endpoint;
mod error;
mod tls;

pub use endpoint::{DohEndpoint, DotEndpoint, ServerIdentity};
pub use error::{IdentityError, SecureError, ServiceUrlError, TlsConfigError};
pub use tls::TlsPolicy;
