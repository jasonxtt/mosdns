//! Typed construction failures for secure upstream endpoints.

use std::error::Error;
use std::fmt;

use crate::SideEffectState;

/// Why a service identity was rejected by [`ServerIdentity::new`].
///
/// [`ServerIdentity::new`]: super::ServerIdentity::new
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    /// The identity string was empty.
    Empty,
    /// The identity is neither a valid DNS name nor an IP literal.
    Malformed,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "empty",
            Self::Malformed => "malformed",
        })
    }
}

impl Error for IdentityError {}

/// Why a DoH service URL was rejected by [`DohEndpoint::new`].
///
/// [`DohEndpoint::new`]: super::DohEndpoint::new
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceUrlError {
    /// The URL is not syntactically valid.
    Malformed,
    /// The URL does not use the `https` scheme.
    UnsupportedScheme,
    /// The URL has no host.
    EmptyHost,
    /// The URL embeds userinfo (credentials).
    UserInfo,
    /// The URL contains a fragment.
    Fragment,
}

impl fmt::Display for ServiceUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed",
            Self::UnsupportedScheme => "unsupported scheme",
            Self::EmptyHost => "empty host",
            Self::UserInfo => "userinfo is not allowed",
            Self::Fragment => "fragment is not allowed",
        })
    }
}

impl Error for ServiceUrlError {}

/// Why a TLS policy was rejected by [`TlsPolicy::verified`].
///
/// [`TlsPolicy::verified`]: super::TlsPolicy::verified
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsConfigError {
    /// A verified policy was constructed with no trust anchors.
    EmptyRootStore,
}

impl fmt::Display for TlsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRootStore => "empty verified TLS root store",
        })
    }
}

impl Error for TlsConfigError {}

/// A typed failure raised while constructing a secure endpoint.
///
/// Every variant is a pre-I/O construction defect, so the DNS side-effect state
/// is always [`SideEffectState::NotSent`] and construction never opens a socket
/// or resolves an identity. The variant payloads are closed enums rather than
/// URL text, so their `Display` output never echoes credentials, a service URL,
/// or query contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureError {
    /// A numeric dial address with port zero was supplied.
    ZeroDialPort,
    /// The TLS service identity was rejected.
    InvalidIdentity(IdentityError),
    /// The DoH service URL was rejected.
    InvalidServiceUrl(ServiceUrlError),
    /// The explicit TLS policy was rejected before any I/O.
    TlsConfig(TlsConfigError),
}

impl SecureError {
    /// Secure endpoint construction performs no network I/O, so the query
    /// side-effect state is always [`SideEffectState::NotSent`].
    #[must_use]
    pub const fn side_effect(self) -> SideEffectState {
        SideEffectState::NotSent
    }
}

impl fmt::Display for SecureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDialPort => formatter.write_str("secure endpoint dial port is zero"),
            Self::InvalidIdentity(reason) => {
                write!(formatter, "invalid secure service identity: {reason}")
            }
            Self::InvalidServiceUrl(reason) => {
                write!(formatter, "invalid DoH service URL: {reason}")
            }
            Self::TlsConfig(reason) => write!(formatter, "invalid TLS policy: {reason}"),
        }
    }
}

impl Error for SecureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ZeroDialPort => None,
            Self::InvalidIdentity(reason) => Some(reason),
            Self::InvalidServiceUrl(reason) => Some(reason),
            Self::TlsConfig(reason) => Some(reason),
        }
    }
}
