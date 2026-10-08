//! The `rewrite` plugin: `domain:x.com <IP or domain>` rules that answer a
//! matching A/AAAA query directly. An IP target is answered locally; a domain
//! target is resolved through one fixed UDP DNS server and its answers are
//! returned with a fixed TTL and no CNAME. A match accepts the query, so the
//! rest of the current chain does not run.
//!
//! The rule file syntax and the `/show` and `/post` endpoints follow the
//! reference product: a line is exactly two fields, only whole-line `#`
//! comments are skipped, and a rule file that does not exist is allowed.

use std::cell::RefCell;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hickory_proto::op::{Edns, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::{Name, RecordType};
use mosdns_matcher_core::{MixMatcher, normalize};
use mosdns_sequence_core::{ExecutionState, RoutingField, StateMutation};
use mosdns_upstream_core::{
    Endpoint, ExchangeContext, ExchangeRequest, Transport, TransportCancellation, Upstream,
};
use serde::Deserialize;

use crate::rule_sources::Reply;

/// The audit `domain_set` label a rewritten query carries.
pub(crate) const LABEL: &str = "重定向";
/// TTL of every answer this plugin produces.
const FIXED_TTL: u32 = 5;
/// Longest wait for the DNS server a domain target resolves through.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
/// A rule line longer than this is rejected, as the reference line scanner does.
const MAX_LINE: usize = 64 * 1024;
/// EDNS payload size of the query sent for a domain target.
const EDNS_PAYLOAD: u16 = 1232;

const TEXT: &str = "text/plain; charset=utf-8";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    /// A static address; an IPv4-mapped IPv6 address counts as IPv4.
    Ip(IpAddr),
    /// A fully qualified name resolved through the configured DNS server.
    Domain(String),
}

/// One immutable generation of rules.
pub(crate) struct RuleSet {
    matcher: MixMatcher<usize>,
    targets: Vec<Target>,
    /// The accepted rule lines, trimmed, in load order (what `/show` prints).
    lines: Vec<String>,
}

/// Rules being collected: a later rule for the same pattern replaces the
/// earlier target.
#[derive(Default)]
struct RuleBuilder {
    entries: Vec<(String, Target)>,
    positions: std::collections::BTreeMap<String, usize>,
    lines: Vec<String>,
}

impl RuleBuilder {
    /// Adds every rule of `text`; an error names the bad line (1-based).
    fn extend(&mut self, text: &str) -> Result<(), String> {
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            if line.len() > MAX_LINE {
                return Err(format!("error on line {number}: token too long"));
            }
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() != 2 {
                return Err(format!(
                    "error on line {number}: rule must have 2 fields, but got {}",
                    fields.len()
                ));
            }
            let target = parse_target(fields[1])
                .map_err(|reason| format!("error on line {number}: {reason}"))?;
            let (kind, pattern) = fields[0].split_once(':').unwrap_or(("full", fields[0]));
            let pattern = if kind == "regexp" {
                pattern.to_owned()
            } else {
                normalize(pattern)
            };
            let canonical = format!("{kind}:{pattern}");
            MixMatcher::new()
                .add(&canonical, ())
                .map_err(|error| format!("error on line {number}: {error}"))?;
            if let Some(position) = self.positions.get(&canonical).copied() {
                self.entries[position].1 = target;
            } else {
                self.positions.insert(canonical.clone(), self.entries.len());
                self.entries.push((canonical, target));
            }
            self.lines.push(line.to_owned());
        }
        Ok(())
    }

    fn finish(self) -> Result<RuleSet, String> {
        let mut matcher = MixMatcher::new();
        let mut targets = Vec::with_capacity(self.entries.len());
        for (rule, target) in self.entries {
            matcher
                .add(&rule, targets.len())
                .map_err(|error| error.to_string())?;
            targets.push(target);
        }
        Ok(RuleSet {
            matcher,
            targets,
            lines: self.lines,
        })
    }
}

impl RuleSet {
    /// Parses rule text; an error names the first bad line (1-based).
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut builder = RuleBuilder::default();
        builder.extend(text)?;
        builder.finish()
    }

    fn lookup(&self, domain: &str) -> Option<&Target> {
        self.matcher
            .r#match(domain)
            .map(|index| &self.targets[*index])
    }

    pub(crate) fn len(&self) -> usize {
        self.lines.len()
    }
}

fn parse_target(value: &str) -> Result<Target, String> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(Target::Ip(match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
            other => other,
        }));
    }
    let mut name =
        Name::from_ascii(value).map_err(|_| format!("invalid domain name syntax: {value}"))?;
    name.set_fqdn(true);
    Ok(Target::Domain(name.to_ascii()))
}

