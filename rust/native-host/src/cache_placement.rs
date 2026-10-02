//! Conservative, monotone successor effects over the validated execution graph.
use crate::config::{CompiledConfig, ConfigError, NativeTarget};
use mosdns_sequence_core::{ExecutableId, ExecutableTarget, SequenceId, ValidatedExecutable};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Eq, PartialEq)]
struct Summary {
    effects: BTreeSet<String>,
    caches: BTreeSet<ExecutableId>,
}
impl Summary {
    fn merge(&mut self, other: Self) {
        self.effects.extend(other.effects);
        self.caches.extend(other.caches);
    }
}
type Nodes = BTreeMap<(usize, usize), Summary>;
fn suffix(nodes: &Nodes, sequence: SequenceId, rule: usize) -> Summary {
    nodes
        .get(&(sequence.index(), rule))
        .cloned()
        .unwrap_or_default()
}
fn external(config: &CompiledConfig, nodes: &Nodes, id: ExecutableId) -> Summary {
    let mut summary = Summary::default();
    if config.cache_for_executable(id).is_some() {
        summary.caches.insert(id);
    }
    if let Some(policy) = config
        .response_policies
        .iter()
        .find(|policy| policy.executable == id)
    {
        if matches!(&policy.policy, crate::policy::ResponsePolicy::Ecs(ecs) if ecs.active) {
            summary
                .effects
                .insert(format!("ecs_handler `{}`", policy.tag));
        }
    }
    if let Some(policy) = config
        .fallbacks
        .iter()
        .find(|policy| policy.executable == id)
    {
        summary.merge(target(config, nodes, policy.primary));
        summary.merge(target(config, nodes, policy.secondary));
    }
    summary
}
fn target(config: &CompiledConfig, nodes: &Nodes, target: NativeTarget) -> Summary {
    match target {
        NativeTarget::Sequence(sequence) => suffix(nodes, sequence, 0),
        NativeTarget::External(id) => external(config, nodes, id),
        NativeTarget::Fixture(_) => Summary::default(),
    }
}
fn executable(config: &CompiledConfig, nodes: &Nodes, exec: &ValidatedExecutable) -> Summary {
    match *exec {
        ValidatedExecutable::Call { target }
        | ValidatedExecutable::Inline { target }
        | ValidatedExecutable::Jump { target }
        | ValidatedExecutable::Goto { target } => suffix(nodes, target, 0),
        ValidatedExecutable::External { target } => external(config, nodes, target),
        ValidatedExecutable::Try {
            target: ExecutableTarget::Sequence(sequence),
        } => suffix(nodes, sequence, 0),
        ValidatedExecutable::Try {
            target: ExecutableTarget::Fixture(id),
        } => external(config, nodes, id),
        _ => Summary::default(),
    }
}
fn terminal(exec: Option<&ValidatedExecutable>) -> bool {
    matches!(
        exec,
        Some(
            ValidatedExecutable::Accept
                | ValidatedExecutable::Reject { .. }
                | ValidatedExecutable::Return
                | ValidatedExecutable::Exit
                | ValidatedExecutable::Goto { .. }
        )
    )
}
pub(crate) fn validate(
    config: &CompiledConfig,
    client_rules: &BTreeSet<String>,
) -> Result<(), ConfigError> {
    let mut nodes = Nodes::new();
    loop {
        let previous = nodes.clone();
        for sequence in &config.program.sequences {
            for (index, rule) in sequence.rules.iter().enumerate().rev() {
                let mut summary = Summary::default();
                if rule
                    .audit_source
                    .as_ref()
                    .is_some_and(|source| client_rules.contains(source))
                {
                    summary
                        .effects
                        .insert(format!("client_ip at {} rule {index}", sequence.name));
                }
                if let Some(exec) = &rule.executable {
                    summary.merge(executable(config, &previous, exec));
                }
                if !rule.matchers.is_empty() || !terminal(rule.executable.as_ref()) {
                    summary.merge(suffix(&previous, sequence.id, index + 1));
                }
                nodes.insert((sequence.id.index(), index), summary);
            }
        }
        if previous == nodes {
            break;
        }
    }
    for sequence in &config.program.sequences {
        for (index, rule) in sequence.rules.iter().enumerate() {
            let next = suffix(&nodes, sequence.id, index + 1);
            let Some(exec) = &rule.executable else {
                continue;
            };
            // A normal child owns its boundary. Fallback targets and direct try
            // dispatches can run an inherited successor and need its effects too.
            let mut dispatched = Summary::default();
            match *exec {
                ValidatedExecutable::External { target: id }
                | ValidatedExecutable::Try {
                    target: ExecutableTarget::Fixture(id),
                } => {
                    if config.cache_for_executable(id).is_some() {
                        dispatched.caches.insert(id);
                    }
                    if let Some(policy) = config
                        .fallbacks
                        .iter()
                        .find(|policy| policy.executable == id)
                    {
                        dispatched.merge(target(config, &nodes, policy.primary));
                        dispatched.merge(target(config, &nodes, policy.secondary));
                    }
                }
                ValidatedExecutable::Jump { target: sequence } => {
                    dispatched.merge(suffix(&nodes, sequence, 0));
                }
                _ => {}
            }
            if let Some(effect) = next.effects.first() {
                if let Some(id) = dispatched.caches.first() {
                    let cache = config
                        .cache_for_executable(*id)
                        .ok_or_else(|| ConfigError::new("$.plugins", "unresolved cache effect"))?;
                    return Err(ConfigError::new(
                        "$.plugins[sequence].args",
                        format!(
                            "unsafe cache `{}` at {} rule {index}: successor may execute {effect}; move policy before cache",
                            cache.tag, sequence.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}
