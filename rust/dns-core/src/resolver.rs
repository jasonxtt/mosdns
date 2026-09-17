//! Pure bootstrap DNS wire codec for endpoint resolution.
//!
//! This is Slice 0 of the endpoint-resolution foundation: the query encoder and
//! the response decoder the future resolver uses to turn a configured upstream
//! hostname into a numeric dial destination. It contains no sockets, timers,
//! cache, runtime, config, or production wiring — only wire bytes in and a
//! typed selection out.
//!
//! # Query contract
//!
//! [`build_resolver_query`] encodes exactly one question for a normalized
//! name, class IN, with RD set and one root-owned OPT record advertising
//! [`RESOLVER_UDP_PAYLOAD_SIZE`]. No DNSSEC OK bit, client subnet, padding, or
//! DO bit is added. The transaction ID comes from the injected
//! [`QueryIdSource`], so production can use an unpredictable source while
//! deterministic tests inject fixed IDs.
//!
//! # Response contract
//!
//! [`parse_resolver_response`] accepts a message only when it is a standard
//! QUERY response (`QR=1`, opcode 0) whose ID and question (name, type, class)
//! match the outstanding request, whose full RCODE is `NOERROR`, and whose TC
//! bit is clear. Correlation completes before any terminal matching-query error
//! is reported, so a reply for a different question is never classified as this
//! query's truncation or negative answer. Once correlated, a truncated response
//! is terminal immediately: it is not walked for records and not read for an
//! extended RCODE, so a cut body is reported as truncation rather than as a
//! malformed message or a negative answer.
//!
//! A selected address must belong to the question name or to a bounded,
//! loop-free CNAME path rooted at it. The path is resolved by walking the
//! retained records rather than by reading the answer section in order, so an
//! address listed before the CNAME that leads to it is still reachable; the
//! first reachable address in message order wins and the effective TTL is the
//! minimum over the path that reaches it, clamped by [`CnameChainPolicy`].
//!
//! The RCODE is the full 12-bit value: the header's low nibble extended by the
//! high 8 bits an OPT record carries in its TTL upper byte, so `BADVERS` (`16`)
//! is rejected rather than read as `NOERROR`. At most one OPT is accepted, only
//! in the additional section, and its owner must be the DNS root; a non-root
//! `TYPE 41` record is malformed and never contributes an extended RCODE. An
//! OPT record contributes no address and no TTL.
//!
//! Every record of every declared section is walked within the packet, so a
//! malformed tail fails the whole message rather than publishing a selection
//! from a partially validated packet. Trailing bytes after the last declared
//! record and unrelated authority/additional records are tolerated; they never
//! create a selection.
//!
//! # Reuse
//!
//! [`crate::query::parse_query`] remains the public query-shape validator and is
//! what callers use to check the encoder's output. The response side cannot
//! reuse [`crate::query::parse_question`]: although its bounds contract matches,
//! it returns the question name exactly as written, so a compressed question
//! name would not be expanded and could not be compared case-insensitively
//! against the expected name. [`read_name`] follows the same label and
//! compression-pointer budget rules and returns an expanded, case-folded wire
//! name. No existing `dns-core` API is changed or weakened.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const DNS_HEADER_LEN: usize = 12;
const RR_FIXED_LEN: usize = 10;
const OPCODE_QUERY: u8 = 0;
const CLASS_IN: u16 = 1;
const TYPE_A: u16 = 1;
const TYPE_CNAME: u16 = 5;
const TYPE_AAAA: u16 = 28;
const TYPE_OPT: u16 = 41;

/// The advertised EDNS(0) UDP payload size for bootstrap queries.
pub const RESOLVER_UDP_PAYLOAD_SIZE: u16 = 1200;

/// The default maximum number of CNAME links accepted in one response.
pub const RESOLVER_DEFAULT_MAX_CNAME_LINKS: u8 = 8;

/// The hard upper bound for [`CnameChainPolicy`]: a policy must stay strictly
/// below it, so the accepted link count can never reach the pointer-budget or
/// record-count regimes that make cycle proofing ambiguous.
pub const RESOLVER_MAX_CNAME_LINKS: u8 = 15;

