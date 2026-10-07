use std::time::Duration;

use mosdns_sequence_core::{
    ExecutionState, Executor, ExecutorError, ExecutorOutcome, MatchOutcome, Matcher, MatcherError,
    ResponseState, RoutingField, StateMutation,
};

/// The typed native representation of one `fast_mark` configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FastMarkConfig {
    mask: u64,
}

impl FastMarkConfig {
    pub(crate) fn parse(args: &str) -> Result<Self, String> {
        let mut mask = 0_u64;
        let mut count = 0;
        for token in args.split_whitespace() {
            let id = token
                .parse::<u16>()
                .map_err(|_| format!("invalid fast_mark ID `{token}`"))?;
            if id > 63 {
                return Err(format!("fast_mark ID must be between 0 and 63, got {id}"));
            }
            mask |= 1_u64 << id;
            count += 1;
        }
        if count == 0 {
            return Err("fast_mark requires at least one ID".to_owned());
        }
        Ok(Self { mask })
    }

    pub(crate) fn matcher(self) -> Box<dyn Matcher> {
        Box::new(FastMarkMatcher { mask: self.mask })
    }

    pub(crate) fn executor(self) -> Box<dyn Executor> {
        Box::new(FastMarkExecutor { mask: self.mask })
    }
}

struct FastMarkMatcher {
    mask: u64,
}

impl Matcher for FastMarkMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(state.fast_flags & self.mask != 0, None))
    }
}

struct FastMarkExecutor {
    mask: u64,
}

impl Executor for FastMarkExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        state.apply_mutation(StateMutation::SetFastFlags(state.fast_flags | self.mask));
        Ok(ExecutorOutcome::Continue)
    }
}

/// The typed native representation of one quick `mark` configuration: a list
/// of context marks (not fast-mark bits), each an unsigned decimal `u32`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkConfig {
    marks: Vec<u32>,
}

impl MarkConfig {
    pub(crate) fn parse(args: &str) -> Result<Self, String> {
        let marks = args
            .split_whitespace()
            .map(|token| {
                // `strconv.ParseUint(_, 10, 32)`: digits only, no sign.
                if !token.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(format!("invalid mark `{token}`"));
                }
                token
                    .parse::<u32>()
                    .map_err(|_| format!("invalid mark `{token}`"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { marks })
    }

    pub(crate) fn matcher(&self) -> Box<dyn Matcher> {
        Box::new(MarkMatcher {
            marks: self.marks.clone(),
        })
    }

    pub(crate) fn executor(&self) -> Box<dyn Executor> {
        Box::new(MarkExecutor {
            marks: self.marks.clone(),
        })
    }
}

struct MarkMatcher {
    marks: Vec<u32>,
}

impl Matcher for MarkMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        Ok(MatchOutcome::new(
            self.marks.iter().any(|mark| state.marks.contains(mark)),
            None,
        ))
    }
}

struct MarkExecutor {
    marks: Vec<u32>,
}

impl Executor for MarkExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        for mark in &self.marks {
            state.apply_mutation(StateMutation::AddMark(*mark));
        }
        Ok(ExecutorOutcome::Continue)
    }
}

/// The three routing fields accepted by the native `flow_setter` subset.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FlowSetterConfig {
    pub(crate) matched_group: Option<String>,
    pub(crate) final_sequence: Option<String>,
    pub(crate) final_upstream: Option<String>,
}

impl FlowSetterConfig {
    pub(crate) fn from_quick_args(args: &str) -> Result<Self, String> {
        let mut config = Self::default();
        for token in args.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                return Err(format!("invalid flow_setter arg `{token}`"));
            };
            if value.is_empty() {
                return Err(format!("flow_setter key `{key}` requires a value"));
            }
            config.set(key, value)?;
        }
        config.ensure_nonempty()?;
        Ok(config)
    }

    pub(crate) fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err(format!("flow_setter key `{key}` requires a value"));
        }
        let slot = match key {
            "group" => &mut self.matched_group,
            "sequence" => &mut self.final_sequence,
            "upstream" => &mut self.final_upstream,
            _ => return Err(format!("unknown flow_setter key `{key}`")),
        };
        *slot = Some(value.to_owned());
        Ok(())
    }

    pub(crate) fn ensure_nonempty(&self) -> Result<(), String> {
        if self.matched_group.is_none()
            && self.final_sequence.is_none()
            && self.final_upstream.is_none()
        {
            return Err("flow_setter requires at least one routing value".to_owned());
        }
        Ok(())
    }

    pub(crate) fn executor(&self) -> Box<dyn Executor> {
        Box::new(FlowSetterExecutor {
            config: self.clone(),
        })
    }
}

