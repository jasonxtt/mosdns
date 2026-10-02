//! Immutable startup response-policy descriptors; no request-time file I/O.
use crate::config::ConfigError;
use mosdns_matcher_core::{IpPrefixList, MixMatcher, normalize};
use mosdns_sequence_core::{
    ExecutableId, ExecutionState, ExecutorError, MatchOutcome, Matcher, MatcherError,
};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::path::PathBuf;
use std::rc::Rc;

pub const POLICY_LINE_LIMIT: usize = 64 * 1024;
pub const POLICY_BYTES_LIMIT: usize = 64 * 1024 * 1024;
pub const POLICY_RULE_LIMIT: usize = 1_000_000;

pub struct DomainPayload<T> {
    matcher: MixMatcher<usize>,
    values: Vec<T>,
}
impl<T> DomainPayload<T> {
    #[must_use]
    pub fn lookup(&self, domain: &str) -> Option<&T> {
        self.matcher
            .r#match(domain)
            .map(|index| &self.values[*index])
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostAddresses {
    pub ipv4: Vec<std::net::Ipv4Addr>,
    pub ipv6: Vec<std::net::Ipv6Addr>,
}
pub enum ResponsePolicy {
    Hosts(Rc<DomainPayload<HostAddresses>>),
    Redirect(Rc<DomainPayload<String>>),
    Ttl(TtlPolicy),
}
pub struct ResponsePolicyConfig {
    pub tag: String,
    pub executable: ExecutableId,
    pub policy: ResponsePolicy,
}
pub struct IpSetConfig {
    pub tag: String,
    pub prefixes: Rc<IpPrefixList>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtlPolicy {
    Fixed(u32),
    Range { min: u32, max: u32 },
}
impl TtlPolicy {
    pub(crate) fn parse(args: &str, path: &str) -> Result<Self, ConfigError> {
        let number = |n: &str| {
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ConfigError::new(
                    path,
                    "ttl requires unsigned decimal uint32",
                ));
            }
            n.parse::<u32>()
                .map_err(|_| ConfigError::new(path, "ttl requires uint32 fixed or min-max"))
        };
        if args.split_whitespace().count() != 1 {
            return Err(ConfigError::new(
                path,
                "ttl requires exactly one fixed or min-max argument",
            ));
        }
        match args.split_once('-') {
            Some((min, max)) => Ok(Self::Range {
                min: number(min)?,
                max: number(max)?,
            }),
            None => Ok(Self::Fixed(number(args)?)),
        }
    }
}

/// Budgets count all source bytes and non-comment rules, including overwritten rules.
#[derive(Default)]
pub(crate) struct RuleBudget {
    bytes: usize,
    rules: usize,
}
impl RuleBudget {
    pub(crate) fn load(
        &mut self,
        inline: &[String],
        files: &[PathBuf],
        missing_ok: bool,
        path: &str,
        mut consume: impl FnMut(&str, &str) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        for (index, line) in inline.iter().enumerate() {
            self.line(line.as_bytes(), &format!("{path}[{index}]"), &mut consume)?;
        }
        for file in files {
            let input = match std::fs::File::open(file) {
                Ok(file) => file,
                Err(error) if missing_ok && error.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("{}: missing IP rule file, skipped", file.display());
                    continue;
                }
                Err(error) => {
                    return Err(ConfigError::new(
                        file.display().to_string(),
                        format!("cannot read rules: {error}"),
                    ));
                }
            };
            let mut reader = BufReader::new(input);
            let mut index = 0;
            loop {
                // A bounded read prevents a single hostile line allocating the whole file.
                let mut line = Vec::new();
                let read = reader
                    .by_ref()
                    .take((POLICY_LINE_LIMIT + 2) as u64)
                    .read_until(b'\n', &mut line)
                    .map_err(|error| {
                        ConfigError::new(
                            file.display().to_string(),
                            format!("cannot read rules: {error}"),
                        )
                    })?;
                if read == 0 {
                    break;
                }
                index += 1;
                self.line(&line, &format!("{}:{index}", file.display()), &mut consume)?;
            }
        }
        Ok(())
    }
    fn line(
        &mut self,
        bytes: &[u8],
        path: &str,
        consume: &mut impl FnMut(&str, &str) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| ConfigError::new(path, "policy byte limit exceeded"))?;
        if self.bytes > POLICY_BYTES_LIMIT {
            return Err(ConfigError::new(path, "policy byte limit exceeded"));
        }
        let content = bytes
            .strip_suffix(b"\n")
            .unwrap_or(bytes)
            .strip_suffix(b"\r")
            .unwrap_or(bytes.strip_suffix(b"\n").unwrap_or(bytes));
        if content.len() > POLICY_LINE_LIMIT {
            return Err(ConfigError::new(path, "policy line limit exceeded"));
        }
        if bytes.contains(&0)
            || bytes.starts_with(b"SRS")
            || bytes.starts_with(b"\x1f\x8b")
            || bytes.starts_with(b"\x78\x01")
            || bytes.starts_with(b"\x78\x9c")
            || bytes.starts_with(b"\x78\xda")
        {
            return Err(ConfigError::new(
                path,
                "unsupported binary/compressed rule format",
            ));
        }
        let text = std::str::from_utf8(content)
            .map_err(|_| ConfigError::new(path, "rules must be UTF-8 text"))?;
        let text = text.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            return Ok(());
        }
        self.rules += 1;
        if self.rules > POLICY_RULE_LIMIT {
            return Err(ConfigError::new(path, "policy rule limit exceeded"));
        }
        consume(text, path)
    }
}