/// The address family one bootstrap query asks for.
///
/// This is deliberately an enum rather than a boolean so that a later
/// dual-stack policy can extend the public API additively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    /// The DNS RR type this family selects.
    #[must_use]
    pub const fn wire_type(self) -> u16 {
        match self {
            Self::Ipv4 => TYPE_A,
            Self::Ipv6 => TYPE_AAAA,
        }
    }

    /// The exact RDATA length of this family's address record.
    #[must_use]
    pub const fn rdata_len(self) -> usize {
        match self {
            Self::Ipv4 => 4,
            Self::Ipv6 => 16,
        }
    }
}

/// The injected source of DNS transaction IDs.
///
/// Production supplies an unpredictable source; tests supply a deterministic
/// one. The codec never reads the operating system itself.
pub trait QueryIdSource {
    /// Returns the transaction ID for the next query.
    fn next_id(&mut self) -> u16;
}

/// A validated CNAME/answer selection policy.
///
/// The default keeps the existing product-compatible five-minute positive floor
/// and bounds long TTLs at seven days, following RFC 1035's guidance that a
/// resolver may limit excessively long TTLs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CnameChainPolicy {
    max_cname_links: u8,
    min_ttl: u32,
    max_ttl: u32,
}

impl CnameChainPolicy {
    /// Builds a policy, rejecting unusable bounds.
    ///
    /// # Errors
    ///
    /// Returns [`ResolverWireError::InvalidPolicy`] when `max_cname_links` is
    /// zero or at least [`RESOLVER_MAX_CNAME_LINKS`], when `min_ttl` is zero,
    /// or when `min_ttl` exceeds `max_ttl`.
    pub fn new(max_cname_links: u8, min_ttl: u32, max_ttl: u32) -> Result<Self, ResolverWireError> {
        if max_cname_links == 0 || max_cname_links >= RESOLVER_MAX_CNAME_LINKS {
            return Err(ResolverWireError::InvalidPolicy);
        }
        if min_ttl == 0 || min_ttl > max_ttl {
            return Err(ResolverWireError::InvalidPolicy);
        }
        Ok(Self {
            max_cname_links,
            min_ttl,
            max_ttl,
        })
    }

    /// The maximum number of CNAME links accepted in one response.
    #[must_use]
    pub const fn max_cname_links(&self) -> u8 {
        self.max_cname_links
    }

    /// The lower clamp applied to the effective TTL.
    #[must_use]
    pub const fn min_ttl(&self) -> u32 {
        self.min_ttl
    }

    /// The upper clamp applied to the effective TTL.
    #[must_use]
    pub const fn max_ttl(&self) -> u32 {
        self.max_ttl
    }

    /// Applies the explicit clamp to an observed TTL.
    #[must_use]
    pub const fn clamp_ttl(&self, ttl: u32) -> u32 {
        if ttl < self.min_ttl {
            self.min_ttl
        } else if ttl > self.max_ttl {
            self.max_ttl
        } else {
            ttl
        }
    }
}

impl Default for CnameChainPolicy {
    fn default() -> Self {
        Self {
            max_cname_links: RESOLVER_DEFAULT_MAX_CNAME_LINKS,
            min_ttl: 300,
            max_ttl: 604_800,
        }
    }
}

/// The validated address selected from a bootstrap response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedAddress {
    /// The numeric address the caller may dial.
    pub address: IpAddr,
    /// The family the address was selected for.
    pub family: AddressFamily,
    /// The effective lifetime in seconds, after the explicit clamp.
    pub ttl: u32,
    /// The number of CNAME links used to reach the selected record.
    pub cname_chain_len: u8,
}

