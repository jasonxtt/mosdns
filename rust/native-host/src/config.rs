use std::collections::BTreeSet;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use mosdns_sequence_core::{
    DispatchMetadata, ExecutableId, ExecutableSpec, ExecutionControl, ExecutionError,
    ExecutionMachine, ExecutionState, ExternalRef, ExternalSpec, MatcherSpecInput, ProgramSpec,
    RuleSpec, SequenceId, SequenceSpec, ValidatedProgram,
};
use mosdns_upstream_core::{Endpoint, Transport};
use serde::de::{self, Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};

/// The only accepted log level in the Phase 5A host subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Error,
}

/// The listener transport accepted by the native host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListenerKind {
    Udp,
    Tcp,
}

/// A compiled forward plugin with one numeric upstream endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardConfig {
    pub tag: String,
    /// The upstream entry's explicit identity, present for the W3 graph.
    pub upstream_tag: Option<String>,
    pub endpoint: Endpoint,
    pub executable: ExecutableId,
}

/// A compiled sequence containing the bounded external dispatches accepted by
/// the native host (forward for W1, cache then forward for W2, or the strict
/// four-rule routing graph for W3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceConfig {
    pub tag: String,
    pub sequence: SequenceId,
    pub forward_executable: ExecutableId,
}

/// The bounded native cache dispatch identity used by the reviewed W2 graph.
/// W1 configurations leave this field absent until Slice 2's strict compiler
/// accepts the cache plugin shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePluginConfig {
    pub tag: String,
    pub executable: ExecutableId,
}

/// A compiled UDP or TCP listener declaration. No socket is owned here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListenerConfig {
    pub tag: String,
    pub kind: ListenerKind,
    pub entry: String,
    pub listen: SocketAddr,
    pub enable_audit: bool,
    pub idle_timeout: Option<Duration>,
}

/// The typed, validated graph consumed by pre-I/O host assembly.
pub struct CompiledConfig {
    pub log_level: LogLevel,
    pub forward: ForwardConfig,
    /// Every validated upstream owner keyed by the executable that can
    /// dispatch it. W1/W2 contain only the primary forward.
    pub forwards: Vec<ForwardConfig>,
    pub cache: Option<CachePluginConfig>,
    pub sequence: SequenceConfig,
    pub listener: ListenerConfig,
    pub program: ValidatedProgram,
}

impl CompiledConfig {
    /// Creates the canonical resumable sequence machine for one parsed query.
    /// The returned machine owns the state/control and can be held across the
    /// later upstream await without borrowing packet or executor data.
    pub fn new_machine(
        &self,
        state: ExecutionState,
        control: ExecutionControl,
    ) -> Result<ExecutionMachine<'_>, ExecutionError> {
        ExecutionMachine::new(&self.program, self.sequence.sequence, state, control)
    }
}

/// A path-aware configuration rejection. Rejections happen before host
/// assembly, so no listener or upstream owner exists when this is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    pub path: String,
    pub reason: String,
}

impl ConfigError {
    fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for ConfigError {}

/// Loads a UTF-8 YAML configuration without compiling or opening resources.
pub fn load_yaml(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|error| {
        ConfigError::new(
            path.display().to_string(),
            format!("cannot read configuration: {error}"),
        )
    })
}

/// Strictly decodes and compiles the supported Phase 5A YAML subset.
pub fn compile_yaml(yaml: &str) -> Result<CompiledConfig, ConfigError> {
    let raw: RawValue = yaml_serde::from_str(yaml)
        .map_err(|error| ConfigError::new("$", format!("invalid YAML: {error}")))?;
    compile_raw(&raw)
}

