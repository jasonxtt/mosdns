use std::collections::BTreeMap;

use crate::{ExecutionState, StateMutation};

const DEFAULT_REJECT_RCODE: u16 = 5;
const MAX_REJECT_RCODE: u16 = 0x0fff;

/// A stable identifier for a validated sequence.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SequenceId(pub usize);

impl SequenceId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// A stable identifier for a validated fixture executable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExecutableId(pub usize);

impl ExecutableId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// A symbolic reference to a user-addressable sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceRef {
    pub name: String,
}

impl SequenceRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// A symbolic reference to a fixture executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureRef {
    pub name: String,
}

impl FixtureRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// A symbolic reference to an externally fulfilled executable.
///
/// External executables are resolved by the sequence program just like local
/// fixtures, but their operation is fulfilled by the host after the machine
/// yields a dispatch request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalRef {
    pub name: String,
}

impl ExternalRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// A typed matcher result. The dispatcher applies the optional mutation after
/// the matcher returns; matchers never receive mutable state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchOutcome {
    pub matched: bool,
    pub mutation: Option<StateMutation>,
}

impl MatchOutcome {
    #[must_use]
    pub const fn new(matched: bool, mutation: Option<StateMutation>) -> Self {
        Self { matched, mutation }
    }
}

/// A pure Rust matcher seam used by the sequence dispatcher and its fixtures.
pub trait Matcher {
    /// Evaluates the matcher against an immutable state view.
    ///
    /// # Errors
    ///
    /// Returns a typed matcher error when evaluation cannot complete.
    fn evaluate(&self, state: &ExecutionState) -> Result<MatchOutcome, MatcherError>;
}

/// A typed matcher failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatcherError {
    Failed(String),
}

impl MatcherError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

/// A pure Rust executable seam for named test fixtures.
pub trait Executor {
    /// Executes a fixture against caller-owned state.
    ///
    /// # Errors
    ///
    /// Returns a typed executor error when the fixture cannot complete.
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError>;
}

/// Control results that a fixture executable may return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutorOutcome {
    Continue,
    Return,
    Accept,
    Reject { rcode: u16 },
    Exit,
}

/// A typed fixture executable failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutorError {
    Failed(String),
    InvalidRcode(u16),
}

impl ExecutorError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

/// A matcher definition before program validation.
pub struct MatcherSpecInput {
    pub matcher: Option<Box<dyn Matcher>>,
    pub reverse: bool,
    pub dispatch_metadata: DispatchMetadata,
    unknown_kind: Option<String>,
}

impl MatcherSpecInput {
    #[must_use]
    pub fn new(
        matcher: Box<dyn Matcher>,
        reverse: bool,
        dispatch_metadata: DispatchMetadata,
    ) -> Self {
        Self {
            matcher: Some(matcher),
            reverse,
            dispatch_metadata,
            unknown_kind: None,
        }
    }

    pub fn unknown(kind: impl Into<String>) -> Self {
        Self {
            matcher: None,
            reverse: false,
            dispatch_metadata: DispatchMetadata::None,
            unknown_kind: Some(kind.into()),
        }
    }
}

/// A validated matcher definition with no unresolved input kind.
pub struct MatcherSpec {
    pub matcher: Box<dyn Matcher>,
    pub reverse: bool,
    pub dispatch_metadata: DispatchMetadata,
}

/// Typed dispatcher metadata for preserved routing/audit side effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchMetadata {
    None,
    AnonymousQname { rule_name: String },
    Switch6,
    Switch5,
}

/// An unvalidated sequence executable definition.
pub enum ExecutableSpec {
    Accept,
    Reject { rcode: u16 },
    Return,
    Goto { target: SequenceRef },
    Jump { target: SequenceRef },
    Exit,
    Try { target: ExecutableTargetSpec },
    Fixture { target: FixtureRef },
    External { target: ExternalRef },
    Unknown { kind: String },
}

impl ExecutableSpec {
    #[must_use]
    pub const fn default_reject() -> Self {
        Self::Reject {
            rcode: DEFAULT_REJECT_RCODE,
        }
    }
}

/// A symbolic target accepted by `try`.
pub enum ExecutableTargetSpec {
    Sequence(SequenceRef),
    Fixture(FixtureRef),
}

/// A resolved executable target in a validated program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableTarget {
    Sequence(SequenceId),
    Fixture(ExecutableId),
}

/// One unvalidated sequence rule.
pub struct RuleSpec {
    pub matchers: Vec<MatcherSpecInput>,
    /// `None` is a missing executable; `Some(vec![])` is an explicit empty
    /// executable list. Both are legal no-op forms.
    pub exec: Option<Vec<ExecutableSpec>>,
}

