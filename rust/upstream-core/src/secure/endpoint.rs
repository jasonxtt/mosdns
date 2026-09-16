//! Pre-I/O secure endpoint construction: validated service identity and a
//! separate numeric dial destination.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use url::{Host, Url};

use super::error::{IdentityError, SecureError, ServiceUrlError};

/// A validated DNS name or IP literal used as a secure upstream's TLS service
/// identity.
///
/// Construction only syntax-checks the identity; it never resolves a DNS name.
/// The numeric destination a socket connects to is supplied separately so an
/// override such as `dial_addr` cannot silently change the authenticated
/// identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerIdentity {
    /// Canonical text for the identity, without URL brackets for IPv6.
    normalized: String,
    kind: IdentityKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IdentityKind {
    DnsName,
    Ip(IpAddr),
}

impl ServerIdentity {
    /// Validates a DNS name or IP literal without performing any DNS lookup.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::InvalidIdentity`] when `identity` is empty or is
    /// neither a valid DNS name nor an IP literal.
    pub fn new(identity: &str) -> Result<Self, SecureError> {
        if identity.is_empty() {
            return Err(SecureError::InvalidIdentity(IdentityError::Empty));
        }
        // A caller may write a bare IPv6 literal, which is not a valid URL host
        // without brackets; accept it directly before delegating to `url`.
        let unwrapped = identity
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(identity);
        if let Ok(ip) = unwrapped.parse::<IpAddr>() {
            return Ok(Self::from_ip(ip));
        }
        match Host::parse(identity) {
            Ok(Host::Domain(name)) => Self::from_dns_name(&name),
            // A canonical IP literal was accepted above; an IP produced only by
            // `url` means a non-canonical numeric form such as `12345` or
            // `127.1`, which must not silently become a service identity.
            _ => Err(SecureError::InvalidIdentity(IdentityError::Malformed)),
        }
    }

    fn from_ip(ip: IpAddr) -> Self {
        Self {
            normalized: ip.to_string(),
            kind: IdentityKind::Ip(ip),
        }
    }

    fn from_dns_name(name: &str) -> Result<Self, SecureError> {
        // `Host::parse` already lowercased and IDNA-encoded the name; an FQDN
        // trailing dot is dropped so the stored identity is the SNI form.
        let normalized = name.strip_suffix('.').unwrap_or(name);
        validate_dns_name(normalized)?;
        Ok(Self {
            normalized: normalized.to_owned(),
            kind: IdentityKind::DnsName,
        })
    }

    pub(crate) fn from_url_host(host: Host<&str>) -> Self {
        match host {
            Host::Domain(name) => Self {
                normalized: name.to_owned(),
                kind: IdentityKind::DnsName,
            },
            Host::Ipv4(addr) => Self::from_ip(IpAddr::V4(addr)),
            Host::Ipv6(addr) => Self::from_ip(IpAddr::V6(addr)),
        }
    }

    /// The canonical text of the identity, without URL brackets for IPv6.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// Whether this identity is a DNS name.
    #[must_use]
    pub const fn is_dns_name(&self) -> bool {
        matches!(self.kind, IdentityKind::DnsName)
    }

    /// Whether this identity is an IP literal.
    #[must_use]
    pub const fn is_ip(&self) -> bool {
        matches!(self.kind, IdentityKind::Ip(_))
    }

    /// The IP literal, when this identity is one.
    #[must_use]
    pub const fn ip(&self) -> Option<IpAddr> {
        match &self.kind {
            IdentityKind::Ip(ip) => Some(*ip),
            IdentityKind::DnsName => None,
        }
    }

    /// The DNS name, when this identity is one.
    #[must_use]
    pub fn dns_name(&self) -> Option<&str> {
        match &self.kind {
            IdentityKind::DnsName => Some(&self.normalized),
            IdentityKind::Ip(_) => None,
        }
    }
}

impl fmt::Display for ServerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.normalized)
    }
}

/// Rejects DNS names that are not plain ASCII hostname syntax. `url` has
/// already performed IDNA/lowercase normalization; this check adds the label
/// length and character rules it does not enforce.
fn validate_dns_name(name: &str) -> Result<(), SecureError> {
    let malformed = || SecureError::InvalidIdentity(IdentityError::Malformed);
    if name.is_empty() || name.len() > 253 {
        return Err(malformed());
    }
    for label in name.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(malformed());
        }
    }
    Ok(())
}