fn compile_raw(raw: &RawValue) -> Result<CompiledConfig, ConfigError> {
    let root = expect_map(raw, "$", "top level must be a mapping")?;
    root.reject_unknown(&["log", "plugins"], "$")?;
    let log = compile_log(root.required("log", "$")?)?;
    let plugins = expect_sequence(root.required("plugins", "$")?, "$.plugins")?;
    if plugins.len() != 3 && plugins.len() != 4 && plugins.len() != 6 {
        return Err(ConfigError::new(
            "$.plugins",
            "exactly one W1 forward, sequence, and listener; one W2 cache, forward, sequence, and UDP listener; or one W3 domain_set, three forwards, sequence, and UDP listener are required",
        ));
    }

    let mut decoded = Vec::with_capacity(plugins.len());
    for (index, plugin) in plugins.iter().enumerate() {
        decoded.push(decode_plugin(plugin, &format!("$.plugins[{index}]"))?);
    }

    let mut forward = None;
    let mut forward_plugins = Vec::new();
    let mut cache = None;
    let mut domain_set = None;
    let mut sequence = None;
    let mut listener = None;
    let mut tags: Vec<String> = Vec::with_capacity(decoded.len());
    for plugin in decoded {
        if let Some(existing) = tags.iter().find(|existing| existing.as_str() == plugin.tag) {
            return Err(ConfigError::new(
                "$.plugins",
                format!("duplicate plugin tag `{existing}`"),
            ));
        }
        tags.push(plugin.tag.clone());
        match plugin.kind.as_str() {
            "forward" => {
                if forward.is_none() {
                    forward = Some(plugin.clone());
                }
                forward_plugins.push(plugin);
            }
            "cache" => assign_unique(&mut cache, plugin, "cache")?,
            "domain_set" => assign_unique(&mut domain_set, plugin, "domain_set")?,
            "sequence" => assign_unique(&mut sequence, plugin, "sequence")?,
            "udp_server" | "tcp_server" => {
                if listener.is_some() {
                    return Err(ConfigError::new(
                        "$.plugins",
                        "exactly one listener plugin is supported",
                    ));
                }
                listener = Some(plugin);
            }
            other => {
                return Err(ConfigError::new(
                    "$.plugins",
                    format!("unsupported plugin type `{other}`"),
                ));
            }
        }
    }

    let sequence =
        sequence.ok_or_else(|| ConfigError::new("$.plugins", "missing sequence plugin"))?;
    let listener =
        listener.ok_or_else(|| ConfigError::new("$.plugins", "missing listener plugin"))?;

    if plugins.len() == 6 || domain_set.is_some() || forward_plugins.len() > 1 {
        return compile_w3(log, forward_plugins, domain_set, cache, sequence, listener);
    }

    let forward = forward.ok_or_else(|| ConfigError::new("$.plugins", "missing forward plugin"))?;

    let (forward_tag, endpoint) = compile_forward(&forward)?;
    if let Some(cache) = cache.as_ref() {
        compile_cache(cache)?;
    }
    let cache_tag = cache.as_ref().map(|plugin| plugin.tag.clone());
    let (sequence_tag, sequence_refs) = compile_sequence(&sequence)?;
    let expected_refs = if let Some(cache_tag) = &cache_tag {
        if !matches!(endpoint.transport(), Transport::Udp) {
            return Err(ConfigError::new(
                "$.plugins.forward.args.upstreams[0].addr",
                "W2 cache configuration requires a UDP upstream",
            ));
        }
        if listener.kind == "tcp_server" {
            return Err(ConfigError::new(
                "$.plugins.listener.type",
                "cache-enabled TCP listeners are outside the supported W2 subset",
            ));
        }
        vec![cache_tag.clone(), forward_tag.clone()]
    } else {
        vec![forward_tag.clone()]
    };
    if sequence_refs != expected_refs {
        return Err(ConfigError::new(
            "$.plugins[sequence].args",
            format!("sequence must contain exactly {:?} in order", expected_refs),
        ));
    }
    let listener = compile_listener(&listener, &sequence_tag)?;
    if listener.entry != sequence_tag {
        return Err(ConfigError::new(
            "$.plugins[listener].args.entry",
            format!("unknown sequence reference `{}`", listener.entry),
        ));
    }

    let executables = sequence_refs
        .iter()
        .cloned()
        .map(|target| ExecutableSpec::External {
            target: ExternalRef::new(target),
        })
        .collect();
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(
            sequence_tag.clone(),
            vec![RuleSpec::unconditional(Some(executables))],
        )],
        Vec::new(),
    )
    .with_externals(
        expected_refs
            .iter()
            .cloned()
            .map(ExternalSpec::new)
            .collect(),
    )
    .validate()
    .map_err(|error| {
        ConfigError::new("$.plugins", format!("sequence compile failed: {error:?}"))
    })?;

    let forward_executable = program
        .externals
        .iter()
        .find_map(|(id, external)| (external.name == forward_tag).then_some(*id))
        .ok_or_else(|| ConfigError::new("$.plugins.forward", "compiled forward is missing"))?;
    let sequence_id = program
        .sequence_id(&sequence_tag)
        .ok_or_else(|| ConfigError::new("$.plugins.sequence", "compiled sequence is missing"))?;
    let cache = cache_tag
        .map(|tag| {
            let executable = program
                .externals
                .iter()
                .find_map(|(id, external)| (external.name == tag).then_some(*id))
                .ok_or_else(|| {
                    ConfigError::new("$.plugins.cache", "compiled cache external is missing")
                })?;
            Ok(CachePluginConfig { tag, executable })
        })
        .transpose()?;

    let forward_config = ForwardConfig {
        tag: forward_tag,
        upstream_tag: None,
        endpoint,
        executable: forward_executable,
    };
    Ok(CompiledConfig {
        log_level: log,
        forwards: vec![forward_config.clone()],
        forward: forward_config,
        cache,
        sequence: SequenceConfig {
            tag: sequence_tag,
            sequence: sequence_id,
            forward_executable,
        },
        listener,
        program,
    })
}

fn compile_log(value: &RawValue) -> Result<LogLevel, ConfigError> {
    let map = expect_map(value, "$.log", "log must be a mapping")?;
    map.reject_unknown(&["level"], "$.log")?;
    let level = expect_string(map.required("level", "$.log")?, "$.log.level")?;
    if level != "error" {
        return Err(ConfigError::new(
            "$.log.level",
            "only the error level is supported",
        ));
    }
    Ok(LogLevel::Error)
}

fn decode_plugin(value: &RawValue, path: &str) -> Result<RawPlugin, ConfigError> {
    let map = expect_map(value, path, "plugin must be a mapping")?;
    map.reject_unknown(&["tag", "type", "args"], path)?;
    let tag = expect_string(map.required("tag", path)?, &format!("{path}.tag"))?;
    if tag.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.tag"),
            "tag must not be empty",
        ));
    }
    let kind = expect_string(map.required("type", path)?, &format!("{path}.type"))?;
    let args = map.required("args", path)?.clone();
    if let RawValue::Null = args {
        return Err(ConfigError::new(
            format!("{path}.args"),
            "args must be a mapping or sequence",
        ));
    }
    Ok(RawPlugin { tag, kind, args })
}

