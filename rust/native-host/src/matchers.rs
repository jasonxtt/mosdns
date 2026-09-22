use std::net::{IpAddr, Ipv4Addr};

use mosdns_dns_core::observe_answer_addresses;
use mosdns_matcher_core::{FullMatcher, IpPrefixList};
use mosdns_sequence_core::{ExecutionState, MatchOutcome, Matcher, MatcherError, ResponseState};

/// Exact qname matcher for the deliberately narrow native W3 grammar.
#[allow(dead_code)] // Slice 0 compiles adapters before Slice 1 binds W3 rules.
pub(crate) struct FullQnameMatcher {
    domains: FullMatcher<()>,
}

impl FullQnameMatcher {
    #[allow(dead_code)]
    pub(crate) fn new(domain: &str) -> Result<Self, MatcherBuildError> {
        let domain = normalize_ascii_domain(domain)?;
        let mut domains = FullMatcher::new();
        domains.add(&domain, ());
        Ok(Self { domains })
    }
}

/// Configuration-time rejection for a matcher expression outside W3's
/// ASCII full-domain grammar.
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

/// Answer-only response-IP matcher for the narrow native W3 grammar.
#[allow(dead_code)]
pub(crate) struct ResponseIpMatcher {
    prefixes: IpPrefixList,
}

impl ResponseIpMatcher {
    #[allow(dead_code)]
    pub(crate) fn ipv4(address: Ipv4Addr) -> Self {
        let mut prefixes = IpPrefixList::new();
        prefixes.append(IpAddr::V4(address), 32);
        prefixes.rebuild();
        Self { prefixes }
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
                .any(|address| self.prefixes.contains(address)),
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
fn wire_name_to_ascii_domain(wire: &[u8]) -> Option<String> {
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
}