/// A query-construction or response-validation failure.
///
/// Every variant is a terminal classification: the codec never returns a
/// partial selection, and no failure becomes a dial attempt by itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverWireError {
    /// The name is empty, over 253 octets, or not plain hostname syntax.
    InvalidName,
    /// The policy bounds are unusable.
    InvalidPolicy,
    /// The message is truncated, has an illegal name encoding, a compression
    /// pointer outside the packet or in a loop, an inexact address RDATA
    /// length, or a declared section count the packet cannot satisfy.
    Malformed,
    /// The message's QR bit is clear, so it is not a response.
    NotAResponse,
    /// The response opcode is not the standard QUERY opcode.
    UnexpectedOpcode(u8),
    /// The response ID does not match the outstanding query.
    MismatchedId,
    /// The response is truncated; this codec performs no TCP fallback.
    Truncated,
    /// The response RCODE is not `NOERROR`. The value is the full 12-bit
    /// RCODE: the header's low nibble extended by the high 8 bits an OPT
    /// record carries in its TTL upper byte (for example BADVERS is `16`).
    /// Negative answers are never cached.
    Rcode(u16),
    /// The echoed question does not match the expected name, type, and class.
    QuestionMismatch,
    /// The CNAME chain loops, exceeds the policy bound, or is otherwise
    /// unusable for reaching an address of the requested family.
    InvalidCnameChain,
    /// The message is well formed and correlated but carries no usable address
    /// of the requested family (empty answer, no data, or a chain that never
    /// reaches an address).
    NoUsableAnswer,
}

/// Encodes one bootstrap query for `name`.
///
/// The name must be plain ASCII hostname syntax; one optional trailing root dot
/// is accepted and normalized away. The question is class IN with the type
/// selected by `family`, RD is set, and one root-owned OPT record advertises
/// [`RESOLVER_UDP_PAYLOAD_SIZE`] with no options.
///
/// # Errors
///
/// Returns [`ResolverWireError::InvalidName`] when the name is empty, longer
/// than 253 octets, or contains an illegal label. The injected ID source cannot
/// fail.
pub fn build_resolver_query(
    name: &str,
    family: AddressFamily,
    ids: &mut impl QueryIdSource,
) -> Result<Vec<u8>, ResolverWireError> {
    let normalized = name.strip_suffix('.').unwrap_or(name);
    validate_name(normalized)?;

    let mut wire = Vec::with_capacity(DNS_HEADER_LEN + normalized.len() + 6 + 11);
    wire.extend_from_slice(&ids.next_id().to_be_bytes());
    wire.extend_from_slice(&0x0100u16.to_be_bytes()); // RD set, QR clear, QUERY
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&1u16.to_be_bytes()); // ARCOUNT: the OPT record
    for label in normalized.split('.') {
        wire.push(u8::try_from(label.len()).map_err(|_| ResolverWireError::InvalidName)?);
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire.extend_from_slice(&family.wire_type().to_be_bytes());
    wire.extend_from_slice(&CLASS_IN.to_be_bytes());
    wire.push(0); // OPT owner: root
    wire.extend_from_slice(&41u16.to_be_bytes()); // OPT type
    wire.extend_from_slice(&RESOLVER_UDP_PAYLOAD_SIZE.to_be_bytes());
    wire.extend_from_slice(&0u32.to_be_bytes()); // extended rcode/version/flags
    wire.extend_from_slice(&0u16.to_be_bytes()); // no options
    Ok(wire)
}