fn compile_forward(plugin: &RawPlugin) -> Result<(String, Endpoint), ConfigError> {
    let path = "$.plugins.forward.args";
    let args = expect_map(&plugin.args, path, "forward args must be a mapping")?;
    args.reject_unknown(&["upstreams"], path)?;
    let upstreams = expect_sequence(
        args.required("upstreams", path)?,
        "$.plugins.forward.args.upstreams",
    )?;
    if upstreams.len() != 1 {
        return Err(ConfigError::new(
            "$.plugins.forward.args.upstreams",
            "exactly one upstream is supported",
        ));
    }
    let upstream_path = "$.plugins.forward.args.upstreams[0]";
    let upstream = expect_map(&upstreams[0], upstream_path, "upstream must be a mapping")?;
    upstream.reject_unknown(&["addr"], upstream_path)?;
    let address = expect_string(
        upstream.required("addr", upstream_path)?,
        "$.plugins.forward.args.upstreams[0].addr",
    )?;
    let endpoint = parse_endpoint(&address, "$.plugins.forward.args.upstreams[0].addr")?;
    Ok((plugin.tag.clone(), endpoint))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum W3MatcherSpec {
    Qname { domain_set: String },
    ResponseIp(Ipv4Addr),
    True,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum W3ExecSpec {
    External(String),
    ExternalExit(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct W3RuleSpec {
    matcher: Option<W3MatcherSpec>,
    executable: W3ExecSpec,
}

fn compile_w3(
    log: LogLevel,
    forwards: Vec<RawPlugin>,
    domain_set: Option<RawPlugin>,
    cache: Option<RawPlugin>,
    sequence: RawPlugin,
    listener: RawPlugin,
) -> Result<CompiledConfig, ConfigError> {
    if forwards.len() != 3 {
        return Err(ConfigError::new(
            "$.plugins",
            "W3 requires exactly three forward plugins",
        ));
    }
    if cache.is_some() {
        return Err(ConfigError::new(
            "$.plugins",
            "W3 routing cannot be combined with cache",
        ));
    }
    let domain_set = domain_set.ok_or_else(|| {
        ConfigError::new("$.plugins", "W3 requires exactly one domain_set plugin")
    })?;
    let (domain_tag, domain) = compile_domain_set(&domain_set)?;

    let listener = compile_listener(&listener, &sequence.tag)?;
    if listener.kind != ListenerKind::Udp {
        return Err(ConfigError::new(
            "$.plugins.listener.type",
            "W3 routing supports only a UDP listener",
        ));
    }
    if listener.entry != sequence.tag {
        return Err(ConfigError::new(
            "$.plugins.listener.args.entry",
            format!("unknown sequence reference `{}`", listener.entry),
        ));
    }

    let mut forward_specs = Vec::with_capacity(forwards.len());
    let mut upstream_tags = BTreeSet::new();
    for (index, forward) in forwards.iter().enumerate() {
        let (plugin_tag, upstream_tag, endpoint) = compile_w3_forward(forward, index)?;
        if !upstream_tags.insert(upstream_tag.clone()) {
            return Err(ConfigError::new(
                "$.plugins",
                format!("duplicate W3 upstream tag `{upstream_tag}`"),
            ));
        }
        forward_specs.push((plugin_tag, upstream_tag, endpoint));
    }

    let rules = parse_w3_sequence(&sequence, &domain_tag)?;
    let (a_tag, b_tag, c_tag, response_ip) = validate_w3_topology(&rules, &forward_specs)?;
    let a_matcher = crate::matchers::FullQnameMatcher::new(&domain).map_err(|error| {
        ConfigError::new(
            "$.plugins[domain_set].args.exps[0]",
            format!("invalid full-domain matcher: {error:?}"),
        )
    })?;
    let response_matcher = crate::matchers::ResponseIpMatcher::ipv4(response_ip);
    let external = |tag: &str| ExecutableSpec::External {
        target: ExternalRef::new(tag),
    };
    let sequence_rules = vec![
        RuleSpec::new(
            vec![MatcherSpecInput::new(
                Box::new(a_matcher),
                false,
                DispatchMetadata::None,
            )],
            Some(vec![external(&a_tag), ExecutableSpec::Exit]),
        ),
        RuleSpec::unconditional(Some(vec![external(&b_tag)])),
        RuleSpec::new(
            vec![MatcherSpecInput::new(
                Box::new(response_matcher),
                false,
                DispatchMetadata::None,
            )],
            Some(vec![external(&a_tag), ExecutableSpec::Exit]),
        ),
        RuleSpec::new(
            vec![MatcherSpecInput::new(
                Box::new(crate::matchers::TrueMatcher),
                false,
                DispatchMetadata::None,
            )],
            Some(vec![external(&c_tag), ExecutableSpec::Exit]),
        ),
    ];
    let program = ProgramSpec::new(
        vec![SequenceSpec::new(sequence.tag.clone(), sequence_rules)],
        Vec::new(),
    )
    .with_externals(
        forward_specs
            .iter()
            .map(|(tag, _, _)| ExternalSpec::new(tag.clone()))
            .collect(),
    )
    .validate()
    .map_err(|error| {
        ConfigError::new(
            "$.plugins.sequence",
            format!("sequence compile failed: {error:?}"),
        )
    })?;

    let mut compiled_forwards = Vec::with_capacity(forward_specs.len());
    for (tag, upstream_tag, endpoint) in forward_specs {
        let executable = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == tag).then_some(*id))
            .ok_or_else(|| {
                ConfigError::new(
                    "$.plugins.sequence",
                    format!("compiled forward `{tag}` is missing"),
                )
            })?;
        compiled_forwards.push(ForwardConfig {
            tag,
            upstream_tag: Some(upstream_tag),
            endpoint,
            executable,
        });
    }
    let forward = compiled_forwards
        .iter()
        .find(|forward| forward.tag == a_tag)
        .cloned()
        .ok_or_else(|| ConfigError::new("$.plugins.sequence", "route A forward is missing"))?;
    let sequence_id = program
        .sequence_id(&sequence.tag)
        .ok_or_else(|| ConfigError::new("$.plugins.sequence", "compiled sequence is missing"))?;

    Ok(CompiledConfig {
        log_level: log,
        forward: forward.clone(),
        forwards: compiled_forwards,
        cache: None,
        sequence: SequenceConfig {
            tag: sequence.tag,
            sequence: sequence_id,
            forward_executable: forward.executable,
        },
        listener,
        program,
    })
}

fn compile_domain_set(plugin: &RawPlugin) -> Result<(String, String), ConfigError> {
    let path = "$.plugins.domain_set.args";
    let args = expect_map(&plugin.args, path, "domain_set args must be a mapping")?;
    args.reject_unknown(&["exps"], path)?;
    let expressions = expect_sequence(
        args.required("exps", path)?,
        "$.plugins.domain_set.args.exps",
    )?;
    if expressions.len() != 1 {
        return Err(ConfigError::new(
            "$.plugins.domain_set.args.exps",
            "exactly one full-domain expression is supported",
        ));
    }
    let expression = expect_string(&expressions[0], "$.plugins.domain_set.args.exps[0]")?;
    let domain = expression.strip_prefix("full:").ok_or_else(|| {
        ConfigError::new(
            "$.plugins.domain_set.args.exps[0]",
            "only full:<ASCII-domain> expressions are supported",
        )
    })?;
    if domain.is_empty() {
        return Err(ConfigError::new(
            "$.plugins.domain_set.args.exps[0]",
            "full-domain expression must not be empty",
        ));
    }
    Ok((plugin.tag.clone(), domain.to_owned()))
}

fn compile_w3_forward(
    plugin: &RawPlugin,
    index: usize,
) -> Result<(String, String, Endpoint), ConfigError> {
    let path = format!("$.plugins.forward[{index}].args");
    let args = expect_map(&plugin.args, &path, "forward args must be a mapping")?;
    args.reject_unknown(&["upstreams"], &path)?;
    let upstream_path = format!("{path}.upstreams");
    let upstreams = expect_sequence(args.required("upstreams", &path)?, &upstream_path)?;
    if upstreams.len() != 1 {
        return Err(ConfigError::new(
            upstream_path,
            "exactly one upstream is supported",
        ));
    }
    let item_path = format!("{path}.upstreams[0]");
    let upstream = expect_map(&upstreams[0], &item_path, "upstream must be a mapping")?;
    upstream.reject_unknown(&["tag", "addr"], &item_path)?;
    let upstream_tag = expect_string(
        upstream.required("tag", &item_path)?,
        &format!("{item_path}.tag"),
    )?;
    if upstream_tag.is_empty() {
        return Err(ConfigError::new(
            format!("{item_path}.tag"),
            "upstream tag must not be empty",
        ));
    }
    let address = expect_string(
        upstream.required("addr", &item_path)?,
        &format!("{item_path}.addr"),
    )?;
    let endpoint = parse_endpoint(&address, &format!("{item_path}.addr"))?;
    if endpoint.transport() != Transport::Udp {
        return Err(ConfigError::new(
            format!("{item_path}.addr"),
            "W3 forwards require UDP upstreams",
        ));
    }
    Ok((plugin.tag.clone(), upstream_tag, endpoint))
}

fn parse_w3_sequence(plugin: &RawPlugin, domain_tag: &str) -> Result<Vec<W3RuleSpec>, ConfigError> {
    let path = "$.plugins.sequence.args";
    let args = expect_sequence(&plugin.args, path)?;
    if args.len() != 4 {
        return Err(ConfigError::new(
            path,
            "W3 sequence must contain exactly four rules",
        ));
    }
    args.iter()
        .enumerate()
        .map(|(index, value)| {
            let item_path = format!("{path}[{index}]");
            let item = expect_map(value, &item_path, "sequence rule must be a mapping")?;
            item.reject_unknown(&["matches", "exec"], &item_path)?;
            let matcher = item
                .get("matches")
                .map(|value| {
                    let matcher_path = format!("{item_path}.matches");
                    let matchers = expect_sequence(value, &matcher_path)?;
                    if matchers.len() != 1 {
                        return Err(ConfigError::new(
                            matcher_path,
                            "each conditional W3 rule must contain exactly one matcher",
                        ));
                    }
                    let expression =
                        expect_string(&matchers[0], &format!("{item_path}.matches[0]"))?;
                    parse_w3_matcher(&expression, domain_tag, &format!("{item_path}.matches[0]"))
                })
                .transpose()?;
            let executable = parse_w3_exec(
                item.required("exec", &item_path)?,
                &format!("{item_path}.exec"),
            )?;
            Ok(W3RuleSpec {
                matcher,
                executable,
            })
        })
        .collect()
}

fn parse_w3_matcher(
    expression: &str,
    domain_tag: &str,
    path: &str,
) -> Result<W3MatcherSpec, ConfigError> {
    if let Some(target) = expression.strip_prefix("qname $") {
        if target == domain_tag {
            return Ok(W3MatcherSpec::Qname {
                domain_set: target.to_owned(),
            });
        }
        return Err(ConfigError::new(
            path,
            format!("qname matcher must reference domain_set `${domain_tag}`"),
        ));
    }
    if let Some(address) = expression.strip_prefix("resp_ip ") {
        let address = address
            .parse::<Ipv4Addr>()
            .map_err(|_| ConfigError::new(path, "resp_ip matcher requires one IPv4 literal"))?;
        return Ok(W3MatcherSpec::ResponseIp(address));
    }
    if expression == "_true" {
        return Ok(W3MatcherSpec::True);
    }
    Err(ConfigError::new(
        path,
        "supported matchers are qname $<domain_set>, resp_ip <IPv4>, and _true",
    ))
}

fn parse_w3_exec(value: &RawValue, path: &str) -> Result<W3ExecSpec, ConfigError> {
    match value {
        RawValue::String(value) => Ok(W3ExecSpec::External(parse_w3_ref(value, path)?)),
        RawValue::Sequence(values) => {
            if values.len() != 2 {
                return Err(ConfigError::new(
                    path,
                    "a W3 exec list must be [\"$forward\", \"exit\"]",
                ));
            }
            let target = expect_string(&values[0], &format!("{path}[0]"))?;
            let terminal = expect_string(&values[1], &format!("{path}[1]"))?;
            if terminal != "exit" {
                return Err(ConfigError::new(
                    format!("{path}[1]"),
                    "W3 exec lists must terminate with exit",
                ));
            }
            Ok(W3ExecSpec::ExternalExit(parse_w3_ref(
                &target,
                &format!("{path}[0]"),
            )?))
        }
        _ => Err(ConfigError::new(
            path,
            "exec must be a $forward reference or [$forward, exit]",
        )),
    }
}

fn parse_w3_ref(value: &str, path: &str) -> Result<String, ConfigError> {
    let Some(target) = value.strip_prefix('$') else {
        return Err(ConfigError::new(
            path,
            "W3 exec references must start with `$`",
        ));
    };
    if target.is_empty() || target.chars().any(char::is_whitespace) {
        return Err(ConfigError::new(
            path,
            "W3 exec reference must name one forward",
        ));
    }
    Ok(target.to_owned())
}

fn validate_w3_topology(
    rules: &[W3RuleSpec],
    forwards: &[(String, String, Endpoint)],
) -> Result<(String, String, String, Ipv4Addr), ConfigError> {
    let Some(first) = rules.first() else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args",
            "W3 sequence is empty",
        ));
    };
    let (Some(W3MatcherSpec::Qname { .. }), W3ExecSpec::ExternalExit(a_tag)) =
        (&first.matcher, &first.executable)
    else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args[0]",
            "first W3 rule must be qname with [forward, exit]",
        ));
    };
    let Some(second) = rules.get(1) else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args",
            "W3 sequence is missing its intermediate forward",
        ));
    };
    let (None, W3ExecSpec::External(b_tag)) = (&second.matcher, &second.executable) else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args[1]",
            "second W3 rule must be an unconditional forward",
        ));
    };
    let Some(third) = rules.get(2) else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args",
            "W3 sequence is missing its response-IP branch",
        ));
    };
    let (Some(W3MatcherSpec::ResponseIp(response_ip)), W3ExecSpec::ExternalExit(hit_tag)) =
        (&third.matcher, &third.executable)
    else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args[2]",
            "third W3 rule must be resp_ip with [same forward, exit]",
        ));
    };
    if a_tag != hit_tag {
        return Err(ConfigError::new(
            "$.plugins.sequence.args[2].exec",
            "response-IP hit must select the same forward as the qname hit",
        ));
    }
    let Some(fourth) = rules.get(3) else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args",
            "W3 sequence is missing its final branch",
        ));
    };
    let (Some(W3MatcherSpec::True), W3ExecSpec::ExternalExit(c_tag)) =
        (&fourth.matcher, &fourth.executable)
    else {
        return Err(ConfigError::new(
            "$.plugins.sequence.args[3]",
            "fourth W3 rule must be _true with [forward, exit]",
        ));
    };
    if a_tag == b_tag || a_tag == c_tag || b_tag == c_tag {
        return Err(ConfigError::new(
            "$.plugins.sequence.args",
            "W3 route A, B, and C must be distinct forwards",
        ));
    }
    let known = forwards
        .iter()
        .map(|(tag, _, _)| tag.as_str())
        .collect::<BTreeSet<_>>();
    for (path, tag) in [
        ("$.plugins.sequence.args[0].exec", a_tag),
        ("$.plugins.sequence.args[1].exec", b_tag),
        ("$.plugins.sequence.args[3].exec", c_tag),
    ] {
        if !known.contains(tag.as_str()) {
            return Err(ConfigError::new(
                path,
                format!("unknown forward reference `${tag}`"),
            ));
        }
    }
    Ok((a_tag.clone(), b_tag.clone(), c_tag.clone(), *response_ip))
}

