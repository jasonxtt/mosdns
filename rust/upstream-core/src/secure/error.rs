//! Typed construction failures for secure upstream endpoints.

use std::error::Error;
use std::fmt;

use crate::{SideEffectState, UpstreamError};

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
    /// The raw URL text contains a carriage return or line feed.
    ///
    /// The WHATWG URL parser strips raw ASCII CR/LF before parsing, so this is
    /// checked before parsing rather than left to become a silently accepted
    /// path, query, or host byte.
    ControlCharacter,
}

impl fmt::Display for ServiceUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed",
            Self::UnsupportedScheme => "unsupported scheme",
            Self::EmptyHost => "empty host",
            Self::UserInfo => "userinfo is not allowed",
            Self::Fragment => "fragment is not allowed",
            Self::ControlCharacter => "raw control character is not allowed",
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
    /// The selected crypto provider supports none of the safe default protocol
    /// versions.
    Provider,
}

impl fmt::Display for TlsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRootStore => "empty verified TLS root store",
            Self::Provider => "TLS crypto provider has no usable protocol version",
        })
    }
}

impl Error for TlsConfigError {}

/// Why a DoH GET request target was rejected by
/// [`DohEndpoint::get_request_target`].
///
/// Both variants are pre-I/O request defects; neither borrows or formats the
/// service URL, its query, or the encoded DNS message, so their `Display` and
/// `Debug` output cannot leak endpoint or query material.
///
/// [`DohEndpoint::get_request_target`]: super::DohEndpoint::get_request_target
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DohRequestError {
    /// The outbound DNS message is longer than the 65535-byte DNS limit.
    QueryTooLarge,
    /// The encoded origin-form request target is longer than 96 KiB.
    TargetTooLarge,
}

impl fmt::Display for DohRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::QueryTooLarge => "DNS query exceeds the 65535-byte limit",
            Self::TargetTooLarge => "request target exceeds the 96 KiB limit",
        })
    }
}

impl Error for DohRequestError {}

/// Why a presented certificate chain was rejected during a TLS handshake.
///
/// These mirror the verification outcomes the selected TLS engine can report.
/// No variant carries certificate bytes, a key, or a raw library error, so
/// `Display`/`Debug` can never leak key material or echo peer-supplied data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CertificateRejection {
    /// The certificate's validity window has already ended.
    Expired,
    /// The certificate's validity window has not started yet.
    NotValidYet,
    /// The certificate does not cover the requested service identity.
    NotValidForName,
    /// The chain does not terminate at any supplied trust anchor.
    UnknownIssuer,
    /// A signature in the chain does not verify against its issuer's key.
    BadSignature,
    /// The certificate could not be parsed.
    BadEncoding,
    /// Any other verification outcome.
    Other,
}

impl fmt::Display for CertificateRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Expired => "certificate expired",
            Self::NotValidYet => "certificate not yet valid",
            Self::NotValidForName => "certificate not valid for the service identity",
            Self::UnknownIssuer => "certificate issuer is not trusted",
            Self::BadSignature => "certificate signature is invalid",
            Self::BadEncoding => "certificate encoding is invalid",
            Self::Other => "certificate verification failed",
        })
    }
}

/// A typed TLS handshake failure observed before any DNS application byte.
///
/// Every variant is a TLS-protocol outcome, not a DNS one. Handshake traffic is
/// not a DNS send, so [`Self::side_effect`] is always
/// [`SideEffectState::NotSent`]: the DNS query for this exchange has provably
/// not been transmitted. No variant triggers a fallback to insecure
/// verification, a plaintext transport, or a retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsHandshakeFailure {
    /// The peer's certificate chain was rejected during verification.
    Certificate(CertificateRejection),
    /// The peer sent a fatal TLS alert.
    Alert,
    /// The peer's handshake data was malformed or unexpected.
    Protocol,
    /// The stream ended before the handshake completed.
    UnexpectedEof,
    /// Any other handshake outcome the selected engine can report.
    ///
    /// `rustls::Error` is `#[non_exhaustive]`, so this catches an outcome the
    /// reviewed classification does not enumerate rather than mislabelling it
    /// as a protocol or certificate failure.
    Other,
}

impl TlsHandshakeFailure {
    /// Handshake traffic alone is never a DNS application send.
    #[must_use]
    pub const fn side_effect(self) -> SideEffectState {
        SideEffectState::NotSent
    }
}

impl fmt::Display for TlsHandshakeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Certificate(reason) => write!(formatter, "TLS certificate {reason}"),
            Self::Alert => formatter.write_str("TLS alert"),
            Self::Protocol => formatter.write_str("TLS protocol failure"),
            Self::UnexpectedEof => formatter.write_str("TLS handshake ended early"),
            Self::Other => formatter.write_str("TLS handshake failed"),
        }
    }
}

impl Error for TlsHandshakeFailure {}

/// A typed failure raised by a secure upstream, in construction or in an
/// exchange.
///
/// The construction variants (through [`Self::DohRequest`]) are pre-I/O
/// defects. The exchange variants are:
///
/// * [`Self::Tls`] wraps a structured handshake failure and is always
///   [`SideEffectState::NotSent`], because no DNS query can be transmitted
///   before the handshake completes.
/// * [`Self::Transport`] wraps the exact existing typed [`UpstreamError`]
///   rather than duplicating or stringifying every control, send, receive, and
///   DNS-response cause; its side-effect state is the wrapped cause's state.
///
/// No variant is ever used to retry an exchange, fall back to plaintext, or
/// downgrade verification after an authentication failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecureError {
    /// A numeric dial address with port zero was supplied.
    ZeroDialPort,
    /// The TLS service identity was rejected.
    InvalidIdentity(IdentityError),
    /// The DoH service URL was rejected.
    InvalidServiceUrl(ServiceUrlError),
    /// The explicit TLS policy was rejected before any I/O.
    TlsConfig(TlsConfigError),
    /// The DoH GET request target was rejected before any I/O.
    DohRequest(DohRequestError),
    /// The TLS handshake failed before any DNS application byte.
    Tls(TlsHandshakeFailure),
    /// A control, framing, send, receive, or DNS-response failure, retained as
    /// the exact existing typed cause.
    Transport(UpstreamError),
}

impl SecureError {
    /// The DNS side-effect state of this failure.
    ///
    /// Construction defects and TLS handshake failures are always
    /// [`SideEffectState::NotSent`]; a transport cause keeps its own tracked
    /// state, so a partially written DoT frame stays `MaybeSent` and a failure
    /// after the frame was flushed stays `Sent`.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectState {
        match self {
            Self::ZeroDialPort
            | Self::InvalidIdentity(_)
            | Self::InvalidServiceUrl(_)
            | Self::TlsConfig(_)
            | Self::DohRequest(_)
            | Self::Tls(_) => SideEffectState::NotSent,
            Self::Transport(cause) => cause.side_effect(),
        }
    }
}

impl From<UpstreamError> for SecureError {
    fn from(cause: UpstreamError) -> Self {
        Self::Transport(cause)
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
            Self::DohRequest(reason) => write!(formatter, "invalid DoH GET request: {reason}"),
            Self::Tls(reason) => write!(formatter, "secure TLS handshake failed: {reason}"),
            Self::Transport(cause) => write!(formatter, "secure transport failed: {cause}"),
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
            Self::DohRequest(reason) => Some(reason),
            Self::Tls(reason) => Some(reason),
            Self::Transport(cause) => Some(cause),
        }
    }
}