struct FlowSetterExecutor {
    config: FlowSetterConfig,
}

impl Executor for FlowSetterExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        for (field, value) in [
            (
                RoutingField::MatchedGroup,
                self.config.matched_group.clone(),
            ),
            (
                RoutingField::FinalSequence,
                self.config.final_sequence.clone(),
            ),
            (
                RoutingField::FinalUpstream,
                self.config.final_upstream.clone(),
            ),
        ] {
            if value.is_some() {
                state.apply_mutation(StateMutation::SetRouting { field, value });
            }
        }
        Ok(ExecutorOutcome::Continue)
    }
}

/// The typed native representation of one quick `rcode` configuration: the
/// response codes to match. Each token is a signed decimal integer
/// (`strconv.Atoi`); an empty list is accepted and never matches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RcodeConfig {
    codes: Vec<i64>,
}

impl RcodeConfig {
    pub(crate) fn parse(args: &str) -> Result<Self, String> {
        let codes = args
            .split_whitespace()
            .map(|token| {
                token
                    .parse::<i64>()
                    .map_err(|_| format!("invalid rcode `{token}`"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { codes })
    }

    pub(crate) fn matcher(&self) -> Box<dyn Matcher> {
        Box::new(RcodeMatcher {
            codes: self.codes.clone(),
        })
    }
}

struct RcodeMatcher {
    codes: Vec<i64>,
}

impl Matcher for RcodeMatcher {
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError> {
        let matched =
            response_rcode(&state.response).is_some_and(|rcode| self.codes.contains(&rcode));
        Ok(MatchOutcome::new(matched, None))
    }
}

/// The full response code (header nibble plus the EDNS extended bits), as the
/// reference decoder reports it. No response, or a wire that cannot be
/// decoded, has none.
fn response_rcode(response: &ResponseState) -> Option<i64> {
    match response {
        ResponseState::None => None,
        ResponseState::Synthesized(response) => Some(i64::from(response.rcode())),
        ResponseState::Raw(wire) => {
            let wire = wire.as_bytes();
            if wire.len() < 12 {
                return None;
            }
            // Without an additional record there is no OPT to extend the code.
            if wire[10] == 0 && wire[11] == 0 {
                return Some(i64::from(wire[3] & 0x0f));
            }
            mosdns_dns_core::observe_response_metadata(wire)
                .ok()
                .map(|metadata| i64::from(metadata.rcode))
        }
    }
}

/// The typed native representation of one `tag_setter`: the audit
/// `domain_set` label to store, where an empty label clears it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TagSetterConfig {
    tag: Option<String>,
}

impl TagSetterConfig {
    pub(crate) fn new(tag: &str) -> Self {
        Self {
            tag: (!tag.is_empty()).then(|| tag.to_owned()),
        }
    }

    pub(crate) fn executor(&self) -> Box<dyn Executor> {
        Box::new(TagSetterExecutor {
            tag: self.tag.clone(),
        })
    }
}

struct TagSetterExecutor {
    tag: Option<String>,
}

impl Executor for TagSetterExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        state.apply_mutation(StateMutation::SetRouting {
            field: RoutingField::DomainSet,
            value: self.tag.clone(),
        });
        Ok(ExecutorOutcome::Continue)
    }
}

/// `drop_resp`: forgets the current response and nothing else.
pub(crate) struct DropRespExecutor;

impl Executor for DropRespExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        state.apply_mutation(StateMutation::SetResponse(ResponseState::None));
        Ok(ExecutorOutcome::Continue)
    }
}