fn compile_cache(plugin: &RawPlugin) -> Result<(), ConfigError> {
    let path = "$.plugins.cache.args";
    let args = expect_map(&plugin.args, path, "cache args must be a mapping")?;
    args.reject_unknown(&["size", "lazy_cache_ttl"], path)?;
    let size = expect_nonnegative_integer(args.required("size", path)?, &format!("{path}.size"))?;
    if size != 64 {
        return Err(ConfigError::new(
            format!("{path}.size"),
            "only cache size 64 is supported",
        ));
    }
    let lazy_cache_ttl = expect_nonnegative_integer(
        args.required("lazy_cache_ttl", path)?,
        &format!("{path}.lazy_cache_ttl"),
    )?;
    if lazy_cache_ttl != 0 {
        return Err(ConfigError::new(
            format!("{path}.lazy_cache_ttl"),
            "lazy_cache_ttl must be exactly 0",
        ));
    }
    Ok(())
}

fn compile_sequence(plugin: &RawPlugin) -> Result<(String, Vec<String>), ConfigError> {
    let path = "$.plugins.sequence.args";
    let args = expect_sequence(&plugin.args, path)?;
    if args.is_empty() || args.len() > 2 {
        return Err(ConfigError::new(
            path,
            "one W1 or two W2 unconditional executables are supported",
        ));
    }
    let mut refs = Vec::with_capacity(args.len());
    for (index, value) in args.iter().enumerate() {
        let item_path = format!("$.plugins.sequence.args[{index}]");
        let item = expect_map(value, &item_path, "sequence item must be a mapping")?;
        item.reject_unknown(&["exec"], &item_path)?;
        let exec_path = format!("{item_path}.exec");
        let exec = expect_string(item.required("exec", &item_path)?, &exec_path)?;
        let Some(target) = exec.strip_prefix('$') else {
            return Err(ConfigError::new(
                exec_path,
                "only named $ references are supported",
            ));
        };
        if target.is_empty() {
            return Err(ConfigError::new(exec_path, "reference must not be empty"));
        }
        refs.push(target.to_owned());
    }
    Ok((plugin.tag.clone(), refs))
}