pub(crate) fn domain_payload<T>(
    inline: &[String],
    files: &[PathBuf],
    path: &str,
    parse: impl Fn(&[&str], &str) -> Result<T, ConfigError>,
) -> Result<DomainPayload<T>, ConfigError> {
    let mut entries: Vec<(String, T)> = Vec::new();
    let mut positions = BTreeMap::new();
    RuleBudget::default().load(inline, files, false, path, |line, location| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let (kind, pattern) = fields[0].split_once(':').unwrap_or(("full", fields[0]));
        let pattern = if kind == "regexp" {
            pattern.to_owned()
        } else {
            normalize(pattern)
        };
        let canonical = format!("{kind}:{pattern}");
        // Validate each rule even if a later duplicate would replace it.
        let mut validator = MixMatcher::new();
        validator
            .add(&canonical, ())
            .map_err(|error| ConfigError::new(location, error.to_string()))?;
        let value = parse(&fields[1..], location)?;
        if let Some(index) = positions.get(&canonical).copied() {
            entries[index] = (canonical, value);
        } else {
            positions.insert(canonical.clone(), entries.len());
            entries.push((canonical, value));
        }
        Ok(())
    })?;
    let mut matcher = MixMatcher::new();
    let mut values = Vec::with_capacity(entries.len());
    for (rule, value) in entries {
        matcher
            .add(&rule, values.len())
            .map_err(|error| ConfigError::new(path, error.to_string()))?;
        values.push(value);
    }
    Ok(DomainPayload { matcher, values })
}
pub(crate) fn hosts(
    inline: &[String],
    files: &[PathBuf],
    path: &str,
) -> Result<DomainPayload<HostAddresses>, ConfigError> {
    domain_payload(inline, files, path, |fields, location| {
        let mut result = HostAddresses {
            ipv4: Vec::new(),
            ipv6: Vec::new(),
        };
        for field in fields {
            match field.parse::<IpAddr>().map_err(|_| {
                ConfigError::new(location, format!("invalid hosts address `{field}`"))
            })? {
                IpAddr::V4(ip) => result.ipv4.push(ip),
                IpAddr::V6(ip) => result.ipv6.push(ip),
            }
        }
        Ok(result)
    })
}
pub(crate) fn redirects(
    inline: &[String],
    files: &[PathBuf],
    path: &str,
) -> Result<DomainPayload<String>, ConfigError> {
    domain_payload(inline, files, path, |fields, location| {
        if fields.len() != 1 {
            return Err(ConfigError::new(
                location,
                "redirect rule requires exactly two fields",
            ));
        }
        let mut name = hickory_proto::rr::Name::from_ascii(fields[0]).map_err(|error| {
            ConfigError::new(location, format!("invalid redirect target: {error}"))
        })?;
        name.set_fqdn(true);
        Ok(name.to_ascii())
    })
}
pub(crate) fn ip_prefix(field: &str, path: &str) -> Result<(IpAddr, u8), ConfigError> {
    let (address, bits) = match field.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (field, None),
    };
    let address = address
        .parse::<IpAddr>()
        .map_err(|_| ConfigError::new(path, format!("invalid IP/prefix `{field}`")))?;
    let width = if address.is_ipv4() { 32 } else { 128 };
    let bits = bits
        .map(|b| b.parse::<u8>())
        .transpose()
        .map_err(|_| ConfigError::new(path, "invalid prefix width"))?
        .unwrap_or(width);
    if bits > width {
        return Err(ConfigError::new(path, "invalid prefix width"));
    }
    Ok((address, bits))
}
pub(crate) fn ip_list(
    inline: &[String],
    files: &[PathBuf],
    path: &str,
) -> Result<Rc<IpPrefixList>, ConfigError> {
    for (index, entry) in inline.iter().enumerate() {
        ip_prefix(entry, &format!("{path}.ips[{index}]"))?;
    }
    let mut prefixes = IpPrefixList::new();
    RuleBudget::default().load(inline, files, true, path, |line, location| {
        let (ip, bits) = ip_prefix(line.split_whitespace().next().unwrap_or(""), location)?;
        prefixes.append(ip, bits);
        Ok(())
    })?;
    prefixes.rebuild();
    Ok(Rc::new(prefixes))
}
/// S1 validates new IP expressions but defers their runtime evaluation to S4.
/// The existing single-IPv4 path remains unchanged until then.
pub(crate) struct PendingIpMatcher;
impl Matcher for PendingIpMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Err(MatcherError::new(
            "response IP policy runtime is not implemented yet",
        ))
    }
}

