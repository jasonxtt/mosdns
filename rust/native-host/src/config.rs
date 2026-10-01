use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mosdns_matcher_core::MixMatcher;
use mosdns_sequence_core::{
    ExecutableId, ExecutableSpec, ExecutableTarget, ExecutableTargetSpec, ExecutionControl,
    ExecutionError, ExecutionMachine, ExecutionState, ExternalRef, ExternalSpec, MatcherSpecInput,
    ProgramSpec, RuleSpec, SequenceId, SequenceRef, SequenceSpec, ValidatedExecutable,
    ValidatedProgram,
};
use mosdns_upstream_core::{Endpoint, ServerIdentity, Transport};
use serde::de::{self, Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use url::Url;

use crate::managed::{DomainSetHandle, ManagedDomainSet};
use crate::matchers::{
    DomainSetError, HasResponseMatcher, QnameMatcher, QtypeMatcher, ResponseIpMatcher, TrueMatcher,
    build_domain_set, resolve_rule_path,
};
use crate::plugins::{FastMarkConfig, FlowSetterConfig};

const MAX_FALLBACK_THRESHOLD: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);

/// The only accepted log level in the native host subset.
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
    /// The upstream entry's explicit identity, present when the upstream
    /// declares a `tag`.
    pub upstream_tag: Option<String>,
    pub endpoint: Endpoint,
    pub executable: ExecutableId,
}

/// A definition owns entries independently of the call sites selecting them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardDefinitionConfig {
    pub tag: String,
    pub entries: Vec<ForwardEntryConfig>,
    pub concurrent: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardEntryConfig {
    pub tag: Option<String>,
    pub identity: String,
    /// Numeric dial endpoint for the legacy view, when this entry is already
    /// numeric or supplies an explicit numeric `dial_addr`.
    pub endpoint: Option<Endpoint>,
    pub target: ForwardTargetConfig,
}

/// The pre-I/O endpoint descriptor consumed by the host catalog. Service
/// identity and numeric dial are intentionally separate so a hostname can be
/// resolved later without changing TLS SNI or a DoH authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardTargetConfig {
    pub scheme: ForwardScheme,
    pub service: String,
    pub host: String,
    pub port: u16,
    pub dial_addr: Option<SocketAddr>,
    pub bootstrap: Option<SocketAddr>,
    pub bootstrap_version: Option<u8>,
    pub query_timeout: Duration,
    pub insecure_skip_verify: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardScheme {
    Udp,
    Tcp,
    Tls,
    Https,
}

impl ForwardScheme {
    const fn default_port(self) -> u16 {
        match self {
            Self::Udp | Self::Tcp => 53,
            Self::Tls => 853,
            Self::Https => 443,
        }
    }

    pub(crate) const fn endpoint_transport(self) -> Transport {
        match self {
            Self::Udp => Transport::Udp,
            Self::Tcp | Self::Tls | Self::Https => Transport::Tcp,
        }
    }
}

/// A call site's ordered subset keeps original definition indices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardInvocationConfig {
    pub executable: ExecutableId,
    pub definition: usize,
    pub entries: Vec<usize>,
}

#[derive(Default)]
struct ForwardCompiler {
    definitions: Vec<ForwardDefinitionConfig>,
    invocations: Vec<(String, usize, Vec<usize>)>,
}

impl ForwardCompiler {
    fn invocation(
        &mut self,
        tag: &str,
        args: &str,
        path: &str,
    ) -> Result<ExecutableSpec, ConfigError> {
        let definition = self
            .definitions
            .iter()
            .position(|item| item.tag == tag)
            .ok_or_else(|| ConfigError::new(path, "forward definition is missing"))?;
        let entries = if args.is_empty() {
            (0..self.definitions[definition].entries.len()).collect()
        } else {
            let mut selected = Vec::new();
            for tag in args.split_whitespace() {
                let index = self.definitions[definition]
                    .entries
                    .iter()
                    .position(|item| item.tag.as_deref() == Some(tag))
                    .ok_or_else(|| {
                        ConfigError::new(path, format!("unknown upstream entry tag `{tag}`"))
                    })?;
                if selected.contains(&index) {
                    return Err(ConfigError::new(
                        path,
                        format!("duplicate selected entry tag `{tag}`"),
                    ));
                }
                selected.push(index);
            }
            selected
        };
        let name = format!("@native-invocation:{}", self.invocations.len());
        self.invocations.push((name.clone(), definition, entries));
        Ok(ExecutableSpec::External {
            target: ExternalRef::new(name),
        })
    }

    fn quick(
        &mut self,
        sequence_tag: &str,
        rule_index: usize,
        exec_index: usize,
        args: &str,
        path: &str,
    ) -> Result<ExecutableSpec, ConfigError> {
        let addresses: Vec<&str> = args.split_whitespace().collect();
        if addresses.is_empty() {
            return Err(ConfigError::new(
                path,
                "quick forward requires one or more upstream addresses",
            ));
        }
        let definition = self.definitions.len();
        let definition_tag = format!(
            "@native-quick-definition:{}:{rule_index}:{exec_index}",
            hex_identity(sequence_tag)
        );
        let mut entries = Vec::with_capacity(addresses.len());
        for (entry_index, address) in addresses.into_iter().enumerate() {
            let target = parse_forward_target(
                address,
                &RawMap {
                    entries: Vec::new(),
                },
                None,
                None,
                &format!("{path}[{entry_index}]"),
            )?;
            let endpoint = target.dial_addr.map(|dial| {
                Endpoint::new(dial, target.scheme.endpoint_transport()).expect("validated dial")
            });
            let identity = format!(
                "@native-quick:{}:{rule_index}:{exec_index}:{entry_index}",
                hex_identity(sequence_tag)
            );
            if self
                .definitions
                .iter()
                .flat_map(|definition| definition.entries.iter())
                .any(|entry| entry.identity == identity)
                || entries
                    .iter()
                    .any(|entry: &ForwardEntryConfig| entry.identity == identity)
            {
                return Err(ConfigError::new(
                    path,
                    format!("duplicate effective upstream identity `{identity}`"),
                ));
            }
            entries.push(ForwardEntryConfig {
                tag: None,
                identity,
                endpoint,
                target,
            });
        }
        self.definitions.push(ForwardDefinitionConfig {
            tag: definition_tag,
            entries,
            concurrent: 3,
        });
        let name = format!(
            "@native-quick-invocation:{}:{rule_index}:{exec_index}",
            hex_identity(sequence_tag)
        );
        self.invocations.push((
            name.clone(),
            definition,
            (0..self.definitions[definition].entries.len()).collect(),
        ));
        Ok(ExecutableSpec::External {
            target: ExternalRef::new(name),
        })
    }
}

/// The compiled entry sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceConfig {
    pub tag: String,
    pub sequence: SequenceId,
    /// Legacy convenience view only; runtime dispatch uses `program`.
    /// Some valid entry graphs have no reachable forward.
    pub forward_executable: Option<ExecutableId>,
}

/// The one native cache dispatch accepted by this host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePluginConfig {
    pub tag: String,
    pub executable: ExecutableId,
    /// The configured entry capacity, passed to the bounded cache store.
    pub capacity: u64,
}

