use std::collections::BTreeSet;

use mosdns_dns_core::{QueryHeader, QuestionInfo, ResponseError as DnsResponseError, TtlInfo};

const MAX_SYNTHESIZED_RCODE: u16 = 0x0fff;

/// Owned query data and the typed state carried by a sequence invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionState {
    pub query: QueryState,
    pub marks: BTreeSet<u32>,
    pub fast_flags: u64,
    pub response: ResponseState,
    pub routing: RoutingState,
}

impl ExecutionState {
    #[must_use]
    pub fn new(header: QueryHeader, question: QuestionInfo) -> Self {
        Self {
            query: QueryState { header, question },
            marks: BTreeSet::new(),
            fast_flags: 0,
            response: ResponseState::None,
            routing: RoutingState::default(),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            query: self.query.clone(),
            marks: self.marks.iter().copied().collect(),
            fast_flags: self.fast_flags,
            response: self.response.clone(),
            routing: self.routing.clone(),
        }
    }

    pub fn set_response(&mut self, response: ResponseState) {
        self.response = response;
    }

    pub fn set_raw_response(&mut self, wire: Vec<u8>) {
        self.set_response(ResponseState::Raw(OwnedResponseWire(wire)));
    }

    /// Sets a synthesized response after validating its configured RCODE.
    ///
    /// # Errors
    ///
    /// Returns [`ResponseError::InvalidRcode`] when `rcode` is outside the
    /// supported `0..=0x0fff` range.
    pub fn set_synthesized_response(&mut self, rcode: u16) -> Result<(), ResponseError> {
        self.set_response(ResponseState::Synthesized(SynthesizedResponse::new(rcode)?));
        Ok(())
    }

    pub fn apply_mutation(&mut self, mutation: StateMutation) {
        match mutation {
            StateMutation::AddMark(mark) => {
                self.marks.insert(mark);
            }
            StateMutation::RemoveMark(mark) => {
                self.marks.remove(&mark);
            }
            StateMutation::SetFastFlags(flags) => {
                self.fast_flags = flags;
            }
            StateMutation::SetRouting { field, value } => {
                self.routing.set(field, value);
            }
            StateMutation::SetResponse(response) => {
                self.set_response(response);
            }
        }
    }

    /// Inspects a raw response without consuming or replacing its wire bytes.
    ///
    /// `None` and synthesized responses do not require wire inspection and
    /// return `Ok(None)`.
    ///
    /// # Errors
    ///
    /// Returns the inspector's typed error when the owned raw wire is
    /// malformed.
    pub fn inspect_response<I: ResponseInspector + ?Sized>(
        &self,
        inspector: &I,
    ) -> Result<Option<ResponseInspection>, ResponseError> {
        match &self.response {
            ResponseState::None | ResponseState::Synthesized(_) => Ok(None),
            ResponseState::Raw(wire) => inspector.inspect(&wire.0).map(Some),
        }
    }
}

/// The owned query/question portion of [`ExecutionState`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryState {
    pub header: QueryHeader,
    pub question: QuestionInfo,
}

/// Typed routing/audit fields used by the sequence contract.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoutingState {
    pub domain_set: Option<String>,
    pub matched_group: Option<String>,
    pub final_sequence: Option<String>,
    pub final_upstream: Option<String>,
    pub final_upstream_targets: Option<String>,
    pub selected_upstream: Option<String>,
    pub matched_rule_source: Option<String>,
}

impl RoutingState {
    fn set(&mut self, field: RoutingField, value: Option<String>) {
        match field {
            RoutingField::DomainSet => self.domain_set = value,
            RoutingField::MatchedGroup => self.matched_group = value,
            RoutingField::FinalSequence => self.final_sequence = value,
            RoutingField::FinalUpstream => self.final_upstream = value,
            RoutingField::FinalUpstreamTargets => self.final_upstream_targets = value,
            RoutingField::SelectedUpstream => self.selected_upstream = value,
            RoutingField::MatchedRuleSource => self.matched_rule_source = value,
        }
    }
}

/// A closed set of routing fields that a matcher may update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingField {
    DomainSet,
    MatchedGroup,
    FinalSequence,
    FinalUpstream,
    FinalUpstreamTargets,
    SelectedUpstream,
    MatchedRuleSource,
}

/// The complete response state; raw bytes are never replaced by a lossy
/// decoded representation during inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseState {
    None,
    Raw(OwnedResponseWire),
    Synthesized(SynthesizedResponse),
}

impl ResponseState {
    #[must_use]
    pub fn synthesized_rcode(&self) -> Option<u16> {
        match self {
            Self::Synthesized(response) => Some(response.rcode),
            Self::None | Self::Raw(_) => None,
        }
    }
}

/// Caller-owned raw DNS response bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedResponseWire(pub Vec<u8>);

impl OwnedResponseWire {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A locally synthesized response. RCODE validation is intentionally wider
/// than the four-bit DNS header because configured `MosDNS` reject values use
/// the range `0..=0x0fff`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SynthesizedResponse {
    rcode: u16,
}

impl SynthesizedResponse {
    /// Creates a synthesized response for a configured RCODE.
    ///
    /// # Errors
    ///
    /// Returns [`ResponseError::InvalidRcode`] when `rcode` is outside the
    /// supported `0..=0x0fff` range.
    pub fn new(rcode: u16) -> Result<Self, ResponseError> {
        if rcode > MAX_SYNTHESIZED_RCODE {
            return Err(ResponseError::InvalidRcode(rcode));
        }
        Ok(Self { rcode })
    }

    #[must_use]
    pub fn rcode(&self) -> u16 {
        self.rcode
    }
}

/// A canonical, deterministic observation of the caller-owned state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateSnapshot {
    pub query: QueryState,
    pub marks: Vec<u32>,
    pub fast_flags: u64,
    pub response: ResponseState,
    pub routing: RoutingState,
}

/// Typed matcher-produced state changes. Matchers cannot mutate state
/// directly; the dispatcher applies this closed mutation set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateMutation {
    AddMark(u32),
    RemoveMark(u32),
    SetFastFlags(u64),
    SetRouting {
        field: RoutingField,
        value: Option<String>,
    },
    SetResponse(ResponseState),
}

/// The only response inspection error exposed by the sequence state layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    InvalidRcode(u16),
    MalformedRawResponse { source: DnsResponseError },
}

/// The small inspection seam required by `Phase3B`. Implementations return a
/// TTL observation but do not take ownership of or modify the wire.
pub trait ResponseInspector {
    /// Inspects a complete raw response without taking ownership of it.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the wire cannot be inspected.
    fn inspect(&self, wire: &[u8]) -> Result<ResponseInspection, ResponseError>;
}

/// The production-free inspector backed by the already-reviewed dns-core
/// response walk.
#[derive(Clone, Copy, Debug, Default)]
pub struct DnsResponseInspector;

impl ResponseInspector for DnsResponseInspector {
    fn inspect(&self, wire: &[u8]) -> Result<ResponseInspection, ResponseError> {
        let ttl = mosdns_dns_core::observe_response_ttl(wire)
            .map_err(|source| ResponseError::MalformedRawResponse { source })?;
        Ok(ResponseInspection { ttl })
    }
}

/// The supported, non-consuming response observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseInspection {
    pub ttl: TtlInfo,
}
