//! Pure Rust endpoint-resolution foundation (Phase 4 resolver).
//!
//! This module turns a configured upstream hostname into a validated numeric
//! dial destination without changing the independent TLS/HTTP service identity.
//! It is a pure-Rust sibling of the plain UDP/TCP and secure transports: it
//! composes with their numeric [`crate::Endpoint`], [`crate::DotEndpoint`] and
//! [`crate::DohEndpoint`] constructors and adds no Go adapter, cgo edge, ABI
//! symbol, backend selector, fallback, or production wiring.
//!
//! [`AddressFamily`] is re-exported from `mosdns-dns-core` so the bootstrap DNS
//! wire codec and the resolver agree on one family type; the reviewed Slice 0
//! wire contract is untouched.
//!
//! Slice 1 (this file) is the pure model: typed target/bootstrap/policy inputs,
//! config-version mapping, numeric dial bypass, deterministic expiry metadata,
//! and atomic publication state. It performs no I/O. The bounded UDP bootstrap
//! exchange arrives in a later slice of the same task.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use mosdns_dns_core::AddressFamily;

/// The family helper `dns-core` does not expose: which family an address is in.
trait AddressFamilyExt {
    fn of(address: IpAddr) -> AddressFamily;
}

impl AddressFamilyExt for AddressFamily {
    fn of(address: IpAddr) -> AddressFamily {
        match address {
            IpAddr::V4(_) => AddressFamily::Ipv4,
            IpAddr::V6(_) => AddressFamily::Ipv6,
        }
    }
}

/// An injected time source.
///
/// The resolver never reads the wall clock directly, so refresh and expiry are
/// deterministic under test and the caller keeps control of the time base.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The production [`Clock`], reading the process time base.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The typed product-level `bootstrap_version` selection.
///
/// This preserves the reviewed single-family contract exactly: `0` and `4`
/// select A/IPv4, `6` selects AAAA/IPv6. Native dual-stack resolution and
/// address racing are a required follow-up task, which is why the selection is
/// a named enum rather than a boolean.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigVersion {
    /// Version `0`, which the product currently maps to IPv4 like `4`.
    Zero,
    /// Version `4`: A records.
    Ipv4,
    /// Version `6`: AAAA records.
    Ipv6,
}

impl ConfigVersion {
    /// Maps the configured numeric version, rejecting undefined values instead
    /// of silently defaulting them.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::UnsupportedConfigVersion`] for any value other
    /// than `0`, `4`, or `6`.
    pub fn from_u8(version: u8) -> Result<Self, ResolverError> {
        match version {
            0 => Ok(Self::Zero),
            4 => Ok(Self::Ipv4),
            6 => Ok(Self::Ipv6),
            other => Err(ResolverError::UnsupportedConfigVersion(other)),
        }
    }

    /// The address family this version selects. `0` and `4` are IPv4.
    #[must_use]
    pub const fn family(self) -> AddressFamily {
        match self {
            Self::Zero | Self::Ipv4 => AddressFamily::Ipv4,
            Self::Ipv6 => AddressFamily::Ipv6,
        }
    }
}

/// A typed resolver/bootstrap failure.
///
/// Every variant is terminal and observable: a malformed name, zero port,
/// empty answer, wrong record family, malformed DNS reply, or terminal DNS
/// rcode never silently becomes a dial attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolverError {
    /// The hostname is empty, over 253 octets, or not plain hostname syntax.
    InvalidHostname,
    /// A port of zero cannot become a dial destination.
    ZeroPort,
    /// The configured `bootstrap_version` value is not defined.
    UnsupportedConfigVersion(u8),
    /// A bootstrap server must be a numeric address; hostname recursion is not
    /// allowed.
    BootstrapNotNumeric,
    /// The requested address family does not match a numeric literal's family.
    FamilyMismatch,
    /// The TTL/policy bounds are unusable.
    InvalidPolicy,
    /// A zero TTL would publish an already-dead result.
    InvalidTtl,
    /// The bootstrap endpoint's address family does not match the target's.
    BootstrapFamilyMismatch,
    /// The bootstrap exchange reached the caller's absolute deadline.
    BootstrapTimeout,
    /// The caller cancelled the resolution.
    Cancelled,
    /// The resolver owner was shut down.
    Closed,
    /// The bootstrap UDP socket could not be set up.
    BootstrapConnect,
    /// The bootstrap UDP send failed.
    BootstrapSend,
    /// The bootstrap UDP receive failed.
    BootstrapReceive,
    /// The bootstrap reply was not a valid, correlated DNS response.
    MalformedBootstrapResponse,
    /// The bootstrap reply carried no usable address of the requested family.
    NoUsableAddress,
    /// The bootstrap reply's DNS rcode was not `NOERROR`.
    BootstrapRcode(u16),
    /// A refresh generation was already running and this caller attached to it.
    AlreadyResolving,
    /// An IPv6 bootstrap socket could not be prepared on this host.
    BootstrapUnavailable,
}