/// Validates and selects one address from a bootstrap response.
///
/// `qname_wire` is the expected question name in uncompressed wire label form
/// (for example the `qname_wire` of [`crate::query::parse_query`] applied to the
/// query built by [`build_resolver_query`]). `expected_id` is the transaction ID
/// sent with that query.
///
/// # Errors
///
/// Returns the [`ResolverWireError`] variant describing the first failed
/// correlation, framing, or selection rule. No partial selection is produced on
/// failure.
pub fn parse_resolver_response(
    packet: &[u8],
    family: AddressFamily,
    qname_wire: &[u8],
    expected_id: u16,
    policy: &CnameChainPolicy,
) -> Result<SelectedAddress, ResolverWireError> {
    let header = packet
        .get(..DNS_HEADER_LEN)
        .ok_or(ResolverWireError::Malformed)?;
    let flags = u16::from_be_bytes([header[2], header[3]]);
    if flags & 0x8000 == 0 {
        return Err(ResolverWireError::NotAResponse);
    }
    let opcode = ((flags >> 11) & 0x0f) as u8;
    if opcode != OPCODE_QUERY {
        return Err(ResolverWireError::UnexpectedOpcode(opcode));
    }
    if u16::from_be_bytes([header[0], header[1]]) != expected_id {
        return Err(ResolverWireError::MismatchedId);
    }

    let qdcount = u16::from_be_bytes([header[4], header[5]]);
    let ancount = usize::from(u16::from_be_bytes([header[6], header[7]]));
    let nscount = usize::from(u16::from_be_bytes([header[8], header[9]]));
    let arcount = usize::from(u16::from_be_bytes([header[10], header[11]]));

    // Correlation runs to completion before any terminal matching-query error
    // (TC or RCODE) is reported, so a stale or off-path reply can never be
    // classified as this query's truncation or negative answer.
    if qdcount != 1 {
        return Err(ResolverWireError::QuestionMismatch);
    }
    let expected = fold(qname_wire);
    let (question_end, question_name) = read_name(packet, DNS_HEADER_LEN)?;
    let question_fields = packet
        .get(question_end..question_end + 4)
        .ok_or(ResolverWireError::Malformed)?;
    let qtype = u16::from_be_bytes([question_fields[0], question_fields[1]]);
    let qclass = u16::from_be_bytes([question_fields[2], question_fields[3]]);
    if question_name != expected || qtype != family.wire_type() || qclass != CLASS_IN {
        return Err(ResolverWireError::QuestionMismatch);
    }

    // A correlated truncated response is terminal immediately: the delivered
    // body is explicitly incomplete, so it is neither walked for records nor
    // read for an extended RCODE, and a cut or hostile body cannot be reported
    // as a different failure. This codec performs no TCP retry.
    if flags & 0x0200 != 0 {
        return Err(ResolverWireError::Truncated);
    }

    // Walk every declared record once, retaining only the framing facts the
    // selection needs. The walk is order-independent, so an answer section that
    // lists an address before the CNAME leading to it is still resolved.
    let mut links: Vec<ChainLink> = Vec::new();
    let mut addresses: Vec<AddressRecord> = Vec::new();
    let mut position = question_end + 4;

    for _ in 0..ancount {
        let (owner_end, owner) = read_name(packet, position)?;
        let fixed_end = owner_end
            .checked_add(RR_FIXED_LEN)
            .ok_or(ResolverWireError::Malformed)?;
        let fixed = packet
            .get(owner_end..fixed_end)
            .ok_or(ResolverWireError::Malformed)?;
        let rrtype = u16::from_be_bytes([fixed[0], fixed[1]]);
        let rclass = u16::from_be_bytes([fixed[2], fixed[3]]);
        let ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
        let rdlength = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
        let rdata_end = fixed_end
            .checked_add(rdlength)
            .ok_or(ResolverWireError::Malformed)?;
        let rdata = packet
            .get(fixed_end..rdata_end)
            .ok_or(ResolverWireError::Malformed)?;

        if rclass == CLASS_IN {
            if rrtype == family.wire_type() {
                if rdata.len() != family.rdata_len() {
                    return Err(ResolverWireError::Malformed);
                }
                addresses.push(AddressRecord {
                    owner,
                    offset: position,
                    ttl,
                    address: decode_address(family, rdata)?,
                });
            } else if rrtype == TYPE_CNAME {
                let (target_end, target) = read_name(packet, fixed_end)?;
                if target_end != rdata_end {
                    return Err(ResolverWireError::InvalidCnameChain);
                }
                links.push(ChainLink { owner, target, ttl });
            }
        }

        position = rdata_end;
    }

    // Walk the remaining sections for framing, and read the EDNS(0) OPT record
    // so the full 12-bit RCODE is available. A response may carry at most one
    // OPT, only in the additional section, and its owner must be the DNS root
    // (`[0]`) — a non-root TYPE 41 record is not OPT and is rejected rather
    // than being read for an extended RCODE.
    let mut opt: Option<u8> = None;
    for index in 0..nscount.saturating_add(arcount) {
        let (owner_end, owner) = read_name(packet, position)?;
        let fixed_end = owner_end
            .checked_add(RR_FIXED_LEN)
            .ok_or(ResolverWireError::Malformed)?;
        let fixed = packet
            .get(owner_end..fixed_end)
            .ok_or(ResolverWireError::Malformed)?;
        let rrtype = u16::from_be_bytes([fixed[0], fixed[1]]);
        let rdlength = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
        if rrtype == TYPE_OPT {
            if index < nscount || !is_root_name(&owner) || opt.is_some() {
                return Err(ResolverWireError::Malformed);
            }
            opt = Some(fixed[4]);
        }
        position = fixed_end
            .checked_add(rdlength)
            .ok_or(ResolverWireError::Malformed)?;
        if position > packet.len() {
            return Err(ResolverWireError::Malformed);
        }
    }

    let extended = opt.map_or(0u16, u16::from);
    let rcode = u16::from((flags & 0x0f) as u8) | (extended << 4);
    if rcode != 0 {
        return Err(ResolverWireError::Rcode(rcode));
    }

    let (address, observed_ttl, links_used) =
        select_address(&links, &addresses, &expected, policy)?;
    Ok(SelectedAddress {
        address,
        family,
        ttl: policy.clamp_ttl(observed_ttl),
        cname_chain_len: links_used,
    })
}