/// A target that a native branch policy can execute without exposing policy
/// internals to sequence-core. The name is resolved only after the complete
/// program catalog has been validated.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PolicyTargetName {
    Sequence(String),
    Fixture(String),
    External(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FallbackConfig {
    pub tag: String,
    pub executable: ExecutableId,
    pub primary: NativeTarget,
    pub secondary: NativeTarget,
    pub threshold: Duration,
    pub always_standby: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTarget {
    Sequence(SequenceId),
    Fixture(ExecutableId),
    External(ExecutableId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferenceFamily {
    Ipv4,
    Ipv6,
}

#[derive(Clone)]
pub struct PreferenceConfig {
    pub executable: ExecutableId,
    pub family: PreferenceFamily,
    pub(crate) evidence: Rc<RefCell<BTreeMap<String, Instant>>>,
    pub(crate) clock: PreferenceClock,
}

/// Monotonic time seam for positive preference evidence. Production uses the
/// process monotonic clock; tests may inject a deterministic source without
/// changing DNS/cache wall-clock behavior.
#[derive(Clone)]
pub(crate) struct PreferenceClock {
    now: Rc<dyn Fn() -> Instant>,
}

impl PreferenceClock {
    pub(crate) fn system() -> Self {
        Self {
            now: Rc::new(Instant::now),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_fn<F>(now: F) -> Self
    where
        F: Fn() -> Instant + 'static,
    {
        Self { now: Rc::new(now) }
    }

    pub(crate) fn now(&self) -> Instant {
        (self.now)()
    }
}

struct FallbackDefinition {
    tag: String,
    source_path: String,
    primary: String,
    secondary: String,
    threshold: Duration,
    always_standby: bool,
}

struct ResolvedFallbackDefinition {
    tag: String,
    source_path: String,
    primary: PolicyTargetName,
    secondary: PolicyTargetName,
    threshold: Duration,
    always_standby: bool,
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

/// The scoped management HTTP listener. Only the bounded `/plugins/{tag}`
/// routes and the read-only special-group list are served; this is not a claim
/// that any other Go API field is supported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiConfig {
    pub http: SocketAddr,
}

/// One compiled `domain_set` plus its management eligibility. Query-only
/// shapes (`exps`, several files, non-`.txt`) stay fully usable for matching
/// but are not management targets.
pub struct DomainSetConfig {
    pub tag: String,
    pub managed: Option<Rc<ManagedDomainSet>>,
    /// Why a query-only or conflicting shape is not a management target. The
    /// bounded API reports this explicitly instead of guessing.
    pub ineligible_reason: Option<String>,
    handle: DomainSetHandle,
}

impl DomainSetConfig {
    /// True when the current rule set accepts `domain`. A managed profile is
    /// read through its published generation at every evaluation.
    #[must_use]
    pub fn matches(&self, domain: &str) -> bool {
        self.handle.matches(domain)
    }

    /// The number of accepted rules in the current rule set.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.handle.rule_count()
    }
}

/// The typed, validated graph consumed by pre-I/O host assembly.
pub struct CompiledConfig {
    pub log_level: LogLevel,
    /// Legacy convenience view of one reachable forward. This value does not
    /// determine configuration validity or runtime dispatch behavior.
    pub forward: Option<ForwardConfig>,
    /// Every validated upstream owner keyed by the executable that can
    /// dispatch it.
    pub forwards: Vec<ForwardConfig>,
    pub forward_definitions: Vec<ForwardDefinitionConfig>,
    pub forward_invocations: Vec<ForwardInvocationConfig>,
    pub cache: Option<CachePluginConfig>,
    pub fallbacks: Vec<FallbackConfig>,
    pub preferences: Vec<PreferenceConfig>,
    pub sequence: SequenceConfig,
    pub listener: ListenerConfig,
    /// Every compiled `domain_set` with its management eligibility.
    pub domain_sets: Vec<DomainSetConfig>,
    /// The scoped management HTTP listener, when one is configured.
    pub api: Option<ApiConfig>,
    pub program: ValidatedProgram,
}

impl CompiledConfig {
    /// Looks up one compiled `domain_set` by tag.
    #[must_use]
    pub fn domain_set(&self, tag: &str) -> Option<&DomainSetConfig> {
        self.domain_sets.iter().find(|set| set.tag == tag)
    }

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

/// Reads and compiles one configuration file. Relative `include` paths and
/// relative rule-file paths resolve against the declaring file's directory,
/// never against the process working directory.
pub fn load_and_compile(path: &Path) -> Result<CompiledConfig, ConfigError> {
    let yaml = load_yaml(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
    compile_yaml_with_base(&yaml, base_dir)
}

/// Strictly decodes and compiles an in-memory configuration that resolves
/// relative paths against the process working directory. Prefer
/// [`load_and_compile`] for file-backed configurations so that include and
/// rule-file paths are never guessed from an unrelated working directory.
pub fn compile_yaml(yaml: &str) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base(yaml, Path::new(""))
}

/// Strictly decodes and compiles the supported YAML subset with an explicit
/// base directory for relative include and rule-file paths.
pub fn compile_yaml_with_base(yaml: &str, base_dir: &Path) -> Result<CompiledConfig, ConfigError> {
    let raw = parse_yaml(yaml)?;
    compile_raw(&raw, base_dir)
}

fn parse_yaml(yaml: &str) -> Result<RawValue, ConfigError> {
    yaml_serde::from_str(yaml)
        .map_err(|error| ConfigError::new("$", format!("invalid YAML: {error}")))
}

fn compile_raw(raw: &RawValue, base_dir: &Path) -> Result<CompiledConfig, ConfigError> {
    let root = expect_map(raw, "$", "top level must be a mapping")?;
    root.reject_unknown(&["log", "include", "plugins", "api"], "$")?;
    let log = compile_log(root.required("log", "$")?)?;
    let api = root.get("api").map(compile_api).transpose()?;

    // Definition collection: included plugin-only files load in declaration
    // order, then this file's plugins. No definition is resolved until the
    // whole ordered catalog exists, so a reference may name a later
    // definition without changing the effective order.
    let mut definitions = Vec::new();
    if let Some(includes) = root.get("include") {
        let includes = expect_sequence(includes, "$.include")?;
        for (index, include) in includes.iter().enumerate() {
            let expression = expect_string(include, &format!("$.include[{index}]"))?;
            collect_included(&expression, base_dir, &mut definitions)?;
        }
    }
    let plugins = expect_sequence(root.required("plugins", "$")?, "$.plugins")?;
    for (index, plugin) in plugins.iter().enumerate() {
        definitions.push(decode_plugin(
            plugin,
            &format!("$.plugins[{index}]"),
            base_dir,
        )?);
    }

    compile_definitions(log, api, definitions)
}

/// Compiles the scoped management listener. Only `http` is supported; every
/// other Go `api` field is rejected rather than silently ignored.
fn compile_api(value: &RawValue) -> Result<ApiConfig, ConfigError> {
    let path = "$.api";
    let map = expect_map(value, path, "api must be a mapping")?;
    map.reject_unknown(&["http"], path)?;
    let http = expect_string(map.required("http", path)?, "$.api.http")?;
    let http = parse_socket_addr(&http, "$.api.http")?;
    Ok(ApiConfig { http })
}

/// Loads one plugins-only included file and appends its definitions in file
/// order. Nested includes are rejected rather than silently flattened.
fn collect_included(
    expression: &str,
    base_dir: &Path,
    definitions: &mut Vec<RawPlugin>,
) -> Result<(), ConfigError> {
    let path = resolve_relative(expression, base_dir);
    let display = path.display().to_string();
    let yaml = load_yaml(&path).map_err(|error| {
        ConfigError::new(
            "$.include",
            format!("cannot read included file `{expression}`: {}", error.reason),
        )
    })?;
    let raw = parse_yaml(&yaml).map_err(|error| {
        ConfigError::new(
            format!("{display}:{}", error.path),
            format!("included file is not valid YAML: {}", error.reason),
        )
    })?;
    let root = expect_map(&raw, &display, "included file must be a mapping")?;
    root.reject_unknown(&["plugins"], &display)?;
    let plugins = expect_sequence(
        root.required("plugins", &display)?,
        &format!("{display}.plugins"),
    )?;
    let included_base_dir = path.parent().unwrap_or_else(|| Path::new(""));
    for (index, plugin) in plugins.iter().enumerate() {
        definitions.push(decode_plugin(
            plugin,
            &format!("{display}.plugins[{index}]"),
            included_base_dir,
        )?);
    }
    Ok(())
}

fn resolve_relative(value: &str, base_dir: &Path) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() || base_dir.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

/// The declared kind of one plugin definition. Domain sets and sequences are
/// compiled into the program; forwards and caches become host-fulfilled
/// external executables; listeners are collected separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PluginKind {
    Forward,
    Cache,
    Sequence,
    DomainSet,
    Listener,
    FastMark,
    FlowSetter,
    Fallback,
}

/// The collected definition catalog used to resolve named references. It is
/// built before any definition is compiled, so a reference order-independent.
struct PluginCatalog<'a> {
    kinds: &'a [(String, PluginKind, usize)],
    domain_sets: &'a [(String, DomainSetHandle)],
    fast_marks: &'a [(String, FastMarkConfig)],
    forwards: &'a RefCell<ForwardCompiler>,
    preferences: &'a RefCell<Vec<(String, PreferenceFamily)>>,
}

impl PluginCatalog<'_> {
    fn kind_of(&self, tag: &str) -> Option<PluginKind> {
        self.kinds
            .iter()
            .find(|(name, _, _)| name.as_str() == tag)
            .map(|(_, kind, _)| *kind)
    }

    fn domain_set(&self, tag: &str) -> Option<DomainSetHandle> {
        self.domain_sets
            .iter()
            .find(|(name, _)| name.as_str() == tag)
            .map(|(_, handle)| handle.clone())
    }

    fn fast_mark(&self, tag: &str) -> Option<FastMarkConfig> {
        self.fast_marks
            .iter()
            .find(|(name, _)| name == tag)
            .map(|(_, config)| *config)
    }
}

fn compile_definitions(
    log: LogLevel,
    api: Option<ApiConfig>,
    definitions: Vec<RawPlugin>,
) -> Result<CompiledConfig, ConfigError> {
    let mut kinds: Vec<(String, PluginKind, usize)> = Vec::with_capacity(definitions.len());
    for (index, plugin) in definitions.iter().enumerate() {
        let kind = match plugin.kind.as_str() {
            "forward" => PluginKind::Forward,
            "cache" => PluginKind::Cache,
            "sequence" => PluginKind::Sequence,
            "domain_set" => PluginKind::DomainSet,
            "udp_server" | "tcp_server" => PluginKind::Listener,
            "fast_mark" => PluginKind::FastMark,
            "flow_setter" => PluginKind::FlowSetter,
            "fallback" => PluginKind::Fallback,
            other => {
                return Err(ConfigError::new(
                    format!("{}.type", plugin.source_path),
                    format!("unsupported plugin type `{other}`"),
                ));
            }
        };
        if let Some((existing, _, _)) = kinds
            .iter()
            .find(|(tag, _, _)| tag.as_str() == plugin.tag.as_str())
        {
            return Err(ConfigError::new(
                format!("{}.tag", plugin.source_path),
                format!("duplicate plugin tag `{existing}`"),
            ));
        }
        kinds.push((plugin.tag.clone(), kind, index));
    }
    // Domain sets and forwards are fully built before any sequence compiles,
    // so definition order never decides whether a reference resolves.
    //
    // Management eligibility is decided once every definition is known. A
    // single-`.txt` candidate may only be managed when no other `domain_set`
    // tag -- managed or query-only -- resolves to the same file; otherwise a
    // POST would rewrite another tag's persistent source behind its live
    // matcher and change that tag's behavior after a restart. Conflicting tags
    // keep loading and matching normally and simply stay unmanaged.
    let mut compiled_domain_sets: Vec<CompiledDomainSet> = Vec::new();
    for (index, plugin) in definitions.iter().enumerate() {
        if kinds[index].1 == PluginKind::DomainSet {
            compiled_domain_sets.push(compile_domain_set(plugin)?);
        }
    }
    let mut file_owners: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for set in &compiled_domain_sets {
        for reference in &set.references {
            file_owners
                .entry(reference.clone())
                .or_default()
                .insert(set.tag.clone());
        }
    }
    let mut domain_sets: Vec<(String, DomainSetHandle)> = Vec::new();
    let mut domain_set_configs: Vec<DomainSetConfig> = Vec::new();
    for set in compiled_domain_sets {
        let conflicting_tags: Option<Vec<String>> = set.candidate.as_ref().and_then(|candidate| {
            let owners = file_owners.get(&candidate.identity)?;
            let others: Vec<String> = owners
                .iter()
                .filter(|tag| tag.as_str() != set.tag)
                .cloned()
                .collect();
            (!others.is_empty()).then_some(others)
        });
        let (managed, ineligible_reason, handle) = match (set.candidate, conflicting_tags) {
            (Some(candidate), None) => {
                let provider = Rc::new(ManagedDomainSet::new(
                    set.tag.clone(),
                    candidate.file,
                    candidate.source_path,
                    candidate.rules,
                    set.matcher,
                ));
                (
                    Some(Rc::clone(&provider)),
                    None,
                    DomainSetHandle::Managed(provider),
                )
            }
            (Some(candidate), Some(others)) => (
                None,
                Some(format!(
                    "writable rule file `{}` is also read by other domain_set tag(s) {}; managing it would rewrite their source, so it may not be managed",
                    candidate.file.display(),
                    others.join(", ")
                )),
                DomainSetHandle::Fixed(Rc::new(set.matcher)),
            ),
            (None, _) => (None, None, DomainSetHandle::Fixed(Rc::new(set.matcher))),
        };
        domain_sets.push((set.tag.clone(), handle.clone()));
        domain_set_configs.push(DomainSetConfig {
            tag: set.tag,
            managed,
            ineligible_reason,
            handle,
        });
    }
    let mut forwards = Vec::new();
    let mut fast_marks = Vec::new();
    let mut flow_setters = Vec::new();
    let mut fallback_definitions = Vec::new();
    let mut upstream_identities: BTreeMap<String, String> = BTreeMap::new();
    let mut cache = None;
    let mut listener = None;
    for (index, plugin) in definitions.iter().enumerate() {
        match kinds[index].1 {
            PluginKind::DomainSet | PluginKind::Sequence => {}
            PluginKind::FastMark => {
                let config = compile_fast_mark(plugin)?;
                fast_marks.push((plugin.tag.clone(), config));
            }
            PluginKind::FlowSetter => {
                let config = compile_flow_setter(plugin)?;
                flow_setters.push((plugin.tag.clone(), config));
            }
            PluginKind::Fallback => {
                fallback_definitions.push(compile_fallback(plugin)?);
            }
            PluginKind::Forward => {
                let forward = compile_forward(plugin)?;
                for (entry_index, entry) in forward.entries.iter().enumerate() {
                    let location = if entry.tag.is_some() {
                        format!("{}.args.upstreams[{entry_index}].tag", plugin.source_path)
                    } else {
                        format!("{}.args.upstreams[{entry_index}]", plugin.source_path)
                    };
                    if let Some(existing) =
                        upstream_identities.insert(entry.identity.clone(), location.clone())
                    {
                        return Err(ConfigError::new(
                            location,
                            format!(
                                "duplicate effective upstream identity `{}` (already used by `{existing}`)",
                                entry.identity
                            ),
                        ));
                    }
                }
                forwards.push(forward);
            }
            PluginKind::Cache => {
                if cache.is_some() {
                    return Err(ConfigError::new(
                        format!("{}.tag", plugin.source_path),
                        "exactly one cache plugin is supported",
                    ));
                }
                cache = Some((
                    plugin.tag.clone(),
                    compile_cache(plugin)?,
                    plugin.source_path.clone(),
                ));
            }
            PluginKind::Listener => {
                if listener.is_some() {
                    return Err(ConfigError::new(
                        format!("{}.tag", plugin.source_path),
                        "exactly one listener plugin is supported",
                    ));
                }
                listener = Some((compile_listener(plugin)?, plugin.source_path.clone()));
            }
        }
    }
    let forward_compiler = RefCell::new(ForwardCompiler {
        definitions: forwards,
        invocations: Vec::new(),
    });
    let preference_invocations = RefCell::new(Vec::new());
    let catalog = PluginCatalog {
        kinds: &kinds,
        domain_sets: &domain_sets,
        fast_marks: &fast_marks,
        forwards: &forward_compiler,
        preferences: &preference_invocations,
    };
    let mut sequences = Vec::new();
    let mut fixtures = Vec::new();
    for (tag, config) in &fast_marks {
        fixtures.push(mosdns_sequence_core::FixtureSpec::new(
            plugin_fixture_name(tag),
            config.executor(),
        ));
    }
    for (tag, config) in &flow_setters {
        fixtures.push(mosdns_sequence_core::FixtureSpec::new(
            plugin_fixture_name(tag),
            config.executor(),
        ));
    }
    let resolved_fallback_definitions = fallback_definitions
        .into_iter()
        .map(|definition| {
            let primary_path = format!("{}.args.primary", definition.source_path);
            let secondary_path = format!("{}.args.secondary", definition.source_path);
            Ok(ResolvedFallbackDefinition {
                tag: definition.tag,
                source_path: definition.source_path,
                primary: resolve_policy_target_name(&definition.primary, &primary_path, &catalog)?,
                secondary: resolve_policy_target_name(
                    &definition.secondary,
                    &secondary_path,
                    &catalog,
                )?,
                threshold: definition.threshold,
                always_standby: definition.always_standby,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    for (index, plugin) in definitions.iter().enumerate() {
        if kinds[index].1 == PluginKind::Sequence {
            sequences.push(compile_sequence(plugin, &catalog, &mut fixtures)?);
        }
    }

    if sequences.is_empty() {
        return Err(ConfigError::new("$.plugins", "missing sequence plugin"));
    }
    let (listener, listener_source_path) =
        listener.ok_or_else(|| ConfigError::new("$.plugins", "missing listener plugin"))?;
    if !kinds
        .iter()
        .any(|(tag, kind, _)| *kind == PluginKind::Sequence && tag.as_str() == listener.entry)
    {
        return Err(ConfigError::new(
            format!("{listener_source_path}.args.entry"),
            format!("unknown sequence reference `{}`", listener.entry),
        ));
    }
    if forward_compiler.borrow().definitions.is_empty() {
        return Err(ConfigError::new(
            "$.plugins",
            "at least one forward plugin is required",
        ));
    }

    // Build the program. Every sequence becomes a named sequence, every
    // forward and cache becomes a host-fulfilled external, and a direct
    // `$sequence` reference becomes a named child call rather than a jump.
    let forward_compiler = forward_compiler.into_inner();
    for (name, _, _) in &forward_compiler.invocations {
        if kinds.iter().any(|(tag, _, _)| tag == name) {
            return Err(ConfigError::new(
                "$.plugins",
                "internal invocation name collision",
            ));
        }
    }
    let preference_invocations = preference_invocations.into_inner();
    let externals: Vec<ExternalSpec> = forward_compiler
        .invocations
        .iter()
        .map(|(name, _, _)| ExternalSpec::new(name.clone()))
        .chain(
            cache
                .as_ref()
                .map(|(tag, _, _)| ExternalSpec::new(tag.clone())),
        )
        .chain(
            resolved_fallback_definitions
                .iter()
                .map(|definition| ExternalSpec::new(definition.tag.clone())),
        )
        .chain(
            preference_invocations
                .iter()
                .map(|(name, _)| ExternalSpec::new(name.clone())),
        )
        .collect();
    let program = ProgramSpec::new(sequences, fixtures)
        .with_externals(externals)
        .validate()
        .map_err(|error| {
            ConfigError::new(
                "$.plugins[sequence].args",
                format!("sequence compile failed: {error:?}"),
            )
        })?;

    let mut compiled_forwards = Vec::new();
    let mut forward_invocations = Vec::new();
    for (name, definition, entries) in forward_compiler.invocations {
        let executable = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == name).then_some(*id))
            .ok_or_else(|| ConfigError::new("$.plugins", "compiled invocation is missing"))?;
        let forward = &forward_compiler.definitions[definition];
        // Legacy introspection is intentionally restricted to a single numeric entry.
        if entries.len() == 1
            && !compiled_forwards
                .iter()
                .any(|item: &ForwardConfig| item.tag == forward.tag)
        {
            let entry = &forward.entries[entries[0]];
            if let Some(endpoint) = entry.endpoint {
                compiled_forwards.push(ForwardConfig {
                    tag: forward.tag.clone(),
                    upstream_tag: entry.tag.clone(),
                    endpoint,
                    executable,
                });
            }
        }
        forward_invocations.push(ForwardInvocationConfig {
            executable,
            definition,
            entries,
        });
    }
    let compiled_cache = cache
        .map(|(tag, capacity, path)| {
            let executable = program
                .externals
                .iter()
                .find_map(|(id, external)| (external.name == tag).then_some(*id))
                .ok_or_else(|| ConfigError::new(path, "compiled cache external is missing"))?;
            Ok(CachePluginConfig {
                tag,
                executable,
                capacity,
            })
        })
        .transpose()?;

    let fallbacks = resolved_fallback_definitions
        .into_iter()
        .map(|definition| {
            let executable = external_id(&program, &definition.tag, &definition.source_path)?;
            Ok(FallbackConfig {
                tag: definition.tag,
                executable,
                primary: resolve_policy_target(
                    &program,
                    definition.primary,
                    &definition.source_path,
                )?,
                secondary: resolve_policy_target(
                    &program,
                    definition.secondary,
                    &definition.source_path,
                )?,
                threshold: definition.threshold,
                always_standby: definition.always_standby,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    let preferences = preference_invocations
        .into_iter()
        .map(|(name, family)| {
            Ok(PreferenceConfig {
                executable: external_id(&program, &name, "$.plugins.sequence.args")?,
                family,
                evidence: Rc::new(RefCell::new(BTreeMap::new())),
                clock: PreferenceClock::system(),
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;

    let entry_sequence = program.sequence_id(&listener.entry).ok_or_else(|| {
        ConfigError::new(
            format!("{listener_source_path}.args.entry"),
            "entry is missing",
        )
    })?;
    let primary = primary_forward(&program, entry_sequence, &compiled_forwards);

    Ok(CompiledConfig {
        log_level: log,
        forward: primary.clone(),
        forwards: compiled_forwards,
        forward_definitions: forward_compiler.definitions,
        forward_invocations,
        cache: compiled_cache,
        fallbacks,
        preferences,
        sequence: SequenceConfig {
            tag: listener.entry.clone(),
            sequence: entry_sequence,
            forward_executable: primary.as_ref().map(|forward| forward.executable),
        },
        listener,
        domain_sets: domain_set_configs,
        api,
        program,
    })
}

/// Finds one reachable forward for legacy convenience accessors. This scan is
/// not used to validate configuration or drive runtime dispatch.
fn external_id(
    program: &ValidatedProgram,
    name: &str,
    path: &str,
) -> Result<ExecutableId, ConfigError> {
    program
        .externals
        .iter()
        .find_map(|(id, external)| (external.name == name).then_some(*id))
        .ok_or_else(|| ConfigError::new(path, format!("compiled external `{name}` is missing")))
}

fn resolve_policy_target(
    program: &ValidatedProgram,
    target: PolicyTargetName,
    path: &str,
) -> Result<NativeTarget, ConfigError> {
    match target {
        PolicyTargetName::Sequence(name) => program
            .sequence_id(&name)
            .map(NativeTarget::Sequence)
            .ok_or_else(|| {
                ConfigError::new(path, format!("compiled sequence `${name}` is missing"))
            }),
        PolicyTargetName::Fixture(name) => program
            .fixtures
            .iter()
            .find_map(|(id, fixture)| (fixture.name == name).then_some(*id))
            .map(NativeTarget::Fixture)
            .ok_or_else(|| ConfigError::new(path, format!("compiled fixture `{name}` is missing"))),
        PolicyTargetName::External(name) => {
            external_id(program, &name, path).map(NativeTarget::External)
        }
    }
}

fn primary_forward(
    program: &ValidatedProgram,
    entry: SequenceId,
    forwards: &[ForwardConfig],
) -> Option<ForwardConfig> {
    let mut visited = BTreeSet::new();
    find_forward(program, entry, forwards, &mut visited)
}

fn find_forward(
    program: &ValidatedProgram,
    sequence: SequenceId,
    forwards: &[ForwardConfig],
    visited: &mut BTreeSet<SequenceId>,
) -> Option<ForwardConfig> {
    if !visited.insert(sequence) {
        return None;
    }
    let sequence = program.sequence(sequence)?;
    for rule in &sequence.rules {
        let Some(executable) = &rule.executable else {
            continue;
        };
        match executable {
            ValidatedExecutable::External { target } => {
                if let Some(forward) = forwards
                    .iter()
                    .find(|forward| forward.executable == *target)
                {
                    return Some(forward.clone());
                }
            }
            ValidatedExecutable::Call { target }
            | ValidatedExecutable::Goto { target }
            | ValidatedExecutable::Jump { target } => {
                if let Some(forward) = find_forward(program, *target, forwards, visited) {
                    return Some(forward);
                }
            }
            ValidatedExecutable::Try {
                target: ExecutableTarget::Sequence(target),
            } => {
                if let Some(forward) = find_forward(program, *target, forwards, visited) {
                    return Some(forward);
                }
            }
            ValidatedExecutable::Inline { target } => {
                // A multi-exec list runs its items in order; the first item
                // that reaches a forward owns the primary identity.
                if let Some(forward) = find_forward(program, *target, forwards, visited) {
                    return Some(forward);
                }
            }
            _ => {}
        }
    }
    None
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

fn decode_plugin(value: &RawValue, path: &str, base_dir: &Path) -> Result<RawPlugin, ConfigError> {
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
    Ok(RawPlugin {
        tag,
        kind,
        args,
        source_path: path.to_owned(),
        base_dir: base_dir.to_path_buf(),
    })
}

fn compile_forward(plugin: &RawPlugin) -> Result<ForwardDefinitionConfig, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "forward args must be a mapping")?;
    args.reject_unknown(
        &["upstreams", "concurrent", "bootstrap", "bootstrap_version"],
        &path,
    )?;
    let concurrent = match args.get("concurrent") {
        None => 1,
        Some(RawValue::Number(RawNumber::Signed(value))) => (*value).clamp(1, 3) as usize,
        Some(RawValue::Number(RawNumber::Unsigned(value))) => (*value).clamp(1, 3) as usize,
        _ => {
            return Err(ConfigError::new(
                format!("{path}.concurrent"),
                "expected an integer",
            ));
        }
    };
    let upstreams_path = format!("{path}.upstreams");
    let upstreams = expect_sequence(args.required("upstreams", &path)?, &upstreams_path)?;
    if upstreams.is_empty() {
        return Err(ConfigError::new(
            upstreams_path,
            "upstreams must not be empty",
        ));
    }
    let global_bootstrap = args
        .get("bootstrap")
        .map(|value| parse_bootstrap(value, &format!("{path}.bootstrap")))
        .transpose()?;
    let global_bootstrap_version = args
        .get("bootstrap_version")
        .map(|value| parse_bootstrap_version(value, &format!("{path}.bootstrap_version")))
        .transpose()?;
    let mut entries = Vec::new();
    for (index, value) in upstreams.iter().enumerate() {
        let item_path = format!("{upstreams_path}[{index}]");
        let item = expect_map(value, &item_path, "upstream must be a mapping")?;
        item.reject_unknown(
            &[
                "tag",
                "addr",
                "dial_addr",
                "bootstrap",
                "bootstrap_version",
                "upstream_query_timeout",
                "insecure_skip_verify",
                "idle_timeout",
                "enable_pipeline",
                "enable_http3",
                "max_conns",
                "socks5",
                "so_mark",
                "bind_to_device",
            ],
            &item_path,
        )?;
        let tag = item
            .get("tag")
            .map(|value| expect_string(value, &format!("{item_path}.tag")))
            .transpose()?;
        if tag
            .as_ref()
            .is_some_and(|tag| tag.is_empty() || tag.contains(char::is_whitespace))
        {
            return Err(ConfigError::new(
                format!("{item_path}.tag"),
                "entry tag must be nonempty without whitespace",
            ));
        }
        let address = expect_string(
            item.required("addr", &item_path)?,
            &format!("{item_path}.addr"),
        )?;
        let target = parse_forward_target(
            &address,
            item,
            global_bootstrap,
            global_bootstrap_version,
            &item_path,
        )?;
        let endpoint = target.dial_addr.map(|dial| {
            Endpoint::new(dial, target.scheme.endpoint_transport()).expect("validated dial")
        });
        let identity = tag.clone().unwrap_or_else(|| {
            if upstreams.len() == 1 {
                plugin.tag.clone()
            } else {
                format!("@native-forward:{}:{index}", hex_identity(&plugin.tag))
            }
        });
        entries.push(ForwardEntryConfig {
            tag,
            identity,
            endpoint,
            target,
        });
    }
    Ok(ForwardDefinitionConfig {
        tag: plugin.tag.clone(),
        entries,
        concurrent,
    })
}

fn hex_identity(value: &str) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn compile_fast_mark(plugin: &RawPlugin) -> Result<FastMarkConfig, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_string(&plugin.args, &path)?;
    FastMarkConfig::parse(&args).map_err(|reason| ConfigError::new(path, reason))
}

fn compile_flow_setter(plugin: &RawPlugin) -> Result<FlowSetterConfig, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "flow_setter args must be a mapping")?;
    args.reject_unknown(
        &["matched_group", "final_sequence", "final_upstream"],
        &path,
    )?;
    let mut config = FlowSetterConfig::default();
    for (key, slot) in [
        ("matched_group", &mut config.matched_group),
        ("final_sequence", &mut config.final_sequence),
        ("final_upstream", &mut config.final_upstream),
    ] {
        if let Some(value) = args.get(key) {
            let value = expect_string(value, &format!("{path}.{key}"))?;
            if value.is_empty() {
                return Err(ConfigError::new(
                    format!("{path}.{key}"),
                    "flow_setter values must not be empty",
                ));
            }
            *slot = Some(value);
        }
    }
    config
        .ensure_nonempty()
        .map_err(|reason| ConfigError::new(path, reason))?;
    Ok(config)
}

fn compile_fallback(plugin: &RawPlugin) -> Result<FallbackDefinition, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "fallback args must be a mapping")?;
    args.reject_unknown(
        &["primary", "secondary", "threshold", "always_standby"],
        &path,
    )?;
    let target = |key: &str| -> Result<String, ConfigError> {
        let value = expect_string(args.required(key, &path)?, &format!("{path}.{key}"))?;
        let tag = value.strip_prefix('$').ok_or_else(|| {
            ConfigError::new(
                format!("{path}.{key}"),
                "fallback targets must be exactly a `$plugin` reference",
            )
        })?;
        if tag.is_empty() || tag.contains(char::is_whitespace) {
            return Err(ConfigError::new(
                format!("{path}.{key}"),
                "fallback targets must be exactly one `$plugin` reference",
            ));
        }
        Ok(tag.to_owned())
    };
    let threshold = match args.get("threshold") {
        None => Duration::from_millis(500),
        Some(RawValue::Number(RawNumber::Signed(value))) if *value < 0 => {
            Duration::from_millis(500)
        }
        Some(RawValue::Number(RawNumber::Signed(value))) => {
            Duration::from_millis(u64::try_from(*value).map_err(|_| {
                ConfigError::new(
                    format!("{path}.threshold"),
                    "threshold is outside the supported range",
                )
            })?)
        }
        Some(RawValue::Number(RawNumber::Unsigned(value))) => Duration::from_millis(*value),
        Some(_) => {
            return Err(ConfigError::new(
                format!("{path}.threshold"),
                "threshold must be an integer number of milliseconds",
            ));
        }
    };
    if threshold > MAX_FALLBACK_THRESHOLD || Instant::now().checked_add(threshold).is_none() {
        return Err(ConfigError::new(
            format!("{path}.threshold"),
            "threshold is too large for monotonic deadline arithmetic",
        ));
    }
    let always_standby = args
        .get("always_standby")
        .map(|value| expect_bool(value, &format!("{path}.always_standby")))
        .transpose()?
        .unwrap_or(false);
    Ok(FallbackDefinition {
        tag: plugin.tag.clone(),
        source_path: plugin.source_path.clone(),
        primary: target("primary")?,
        secondary: target("secondary")?,
        threshold,
        always_standby,
    })
}

fn resolve_policy_target_name(
    tag: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
) -> Result<PolicyTargetName, ConfigError> {
    match catalog.kind_of(tag) {
        Some(PluginKind::Sequence) => Ok(PolicyTargetName::Sequence(tag.to_owned())),
        Some(PluginKind::Forward) => {
            let executable = catalog.forwards.borrow_mut().invocation(tag, "", path)?;
            let ExecutableSpec::External { target } = executable else {
                return Err(ConfigError::new(
                    path,
                    "forward target did not compile as external",
                ));
            };
            Ok(PolicyTargetName::External(target.name))
        }
        Some(PluginKind::Cache | PluginKind::Fallback) => {
            Ok(PolicyTargetName::External(tag.to_owned()))
        }
        Some(PluginKind::FastMark | PluginKind::FlowSetter) => {
            Ok(PolicyTargetName::Fixture(plugin_fixture_name(tag)))
        }
        Some(PluginKind::DomainSet | PluginKind::Listener) => Err(ConfigError::new(
            path,
            format!("`${tag}` is not executable"),
        )),
        None => Err(ConfigError::new(
            path,
            format!("unknown executable reference `${tag}`"),
        )),
    }
}

fn plugin_fixture_name(tag: &str) -> String {
    format!("__native_plugin_{tag}")
}

fn quick_fixture_name(path: &str) -> String {
    format!("__native_quick_{path}")
}

/// One compiled `domain_set` before management eligibility is decided.
struct CompiledDomainSet {
    tag: String,
    matcher: MixMatcher<()>,
    /// Present when the declared shape is a single-`.txt` management candidate.
    candidate: Option<ManagedCandidate>,
    /// Canonical identity of every resolved `files` entry this tag references,
    /// whatever its shape. A management candidate may not own a file that any
    /// other `domain_set` also reads, or a POST would rewrite that tag's
    /// persistent source behind its live matcher.
    references: Vec<PathBuf>,
}

/// A single-`.txt` profile that could be managed, pending the shared-file check.
struct ManagedCandidate {
    file: PathBuf,
    /// Canonical identity of the writable file, used for the conflict check.
    identity: PathBuf,
    rules: Vec<String>,
    source_path: String,
}

fn compile_domain_set(plugin: &RawPlugin) -> Result<CompiledDomainSet, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "domain_set args must be a mapping")?;
    args.reject_unknown(&["exps", "files"], &path)?;
    let expressions = string_list(args.get("exps"), &format!("{path}.exps"))?;
    let files = string_list(args.get("files"), &format!("{path}.files"))?;
    if expressions.is_empty() && files.is_empty() {
        return Err(ConfigError::new(
            &path,
            "a domain_set requires at least one expression or file",
        ));
    }
    let (matcher, accepted) =
        build_domain_set(&expressions, &files, &plugin.base_dir).map_err(|error| match error {
            DomainSetError::Expression { index, .. } => {
                ConfigError::new(format!("{path}.exps[{index}]"), error.to_string())
            }
            DomainSetError::File { index, .. } => {
                ConfigError::new(format!("{path}.files[{index}]"), error.to_string())
            }
        })?;

    // A managed profile needs exactly one unambiguous writable `.txt` source,
    // matching Go's POST precondition, so an edit can never silently drop part
    // of the configured rule set. Everything else stays query-only.
    let single_txt_file = expressions.is_empty()
        && files.len() == 1
        && Path::new(&files[0])
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
    // Every declared file reference is resolved the same way the loader
    // resolved it, so the conflict check sees query-only shapes too.
    let references: Vec<PathBuf> = files
        .iter()
        .map(|file| {
            let path = resolve_rule_path(file, &plugin.base_dir);
            std::fs::canonicalize(&path).unwrap_or(path)
        })
        .collect();
    let candidate = if single_txt_file {
        let file = resolve_rule_path(&files[0], &plugin.base_dir);
        Some(ManagedCandidate {
            file,
            identity: references
                .first()
                .cloned()
                .unwrap_or_else(|| files[0].clone().into()),
            rules: accepted,
            source_path: plugin.source_path.clone(),
        })
    } else {
        None
    };
    Ok(CompiledDomainSet {
        tag: plugin.tag.clone(),
        matcher,
        candidate,
        references,
    })
}

fn string_list(value: Option<&RawValue>, path: &str) -> Result<Vec<String>, ConfigError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = expect_sequence(value, path)?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| expect_string(value, &format!("{path}[{index}]")))
        .collect()
}

fn compile_cache(plugin: &RawPlugin) -> Result<u64, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "cache args must be a mapping")?;
    args.reject_unknown(&["size", "lazy_cache_ttl"], &path)?;
    let size = expect_positive_integer(args.required("size", &path)?, &format!("{path}.size"))?;
    let lazy_cache_ttl = expect_nonnegative_integer(
        args.required("lazy_cache_ttl", &path)?,
        &format!("{path}.lazy_cache_ttl"),
    )?;
    if lazy_cache_ttl != 0 {
        return Err(ConfigError::new(
            format!("{path}.lazy_cache_ttl"),
            "lazy_cache_ttl must be exactly 0",
        ));
    }
    Ok(size)
}

fn compile_sequence(
    plugin: &RawPlugin,
    catalog: &PluginCatalog<'_>,
    fixtures: &mut Vec<mosdns_sequence_core::FixtureSpec>,
) -> Result<SequenceSpec, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_sequence(&plugin.args, &path)?;
    let mut rules = Vec::with_capacity(args.len());
    for (rule_index, value) in args.iter().enumerate() {
        let item_path = format!("{path}[{rule_index}]");
        let item = expect_map(value, &item_path, "sequence rule must be a mapping")?;
        item.reject_unknown(&["matches", "exec"], &item_path)?;
        let mut matchers = Vec::new();
        if let Some(matches) = item.get("matches") {
            let matches_path = format!("{item_path}.matches");
            for (match_index, expression) in match_expressions(matches, &matches_path)?
                .into_iter()
                .enumerate()
            {
                matchers.push(compile_matcher(
                    &expression,
                    &format!("{matches_path}[{match_index}]"),
                    catalog,
                )?);
            }
        }
        let executable = item
            .get("exec")
            .map(|value| {
                compile_exec(
                    value,
                    &format!("{item_path}.exec"),
                    catalog,
                    fixtures,
                    &plugin.tag,
                    rule_index,
                )
            })
            .transpose()?;
        if matchers.is_empty() && executable.is_none() {
            return Err(ConfigError::new(
                &item_path,
                "a sequence rule requires at least one matcher or executable",
            ));
        }
        rules.push(
            RuleSpec::new(matchers, executable)
                .with_audit_source(format!("inline:{}#{}", plugin.tag, rule_index)),
        );
    }
    Ok(SequenceSpec::new(plugin.tag.clone(), rules))
}