impl fmt::Display for ResolverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidHostname => "invalid hostname",
            Self::ZeroPort => "zero port",
            Self::UnsupportedConfigVersion(_) => "unsupported bootstrap version",
            Self::BootstrapNotNumeric => "bootstrap server is not numeric",
            Self::FamilyMismatch => "address family mismatch",
            Self::InvalidPolicy => "invalid resolution policy",
            Self::InvalidTtl => "invalid ttl",
            Self::BootstrapFamilyMismatch => "bootstrap family mismatch",
            Self::BootstrapTimeout => "bootstrap timeout",
            Self::Cancelled => "cancelled",
            Self::Closed => "closed",
            Self::BootstrapConnect => "bootstrap connect failure",
            Self::BootstrapSend => "bootstrap send failure",
            Self::BootstrapReceive => "bootstrap receive failure",
            Self::MalformedBootstrapResponse => "malformed bootstrap response",
            Self::NoUsableAddress => "no usable address",
            Self::BootstrapRcode(_) => "bootstrap rcode",
            Self::AlreadyResolving => "resolution already in flight",
            Self::BootstrapUnavailable => "bootstrap transport unavailable",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for ResolverError {}

/// Validates plain ASCII hostname syntax and returns the normalized form.
///
/// The rules match the secure transport's identity validation: labels of at
/// most 63 octets, ASCII alphanumerics and `-` only, no leading or trailing
/// hyphen, and at most 253 octets overall. One optional trailing root dot is
/// removed, and the result is ASCII-lowercased.
fn normalize_hostname(host: &str) -> Result<String, ResolverError> {
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    if trimmed.is_empty() || trimmed.len() > 253 {
        return Err(ResolverError::InvalidHostname);
    }
    for label in trimmed.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ResolverError::InvalidHostname);
        }
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// A validated resolution input: a normalized hostname or numeric literal, a
/// nonzero port, and the selected address family.
///
/// A numeric host needs no DNS lookup at all; [`Self::is_numeric`] reports that
/// and [`Self::numeric_address`] returns the already-usable destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionTarget {
    host: Arc<str>,
    port: u16,
    family: AddressFamily,
    numeric: Option<IpAddr>,
}

impl ResolutionTarget {
    /// Validates and normalizes a target.
    ///
    /// `host` may be a DNS hostname or an IP literal. A hostname is normalized
    /// to lowercase without a trailing root dot; an IP literal is detected and
    /// bypasses resolution.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] for port zero,
    /// [`ResolverError::InvalidHostname`] for a malformed name, and
    /// [`ResolverError::FamilyMismatch`] when a literal's family disagrees with
    /// `family`.
    pub fn new(host: &str, port: u16, family: AddressFamily) -> Result<Self, ResolverError> {
        if port == 0 {
            return Err(ResolverError::ZeroPort);
        }
        if let Ok(literal) = host.parse::<IpAddr>() {
            if AddressFamily::of(literal) != family {
                return Err(ResolverError::FamilyMismatch);
            }
            return Ok(Self {
                host: Arc::from(literal.to_string().as_str()),
                port,
                family,
                numeric: Some(literal),
            });
        }
        Ok(Self {
            host: Arc::from(normalize_hostname(host)?.as_str()),
            port,
            family,
            numeric: None,
        })
    }