/// One CNAME record retained from the answer section.
struct ChainLink {
    owner: Vec<u8>,
    target: Vec<u8>,
    ttl: u32,
}

/// One address record retained from the answer section.
struct AddressRecord {
    owner: Vec<u8>,
    /// The record's own offset in the message, the wire-order key.
    offset: usize,
    ttl: u32,
    address: IpAddr,
}

/// Resolves the CNAME path rooted at `root` and returns the first reachable
/// address in answer wire order, the minimum TTL over the path that reaches it,
/// and the number of CNAME links on that path.
///
/// The traversal is an explicit iterative walk over the retained records, so the
/// result never depends on the order the records appear in the message: an
/// address listed before the CNAME that leads to it is still reachable. Every
/// CNAME whose owner is the current name is followed, so a branching answer
/// section is resolved deterministically by answering "which reachable address
/// comes first in the message" rather than by which branch was discovered
/// first. A name already on the current path closes a cycle, a path longer than
/// the policy allows is rejected, and the walk has a hard state budget so a
/// pathological answer section cannot make it super-linear.
///
/// Addresses not reachable from `root` are ignored; a message with no reachable
/// address of the requested family is [`ResolverWireError::NoUsableAnswer`].
fn select_address(
    links: &[ChainLink],
    addresses: &[AddressRecord],
    root: &[u8],
    policy: &CnameChainPolicy,
) -> Result<(IpAddr, u32, u8), ResolverWireError> {
    // A crafted answer section can contain exponentially many distinct acyclic
    // CNAME paths, so the walk carries a hard state budget and fails closed
    // rather than resolving unbounded work. A genuine authoritative answer has
    // far fewer links than this bound.
    let mut budget = links
        .len()
        .saturating_add(1)
        .saturating_mul(usize::from(policy.max_cname_links()) + 1)
        .saturating_add(1);

    // Paths still to expand: the names visited so far, the running TTL minimum,
    // and the link count. The visited-name list is the cycle check and is
    // independent of the order records appeared in.
    let mut frontier: Vec<(Vec<Vec<u8>>, u32, u8)> = vec![(vec![root.to_vec()], u32::MAX, 0)];
    let mut best: Option<(&AddressRecord, u32, u8)> = None;

    while !frontier.is_empty() {
        let level = std::mem::take(&mut frontier);
        for (path, ttl_so_far, depth) in level {
            budget = budget
                .checked_sub(1)
                .ok_or(ResolverWireError::InvalidCnameChain)?;
            let current = path.last().ok_or(ResolverWireError::Malformed)?;

            for record in addresses.iter().filter(|record| &record.owner == current) {
                let effective = ttl_so_far.min(record.ttl);
                best = Some(match best {
                    None => (record, effective, depth),
                    Some((chosen, chosen_ttl, chosen_depth)) => {
                        // First in message order wins; ties break on the smaller
                        // TTL and then the shorter path, so the result is total.
                        let candidate = (record.offset, effective, depth);
                        let incumbent = (chosen.offset, chosen_ttl, chosen_depth);
                        if candidate < incumbent {
                            (record, effective, depth)
                        } else {
                            (chosen, chosen_ttl, chosen_depth)
                        }
                    }
                });
            }

            for link in links.iter().filter(|link| &link.owner == current) {
                if path.contains(&link.target) {
                    return Err(ResolverWireError::InvalidCnameChain);
                }
                if depth >= policy.max_cname_links() {
                    return Err(ResolverWireError::InvalidCnameChain);
                }
                let mut extended = path.clone();
                extended.push(link.target.clone());
                frontier.push((extended, ttl_so_far.min(link.ttl), depth + 1));
            }
        }
    }

    match best {
        Some((record, ttl, depth)) => Ok((record.address, ttl, depth)),
        None => Err(ResolverWireError::NoUsableAnswer),
    }
}