/// A rule's `matches` accepts one expression or an ordered list.
fn match_expressions(value: &RawValue, path: &str) -> Result<Vec<String>, ConfigError> {
    match value {
        RawValue::String(expression) => Ok(vec![expression.clone()]),
        RawValue::Sequence(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| expect_string(value, &format!("{path}[{index}]")))
            .collect(),
        _ => Err(ConfigError::new(
            path,
            "matches must be a string or a sequence of strings",
        )),
    }
}

fn compile_matcher(
    expression: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
) -> Result<MatcherSpecInput, ConfigError> {
    let (expression, reverse) = match expression.strip_prefix('!') {
        Some(rest) => (rest.trim(), true),
        None => (expression.trim(), false),
    };
    if expression.is_empty() {
        return Err(ConfigError::new(
            path,
            "matcher expression must not be empty",
        ));
    }
    let (name, args) = match expression.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (expression, ""),
    };
    let matcher: Box<dyn mosdns_sequence_core::Matcher> = match name {
        name if name.starts_with('$') => {
            let tag = name.trim_start_matches('$');
            if tag.is_empty() || !args.is_empty() {
                return Err(ConfigError::new(
                    path,
                    "named matcher references require exactly `$tag` without arguments",
                ));
            }
            let config = catalog.fast_mark(tag).ok_or_else(|| {
                ConfigError::new(path, format!("unknown matcher reference `${tag}`"))
            })?;
            config.matcher()
        }
        "fast_mark" => FastMarkConfig::parse(args)
            .map_err(|reason| ConfigError::new(path, reason))?
            .matcher(),
        "qname" => Box::new(compile_qname(args, path, catalog)?),
        "qtype" => {
            let mut types = Vec::new();
            for field in args.split_whitespace() {
                let value = field.parse::<u16>().map_err(|_| {
                    ConfigError::new(
                        path,
                        format!("qtype requires a numeric type, got `{field}`"),
                    )
                })?;
                types.push(value);
            }
            if types.is_empty() {
                return Err(ConfigError::new(path, "qtype requires at least one type"));
            }
            Box::new(QtypeMatcher::new(types))
        }
        "has_resp" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, "has_resp takes no arguments"));
            }
            Box::new(HasResponseMatcher)
        }
        "resp_ip" => {
            let address = args
                .parse::<Ipv4Addr>()
                .map_err(|_| ConfigError::new(path, "resp_ip requires one IPv4 literal"))?;
            Box::new(ResponseIpMatcher::ipv4(address))
        }
        "_true" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, "_true takes no arguments"));
            }
            Box::new(TrueMatcher)
        }
        "_false" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, "_false takes no arguments"));
            }
            Box::new(FalseMatcher)
        }
        other => {
            return Err(ConfigError::new(
                path,
                format!("unsupported matcher `{other}`"),
            ));
        }
    };
    // `!` composes with every matcher, `_true`/`_false` included.
    Ok(MatcherSpecInput::new(
        matcher,
        reverse,
        mosdns_sequence_core::DispatchMetadata::None,
    ))
}