/// Parses the quick `sleep <milliseconds>` argument (`strconv.Atoi`). A
/// non-positive value is accepted and does not wait.
pub(crate) fn parse_sleep_millis(args: &str) -> Result<Duration, String> {
    let millis = args
        .parse::<i64>()
        .map_err(|_| format!("invalid sleep duration `{args}`"))?;
    Ok(Duration::from_millis(u64::try_from(millis).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use mosdns_sequence_core::ExecutionState;

    use mosdns_sequence_core::{Executor, ResponseState, StateMutation};

    use super::{
        DropRespExecutor, FastMarkConfig, FlowSetterConfig, RcodeConfig, TagSetterConfig,
        parse_sleep_millis,
    };

    fn state() -> ExecutionState {
        ExecutionState::new(
            mosdns_dns_core::QueryHeader {
                id: 1,
                qr: false,
                opcode: 0,
                qdcount: 1,
                ancount: 0,
                nscount: 0,
                arcount: 0,
            },
            mosdns_dns_core::QuestionInfo {
                qname_wire: vec![0],
                qtype: 1,
                qclass: 1,
            },
        )
    }

    #[test]
    fn fast_mark_matches_any_id_and_sets_bits_without_clearing_existing_flags() {
        let config = FastMarkConfig::parse("7 9").expect("fast mark");
        let mut state = state();
        state.fast_flags = 1 << 1;
        config
            .executor()
            .execute(&mut state)
            .expect("fast mark execute");
        assert_eq!(state.fast_flags, (1 << 1) | (1 << 7) | (1 << 9));
        assert!(config.matcher().evaluate(&state).expect("match").matched);

        state.fast_flags = 1 << 3;
        assert!(!config.matcher().evaluate(&state).expect("match").matched);
    }

    #[test]
    fn flow_setter_rejects_empty_quick_setup() {
        assert!(FlowSetterConfig::from_quick_args("").is_err());
        assert!(FlowSetterConfig::from_quick_args("group=").is_err());
    }
    /// A response header-only wire with `ancount`/`arcount` zero, an optional
    /// OPT additional record carrying `extended` in its TTL, and `low` in the
    /// header nibble.
    fn response_wire(low: u8, extended: Option<u8>) -> Vec<u8> {
        let mut wire = vec![0x12, 0x34, 0x81, 0x80 | low, 0, 0, 0, 0, 0, 0, 0, 0];
        if let Some(extended) = extended {
            wire[11] = 1;
            // root owner, TYPE OPT, class (udp size), TTL = ext rcode in the top byte, no RDATA
            wire.extend_from_slice(&[0, 0, 41, 0x10, 0, extended, 0, 0, 0, 0, 0]);
        }
        wire
    }

    fn matches(config: &RcodeConfig, state: &ExecutionState) -> bool {
        config
            .matcher()
            .evaluate(state)
            .expect("rcode match")
            .matched
    }

    #[test]
    fn rcode_matches_the_response_code_and_never_a_missing_response() {
        let config = RcodeConfig::parse("0 3").expect("rcode");
        let mut state = state();
        // No response yet: no match, even for rcode 0.
        assert!(!matches(&config, &state));

        state.set_raw_response(response_wire(0, None));
        assert!(matches(&config, &state));
        state.set_raw_response(response_wire(3, None));
        assert!(matches(&config, &state));
        state.set_raw_response(response_wire(2, None));
        assert!(!matches(&config, &state));

        // A synthesized response (`reject N`) carries its rcode, including
        // values above the four header bits.
        state.set_synthesized_response(5).expect("rcode 5");
        assert!(!matches(&config, &state));
        assert!(matches(&RcodeConfig::parse("5").unwrap(), &state));
        state.set_synthesized_response(0x0fff).expect("rcode max");
        assert!(matches(&RcodeConfig::parse("4095").unwrap(), &state));

        // The EDNS extended bits join the header nibble (BADVERS = 16).
        state.set_raw_response(response_wire(0, Some(1)));
        assert!(matches(&RcodeConfig::parse("16").unwrap(), &state));
        assert!(!matches(&RcodeConfig::parse("0").unwrap(), &state));
        state.set_raw_response(response_wire(3, Some(2)));
        assert!(matches(&RcodeConfig::parse("35").unwrap(), &state));

        // Undecodable bytes are no response (the reference decoder fails).
        state.set_raw_response(vec![1, 2, 3]);
        assert!(!matches(&RcodeConfig::parse("0 1 2 3").unwrap(), &state));
        let mut torn = response_wire(0, Some(0));
        torn.truncate(torn.len() - 3);
        state.set_raw_response(torn);
        assert!(!matches(&RcodeConfig::parse("0").unwrap(), &state));
    }

    #[test]
    fn rcode_arguments_follow_atoi() {
        assert_eq!(
            RcodeConfig::parse("").unwrap(),
            RcodeConfig { codes: vec![] }
        );
        assert_eq!(
            RcodeConfig::parse(" 0\t+3  -1 ").unwrap().codes,
            vec![0, 3, -1]
        );
        for bad in ["x", "1.5", "0x3", "1_0", "99999999999999999999", "0 two"] {
            assert!(RcodeConfig::parse(bad).is_err(), "`{bad}`");
        }
        // A negative or oversized code is legal and simply never matches.
        let mut state = state();
        state.set_raw_response(response_wire(0, None));
        assert!(!matches(
            &RcodeConfig::parse("-1 4096 70000").unwrap(),
            &state
        ));
    }

    #[test]
    fn tag_setter_overwrites_clears_and_leaves_other_routing_alone() {
        let mut state = state();
        state.apply_mutation(StateMutation::SetRouting {
            field: mosdns_sequence_core::RoutingField::MatchedRuleSource,
            value: Some("source".to_owned()),
        });
        TagSetterConfig::new("first")
            .executor()
            .execute(&mut state)
            .unwrap();
        assert_eq!(state.routing.domain_set.as_deref(), Some("first"));
        TagSetterConfig::new("second")
            .executor()
            .execute(&mut state)
            .unwrap();
        assert_eq!(state.routing.domain_set.as_deref(), Some("second"));
        assert_eq!(state.routing.matched_rule_source.as_deref(), Some("source"));
        TagSetterConfig::new("")
            .executor()
            .execute(&mut state)
            .unwrap();
        assert_eq!(state.routing.domain_set, None);
        assert_eq!(state.routing.matched_rule_source.as_deref(), Some("source"));
    }

    #[test]
    fn drop_resp_forgets_the_response_and_keeps_everything_else() {
        let mut state = state();
        state.set_raw_response(response_wire(0, None));
        state.fast_flags = 1 << 4;
        state.apply_mutation(StateMutation::AddMark(9));
        state.apply_mutation(StateMutation::SetRouting {
            field: mosdns_sequence_core::RoutingField::DomainSet,
            value: Some("tag".to_owned()),
        });
        DropRespExecutor.execute(&mut state).unwrap();
        assert_eq!(state.response, ResponseState::None);
        assert_eq!(state.fast_flags, 1 << 4);
        assert!(state.marks.contains(&9));
        assert_eq!(state.routing.domain_set.as_deref(), Some("tag"));
        // Dropping nothing is fine.
        DropRespExecutor.execute(&mut state).unwrap();
        assert_eq!(state.response, ResponseState::None);
    }

    #[test]
    fn sleep_arguments_follow_atoi_and_clamp_non_positive_to_zero() {
        use std::time::Duration;
        assert_eq!(parse_sleep_millis("1000").unwrap(), Duration::from_secs(1));
        assert_eq!(parse_sleep_millis("+7").unwrap(), Duration::from_millis(7));
        assert_eq!(parse_sleep_millis("0").unwrap(), Duration::ZERO);
        assert_eq!(parse_sleep_millis("-5").unwrap(), Duration::ZERO);
        for bad in ["", "1s", "1.5", "1 2", "9223372036854775808"] {
            assert!(parse_sleep_millis(bad).is_err(), "`{bad}`");
        }
    }
}