    /// The normalized hostname, or the canonical literal text.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The nonzero destination port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The selected address family.
    #[must_use]
    pub const fn family(&self) -> AddressFamily {
        self.family
    }

    /// Whether this target is an IP literal that bypasses DNS resolution.
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        self.numeric.is_some()
    }

    /// The already-usable numeric destination for a literal target, without any
    /// network access.
    #[must_use]
    pub fn numeric_address(&self) -> Option<SocketAddr> {
        self.numeric.map(|ip| SocketAddr::new(ip, self.port))
    }
}

/// A validated numeric UDP bootstrap peer.
///
/// The bootstrap server must be numeric so that resolving it can never recurse
/// into the resolver it configures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapEndpoint {
    address: SocketAddr,
}

impl BootstrapEndpoint {
    /// Validates a numeric bootstrap host and port.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::ZeroPort`] for port zero and
    /// [`ResolverError::BootstrapNotNumeric`] when `host` is not an IP literal
    /// (including a literal supplied with an embedded `:port`).
    pub fn new(host: &str, port: u16) -> Result<Self, ResolverError> {
        if port == 0 {
            return Err(ResolverError::ZeroPort);
        }
        let literal = host
            .parse::<IpAddr>()
            .map_err(|_| ResolverError::BootstrapNotNumeric)?;
        Ok(Self {
            address: SocketAddr::new(literal, port),
        })
    }

    /// The numeric UDP peer address.
    #[must_use]
    pub const fn address(self) -> SocketAddr {
        self.address
    }

    /// The peer's address family.
    #[must_use]
    pub fn family(self) -> AddressFamily {
        AddressFamily::of(self.address.ip())
    }
}

/// The reviewed resolution policy: TTL bounds and the retransmit interval.
///
/// The default keeps the existing product-compatible five-minute positive floor
/// and bounds excessively long TTLs at seven days.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolutionPolicy {
    min_ttl: Duration,
    max_ttl: Duration,
    retransmit_interval: Duration,
}

impl ResolutionPolicy {
    /// Builds a policy, rejecting unusable bounds instead of clamping silently.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::InvalidPolicy`] when `min_ttl` is zero or
    /// exceeds `max_ttl`.
    pub fn new(min_ttl: Duration, max_ttl: Duration) -> Result<Self, ResolverError> {
        if min_ttl.is_zero() || min_ttl > max_ttl {
            return Err(ResolverError::InvalidPolicy);
        }
        Ok(Self {
            min_ttl,
            max_ttl,
            retransmit_interval: Duration::from_secs(1),
        })
    }

    /// The lower clamp applied to an observed positive TTL.
    #[must_use]
    pub const fn min_ttl(&self) -> Duration {
        self.min_ttl
    }

    /// The upper clamp applied to an observed TTL.
    #[must_use]
    pub const fn max_ttl(&self) -> Duration {
        self.max_ttl
    }

    /// The interval between bootstrap retransmissions of the same query.
    #[must_use]
    pub const fn retransmit_interval(&self) -> Duration {
        self.retransmit_interval
    }

    /// Applies the explicit clamp to an observed TTL in seconds.
    #[must_use]
    pub fn clamp_ttl_secs(&self, ttl_secs: u32) -> Duration {
        let observed = Duration::from_secs(u64::from(ttl_secs));
        observed.clamp(self.min_ttl, self.max_ttl)
    }
}

impl Default for ResolutionPolicy {
    fn default() -> Self {
        Self {
            min_ttl: Duration::from_secs(300),
            max_ttl: Duration::from_secs(604_800),
            retransmit_interval: Duration::from_secs(1),
        }
    }
}

/// A validated numeric address with its freshness metadata.
///
/// The type carries no service identity: a resolved address never overwrites or
/// reconstructs TLS SNI or a DoH URL authority. [`Self::new_literal`] produces
/// the timeless form used by numeric dialing, which has no expiry at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedDestination {
    address: IpAddr,
    family: AddressFamily,
    ttl: Duration,
    expires_at: Option<Instant>,
}