/// A matcher that always misses, so `!_false` matches everything.
pub(crate) struct FalseMatcher;

impl mosdns_sequence_core::Matcher for FalseMatcher {
    fn evaluate(
        &self,
        _state: &ExecutionState,
    ) -> Result<mosdns_sequence_core::MatchOutcome, mosdns_sequence_core::MatcherError> {
        Ok(mosdns_sequence_core::MatchOutcome::new(false, None))
    }
}

/// Builds the qname matcher for `qname $provider`, inline expressions, and
/// `&file` references. Referenced domain sets keep their declaration order and
/// the anonymous inline set is appended last, matching the provider contract.
fn compile_qname(
    args: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
) -> Result<QnameMatcher, ConfigError> {
    let mut groups: Vec<(String, DomainSetHandle)> = Vec::new();
    let mut inline = MixMatcher::new();
    inline.set_default("domain");
    let mut has_inline = false;
    let mut first_inline_index = None;
    for field in args.split_whitespace() {
        if let Some(tag) = field.strip_prefix('$') {
            let set = catalog.domain_set(tag).ok_or_else(|| {
                ConfigError::new(path, format!("unknown domain_set reference `${tag}`"))
            })?;
            groups.push((format!("domain_set:{tag}"), set));
        } else if field.starts_with('&') {
            return Err(ConfigError::new(
                path,
                "qname file references use a domain_set plugin `files` entry",
            ));
        } else {
            if first_inline_index.is_none() {
                first_inline_index = Some(0);
            }
            inline.add(field, ()).map_err(|error| {
                ConfigError::new(path, format!("invalid qname rule `{field}`: {error}"))
            })?;
            has_inline = true;
        }
    }
    if groups.is_empty() && !has_inline {
        return Err(ConfigError::new(
            path,
            "qname requires a domain_set reference or an inline rule",
        ));
    }
    if has_inline {
        groups.push((
            format!("inline:{path}#{}", first_inline_index.unwrap_or(0)),
            DomainSetHandle::Fixed(Rc::new(inline)),
        ));
    }
    Ok(QnameMatcher::new(groups))
}

