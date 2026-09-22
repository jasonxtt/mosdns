use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use mosdns_sequence_core::{
    ExecutableId, ExecutableSpec, ExecutionControl, ExecutionError, ExecutionMachine,
    ExecutionState, ExternalRef, ExternalSpec, ProgramSpec, RuleSpec, SequenceId, SequenceSpec,
    ValidatedProgram,
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
    pub endpoint: Endpoint,
    pub executable: ExecutableId,
}

/// A compiled sequence containing the bounded external dispatches accepted by
/// the native host (forward for W1, cache then forward for W2).
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
    if plugins.len() != 3 && plugins.len() != 4 {
        return Err(ConfigError::new(
            "$.plugins",
            "exactly one W1 forward, sequence, and listener or one W2 cache, forward, sequence, and UDP listener are required",
        ));
    }

    let mut decoded = Vec::with_capacity(plugins.len());
    for (index, plugin) in plugins.iter().enumerate() {
        decoded.push(decode_plugin(plugin, &format!("$.plugins[{index}]"))?);
    }

    let mut forward = None;
    let mut cache = None;
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
            "forward" => assign_unique(&mut forward, plugin, "forward")?,
            "cache" => assign_unique(&mut cache, plugin, "cache")?,
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

    let forward = forward.ok_or_else(|| ConfigError::new("$.plugins", "missing forward plugin"))?;
    let sequence =
        sequence.ok_or_else(|| ConfigError::new("$.plugins", "missing sequence plugin"))?;
    let listener =
        listener.ok_or_else(|| ConfigError::new("$.plugins", "missing listener plugin"))?;

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

    Ok(CompiledConfig {
        log_level: log,
        forward: ForwardConfig {
            tag: forward_tag,
            endpoint,
            executable: forward_executable,
        },
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