impl RuleSpec {
    #[must_use]
    pub fn new(matchers: Vec<MatcherSpecInput>, exec: Option<Vec<ExecutableSpec>>) -> Self {
        Self { matchers, exec }
    }

    #[must_use]
    pub fn unconditional(exec: Option<Vec<ExecutableSpec>>) -> Self {
        Self::new(Vec::new(), exec)
    }
}

/// One unvalidated sequence definition.
pub struct SequenceSpec {
    pub name: String,
    pub rules: Vec<RuleSpec>,
}

impl SequenceSpec {
    pub fn new(name: impl Into<String>, rules: Vec<RuleSpec>) -> Self {
        Self {
            name: name.into(),
            rules,
        }
    }
}

/// One named pure Rust fixture executable.
pub struct FixtureSpec {
    pub name: String,
    pub executable: Box<dyn Executor>,
}

impl FixtureSpec {
    pub fn new(name: impl Into<String>, executable: Box<dyn Executor>) -> Self {
        Self {
            name: name.into(),
            executable,
        }
    }
}

/// One named executable fulfilled outside the synchronous sequence crate.
pub struct ExternalSpec {
    pub name: String,
}

impl ExternalSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// The unvalidated program input model.
pub struct ProgramSpec {
    pub sequences: Vec<SequenceSpec>,
    pub fixtures: Vec<FixtureSpec>,
    pub externals: Vec<ExternalSpec>,
}

impl ProgramSpec {
    #[must_use]
    pub fn new(sequences: Vec<SequenceSpec>, fixtures: Vec<FixtureSpec>) -> Self {
        Self {
            sequences,
            fixtures,
            externals: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_externals(mut self, externals: Vec<ExternalSpec>) -> Self {
        self.externals = externals;
        self
    }

    /// Validates names, targets, matcher kinds, executable kinds and RCODEs,
    /// then builds the stable runtime program.
    ///
    /// # Errors
    ///
    /// Returns [`ProgramError`] when any definition is ambiguous, unknown or
    /// unresolved. No execution state is involved in this operation.
    pub fn validate(self) -> Result<ValidatedProgram, ProgramError> {
        let Self {
            sequences,
            fixtures: fixture_specs,
            externals: external_specs,
        } = self;
        let mut sequence_ids = BTreeMap::new();
        let mut sequence_specs = Vec::with_capacity(sequences.len());
        for (index, sequence) in sequences.into_iter().enumerate() {
            if sequence.name.is_empty() {
                return Err(ProgramError::EmptyName { kind: "sequence" });
            }
            if sequence_ids
                .insert(sequence.name.clone(), SequenceId(index))
                .is_some()
            {
                return Err(ProgramError::DuplicateSequenceName(sequence.name));
            }
            sequence_specs.push(Some(sequence));
        }

        let mut fixture_ids = BTreeMap::new();
        let mut fixtures = BTreeMap::new();
        for (index, fixture) in fixture_specs.into_iter().enumerate() {
            if fixture.name.is_empty() {
                return Err(ProgramError::EmptyName { kind: "fixture" });
            }
            let id = ExecutableId(index);
            if fixture_ids.insert(fixture.name.clone(), id).is_some() {
                return Err(ProgramError::DuplicateFixtureName(fixture.name));
            }
            fixtures.insert(
                id,
                ValidatedFixture {
                    id,
                    name: fixture.name,
                    executable: fixture.executable,
                },
            );
        }

        let mut external_ids = BTreeMap::new();
        let mut externals = BTreeMap::new();
        for (index, external) in external_specs.into_iter().enumerate() {
            if external.name.is_empty() {
                return Err(ProgramError::EmptyName { kind: "external" });
            }
            if fixture_ids.contains_key(&external.name)
                || external_ids
                    .insert(external.name.clone(), ExecutableId(fixtures.len() + index))
                    .is_some()
            {
                return Err(ProgramError::DuplicateExternalName(external.name));
            }
            let id = ExecutableId(fixtures.len() + index);
            externals.insert(
                id,
                ValidatedExternal {
                    id,
                    name: external.name,
                },
            );
        }

        let user_sequence_count = sequence_specs.len();
        let mut sequence_slots: Vec<Option<ValidatedSequence>> =
            (0..user_sequence_count).map(|_| None).collect();
        let mut inline_counter = 0usize;
        for index in 0..user_sequence_count {
            let Some(sequence) = sequence_specs[index].take() else {
                return Err(ProgramError::InternalInvariant);
            };
            let id = SequenceId(index);
            let rules = normalize_rules(
                sequence.rules,
                &sequence_ids,
                &fixture_ids,
                &external_ids,
                &mut sequence_slots,
                &mut inline_counter,
            )?;
            sequence_slots[index] = Some(ValidatedSequence {
                id,
                name: sequence.name,
                rules,
            });
        }

        let mut sequences = Vec::with_capacity(sequence_slots.len());
        for slot in sequence_slots {
            let Some(sequence) = slot else {
                return Err(ProgramError::InternalInvariant);
            };
            sequences.push(sequence);
        }

        Ok(ValidatedProgram {
            sequences,
            fixtures,
            externals,
            sequence_ids,
        })
    }
}

/// A validated rule with typed matchers and resolved executable targets.
pub struct ValidatedRule {
    pub matchers: Vec<MatcherSpec>,
    pub executable: Option<ValidatedExecutable>,
}

/// A validated sequence. Synthetic inline sequences have stable IDs but do
/// not appear in the public name catalog.
pub struct ValidatedSequence {
    pub id: SequenceId,
    pub name: String,
    pub rules: Vec<ValidatedRule>,
}

/// A validated fixture entry.
pub struct ValidatedFixture {
    pub id: ExecutableId,
    pub name: String,
    pub executable: Box<dyn Executor>,
}

/// A validated external executable catalog entry.
pub struct ValidatedExternal {
    pub id: ExecutableId,
    pub name: String,
}

/// Runtime executable variants with no unresolved names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatedExecutable {
    Accept,
    Reject { rcode: u16 },
    Return,
    Goto { target: SequenceId },
    Jump { target: SequenceId },
    Exit,
    Try { target: ExecutableTarget },
    Fixture { target: ExecutableId },
    External { target: ExecutableId },
    Inline { target: SequenceId },
}

/// The only program model accepted by the execution engine.
pub struct ValidatedProgram {
    pub sequences: Vec<ValidatedSequence>,
    pub fixtures: BTreeMap<ExecutableId, ValidatedFixture>,
    pub externals: BTreeMap<ExecutableId, ValidatedExternal>,
    sequence_ids: BTreeMap<String, SequenceId>,
}

impl ValidatedProgram {
    #[must_use]
    pub fn sequence_id(&self, name: &str) -> Option<SequenceId> {
        self.sequence_ids.get(name).copied()
    }