/// How one dispatch ended.
pub(crate) enum RewriteEnd {
    /// No rule matched: the chain continues.
    NoMatch,
    /// A response was set: the chain is accepted.
    Answered,
    /// The request ended while the DNS server was being asked.
    Cancelled,
}

/// The DNS server a domain target resolves through: a fixed address, or a
/// host name the system resolver turns into one address for each lookup.
pub(crate) enum DnsServer {
    Fixed(Upstream),
    Named { host: String, port: u16 },
}

impl DnsServer {
    /// Parses `host[:port]` (port 53 by default; a bracketed or bare IPv6
    /// address is accepted).
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let fixed = |address: SocketAddr| {
            Endpoint::new(address, Transport::Udp)
                .map(|endpoint| Self::Fixed(Upstream::new(endpoint)))
                .map_err(|error| error.to_string())
        };
        if let Ok(address) = text.parse::<SocketAddr>() {
            return fixed(address);
        }
        if let Ok(ip) = text.trim_matches(['[', ']']).parse::<IpAddr>() {
            return fixed(SocketAddr::new(ip, 53));
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (
                host,
                port.parse::<u16>()
                    .map_err(|_| format!("invalid port in `{text}`"))?,
            ),
            _ => (text, 53),
        };
        if host.is_empty() || port == 0 || host.contains(char::is_whitespace) {
            return Err(format!("invalid dns server address `{text}`"));
        }
        Ok(Self::Named {
            host: host.to_owned(),
            port,
        })
    }
}

/// The runtime owner of one `rewrite` plugin.
pub struct Rewrite {
    rules: RefCell<Rc<RuleSet>>,
    /// All configured files; the first one is where `/post` writes.
    files: Vec<PathBuf>,
    dns: DnsServer,
    write_lock: tokio::sync::Mutex<()>,
}

/// A compiled `rewrite` definition and the external executable it runs as.
#[derive(Clone)]
pub struct RewriteConfig {
    pub tag: String,
    pub executable: mosdns_sequence_core::ExecutableId,
    pub runtime: Rc<Rewrite>,
}