/// Rejects names that are not plain ASCII hostname syntax.
fn validate_name(name: &str) -> Result<(), ResolverWireError> {
    if name.is_empty() || name.len() > 253 {
        return Err(ResolverWireError::InvalidName);
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
            return Err(ResolverWireError::InvalidName);
        }
    }
    Ok(())
}

/// Whether an expanded wire name is the DNS root: the single zero octet.
///
/// [`read_name`] always returns a terminated name whose last octet is the root
/// label, so a bare `[0]` is the only root encoding it can produce.
fn is_root_name(owner: &[u8]) -> bool {
    owner == [0]
}

/// Lowercases the ASCII letters of an uncompressed wire name.
///
/// Length octets are at most 63 and the root terminator is zero, so they are
/// never mistaken for ASCII letters.
fn fold(qname_wire: &[u8]) -> Vec<u8> {
    qname_wire.iter().map(u8::to_ascii_lowercase).collect()
}

/// Reads one wire name at `offset`, returning the offset just past the name as
/// written and its expanded, ASCII-folded wire form.
///
/// Labels must be at most 63 octets, the expanded name must stay within the
/// 255-octet DNS limit, and compression pointers are followed with a bounded
/// budget; an out-of-packet target, a self-pointer, or a pointer cycle fails.
fn read_name(packet: &[u8], offset: usize) -> Result<(usize, Vec<u8>), ResolverWireError> {
    const MAX_NAME_OCTETS: usize = 255;
    const MAX_POINTERS: usize = 255_usize.div_ceil(2) - 2;

    let mut name: Vec<u8> = Vec::with_capacity(32);
    let mut budget = MAX_NAME_OCTETS;
    let mut position = offset;
    let mut written_end: Option<usize> = None;
    let mut pointers = 0usize;

    loop {
        let label = *packet.get(position).ok_or(ResolverWireError::Malformed)?;
        match label & 0xc0 {
            0 => {
                if label == 0 {
                    name.push(0);
                    // A followed pointer already ends the name as written; only
                    // a root label needs its own byte counted.
                    let end = match written_end {
                        Some(end) => end,
                        None => position
                            .checked_add(1)
                            .ok_or(ResolverWireError::Malformed)?,
                    };
                    return Ok((end, name));
                }
                let length = usize::from(label);
                let label_start = position
                    .checked_add(1)
                    .ok_or(ResolverWireError::Malformed)?;
                let label_end = label_start
                    .checked_add(length)
                    .ok_or(ResolverWireError::Malformed)?;
                let bytes = packet
                    .get(label_start..label_end)
                    .ok_or(ResolverWireError::Malformed)?;
                if budget <= length + 1 {
                    return Err(ResolverWireError::Malformed);
                }
                budget -= length + 1;
                name.push(label);
                name.extend(bytes.iter().map(u8::to_ascii_lowercase));
                position = label_end;
            }
            0xc0 => {
                if pointers == MAX_POINTERS {
                    return Err(ResolverWireError::Malformed);
                }
                let second = *packet
                    .get(position + 1)
                    .ok_or(ResolverWireError::Malformed)?;
                let target = usize::from(label & 0x3f) << 8 | usize::from(second);
                if target >= packet.len() || target == position {
                    return Err(ResolverWireError::Malformed);
                }
                if written_end.is_none() {
                    written_end = position.checked_add(2);
                }
                pointers += 1;
                position = target;
            }
            _ => return Err(ResolverWireError::Malformed),
        }
    }
}