fn compile_exec(
    value: &RawValue,
    path: &str,
    catalog: &PluginCatalog<'_>,
    fixtures: &mut Vec<mosdns_sequence_core::FixtureSpec>,
    sequence_tag: &str,
    rule_index: usize,
) -> Result<Vec<ExecutableSpec>, ConfigError> {
    match value {
        RawValue::String(expression) => Ok(vec![compile_exec_item(
            expression,
            path,
            catalog,
            fixtures,
            sequence_tag,
            rule_index,
            0,
        )?]),
        RawValue::Sequence(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let item_path = format!("{path}[{index}]");
                let expression = expect_string(value, &item_path)?;
                compile_exec_item(
                    &expression,
                    &item_path,
                    catalog,
                    fixtures,
                    sequence_tag,
                    rule_index,
                    index,
                )
            })
            .collect(),
        _ => Err(ConfigError::new(
            path,
            "exec must be a string or a sequence of strings",
        )),
    }
}

fn compile_exec_item(
    expression: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
    fixtures: &mut Vec<mosdns_sequence_core::FixtureSpec>,
    sequence_tag: &str,
    rule_index: usize,
    exec_index: usize,
) -> Result<ExecutableSpec, ConfigError> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err(ConfigError::new(path, "exec expression must not be empty"));
    }
    let (name, args) = match expression.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (expression, ""),
    };
    match name {
        "prefer_ipv4" | "prefer_ipv6" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, format!("{name} takes no arguments")));
            }
            let family = if name == "prefer_ipv4" {
                PreferenceFamily::Ipv4
            } else {
                PreferenceFamily::Ipv6
            };
            let internal_name = format!(
                "@native-{}:{}:{}:{}",
                name,
                hex_identity(sequence_tag),
                rule_index,
                exec_index
            );
            catalog
                .preferences
                .borrow_mut()
                .push((internal_name.clone(), family));
            Ok(ExecutableSpec::External {
                target: ExternalRef::new(internal_name),
            })
        }
        "forward" => {
            catalog
                .forwards
                .borrow_mut()
                .quick(sequence_tag, rule_index, exec_index, args, path)
        }
        "fast_mark" => {
            let config =
                FastMarkConfig::parse(args).map_err(|reason| ConfigError::new(path, reason))?;
            let fixture_name = quick_fixture_name(path);
            fixtures.push(mosdns_sequence_core::FixtureSpec::new(
                fixture_name.clone(),
                config.executor(),
            ));
            Ok(ExecutableSpec::Fixture {
                target: mosdns_sequence_core::FixtureRef::new(fixture_name),
            })
        }
        "flow_setter" => {
            let config = FlowSetterConfig::from_quick_args(args)
                .map_err(|reason| ConfigError::new(path, reason))?;
            let fixture_name = quick_fixture_name(path);
            fixtures.push(mosdns_sequence_core::FixtureSpec::new(
                fixture_name.clone(),
                config.executor(),
            ));
            Ok(ExecutableSpec::Fixture {
                target: mosdns_sequence_core::FixtureRef::new(fixture_name),
            })
        }
        "accept" | "return" | "exit" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, format!("{name} takes no arguments")));
            }
            Ok(match name {
                "accept" => ExecutableSpec::Accept,
                "return" => ExecutableSpec::Return,
                _ => ExecutableSpec::Exit,
            })
        }
        "reject" => {
            let rcode = if args.is_empty() {
                DEFAULT_REJECT_RCODE
            } else {
                args.parse::<u16>().map_err(|_| {
                    ConfigError::new(
                        path,
                        format!("reject requires a numeric rcode, got `{args}`"),
                    )
                })?
            };
            if rcode > MAX_SUPPORTED_REJECT_RCODE {
                return Err(ConfigError::new(
                    path,
                    format!(
                        "reject rcode {rcode} is unsupported; the native host currently supports 0..={MAX_SUPPORTED_REJECT_RCODE}"
                    ),
                ));
            }
            Ok(ExecutableSpec::Reject { rcode })
        }
        "goto" | "jump" => {
            let target = named_sequence_target(args, path, catalog, name)?;
            Ok(if name == "goto" {
                ExecutableSpec::Goto { target }
            } else {
                ExecutableSpec::Jump { target }
            })
        }
        "try" => Ok(ExecutableSpec::Try {
            target: ExecutableTargetSpec::Sequence(named_sequence_target(
                args, path, catalog, name,
            )?),
        }),
        _ => {
            let Some(tag) = name.strip_prefix('$') else {
                return Err(ConfigError::new(
                    path,
                    format!("unsupported executable `{name}`"),
                ));
            };
            if catalog.kind_of(tag) == Some(PluginKind::Forward) {
                return catalog.forwards.borrow_mut().invocation(tag, args, path);
            }
            if !args.is_empty() {
                return Err(ConfigError::new(
                    path,
                    format!("`{name}` takes no arguments"),
                ));
            }
            match catalog.kind_of(tag) {
                Some(PluginKind::Sequence) => Ok(ExecutableSpec::Call {
                    target: SequenceRef::new(tag),
                }),
                Some(PluginKind::Forward) => {
                    catalog.forwards.borrow_mut().invocation(tag, args, path)
                }
                Some(PluginKind::Cache | PluginKind::Fallback) => Ok(ExecutableSpec::External {
                    target: ExternalRef::new(tag),
                }),
                Some(PluginKind::FastMark | PluginKind::FlowSetter) => {
                    Ok(ExecutableSpec::Fixture {
                        target: mosdns_sequence_core::FixtureRef::new(plugin_fixture_name(tag)),
                    })
                }
                Some(PluginKind::DomainSet | PluginKind::Listener) => Err(ConfigError::new(
                    path,
                    format!("`{name}` is not executable"),
                )),
                None => Err(ConfigError::new(
                    path,
                    format!("unknown executable reference `{name}`"),
                )),
            }
        }
    }
}

