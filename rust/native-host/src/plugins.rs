use mosdns_sequence_core::{
    ExecutionState, Executor, ExecutorError, ExecutorOutcome, MatchOutcome, Matcher, MatcherError,
    RoutingField, StateMutation,
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

#[cfg(test)]
mod tests {
    use mosdns_sequence_core::ExecutionState;

    use super::{FastMarkConfig, FlowSetterConfig};

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
}
