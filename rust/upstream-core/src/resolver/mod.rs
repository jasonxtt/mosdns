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
/// This is the *configured* value, not the effective lookup plan: `0` is the
/// explicit dual-stack entry, while `4` and `6` stay single-family. Map it
/// through [`Self::mode`] to get the families that will actually be queried.
/// Address racing and Happy Eyeballs are explicitly not part of that plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigVersion {
    /// Explicit version `0`: collect A and AAAA, preferring A.
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

    /// Maps an omitted configuration value to the product default of `4`.
    ///
    /// Omission and an explicit `0` must never collapse into the same integer:
    /// `None` is the A-only default, while `Some(0)` is the explicit dual-stack
    /// entry. This is the boundary that keeps the default from silently becoming
    /// a second A+AAAA lookup.
    ///
    /// # Errors
    ///
    /// As [`Self::from_u8`] for any present but undefined value.
    pub fn from_optional(version: Option<u8>) -> Result<Self, ResolverError> {
        match version {
            None => Ok(Self::Ipv4),
            Some(value) => Self::from_u8(value),
        }
    }

    /// The address family this version prefers. `0` and `4` prefer IPv4.
    ///
    /// For [`Self::Zero`] this is the *preferred* family, not the only one; use
    /// [`Self::mode`] for the complete lookup plan.
    #[must_use]
    pub const fn family(self) -> AddressFamily {
        match self {
            Self::Zero | Self::Ipv4 => AddressFamily::Ipv4,
            Self::Ipv6 => AddressFamily::Ipv6,
        }
    }

    /// The effective lookup mode for this version.
    #[must_use]
    pub const fn mode(self) -> ResolutionMode {
        match self {
            Self::Zero => ResolutionMode::PreferIpv4Dual,
            Self::Ipv4 => ResolutionMode::Ipv4,
            Self::Ipv6 => ResolutionMode::Ipv6,
        }
    }
}

impl Default for ConfigVersion {
    /// The product default when no version is configured: `4`, A-only.
    fn default() -> Self {
        Self::Ipv4
    }
}

/// The effective set of families a resolution will query.
///
/// This is deliberately an enum rather than a boolean or an integer so the
/// distinction between "omitted/`4`", "`6`", and "explicit `0`" stays explicit
/// at the type level, and so a later address-selection policy can be additive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionMode {
    /// A only.
    Ipv4,
    /// AAAA only.
    Ipv6,
    /// Collect A and AAAA independently; prefer a usable A.
    PreferIpv4Dual,
}

/// The families asked for in a single-family mode, in query order.
const IPV4_FAMILIES: &[AddressFamily] = &[AddressFamily::Ipv4];
/// The families asked for in single-family AAAA mode.
const IPV6_FAMILIES: &[AddressFamily] = &[AddressFamily::Ipv6];
/// The families asked for in explicit dual mode: A first, then AAAA.
///
/// The order is the query and selection order, not a race: the two lookups are
/// issued under the same deadline and neither opens a target connection.
const DUAL_FAMILIES: &[AddressFamily] = &[AddressFamily::Ipv4, AddressFamily::Ipv6];

impl ResolutionMode {
    /// The mode implied by a target's single declared family.
    ///
    /// This is the compatibility seam: the existing single-family
    /// `ResolutionTarget` constructors keep their exact meaning.
    #[must_use]
    pub const fn from_family(family: AddressFamily) -> Self {
        match family {
            AddressFamily::Ipv4 => Self::Ipv4,
            AddressFamily::Ipv6 => Self::Ipv6,
        }
    }