impl ResolvedDestination {
    /// Builds a DNS-derived destination whose freshness starts at `now`.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::InvalidTtl`] for a zero TTL, because publishing
    /// an already-dead result is never a success.
    pub fn new(
        address: IpAddr,
        family: AddressFamily,
        ttl_secs: u32,
        now: Instant,
    ) -> Result<Self, ResolverError> {
        if ttl_secs == 0 {
            return Err(ResolverError::InvalidTtl);
        }
        let ttl = Duration::from_secs(u64::from(ttl_secs));
        Ok(Self {
            address,
            family,
            ttl,
            expires_at: Some(now + ttl),
        })
    }

    /// Builds the timeless destination of an IP literal.
    #[must_use]
    pub const fn new_literal(address: IpAddr, family: AddressFamily) -> Self {
        Self {
            address,
            family,
            ttl: Duration::ZERO,
            expires_at: None,
        }
    }

    /// The numeric address a transport may dial.
    #[must_use]
    pub const fn address(&self) -> IpAddr {
        self.address
    }

    /// The family this address was selected for.
    #[must_use]
    pub const fn family(&self) -> AddressFamily {
        self.family
    }

    /// The effective TTL; zero for a literal.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// The instant this destination stops being fresh; `None` for a literal,
    /// which never expires on its own.
    #[must_use]
    pub const fn expiry(&self) -> Option<Instant> {
        self.expires_at
    }

    /// Whether the destination is no longer fresh. A literal is never expired.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expiry| now >= expiry)
    }
}

/// A resolved destination together with the validated target it belongs to.
///
/// Combining both is what makes the numeric dial address available without ever
/// touching the service identity: the port comes from the target, the address
/// from resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedTarget {
    target: ResolutionTarget,
    destination: ResolvedDestination,
}

impl PublishedTarget {
    /// Publishes a destination for a validated target.
    #[must_use]
    pub const fn new(target: ResolutionTarget, destination: ResolvedDestination) -> Self {
        Self {
            target,
            destination,
        }
    }

    /// The validated target this publication belongs to.
    #[must_use]
    pub const fn target(&self) -> &ResolutionTarget {
        &self.target
    }

    /// The resolved numeric destination and its freshness metadata.
    #[must_use]
    pub const fn destination(&self) -> &ResolvedDestination {
        &self.destination
    }

    /// The resolved numeric address, without the port.
    #[must_use]
    pub const fn address(&self) -> IpAddr {
        self.destination.address()
    }

    /// The numeric address the transport dials: the resolved address at the
    /// target's port.
    #[must_use]
    pub fn dial(&self) -> SocketAddr {
        SocketAddr::new(self.destination.address(), self.target.port())
    }

    /// The destination port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.target.port()
    }

    /// The selected address family.
    #[must_use]
    pub const fn family(&self) -> AddressFamily {
        self.destination.family()
    }

    /// The effective TTL; zero for a literal.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.destination.ttl()
    }

    /// The freshness deadline; `None` for a literal.
    #[must_use]
    pub const fn expiry(&self) -> Option<Instant> {
        self.destination.expiry()
    }

    /// Whether the publication is no longer fresh.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.destination.is_expired(now)
    }
}

/// Turns an IP literal into an immediately usable published target.
///
/// This is the numeric `dial_addr` path: no DNS socket is opened, no bootstrap
/// traffic is generated, and no TTL applies.
///
/// # Errors
///
/// Returns [`ResolverError::ZeroPort`] for port zero.
pub fn resolve_numeric(address: SocketAddr) -> Result<PublishedTarget, ResolverError> {
    let family = AddressFamily::of(address.ip());
    let target = ResolutionTarget::new(&address.ip().to_string(), address.port(), family)?;
    Ok(PublishedTarget::new(
        target,
        ResolvedDestination::new_literal(address.ip(), family),
    ))
}

/// The owned publication state for one target/bootstrap tuple.
///
/// There is no global cache: the owner holds the state for exactly one
/// resolution key, so the tuple itself is the key. State is behind a short
/// synchronous mutex; no await ever happens while it is held.
#[derive(Debug)]
pub struct ResolverState {
    inner: Mutex<StateInner>,
}