impl Rewrite {
    /// Reads every file (a missing one is skipped) into one rule set.
    pub(crate) fn load(files: Vec<PathBuf>, dns: DnsServer) -> Result<Self, String> {
        let mut builder = RuleBuilder::default();
        for (index, file) in files.iter().enumerate() {
            let bytes = match std::fs::read(file) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!(
                        "failed to read file #{index} {}, {error}",
                        file.display()
                    ));
                }
            };
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                format!(
                    "failed to load rules from file #{index} {}, rules must be UTF-8 text",
                    file.display()
                )
            })?;
            builder.extend(text).map_err(|reason| {
                format!(
                    "failed to load rules from file #{index} {}, {reason}",
                    file.display()
                )
            })?;
        }
        let set = builder.finish()?;
        Ok(Self {
            rules: RefCell::new(Rc::new(set)),
            files,
            dns,
            write_lock: tokio::sync::Mutex::new(()),
        })
    }

    fn current(&self) -> Rc<RuleSet> {
        Rc::clone(&self.rules.borrow())
    }

    /// Answers `state`'s query when a rule matches. `cancellations` end a
    /// pending DNS lookup; `deadline` is the request budget.
    pub(crate) async fn run(
        &self,
        state: &mut ExecutionState,
        raw: &[u8],
        deadline: Instant,
        cancellations: &[&TransportCancellation],
    ) -> RewriteEnd {
        let question = &state.query.question;
        let Some(name) = crate::matchers::wire_name_to_ascii_domain(&question.qname_wire) else {
            return RewriteEnd::NoMatch;
        };
        let rules = self.current();
        let Some(target) = rules.lookup(&name).cloned() else {
            return RewriteEnd::NoMatch;
        };
        state.apply_mutation(StateMutation::SetRouting {
            field: RoutingField::DomainSet,
            value: Some(LABEL.to_owned()),
        });
        let qtype = state.query.question.qtype;
        let response = match target {
            Target::Ip(ip) => ip_response(state, raw, ip, qtype),
            Target::Domain(domain) => {
                if !matches!(qtype, 1 | 28) {
                    skeleton(state, raw)
                } else {
                    match self
                        .resolve(&domain, qtype, raw, deadline, cancellations)
                        .await
                    {
                        Resolved::Answers(answers) => domain_response(raw, &answers),
                        Resolved::Failed => None,
                        Resolved::Cancelled => return RewriteEnd::Cancelled,
                    }
                }
            }
        };
        match response {
            Some(wire) => state.set_raw_response(wire),
            None => {
                // A failed lookup or an unbuildable answer is SERVFAIL.
                let _ = state.set_synthesized_response(2);
            }
        }
        RewriteEnd::Answered
    }

    async fn resolve(
        &self,
        domain: &str,
        qtype: u16,
        raw: &[u8],
        deadline: Instant,
        cancellations: &[&TransportCancellation],
    ) -> Resolved {
        let Some(query) = upstream_query(domain, qtype, raw) else {
            return Resolved::Failed;
        };
        let deadline = deadline.min(Instant::now() + DNS_TIMEOUT);
        let exchange = async {
            let request = ExchangeRequest::new(&query).ok()?;
            let context = ExchangeContext::new(deadline, TransportCancellation::new());
            let remaining = deadline.saturating_duration_since(Instant::now());
            match &self.dns {
                DnsServer::Fixed(upstream) => upstream.exchange(request, context).await.ok(),
                DnsServer::Named { host, port } => {
                    // The system resolver picks the address for this lookup.
                    let address = tokio::time::timeout(
                        remaining,
                        tokio::net::lookup_host((host.as_str(), *port)),
                    )
                    .await
                    .ok()?
                    .ok()?
                    .next()?;
                    let endpoint = Endpoint::new(address, Transport::Udp).ok()?;
                    Upstream::new(endpoint)
                        .exchange(request, context)
                        .await
                        .ok()
                }
            }
        };
        let cancelled = async {
            match cancellations {
                [] => std::future::pending().await,
                [only] => only.cancelled().await,
                [first, second, ..] => {
                    tokio::select! {
                        () = first.cancelled() => {}
                        () = second.cancelled() => {}
                    }
                }
            }
        };
        let response = tokio::select! {
            biased;
            () = cancelled => return Resolved::Cancelled,
            response = exchange => response,
        };
        let Some(response) = response else {
            return Resolved::Failed;
        };
        match Message::from_vec(response.wire()) {
            Ok(message) if message.response_code() == ResponseCode::NoError => {
                Resolved::Answers(message.answers().to_vec())
            }
            _ => Resolved::Failed,
        }
    }

    /// `GET /show`: the accepted rule lines.
    pub(crate) fn show(&self) -> Reply {
        Reply {
            status: 200,
            content_type: Some(TEXT),
            body: self.current().lines.join("\n").into_bytes(),
            headers: Vec::new(),
        }
    }

    /// `POST /post`: replaces every rule with the posted ones, then writes
    /// the first file. The in-memory rules change even when the write fails,
    /// as the reference does, and the reply says so.
    pub(crate) async fn post(&self, body: &[u8]) -> Reply {
        #[derive(Deserialize)]
        struct Payload {
            #[serde(default)]
            values: Option<Vec<String>>,
        }
        let Ok(payload) = serde_json::from_slice::<Payload>(body) else {
            return error(400, "invalid JSON payload");
        };
        let Some(file) = self.files.first().cloned() else {
            return error(400, "no rule file configured, cannot post rules");
        };
        let text = payload.values.unwrap_or_default().join("\n");
        let set = match RuleSet::parse(&text) {
            Ok(set) => set,
            Err(reason) => return error(400, &format!("failed to parse new rules: {reason}")),
        };
        let _serialized = self.write_lock.lock().await;
        let mut content = set.lines.join("\n");
        content.push('\n');
        let count = set.len();
        *self.rules.borrow_mut() = Rc::new(set);
        let written = crate::transaction::blocking_io(move || {
            crate::rule_sources::write_atomically(&file, content.as_bytes())
        });
        match written {
            Ok(Ok(())) => Reply {
                status: 200,
                content_type: Some(TEXT),
                body: format!("rewrite rules replaced with {count} entries").into_bytes(),
                headers: Vec::new(),
            },
            Ok(Err(reason)) | Err(reason) => error(
                500,
                &format!("In-memory rules updated, but failed to write to file: {reason}"),
            ),
        }
    }
}

enum Resolved {
    Answers(Vec<hickory_proto::rr::Record>),
    Failed,
    Cancelled,
}

fn error(status: u16, message: &str) -> Reply {
    Reply {
        status,
        content_type: Some(TEXT),
        body: format!("{message}\n").into_bytes(),
        headers: Vec::new(),
    }
}

/// A response skeleton for the query: QR, RA, the query's RD, one question.
fn skeleton(state: &ExecutionState, raw: &[u8]) -> Option<Vec<u8>> {
    let mut wire =
        mosdns_dns_core::synthesize_response(&state.query.header, &state.query.question, 0).ok()?;
    wire[2] |= raw.get(2).copied().unwrap_or(0) & 1;
    Some(wire)
}