    /// Every family this mode queries, in query order.
    #[must_use]
    pub const fn families(self) -> &'static [AddressFamily] {
        match self {
            Self::Ipv4 => IPV4_FAMILIES,
            Self::Ipv6 => IPV6_FAMILIES,
            Self::PreferIpv4Dual => DUAL_FAMILIES,
        }
    }

    /// Whether this mode issues more than one DNS lookup for a hostname target.
    #[must_use]
    pub const fn is_dual(self) -> bool {
        matches!(self, Self::PreferIpv4Dual)
    }

    /// The family a fresh candidate is preferred from.
    #[must_use]
    pub const fn preferred_family(self) -> AddressFamily {
        match self {
            Self::Ipv6 => AddressFamily::Ipv6,
            Self::Ipv4 | Self::PreferIpv4Dual => AddressFamily::Ipv4,
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
    /// An address disagrees with the family it was validated or published for:
    /// a numeric literal whose family differs from the requested one, or a
    /// resolved destination whose address is not in its declared family.
    FamilyMismatch,
    /// The TTL/policy bounds are unusable.
    InvalidPolicy,
    /// A zero TTL would publish an already-dead result.
    InvalidTtl,
    /// Retained for API stability. The bootstrap peer's transport family and the
    /// target's answer family are independent, so this resolver never returns
    /// it: an IPv4 bootstrap may answer AAAA and an IPv6 bootstrap may answer A.
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
    /// The bootstrap reply carried the DNS truncation (TC) flag. It is kept
    /// distinct from a malformed reply because a truncated answer is a
    /// legitimate DNS observation, and this foundation deliberately performs no
    /// TCP bootstrap fallback: the typed error is the whole policy.
    Truncated,
    /// The bootstrap reply carried no usable address of the requested family.
    NoUsableAddress,
    /// The bootstrap reply's DNS rcode was not `NOERROR`.
    BootstrapRcode(u16),
    /// A refresh generation was already running and this caller attached to it.
    AlreadyResolving,
    /// Unpredictable bootstrap transaction IDs are unavailable on this host, so
    /// the production construction path refused to fall back to a predictable
    /// source.
    UnpredictableIdsUnavailable,
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
            Self::Truncated => "truncated bootstrap response",
            Self::NoUsableAddress => "no usable address",
            Self::BootstrapRcode(_) => "bootstrap rcode",
            Self::AlreadyResolving => "resolution already in flight",
            Self::UnpredictableIdsUnavailable => "unpredictable ids unavailable",
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
    /// The bounds are second-granular DNS TTLs, so a bound that cannot be
    /// expressed as a whole number of seconds within the wire's 32-bit field is
    /// refused here rather than being truncated later at parse time. This is
    /// what keeps `dns_core_policy` infallible for any constructed policy.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::InvalidPolicy`] when `min_ttl` is zero, when
    /// `min_ttl` exceeds `max_ttl`, or when either bound does not fit the wire's
    /// unsigned 32-bit second field exactly.
    pub fn new(min_ttl: Duration, max_ttl: Duration) -> Result<Self, ResolverError> {
        if min_ttl.is_zero() || min_ttl > max_ttl {
            return Err(ResolverError::InvalidPolicy);
        }
        // Reject any bound the wire cannot carry verbatim.
        let max_expressible = Duration::from_secs(u64::from(u32::MAX));
        if min_ttl.subsec_nanos() != 0 || max_ttl.subsec_nanos() != 0 || max_ttl > max_expressible {
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

    /// Derives the `dns-core` wire-parse policy from these bounds.
    ///
    /// The wire codec clamps the effective TTL it reports, so parsing a reply
    /// under `dns-core`'s own defaults would silently override a caller's custom
    /// floor or ceiling. The resolver therefore always parses under its own
    /// bounds. The codec's CNAME link bound is not a resolver policy knob, so it
    /// keeps the reviewed default.
    ///
    /// The constructor already guaranteed both bounds fit the wire's unsigned
    /// 32-bit second field exactly, so this conversion cannot truncate.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverError::InvalidPolicy`] only if `dns-core` itself
    /// rejects the derived bounds, which the constructor's ordering check makes
    /// unreachable for a constructed policy.
    pub fn dns_core_policy(&self) -> Result<mosdns_dns_core::CnameChainPolicy, ResolverError> {
        let min_ttl =
            u32::try_from(self.min_ttl.as_secs()).map_err(|_| ResolverError::InvalidPolicy)?;
        let max_ttl =
            u32::try_from(self.max_ttl.as_secs()).map_err(|_| ResolverError::InvalidPolicy)?;
        mosdns_dns_core::CnameChainPolicy::new(
            mosdns_dns_core::RESOLVER_DEFAULT_MAX_CNAME_LINKS,
            min_ttl,
            max_ttl,
        )
        .map_err(|_| ResolverError::InvalidPolicy)
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
    /// an already-dead result is never a success, and
    /// [`ResolverError::FamilyMismatch`] when `address` is not in `family`.
    pub fn new(
        address: IpAddr,
        family: AddressFamily,
        ttl_secs: u32,
        now: Instant,
    ) -> Result<Self, ResolverError> {
        if AddressFamily::of(address) != family {
            return Err(ResolverError::FamilyMismatch);
        }
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
    ///
    /// The family must match `address`; a mismatch is a programming error and is
    /// caught by a debug assertion, while [`Self::new`] returns the typed error
    /// for untrusted input.
    #[must_use]
    pub const fn new_literal(address: IpAddr, family: AddressFamily) -> Self {
        debug_assert!(matches!(
            (address, family),
            (IpAddr::V4(_), AddressFamily::Ipv4) | (IpAddr::V6(_), AddressFamily::Ipv6)
        ));
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

/// One family's contribution to a resolution generation.
///
/// A candidate is either a fresh address for its family or a typed failure for
/// that family. Keeping both shapes in one type is what lets a dual-mode
/// generation record "A failed, AAAA succeeded" without discarding either fact.
#[derive(Clone, Debug)]
pub enum FamilyCandidate {
    /// A usable address of this family, with its own freshness metadata.
    Address(ResolvedDestination),
    /// This family's lookup reached a terminal typed failure.
    Failed(ResolverError),
}

impl FamilyCandidate {
    /// The usable destination, if this family produced one.
    #[must_use]
    pub const fn destination(&self) -> Option<&ResolvedDestination> {
        match self {
            Self::Address(destination) => Some(destination),
            Self::Failed(_) => None,
        }
    }

    /// The typed failure, if this family produced one.
    #[must_use]
    pub const fn error(&self) -> Option<&ResolverError> {
        match self {
            Self::Address(_) => None,
            Self::Failed(error) => Some(error),
        }
    }

    /// Whether the candidate carries an address that is still fresh at `now`.
    ///
    /// A failed candidate is never fresh, so a failure can never be selected.
    #[must_use]
    pub fn is_fresh(&self, now: Instant) -> bool {
        match self {
            Self::Address(destination) => !destination.is_expired(now),
            Self::Failed(_) => false,
        }
    }
}

/// The complete result of one resolution generation for one target.
///
/// This is the multi-family publication shape: it carries the target identity,
/// the generation ordinal, each queried family's candidate, and each family's
/// typed diagnostic for this generation. It is observation-only — the owner is
/// the only writer — and it deliberately exposes no mutation. The type is
/// multi-family even though the current wire codec still selects one
/// deterministic address per query.
///
/// A candidate and a diagnostic are independent slots on purpose. When a
/// family's refresh fails but a still-fresh address from an earlier generation
/// is carried forward, *both* facts must stay observable: the carried address is
/// the candidate, and the current generation's failure is the diagnostic. That
/// is why a carried-forward candidate is not rewritten into a `Failed`
/// candidate, and why a diagnostic is not derived only from the candidate shape.
///
/// The snapshot carries no service identity: a resolved address never rewrites
/// TLS SNI or a DoH URL authority.
#[derive(Clone, Debug)]
pub struct ResolutionSnapshot {
    target: ResolutionTarget,
    mode: ResolutionMode,
    generation: u64,
    ipv4: Option<FamilyCandidate>,
    ipv6: Option<FamilyCandidate>,
    ipv4_diagnostic: Option<ResolverError>,
    ipv6_diagnostic: Option<ResolverError>,
}

impl ResolutionSnapshot {
    /// Builds a snapshot for one target and mode, as generation zero.
    ///
    /// The generation ordinal is assigned by the owner at publication, so a
    /// snapshot built directly is only an uncommitted candidate.
    #[must_use]
    pub const fn new(
        target: ResolutionTarget,
        mode: ResolutionMode,
        ipv4: Option<FamilyCandidate>,
        ipv6: Option<FamilyCandidate>,
    ) -> Self {
        Self {
            target,
            mode,
            generation: 0,
            ipv4,
            ipv6,
            ipv4_diagnostic: None,
            ipv6_diagnostic: None,
        }
    }

    /// The target this generation resolved.
    #[must_use]
    pub const fn target(&self) -> &ResolutionTarget {
        &self.target
    }

    /// The mode this generation ran under.
    #[must_use]
    pub const fn mode(&self) -> ResolutionMode {
        self.mode
    }

    /// The monotonic ordinal of the generation that produced this snapshot.
    ///
    /// Every publication through the owner's state bumps this value, so a
    /// caller can observe which generation it is looking at and detect that a
    /// superseded generation did not replace a newer one. Zero means the
    /// snapshot has not been published.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Assigns the generation ordinal. Only the owner's publication path does.
    pub(crate) const fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// The IPv4 candidate, if that family was queried.
    #[must_use]
    pub const fn ipv4(&self) -> Option<&FamilyCandidate> {
        self.ipv4.as_ref()
    }

    /// The IPv6 candidate, if that family was queried.
    #[must_use]
    pub const fn ipv6(&self) -> Option<&FamilyCandidate> {
        self.ipv6.as_ref()
    }

    /// The candidate recorded for `family`, if that family was queried.
    #[must_use]
    pub const fn candidate(&self, family: AddressFamily) -> Option<&FamilyCandidate> {
        match family {
            AddressFamily::Ipv4 => self.ipv4(),
            AddressFamily::Ipv6 => self.ipv6(),
        }
    }

    /// Replaces one family's candidate.
    ///
    /// Crate-private by construction of the type: a snapshot is observation-only
    /// to every consumer, and only the resolver owner's publication path may
    /// carry a still-fresh candidate forward across a generation.
    pub(crate) fn set_candidate(&mut self, family: AddressFamily, candidate: FamilyCandidate) {
        match family {
            AddressFamily::Ipv4 => self.ipv4 = Some(candidate),
            AddressFamily::Ipv6 => self.ipv6 = Some(candidate),
        }
    }

    /// Records this generation's typed diagnostic for one family.
    pub(crate) fn set_diagnostic(&mut self, family: AddressFamily, error: ResolverError) {
        match family {
            AddressFamily::Ipv4 => self.ipv4_diagnostic = Some(error),
            AddressFamily::Ipv6 => self.ipv6_diagnostic = Some(error),
        }
    }

    /// This generation's typed diagnostic for `family`, if that family failed.
    ///
    /// This is the *current generation's* failure, which is retained even when a
    /// still-fresh address from an earlier generation is carried forward for the
    /// same family. Falling back to the candidate's own failure keeps snapshots
    /// built without an explicit diagnostic (single-family and test paths)
    /// reporting their error exactly as before.
    #[must_use]
    pub fn family_error(&self, family: AddressFamily) -> Option<ResolverError> {
        let explicit = match family {
            AddressFamily::Ipv4 => self.ipv4_diagnostic.as_ref(),
            AddressFamily::Ipv6 => self.ipv6_diagnostic.as_ref(),
        };
        explicit
            .or_else(|| self.candidate(family).and_then(FamilyCandidate::error))
            .cloned()
    }

    /// The fresh address selected for this generation, if any.
    ///
    /// Selection is explicit and deterministic: a fresh IPv4 candidate always
    /// wins; AAAA is selected only when no IPv4 candidate is fresh. Families are
    /// considered in a fixed order regardless of which lookup completed first.
    #[must_use]
    pub fn select(&self, now: Instant) -> Option<ResolvedDestination> {
        for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
            if let Some(FamilyCandidate::Address(destination)) = self.candidate(family) {
                if !destination.is_expired(now) {
                    return Some(*destination);
                }
            }
        }
        None
    }

    /// Whether the family this mode prefers has a fresh candidate.
    ///
    /// A dual-mode generation is only completely satisfied while its preferred
    /// family is fresh: an expired A with a fresh AAAA is still serviceable, but
    /// it is a state a later caller must be allowed to refresh.
    #[must_use]
    pub fn preferred_is_fresh(&self, now: Instant) -> bool {
        matches!(
            self.candidate(self.mode.preferred_family()),
            Some(FamilyCandidate::Address(destination)) if !destination.is_expired(now)
        )
    }

    /// The selected fresh destination as a dialable publication, if any.
    ///
    /// This is the bridge to the existing single-address composition boundary:
    /// the returned [`PublishedTarget`] carries only the numeric address and the
    /// target's original port.
    #[must_use]
    pub fn selected_target(&self, now: Instant) -> Option<PublishedTarget> {
        self.select(now)
            .map(|destination| PublishedTarget::new(self.target.clone(), destination))
    }

    /// The selected destination **ignoring freshness**, as a publication.
    ///
    /// This exists for the diagnostic `published`/`last_expired` accessors,
    /// whose contract is "the value exactly as it was committed, fresh or not".
    /// It is never the path that decides success: freshness is always applied by
    /// [`Self::selected_target`] before a caller can receive an address.
    #[must_use]
    pub fn committed_target(&self) -> Option<PublishedTarget> {
        for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
            if let Some(FamilyCandidate::Address(destination)) = self.candidate(family) {
                return Some(PublishedTarget::new(self.target.clone(), *destination));
            }
        }
        None
    }

    /// The aggregate typed failure when no family has a fresh candidate.
    ///
    /// A single-family mode reports its own family's error directly. Dual mode
    /// prefers the preferred family's cause, then any other recorded cause; the
    /// per-family causes always stay available through [`Self::family_error`].
    #[must_use]
    pub fn aggregate_error(&self) -> ResolverError {
        if let Some(error) = self.family_error(self.mode.preferred_family()) {
            return error;
        }
        for family in self.mode.families() {
            if let Some(error) = self.family_error(*family) {
                return error;
            }
        }
        ResolverError::NoUsableAddress
    }

    /// Every recorded family failure, in the mode's family order.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<(AddressFamily, ResolverError)> {
        let mut diagnostics = Vec::new();
        for family in self.mode.families() {
            if let Some(error) = self.family_error(*family) {
                diagnostics.push((*family, error));
            }
        }
        diagnostics
    }
}

/// The owned publication state for one target/bootstrap tuple.
///
/// There is no global cache: the owner holds the state for exactly one
/// resolution key, so the tuple itself is the key. State is behind a short
/// synchronous mutex; no await ever happens while it is held.
///
/// Only the *read* side is public. The mutating operations are crate-private, so
/// a caller holding an `Arc<ResolverState>` can observe diagnostics but can
/// never publish, fake freshness, or clear a recorded failure from outside the
/// owner — every mutation in production goes through the owner's lifecycle
/// linearization gate in [`crate::resolver::BootstrapResolver`].
///
/// The state is multi-family: it retains one candidate per family so a failed
/// or expired family cannot erase a still-fresh sibling. The single-family
/// accessors below are derived from that snapshot, so existing callers keep
/// their meaning.
#[derive(Debug)]
pub struct ResolverState {
    inner: Mutex<StateInner>,
}

#[derive(Debug, Default)]
struct StateInner {
    /// The current multi-family generation result.
    published: Option<ResolutionSnapshot>,
    /// The most recent generation whose candidates had all expired, retained as
    /// diagnostic evidence only and never served as success.
    expired: Option<ResolutionSnapshot>,
    /// The last typed generation-level diagnostic, if any.
    last_error: Option<ResolverError>,
    /// The monotonic ordinal assigned to the most recent publication.
    generation: u64,
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
                generation: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StateInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The published generation snapshot, fresh or not, exactly as committed.
    #[must_use]
    pub fn snapshot(&self) -> Option<ResolutionSnapshot> {
        self.lock().published.clone()
    }

    /// The address published for the state's selected family, if any.
    ///
    /// This preserves the original single-family read API exactly: it is the
    /// selected candidate of the published generation, independent of freshness.
    #[must_use]
    pub fn published(&self) -> Option<PublishedTarget> {
        self.lock()
            .published
            .as_ref()
            .and_then(ResolutionSnapshot::committed_target)
    }

    /// The most recent generation that expired without being replaced. It is
    /// retained as diagnostic evidence only and is never served as success.
    #[must_use]
    pub fn last_expired(&self) -> Option<PublishedTarget> {
        self.lock()
            .expired
            .as_ref()
            .and_then(ResolutionSnapshot::committed_target)
    }

    /// The last typed refresh diagnostic, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<ResolverError> {
        self.lock().last_error.clone()
    }

    /// The ordinal of the most recently published generation; zero if none.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Atomically publishes a complete, validated generation.
    ///
    /// Publication assigns the next monotonic generation ordinal and clears the
    /// recorded refresh diagnostic, because the failure it described has been
    /// resolved. Returns the committed snapshot with its ordinal.
    pub(crate) fn publish_snapshot(&self, mut snapshot: ResolutionSnapshot) -> ResolutionSnapshot {
        let mut inner = self.lock();
        inner.generation += 1;
        snapshot.set_generation(inner.generation);
        inner.expired = None;
        inner.last_error = None;
        inner.published = Some(snapshot.clone());
        drop(inner);
        snapshot
    }

    /// Publishes a generation, preserving a still-fresh candidate for any family
    /// the new generation failed to produce, and returns the merged snapshot.
    ///
    /// A complete snapshot replaces the previous one atomically, but a family
    /// whose lookup failed must not erase a candidate that is still fresh: doing
    /// so would let one family's outage destroy a usable address. Only a *fresh*
    /// prior candidate is carried over; an expired one is never resurrected, so
    /// this can never serve a stale address as success.
    ///
    /// Carrying a candidate forward does **not** discard this generation's
    /// failure. The carried address stays the family's candidate and the new
    /// typed error stays that family's diagnostic, so both remain observable
    /// through [`ResolutionSnapshot::family_error`] and
    /// [`ResolutionSnapshot::diagnostics`].
    ///
    /// The returned merged snapshot is what callers must select from: an earlier
    /// selection taken before the merge would miss a candidate this generation
    /// only became serviceable through.
    pub(crate) fn publish_generation(
        &self,
        mut snapshot: ResolutionSnapshot,
        now: Instant,
    ) -> ResolutionSnapshot {
        // Capture this generation's own failures as diagnostics *before* any
        // candidate is carried forward. This is what keeps a carried address
        // from hiding the failure that caused the carry: the failure is recorded
        // from the incoming snapshot, so overwriting the candidate slot below
        // cannot erase it.
        for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
            let failed = snapshot
                .candidate(family)
                .and_then(FamilyCandidate::error)
                .cloned();
            if let Some(error) = failed {
                snapshot.set_diagnostic(family, error);
            }
        }
        let mut inner = self.lock();
        if let Some(previous) = inner.published.as_ref() {
            for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
                // Only fill a family the new generation did not satisfy with a
                // fresh address of its own.
                let new_is_fresh = snapshot
                    .candidate(family)
                    .is_some_and(|candidate| candidate.is_fresh(now));
                if new_is_fresh {
                    continue;
                }
                if let Some(FamilyCandidate::Address(destination)) = previous.candidate(family) {
                    if !destination.is_expired(now) {
                        // The address is carried forward; the diagnostic recorded
                        // just above is kept alongside it.
                        snapshot.set_candidate(family, FamilyCandidate::Address(*destination));
                    }
                }
            }
        }
        inner.generation += 1;
        snapshot.set_generation(inner.generation);
        inner.expired = None;
        inner.last_error = None;
        inner.published = Some(snapshot.clone());
        drop(inner);
        snapshot
    }

    /// Publishes a single-family result.
    ///
    /// Compatibility seam for the one-family constructor paths; the snapshot
    /// shape is identical to what the dual path produces. A failure is recorded
    /// as that family's diagnostic so the aggregate error stays readable.
    pub(crate) fn publish(&self, target: PublishedTarget) -> ResolutionSnapshot {
        let destination = *target.destination();
        let (ipv4, ipv6) = match destination.family() {
            AddressFamily::Ipv4 => (Some(FamilyCandidate::Address(destination)), None),
            AddressFamily::Ipv6 => (None, Some(FamilyCandidate::Address(destination))),
        };
        self.publish_snapshot(ResolutionSnapshot::new(
            target.target().clone(),
            ResolutionMode::from_family(destination.family()),
            ipv4,
            ipv6,
        ))
    }

    /// Returns the published result for the fast path, or `None` when this
    /// caller should run (or join) a refresh generation.
    ///
    /// The fast path only satisfies a caller whose *preferred* family is fresh.
    /// That matters for dual mode: when the preferred A has expired while AAAA
    /// is still fresh, the address is usable but the generation is not settled,
    /// so returning here would keep serving AAAA forever without ever retrying
    /// the expired family. Treating it as a miss lets the next caller refresh A
    /// — and if that A leg fails, the still-fresh AAAA is carried forward and
    /// still satisfies the caller.
    ///
    /// A generation with *no* fresh family at all is moved to the diagnostic
    /// slot and is never served. A partially fresh generation is deliberately
    /// left published: its fresh sibling is still legitimately serviceable while
    /// refreshes are attempted.
    pub(crate) fn serve_fresh(&self, now: Instant) -> Option<PublishedTarget> {
        let mut inner = self.lock();
        let snapshot = inner.published.clone()?;
        if snapshot.preferred_is_fresh(now) {
            return snapshot.selected_target(now);
        }
        // Nothing fresh in any family: retain it as evidence, never serve it.
        if snapshot.select(now).is_none() {
            inner.expired = Some(snapshot);
        }
        None
    }

    /// Records a failed refresh without touching the published generation.
    ///
    /// The previously published result stays exactly as it was: a failed
    /// refresh must never replace a valid one.
    pub(crate) fn record_refresh_failure(&self, error: ResolverError) {
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
        AddressFamily, Clock, ConfigVersion, FamilyCandidate, ResolutionMode, ResolutionPolicy,
        ResolutionSnapshot, ResolvedDestination, ResolverError, ResolverState, SystemClock,
    };

    /// A deterministic clock, advanced by hand. It is used only by the
    /// crate-internal state-model tests below, which must reach the state's
    /// crate-private mutations and therefore cannot live in the external
    /// integration tests.
    struct SteppingClock {
        now: Instant,
    }

    impl SteppingClock {
        fn new() -> Self {
            Self {
                now: Instant::now(),
            }
        }

        fn advance(&mut self, seconds: u64) {
            self.now += Duration::from_secs(seconds);
        }
    }

    impl Clock for SteppingClock {
        fn now(&self) -> Instant {
            self.now
        }
    }

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

    /// The state starts empty and never serves a value past its expiry, while
    /// retaining the expired value as diagnostic evidence only.
    ///
    /// This is a crate-internal test because it exercises the state's mutation
    /// surface directly; that surface is deliberately not public, so an
    /// external caller of the crate cannot publish or fake freshness around the
    /// owner's lifecycle gate.
    #[test]
    fn resolver_state_starts_empty_and_never_serves_a_stale_value() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        assert!(state.published().is_none());

        let now = clock.now();
        let destination = ResolvedDestination::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            AddressFamily::Ipv4,
            600,
            now,
        )
        .expect("valid destination");
        let target =
            super::ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
                .expect("valid target");
        state.publish(super::PublishedTarget::new(target, destination));

        // A fresh entry is served.
        assert!(state.serve_fresh(now).is_some());
        assert!(state.published().is_some());

        // Once expired it is never served, but it is retained as diagnostics.
        clock.advance(600);
        let expired_at = clock.now();
        assert!(state.serve_fresh(expired_at).is_none());
        assert!(
            state.published().is_some(),
            "the expired value is retained as evidence, not served"
        );
        assert!(state.last_expired().is_some());
    }

    /// A failed refresh records a typed diagnostic and never replaces the
    /// previously published value; a later success does advance it.
    #[test]
    fn a_failed_refresh_never_replaces_a_published_value() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        let target =
            super::ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
                .expect("valid target");
        let now = clock.now();
        state.publish(super::PublishedTarget::new(
            target.clone(),
            ResolvedDestination::new(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                AddressFamily::Ipv4,
                600,
                now,
            )
            .expect("valid destination"),
        ));
        let published_before = state.published().expect("published");

        // A failed refresh records a typed diagnostic and changes nothing else.
        state.record_refresh_failure(ResolverError::BootstrapTimeout);
        let after = state.published().expect("the old value survives");
        assert_eq!(after.address(), published_before.address());
        assert_eq!(
            state.last_error(),
            Some(ResolverError::BootstrapTimeout),
            "the diagnostic is observable"
        );

        // A successful replacement does advance the published value.
        clock.advance(10);
        state.publish(super::PublishedTarget::new(
            target,
            ResolvedDestination::new(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2)),
                AddressFamily::Ipv4,
                900,
                clock.now(),
            )
            .expect("valid destination"),
        ));
        assert_eq!(
            state.published().expect("published").address(),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2))
        );
    }

    /// An expired value is never served as success, including exactly at the
    /// expiry boundary.
    #[test]
    fn resolver_state_never_serves_an_expired_value_as_success() {
        let clock = SteppingClock::new();
        let state = ResolverState::new();
        let target =
            super::ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
                .expect("valid target");
        let now = clock.now();
        state.publish(super::PublishedTarget::new(
            target,
            ResolvedDestination::new(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                AddressFamily::Ipv4,
                300,
                now,
            )
            .expect("valid destination"),
        ));
        assert!(state.serve_fresh(now + Duration::from_secs(299)).is_some());
        // At the boundary and beyond, the caller must resolve again.
        assert!(state.serve_fresh(now + Duration::from_secs(300)).is_none());
        assert!(state.serve_fresh(now + Duration::from_secs(301)).is_none());
    }

    // -----------------------------------------------------------------------
    // Partial refresh, merged publication, and generation metadata
    // -----------------------------------------------------------------------

    /// A dual-mode target.
    fn dual_target() -> super::ResolutionTarget {
        super::ResolutionTarget::new("dual.example.org", 853, AddressFamily::Ipv4)
            .expect("valid target")
    }

    /// An A candidate with the given TTL, starting at `now`.
    fn a_candidate(ttl_secs: u32, now: Instant) -> FamilyCandidate {
        FamilyCandidate::Address(
            ResolvedDestination::new(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
                AddressFamily::Ipv4,
                ttl_secs,
                now,
            )
            .expect("valid A"),
        )
    }

    /// An AAAA candidate with the given TTL, starting at `now`.
    fn aaaa_candidate(ttl_secs: u32, now: Instant) -> FamilyCandidate {
        FamilyCandidate::Address(
            ResolvedDestination::new(
                IpAddr::V6("2001:db8::10".parse().expect("v6")),
                AddressFamily::Ipv6,
                ttl_secs,
                now,
            )
            .expect("valid AAAA"),
        )
    }

    /// A failed A leg carrying the given typed error.
    fn a_failure(error: ResolverError) -> FamilyCandidate {
        FamilyCandidate::Failed(error)
    }

    /// A dual snapshot for `target` with the given family slots.
    fn dual_snapshot(
        target: &super::ResolutionTarget,
        ipv4: Option<FamilyCandidate>,
        ipv6: Option<FamilyCandidate>,
    ) -> ResolutionSnapshot {
        ResolutionSnapshot::new(target.clone(), ResolutionMode::PreferIpv4Dual, ipv4, ipv6)
    }

    /// The reviewer's item 1: carrying a still-fresh address forward must not
    /// destroy the new generation's diagnostic for that same family. Both facts
    /// must stay observable.
    #[test]
    fn a_carried_forward_candidate_keeps_its_familys_new_diagnostic() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        // Generation 1: A (300s) and AAAA (900s) both succeed.
        let first = dual_snapshot(
            &target,
            Some(a_candidate(300, now)),
            Some(aaaa_candidate(900, now)),
        );
        let first = state.publish_generation(first, now);
        assert_eq!(
            first.generation(),
            1,
            "the first publication is generation 1"
        );

        // Generation 2: the A leg fails and the AAAA leg still succeeds. The
        // prior A is still fresh, so it is carried forward.
        clock.advance(100);
        let second_now = clock.now();
        let second = dual_snapshot(
            &target,
            Some(a_failure(ResolverError::BootstrapRcode(2))),
            Some(aaaa_candidate(900, second_now)),
        );
        let merged = state.publish_generation(second, second_now);

        // The carried address is the A candidate ...
        assert!(
            merged
                .candidate(AddressFamily::Ipv4)
                .is_some_and(|candidate| candidate.destination().is_some()),
            "the still-fresh prior A is carried forward"
        );
        // ... and the new generation's typed A failure is STILL observable.
        assert_eq!(
            merged.family_error(AddressFamily::Ipv4),
            Some(ResolverError::BootstrapRcode(2)),
            "the carried-forward address must not hide this generation's failure"
        );
        // The diagnostic list reports the A failure alongside the fresh AAAA.
        assert_eq!(
            merged.diagnostics(),
            vec![(AddressFamily::Ipv4, ResolverError::BootstrapRcode(2))]
        );
        // Selection still prefers the fresh A, which is the carried one.
        assert_eq!(
            merged.select(second_now).map(|d| d.family()),
            Some(AddressFamily::Ipv4)
        );
    }

    /// The reviewer's item 2: the merged snapshot is what a caller selects from,
    /// so a generation whose own legs produced nothing usable can still succeed
    /// through a carried forward sibling.
    #[test]
    fn a_generation_succeeds_through_the_merged_snapshot() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        // Generation 1: only AAAA succeeds; A fails.
        let first = dual_snapshot(
            &target,
            Some(a_failure(ResolverError::BootstrapRcode(3))),
            Some(aaaa_candidate(300, now)),
        );
        state.publish_generation(first, now);

        // Generation 2: A fails again and AAAA fails too, but the AAAA from
        // generation 1 is still fresh. Selecting from the *merged* snapshot is
        // what makes this generation serviceable.
        clock.advance(100);
        let second_now = clock.now();
        let second = dual_snapshot(
            &target,
            Some(a_failure(ResolverError::BootstrapRcode(3))),
            Some(a_failure(ResolverError::BootstrapTimeout)),
        );
        let pre_merge_selection = second.selected_target(second_now);
        assert!(
            pre_merge_selection.is_none(),
            "the unmerged snapshot alone has no usable address"
        );

        let merged = state.publish_generation(second, second_now);
        assert!(
            merged.selected_target(second_now).is_some(),
            "the merged snapshot is serviceable via the carried-forward AAAA"
        );
        assert_eq!(
            merged.select(second_now).map(|d| d.family()),
            Some(AddressFamily::Ipv6)
        );
        // And both of this generation's failures remain observable.
        assert_eq!(merged.diagnostics().len(), 2);
    }

    /// The reviewer's item 2, second half: an expired prior candidate must never
    /// be resurrected by a failed refresh.
    #[test]
    fn an_expired_prior_candidate_is_never_resurrected_by_a_failed_refresh() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        let first = dual_snapshot(
            &target,
            Some(a_candidate(300, now)),
            Some(aaaa_candidate(300, now)),
        );
        state.publish_generation(first, now);

        // Advance past both expiries, then run a generation where both fail.
        clock.advance(400);
        let later = clock.now();
        let second = dual_snapshot(
            &target,
            Some(a_failure(ResolverError::BootstrapTimeout)),
            Some(a_failure(ResolverError::BootstrapTimeout)),
        );
        let merged = state.publish_generation(second, later);

        assert!(
            merged.selected_target(later).is_none(),
            "an expired candidate must never be carried forward as fresh"
        );
        assert!(
            merged.committed_target().is_none(),
            "the expired addresses are not retained as candidates at all"
        );
        assert_eq!(
            merged.aggregate_error(),
            ResolverError::BootstrapTimeout,
            "the aggregate failure is reported instead"
        );
    }

    /// The reviewer's item 4: publication assigns monotonic generation metadata,
    /// and a superseded generation is still identifiable.
    #[test]
    fn publication_assigns_monotonic_generation_metadata() {
        let clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        assert_eq!(state.generation(), 0, "an empty state has no generation");

        let first = state.publish_generation(
            dual_snapshot(
                &target,
                Some(a_candidate(300, now)),
                Some(aaaa_candidate(300, now)),
            ),
            now,
        );
        assert_eq!(first.generation(), 1);
        assert_eq!(state.generation(), 1);

        let second = state.publish_generation(
            dual_snapshot(
                &target,
                Some(a_candidate(300, now)),
                Some(aaaa_candidate(300, now)),
            ),
            now,
        );
        assert_eq!(
            second.generation(),
            2,
            "each publication advances the ordinal"
        );
        assert_eq!(state.generation(), 2);

        // The superseded snapshot keeps its own ordinal, so a late holder can
        // tell that it is not looking at the current generation.
        assert_eq!(first.generation(), 1);
        assert_eq!(
            state.snapshot().expect("published").generation(),
            2,
            "the published snapshot carries the newest ordinal"
        );
        assert_ne!(
            first.generation(),
            state.generation(),
            "a superseded generation is distinguishable from the current one"
        );
    }

    /// The reviewer's item 5: a dual generation whose preferred A has expired
    /// while AAAA is still fresh must not be served as settled forever. The fast
    /// path has to become reachable again so a later caller can refresh A.
    #[test]
    fn a_fresh_sibling_does_not_mask_an_expired_preferred_family() {
        let clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        // A expires at 300s, AAAA is fresh for 900s.
        state.publish_generation(
            dual_snapshot(
                &target,
                Some(a_candidate(300, now)),
                Some(aaaa_candidate(900, now)),
            ),
            now,
        );

        // While both are fresh, the fast path serves the preferred A.
        let early = state
            .serve_fresh(now + Duration::from_secs(100))
            .expect("fresh A");
        assert_eq!(early.family(), AddressFamily::Ipv4);

        // Past the A expiry but inside the AAAA window the fast path is a MISS
        // rather than a silent permanent AAAA answer, so a refresh can happen.
        assert!(
            state.serve_fresh(now + Duration::from_secs(400)).is_none(),
            "an expired preferred family must make the fast path miss"
        );
        // The partially fresh generation is still published, not moved to the
        // expired slot: its fresh AAAA remains legitimately serviceable.
        let snapshot = state.snapshot().expect("still published");
        assert!(
            snapshot.select(now + Duration::from_secs(400)).is_some(),
            "the fresh AAAA remains available for selection"
        );
        assert!(
            state.last_expired().is_none(),
            "a partially fresh generation is not treated as expired"
        );

        // Once nothing is fresh at all, it does move to the diagnostic slot.
        assert!(state.serve_fresh(now + Duration::from_secs(1000)).is_none());
        assert!(state.last_expired().is_some());
    }

    /// Item 5 continued: after A expires, a later generation whose A leg fails
    /// still serves AAAA and keeps the A failure as a diagnostic.
    #[test]
    fn a_later_generation_can_refresh_a_and_still_serve_the_fresh_aaaa() {
        let mut clock = SteppingClock::new();
        let state = ResolverState::new();
        let target = dual_target();
        let now = clock.now();

        state.publish_generation(
            dual_snapshot(
                &target,
                Some(a_candidate(300, now)),
                Some(aaaa_candidate(900, now)),
            ),
            now,
        );

        // The A has expired but AAAA has not, so a refresh is attempted.
        clock.advance(400);
        let later = clock.now();
        let refresh = dual_snapshot(
            &target,
            Some(a_failure(ResolverError::BootstrapRcode(2))),
            Some(aaaa_candidate(900, later)),
        );
        let merged = state.publish_generation(refresh, later);

        // The expired A is NOT resurrected, the AAAA serves, and the A failure
        // is visible.
        assert_eq!(
            merged.select(later).map(|d| d.family()),
            Some(AddressFamily::Ipv6),
            "the expired A is not resurrected; the fresh AAAA serves"
        );
        assert_eq!(
            merged.family_error(AddressFamily::Ipv4),
            Some(ResolverError::BootstrapRcode(2))
        );
    }
}