fn compile_listener(
    plugin: &RawPlugin,
    _sequence_tag: &str,
) -> Result<ListenerConfig, ConfigError> {
    let is_tcp = plugin.kind == "tcp_server";
    let path = if is_tcp {
        "$.plugins.tcp_server.args"
    } else {
        "$.plugins.udp_server.args"
    };
    let args = expect_map(&plugin.args, path, "listener args must be a mapping")?;
    let allowed = if is_tcp {
        &["entry", "listen", "enable_audit", "idle_timeout"][..]
    } else {
        &["entry", "listen", "enable_audit"][..]
    };
    args.reject_unknown(allowed, path)?;
    let entry = expect_string(args.required("entry", path)?, &format!("{path}.entry"))?;
    if entry.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.entry"),
            "entry must not be empty",
        ));
    }
    let listen = expect_string(args.required("listen", path)?, &format!("{path}.listen"))?;
    let listen = parse_socket_addr(&listen, &format!("{path}.listen"))?;
    let enable_audit = expect_bool(
        args.required("enable_audit", path)?,
        &format!("{path}.enable_audit"),
    )?;
    if enable_audit {
        return Err(ConfigError::new(
            format!("{path}.enable_audit"),
            "audit is outside the supported subset; use false",
        ));
    }
    let idle_timeout = if is_tcp {
        let timeout = expect_positive_integer(
            args.required("idle_timeout", path)?,
            &format!("{path}.idle_timeout"),
        )?;
        Some(Duration::from_secs(timeout))
    } else {
        None
    };
    Ok(ListenerConfig {
        tag: plugin.tag.clone(),
        kind: if is_tcp {
            ListenerKind::Tcp
        } else {
            ListenerKind::Udp
        },
        entry,
        listen,
        enable_audit,
        idle_timeout,
    })
}