    #[must_use]
    pub fn sequence(&self, id: SequenceId) -> Option<&ValidatedSequence> {
        self.sequences.get(id.index())
    }

    #[must_use]
    pub fn fixture(&self, id: ExecutableId) -> Option<&ValidatedFixture> {
        self.fixtures.get(&id)
    }

    #[must_use]
    pub fn external(&self, id: ExecutableId) -> Option<&ValidatedExternal> {
        self.externals.get(&id)
    }
}

/// Deterministic program-construction failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProgramError {
    EmptyName { kind: &'static str },
    DuplicateSequenceName(String),
    DuplicateFixtureName(String),
    DuplicateExternalName(String),
    MissingSequence(String),
    MissingFixture(String),
    MissingExternal(String),
    MissingMatcher,
    UnknownMatcher(String),
    UnknownExecutable(String),
    InvalidRcode(u16),
    InternalInvariant,
}

fn normalize_rules(
    rules: Vec<RuleSpec>,
    sequence_ids: &BTreeMap<String, SequenceId>,
    fixture_ids: &BTreeMap<String, ExecutableId>,
    external_ids: &BTreeMap<String, ExecutableId>,
    sequence_slots: &mut Vec<Option<ValidatedSequence>>,
    inline_counter: &mut usize,
) -> Result<Vec<ValidatedRule>, ProgramError> {
    rules
        .into_iter()
        .map(|rule| {
            let matchers = rule
                .matchers
                .into_iter()
                .map(validate_matcher)
                .collect::<Result<Vec<_>, _>>()?;
            let executable = normalize_executable_list(
                rule.exec,
                sequence_ids,
                fixture_ids,
                external_ids,
                sequence_slots,
                inline_counter,
            )?;
            Ok(ValidatedRule {
                matchers,
                executable,
            })
        })
        .collect()
}

fn validate_matcher(input: MatcherSpecInput) -> Result<MatcherSpec, ProgramError> {
    let MatcherSpecInput {
        matcher,
        reverse,
        dispatch_metadata,
        unknown_kind,
    } = input;
    match matcher {
        Some(matcher) => Ok(MatcherSpec {
            matcher,
            reverse,
            dispatch_metadata,
        }),
        None => match unknown_kind {
            Some(kind) => Err(ProgramError::UnknownMatcher(kind)),
            None => Err(ProgramError::MissingMatcher),
        },
    }
}

fn normalize_executable_list(
    exec: Option<Vec<ExecutableSpec>>,
    sequence_ids: &BTreeMap<String, SequenceId>,
    fixture_ids: &BTreeMap<String, ExecutableId>,
    external_ids: &BTreeMap<String, ExecutableId>,
    sequence_slots: &mut Vec<Option<ValidatedSequence>>,
    inline_counter: &mut usize,
) -> Result<Option<ValidatedExecutable>, ProgramError> {
    let Some(exec) = exec else {
        return Ok(None);
    };
    if exec.is_empty() {
        return Ok(None);
    }
    if exec.len() == 1 {
        let executable = exec
            .into_iter()
            .next()
            .ok_or(ProgramError::InternalInvariant)?;
        return resolve_executable(executable, sequence_ids, fixture_ids, external_ids);
    }

    let target = SequenceId(sequence_slots.len());
    sequence_slots.push(None);
    let mut inline_rules = Vec::with_capacity(exec.len());
    for executable in exec {
        inline_rules.push(RuleSpec::unconditional(Some(vec![executable])));
    }
    let rules = normalize_rules(
        inline_rules,
        sequence_ids,
        fixture_ids,
        external_ids,
        sequence_slots,
        inline_counter,
    )?;
    let name = format!("<inline:{}>", *inline_counter);
    *inline_counter += 1;
    sequence_slots[target.index()] = Some(ValidatedSequence {
        id: target,
        name,
        rules,
    });
    Ok(Some(ValidatedExecutable::Inline { target }))
}

fn resolve_executable(
    executable: ExecutableSpec,
    sequence_ids: &BTreeMap<String, SequenceId>,
    fixture_ids: &BTreeMap<String, ExecutableId>,
    external_ids: &BTreeMap<String, ExecutableId>,
) -> Result<Option<ValidatedExecutable>, ProgramError> {
    let resolved = match executable {
        ExecutableSpec::Accept => ValidatedExecutable::Accept,
        ExecutableSpec::Reject { rcode } => {
            validate_rcode(rcode)?;
            ValidatedExecutable::Reject { rcode }
        }
        ExecutableSpec::Return => ValidatedExecutable::Return,
        ExecutableSpec::Goto { target } => ValidatedExecutable::Goto {
            target: resolve_sequence(target, sequence_ids)?,
        },
        ExecutableSpec::Jump { target } => ValidatedExecutable::Jump {
            target: resolve_sequence(target, sequence_ids)?,
        },
        ExecutableSpec::Exit => ValidatedExecutable::Exit,
        ExecutableSpec::Try { target } => ValidatedExecutable::Try {
            target: resolve_target(target, sequence_ids, fixture_ids)?,
        },
        ExecutableSpec::Fixture { target } => ValidatedExecutable::Fixture {
            target: resolve_fixture(target, fixture_ids)?,
        },
        ExecutableSpec::External { target } => ValidatedExecutable::External {
            target: resolve_external(target, external_ids)?,
        },
        ExecutableSpec::Unknown { kind } => return Err(ProgramError::UnknownExecutable(kind)),
    };
    Ok(Some(resolved))
}

fn resolve_sequence(
    target: SequenceRef,
    sequence_ids: &BTreeMap<String, SequenceId>,
) -> Result<SequenceId, ProgramError> {
    sequence_ids
        .get(&target.name)
        .copied()
        .ok_or(ProgramError::MissingSequence(target.name))
}

fn resolve_fixture(
    target: FixtureRef,
    fixture_ids: &BTreeMap<String, ExecutableId>,
) -> Result<ExecutableId, ProgramError> {
    fixture_ids
        .get(&target.name)
        .copied()
        .ok_or(ProgramError::MissingFixture(target.name))
}

fn resolve_external(
    target: ExternalRef,
    external_ids: &BTreeMap<String, ExecutableId>,
) -> Result<ExecutableId, ProgramError> {
    external_ids
        .get(&target.name)
        .copied()
        .ok_or(ProgramError::MissingExternal(target.name))
}

fn resolve_target(
    target: ExecutableTargetSpec,
    sequence_ids: &BTreeMap<String, SequenceId>,
    fixture_ids: &BTreeMap<String, ExecutableId>,
) -> Result<ExecutableTarget, ProgramError> {
    match target {
        ExecutableTargetSpec::Sequence(target) => {
            resolve_sequence(target, sequence_ids).map(ExecutableTarget::Sequence)
        }
        ExecutableTargetSpec::Fixture(target) => {
            resolve_fixture(target, fixture_ids).map(ExecutableTarget::Fixture)
        }
    }
}

fn validate_rcode(rcode: u16) -> Result<(), ProgramError> {
    if rcode > MAX_REJECT_RCODE {
        return Err(ProgramError::InvalidRcode(rcode));
    }
    Ok(())
}