fn ip_response(state: &ExecutionState, raw: &[u8], ip: IpAddr, qtype: u16) -> Option<Vec<u8>> {
    let mut wire = skeleton(state, raw)?;
    let data: Vec<u8> = match (ip, qtype) {
        (IpAddr::V4(ip), 1) => ip.octets().to_vec(),
        (IpAddr::V6(ip), 28) => ip.octets().to_vec(),
        // The other family or any other type: NOERROR without answers.
        _ => return Some(wire),
    };
    wire[6..8].copy_from_slice(&1_u16.to_be_bytes());
    wire.extend_from_slice(&state.query.question.qname_wire);
    wire.extend_from_slice(&qtype.to_be_bytes());
    wire.extend_from_slice(&1_u16.to_be_bytes());
    wire.extend_from_slice(&FIXED_TTL.to_be_bytes());
    wire.extend_from_slice(&u16::try_from(data.len()).ok()?.to_be_bytes());
    wire.extend_from_slice(&data);
    Some(wire)
}

/// The query sent to the DNS server: the target name with the original
/// RD/AD/CD flags and an OPT carrying the original DO bit.
fn upstream_query(domain: &str, qtype: u16, raw: &[u8]) -> Option<Vec<u8>> {
    let original = Message::from_vec(raw).ok()?;
    let dnssec_ok = original
        .extensions()
        .as_ref()
        .is_some_and(|edns| edns.flags().dnssec_ok);
    let mut id = [0_u8; 2];
    getrandom::fill(&mut id).ok()?;
    let mut query = Message::new();
    query.set_id(u16::from_be_bytes(id));
    query.set_message_type(MessageType::Query);
    query.set_op_code(OpCode::Query);
    query.set_recursion_desired(original.recursion_desired());
    query.set_authentic_data(original.authentic_data());
    query.set_checking_disabled(original.checking_disabled());
    query.add_query(hickory_proto::op::Query::query(
        Name::from_ascii(domain).ok()?,
        RecordType::from(qtype),
    ));
    let mut edns = Edns::new();
    edns.set_max_payload(EDNS_PAYLOAD);
    edns.set_dnssec_ok(dnssec_ok);
    query.set_edns(edns);
    query.to_vec().ok()
}

/// The reply for a domain target: the original question with the DNS
/// server's answers, every TTL fixed.
fn domain_response(raw: &[u8], answers: &[hickory_proto::rr::Record]) -> Option<Vec<u8>> {
    let original = Message::from_vec(raw).ok()?;
    let mut reply = Message::new();
    reply.set_id(original.id());
    reply.set_message_type(MessageType::Response);
    reply.set_op_code(original.op_code());
    reply.set_recursion_desired(original.recursion_desired());
    reply.set_checking_disabled(original.checking_disabled());
    reply.set_response_code(ResponseCode::NoError);
    for query in original.queries() {
        reply.add_query(query.clone());
    }
    for answer in answers {
        let mut answer = answer.clone();
        answer.set_ttl(FIXED_TTL);
        reply.add_answer(answer);
    }
    reply.to_vec().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_follow_the_reference_grammar() {
        let set = RuleSet::parse(
            "# comment\n\n  domain:Example.COM   1.2.3.4  \nfull:a.test ::ffff:5.6.7.8\nkeyword:cdn target.test\n",
        )
        .unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(
            set.lookup("www.example.com"),
            Some(&Target::Ip("1.2.3.4".parse().unwrap()))
        );
        // An IPv4-mapped address is an IPv4 target.
        assert_eq!(
            set.lookup("a.test"),
            Some(&Target::Ip("5.6.7.8".parse().unwrap()))
        );
        assert_eq!(
            set.lookup("x.cdn.net"),
            Some(&Target::Domain("target.test.".into()))
        );
        assert_eq!(set.lookup("other.test"), None);
        assert_eq!(set.lines[0], "domain:Example.COM   1.2.3.4");
    }

    #[test]
    fn bad_lines_name_their_number() {
        assert_eq!(
            RuleSet::parse("a.test 1.1.1.1\nb.test\n").err().unwrap(),
            "error on line 2: rule must have 2 fields, but got 1"
        );
        // Only whole-line comments are skipped: a trailing comment is a field.
        assert!(RuleSet::parse("a.test 1.1.1.1 # c").is_err());
        assert!(RuleSet::parse("a.test not_a_domain!").is_err());
    }

    #[test]
    fn a_later_rule_for_the_same_pattern_wins() {
        let set = RuleSet::parse("full:a.test 1.1.1.1\nfull:a.test 2.2.2.2\n").unwrap();
        assert_eq!(
            set.lookup("a.test"),
            Some(&Target::Ip("2.2.2.2".parse().unwrap()))
        );
        assert_eq!(set.len(), 2);
    }
}