fn parse_endpoint(value: &str, path: &str) -> Result<Endpoint, ConfigError> {
    let Some((scheme, address)) = value.split_once("://") else {
        return Err(ConfigError::new(
            path,
            "upstream must use udp:// or tcp:// with a numeric SocketAddr",
        ));
    };
    let transport = match scheme {
        "udp" => Transport::Udp,
        "tcp" => Transport::Tcp,
        other => {
            return Err(ConfigError::new(
                path,
                format!("unsupported upstream scheme `{other}`"),
            ));
        }
    };
    let socket = parse_socket_addr(address, path)?;
    Endpoint::new(socket, transport)
        .map_err(|error| ConfigError::new(path, format!("invalid upstream endpoint: {error}")))
}

fn parse_socket_addr(value: &str, path: &str) -> Result<SocketAddr, ConfigError> {
    let socket = value.parse::<SocketAddr>().map_err(|_| {
        ConfigError::new(
            path,
            "only a numeric IP SocketAddr with a nonzero port is supported",
        )
    })?;
    if socket.port() == 0 {
        return Err(ConfigError::new(path, "port must be nonzero"));
    }
    Ok(socket)
}

fn assign_unique(
    slot: &mut Option<RawPlugin>,
    plugin: RawPlugin,
    role: &str,
) -> Result<(), ConfigError> {
    if let Some(existing) = slot {
        return Err(ConfigError::new(
            "$.plugins",
            format!(
                "duplicate {role} plugin tags `{}` and `{}`",
                existing.tag, plugin.tag
            ),
        ));
    }
    *slot = Some(plugin);
    Ok(())
}

fn expect_map<'a>(
    value: &'a RawValue,
    path: &str,
    reason: &str,
) -> Result<&'a RawMap, ConfigError> {
    match value {
        RawValue::Map(map) => Ok(map),
        _ => Err(ConfigError::new(path, reason)),
    }
}

fn expect_sequence<'a>(value: &'a RawValue, path: &str) -> Result<&'a [RawValue], ConfigError> {
    match value {
        RawValue::Sequence(sequence) => Ok(sequence),
        _ => Err(ConfigError::new(path, "expected a sequence")),
    }
}

fn expect_string(value: &RawValue, path: &str) -> Result<String, ConfigError> {
    match value {
        RawValue::String(value) => Ok(value.clone()),
        _ => Err(ConfigError::new(path, "expected a string")),
    }
}

fn expect_bool(value: &RawValue, path: &str) -> Result<bool, ConfigError> {
    match value {
        RawValue::Bool(value) => Ok(*value),
        _ => Err(ConfigError::new(path, "expected a boolean")),
    }
}