/// Decodes an exact-width address record.
fn decode_address(family: AddressFamily, rdata: &[u8]) -> Result<IpAddr, ResolverWireError> {
    match family {
        AddressFamily::Ipv4 => {
            let octets: [u8; 4] = rdata.try_into().map_err(|_| ResolverWireError::Malformed)?;
            Ok(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        AddressFamily::Ipv6 => {
            let octets: [u8; 16] = rdata.try_into().map_err(|_| ResolverWireError::Malformed)?;
            Ok(IpAddr::V6(Ipv6Addr::from(octets)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AddressFamily, CnameChainPolicy, RESOLVER_DEFAULT_MAX_CNAME_LINKS,
        RESOLVER_MAX_CNAME_LINKS, ResolverWireError, build_resolver_query,
    };

    struct FixedIds(u16);

    impl super::QueryIdSource for FixedIds {
        fn next_id(&mut self) -> u16 {
            self.0
        }
    }

    #[test]
    fn accepts_the_full_default_policy() {
        let policy = CnameChainPolicy::default();
        assert_eq!(policy.max_cname_links(), RESOLVER_DEFAULT_MAX_CNAME_LINKS);
        assert_eq!(policy.min_ttl(), 300);
        assert_eq!(policy.max_ttl(), 604_800);
        assert_eq!(policy.clamp_ttl(1), 300);
        assert_eq!(policy.clamp_ttl(600), 600);
        assert_eq!(policy.clamp_ttl(u32::MAX), 604_800);
    }

    #[test]
    fn policy_bounds_are_rejected() {
        for (links, min, max) in [
            (0u8, 300u32, 604_800u32),
            (RESOLVER_MAX_CNAME_LINKS, 300, 604_800),
            (1, 0, 604_800),
            (1, 600, 300),
        ] {
            assert_eq!(
                CnameChainPolicy::new(links, min, max),
                Err(ResolverWireError::InvalidPolicy),
                "links={links} min={min} max={max}"
            );
        }
    }

    #[test]
    fn family_selects_the_expected_wire_record() {
        assert_eq!(AddressFamily::Ipv4.wire_type(), 1);
        assert_eq!(AddressFamily::Ipv4.rdata_len(), 4);
        assert_eq!(AddressFamily::Ipv6.wire_type(), 28);
        assert_eq!(AddressFamily::Ipv6.rdata_len(), 16);
    }

    #[test]
    fn rejects_names_outside_plain_hostname_syntax() {
        for bad in ["", "example..org", "-a.org", "a-.org", "a b.org", "ü.org"] {
            assert_eq!(
                build_resolver_query(bad, AddressFamily::Ipv4, &mut FixedIds(1)),
                Err(ResolverWireError::InvalidName),
                "{bad:?}"
            );
        }
        assert_eq!(
            build_resolver_query(&"a".repeat(64), AddressFamily::Ipv4, &mut FixedIds(1)),
            Err(ResolverWireError::InvalidName)
        );
        assert_eq!(
            build_resolver_query(&"a".repeat(254), AddressFamily::Ipv4, &mut FixedIds(1)),
            Err(ResolverWireError::InvalidName)
        );
    }
}
