#[cfg(test)]
use std::net::{IpAddr, Ipv4Addr};
use std::rc::Rc;

use mosdns_dns_core::observe_answer_addresses;
use mosdns_matcher_core::{IpPrefixList, MixMatcher};
use mosdns_sequence_core::{
    ExecutionState, MatchOutcome, Matcher, MatcherError, ResponseState, StateMutation,
};

use crate::managed::DomainSetHandle;

/// One qname matcher plus every domain set it consults. A match is true when
/// any consulted set matches, preserving the configured reference order. A
/// managed set is read through its current published generation at every
/// evaluation, so a POST is visible to the very next query.
pub(crate) struct QnameMatcher {
    groups: Vec<(String, DomainSetHandle)>,
}

impl QnameMatcher {
    pub(crate) fn new(groups: Vec<(String, DomainSetHandle)>) -> Self {
        Self { groups }
    }
}

impl Matcher for QnameMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let Some(domain) = wire_name_to_ascii_domain(&state.query.question.qname_wire) else {
            return Ok(MatchOutcome::new(false, None));
        };
        let Some((source, _set)) = self.groups.iter().find(|(_, set)| set.matches(&domain)) else {
            return Ok(MatchOutcome::new(false, None));
        };
        let is_provider = source.starts_with("domain_set:");
        Ok(MatchOutcome::new(
            true,
            Some(StateMutation::SetRoutingFields {
                domain_set: is_provider.then(|| source["domain_set:".len()..].to_owned()),
                effective_tag: None,
                // Inline qname identity is owned by the containing YAML
                // rule. The compiler installs that identity only after all
                // matchers succeed, so the matcher must not publish its
                // helper path as public provenance.
                matched_rule_source: is_provider.then(|| source.clone()),
            }),
        ))
    }
}

/// Exact qname matcher for the deliberately narrow native W3 grammar.
#[allow(dead_code)] // The W3 domain_set grammar still compiles through this helper.
pub(crate) struct FullQnameMatcher {
    domains: mosdns_matcher_core::FullMatcher<()>,
}

impl FullQnameMatcher {
    #[allow(dead_code)]
    pub(crate) fn new(domain: &str) -> Result<Self, MatcherBuildError> {
        let domain = normalize_ascii_domain(domain)?;
        let mut domains = mosdns_matcher_core::FullMatcher::new();
        domains.add(&domain, ());
        Ok(Self { domains })
    }
}

/// Configuration-time rejection for a matcher expression outside the accepted
/// ASCII domain grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MatcherBuildError {
    EmptyDomain,
    InvalidDomain,
}

impl Matcher for FullQnameMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let matched = wire_name_to_ascii_domain(&state.query.question.qname_wire)
            .is_some_and(|domain| self.domains.r#match(&domain).is_some());
        Ok(MatchOutcome::new(matched, None))
    }
}

/// Matches when the question type is one of the configured types.
pub(crate) struct QtypeMatcher {
    types: Vec<u16>,
}

impl QtypeMatcher {
    pub(crate) fn new(types: Vec<u16>) -> Self {
        Self { types }
    }
}

impl Matcher for QtypeMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let qtype = state.query.question.qtype;
        Ok(MatchOutcome::new(self.types.contains(&qtype), None))
    }
}

/// Matches when execution has already formed any response.
pub(crate) struct HasResponseMatcher;

impl Matcher for HasResponseMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(
            !matches!(state.response, ResponseState::None),
            None,
        ))
    }
}

/// Answer-only response-IP matcher over immutable startup prefix snapshots.
#[allow(dead_code)]
pub(crate) struct ResponseIpMatcher {
    prefixes: Vec<Rc<IpPrefixList>>,
}

impl ResponseIpMatcher {
    pub(crate) fn new(prefixes: Vec<Rc<IpPrefixList>>) -> Self {
        Self { prefixes }
    }
    #[cfg(test)]
    pub(crate) fn ipv4(address: Ipv4Addr) -> Self {
        let mut prefixes = IpPrefixList::new();
        prefixes.append(IpAddr::V4(address), 32);
        prefixes.rebuild();
        Self {
            prefixes: vec![Rc::new(prefixes)],
        }
    }
}

impl Matcher for ResponseIpMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let ResponseState::Raw(response) = &state.response else {
            return Ok(MatchOutcome::new(false, None));
        };
        let addresses = observe_answer_addresses(response.as_bytes()).map_err(|error| {
            MatcherError::new(format!("cannot inspect response addresses: {error:?}"))
        })?;
        Ok(MatchOutcome::new(
            addresses
                .into_iter()
                .any(|address| self.prefixes.iter().any(|list| list.contains(address))),
            None,
        ))
    }
}

/// The W3 terminal catch-all expressed through the normal sequence matcher seam.
#[allow(dead_code)]
pub(crate) struct TrueMatcher;

impl Matcher for TrueMatcher {
    fn evaluate(&self, _state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(true, None))
    }
}