/// A DNS-over-TLS destination: a numeric dial address plus a separate TLS
/// service identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DotEndpoint {
    dial: SocketAddr,
    identity: ServerIdentity,
}

impl DotEndpoint {
    /// Creates a DoT endpoint without resolving the identity or opening a
    /// socket.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::ZeroDialPort`] when `dial` has port zero.
    pub fn new(dial: SocketAddr, identity: ServerIdentity) -> Result<Self, SecureError> {
        reject_zero_port(dial)?;
        Ok(Self { dial, identity })
    }

    /// The numeric destination a DoT connection is opened to.
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

/// A DNS-over-HTTPS destination: an HTTPS service URL plus a separate numeric
/// dial address.
///
/// The URL authority and path stay those of the service; the numeric dial
/// address only selects where the connection is opened. An empty path is
/// normalized to `/`, and unrelated query parameters are preserved for a later
/// request encoder to keep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DohEndpoint {
    service: Url,
    dial: SocketAddr,
    identity: ServerIdentity,
}

impl DohEndpoint {
    /// Creates a DoH endpoint without resolving the service host or opening a
    /// socket.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::ZeroDialPort`] when `dial` has port zero, or
    /// [`SecureError::InvalidServiceUrl`] when `service_url` is not a valid
    /// HTTPS URL, has no host, or carries userinfo or a fragment.
    pub fn new(service_url: &str, dial: SocketAddr) -> Result<Self, SecureError> {
        reject_zero_port(dial)?;
        let mut service = Url::parse(service_url).map_err(map_parse_error)?;
        if service.scheme() != "https" {
            return Err(SecureError::InvalidServiceUrl(
                ServiceUrlError::UnsupportedScheme,
            ));
        }
        if !service.username().is_empty() || service.password().is_some() {
            return Err(SecureError::InvalidServiceUrl(ServiceUrlError::UserInfo));
        }
        if service.fragment().is_some() {
            return Err(SecureError::InvalidServiceUrl(ServiceUrlError::Fragment));
        }
        let host = match service.host() {
            Some(host) => host,
            None => return Err(SecureError::InvalidServiceUrl(ServiceUrlError::EmptyHost)),
        };
        let identity = ServerIdentity::from_url_host(host);
        // A parsed special URL normally already has `/`; keep the invariant
        // explicit so an empty path can never reach a later request encoder.
        if service.path().is_empty() {
            service.set_path("/");
        }
        Ok(Self {
            service,
            dial,
            identity,
        })
    }

    /// The numeric destination a DoH connection is opened to.
    #[must_use]
    pub const fn dial(&self) -> SocketAddr {
        self.dial
    }

    /// The validated service URL, including its authority, path, and query.
    #[must_use]
    pub fn service_url(&self) -> &Url {
        &self.service
    }

    /// The TLS/HTTP identity derived from the service URL host.
    #[must_use]
    pub const fn identity(&self) -> &ServerIdentity {
        &self.identity
    }

    /// The service host as serialized by the URL; an IPv6 host keeps the
    /// brackets required in an authority.
    #[must_use]
    pub fn host(&self) -> &str {
        self.service.host_str().unwrap_or_default()
    }

    /// The HTTP authority: the service host plus its explicit port, if any.
    #[must_use]
    pub fn authority(&self) -> String {
        match self.service.port() {
            Some(port) => format!("{}:{port}", self.host()),
            None => self.host().to_owned(),
        }
    }

    /// The service path, always at least `/`.
    #[must_use]
    pub fn path(&self) -> &str {
        self.service.path()
    }

    /// The raw service query, when one is present.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.service.query()
    }
}

fn reject_zero_port(dial: SocketAddr) -> Result<(), SecureError> {
    if dial.port() == 0 {
        Err(SecureError::ZeroDialPort)
    } else {
        Ok(())
    }
}

fn map_parse_error(error: url::ParseError) -> SecureError {
    let reason = match error {
        url::ParseError::EmptyHost => ServiceUrlError::EmptyHost,
        _ => ServiceUrlError::Malformed,
    };
    SecureError::InvalidServiceUrl(reason)
}