fn expect_positive_integer(value: &RawValue, path: &str) -> Result<u64, ConfigError> {
    match value {
        RawValue::Number(RawNumber::Unsigned(value)) if *value > 0 => Ok(*value),
        RawValue::Number(RawNumber::Signed(value)) if *value > 0 => Ok(*value as u64),
        _ => Err(ConfigError::new(path, "expected a positive integer")),
    }
}

fn expect_nonnegative_integer(value: &RawValue, path: &str) -> Result<u64, ConfigError> {
    match value {
        RawValue::Number(RawNumber::Unsigned(value)) => Ok(*value),
        RawValue::Number(RawNumber::Signed(value)) if *value >= 0 => Ok(*value as u64),
        _ => Err(ConfigError::new(path, "expected a nonnegative integer")),
    }
}

#[derive(Clone, Debug)]
struct RawPlugin {
    tag: String,
    kind: String,
    args: RawValue,
}

#[derive(Clone, Debug)]
struct RawMap {
    entries: Vec<(String, RawValue)>,
}

impl RawMap {
    fn required(&self, key: &str, path: &str) -> Result<&RawValue, ConfigError> {
        self.get(key)
            .ok_or_else(|| ConfigError::new(format!("{path}.{key}"), "required field is missing"))
    }

    fn get(&self, key: &str) -> Option<&RawValue> {
        self.entries
            .iter()
            .find_map(|(entry, value)| (entry == key).then_some(value))
    }