/// Converts a decoded qname wire form into the ASCII-only domain grammar used
/// by the bounded W3 matcher. Unsupported labels intentionally become a
/// non-match instead of receiving a lossy string conversion.
#[allow(dead_code)]
pub(crate) fn wire_name_to_ascii_domain(wire: &[u8]) -> Option<String> {
    let mut position = 0;
    let mut labels = Vec::new();
    loop {
        let length = usize::from(*wire.get(position)?);
        position = position.checked_add(1)?;
        if length == 0 {
            return (position == wire.len()).then(|| labels.join("."));
        }
        if length > 63 {
            return None;
        }
        let end = position.checked_add(length)?;
        let label = wire.get(position..end)?;
        if !valid_ascii_label(label) {
            return None;
        }
        labels.push(std::str::from_utf8(label).ok()?.to_owned());
        position = end;
    }
}

/// Resolves one configured rule-file path against the declaring YAML
/// directory. An absolute path and an in-memory compile (empty base) are used
/// verbatim.
pub(crate) fn resolve_rule_path(file: &str, base_dir: &std::path::Path) -> std::path::PathBuf {
    if base_dir.as_os_str().is_empty() || std::path::Path::new(file).is_absolute() {
        std::path::PathBuf::from(file)
    } else {
        base_dir.join(file)
    }
}

/// Builds one domain-set matcher from ordered rule expressions and rule files.
///
/// The set uses the shared [`MixMatcher`] grammar and normalization rather than
/// a second matching engine. The returned matcher is owned by the caller and
/// the vector is the accepted rule text in
/// Go's provider order: every `exps` entry, then each file's accepted rules.
///
/// File rules follow the Go text-file policy: trim outer whitespace, skip blank
/// lines and whole-line `#` comments, keep an inline `#` inside the candidate
/// rule, and skip an invalid individual rule instead of failing the whole load.
/// A missing or unreadable file stays a load error, because the caller
/// configured an explicit path.
pub(crate) fn build_domain_set(
    expressions: &[String],
    files: &[String],
    base_dir: &std::path::Path,
) -> Result<(MixMatcher<()>, Vec<String>), DomainSetError> {
    let mut set = MixMatcher::new();
    set.set_default("domain");
    let mut accepted: Vec<String> = Vec::with_capacity(expressions.len());
    for (index, expression) in expressions.iter().enumerate() {
        set.add(expression, ())
            .map_err(|error| DomainSetError::Expression {
                index,
                expression: expression.clone(),
                reason: error.to_string(),
            })?;
        accepted.push(expression.clone());
    }
    for (index, file) in files.iter().enumerate() {
        let path = resolve_rule_path(file, base_dir);
        let text = std::fs::read_to_string(&path).map_err(|error| DomainSetError::File {
            index,
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Go adds every candidate line to the matcher and only records the
            // ones the matcher accepted, so one bad rule never fails the file.
            if set.add(line, ()).is_ok() {
                accepted.push(line.to_owned());
            }
        }
    }
    Ok((set, accepted))
}

/// A rule-level rejection from the domain-set loader, keeping the offending
/// source path visible to the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DomainSetError {
    Expression {
        index: usize,
        expression: String,
        reason: String,
    },
    File {
        index: usize,
        path: String,
        reason: String,
    },
}

impl std::fmt::Display for DomainSetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expression {
                index,
                expression,
                reason,
            } => write!(formatter, "expression {index} `{expression}`: {reason}"),
            Self::File {
                index,
                path,
                reason,
            } => write!(formatter, "file {index} {path}: {reason}"),
        }
    }
}

impl std::error::Error for DomainSetError {}

fn normalize_ascii_domain(domain: &str) -> Result<String, MatcherBuildError> {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    if domain.is_empty() {
        return Err(MatcherBuildError::EmptyDomain);
    }
    if domain.len() > 253
        || domain
            .split('.')
            .any(|label| !valid_ascii_label(label.as_bytes()))
    {
        return Err(MatcherBuildError::InvalidDomain);
    }
    Ok(domain.to_ascii_lowercase())
}