fn named_sequence_target(
    args: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
    name: &str,
) -> Result<SequenceRef, ConfigError> {
    let tag = args
        .strip_prefix('$')
        .ok_or_else(|| ConfigError::new(path, format!("{name} requires a `$sequence` target")))?;
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return Err(ConfigError::new(
            path,
            format!("{name} requires exactly one `$sequence` target"),
        ));
    }
    match catalog.kind_of(tag) {
        Some(PluginKind::Sequence) => Ok(SequenceRef::new(tag)),
        Some(_) => Err(ConfigError::new(
            path,
            format!("`{name} ${tag}` does not reference a sequence"),
        )),
        None => Err(ConfigError::new(
            path,
            format!("unknown sequence reference `${tag}`"),
        )),
    }
}

fn compile_listener(plugin: &RawPlugin) -> Result<ListenerConfig, ConfigError> {
    let is_tcp = plugin.kind == "tcp_server";
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "listener args must be a mapping")?;
    let allowed = if is_tcp {
        &["entry", "listen", "enable_audit", "idle_timeout"][..]
    } else {
        &["entry", "listen", "enable_audit"][..]
    };
    args.reject_unknown(allowed, &path)?;
    let entry = expect_string(args.required("entry", &path)?, &format!("{path}.entry"))?;
    if entry.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.entry"),
            "entry must not be empty",
        ));
    }
    let listen = expect_string(args.required("listen", &path)?, &format!("{path}.listen"))?;
    let listen = parse_socket_addr(&listen, &format!("{path}.listen"))?;
    let enable_audit = expect_bool(
        args.required("enable_audit", &path)?,
        &format!("{path}.enable_audit"),
    )?;
    let idle_timeout = if is_tcp {
        let timeout = expect_positive_integer(
            args.required("idle_timeout", &path)?,
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

fn parse_forward_target(
    value: &str,
    item: &RawMap,
    global_bootstrap: Option<SocketAddr>,
    global_bootstrap_version: Option<u8>,
    path: &str,
) -> Result<ForwardTargetConfig, ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.addr"),
            "addr must not be empty",
        ));
    }
    let (scheme, host, port, service) = parse_service_address(value, path)?;
    let dial_addr = item
        .get("dial_addr")
        .map(|value| parse_numeric_host_port(value, port, &format!("{path}.dial_addr")))
        .transpose()?;
    let dial_addr = dial_addr.or_else(|| {
        host.parse::<std::net::IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, port))
    });
    let bootstrap = item
        .get("bootstrap")
        .map(|value| parse_bootstrap(value, &format!("{path}.bootstrap")))
        .transpose()?
        .or(global_bootstrap);
    let bootstrap_version = item
        .get("bootstrap_version")
        .map(|value| parse_bootstrap_version(value, &format!("{path}.bootstrap_version")))
        .transpose()?
        .or(global_bootstrap_version);
    let query_timeout = parse_query_timeout(item.get("upstream_query_timeout"), path)?;
    let insecure_skip_verify = item
        .get("insecure_skip_verify")
        .map(|value| expect_bool(value, &format!("{path}.insecure_skip_verify")))
        .transpose()?
        .unwrap_or(false);
    validate_inactive_forward_options(item, path)?;

    if host.parse::<std::net::IpAddr>().is_err() && dial_addr.is_none() && bootstrap.is_none() {
        return Err(ConfigError::new(
            format!("{path}.addr"),
            "hostname upstream requires numeric dial_addr or bootstrap",
        ));
    }
    if matches!(scheme, ForwardScheme::Udp | ForwardScheme::Tcp) && insecure_skip_verify {
        return Err(ConfigError::new(
            format!("{path}.insecure_skip_verify"),
            "insecure_skip_verify is only valid for tls/https upstreams",
        ));
    }
    if matches!(scheme, ForwardScheme::Tls | ForwardScheme::Https) {
        ServerIdentity::new(&host).map_err(|error| {
            ConfigError::new(
                format!("{path}.addr"),
                format!("invalid secure identity: {error}"),
            )
        })?;
        if scheme == ForwardScheme::Https {
            let dial = dial_addr.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], port)));
            mosdns_upstream_core::DohEndpoint::new(&service, dial).map_err(|error| {
                ConfigError::new(
                    format!("{path}.addr"),
                    format!("invalid https endpoint: {error}"),
                )
            })?;
        }
    }

    Ok(ForwardTargetConfig {
        scheme,
        service,
        host,
        port,
        dial_addr,
        bootstrap,
        bootstrap_version,
        query_timeout,
        insecure_skip_verify,
    })
}