#[derive(Debug, Default)]
struct StateInner {
    published: Option<PublishedTarget>,
    expired: Option<PublishedTarget>,
    last_error: Option<ResolverError>,
}

impl Default for ResolverState {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolverState {
    /// Creates an empty publication state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(StateInner {
                published: None,
                expired: None,
                last_error: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StateInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The published value, fresh or not, exactly as it was committed.
    #[must_use]
    pub fn published(&self) -> Option<PublishedTarget> {
        self.lock().published.clone()
    }

    /// The most recent value that expired without being replaced. It is
    /// retained as diagnostic evidence only and is never served as success.
    #[must_use]
    pub fn last_expired(&self) -> Option<PublishedTarget> {
        self.lock().expired.clone()
    }

    /// The last typed refresh diagnostic, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<ResolverError> {
        self.lock().last_error.clone()
    }

    /// Atomically publishes a complete, validated result.
    ///
    /// Publication also clears the recorded refresh diagnostic, because the
    /// failure it described has been resolved.
    pub fn publish(&self, target: PublishedTarget) {
        let mut inner = self.lock();
        inner.expired = None;
        inner.last_error = None;
        inner.published = Some(target);
    }

    /// Returns the published value only when it is still fresh at `now`.
    ///
    /// An expired value is moved to the diagnostic slot and never returned, so
    /// a caller can never receive a stale success. A literal publication has no
    /// expiry and is always fresh.
    pub fn serve_fresh(&self, now: Instant) -> Option<PublishedTarget> {
        let mut inner = self.lock();
        let published = inner.published.clone()?;
        if published.is_expired(now) {
            inner.expired = Some(published);
            return None;
        }
        Some(published)
    }

    /// Records a failed refresh without touching the published value.
    ///
    /// The previously published result stays exactly as it was: a failed
    /// refresh must never replace a valid one.
    pub fn record_refresh_failure(&self, error: ResolverError) {
        self.lock().last_error = Some(error);
    }
}

/// A shared publication state handle, as held by the resolver owner.
pub type SharedResolverState = Arc<ResolverState>;

/// The bounded UDP bootstrap exchange: one encoded query, one connected
/// ephemeral socket, one correlated reply.
mod bootstrap;

/// The resolver owner: typed inputs, one absolute deadline, owner shutdown, and
/// composition into the existing numeric transports.
mod owner;

pub use owner::{BootstrapResolver, ResolvedUpstream, ResolverComposition};

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    use super::{
        AddressFamily, Clock, ConfigVersion, ResolutionPolicy, ResolverState, SystemClock,
    };

    #[test]
    fn default_policy_clamps_both_ends() {
        let policy = ResolutionPolicy::default();
        assert_eq!(policy.clamp_ttl_secs(1), Duration::from_secs(300));
        assert_eq!(policy.clamp_ttl_secs(600), Duration::from_secs(600));
        assert_eq!(
            policy.clamp_ttl_secs(u32::MAX),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn zero_and_four_both_select_ipv4() {
        assert_eq!(
            ConfigVersion::from_u8(0).unwrap().family(),
            AddressFamily::Ipv4
        );
        assert_eq!(
            ConfigVersion::from_u8(4).unwrap().family(),
            AddressFamily::Ipv4
        );
        assert_eq!(
            ConfigVersion::from_u8(6).unwrap().family(),
            AddressFamily::Ipv6
        );
    }

    #[test]
    fn literal_publication_never_expires() {
        let state = ResolverState::new();
        let now = Instant::now();
        let target = super::ResolutionTarget::new("192.0.2.1", 853, AddressFamily::Ipv4).unwrap();
        state.publish(super::PublishedTarget::new(
            target,
            super::ResolvedDestination::new_literal(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                AddressFamily::Ipv4,
            ),
        ));
        assert!(
            state
                .serve_fresh(now + Duration::from_secs(1_000_000))
                .is_some()
        );
    }

    #[test]
    fn system_clock_is_monotonic_enough_to_order_calls() {
        let clock = SystemClock;
        let first = clock.now();
        let second = clock.now();
        assert!(second >= first);
    }
}