/// Applies one non-scoped policy atomically. True means hosts replaced the
/// response; TTL transforms preserve the existing supplier identity.
pub(crate) fn apply_wire_policy(
    policy: &ResponsePolicy,
    state: &mut ExecutionState,
    query_wire: &[u8],
) -> Result<bool, ExecutorError> {
    match policy {
        ResponsePolicy::Hosts(rules) => {
            let q = &state.query.question;
            if q.qclass != 1 || !matches!(q.qtype, 1 | 28) {
                return Ok(false);
            }
            let Some(domain) = crate::matchers::wire_name_to_ascii_domain(&q.qname_wire) else {
                return Ok(false);
            };
            let Some(addresses) = rules.lookup(&domain) else {
                return Ok(false);
            };
            if addresses.ipv4.is_empty() && addresses.ipv6.is_empty() {
                return Ok(false);
            }
            let data: Vec<Vec<u8>> = if q.qtype == 1 {
                addresses
                    .ipv4
                    .iter()
                    .map(|ip| ip.octets().to_vec())
                    .collect()
            } else {
                addresses
                    .ipv6
                    .iter()
                    .map(|ip| ip.octets().to_vec())
                    .collect()
            };
            let count = u16::try_from(data.len())
                .map_err(|_| ExecutorError::new("too many hosts addresses"))?;
            let mut wire = mosdns_dns_core::synthesize_response(&state.query.header, q, 0)
                .map_err(|_| ExecutorError::new("cannot construct hosts response"))?;
            wire[2] |= query_wire.get(2).copied().unwrap_or(0) & 1;
            wire[6..8].copy_from_slice(&count.to_be_bytes());
            if data.is_empty() {
                wire[8..10].copy_from_slice(&1_u16.to_be_bytes());
                let mut soa = encode_name("fake-ns.mosdns.fake.root.");
                soa.extend(encode_name("fake-mbox.mosdns.fake.root."));
                for field in [2_021_110_400_u32, 1800, 900, 604800, 86400] {
                    soa.extend(field.to_be_bytes());
                }
                append_record(&mut wire, &q.qname_wire, 6, 300, &soa)?;
            } else {
                for address in data {
                    append_record(&mut wire, &q.qname_wire, q.qtype, 10, &address)?;
                }
            }
            if wire.len() > 65535 {
                return Err(ExecutorError::new("hosts response exceeds DNS wire limit"));
            }
            state.set_raw_response(wire);
            Ok(true)
        }
        ResponsePolicy::Ttl(policy) => {
            let wire = match &state.response {
                mosdns_sequence_core::ResponseState::None => return Ok(false),
                mosdns_sequence_core::ResponseState::Raw(wire) => wire.as_bytes().to_vec(),
                mosdns_sequence_core::ResponseState::Synthesized(response) => {
                    mosdns_dns_core::synthesize_response(
                        &state.query.header,
                        &state.query.question,
                        u8::try_from(response.rcode())
                            .map_err(|_| ExecutorError::new("unsupported synthesized rcode"))?,
                    )
                    .map_err(|_| ExecutorError::new("cannot construct TTL response"))?
                }
            };
            let patched = match policy {
                TtlPolicy::Fixed(0) => {
                    mosdns_dns_core::observe_response_ttl(&wire).map_err(|error| {
                        ExecutorError::new(format!("invalid TTL response: {error:?}"))
                    })?;
                    return Ok(false);
                }
                TtlPolicy::Fixed(ttl) => mosdns_dns_core::replace_response_ttls(&wire, *ttl),
                TtlPolicy::Range { min, max } => {
                    mosdns_dns_core::clamp_response_ttls(&wire, *min, *max)
                }
            }
            .map_err(|error| ExecutorError::new(format!("invalid TTL response: {error:?}")))?;
            state.set_raw_response(patched);
            Ok(false)
        }
        ResponsePolicy::Redirect(_) => Err(ExecutorError::new("scoped redirect runtime pending")),
    }
}
fn encode_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap_or(0));
        out.extend(label.as_bytes());
    }
    out.push(0);
    out
}
fn append_record(
    wire: &mut Vec<u8>,
    name: &[u8],
    kind: u16,
    ttl: u32,
    data: &[u8],
) -> Result<(), ExecutorError> {
    let len = u16::try_from(data.len()).map_err(|_| ExecutorError::new("record too large"))?;
    wire.extend(name);
    wire.extend(kind.to_be_bytes());
    wire.extend(1_u16.to_be_bytes());
    wire.extend(ttl.to_be_bytes());
    wire.extend(len.to_be_bytes());
    wire.extend(data);
    Ok(())
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use hickory_proto::op::{Message, Query};
    use hickory_proto::rr::{DNSClass, Name, RecordType};
    fn state(kind: RecordType, class: DNSClass, name: &str) -> (ExecutionState, Vec<u8>) {
        let mut query = Query::query(Name::from_ascii(name).unwrap(), kind);
        query.set_query_class(class);
        let mut message = Message::new();
        message.add_query(query);
        let wire = message.to_vec().unwrap();
        let (header, question) = mosdns_dns_core::parse_query(&wire).unwrap();
        (ExecutionState::new(header, question), wire)
    }
    #[test]
    fn no_op_hosts_preserves_existing_response_and_generation() {
        let rules = hosts(
            &["a.example 192.0.2.1".into(), "empty.example".into()],
            &[],
            "fixture",
        )
        .unwrap();
        let policy = ResponsePolicy::Hosts(Rc::new(rules));
        for (name, kind, class) in [
            ("miss.example", RecordType::A, DNSClass::IN),
            ("empty.example", RecordType::A, DNSClass::IN),
            ("a.example", RecordType::MX, DNSClass::IN),
            ("a.example", RecordType::A, DNSClass::CH),
        ] {
            let (mut state, raw) = state(kind, class, name);
            state.set_synthesized_response(3).unwrap();
            let previous = state.clone();
            assert!(!apply_wire_policy(&policy, &mut state, &raw).unwrap());
            assert_eq!(state, previous);
        }
    }
    #[test]
    fn malformed_ttl_response_is_not_partially_committed() {
        for policy in [
            TtlPolicy::Fixed(0),
            TtlPolicy::Fixed(60),
            TtlPolicy::Range { min: 10, max: 100 },
        ] {
            let (mut state, raw) = state(RecordType::A, DNSClass::IN, "a.example");
            state.set_raw_response(vec![0; 13]);
            let previous = state.clone();
            assert!(apply_wire_policy(&ResponsePolicy::Ttl(policy), &mut state, &raw).is_err());
            assert_eq!(state, previous);
        }
    }
}