fn valid_ascii_label(label: &[u8]) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.first().is_some_and(u8::is_ascii_alphanumeric)
        && label.last().is_some_and(u8::is_ascii_alphanumeric)
        && label
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use mosdns_dns_core::parse_query;
    use mosdns_sequence_core::{ExecutionState, Matcher};

    use super::{
        FullQnameMatcher, MatcherBuildError, ResponseIpMatcher, TrueMatcher,
        wire_name_to_ascii_domain,
    };

    fn query(name: &[u8]) -> Vec<u8> {
        let mut wire = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        wire.extend_from_slice(name);
        wire.extend_from_slice(&[0, 1, 0, 1]);
        wire
    }

    fn state(query_wire: &[u8]) -> ExecutionState {
        let (header, question) = parse_query(query_wire).expect("test query");
        ExecutionState::new(header, question)
    }

    fn response_with_answer(ip: [u8; 4]) -> Vec<u8> {
        let mut response = query(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]);
        response[2] = 0x81;
        response[6..8].copy_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 10, 0, 4]);
        response.extend_from_slice(&ip);
        response
    }

    fn response_without_answers() -> Vec<u8> {
        let mut response = query(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]);
        response[2] = 0x81;
        response
    }

    #[test]
    fn qname_is_exact_case_insensitive_and_requires_plain_wire_labels() {
        let matcher = FullQnameMatcher::new("EXAMPLE.test.").expect("valid domain");
        for name in [
            &[
                7, b'E', b'x', b'A', b'm', b'p', b'l', b'e', 4, b'T', b'e', b'S', b't', 0,
            ][..],
            &[
                7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 4, b't', b'e', b's', b't', 0,
            ][..],
        ] {
            assert!(
                matcher
                    .evaluate(&state(&query(name)))
                    .expect("match")
                    .matched
            );
        }
        assert!(
            !matcher
                .evaluate(&state(&query(&[
                    3, b'a', b'p', b'i', 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 4, b't',
                    b'e', b's', b't', 0
                ])))
                .expect("miss")
                .matched
        );
        assert_eq!(wire_name_to_ascii_domain(&[3, b'a', b'.', b'b', 0]), None);
        assert_eq!(wire_name_to_ascii_domain(&[1, 0xff, 0]), None);
        for (domain, error) in [
            ("", MatcherBuildError::EmptyDomain),
            (".", MatcherBuildError::EmptyDomain),
            ("bad..test", MatcherBuildError::InvalidDomain),
            ("-bad.test", MatcherBuildError::InvalidDomain),
            ("bad-.test", MatcherBuildError::InvalidDomain),
            ("bad_.test", MatcherBuildError::InvalidDomain),
        ] {
            assert!(
                matches!(FullQnameMatcher::new(domain), Err(actual) if actual == error),
                "{domain}"
            );
        }
    }

    #[test]
    fn response_ip_matches_only_a_raw_answer_without_mutating_state() {
        let matcher = ResponseIpMatcher::ipv4(Ipv4Addr::new(192, 0, 2, 10));
        let mut state = state(&query(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]));
        let before = state.clone();
        assert!(!matcher.evaluate(&state).expect("none misses").matched);
        assert_eq!(state, before);

        state.set_raw_response(response_with_answer([192, 0, 2, 10]));
        let before = state.clone();
        assert!(matcher.evaluate(&state).expect("raw matches").matched);
        assert_eq!(state, before);
        state.set_synthesized_response(2).expect("valid rcode");
        assert!(
            !matcher
                .evaluate(&state)
                .expect("synthesized misses")
                .matched
        );
        state.set_raw_response(response_without_answers());
        assert!(
            !matcher
                .evaluate(&state)
                .expect("empty answer misses")
                .matched
        );
        assert!(TrueMatcher.evaluate(&state).expect("true").matched);
    }
    #[test]
    fn expanded_response_ip_ignores_non_answer_addresses_and_cname_without_mutation() {
        let prefixes = crate::policy::ip_list(
            &["192.0.2.0/24".into(), "2001:db8::/32".into()],
            &[],
            "fixture",
        )
        .unwrap();
        let matcher = ResponseIpMatcher::new(vec![prefixes]);
        let mut state = state(&query(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]));
        let mut message =
            hickory_proto::op::Message::from_vec(&response_without_answers()).unwrap();
        let owner = hickory_proto::rr::Name::from_ascii("example.").unwrap();
        message.add_name_server(hickory_proto::rr::Record::from_rdata(
            owner.clone(),
            60,
            hickory_proto::rr::RData::AAAA(hickory_proto::rr::rdata::AAAA(
                "2001:db8::1".parse().unwrap(),
            )),
        ));
        message.add_additional(hickory_proto::rr::Record::from_rdata(
            owner.clone(),
            60,
            hickory_proto::rr::RData::A(hickory_proto::rr::rdata::A("192.0.2.1".parse().unwrap())),
        ));
        message.add_answer(hickory_proto::rr::Record::from_rdata(
            owner.clone(),
            1,
            hickory_proto::rr::RData::CNAME(hickory_proto::rr::rdata::CNAME(owner.clone())),
        ));
        state.set_raw_response(message.to_vec().unwrap());
        let before = state.clone();
        assert!(!matcher.evaluate(&state).unwrap().matched);
        assert_eq!(state, before);
        message.add_answer(hickory_proto::rr::Record::from_rdata(
            owner,
            60,
            hickory_proto::rr::RData::AAAA(hickory_proto::rr::rdata::AAAA(
                "2001:db8::1".parse().unwrap(),
            )),
        ));
        state.set_raw_response(message.to_vec().unwrap());
        let before = state.clone();
        assert!(matcher.evaluate(&state).unwrap().matched);
        assert_eq!(state, before);
        let mut broken = response_with_answer([192, 0, 2, 1]);
        broken.pop();
        state.set_raw_response(broken);
        let before = state.clone();
        assert!(matcher.evaluate(&state).is_err());
        assert_eq!(state, before);
    }
}