    fn reject_unknown(&self, allowed: &[&str], path: &str) -> Result<(), ConfigError> {
        for (key, _) in &self.entries {
            if !allowed.contains(&key.as_str()) {
                return Err(ConfigError::new(
                    format!("{path}.{key}"),
                    "unsupported field",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum RawValue {
    Null,
    Bool(bool),
    Number(RawNumber),
    String(String),
    Sequence(Vec<RawValue>),
    Map(RawMap),
}

#[derive(Clone, Copy, Debug)]
enum RawNumber {
    Signed(i64),
    Unsigned(u64),
    Float,
}

struct RawValueVisitor;

impl<'de> Visitor<'de> for RawValueVisitor {
    type Value = RawValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML scalar, sequence, or mapping")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::Number(RawNumber::Signed(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::Number(RawNumber::Unsigned(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let _ = value;
        Ok(RawValue::Number(RawNumber::Float))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        i64::try_from(value)
            .map(|value| RawValue::Number(RawNumber::Signed(value)))
            .map_err(|_| E::custom("integer is outside the supported range"))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        u64::try_from(value)
            .map(|value| RawValue::Number(RawNumber::Unsigned(value)))
            .map_err(|_| E::custom("integer is outside the supported range"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::String(value))
    }

    fn visit_char<E>(self, value: char) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::String(value.to_string()))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = access.next_element::<RawValue>()? {
            values.push(value);
        }
        Ok(RawValue::Sequence(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some(key) = access.next_key::<String>()? {
            if entries.iter().any(|(existing, _)| existing == &key) {
                return Err(A::Error::custom(format!("duplicate key `{key}`")));
            }
            let value = access.next_value::<RawValue>()?;
            entries.push((key, value));
        }
        Ok(RawValue::Map(RawMap { entries }))
    }
}

impl<'de> Deserialize<'de> for RawValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mosdns_sequence_core::{ExecutionControl, ExecutionState};
    use mosdns_upstream_core::Transport;

    use super::{ListenerKind, LogLevel, compile_yaml};

    const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");
    const TCP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-tcp.yaml");

    #[test]
    fn compiles_the_frozen_udp_and_tcp_shapes_without_rewriting_them() {
        let udp = compile_yaml(UDP).expect("frozen UDP config must compile");
        assert_eq!(udp.log_level, LogLevel::Error);
        assert_eq!(udp.listener.kind, ListenerKind::Udp);
        assert_eq!(udp.listener.listen.port(), 15353);
        assert_eq!(udp.forward.endpoint.transport(), Transport::Udp);
        assert_eq!(udp.forward.endpoint.address().port(), 15453);
        assert_eq!(udp.listener.idle_timeout, None);
        assert_eq!(udp.sequence.tag, "phase5a_entry");
        assert_eq!(udp.sequence.forward_executable, udp.forward.executable);

        let tcp = compile_yaml(TCP).expect("frozen TCP config must compile");
        assert_eq!(tcp.listener.kind, ListenerKind::Tcp);
        assert_eq!(tcp.listener.listen.port(), 15354);
        assert_eq!(tcp.forward.endpoint.transport(), Transport::Tcp);
        assert_eq!(tcp.forward.endpoint.address().port(), 15454);
        assert_eq!(tcp.listener.idle_timeout, Some(Duration::from_secs(2)));
    }

    #[test]
    fn declaration_order_and_references_are_compiled_after_collection() {
        let yaml = r#"
log: { level: error }
plugins:
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: false }
  - tag: entry
    type: sequence
    args: [ { exec: "$forward" } ]
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
"#;
        let config = compile_yaml(yaml).expect("order-independent graph");
        assert_eq!(config.sequence.tag, "entry");
        assert_eq!(config.forward.tag, "forward");

        let state = ExecutionState::new(
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
        );
        let mut machine = config
            .new_machine(state, ExecutionControl::with_fuel(8))
            .expect("compiled entry must create the canonical machine");
        let step = machine.step().expect("machine should dispatch forward");
        assert!(matches!(
            step,
            mosdns_sequence_core::MachineStep::Dispatch(dispatch)
                if dispatch.executable() == config.forward.executable
        ));
    }

    #[test]
    fn rejects_every_unsupported_shape_before_assembly() {
        let cases = [
            ("top-level", "extra: true\n"),
            ("log field", "log: { level: error, format: json }\n"),
            ("log level", "log: { level: info }\n"),
            ("unknown plugin", "type: cache\n"),
            ("forward field", "concurrent: 2\n"),
            ("hostname", "addr: udp://dns.example:53\n"),
            ("zero upstream port", "addr: udp://127.0.0.1:0\n"),
            ("zero listener port", "listen: 127.0.0.1:0\n"),
            ("missing ref", "exec: $missing\n"),
            ("sequence matcher", "match: foo\n"),
            ("tcp timeout", "idle_timeout: 0\n"),
            ("duplicate key", "level: error\nlevel: error\n"),
        ];
        for (name, marker) in cases {
            let yaml = match name {
                "top-level" => format!("log: {{ level: error }}\n{marker}plugins: []\n"),
                "log field" | "log level" | "duplicate key" => {
                    format!("log: {{ {marker} }}\nplugins: []\n")
                }
                "unknown plugin" => {
                    "log: { level: error }\nplugins: [{ tag: x, type: cache, args: {} }]\n"
                        .to_owned()
                }
                "forward field" => format!(
                    "log: {{ level: error }}\nplugins: [{{ tag: f, type: forward, args: {{ upstreams: [{{ addr: udp://127.0.0.1:53, {marker} }}] }} }}, {{ tag: s, type: sequence, args: [{{ exec: $f }}] }}, {{ tag: l, type: udp_server, args: {{ entry: s, listen: 127.0.0.1:53, enable_audit: false }} }}]\n"
                ),
                "hostname" | "zero upstream port" => format!(
                    "log: {{ level: error }}\nplugins: [{{ tag: f, type: forward, args: {{ upstreams: [{{ {marker} }}] }} }}, {{ tag: s, type: sequence, args: [{{ exec: $f }}] }}, {{ tag: l, type: udp_server, args: {{ entry: s, listen: 127.0.0.1:53, enable_audit: false }} }}]\n"
                ),
                "zero listener port" => format!(
                    "log: {{ level: error }}\nplugins: [{{ tag: f, type: forward, args: {{ upstreams: [{{ addr: udp://127.0.0.1:53 }}] }} }}, {{ tag: s, type: sequence, args: [{{ exec: $f }}] }}, {{ tag: l, type: udp_server, args: {{ entry: s, {marker}, enable_audit: false }} }}]\n"
                ),
                "missing ref" | "sequence matcher" => format!(
                    "log: {{ level: error }}\nplugins: [{{ tag: f, type: forward, args: {{ upstreams: [{{ addr: udp://127.0.0.1:53 }}] }} }}, {{ tag: s, type: sequence, args: [{{ {marker} }}] }}, {{ tag: l, type: udp_server, args: {{ entry: s, listen: 127.0.0.1:53, enable_audit: false }} }}]\n"
                ),
                "tcp timeout" => format!(
                    "log: {{ level: error }}\nplugins: [{{ tag: f, type: forward, args: {{ upstreams: [{{ addr: tcp://127.0.0.1:53 }}] }} }}, {{ tag: s, type: sequence, args: [{{ exec: $f }}] }}, {{ tag: l, type: tcp_server, args: {{ entry: s, listen: 127.0.0.1:53, enable_audit: false, {marker} }} }}]\n"
                ),
                _ => unreachable!(),
            };
            assert!(compile_yaml(&yaml).is_err(), "case {name} must reject");
        }
    }

    #[test]
    fn independently_proves_the_remaining_fail_closed_categories() {
        let cases = [
            (
                "unsupported scheme",
                UDP.replace("udp://127.0.0.1:15453", "quic://127.0.0.1:15453"),
            ),
            (
                "wrong YAML type",
                UDP.replace("enable_audit: false", "enable_audit: \"false\""),
            ),
            (
                "listener missing sequence reference",
                UDP.replace("entry: phase5a_entry", "entry: missing_entry"),
            ),
            (
                "invalid sequence control form",
                UDP.replace("- exec: $phase5a_forward", "- goto: another_sequence"),
            ),
            (
                "audit true",
                UDP.replace("enable_audit: false", "enable_audit: true"),
            ),
            (
                "negative TCP timeout",
                TCP.replace("idle_timeout: 2", "idle_timeout: -1"),
            ),
            (
                "noninteger TCP timeout",
                TCP.replace("idle_timeout: 2", "idle_timeout: \"2\""),
            ),
        ];
        for (name, yaml) in cases {
            assert!(compile_yaml(&yaml).is_err(), "case {name} must reject");
        }
    }

    #[test]
    fn rejects_duplicates_and_missing_required_roles() {
        let duplicate_tag = r#"
log: { level: error }
plugins:
  - { tag: same, type: forward, args: { upstreams: [ { addr: udp://127.0.0.1:53 } ] } }
  - { tag: same, type: sequence, args: [ { exec: "$same" } ] }
  - { tag: listener, type: udp_server, args: { entry: same, listen: "127.0.0.1:53", enable_audit: false } }
"#;
        assert!(compile_yaml(duplicate_tag).is_err());

        let duplicate_role = r#"
log: { level: error }
plugins:
  - { tag: f1, type: forward, args: { upstreams: [ { addr: udp://127.0.0.1:53 } ] } }
  - { tag: f2, type: forward, args: { upstreams: [ { addr: udp://127.0.0.1:54 } ] } }
  - { tag: s, type: sequence, args: [ { exec: "$f1" } ] }
"#;
        assert!(compile_yaml(duplicate_role).is_err());
    }
}