fn parse_service_address(
    value: &str,
    path: &str,
) -> Result<(ForwardScheme, String, u16, String), ConfigError> {
    if let Some((scheme_name, _)) = value.split_once("://") {
        let scheme = match scheme_name {
            "udp" => ForwardScheme::Udp,
            "tcp" => ForwardScheme::Tcp,
            "tls" => ForwardScheme::Tls,
            "https" => ForwardScheme::Https,
            other => {
                return Err(ConfigError::new(
                    path,
                    format!("unsupported upstream scheme `{other}`"),
                ));
            }
        };
        let url = Url::parse(value)
            .map_err(|error| ConfigError::new(path, format!("invalid upstream URL: {error}")))?;
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(ConfigError::new(
                path,
                "upstream URL must not contain userinfo or fragment",
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| ConfigError::new(path, "upstream URL host is missing"))?
            .trim_matches(['[', ']'])
            .to_owned();
        let port = url.port().unwrap_or_else(|| scheme.default_port());
        if port == 0 {
            return Err(ConfigError::new(path, "port must be nonzero"));
        }
        if !matches!(scheme, ForwardScheme::Https)
            && ((!url.path().is_empty() && url.path() != "/") || url.query().is_some())
        {
            return Err(ConfigError::new(
                path,
                "only https upstreams accept path/query components",
            ));
        }
        Ok((scheme, host, port, value.to_owned()))
    } else if let Ok(socket) = value.parse::<SocketAddr>() {
        Ok((
            ForwardScheme::Udp,
            socket.ip().to_string(),
            socket.port(),
            format!("udp://{value}"),
        ))
    } else if let Ok(ip) = value.parse::<std::net::IpAddr>() {
        Ok((
            ForwardScheme::Udp,
            ip.to_string(),
            53,
            format!("udp://{ip}:53"),
        ))
    } else if let Some((host, port)) = value.rsplit_once(':') {
        if !host.contains(':') {
            let port = port
                .parse::<u16>()
                .map_err(|_| ConfigError::new(path, "invalid upstream port"))?;
            if port == 0 {
                return Err(ConfigError::new(path, "port must be nonzero"));
            }
            return Ok((
                ForwardScheme::Udp,
                host.to_owned(),
                port,
                format!("udp://{value}"),
            ));
        }
        Err(ConfigError::new(path, "IPv6 upstreams must use brackets"))
    } else {
        Ok((
            ForwardScheme::Udp,
            value.to_owned(),
            53,
            format!("udp://{value}:53"),
        ))
    }
}

fn parse_numeric_host_port(
    value: &RawValue,
    inherited_port: u16,
    path: &str,
) -> Result<SocketAddr, ConfigError> {
    let text = expect_string(value, path)?;
    if text.is_empty() {
        return Err(ConfigError::new(path, "numeric address must not be empty"));
    }
    if let Ok(socket) = text.parse::<SocketAddr>() {
        if socket.port() == 0 {
            return Err(ConfigError::new(path, "port must be nonzero"));
        }
        return Ok(socket);
    }
    let ip = text
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .map_err(|_| ConfigError::new(path, "address must be numeric"))?;
    Ok(SocketAddr::new(ip, inherited_port))
}

fn parse_bootstrap(value: &RawValue, path: &str) -> Result<SocketAddr, ConfigError> {
    parse_numeric_host_port(value, 53, path)
}

fn parse_bootstrap_version(value: &RawValue, path: &str) -> Result<u8, ConfigError> {
    let version = match value {
        RawValue::Number(RawNumber::Unsigned(value)) => u8::try_from(*value).ok(),
        RawValue::Number(RawNumber::Signed(value)) if *value >= 0 => u8::try_from(*value).ok(),
        _ => None,
    };
    match version {
        Some(0 | 4 | 6) => Ok(version.unwrap_or_default()),
        Some(value) => Err(ConfigError::new(
            path,
            format!("unsupported bootstrap_version {value}; expected 0, 4, or 6"),
        )),
        None => Err(ConfigError::new(
            path,
            "bootstrap_version must be an integer 0, 4, or 6",
        )),
    }
}

fn parse_query_timeout(value: Option<&RawValue>, path: &str) -> Result<Duration, ConfigError> {
    let Some(value) = value else {
        return Ok(Duration::from_secs(5));
    };
    let millis = match value {
        RawValue::Number(RawNumber::Unsigned(value)) => *value,
        RawValue::Number(RawNumber::Signed(value)) if *value >= 0 => *value as u64,
        RawValue::Number(RawNumber::Signed(_)) => {
            return Err(ConfigError::new(
                format!("{path}.upstream_query_timeout"),
                "timeout must not be negative",
            ));
        }
        _ => {
            return Err(ConfigError::new(
                format!("{path}.upstream_query_timeout"),
                "timeout must be an integer number of milliseconds",
            ));
        }
    };
    if millis == 0 {
        return Ok(Duration::from_secs(5));
    }
    Ok(Duration::from_millis(millis))
}

fn validate_inactive_forward_options(item: &RawMap, path: &str) -> Result<(), ConfigError> {
    if let Some(value) = item.get("idle_timeout") {
        let timeout = expect_nonnegative_integer(value, &format!("{path}.idle_timeout"))?;
        if timeout != 0 {
            return Err(ConfigError::new(
                format!("{path}.idle_timeout"),
                "positive idle_timeout is unsupported",
            ));
        }
    }
    if let Some(value) = item.get("enable_pipeline") {
        if expect_bool(value, &format!("{path}.enable_pipeline"))? {
            return Err(ConfigError::new(
                format!("{path}.enable_pipeline"),
                "pipeline is unsupported",
            ));
        }
    }
    if let Some(value) = item.get("enable_http3") {
        if expect_bool(value, &format!("{path}.enable_http3"))? {
            return Err(ConfigError::new(
                format!("{path}.enable_http3"),
                "http3 is unsupported",
            ));
        }
    }
    for key in ["max_conns", "socks5", "so_mark", "bind_to_device"] {
        if item.get(key).is_some() {
            return Err(ConfigError::new(
                format!("{path}.{key}"),
                "option is unsupported",
            ));
        }
    }
    Ok(())
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

/// The default configured `reject` response, matching the product default.
pub(crate) const DEFAULT_REJECT_RCODE: u16 = 5;
/// The largest `reject` rcode this host renders on the wire today. The full
/// 12-bit range stays supported by the sequence core and is planned for a
/// later batch with the complete EDNS work.
pub(crate) const MAX_SUPPORTED_REJECT_RCODE: u16 = 15;

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
    source_path: String,
    base_dir: PathBuf,
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

    use super::{ListenerKind, LogLevel, PreferenceFamily, compile_yaml};

    const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");
    const TCP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-tcp.yaml");

    #[test]
    fn compiles_the_frozen_udp_and_tcp_shapes_without_rewriting_them() {
        let udp = compile_yaml(UDP).expect("frozen UDP config must compile");
        assert_eq!(udp.log_level, LogLevel::Error);
        assert_eq!(udp.listener.kind, ListenerKind::Udp);
        assert_eq!(udp.listener.listen.port(), 15353);
        let udp_forward = udp.forward.as_ref().expect("legacy forward view");
        assert_eq!(udp_forward.endpoint.transport(), Transport::Udp);
        assert_eq!(udp_forward.endpoint.address().port(), 15453);
        assert_eq!(udp.listener.idle_timeout, None);
        assert_eq!(udp.sequence.tag, "phase5a_entry");
        assert_eq!(
            udp.sequence.forward_executable,
            Some(udp_forward.executable)
        );

        let tcp = compile_yaml(TCP).expect("frozen TCP config must compile");
        assert_eq!(tcp.listener.kind, ListenerKind::Tcp);
        assert_eq!(tcp.listener.listen.port(), 15354);
        let tcp_forward = tcp.forward.as_ref().expect("legacy forward view");
        assert_eq!(tcp_forward.endpoint.transport(), Transport::Tcp);
        assert_eq!(tcp_forward.endpoint.address().port(), 15454);
        assert_eq!(tcp.listener.idle_timeout, Some(Duration::from_secs(2)));
    }

    #[test]
    fn compiles_fallback_defaults_and_preserves_explicit_zero() {
        let yaml = r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args: [ { exec: "$fb" } ]
  - tag: primary
    type: sequence
    args: [ { exec: "$forward" } ]
  - tag: secondary
    type: sequence
    args: [ { exec: "$forward" } ]
  - tag: fb
    type: fallback
    args: { primary: "$primary", secondary: "$secondary", always_standby: true }
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: false }
"#;
        let config = compile_yaml(yaml).expect("fallback config");
        let fallback = config.fallbacks.first().expect("fallback descriptor");
        assert_eq!(fallback.threshold, Duration::from_millis(500));
        assert!(fallback.always_standby);

        let explicit_zero = compile_yaml(
            &yaml.replace("always_standby: true", "threshold: 0, always_standby: true"),
        )
        .expect("explicit zero fallback");
        assert_eq!(
            explicit_zero.fallbacks[0].threshold,
            Duration::from_millis(0)
        );

        let negative = compile_yaml(&yaml.replace(
            "always_standby: true",
            "threshold: -1, always_standby: true",
        ))
        .expect("negative compatibility threshold");
        assert_eq!(negative.fallbacks[0].threshold, Duration::from_millis(500));

        let overflow =
            compile_yaml(&yaml.replace("always_standby: true", "threshold: 18446744073709551615"));
        assert!(
            overflow.is_err(),
            "absolute threshold overflow must fail load"
        );
    }

    #[test]
    fn compiles_both_preference_quick_forms_and_rejects_arguments() {
        let yaml = r#"
log: { level: error }
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: prefer_ipv4
      - exec: prefer_ipv6
      - exec: $forward
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: false }
"#;
        let config = compile_yaml(yaml).expect("preference config");
        assert_eq!(config.preferences.len(), 2);
        assert_eq!(config.preferences[0].family, PreferenceFamily::Ipv4);
        assert_eq!(config.preferences[1].family, PreferenceFamily::Ipv6);

        let error = match compile_yaml(&yaml.replace("prefer_ipv4", "prefer_ipv4 extra")) {
            Ok(_) => panic!("preference args must be rejected"),
            Err(error) => error,
        };
        assert!(error.reason.contains("takes no arguments"));
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
        assert_eq!(
            config.forward.as_ref().expect("primary forward").tag,
            "forward"
        );

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
                if dispatch.executable()
                    == config.forward.as_ref().expect("primary forward").executable
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

        let duplicate_listener = r#"
log: { level: error }
plugins:
  - { tag: f, type: forward, args: { upstreams: [ { addr: udp://127.0.0.1:53 } ] } }
  - { tag: s, type: sequence, args: [ { exec: "$f" } ] }
  - { tag: l1, type: udp_server, args: { entry: s, listen: "127.0.0.1:53", enable_audit: false } }
  - { tag: l2, type: udp_server, args: { entry: s, listen: "127.0.0.1:54", enable_audit: false } }
"#;
        assert!(compile_yaml(duplicate_listener).is_err());
    }

    /// A one-rule program whose rule matches `expression`, so the compiled
    /// matcher's decision can be observed through a real machine.
    fn matcher_expression(expression: &str) -> Result<bool, super::ConfigError> {
        let yaml = format!(
            r#"
log: {{ level: error }}
plugins:
  - tag: rules
    type: domain_set
    args: {{ exps: ["full:blocked.test"] }}
  - tag: entry
    type: sequence
    args:
      - matches: "{expression}"
        exec: reject 3
      - exec: $f
  - tag: f
    type: forward
    args: {{ upstreams: [ {{ addr: udp://127.0.0.1:53 }} ] }}
  - tag: l
    type: udp_server
    args: {{ entry: entry, listen: "127.0.0.1:53", enable_audit: false }}
"#
        );
        let config = super::compile_yaml(&yaml)?;
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
                qname_wire: vec![
                    7, b'b', b'l', b'o', b'c', b'k', b'e', b'd', 4, b't', b'e', b's', b't', 0,
                ],
                qtype: 1,
                qclass: 1,
            },
        );
        let mut machine = config
            .new_machine(state, ExecutionControl::with_fuel(8))
            .expect("machine");
        let step = machine.step().expect("step");
        // A matched rule rejects locally; an unmatched rule falls through to
        // the unconditional forward and dispatches it instead.
        Ok(matches!(
            step,
            mosdns_sequence_core::MachineStep::Dispatch(_)
        ))
    }

    #[test]
    fn matcher_expressions_follow_the_documented_grammar_and_negation() {
        // A matching rule rejects locally, so no dispatch is observed; a
        // missing rule falls through to the forward instead.
        // `_true` and `!_false` both match, so the reject runs.
        assert!(!matcher_expression("_true").expect("_true compiles"));
        assert!(!matcher_expression("!_false").expect("!_false compiles"));
        // `_false` and `!_true` both miss, so the forward is reached.
        assert!(matcher_expression("_false").expect("_false compiles"));
        assert!(matcher_expression("!_true").expect("!_true compiles"));
        // A named domain set matches the blocked name; its negation misses.
        assert!(!matcher_expression("qname $rules").expect("named qname"));
        assert!(matcher_expression("!qname $rules").expect("negated qname"));
        assert!(!matcher_expression("qname full:blocked.test").expect("inline qname"));
        assert!(!matcher_expression("qtype 1").expect("qtype"));
        assert!(matcher_expression("qtype 65").expect("qtype miss"));
        // Nothing has formed a response at this rule yet, so `has_resp`
        // misses, while its negation matches.
        assert!(matcher_expression("has_resp").expect("has_resp compiles"));
        assert!(!matcher_expression("!has_resp").expect("negated has_resp"));
        // No answer has been observed either, so `resp_ip` misses.
        assert!(matcher_expression("resp_ip 192.0.2.1").expect("resp_ip compiles"));
    }

    #[test]
    fn unsupported_or_malformed_matchers_are_rejected_at_load_time() {
        for expression in [
            "unknown_matcher",
            "qtype",
            "qtype notanumber",
            "has_resp extra",
            "resp_ip ::1",
            "resp_ip 192.0.2.1/24",
            "qname $missing_set",
            "qname &file.txt",
            "_true extra",
        ] {
            assert!(
                matcher_expression(expression).is_err(),
                "`{expression}` must be rejected at load time"
            );
        }
    }

    fn native_plugin_yaml(sequence: &str, definitions: &str) -> String {
        format!(
            r#"
log: {{ level: error }}
plugins:
  - tag: entry
    type: sequence
    args:
{sequence}
{definitions}
  - tag: forward
    type: forward
    args: {{ upstreams: [ {{ addr: udp://127.0.0.1:15353 }} ] }}
  - tag: listener
    type: udp_server
    args: {{ entry: entry, listen: "127.0.0.1:15352", enable_audit: false }}
"#
        )
    }

    #[test]
    fn fast_mark_and_flow_setter_support_quick_and_named_forms() {
        let quick = native_plugin_yaml(
            "      - exec: fast_mark 1 2\n      - matches: fast_mark 2 7\n        exec: flow_setter group=quick sequence=quick_seq upstream=quick_up\n      - exec: $forward\n",
            "",
        );
        let config = compile_yaml(&quick).expect("quick native plugins compile");
        let mut machine = config
            .new_machine(
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
                ),
                ExecutionControl::with_fuel(16),
            )
            .expect("quick machine");
        let step = machine.step().expect("quick machine step");
        assert!(matches!(
            step,
            mosdns_sequence_core::MachineStep::Dispatch(_)
        ));
        assert_eq!(machine.state().fast_flags, (1 << 1) | (1 << 2));
        assert_eq!(
            machine.state().routing.matched_group.as_deref(),
            Some("quick")
        );
        assert_eq!(
            machine.state().routing.final_sequence.as_deref(),
            Some("quick_seq")
        );
        assert_eq!(
            machine.state().routing.final_upstream.as_deref(),
            Some("quick_up")
        );

        let named = native_plugin_yaml(
            "      - exec: $mark\n      - matches: $mark\n        exec: $setter\n      - exec: $forward\n",
            "  - tag: mark\n    type: fast_mark\n    args: \"3 4\"\n  - tag: setter\n    type: flow_setter\n    args: { matched_group: named, final_sequence: named_seq, final_upstream: named_up }\n",
        );
        let config = compile_yaml(&named).expect("named native plugins compile");
        let mut machine = config
            .new_machine(
                ExecutionState::new(
                    mosdns_dns_core::QueryHeader {
                        id: 2,
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
                ),
                ExecutionControl::with_fuel(16),
            )
            .expect("named machine");
        let step = machine.step().expect("named machine step");
        assert!(matches!(
            step,
            mosdns_sequence_core::MachineStep::Dispatch(_)
        ));
        assert_eq!(machine.state().fast_flags, (1 << 3) | (1 << 4));
        assert_eq!(
            machine.state().routing.matched_group.as_deref(),
            Some("named")
        );
    }

    #[test]
    fn native_plugin_compilation_rejects_bad_values_with_paths() {
        let cases = [
            ("fast_mark 64", "$.plugins[0].args[0].exec"),
            ("fast_mark nope", "$.plugins[0].args[0].exec"),
            ("flow_setter unknown=x", "$.plugins[0].args[0].exec"),
            ("flow_setter group=", "$.plugins[0].args[0].exec"),
        ];
        for (expression, expected_path) in cases {
            let yaml = native_plugin_yaml(
                &format!("      - exec: {expression}\n      - exec: $forward\n"),
                "",
            );
            let error = match compile_yaml(&yaml) {
                Ok(_) => panic!("invalid quick setup must fail"),
                Err(error) => error,
            };
            assert_eq!(error.path, expected_path, "{expression}: {error}");
        }

        let normal = native_plugin_yaml(
            "      - exec: $setter\n      - exec: $forward\n",
            "  - tag: setter\n    type: flow_setter\n    args: { unknown: value }\n",
        );
        let error = match compile_yaml(&normal) {
            Ok(_) => panic!("unknown normal key must fail"),
            Err(error) => error,
        };
        assert_eq!(error.path, "$.plugins[1].args.unknown");

        let cross_type = native_plugin_yaml(
            "      - matches: $setter\n        exec: $forward\n",
            "  - tag: setter\n    type: flow_setter\n    args: { matched_group: named }\n",
        );
        let error = match compile_yaml(&cross_type) {
            Ok(_) => panic!("flow setter is not a matcher"),
            Err(error) => error,
        };
        assert_eq!(error.path, "$.plugins[0].args[0].matches[0]");
    }
}
