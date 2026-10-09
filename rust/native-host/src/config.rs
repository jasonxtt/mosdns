use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::{Ipv6Addr, SocketAddr};
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

pub use crate::domain_output::{DomainOutput, DomainOutputConfig};
use crate::managed::{DomainSetHandle, ManagedDomainSet};
use crate::matchers::{
    ClientIpMatcher, DomainSetError, HasResponseMatcher, QnameMatcher, QtypeMatcher,
    ResponseIpMatcher, TrueMatcher, build_domain_set_with_inputs, resolve_rule_path,
};
use crate::plugins::{
    DropRespExecutor, FastMarkConfig, FlowSetterConfig, MarkConfig, RcodeConfig, TagSetterConfig,
    parse_sleep_millis,
};
use crate::policy::{self, IpSetConfig, ResponsePolicy, ResponsePolicyConfig, TtlPolicy};
pub use crate::requery::{Requery, RequeryConfig};
pub use crate::rewrite::RewriteConfig;
use crate::rewrite::{DnsServer, Rewrite};
use crate::webinfo::WebInfo;

pub(crate) mod aliapi;
mod check;
pub(crate) mod providers;
pub(crate) mod settings;
pub(crate) mod standard_groups;
pub use check::{CheckFinding, CheckFindingKind, CheckReport, check_config, check_config_in};
pub(crate) use providers::presentation_name;
pub use providers::{DomainMapperConfig, MapperOutcome};
pub use settings::EffectiveItem;

/// The diagnostic threshold of the `log.level` section; ordered from the most
/// to the least verbose.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// The compiled `log` section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogSettings {
    pub level: LogLevel,
    /// Diagnostics are appended here instead of stderr; already resolved
    /// against the configuration root.
    pub file: Option<PathBuf>,
    /// A `level` that is not a known name; the level then falls back to `info`.
    pub unknown_level: Option<String>,
}

/// The listener transport accepted by the native host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
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
    pub(crate) response_identity: std::sync::Arc<str>,
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
    /// SOCKS5 proxy (`host:port`) that TCP-based schemes connect through.
    /// Kept only for `tcp`/`tls`/`https` targets; UDP ignores a proxy.
    pub socks5: Option<String>,
}

impl ForwardTargetConfig {
    /// True when the connection is made through the SOCKS5 proxy.
    #[must_use]
    pub(crate) fn proxied(&self) -> bool {
        self.socks5.is_some() && self.scheme != ForwardScheme::Udp
    }
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
                response_identity: std::sync::Arc::from(identity.as_str()),
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

/// A compile-time cache identity. One id owns exactly one cache store and one
/// lifecycle owner; a named cache keeps its configured tag, while a quick cache
/// owns a synthetic tag that is never exposed through the management catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CacheId(pub usize);

/// How one compiled cache was declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheKind {
    /// A `type: cache` plugin definition. It is managed: catalog listing, dump
    /// persistence and the plugin management actions apply to it.
    Named,
    /// An inline `exec: cache` callsite. It is owned and drained by the host
    /// like a named cache but has no tag, no dump and no public metrics.
    Quick,
}

/// One native cache dispatch accepted by this host. Several named caches may
/// coexist; every callsite that uses the inline form owns a private instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePluginConfig {
    pub id: CacheId,
    /// The configured tag for a named cache, or a synthetic stable identity for
    /// a quick callsite. Only [`CacheKind::Named`] identities are public.
    pub tag: String,
    pub executable: ExecutableId,
    pub kind: CacheKind,
    /// The configured entry capacity, passed to the bounded cache store.
    pub capacity: u64,
    pub enable_ecs: bool,
    /// Milliseconds-free retention window for a stale-but-usable response. Zero
    /// disables lazy retention entirely.
    pub lazy_cache_ttl_secs: u32,
    /// The resolved absolute dump target, when one was configured.
    pub dump_file: Option<PathBuf>,
    /// The periodic dump interval. It is accepted even without a dump file.
    pub dump_interval_secs: u64,
    /// Declared `exclude_ip` entries, verbatim, including ones that fail to
    /// parse. Invalid entries are warned and skipped at store time, matching
    /// the product behavior, so they must survive compilation.
    pub exclude_ip: Vec<String>,
}

impl CachePluginConfig {
    /// The management identity of this cache, when it has one.
    #[must_use]
    pub fn public_tag(&self) -> Option<&str> {
        match self.kind {
            CacheKind::Named => Some(self.tag.as_str()),
            CacheKind::Quick => None,
        }
    }
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

/// One compiled `sleep`: the external the sequence dispatches and its wait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleepConfig {
    pub tag: String,
    pub executable: ExecutableId,
    pub duration: Duration,
}

/// The compiled plugins that own a state file or a lookup of their own:
/// `rewrite` and `domain_output` (dispatched as externals) and `webinfo`
/// (HTTP only).
#[derive(Default)]
pub struct LocalPlugins {
    pub rewrites: Vec<RewriteConfig>,
    pub webinfos: Vec<Rc<WebInfo>>,
    pub domain_outputs: Vec<DomainOutputConfig>,
    pub requeries: Vec<RequeryConfig>,
}

impl LocalPlugins {
    /// Loads the `domain_output` statistics and starts their writers. Called
    /// when this view starts serving.
    pub(crate) fn start(&self, api: &crate::local_url::LocalApi) {
        for output in &self.domain_outputs {
            output.runtime.start(api);
        }
        for requery in &self.requeries {
            requery.runtime.start(api);
        }
    }

    /// A newer view takes over: every `domain_output` saves and stops writing.
    pub(crate) fn fence(&self) {
        for output in &self.domain_outputs {
            output.runtime.fence();
        }
        for requery in &self.requeries {
            requery.runtime.fence();
        }
    }

    /// Stops every `domain_output` writer and saves once more.
    pub(crate) async fn close(&self) {
        for requery in &self.requeries {
            requery.runtime.close().await;
        }
        for output in &self.domain_outputs {
            output.runtime.close().await;
        }
    }
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
    /// Semantic identity of the accepted rules, independent of its display
    /// tag or source path. Cache dependency closure uses this instead of
    /// invalidating every cache when any managed rule file changes.
    pub(crate) semantic_sha256: String,
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

/// The file-backed inputs of one compilation, kept so the same configuration
/// can be compiled again with other settings.
#[derive(Clone, Debug)]
pub(crate) struct ReloadSource {
    pub(crate) config_path: PathBuf,
    pub(crate) base_dir: PathBuf,
    pub(crate) root_dir: PathBuf,
}

/// The typed, validated graph consumed by pre-I/O host assembly.
pub struct CompiledConfig {
    /// Captured top-level configuration base for startup-only UI mounts.
    pub(crate) ui_base: Option<PathBuf>,
    /// The typed settings this generation was compiled with and the report of
    /// every settable item's effective value and source.
    pub(crate) settings: settings::Settings,
    pub(crate) settings_report: Vec<EffectiveItem>,
    /// The upstream list of every `aliapi` in the UI shape.
    pub(crate) effective_upstreams: BTreeMap<String, Vec<settings::UpstreamEntry>>,
    /// What the YAML alone gives for every settable item.
    pub(crate) yaml_defaults: settings::YamlDefaults,
    /// How to compile the same configuration again (hot apply of settings).
    pub(crate) reload: Option<ReloadSource>,
    pub(crate) standard_groups: Option<standard_groups::Profile>,
    pub(crate) compile_inputs: crate::special_groups::CandidateInputSet,
    /// Runtime generation qualifies snapshot-local executable/metric IDs.
    pub(crate) generation: u64,
    /// Per-cache transitive wire-policy dependency fingerprints. `None`
    /// means the compiler cannot prove reuse safe for that cache owner.
    pub(crate) cache_dependencies: Vec<Option<String>>,
    pub(crate) response_identities: std::collections::HashMap<(usize, usize), std::sync::Arc<str>>,
    pub managed_profile: Option<crate::special_groups::ManagedProfile>,
    pub managed_router: Option<ManagedRouterConfig>,
    /// Ordered listener inventory. `listener` remains the primary helper view.
    pub listeners: Vec<ListenerConfig>,
    pub response_policies: Vec<ResponsePolicyConfig>,
    pub ip_sets: Vec<IpSetConfig>,
    /// Every compiled `domain_mapper`, in declaration order.
    pub domain_mappers: Vec<DomainMapperConfig>,
    pub response_ip_rules: Vec<ResponseIpRuleConfig>,
    pub log_level: LogLevel,
    /// Legacy convenience view of one reachable forward. This value does not
    /// determine configuration validity or runtime dispatch behavior.
    pub forward: Option<ForwardConfig>,
    /// Every validated upstream owner keyed by the executable that can
    /// dispatch it.
    pub forwards: Vec<ForwardConfig>,
    pub forward_definitions: Vec<ForwardDefinitionConfig>,
    pub forward_invocations: Vec<ForwardInvocationConfig>,
    /// Every compiled cache, named definitions first in declaration order and
    /// then quick callsites in compile order. `CacheId(i)` indexes this vector.
    pub caches: Vec<CachePluginConfig>,
    pub fallbacks: Vec<FallbackConfig>,
    pub preferences: Vec<PreferenceConfig>,
    pub sleeps: Vec<SleepConfig>,
    /// `rewrite` and `webinfo` plugins.
    pub local: LocalPlugins,
    pub sequence: SequenceConfig,
    pub listener: ListenerConfig,
    /// Every compiled `domain_set` with its management eligibility.
    pub domain_sets: Vec<DomainSetConfig>,
    /// Every declared switch1..17 owner in stable type order. Declarations
    /// are immutable compile output; live values live in the host registry.
    pub switches: Vec<crate::switch::SwitchDeclaration>,
    /// The scoped management HTTP listener, when one is configured.
    pub api: Option<ApiConfig>,
    pub program: ValidatedProgram,
    /// Management state of the rule providers and their `domain_mapper`s.
    pub(crate) rule_sources: Rc<crate::rule_sources::RuleSources>,
}

/// The native router's canonical named calls. All IDs belong to this program.
pub struct ManagedRouterConfig {
    pub sequence: SequenceId,
    pub groups: Vec<ManagedRouterGroup>,
}
pub struct ManagedRouterGroup {
    pub slot: u32,
    /// Snapshot-local indices into `CompiledConfig.domain_sets` in priority order.
    pub providers: [usize; 2],
    pub child: SequenceId,
}

pub struct ResponseIpRuleConfig {
    pub source_path: String,
    pub prefixes: Vec<Rc<crate::policy::IpListHandle>>,
}

impl CompiledConfig {
    /// Every settable item with its effective value and source.
    #[doc(hidden)]
    #[must_use]
    pub fn settings_report(&self) -> &[EffectiveItem] {
        &self.settings_report
    }

    /// The rule-source management state (tests drive and observe it).
    #[doc(hidden)]
    #[must_use]
    pub fn rule_sources(&self) -> &crate::rule_sources::RuleSources {
        &self.rule_sources
    }

    /// Looks up one compiled `domain_set` by tag.
    #[must_use]
    pub fn domain_set(&self, tag: &str) -> Option<&DomainSetConfig> {
        self.domain_sets.iter().find(|set| set.tag == tag)
    }

    /// Resolves the cache a dispatch executable belongs to, when it is one.
    #[must_use]
    pub fn cache_for_executable(&self, executable: ExecutableId) -> Option<&CachePluginConfig> {
        self.caches
            .iter()
            .find(|cache| cache.executable == executable)
    }

    /// The named caches in configuration order. Quick callsites are private
    /// implementation identities and are deliberately absent.
    pub fn named_caches(&self) -> impl Iterator<Item = &CachePluginConfig> {
        self.caches
            .iter()
            .filter(|cache| cache.kind == CacheKind::Named)
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
    /// Structured classification used by `check`; never part of the text.
    pub(crate) kind: ConfigErrorKind,
}

/// Rejection classes `check` must tell apart without matching error text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ConfigErrorKind {
    #[default]
    Other,
    /// A reference names a tag that no definition declares.
    UnknownReference { tag: String },
    /// A mapping key outside the accepted field list.
    UnsupportedField,
}

impl ConfigError {
    pub(crate) fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
            kind: ConfigErrorKind::Other,
        }
    }

    fn unknown_reference(
        path: impl Into<String>,
        reason: impl Into<String>,
        tag: impl Into<String>,
    ) -> Self {
        Self {
            kind: ConfigErrorKind::UnknownReference { tag: tag.into() },
            ..Self::new(path, reason)
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
    let display = path.display().to_string();
    let file_path = path.to_path_buf();
    let text = crate::transaction::blocking_io(move || std::fs::read_to_string(file_path))
        .map_err(|error| {
            ConfigError::new(
                display.clone(),
                format!("cannot read configuration: {error}"),
            )
        })?;
    text.map_err(|error| ConfigError::new(display, format!("cannot read configuration: {error}")))
}

pub(crate) fn load_yaml_with_inputs(
    path: &Path,
    inputs: &mut crate::special_groups::CandidateInputSet,
) -> Result<String, ConfigError> {
    let display = path.display().to_string();
    let file_path = path.to_path_buf();
    let bytes = crate::transaction::blocking_io(move || std::fs::read(file_path))
        .map_err(|error| {
            ConfigError::new(
                display.clone(),
                format!("cannot read configuration: {error}"),
            )
        })?
        .map_err(|error| {
            ConfigError::new(
                display.clone(),
                format!("cannot read configuration: {error}"),
            )
        })?;
    inputs
        .record_bytes(path, &bytes)
        .map_err(|reason| ConfigError::new(display.clone(), reason))?;
    String::from_utf8(bytes).map_err(|error| {
        ConfigError::new(
            path.display().to_string(),
            format!("configuration is not UTF-8: {error}"),
        )
    })
}

/// Reads and compiles one configuration file. Relative `include` paths and
/// every relative plugin-argument path (rule, state, dump files) resolve
/// against that file's directory, never against the process working
/// directory.
pub fn load_and_compile(path: &Path) -> Result<CompiledConfig, ConfigError> {
    let yaml = load_yaml(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
    let mut compiled = compile_yaml_with_base_and_origin(&yaml, base_dir, base_dir, Some(path))?;
    compiled.ui_base = Some(absolute_base(base_dir)?);
    Ok(compiled)
}

/// Strictly decodes and compiles an in-memory configuration that resolves
/// relative paths against the process working directory. Prefer
/// [`load_and_compile`] for file-backed configurations so that include and
/// rule-file paths are never guessed from an unrelated working directory.
pub fn compile_yaml(yaml: &str) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base(yaml, Path::new(""))
}

/// Strictly decodes and compiles the supported YAML subset with an explicit
/// base directory for relative include and plugin-argument paths.
pub fn compile_yaml_with_base(yaml: &str, base_dir: &Path) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base_and_origin(yaml, base_dir, base_dir, None)
}

/// Like [`compile_yaml_with_base`], with a separate configuration root for
/// relative plugin-argument paths (`mosdns start -d <root>`). `include` paths
/// still resolve against `base_dir`, the main configuration's directory.
pub fn compile_yaml_with_roots(
    yaml: &str,
    base_dir: &Path,
    root_dir: &Path,
) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base_and_origin(yaml, base_dir, root_dir, None)
}

/// The shared file-backed and in-memory compile entry. `origin` names the
/// configuration file itself when one exists, so switch state-file collision
/// checks can protect it like every other known artifact.
fn compile_yaml_with_base_and_origin(
    yaml: &str,
    base_dir: &Path,
    root_dir: &Path,
    origin: Option<&Path>,
) -> Result<CompiledConfig, ConfigError> {
    let settings = match origin {
        Some(_) => settings::Settings::load(root_dir)?,
        None => settings::Settings::default(),
    };
    compile_with_settings(yaml, base_dir, root_dir, origin, &settings, None)
}

/// Compiles with explicit typed settings (hot apply passes the candidate's).
fn compile_with_settings(
    yaml: &str,
    base_dir: &Path,
    root_dir: &Path,
    origin: Option<&Path>,
    settings: &settings::Settings,
    groups: Option<&[standard_groups::Group]>,
) -> Result<CompiledConfig, ConfigError> {
    let raw = parse_yaml(yaml)?;
    let mut inputs = crate::special_groups::CandidateInputSet::default();
    if let Some(origin) = origin {
        inputs
            .record_bytes(origin, yaml.as_bytes())
            .map_err(|reason| ConfigError::new(origin.display().to_string(), reason))?;
        let path = settings::Settings::path(root_dir);
        match std::fs::read(&path) {
            Ok(bytes) => inputs.record_bytes(&path, &bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                inputs.record_missing(&path)
            }
            Err(error) => {
                return Err(ConfigError::new(
                    path.display().to_string(),
                    error.to_string(),
                ));
            }
        }
        .map_err(|reason| ConfigError::new(path.display().to_string(), reason))?;
    }
    let mut compiled = compile_raw(
        &raw,
        base_dir,
        root_dir,
        None,
        None,
        origin.map(|_| &mut inputs),
        origin,
        settings,
        groups,
    )?;
    compiled.compile_inputs = inputs;
    if !base_dir.as_os_str().is_empty() {
        compiled.ui_base = Some(absolute_base(base_dir)?);
    }
    if let Some(origin) = origin {
        compiled.reload = Some(ReloadSource {
            config_path: origin.to_path_buf(),
            base_dir: base_dir.to_path_buf(),
            root_dir: root_dir.to_path_buf(),
        });
    }
    Ok(compiled)
}

/// Compiles `reload`'s configuration again with other settings.
pub(crate) fn recompile(
    reload: &ReloadSource,
    settings: &settings::Settings,
) -> Result<CompiledConfig, ConfigError> {
    let yaml = load_yaml(&reload.config_path)?;
    let mut compiled = compile_with_settings(
        &yaml,
        &reload.base_dir,
        &reload.root_dir,
        Some(&reload.config_path),
        settings,
        None,
    )?;
    compiled.ui_base = Some(absolute_base(&reload.base_dir)?);
    Ok(compiled)
}

/// `start`'s file-backed compile: the configuration file `path` with `base_dir`
/// for includes and `root_dir` for plugin paths; the typed settings under
/// `root_dir` apply.
pub(crate) fn compile_file_with_roots(
    yaml: &str,
    path: &Path,
    base_dir: &Path,
    root_dir: &Path,
) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base_and_origin(yaml, base_dir, root_dir, Some(path))
}

fn absolute_base(base: &Path) -> Result<PathBuf, ConfigError> {
    if base.is_absolute() {
        return Ok(base.to_owned());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(base))
        .map_err(|error| ConfigError::new("$", format!("configuration base: {error}")))
}

fn is_standard_include(expression: &str, base: &Path) -> bool {
    crate::special_groups::lexical_path(&resolve_relative(expression, base))
        == crate::special_groups::lexical_path(&base.join(standard_groups::GENERATED))
}

pub(crate) fn recompile_groups(
    reload: &ReloadSource,
    settings: &settings::Settings,
    groups: &[standard_groups::Group],
) -> Result<CompiledConfig, ConfigError> {
    let yaml = load_yaml(&reload.config_path)?;
    compile_with_settings(
        &yaml,
        &reload.base_dir,
        &reload.root_dir,
        Some(&reload.config_path),
        settings,
        Some(groups),
    )
}

pub(crate) fn compile_candidate_yaml(
    yaml: &str,
    base_dir: &Path,
    root_dir: &Path,
    profile: crate::special_groups::ManagedProfile,
    files: &crate::special_groups::StagedFiles,
    inputs: &mut crate::special_groups::CandidateInputSet,
) -> Result<CompiledConfig, ConfigError> {
    let settings = settings::Settings::load(root_dir)?;
    compile_raw(
        &parse_yaml(yaml)?,
        base_dir,
        root_dir,
        Some(profile),
        Some(files),
        Some(inputs),
        None,
        &settings,
        None,
    )
}

fn parse_yaml(yaml: &str) -> Result<RawValue, ConfigError> {
    yaml_serde::from_str(yaml)
        .map_err(|error| ConfigError::new("$", format!("invalid YAML: {error}")))
}

/// Preflight only the explicit opt-in before acquiring the state-root writer.
/// Full compilation is deliberately deferred until durable recovery completes.
pub(crate) fn management_enabled(yaml: &str) -> Result<bool, ConfigError> {
    managed_opt_in(&parse_yaml(yaml)?)
}

fn managed_opt_in(raw: &RawValue) -> Result<bool, ConfigError> {
    let root = expect_map(raw, "$", "top level must be a mapping")?;
    let Some(value) = root.get("native_management") else {
        return Ok(false);
    };
    let options = expect_map(
        value,
        "$.native_management",
        "native_management must be a mapping",
    )?;
    options.reject_unknown(&["special_groups"], "$.native_management")?;
    expect_bool(
        options.required("special_groups", "$.native_management")?,
        "$.native_management.special_groups",
    )
}

const ROOT_FIELDS: &[&str] = &["log", "include", "plugins", "api", "native_management"];

#[allow(clippy::too_many_arguments)]
fn compile_raw(
    raw: &RawValue,
    base_dir: &Path,
    root_dir: &Path,
    candidate: Option<crate::special_groups::ManagedProfile>,
    staged: Option<&crate::special_groups::StagedFiles>,
    mut inputs: Option<&mut crate::special_groups::CandidateInputSet>,
    config_origin: Option<&Path>,
    settings: &settings::Settings,
    groups: Option<&[standard_groups::Group]>,
) -> Result<CompiledConfig, ConfigError> {
    let root = expect_map(raw, "$", "top level must be a mapping")?;
    root.reject_unknown(ROOT_FIELDS, "$")?;
    let log = compile_log(root.required("log", "$")?, root_dir)?.level;
    let api = root.get("api").map(compile_api).transpose()?;
    let managed = if managed_opt_in(raw)? {
        Some(match candidate {
            Some(profile) => profile,
            None => crate::special_groups::ManagedProfile::load(base_dir)?,
        })
    } else {
        if candidate.is_some() {
            return Err(ConfigError::new(
                "native_management",
                "candidate requires the frozen managed opt-in",
            ));
        }
        None
    };

    // Definition collection: included plugin-only files load in declaration
    // order, then this file's plugins. No definition is resolved until the
    // whole ordered catalog exists, so a reference may name a later
    // definition without changing the effective order.
    let mut definitions = Vec::new();
    let mut include_paths = Vec::new();
    let mut standard = false;
    if let Some(includes) = root.get("include") {
        let includes = expect_sequence(includes, "$.include")?;
        for (index, include) in includes.iter().enumerate() {
            let expression = expect_string(include, &format!("$.include[{index}]"))?;
            include_paths.push(resolve_relative(&expression, base_dir));
            if managed.is_none()
                && config_origin.is_some()
                && is_standard_include(&expression, base_dir)
            {
                standard = true;
                continue;
            }
            if let Some(profile) = &managed {
                reject_generated_include(profile, &expression)?;
            }
            collect_included(
                &expression,
                base_dir,
                root_dir,
                &mut definitions,
                inputs.as_deref_mut(),
            )?;
        }
    }
    let plugins = expect_sequence(root.required("plugins", "$")?, "$.plugins")?;
    for (index, plugin) in plugins.iter().enumerate() {
        definitions.push(decode_plugin(
            plugin,
            &format!("$.plugins[{index}]"),
            root_dir,
        )?);
    }

    if let Some(profile) = &managed {
        apply_managed_definitions(profile, &mut definitions)?;
    }
    let sources = CompileSources {
        config_origin,
        include_paths: &include_paths,
        settings,
        standard: standard.then_some((root_dir, groups)),
    };
    finish_compile(log, api, definitions, managed, staged, inputs, &sources)
}

/// Applies the managed opt-in to the collected definitions: reserved tag
/// check, `aliapi` descriptor normalization, upstream overrides, and the
/// generated special_groups plugins.
fn apply_managed_definitions(
    profile: &crate::special_groups::ManagedProfile,
    definitions: &mut Vec<RawPlugin>,
) -> Result<(), ConfigError> {
    if let Some(plugin) = definitions
        .iter()
        .find(|plugin| crate::special_groups::reserved_tag(&plugin.tag))
    {
        return Err(ConfigError::new(
            &plugin.source_path,
            "reserved generated plugin tag collision",
        ));
    }
    // `aliapi` is also the legacy wrapper name for ordinary DNS entries.
    // Normalize only its descriptor to the native forward compiler. Signed
    // API fields and protocols still fail that strict compiler; no fallback.
    for plugin in definitions.iter_mut() {
        if plugin.kind == "aliapi" {
            plugin.kind = "forward".into();
        }
    }
    for (tag, entries) in &profile.overrides {
        if tag.starts_with("special_upstream_") {
            continue;
        }
        let plugin = definitions
            .iter_mut()
            .find(|plugin| &plugin.tag == tag && plugin.kind == "forward")
            .ok_or_else(|| {
                ConfigError::new(
                    "webinfo/upstream_overrides.json",
                    format!("unknown forward tag `{tag}`"),
                )
            })?;
        let translated = crate::special_groups::forward_entries(entries, tag)?;
        let RawValue::Map(args) = &mut plugin.args else {
            return Err(ConfigError::new(tag, "forward args must be a mapping"));
        };
        let raw = parse_yaml(
            &serde_json::to_string(&translated)
                .map_err(|e| ConfigError::new(tag, e.to_string()))?,
        )?;
        args.entries.retain(|(key, _)| key != "upstreams");
        args.entries.push(("upstreams".into(), raw));
    }
    let generated = parse_yaml(&profile.generated_yaml)?;
    let generated = expect_map(
        &generated,
        crate::special_groups::GENERATED_PATH,
        "generated input",
    )?;
    let plugins = expect_sequence(
        generated.required("plugins", "generated")?,
        "generated.plugins",
    )?;
    for (index, plugin) in plugins.iter().enumerate() {
        definitions.push(decode_plugin(
            plugin,
            &format!("@managed-generated.plugins[{index}]"),
            &profile.base_dir,
        )?);
    }
    Ok(())
}

fn reject_generated_include(
    profile: &crate::special_groups::ManagedProfile,
    expression: &str,
) -> Result<(), ConfigError> {
    let path = resolve_relative(expression, &profile.base_dir);
    let generated = profile.base_dir.join(crate::special_groups::GENERATED_PATH);
    if crate::special_groups::lexical_path(&path) == generated
        || canonicalized_path(&path)
            .is_some_and(|path| canonicalized_path(&generated).as_ref() == Some(&path))
    {
        return Err(ConfigError::new(
            "$.include",
            "generated special_groups input must not be explicitly included",
        ));
    }
    Ok(())
}

/// Compiles fully collected definitions; shared by `start` and `check`.
fn finish_compile(
    log: LogLevel,
    api: Option<ApiConfig>,
    definitions: Vec<RawPlugin>,
    managed: Option<crate::special_groups::ManagedProfile>,
    staged: Option<&crate::special_groups::StagedFiles>,
    mut inputs: Option<&mut crate::special_groups::CandidateInputSet>,
    sources: &CompileSources<'_>,
) -> Result<CompiledConfig, ConfigError> {
    let mut definitions = definitions;
    let mut applied = settings::apply(&mut definitions, sources.settings, managed.is_some())?;
    let standard = match sources.standard {
        Some((base, groups)) => Some(standard_groups::apply(
            base,
            groups,
            &mut definitions,
            &mut applied,
        )?),
        None => None,
    };
    if let (Some(profile), Some(inputs)) = (&standard, inputs.as_deref_mut()) {
        let path = profile.base.join(standard_groups::STATE);
        match &profile.expected_state {
            Some(bytes) => inputs.record_bytes(&path, bytes),
            None => inputs.record_missing(&path),
        }
        .map_err(|reason| ConfigError::new(standard_groups::STATE, reason))?;
    }
    let dependency_definitions = definitions.clone();
    let mut compiled = compile_definitions(
        log,
        api,
        definitions,
        managed.is_some(),
        staged,
        inputs,
        sources,
    )?;
    if let Some(profile) = managed {
        let router = compiled
            .program
            .sequence_id(crate::special_groups::ROUTER_TAG)
            .ok_or_else(|| ConfigError::new("native_management", "generated router missing"))?;
        if !reachable_sequence(
            &compiled,
            compiled.sequence.sequence,
            router,
            &mut BTreeSet::new(),
        ) {
            return Err(ConfigError::new(
                "native_management",
                "special_groups hook must be reachable from primary entry",
            ));
        }
        let groups = profile
            .groups
            .iter()
            .filter(|g| !g.custom_port_only)
            .map(|g| {
                let provider = |tag: String| {
                    compiled
                        .domain_sets
                        .iter()
                        .position(|set| set.tag == tag)
                        .ok_or_else(|| {
                            ConfigError::new("native_management", "group provider missing")
                        })
                };
                Ok(ManagedRouterGroup {
                    slot: g.slot,
                    providers: [
                        provider(format!("special_route_{}", g.slot))?,
                        provider(format!("special_manual_{}", g.slot))?,
                    ],
                    child: compiled
                        .program
                        .sequence_id(&format!("sequence_special_{}", g.slot))
                        .ok_or_else(|| {
                            ConfigError::new("native_management", "group sequence missing")
                        })?,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        compiled.managed_router = Some(ManagedRouterConfig {
            sequence: router,
            groups,
        });
        compiled.managed_profile = Some(profile);
    }
    compiled.cache_dependencies = cache_dependency_fingerprints(&compiled, &dependency_definitions);
    compiled.settings = sources.settings.clone();
    compiled.settings_report = applied.items;
    compiled.effective_upstreams = applied.upstreams;
    compiled.yaml_defaults = applied.defaults;
    compiled.standard_groups = standard;
    Ok(compiled)
}

fn cache_dependency_fingerprints(
    config: &CompiledConfig,
    definitions: &[RawPlugin],
) -> Vec<Option<String>> {
    let plugins: BTreeMap<String, &RawPlugin> = definitions
        .iter()
        .map(|plugin| (plugin.tag.clone(), plugin))
        .collect();
    config
        .caches
        .iter()
        .map(|cache| {
            let roots: BTreeSet<SequenceId> = config
                .program
                .sequences
                .iter()
                .filter(|sequence| {
                    sequence.rules.iter().any(|rule| {
                        matches!(
                            rule.executable,
                            Some(ValidatedExecutable::External { target }) if target == cache.executable
                        )
                    })
                })
                .map(|sequence| sequence.id)
                .collect();
            if roots.is_empty() {
                return None;
            }

            let mut closure = CacheDependencyClosure::new(config, &plugins);
            closure
                .material
                .insert(cache_config_material(cache));
            for root in &roots {
                closure.add_sequence(*root);
                closure.add_group_context_for_sequence(*root);
            }

            for listener in &config.listeners {
                let Some(entry) = config.program.sequence_id(&listener.entry) else {
                    closure.refuse_reuse();
                    continue;
                };
                let reaches_cache = roots.iter().any(|root| {
                    reachable_sequence(config, entry, *root, &mut BTreeSet::new())
                });
                if !reaches_cache {
                    continue;
                }
                closure.material.insert(format!(
                    "listener-context:{:?}:{}",
                    listener.kind, listener.entry
                ));
                for root in &roots {
                    closure.add_ancestor_context(entry, *root, &mut BTreeSet::new());
                }
            }
            closure.drain();
            closure.safe.then(|| {
                crate::special_groups::sha256(
                    closure
                        .material
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("\n")
                        .as_bytes(),
                )
            })
        })
        .collect()
}

struct CacheDependencyClosure<'a> {
    config: &'a CompiledConfig,
    plugins: &'a BTreeMap<String, &'a RawPlugin>,
    material: BTreeSet<String>,
    pending_sequences: Vec<SequenceId>,
    pending_externals: Vec<ExecutableId>,
    pending_fixtures: Vec<ExecutableId>,
    seen_sequences: BTreeSet<SequenceId>,
    seen_externals: BTreeSet<ExecutableId>,
    seen_fixtures: BTreeSet<ExecutableId>,
    seen_plugins: BTreeSet<String>,
    safe: bool,
}

impl<'a> CacheDependencyClosure<'a> {
    fn new(config: &'a CompiledConfig, plugins: &'a BTreeMap<String, &'a RawPlugin>) -> Self {
        Self {
            config,
            plugins,
            material: BTreeSet::new(),
            pending_sequences: Vec::new(),
            pending_externals: Vec::new(),
            pending_fixtures: Vec::new(),
            seen_sequences: BTreeSet::new(),
            seen_externals: BTreeSet::new(),
            seen_fixtures: BTreeSet::new(),
            seen_plugins: BTreeSet::new(),
            safe: true,
        }
    }

    fn refuse_reuse(&mut self) {
        self.safe = false;
    }

    fn drain(&mut self) {
        loop {
            if let Some(sequence) = self.pending_sequences.pop() {
                self.visit_sequence(sequence);
                continue;
            }
            if let Some(executable) = self.pending_externals.pop() {
                self.visit_external(executable);
                continue;
            }
            if let Some(executable) = self.pending_fixtures.pop() {
                self.visit_fixture(executable);
                continue;
            }
            break;
        }
    }

    fn add_sequence(&mut self, sequence: SequenceId) {
        self.pending_sequences.push(sequence);
    }

    fn visit_sequence(&mut self, id: SequenceId) {
        if !self.seen_sequences.insert(id) {
            return;
        }
        let Some(sequence) = self.config.program.sequence(id) else {
            self.refuse_reuse();
            return;
        };
        if let Some(plugin) = self.plugins.get(&sequence.name).copied() {
            self.add_plugin(plugin);
            self.add_references(&plugin.args);
        } else if sequence.synthetic {
            self.material
                .insert(format!("synthetic-sequence-rules:{}", sequence.rules.len()));
            for (index, rule) in sequence.rules.iter().enumerate() {
                if !rule.matchers.is_empty() {
                    self.refuse_reuse();
                }
                match rule.executable.as_ref() {
                    Some(executable) => {
                        if let Some(material) = self.executable_material(executable) {
                            self.material
                                .insert(format!("synthetic-sequence-exec:{index}:{material}"));
                        } else {
                            self.refuse_reuse();
                        }
                    }
                    None => self.refuse_reuse(),
                }
            }
        } else {
            self.refuse_reuse();
            return;
        }
        for rule in &sequence.rules {
            let Some(executable) = &rule.executable else {
                continue;
            };
            match executable {
                ValidatedExecutable::Call { target }
                | ValidatedExecutable::Goto { target }
                | ValidatedExecutable::Jump { target }
                | ValidatedExecutable::Inline { target } => self.add_sequence(*target),
                ValidatedExecutable::Try { target } => match target {
                    ExecutableTarget::Sequence(target) => self.add_sequence(*target),
                    ExecutableTarget::Fixture(target) => {
                        self.pending_fixtures.push(*target);
                    }
                },
                ValidatedExecutable::Fixture { target } => {
                    self.pending_fixtures.push(*target);
                }
                ValidatedExecutable::External { target } => {
                    self.pending_externals.push(*target);
                }
                ValidatedExecutable::Accept
                | ValidatedExecutable::Reject { .. }
                | ValidatedExecutable::Return
                | ValidatedExecutable::Exit => {}
            }
        }
    }

    fn executable_material(&self, executable: &ValidatedExecutable) -> Option<String> {
        let sequence = |target: SequenceId| {
            let sequence = self.config.program.sequence(target)?;
            Some(if sequence.synthetic {
                "synthetic".to_owned()
            } else {
                format!("sequence:{}", sequence.name)
            })
        };
        let target_label = |target: NativeTarget| match target {
            NativeTarget::Sequence(sequence_id) => sequence(sequence_id),
            NativeTarget::Fixture(executable_id) => self
                .config
                .program
                .fixture(executable_id)
                .map(|fixture| format!("fixture:{}", fixture.name)),
            NativeTarget::External(executable_id) => self
                .config
                .program
                .external(executable_id)
                .map(|external| format!("external:{}", external.name)),
        };
        Some(match executable {
            ValidatedExecutable::Accept => "accept".to_owned(),
            ValidatedExecutable::Reject { rcode } => format!("reject:{rcode}"),
            ValidatedExecutable::Return => "return".to_owned(),
            ValidatedExecutable::Call { target: id } => format!("call:{}", sequence(*id)?),
            ValidatedExecutable::Goto { target: id } => format!("goto:{}", sequence(*id)?),
            ValidatedExecutable::Jump { target: id } => format!("jump:{}", sequence(*id)?),
            ValidatedExecutable::Exit => "exit".to_owned(),
            ValidatedExecutable::Try { target: item } => {
                let typed_target = match item {
                    ExecutableTarget::Sequence(id) => NativeTarget::Sequence(*id),
                    ExecutableTarget::Fixture(id) => NativeTarget::Fixture(*id),
                };
                format!("try:{}", target_label(typed_target)?)
            }
            ValidatedExecutable::Fixture { target: id } => {
                format!("fixture:{}", self.config.program.fixture(*id)?.name)
            }
            ValidatedExecutable::External { target: id } => {
                format!("external:{}", self.config.program.external(*id)?.name)
            }
            ValidatedExecutable::Inline { target: id } => {
                format!("inline:{}", sequence(*id)?)
            }
        })
    }

    fn add_ancestor_context(
        &mut self,
        current: SequenceId,
        goal: SequenceId,
        visited: &mut BTreeSet<(SequenceId, SequenceId)>,
    ) {
        if current == goal || !visited.insert((current, goal)) {
            return;
        }
        if self
            .config
            .managed_router
            .as_ref()
            .is_some_and(|router| router.sequence == current)
        {
            let groups: Vec<u32> = self
                .config
                .managed_router
                .as_ref()
                .into_iter()
                .flat_map(|router| router.groups.iter())
                .filter(|group| {
                    reachable_sequence(self.config, group.child, goal, &mut BTreeSet::new())
                })
                .map(|group| group.slot)
                .collect();
            for slot in groups {
                self.add_group_context(slot);
            }
            return;
        }
        let Some(sequence) = self.config.program.sequence(current) else {
            self.refuse_reuse();
            return;
        };
        let Some(plugin) = self.plugins.get(&sequence.name).copied() else {
            self.refuse_reuse();
            return;
        };
        self.add_plugin(plugin);
        self.add_references(&plugin.args);
        for rule in &sequence.rules {
            match rule.executable.as_ref() {
                Some(ValidatedExecutable::Call { target })
                | Some(ValidatedExecutable::Goto { target })
                | Some(ValidatedExecutable::Jump { target })
                | Some(ValidatedExecutable::Inline { target }) => {
                    self.add_ancestor_context(*target, goal, visited);
                }
                Some(ValidatedExecutable::Try {
                    target: ExecutableTarget::Sequence(sequence),
                }) => self.add_ancestor_context(*sequence, goal, visited),
                Some(ValidatedExecutable::Try {
                    target: ExecutableTarget::Fixture(executable),
                })
                | Some(ValidatedExecutable::Fixture { target: executable }) => {
                    self.pending_fixtures.push(*executable);
                }
                Some(ValidatedExecutable::External { target }) => {
                    self.pending_externals.push(*target);
                }
                _ => {}
            }
        }
    }

    fn add_group_context_for_sequence(&mut self, sequence: SequenceId) {
        let Some(sequence) = self.config.program.sequence(sequence) else {
            return;
        };
        let Some(slot) = sequence
            .name
            .strip_prefix("sequence_special_")
            .and_then(|slot| slot.parse::<u32>().ok())
        else {
            return;
        };
        self.add_group_context(slot);
    }

    fn add_group_context(&mut self, slot: u32) {
        let Some(profile) = self.config.managed_profile.as_ref() else {
            self.refuse_reuse();
            return;
        };
        let Some(group) = profile.groups.iter().find(|group| group.slot == slot) else {
            self.refuse_reuse();
            return;
        };
        self.material.insert(format!(
            "managed-group:{slot}:main-enabled:{}",
            !group.custom_port_only
        ));
        if let Some(router_group) = self
            .config
            .managed_router
            .as_ref()
            .and_then(|router| router.groups.iter().find(|item| item.slot == slot))
        {
            for provider in router_group.providers {
                if let Some(domain_set) = self.config.domain_sets.get(provider) {
                    self.material.insert(format!(
                        "managed-route:{}:{}",
                        domain_set.tag, domain_set.semantic_sha256
                    ));
                } else {
                    self.refuse_reuse();
                }
            }
        }
    }

    fn visit_external(&mut self, id: ExecutableId) {
        if !self.seen_externals.insert(id) {
            return;
        }
        let Some(external) = self.config.program.external(id) else {
            self.refuse_reuse();
            return;
        };
        self.material.insert(format!("external:{}", external.name));
        let mut recognized = false;
        if let Some(plugin) = self.plugins.get(&external.name).copied() {
            self.add_plugin(plugin);
            recognized = true;
        }
        if let Some(invocation) = self
            .config
            .forward_invocations
            .iter()
            .find(|invocation| invocation.executable == id)
        {
            recognized = true;
            let Some(definition) = self.config.forward_definitions.get(invocation.definition)
            else {
                self.refuse_reuse();
                return;
            };
            self.material.insert(format!(
                "forward-definition:{}:{}",
                definition.tag, definition.concurrent
            ));
            for index in &invocation.entries {
                if let Some(entry) = definition.entries.get(*index) {
                    self.material
                        .insert(format!("forward-entry:{}:{entry:?}", definition.tag));
                } else {
                    self.refuse_reuse();
                }
            }
        }
        if let Some(cache) = self
            .config
            .caches
            .iter()
            .find(|cache| cache.executable == id)
        {
            self.material.insert(cache_config_material(cache));
            recognized = true;
        }
        if let Some(fallback) = self
            .config
            .fallbacks
            .iter()
            .find(|fallback| fallback.executable == id)
        {
            recognized = true;
            self.material.insert(format!(
                "fallback:{}:{}:{}:{}:{}",
                fallback.tag,
                target_name(self.config, fallback.primary),
                target_name(self.config, fallback.secondary),
                fallback.threshold.as_millis(),
                fallback.always_standby
            ));
            self.add_target(fallback.primary);
            self.add_target(fallback.secondary);
        }
        if let Some(preference) = self
            .config
            .preferences
            .iter()
            .find(|preference| preference.executable == id)
        {
            let family = match preference.family {
                PreferenceFamily::Ipv4 => "ipv4",
                PreferenceFamily::Ipv6 => "ipv6",
            };
            self.material.insert(format!("preference:{family}"));
            recognized = true;
        }
        if let Some(policy) = self
            .config
            .response_policies
            .iter()
            .find(|policy| policy.executable == id)
        {
            self.material
                .insert(format!("response-policy:{}", policy.tag));
            // A fixed-address ECS may follow a cache, so its parameters (not
            // only its name) decide whether an old cache is still valid.
            if let ResponsePolicy::Ecs(ecs) = &policy.policy {
                self.material
                    .insert(format!("ecs-policy:{}:{ecs:?}", policy.tag));
            }
            recognized = true;
        }
        if let Some(sleep) = self
            .config
            .sleeps
            .iter()
            .find(|sleep| sleep.executable == id)
        {
            self.material
                .insert(format!("sleep:{}", sleep.duration.as_millis()));
            recognized = true;
        }
        if external.name.starts_with("@native-cname:") || external.name.starts_with("@native-ecs:")
        {
            recognized = true;
        }
        if !recognized {
            self.refuse_reuse();
        }
    }

    fn visit_fixture(&mut self, id: ExecutableId) {
        if !self.seen_fixtures.insert(id) {
            return;
        }
        let Some(fixture) = self.config.program.fixture(id) else {
            self.refuse_reuse();
            return;
        };
        self.material.insert(format!("fixture:{}", fixture.name));
        if let Some(tag) = fixture.name.strip_prefix("__native_plugin_") {
            if let Some(plugin) = self.plugins.get(tag).copied() {
                self.add_plugin(plugin);
            } else {
                self.refuse_reuse();
            }
        } else if !fixture.name.starts_with("__native_quick_") {
            self.refuse_reuse();
        }
    }

    fn add_target(&mut self, target: NativeTarget) {
        match target {
            NativeTarget::Sequence(sequence) => self.add_sequence(sequence),
            NativeTarget::Fixture(executable) => self.pending_fixtures.push(executable),
            NativeTarget::External(executable) => self.pending_externals.push(executable),
        }
    }

    fn add_plugin(&mut self, plugin: &RawPlugin) {
        if !self.seen_plugins.insert(plugin.tag.clone()) {
            return;
        }
        self.material.insert(format!(
            "plugin:{}:{}:{:?}:{:?}",
            plugin.tag, plugin.kind, plugin.args, plugin.root_dir
        ));
        if matches!(
            plugin.kind.as_str(),
            "hosts" | "redirect" | "ip_set" | "si_set" | "domain_mapper" | "adguard_rule"
        ) || (plugin.kind == "sequence" && sequence_has_file_backed_ip_matcher(&plugin.args))
        {
            // These loaders currently expose no stable, complete semantic
            // fingerprint. Caches depending on them must start fresh. The
            // response-IP matcher also accepts external `&file` inputs which
            // are not represented in the raw sequence arguments' semantics.
            self.refuse_reuse();
        }
    }

    fn add_references(&mut self, value: &RawValue) {
        let mut strings = Vec::new();
        collect_raw_strings(value, &mut strings);
        for field in strings.iter().flat_map(|value| value.split_whitespace()) {
            let field = field.trim_start_matches('!');
            let Some(tag) = field.strip_prefix('$') else {
                continue;
            };
            if let Some(domain_set) = self
                .config
                .domain_sets
                .iter()
                .find(|domain_set| domain_set.tag == tag)
            {
                self.material.insert(format!(
                    "domain-set:{}:{}",
                    domain_set.tag, domain_set.semantic_sha256
                ));
            }
            if let Some(plugin) = self.plugins.get(tag).copied() {
                self.add_plugin(plugin);
            }
        }
    }
}

fn collect_raw_strings(value: &RawValue, output: &mut Vec<String>) {
    match value {
        RawValue::String(value) => output.push(value.clone()),
        RawValue::Sequence(values) => {
            for value in values {
                collect_raw_strings(value, output);
            }
        }
        RawValue::Map(map) => {
            for (_, value) in &map.entries {
                collect_raw_strings(value, output);
            }
        }
        RawValue::Null | RawValue::Bool(_) | RawValue::Number(_) => {}
    }
}

fn sequence_has_file_backed_ip_matcher(args: &RawValue) -> bool {
    let RawValue::Sequence(rules) = args else {
        return false;
    };
    rules.iter().any(|rule| {
        let RawValue::Map(rule) = rule else {
            return false;
        };
        let Some(matches) = rule.get("matches") else {
            return false;
        };
        let mut expressions = Vec::new();
        collect_raw_strings(matches, &mut expressions);
        expressions.iter().any(|expression| {
            let expression = expression.strip_prefix('!').unwrap_or(expression).trim();
            let mut fields = expression.split_whitespace();
            matches!(fields.next(), Some("client_ip" | "resp_ip"))
                && fields.any(|field| field.starts_with('&'))
        })
    })
}

fn cache_config_material(cache: &CachePluginConfig) -> String {
    let kind = match cache.kind {
        CacheKind::Named => "named",
        CacheKind::Quick => "quick",
    };
    format!(
        "cache:{}:{kind}:{}:{}:{}:{:?}:{}:{:?}",
        cache.tag,
        cache.capacity,
        cache.enable_ecs,
        cache.lazy_cache_ttl_secs,
        cache.dump_file,
        cache.dump_interval_secs,
        cache.exclude_ip
    )
}

fn target_name(config: &CompiledConfig, target: NativeTarget) -> String {
    match target {
        NativeTarget::Sequence(sequence) => config
            .program
            .sequence(sequence)
            .map(|sequence| sequence.name.clone())
            .unwrap_or_else(|| "<missing-sequence>".into()),
        NativeTarget::Fixture(executable) => config
            .program
            .fixture(executable)
            .map(|fixture| fixture.name.clone())
            .unwrap_or_else(|| "<missing-fixture>".into()),
        NativeTarget::External(executable) => config
            .program
            .external(executable)
            .map(|external| external.name.clone())
            .unwrap_or_else(|| "<missing-external>".into()),
    }
}

fn reachable_sequence(
    config: &CompiledConfig,
    sequence: SequenceId,
    target: SequenceId,
    visited: &mut BTreeSet<SequenceId>,
) -> bool {
    if sequence == target {
        return true;
    }
    if !visited.insert(sequence) {
        return false;
    }
    let Some(sequence) = config.program.sequence(sequence) else {
        return false;
    };
    for rule in &sequence.rules {
        if let Some(exec) = &rule.executable {
            let found = match *exec {
                ValidatedExecutable::Call {target:child} | ValidatedExecutable::Inline {target:child} | ValidatedExecutable::Jump {target:child} | ValidatedExecutable::Goto {target:child} | ValidatedExecutable::Try {target:ExecutableTarget::Sequence(child)} => reachable_sequence(config,child,target,visited),
                ValidatedExecutable::External {target:id} => config.fallbacks.iter().filter(|p| p.executable==id).any(|p| [p.primary,p.secondary].iter().any(|t| matches!(*t, NativeTarget::Sequence(child) if reachable_sequence(config,child,target,visited)))),
                _ => false,
            };
            if found {
                return true;
            }
            if rule.matchers.is_empty()
                && matches!(*exec, ValidatedExecutable::Call{target:child}|ValidatedExecutable::Inline{target:child} if !sequence_can_return(config,child,&mut BTreeSet::new()))
            {
                break;
            }
            if rule.matchers.is_empty()
                && matches!(
                    exec,
                    ValidatedExecutable::Accept
                        | ValidatedExecutable::Reject { .. }
                        | ValidatedExecutable::Exit
                        | ValidatedExecutable::Return
                        | ValidatedExecutable::Goto { .. }
                )
            {
                break;
            }
        }
    }
    false
}

fn sequence_can_return(
    config: &CompiledConfig,
    sequence: SequenceId,
    visiting: &mut BTreeSet<SequenceId>,
) -> bool {
    if !visiting.insert(sequence) {
        return false;
    }
    let Some(sequence_config) = config.program.sequence(sequence) else {
        return false;
    };
    for rule in &sequence_config.rules {
        if !rule.matchers.is_empty() {
            continue;
        }
        let Some(exec) = &rule.executable else {
            continue;
        };
        match *exec {
            ValidatedExecutable::Exit => {
                visiting.remove(&sequence);
                return false;
            }
            ValidatedExecutable::Accept
            | ValidatedExecutable::Reject { .. }
            | ValidatedExecutable::Return => {
                visiting.remove(&sequence);
                return true;
            }
            ValidatedExecutable::Goto { target } | ValidatedExecutable::Jump { target } => {
                let result = sequence_can_return(config, target, visiting);
                visiting.remove(&sequence);
                return result;
            }
            ValidatedExecutable::Call { target } | ValidatedExecutable::Inline { target }
                if !sequence_can_return(config, target, visiting) =>
            {
                visiting.remove(&sequence);
                return false;
            }
            _ => {}
        }
    }
    visiting.remove(&sequence);
    true
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
    root_dir: &Path,
    definitions: &mut Vec<RawPlugin>,
    inputs: Option<&mut crate::special_groups::CandidateInputSet>,
) -> Result<(), ConfigError> {
    let included = load_included(expression, base_dir, inputs)?;
    for (index, plugin) in included.plugins.iter().enumerate() {
        definitions.push(decode_plugin(
            plugin,
            &format!("{}.plugins[{index}]", included.display),
            root_dir,
        )?);
    }
    Ok(())
}

/// One loaded plugins-only include, before its definitions are decoded.
struct IncludedFile {
    display: String,
    plugins: Vec<RawValue>,
}

fn load_included(
    expression: &str,
    base_dir: &Path,
    inputs: Option<&mut crate::special_groups::CandidateInputSet>,
) -> Result<IncludedFile, ConfigError> {
    let (path, raw) = read_included(expression, base_dir, inputs)?;
    included_plugins(&path, &raw)
}

/// Reads and parses one include: the I/O and YAML half of `load_included`.
fn read_included(
    expression: &str,
    base_dir: &Path,
    inputs: Option<&mut crate::special_groups::CandidateInputSet>,
) -> Result<(PathBuf, RawValue), ConfigError> {
    let path = resolve_relative(expression, base_dir);
    let display = path.display().to_string();
    let loaded = match inputs {
        Some(inputs) => load_yaml_with_inputs(&path, inputs),
        None => load_yaml(&path),
    };
    let yaml = loaded.map_err(|error| {
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
    Ok((path, raw))
}

/// Validates a parsed include's shape: the schema half of `load_included`.
fn included_plugins(path: &Path, raw: &RawValue) -> Result<IncludedFile, ConfigError> {
    let display = path.display().to_string();
    let root = expect_map(raw, &display, "included file must be a mapping")?;
    root.reject_unknown(&["plugins"], &display)?;
    let plugins = expect_sequence(
        root.required("plugins", &display)?,
        &format!("{display}.plugins"),
    )?;
    Ok(IncludedFile {
        plugins: plugins.to_vec(),
        display,
    })
}

pub(crate) fn resolve_relative(value: &str, base_dir: &Path) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() || base_dir.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

pub(crate) fn canonicalized_path(path: &Path) -> Option<PathBuf> {
    let path = path.to_path_buf();
    crate::transaction::blocking_io(move || std::fs::canonicalize(path))
        .ok()?
        .ok()
}

/// The declared kind of one plugin definition. Domain sets and sequences are
/// compiled into the program; forwards and caches become host-fulfilled
/// external executables; listeners are collected separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PluginKind {
    Hosts,
    Ecs,
    Redirect,
    IpSet,
    Forward,
    Cache,
    Sequence,
    DomainSet,
    Listener,
    FastMark,
    FlowSetter,
    Fallback,
    Switch,
    DomainSetLight,
    SdSetLight,
    AdguardRule,
    SiSet,
    DomainMapper,
    TagSetter,
    Sleep,
    Rewrite,
    WebInfo,
    DomainOutput,
    Requery,
    Socks5Proxy,
    EcsAddress,
}

/// The collected definition catalog used to resolve named references. It is
/// built before any definition is compiled, so a reference order-independent.
struct PluginCatalog<'a> {
    policies: &'a RefCell<Vec<(String, ResponsePolicy)>>,
    ip_sets: &'a [IpSetConfig],
    response_ip_rules: &'a RefCell<Vec<ResponseIpRuleConfig>>,
    ip_budgets: &'a RefCell<BTreeMap<String, policy::RuleBudget>>,
    inputs: &'a RefCell<Option<crate::special_groups::CandidateInputSet>>,
    kinds: &'a [(String, PluginKind, usize)],
    domain_sets: &'a [(String, DomainSetHandle)],
    /// `adguard_rule` definitions, which `qname $tag` may reference directly.
    adguard_rules: &'a BTreeMap<String, providers::AdguardRules>,
    /// The direct matcher of each referenced `adguard_rule`, built once and
    /// shared by every reference (a hot reload republishes it).
    adguard_direct:
        &'a RefCell<BTreeMap<String, Rc<crate::rule_sources::Live<providers::Filtered>>>>,
    fast_marks: &'a [(String, FastMarkConfig)],
    forwards: &'a RefCell<ForwardCompiler>,
    preferences: &'a RefCell<Vec<(String, PreferenceFamily)>>,
    /// Every sleep external (named plugin, quick callsite or preset use) and
    /// its wait, keyed by the external's name.
    sleeps: &'a RefCell<Vec<(String, Duration)>>,
    /// Inline `exec: cache` callsites discovered while sequences compile. Each
    /// entry owns a private cache instance keyed by its synthetic external name.
    quick_caches: &'a RefCell<Vec<QuickCacheSpec>>,
}

/// One inline cache callsite: the synthetic external the sequence dispatches
/// and the validated options that instance owns.
struct QuickCacheSpec {
    external: String,
    tag: String,
    args: CompiledCacheArgs,
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

    /// The direct matcher of an `adguard_rule` definition.
    fn adguard_matcher(&self, tag: &str) -> Option<Result<DomainSetHandle, String>> {
        let rules = self.adguard_rules.get(tag)?;
        if let Some(live) = self.adguard_direct.borrow().get(tag) {
            return Some(Ok(DomainSetHandle::Filtered(Rc::clone(live))));
        }
        Some(rules.filtered(tag).map(|filtered| {
            let live = Rc::new(crate::rule_sources::Live::new(filtered));
            self.adguard_direct
                .borrow_mut()
                .insert(tag.to_owned(), Rc::clone(&live));
            DomainSetHandle::Filtered(live)
        }))
    }

    fn fast_mark(&self, tag: &str) -> Option<FastMarkConfig> {
        self.fast_marks
            .iter()
            .find(|(name, _)| name == tag)
            .map(|(_, config)| *config)
    }
}

/// File locations of the compilation inputs, used for switch state-file
/// ownership checks.
struct CompileSources<'a> {
    /// The configuration file itself, when compiling from a file.
    config_origin: Option<&'a Path>,
    /// Every top-level `include` target resolved against the base.
    include_paths: &'a [PathBuf],
    /// The typed settings to overlay on the definitions.
    settings: &'a settings::Settings,
    standard: Option<(&'a Path, Option<&'a [standard_groups::Group]>)>,
}

fn compile_definitions(
    log: LogLevel,
    api: Option<ApiConfig>,
    definitions: Vec<RawPlugin>,
    managed_listeners: bool,
    staged: Option<&crate::special_groups::StagedFiles>,
    inputs: Option<&mut crate::special_groups::CandidateInputSet>,
    sources: &CompileSources<'_>,
) -> Result<CompiledConfig, ConfigError> {
    let candidate_inputs = RefCell::new(inputs.as_deref().cloned());
    let mut kinds: Vec<(String, PluginKind, usize)> = Vec::with_capacity(definitions.len());
    for (index, plugin) in definitions.iter().enumerate() {
        let kind = match plugin.kind.as_str() {
            "hosts" => PluginKind::Hosts,
            "ecs_handler" => PluginKind::Ecs,
            "redirect" => PluginKind::Redirect,
            "ip_set" => PluginKind::IpSet,
            "forward" | "aliapi" => PluginKind::Forward,
            "cache" => PluginKind::Cache,
            "sequence" => PluginKind::Sequence,
            "domain_set" => PluginKind::DomainSet,
            "udp_server" | "tcp_server" => PluginKind::Listener,
            "fast_mark" => PluginKind::FastMark,
            "flow_setter" => PluginKind::FlowSetter,
            "fallback" => PluginKind::Fallback,
            "domain_set_light" => PluginKind::DomainSetLight,
            "sd_set_light" => PluginKind::SdSetLight,
            "adguard_rule" => PluginKind::AdguardRule,
            "si_set" => PluginKind::SiSet,
            "domain_mapper" => PluginKind::DomainMapper,
            "tag_setter" => PluginKind::TagSetter,
            "sleep" => PluginKind::Sleep,
            "rewrite" => PluginKind::Rewrite,
            "webinfo" => PluginKind::WebInfo,
            "domain_output" => PluginKind::DomainOutput,
            "requery" => PluginKind::Requery,
            "socks5_proxy" => PluginKind::Socks5Proxy,
            "ecs_address" => PluginKind::EcsAddress,
            other if crate::switch::switch_type_number(other).is_some() => PluginKind::Switch,
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
            let mut inputs = candidate_inputs.borrow_mut();
            compiled_domain_sets.push(compile_domain_set(
                plugin,
                managed_listeners,
                staged,
                inputs.as_mut(),
            )?);
        }
    }
    let mut file_owners: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    // Every persistent file the compiled configuration reads or writes that
    // is visible to the compiler. Switch state files are durable POST
    // targets and may never alias any of these artifacts.
    let mut known_artifacts: Vec<PathBuf> = Vec::new();
    if let Some(origin) = sources.config_origin {
        known_artifacts.push(origin.to_path_buf());
    }
    known_artifacts.extend(sources.include_paths.iter().cloned());
    for set in &compiled_domain_sets {
        for reference in &set.references {
            file_owners
                .entry(reference.clone())
                .or_default()
                .insert(set.tag.clone());
            known_artifacts.push(reference.clone());
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
            semantic_sha256: set.semantic_sha256,
            managed,
            ineligible_reason,
            handle,
        });
    }
    let policies = RefCell::new(Vec::new());
    let mut ip_sets = Vec::new();
    let response_ip_rules = RefCell::new(Vec::new());
    let ip_budgets = RefCell::new(BTreeMap::new());
    let mut forwards = Vec::new();
    let mut switches: Vec<crate::switch::SwitchDeclaration> = Vec::new();
    let mut fast_marks = Vec::new();
    let mut flow_setters = Vec::new();
    let mut tag_setters = Vec::new();
    let named_sleeps = RefCell::new(Vec::new());
    let mut rewrites: Vec<(String, Rc<Rewrite>)> = Vec::new();
    let mut webinfos: Vec<Rc<WebInfo>> = Vec::new();
    let mut pending_outputs: Vec<PendingOutput> = Vec::new();
    let mut requeries: Vec<RequeryConfig> = Vec::new();
    let mut fallback_definitions = Vec::new();
    let mut upstream_identities: BTreeMap<String, String> = BTreeMap::new();
    let mut caches: Vec<(String, CompiledCacheArgs, String)> = Vec::new();
    let mut listeners = Vec::new();
    let mut provider_rules: BTreeMap<String, providers::ProviderRules> = BTreeMap::new();
    let mut adguard_rules: BTreeMap<String, providers::AdguardRules> = BTreeMap::new();
    let mut rule_providers: Vec<Rc<crate::rule_sources::Provider>> = Vec::new();
    for (index, plugin) in definitions.iter().enumerate() {
        match kinds[index].1 {
            PluginKind::Ecs => {
                policies.borrow_mut().push((
                    plugin.tag.clone(),
                    ResponsePolicy::Ecs(compile_ecs(plugin)?),
                ));
            }
            PluginKind::Hosts | PluginKind::Redirect => {
                let (inline, files) = policy_sources(
                    plugin,
                    if kinds[index].1 == PluginKind::Hosts {
                        "entries"
                    } else {
                        "rules"
                    },
                    &["entries", "files"],
                    &["rules", "files"],
                )?;
                known_artifacts.extend(files.iter().cloned());
                let path = format!("{}.args", plugin.source_path);
                let policy = if kinds[index].1 == PluginKind::Hosts {
                    let mut candidate_inputs = candidate_inputs.borrow_mut();
                    ResponsePolicy::Hosts(Rc::new(policy::hosts_with_inputs(
                        &inline,
                        &files,
                        &path,
                        candidate_inputs.as_mut(),
                    )?))
                } else {
                    let mut candidate_inputs = candidate_inputs.borrow_mut();
                    ResponsePolicy::Redirect(Rc::new(policy::redirects_with_inputs(
                        &inline,
                        &files,
                        &path,
                        candidate_inputs.as_mut(),
                    )?))
                };
                policies.borrow_mut().push((plugin.tag.clone(), policy));
            }
            PluginKind::IpSet => {
                let args = expect_map(
                    &plugin.args,
                    &format!("{}.args", plugin.source_path),
                    "ip_set args must be a mapping",
                )?;
                args.reject_unknown(&["ips", "files", "sets"], &plugin.source_path)?;
                if !policy_strings(
                    args.get("sets"),
                    &format!("{}.args.sets", plugin.source_path),
                )?
                .is_empty()
                {
                    return Err(ConfigError::new(
                        &plugin.source_path,
                        "non-empty ip_set sets are unsupported",
                    ));
                }
                let (inline, files) =
                    policy_sources(plugin, "ips", &["ips", "files", "sets"], &[])?;
                known_artifacts.extend(files.iter().cloned());
                let mut candidate_inputs = candidate_inputs.borrow_mut();
                ip_sets.push(IpSetConfig {
                    tag: plugin.tag.clone(),
                    prefixes: policy::ip_list_with_inputs(
                        &inline,
                        &files,
                        &format!("{}.args", plugin.source_path),
                        candidate_inputs.as_mut(),
                    )?,
                });
            }
            PluginKind::DomainSet | PluginKind::Sequence => {}
            PluginKind::DomainSetLight | PluginKind::SdSetLight | PluginKind::AdguardRule => {
                use crate::rule_sources::Provider;
                let mut inputs = candidate_inputs.borrow_mut();
                let entries = match kinds[index].1 {
                    PluginKind::DomainSetLight => {
                        let spec = providers::light_spec(plugin)?;
                        let rules = providers::load_light(&spec, inputs.as_mut())?;
                        rule_providers.push(Rc::new(Provider::light(spec)));
                        Rc::new(rules)
                    }
                    PluginKind::SdSetLight => {
                        let spec = providers::manifest_spec(plugin, providers::Schema::Sd)?;
                        let manifest = providers::read_manifest(&spec, inputs.as_mut())?;
                        let (rules, counts) =
                            providers::load_sd(&spec, &manifest.entries, inputs.as_mut());
                        rule_providers
                            .push(Rc::new(Provider::manifest(spec, manifest, counts, None)));
                        Rc::new(rules)
                    }
                    _ => {
                        let spec = providers::manifest_spec(plugin, providers::Schema::Adguard)?;
                        let manifest = providers::read_manifest(&spec, inputs.as_mut())?;
                        let (rules, counts) = providers::load_adguard(
                            &spec,
                            &manifest.entries,
                            false,
                            inputs.as_mut(),
                        );
                        let deny = Rc::clone(&rules.deny);
                        adguard_rules.insert(plugin.tag.clone(), rules);
                        rule_providers
                            .push(Rc::new(Provider::manifest(spec, manifest, counts, None)));
                        deny
                    }
                };
                provider_rules.insert(plugin.tag.clone(), entries);
            }
            PluginKind::SiSet => {
                let mut inputs = candidate_inputs.borrow_mut();
                let spec = providers::manifest_spec(plugin, providers::Schema::Si)?;
                let manifest = providers::read_manifest(&spec, inputs.as_mut())?;
                let (list, counts) = providers::load_si(&spec, &manifest.entries, inputs.as_mut());
                let handle = Rc::new(crate::policy::IpListHandle::new(Rc::new(list)));
                ip_sets.push(IpSetConfig {
                    tag: plugin.tag.clone(),
                    prefixes: Rc::clone(&handle),
                });
                rule_providers.push(Rc::new(crate::rule_sources::Provider::manifest(
                    spec,
                    manifest,
                    counts,
                    Some(handle),
                )));
            }
            PluginKind::DomainMapper => {}
            PluginKind::Switch => {
                let declaration = compile_switch_declaration(plugin)?;
                if let Some(existing) = switches
                    .iter()
                    .find(|existing| existing.type_number == declaration.type_number)
                {
                    return Err(ConfigError::new(
                        format!("{}.type", plugin.source_path),
                        format!(
                            "duplicate switch{} declaration `{}`; at most one instance of each                              switch type is supported (already declared as `{}`)",
                            declaration.type_number, declaration.tag, existing.tag
                        ),
                    ));
                }
                switches.push(declaration);
            }
            PluginKind::FastMark => {
                let config = compile_fast_mark(plugin)?;
                fast_marks.push((plugin.tag.clone(), config));
            }
            PluginKind::FlowSetter => {
                let config = compile_flow_setter(plugin)?;
                flow_setters.push((plugin.tag.clone(), config));
            }
            PluginKind::TagSetter => {
                tag_setters.push((plugin.tag.clone(), compile_tag_setter(plugin)?));
            }
            PluginKind::Sleep => {
                named_sleeps
                    .borrow_mut()
                    .push((plugin.tag.clone(), compile_sleep(plugin)?));
            }
            PluginKind::Rewrite => {
                let (rewrite, files) = compile_rewrite(plugin)?;
                known_artifacts.extend(files);
                rewrites.push((plugin.tag.clone(), Rc::new(rewrite)));
            }
            // Validated and resolved by the settings overlay before compiling.
            PluginKind::Socks5Proxy | PluginKind::EcsAddress => {}
            PluginKind::Requery => {
                let (requery, file) = compile_requery(plugin)?;
                known_artifacts.push(file);
                requeries.push(RequeryConfig {
                    tag: plugin.tag.clone(),
                    runtime: Rc::new(requery),
                });
            }
            PluginKind::DomainOutput => {
                let pending = compile_domain_output(plugin)?;
                known_artifacts.extend(pending.settings.files());
                pending_outputs.push(pending);
            }
            PluginKind::WebInfo => {
                let (webinfo, file) = compile_webinfo(plugin)?;
                known_artifacts.push(file);
                webinfos.push(Rc::new(webinfo));
            }
            PluginKind::Fallback => {
                fallback_definitions.push(compile_fallback(plugin)?);
            }
            PluginKind::Forward => {
                let forward = if plugin.kind == "aliapi" {
                    aliapi::compile(plugin)?
                } else {
                    compile_forward(plugin)?
                };
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
                let compiled = compile_cache(plugin)?;
                if let Some(dump_file) = &compiled.dump_file {
                    known_artifacts.push(dump_file.clone());
                }
                caches.push((plugin.tag.clone(), compiled, plugin.source_path.clone()));
            }
            PluginKind::Listener => {
                let candidate = compile_listener(plugin)?;
                if listeners
                    .iter()
                    .any(|(existing, _): &(ListenerConfig, String)| {
                        existing.kind == candidate.kind && existing.listen == candidate.listen
                    })
                {
                    return Err(ConfigError::new(
                        format!("{}.tag", plugin.source_path),
                        "duplicate listener protocol/address",
                    ));
                }
                listeners.push((candidate, plugin.source_path.clone()));
            }
        }
    }
    if let Some(staged) = staged {
        known_artifacts.extend(staged.keys().cloned());
    }
    if let Some(inputs) = candidate_inputs.borrow().as_ref() {
        known_artifacts.extend(inputs.files.keys().cloned());
    }
    // A switch state file is a durable POST target. Rejecting every alias
    // here keeps one owner's write from rewriting another switch's state or
    // a known configuration, rule, or cache-dump artifact.
    for (index, declaration) in switches.iter().enumerate() {
        for other in switches.iter().skip(index + 1) {
            if crate::switch::paths_collide(&declaration.state_file, &other.state_file) {
                return Err(ConfigError::new(
                    format!("{}.args.initial_value", other.source_path),
                    format!(
                        "switch state file `{}` collides with switch `{}`",
                        other.state_file.display(),
                        declaration.tag
                    ),
                ));
            }
        }
        for artifact in &known_artifacts {
            if crate::switch::paths_collide(&declaration.state_file, artifact) {
                return Err(ConfigError::new(
                    format!("{}.args.initial_value", declaration.source_path),
                    format!(
                        "switch state file `{}` collides with known artifact `{}`",
                        declaration.state_file.display(),
                        artifact.display()
                    ),
                ));
            }
        }
    }
    switches.sort_by_key(|declaration| declaration.type_number);

    // Mappers compile after every provider is loaded, so a rule may name a
    // provider declared later without changing the result.
    let mut domain_mappers: Vec<DomainMapperConfig> = Vec::new();
    let mut mapper_slots: Vec<Rc<crate::rule_sources::MapperSlot>> = Vec::new();
    {
        let provider_kinds: Vec<(String, String)> = definitions
            .iter()
            .map(|plugin| (plugin.tag.clone(), plugin.kind.clone()))
            .collect();
        let kind_name = |tag: &str| {
            provider_kinds
                .iter()
                .find(|(name, _)| name == tag)
                .map(|(_, kind)| kind.clone())
        };
        for (index, plugin) in definitions.iter().enumerate() {
            if kinds[index].1 == PluginKind::DomainMapper {
                let spec = providers::mapper_spec(plugin, &kind_name, &|tag| {
                    provider_rules.contains_key(tag)
                })?;
                let matcher = providers::build_mapper_matcher(&spec, &|tag| {
                    provider_rules.get(tag).map(|rules| rules.as_slice())
                })?;
                let mapper = spec.config(matcher);
                mapper_slots.push(Rc::new(crate::rule_sources::MapperSlot::new(
                    spec,
                    Rc::clone(&mapper.matcher),
                )));
                domain_mappers.push(mapper);
            }
        }
    }
    drop(provider_rules);
    let mut domain_outputs: Vec<(String, Rc<DomainOutput>)> = Vec::new();
    for pending in pending_outputs {
        let target = match pending.target {
            None => None,
            Some((mapper, mark, tag)) => {
                let layer = domain_mappers
                    .iter()
                    .find(|candidate| candidate.tag == mapper)
                    .map(|candidate| candidate.hot.layer(mark, &tag))
                    .ok_or_else(|| {
                        ConfigError::new(
                            format!("{}.args.target_mapper", pending.path),
                            format!("`{mapper}` is not a domain_mapper"),
                        )
                    })?;
                Some(crate::domain_output::MapperTarget { layer })
            }
        };
        domain_outputs.push((
            pending.tag.clone(),
            Rc::new(DomainOutput::new(&pending.tag, pending.settings, target)),
        ));
    }

    let forward_compiler = RefCell::new(ForwardCompiler {
        definitions: forwards,
        invocations: Vec::new(),
    });
    let preference_invocations = RefCell::new(Vec::new());
    let quick_caches = RefCell::new(Vec::new());
    let adguard_direct = RefCell::new(BTreeMap::new());
    let catalog = PluginCatalog {
        policies: &policies,
        ip_sets: &ip_sets,
        response_ip_rules: &response_ip_rules,
        ip_budgets: &ip_budgets,
        inputs: &candidate_inputs,
        kinds: &kinds,
        domain_sets: &domain_sets,
        adguard_rules: &adguard_rules,
        adguard_direct: &adguard_direct,
        fast_marks: &fast_marks,
        forwards: &forward_compiler,
        preferences: &preference_invocations,
        sleeps: &named_sleeps,
        quick_caches: &quick_caches,
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
    for (tag, config) in &tag_setters {
        fixtures.push(mosdns_sequence_core::FixtureSpec::new(
            plugin_fixture_name(tag),
            config.executor(),
        ));
    }
    for declaration in &switches {
        fixtures.push(mosdns_sequence_core::FixtureSpec::new(
            plugin_fixture_name(&declaration.tag),
            Box::new(crate::switch::SwitchNoopExecutor),
        ));
    }
    for mapper in &domain_mappers {
        fixtures.push(mosdns_sequence_core::FixtureSpec::new(
            plugin_fixture_name(&mapper.tag),
            mapper.executor(),
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
    let (listener, listener_source_path) = listeners
        .first()
        .cloned()
        .ok_or_else(|| ConfigError::new("$.plugins", "missing listener plugin"))?;
    for (listener, listener_source_path) in &listeners {
        if !kinds
            .iter()
            .any(|(tag, kind, _)| *kind == PluginKind::Sequence && tag.as_str() == listener.entry)
        {
            let path = format!("{listener_source_path}.args.entry");
            let reason = format!("unknown sequence reference `{}`", listener.entry);
            return Err(
                if kinds
                    .iter()
                    .any(|(tag, _, _)| tag.as_str() == listener.entry)
                {
                    ConfigError::new(path, reason)
                } else {
                    ConfigError::unknown_reference(path, reason, &listener.entry)
                },
            );
        }
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
    let sleep_specs = named_sleeps.into_inner();
    let quick_cache_specs = quick_caches.into_inner();
    let policies = policies.into_inner();
    let externals: Vec<ExternalSpec> = forward_compiler
        .invocations
        .iter()
        .map(|(name, _, _)| ExternalSpec::new(name.clone()))
        .chain(
            policies
                .iter()
                .map(|(name, _)| ExternalSpec::new(name.clone())),
        )
        .chain(
            caches
                .iter()
                .map(|(tag, _, _)| ExternalSpec::new(tag.clone())),
        )
        .chain(
            quick_cache_specs
                .iter()
                .map(|spec| ExternalSpec::new(spec.external.clone())),
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
        .chain(
            sleep_specs
                .iter()
                .map(|(name, _)| ExternalSpec::new(name.clone())),
        )
        .chain(
            rewrites
                .iter()
                .map(|(tag, _)| ExternalSpec::new(tag.clone())),
        )
        .chain(
            domain_outputs
                .iter()
                .map(|(tag, _)| ExternalSpec::new(tag.clone())),
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
    // Named caches keep their declaration order; quick callsites follow in
    // compile order. `CacheId(i)` is a stable index into this vector, so the
    // order must be deterministic and identical for every compile.
    let mut compiled_caches: Vec<CachePluginConfig> = Vec::new();
    for (tag, args, path) in caches {
        let executable = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == tag).then_some(*id))
            .ok_or_else(|| ConfigError::new(path, "compiled cache external is missing"))?;
        compiled_caches.push(CachePluginConfig {
            id: CacheId(compiled_caches.len()),
            tag,
            executable,
            kind: CacheKind::Named,
            capacity: args.capacity,
            enable_ecs: args.enable_ecs,
            lazy_cache_ttl_secs: args.lazy_cache_ttl_secs,
            dump_file: args.dump_file,
            dump_interval_secs: args.dump_interval_secs,
            exclude_ip: args.exclude_ip,
        });
    }
    let mut dump_targets = std::collections::BTreeSet::new();
    for cache in &compiled_caches {
        if let Some(path) = &cache.dump_file {
            if !dump_targets.insert(path.clone()) {
                return Err(ConfigError::new(
                    "$.plugins[cache].args.dump_file",
                    "duplicate resolved dump_file target",
                ));
            }
        }
    }
    for spec in quick_cache_specs {
        let executable = program
            .externals
            .iter()
            .find_map(|(id, external)| (external.name == spec.external).then_some(*id))
            .ok_or_else(|| {
                ConfigError::new(
                    "$.plugins[sequence].args",
                    "compiled quick cache is missing",
                )
            })?;
        compiled_caches.push(CachePluginConfig {
            id: CacheId(compiled_caches.len()),
            tag: spec.tag,
            executable,
            kind: CacheKind::Quick,
            capacity: spec.args.capacity,
            enable_ecs: false,
            lazy_cache_ttl_secs: spec.args.lazy_cache_ttl_secs,
            dump_file: spec.args.dump_file,
            dump_interval_secs: spec.args.dump_interval_secs,
            exclude_ip: spec.args.exclude_ip,
        });
    }

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

    let sleeps = sleep_specs
        .into_iter()
        .map(|(tag, duration)| {
            Ok(SleepConfig {
                executable: external_id(&program, &tag, "$.plugins.sequence.args")?,
                tag,
                duration,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;

    let rewrites = rewrites
        .into_iter()
        .map(|(tag, runtime)| {
            Ok(RewriteConfig {
                executable: external_id(&program, &tag, "$.plugins")?,
                tag,
                runtime,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;

    let domain_outputs = domain_outputs
        .into_iter()
        .map(|(tag, runtime)| {
            Ok(DomainOutputConfig {
                executable: external_id(&program, &tag, "$.plugins")?,
                tag,
                runtime,
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

    let response_policies = policies
        .into_iter()
        .map(|(tag, policy)| {
            Ok(ResponsePolicyConfig {
                executable: external_id(&program, &tag, "$.plugins")?,
                tag,
                policy,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    let mut response_identities = std::collections::HashMap::new();
    for invocation in &forward_invocations {
        for index in &invocation.entries {
            response_identities.insert(
                (invocation.executable.0, *index),
                forward_compiler.definitions[invocation.definition].entries[*index]
                    .response_identity
                    .clone(),
            );
        }
    }
    for (tag, live) in adguard_direct.borrow().iter() {
        if let Some(provider) = rule_providers.iter().find(|p| p.tag == *tag) {
            provider.attach_filtered(Rc::clone(live));
        }
    }
    let compiled = CompiledConfig {
        ui_base: None,
        settings: settings::Settings::default(),
        settings_report: Vec::new(),
        effective_upstreams: BTreeMap::new(),
        yaml_defaults: settings::YamlDefaults::default(),
        reload: None,
        standard_groups: None,
        compile_inputs: crate::special_groups::CandidateInputSet::default(),
        generation: 0,
        cache_dependencies: Vec::new(),
        response_identities,
        managed_profile: None,
        managed_router: None,
        listeners: listeners
            .into_iter()
            .map(|(listener, _)| listener)
            .collect(),
        response_policies,
        ip_sets,
        domain_mappers,
        response_ip_rules: response_ip_rules.into_inner(),
        log_level: log,
        forward: primary.clone(),
        forwards: compiled_forwards,
        forward_definitions: forward_compiler.definitions,
        forward_invocations,
        caches: compiled_caches,
        fallbacks,
        preferences,
        sleeps,
        local: LocalPlugins {
            rewrites,
            webinfos,
            domain_outputs,
            requeries,
        },
        sequence: SequenceConfig {
            tag: listener.entry.clone(),
            sequence: entry_sequence,
            forward_executable: primary.as_ref().map(|forward| forward.executable),
        },
        listener,
        domain_sets: domain_set_configs,
        switches,
        api,
        program,
        rule_sources: Rc::new(crate::rule_sources::RuleSources::new(
            rule_providers,
            mapper_slots,
        )),
    };
    let mut client_rules = BTreeSet::new();
    for plugin in &definitions {
        if plugin.kind == "sequence" {
            for (index, rule) in expect_sequence(&plugin.args, &plugin.source_path)?
                .iter()
                .enumerate()
            {
                let rule = expect_map(rule, &plugin.source_path, "rule")?;
                if let Some(matches) = rule.get("matches") {
                    if match_expressions(matches, &plugin.source_path)?
                        .iter()
                        .any(|value| {
                            value
                                .trim_start()
                                .trim_start_matches('!')
                                .split_whitespace()
                                .next()
                                == Some("client_ip")
                        })
                    {
                        client_rules.insert(format!("inline:{}#{index}", plugin.tag));
                    }
                }
            }
        }
    }
    crate::cache_placement::validate(&compiled, &client_rules)?;
    if let (Some(destination), Some(collected)) = (inputs, candidate_inputs.into_inner()) {
        *destination = collected;
    }
    Ok(compiled)
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

fn compile_log(value: &RawValue, root_dir: &Path) -> Result<LogSettings, ConfigError> {
    let map = expect_map(value, "$.log", "log must be a mapping")?;
    map.reject_unknown(&["level", "file"], "$.log")?;
    let level = expect_string(map.required("level", "$.log")?, "$.log.level")?;
    let (level, unknown_level) = match level.as_str() {
        "debug" => (LogLevel::Debug, None),
        "info" => (LogLevel::Info, None),
        "warn" => (LogLevel::Warn, None),
        "error" => (LogLevel::Error, None),
        _ => (LogLevel::Info, Some(level)),
    };
    let file = match map.get("file") {
        None => None,
        Some(value) => {
            let file = expect_string(value, "$.log.file")?;
            (!file.is_empty()).then(|| resolve_relative(&file, root_dir))
        }
    };
    Ok(LogSettings {
        level,
        file,
        unknown_level,
    })
}

/// Reads only the `log` section so `start` can route diagnostics before the
/// full compilation emits any.
pub(crate) fn log_settings(yaml: &str, root_dir: &Path) -> Result<LogSettings, ConfigError> {
    let raw = parse_yaml(yaml)?;
    let root = expect_map(&raw, "$", "top level must be a mapping")?;
    compile_log(root.required("log", "$")?, root_dir)
}

fn decode_plugin(value: &RawValue, path: &str, root_dir: &Path) -> Result<RawPlugin, ConfigError> {
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
    let args = if matches!(
        kind.as_str(),
        "hosts" | "redirect" | "ip_set" | "ecs_handler"
    ) && matches!(map.get("args"), None | Some(RawValue::Null))
    {
        RawValue::Map(RawMap {
            entries: Vec::new(),
        })
    } else {
        map.required("args", path)?.clone()
    };
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
        root_dir: root_dir.to_path_buf(),
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
            response_identity: std::sync::Arc::from(identity.as_str()),
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

/// Compiles one `switchN` declaration. The state file is resolved but never
/// read, created, or validated for readability: compilation stays
/// side-effect-free and startup owns the bounded file admission.
fn compile_switch_declaration(
    plugin: &RawPlugin,
) -> Result<crate::switch::SwitchDeclaration, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "switch args must be a mapping")?;
    args.reject_unknown(&["initial_value"], &path)?;
    let declared = expect_string(
        args.required("initial_value", &path)?,
        &format!("{path}.initial_value"),
    )?;
    if declared.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.initial_value"),
            "switch initial_value must name a state file",
        ));
    }
    let type_number = crate::switch::switch_type_number(&plugin.kind)
        .expect("switch kind is validated before compilation");
    let state_file = crate::switch::resolved_state_file(&declared, &plugin.root_dir, &path)?;
    Ok(crate::switch::SwitchDeclaration {
        type_number,
        tag: plugin.tag.clone(),
        state_file,
        source_path: plugin.source_path.clone(),
    })
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

fn compile_tag_setter(plugin: &RawPlugin) -> Result<TagSetterConfig, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    if matches!(plugin.args, RawValue::Null) {
        return Ok(TagSetterConfig::new(""));
    }
    let args = expect_map(&plugin.args, &path, "tag_setter args must be a mapping")?;
    args.reject_unknown(&["tag"], &path)?;
    let tag = match args.get("tag") {
        None | Some(RawValue::Null) => String::new(),
        Some(value) => expect_string(value, &format!("{path}.tag"))?,
    };
    Ok(TagSetterConfig::new(&tag))
}

fn compile_sleep(plugin: &RawPlugin) -> Result<Duration, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    if matches!(plugin.args, RawValue::Null) {
        return Ok(Duration::ZERO);
    }
    let args = expect_map(&plugin.args, &path, "sleep args must be a mapping")?;
    args.reject_unknown(&["duration"], &path)?;
    match args.get("duration") {
        None | Some(RawValue::Null) => Ok(Duration::ZERO),
        Some(value) => Ok(Duration::from_millis(expect_nonnegative_integer(
            value,
            &format!("{path}.duration"),
        )?)),
    }
}

/// `rewrite`: the rule files (the first one is where `/post` writes) and the
/// DNS server a domain target resolves through. A missing file is allowed.
fn compile_rewrite(plugin: &RawPlugin) -> Result<(Rewrite, Vec<PathBuf>), ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "rewrite args must be a mapping")?;
    args.reject_unknown(&["files", "dns"], &path)?;
    let files: Vec<PathBuf> = policy_strings(args.get("files"), &format!("{path}.files"))?
        .into_iter()
        .map(|file| resolve_rule_path(&file, &plugin.root_dir))
        .collect();
    if files.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.files"),
            "at least one rule file must be specified in `files`",
        ));
    }
    let dns_text = match args.get("dns") {
        None | Some(RawValue::Null) => String::new(),
        Some(value) => expect_string(value, &format!("{path}.dns"))?,
    };
    if dns_text.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.dns"),
            "upstream dns server address cannot be empty",
        ));
    }
    let dns = DnsServer::parse(&dns_text)
        .map_err(|reason| ConfigError::new(format!("{path}.dns"), reason))?;
    let rewrite = Rewrite::load(files.clone(), dns)
        .map_err(|reason| ConfigError::new(format!("{path}.files"), reason))?;
    Ok((rewrite, files))
}

/// A validated `domain_output` waiting for its `target_mapper` to compile.
struct PendingOutput {
    tag: String,
    path: String,
    settings: crate::domain_output::Settings,
    /// `target_mapper`, `target_mark`, `target_tag`.
    target: Option<(String, u8, String)>,
}

/// `domain_output`: counters, generated files, rule publication and mapper
/// injection. Every argument is optional, as in the reference.
fn compile_domain_output(plugin: &RawPlugin) -> Result<PendingOutput, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let empty = RawMap {
        entries: Vec::new(),
    };
    let args = match &plugin.args {
        RawValue::Null => &empty,
        other => expect_map(other, &path, "domain_output args must be a mapping")?,
    };
    args.reject_unknown(
        &[
            "file_stat",
            "file_rule",
            "gen_rule",
            "pattern",
            "appended_string",
            "max_entries",
            "dump_interval",
            "domain_set_url",
            "enable_flags",
            "enabled",
            "target_mapper",
            "target_mark",
            "target_tag",
        ],
        &path,
    )?;
    let text = |key: &str| -> Result<String, ConfigError> {
        match args.get(key) {
            None | Some(RawValue::Null) => Ok(String::new()),
            Some(value) => expect_string(value, &format!("{path}.{key}")),
        }
    };
    let file = |key: &str| -> Result<Option<PathBuf>, ConfigError> {
        let name = text(key)?;
        Ok((!name.is_empty()).then(|| resolve_rule_path(&name, &plugin.root_dir)))
    };
    let integer = |key: &str| -> Result<i64, ConfigError> {
        match args.get(key) {
            None | Some(RawValue::Null) => Ok(0),
            Some(value) => expect_integer(value, &format!("{path}.{key}")),
        }
    };
    let dump_interval = match integer("dump_interval")? {
        seconds if seconds > 0 => Duration::from_secs(seconds.unsigned_abs()),
        _ => Duration::from_secs(60),
    };
    let enable_flags = match args.get("enable_flags") {
        None | Some(RawValue::Null) => false,
        Some(value) => expect_bool(value, &format!("{path}.enable_flags"))?,
    };
    let mapper = text("target_mapper")?;
    let mark = integer("target_mark")?;
    let mark = u8::try_from(mark)
        .ok()
        .filter(|mark| *mark < 64)
        .ok_or_else(|| {
            ConfigError::new(
                format!("{path}.target_mark"),
                "target_mark must be between 0 and 63",
            )
        })?;
    let target_tag = text("target_tag")?;
    let target = (!mapper.is_empty()).then_some((mapper, mark, target_tag));
    Ok(PendingOutput {
        tag: plugin.tag.clone(),
        path: plugin.source_path.clone(),
        settings: crate::domain_output::Settings {
            file_stat: file("file_stat")?,
            file_rule: file("file_rule")?,
            gen_rule: file("gen_rule")?,
            pattern: text("pattern")?,
            appended_string: text("appended_string")?,
            max_entries: integer("max_entries")?,
            dump_interval,
            domain_set_url: text("domain_set_url")?,
            enable_flags,
            enabled: match args.get("enabled") {
                None | Some(RawValue::Null) => true,
                Some(value) => expect_bool(value, &format!("{path}.enabled"))?,
            },
            max_keys: crate::domain_output::MAX_KEYS,
        },
        target,
    })
}

/// `requery`: the JSON state file. A present file must parse; nothing is
/// written while compiling.
fn compile_requery(plugin: &RawPlugin) -> Result<(Requery, PathBuf), ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "requery args must be a mapping")?;
    args.reject_unknown(&["file"], &path)?;
    let file = match args.get("file") {
        None | Some(RawValue::Null) => String::new(),
        Some(value) => expect_string(value, &format!("{path}.file"))?,
    };
    if file.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.file"),
            "'file' for config json must be specified",
        ));
    }
    let file = resolve_rule_path(&file, &plugin.root_dir);
    let requery = Requery::load(&plugin.tag, file.clone(), plugin.root_dir.clone())
        .map_err(|reason| ConfigError::new(format!("{path}.file"), reason))?;
    Ok((requery, file))
}

/// `webinfo`: the JSON document file.
fn compile_webinfo(plugin: &RawPlugin) -> Result<(WebInfo, PathBuf), ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "webinfo args must be a mapping")?;
    args.reject_unknown(&["file"], &path)?;
    let file = match args.get("file") {
        None | Some(RawValue::Null) => String::new(),
        Some(value) => expect_string(value, &format!("{path}.file"))?,
    };
    if file.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.file"),
            "'file' must be specified",
        ));
    }
    let file = resolve_rule_path(&file, &plugin.root_dir);
    let webinfo = WebInfo::load(&plugin.tag, file.clone())
        .map_err(|reason| ConfigError::new(format!("{path}.file"), reason))?;
    Ok((webinfo, file))
}

/// Registers one quick `sleep` callsite (or one use of the `_sleep_500ms`
/// preset) as its own external and dispatches it.
fn quick_sleep(
    catalog: &PluginCatalog<'_>,
    sequence_tag: &str,
    rule_index: usize,
    exec_index: usize,
    duration: Duration,
) -> ExecutableSpec {
    let name = format!(
        "@native-sleep:{}:{rule_index}:{exec_index}",
        hex_identity(sequence_tag)
    );
    catalog.sleeps.borrow_mut().push((name.clone(), duration));
    ExecutableSpec::External {
        target: ExternalRef::new(name),
    }
}

/// The preset plugin the reference product registers for `$_sleep_500ms`.
const PRESET_SLEEP_500MS: &str = "_sleep_500ms";

fn compile_fallback(plugin: &RawPlugin) -> Result<FallbackDefinition, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "fallback args must be a mapping")?;
    args.reject_unknown(
        &["primary", "secondary", "threshold", "always_standby"],
        &path,
    )?;
    let target = |key: &str| -> Result<String, ConfigError> {
        let value = expect_string(args.required(key, &path)?, &format!("{path}.{key}"))?;
        // The reference product names the plugin bare; `$plugin` is accepted too.
        let tag = value.strip_prefix('$').unwrap_or(&value);
        if tag.is_empty() || tag.contains(char::is_whitespace) {
            return Err(ConfigError::new(
                format!("{path}.{key}"),
                "fallback targets must be exactly one plugin tag",
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
    if Instant::now().checked_add(threshold).is_none() {
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
        Some(
            PluginKind::Cache
            | PluginKind::Fallback
            | PluginKind::Hosts
            | PluginKind::Redirect
            | PluginKind::Ecs
            | PluginKind::Sleep
            | PluginKind::Rewrite
            | PluginKind::DomainOutput,
        ) => Ok(PolicyTargetName::External(tag.to_owned())),
        Some(
            PluginKind::FastMark
            | PluginKind::FlowSetter
            | PluginKind::TagSetter
            | PluginKind::Switch
            | PluginKind::DomainMapper,
        ) => Ok(PolicyTargetName::Fixture(plugin_fixture_name(tag))),
        Some(
            PluginKind::DomainSet
            | PluginKind::Listener
            | PluginKind::IpSet
            | PluginKind::SiSet
            | PluginKind::DomainSetLight
            | PluginKind::SdSetLight
            | PluginKind::AdguardRule
            | PluginKind::WebInfo
            | PluginKind::Requery
            | PluginKind::Socks5Proxy
            | PluginKind::EcsAddress,
        ) => Err(ConfigError::new(
            path,
            format!("`${tag}` is not executable"),
        )),
        None => Err(ConfigError::unknown_reference(
            path,
            format!("unknown executable reference `${tag}`"),
            tag,
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
    semantic_sha256: String,
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

fn compile_domain_set(
    plugin: &RawPlugin,
    immutable: bool,
    staged: Option<&crate::special_groups::StagedFiles>,
    mut inputs: Option<&mut crate::special_groups::CandidateInputSet>,
) -> Result<CompiledDomainSet, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "domain_set args must be a mapping")?;
    args.reject_unknown(&["exps", "files"], &path)?;
    let expressions = string_list(args.get("exps"), &format!("{path}.exps"))?;
    let files = string_list(args.get("files"), &format!("{path}.files"))?;
    if expressions.is_empty()
        && files.is_empty()
        && !plugin.source_path.starts_with("@managed-generated.")
    {
        return Err(ConfigError::new(
            &path,
            "a domain_set requires at least one expression or file",
        ));
    }
    // Managed routing cannot silently drop a malformed enabled rule file.
    // Preserve the legacy tolerant loader for ordinary unmanaged providers.
    let strict = plugin.source_path.starts_with("@managed-generated.");
    let mut strict_expressions = expressions.clone();
    let mut remaining_files = Vec::new();
    for file in &files {
        let path = resolve_rule_path(file, &plugin.root_dir);
        let candidate =
            staged.and_then(|files| files.get(&crate::special_groups::lexical_path(&path)));
        if !strict && candidate.is_none() {
            remaining_files.push(file.clone());
            continue;
        }
        let bytes = match candidate {
            Some(Some(bytes)) => bytes.clone(),
            Some(None) => {
                return Err(ConfigError::new(
                    path.display().to_string(),
                    "enabled rule file is deleted in candidate",
                ));
            }
            None => {
                let display = path.display().to_string();
                let file_path = path.clone();
                let worker_display = display.clone();
                let read_display = display.clone();
                let bytes = crate::transaction::blocking_io(move || {
                    let metadata = std::fs::metadata(&file_path).map_err(|error| {
                        ConfigError::new(worker_display.clone(), error.to_string())
                    })?;
                    if metadata.len() > crate::policy::POLICY_BYTES_LIMIT as u64 {
                        return Err(ConfigError::new(
                            worker_display,
                            "rule file exceeds byte limit",
                        ));
                    }
                    std::fs::read(file_path).map_err(|error| {
                        ConfigError::new(
                            read_display,
                            format!("invalid enabled rule file: {error}"),
                        )
                    })
                })
                .map_err(|error| {
                    ConfigError::new(
                        display.clone(),
                        format!("bounded I/O worker failed: {error}"),
                    )
                })??;
                if let Some(inputs) = inputs.as_deref_mut() {
                    inputs
                        .record_bytes(&path, &bytes)
                        .map_err(|reason| ConfigError::new(path.display().to_string(), reason))?;
                }
                bytes
            }
        };
        if bytes.len() > crate::policy::POLICY_BYTES_LIMIT {
            return Err(ConfigError::new(
                path.display().to_string(),
                "rule file exceeds byte limit",
            ));
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            ConfigError::new(
                path.display().to_string(),
                format!("invalid enabled rule file: {e}"),
            )
        })?;
        strict_expressions.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned),
        );
    }
    let (matcher, accepted) = build_domain_set_with_inputs(
        &strict_expressions,
        &remaining_files,
        &plugin.root_dir,
        inputs,
    )
    .map_err(|error| match error {
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
    let semantic_sha256 = crate::special_groups::sha256(format!("{accepted:?}").as_bytes());
    let single_txt_file = !immutable && expressions.is_empty()
        && files.len() == 1
        // Generated routing providers belong to the immutable snapshot. Their
        // future management writes must rebuild a complete candidate instead
        // of replacing a live handle behind queries already admitted.
        && !plugin.source_path.starts_with("@managed-generated.")
        && Path::new(&files[0])
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
    // Every declared file reference is resolved the same way the loader
    // resolved it, so the conflict check sees query-only shapes too.
    let references: Vec<PathBuf> = files
        .iter()
        .map(|file| {
            let path = resolve_rule_path(file, &plugin.root_dir);
            canonicalized_path(&path).unwrap_or(path)
        })
        .collect();
    let candidate = if single_txt_file {
        let file = resolve_rule_path(&files[0], &plugin.root_dir);
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
        semantic_sha256,
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

/// The product default entry capacity for a cache whose `size` is absent or
/// non-positive.
const DEFAULT_CACHE_SIZE: u64 = 1024;

/// The product default periodic dump interval, in seconds.
const DEFAULT_CACHE_DUMP_INTERVAL_SECS: u64 = 600;

fn compile_cache(plugin: &RawPlugin) -> Result<CompiledCacheArgs, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "cache args must be a mapping")?;
    args.reject_unknown(
        &[
            "size",
            "lazy_cache_ttl",
            "dump_file",
            "dump_interval",
            "exclude_ip",
            "enable_ecs",
        ],
        &path,
    )?;
    // Absent or non-positive values fall back to the product defaults instead
    // of being rejected, matching the reference cache plugin's `init`.
    let capacity = match args.get("size") {
        None | Some(RawValue::Null) => DEFAULT_CACHE_SIZE,
        Some(value) => {
            let size = expect_integer(value, &format!("{path}.size"))?;
            if size <= 0 {
                DEFAULT_CACHE_SIZE
            } else {
                u64::try_from(size)
                    .map_err(|_| ConfigError::new(format!("{path}.size"), "size is out of range"))?
            }
        }
    };
    let lazy_cache_ttl_secs = match args.get("lazy_cache_ttl") {
        None | Some(RawValue::Null) => 0,
        Some(value) => {
            let ttl = expect_integer(value, &format!("{path}.lazy_cache_ttl"))?;
            if ttl < 0 {
                return Err(ConfigError::new(
                    format!("{path}.lazy_cache_ttl"),
                    "lazy_cache_ttl must not be negative",
                ));
            }
            u32::try_from(ttl).map_err(|_| {
                ConfigError::new(
                    format!("{path}.lazy_cache_ttl"),
                    "lazy_cache_ttl is out of range",
                )
            })?
        }
    };
    let dump_interval_secs = match args.get("dump_interval") {
        None | Some(RawValue::Null) => DEFAULT_CACHE_DUMP_INTERVAL_SECS,
        Some(value) => {
            let interval = expect_integer(value, &format!("{path}.dump_interval"))?;
            if interval <= 0 {
                DEFAULT_CACHE_DUMP_INTERVAL_SECS
            } else {
                u64::try_from(interval).map_err(|_| {
                    ConfigError::new(
                        format!("{path}.dump_interval"),
                        "dump_interval is out of range",
                    )
                })?
            }
        }
    };
    let dump_file = match args.get("dump_file") {
        None | Some(RawValue::Null) => None,
        Some(value) => {
            let declared = expect_string(value, &format!("{path}.dump_file"))?;
            if declared.is_empty() {
                return Err(ConfigError::new(
                    format!("{path}.dump_file"),
                    "dump_file must not be empty",
                ));
            }
            let resolved = resolve_relative(&declared, &plugin.root_dir);
            let absolute = if resolved.is_absolute() {
                resolved
            } else {
                std::env::current_dir()
                    .map_err(|e| ConfigError::new(&path, e.to_string()))?
                    .join(resolved)
            };
            let mut normalized = PathBuf::new();
            for component in absolute.components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    part => normalized.push(part.as_os_str()),
                }
            }
            Some(normalized)
        }
    };
    let enable_ecs = match args.get("enable_ecs") {
        None | Some(RawValue::Null) => false,
        Some(value) => expect_bool(value, &format!("{path}.enable_ecs"))?,
    };
    // The shape is still validated the way the product does, so a malformed
    // declaration reports the same reason it always did; a well-formed one is
    // refused because answer filtering is not implemented yet.
    let exclude_ip = match args.get("exclude_ip") {
        None | Some(RawValue::Null) => Vec::new(),
        Some(value) => match value {
            RawValue::String(value) => value.split_whitespace().map(str::to_owned).collect(),
            RawValue::Sequence(values) => {
                let mut entries = Vec::with_capacity(values.len());
                for (index, value) in values.iter().enumerate() {
                    entries.push(expect_string(
                        value,
                        &format!("{path}.exclude_ip[{index}]"),
                    )?);
                }
                entries
            }
            _ => {
                return Err(ConfigError::new(
                    format!("{path}.exclude_ip"),
                    "exclude_ip must be a whitespace-separated string or a list of strings",
                ));
            }
        },
    };
    Ok(CompiledCacheArgs {
        capacity,
        enable_ecs,
        lazy_cache_ttl_secs,
        dump_file,
        dump_interval_secs,
        exclude_ip,
    })
}

/// The validated cache arguments before an executable identity is known.
struct CompiledCacheArgs {
    capacity: u64,
    enable_ecs: bool,
    lazy_cache_ttl_secs: u32,
    dump_file: Option<PathBuf>,
    dump_interval_secs: u64,
    exclude_ip: Vec<String>,
}

fn compile_ecs(plugin: &RawPlugin) -> Result<crate::ecs::EcsPolicy, ConfigError> {
    let mut policy = crate::ecs::EcsPolicy::default();
    let path = format!("{}.args", plugin.source_path);
    if matches!(plugin.args, RawValue::Null) {
        return Ok(policy);
    }
    let args = expect_map(&plugin.args, &path, "ecs_handler args must be a mapping")?;
    args.reject_unknown(&["forward", "send", "preset", "mask4", "mask6"], &path)?;
    if let Some(value) = args.get("forward") {
        policy.forward = expect_bool(value, &format!("{path}.forward"))?;
    }
    if let Some(value) = args.get("send") {
        policy.send = expect_bool(value, &format!("{path}.send"))?;
    }
    if let Some(value) = args.get("preset") {
        let value = expect_string(value, &format!("{path}.preset"))?;
        if !value.is_empty() {
            policy.preset =
                Some(crate::ecs::unmap(value.parse().map_err(|_| {
                    ConfigError::new(&path, "preset must be an IP address")
                })?));
        }
    }
    for (key, max, default) in [("mask4", 32, 24), ("mask6", 128, 48)] {
        if let Some(value) = args.get(key) {
            let n = expect_integer(value, &format!("{path}.{key}"))?;
            if n < 0 || n > max {
                return Err(ConfigError::new(
                    format!("{path}.{key}"),
                    "ECS mask out of range",
                ));
            }
            let n = if n == 0 {
                default
            } else {
                u8::try_from(n).map_err(|_| ConfigError::new(&path, "ECS mask out of range"))?
            };
            if key == "mask4" {
                policy.mask4 = n;
            } else {
                policy.mask6 = n;
            }
        }
    }
    Ok(policy)
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
                    &plugin.root_dir,
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
    base: &Path,
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
            let config = match catalog.fast_mark(tag) {
                Some(config) => config,
                None if catalog.kind_of(tag) == Some(PluginKind::Switch) => {
                    return Err(ConfigError::new(
                        path,
                        format!(
                            "switch plugin `${tag}` is not a matcher; use the `switchN <value>` \
                             quick matcher"
                        ),
                    ));
                }
                None => {
                    let reason = format!("unknown matcher reference `${tag}`");
                    return Err(if catalog.kind_of(tag).is_none() {
                        ConfigError::unknown_reference(path, reason, tag)
                    } else {
                        ConfigError::new(path, reason)
                    });
                }
            };
            config.matcher()
        }
        name if crate::switch::switch_type_number(name).is_some() => {
            let type_number = crate::switch::switch_type_number(name)
                .expect("switch kind is validated before compilation");
            Box::new(crate::switch::SwitchMatcher::new(
                type_number,
                crate::switch::trim_expectation(args).to_owned(),
            ))
        }
        "fast_mark" => FastMarkConfig::parse(args)
            .map_err(|reason| ConfigError::new(path, reason))?
            .matcher(),
        "mark" => MarkConfig::parse(args)
            .map_err(|reason| ConfigError::new(path, reason))?
            .matcher(),
        "rcode" => RcodeConfig::parse(args)
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
        "client_ip" => Box::new(ClientIpMatcher::new(compile_response_ip(
            args, path, catalog, base,
        )?)),
        "resp_ip" => {
            let prefixes = compile_response_ip(args, path, catalog, base)?;
            catalog
                .response_ip_rules
                .borrow_mut()
                .push(ResponseIpRuleConfig {
                    source_path: path.to_owned(),
                    prefixes: prefixes.clone(),
                });
            Box::new(ResponseIpMatcher::new(prefixes))
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
            if let Some(handle) = catalog.adguard_matcher(tag) {
                groups.push((
                    format!("domain_set:{tag}"),
                    handle.map_err(|reason| ConfigError::new(path, reason))?,
                ));
                continue;
            }
            let set = catalog.domain_set(tag).ok_or_else(|| {
                let reason = format!("unknown domain_set reference `${tag}`");
                if catalog.kind_of(tag).is_none() {
                    ConfigError::unknown_reference(path, reason, tag)
                } else {
                    ConfigError::new(path, reason)
                }
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
        name if crate::switch::switch_type_number(name).is_some() => Err(ConfigError::new(
            path,
            format!(
                "switch `{name}` has no quick executable; reference its configured tag as \
                     `$tag`"
            ),
        )),
        "cname_remover" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, "cname_remover takes no arguments"));
            }
            let tag = format!("@native-cname:{sequence_tag}:{rule_index}:{exec_index}");
            catalog
                .policies
                .borrow_mut()
                .push((tag.clone(), ResponsePolicy::CnameRemover));
            Ok(ExecutableSpec::External {
                target: ExternalRef::new(tag),
            })
        }
        "ecs" => {
            let mut policy = crate::ecs::EcsPolicy::default();
            let mut tokens = args.split_whitespace();
            if let Some(first) = tokens.next() {
                let (ip, mask) = first
                    .split_once('/')
                    .map_or((first, None), |(ip, mask)| (ip, Some(mask)));
                policy.preset =
                    Some(crate::ecs::unmap(ip.parse().map_err(|_| {
                        ConfigError::new(path, "legacy ecs requires an IP")
                    })?));
                if mask.is_some() {
                    crate::diag::warn_line!("native {path}: legacy ecs mask is ignored");
                }
                if tokens.next().is_some() {
                    crate::diag::warn_line!(
                        "native {path}: trailing legacy ecs arguments are ignored"
                    );
                }
            } else {
                policy.active = false;
            }
            let tag = format!(
                "@native-ecs:{}:{rule_index}:{exec_index}",
                hex_identity(sequence_tag)
            );
            catalog
                .policies
                .borrow_mut()
                .push((tag.clone(), ResponsePolicy::Ecs(policy)));
            Ok(ExecutableSpec::External {
                target: ExternalRef::new(tag),
            })
        }
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
        "cache" => {
            // The inline cache form keeps the reference product's `cache [size]`
            // spelling: it takes at most one non-negative size and never a
            // `lazy_cache_ttl`, so inline use cannot silently enable lazy
            // retention. Each callsite owns a private instance.
            if args.split_whitespace().count() > 1 {
                return Err(ConfigError::new(
                    path,
                    "inline cache accepts at most one size argument",
                ));
            }
            let capacity = if args.trim().is_empty() {
                DEFAULT_CACHE_SIZE
            } else {
                let size: i64 = args
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::new(path, "inline cache size must be an integer"))?;
                if size <= 0 {
                    DEFAULT_CACHE_SIZE
                } else {
                    u64::try_from(size)
                        .map_err(|_| ConfigError::new(path, "inline cache size is out of range"))?
                }
            };
            let external = format!(
                "@native-quick-cache:{}:{}:{}",
                hex_identity(sequence_tag),
                rule_index,
                exec_index
            );
            let tag = format!(
                "quick-cache:{}:{rule_index}:{exec_index}",
                hex_identity(sequence_tag)
            );
            catalog.quick_caches.borrow_mut().push(QuickCacheSpec {
                external: external.clone(),
                tag,
                args: CompiledCacheArgs {
                    capacity,
                    enable_ecs: false,
                    lazy_cache_ttl_secs: 0,
                    dump_file: None,
                    dump_interval_secs: DEFAULT_CACHE_DUMP_INTERVAL_SECS,
                    exclude_ip: Vec::new(),
                },
            });
            Ok(ExecutableSpec::External {
                target: ExternalRef::new(external),
            })
        }
        "ttl" => {
            let policy = TtlPolicy::parse(args, path)?;
            let name = format!(
                "@native-ttl:{}:{rule_index}:{exec_index}",
                hex_identity(sequence_tag)
            );
            catalog
                .policies
                .borrow_mut()
                .push((name.clone(), ResponsePolicy::Ttl(policy)));
            Ok(ExecutableSpec::External {
                target: ExternalRef::new(name),
            })
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
        "mark" => {
            let config =
                MarkConfig::parse(args).map_err(|reason| ConfigError::new(path, reason))?;
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
        "tag_setter" => {
            let fixture_name = quick_fixture_name(path);
            fixtures.push(mosdns_sequence_core::FixtureSpec::new(
                fixture_name.clone(),
                TagSetterConfig::new(args).executor(),
            ));
            Ok(ExecutableSpec::Fixture {
                target: mosdns_sequence_core::FixtureRef::new(fixture_name),
            })
        }
        "drop_resp" => {
            if !args.is_empty() {
                return Err(ConfigError::new(path, "drop_resp takes no arguments"));
            }
            let fixture_name = quick_fixture_name(path);
            fixtures.push(mosdns_sequence_core::FixtureSpec::new(
                fixture_name.clone(),
                Box::new(DropRespExecutor),
            ));
            Ok(ExecutableSpec::Fixture {
                target: mosdns_sequence_core::FixtureRef::new(fixture_name),
            })
        }
        "sleep" => {
            let duration =
                parse_sleep_millis(args).map_err(|reason| ConfigError::new(path, reason))?;
            Ok(quick_sleep(
                catalog,
                sequence_tag,
                rule_index,
                exec_index,
                duration,
            ))
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
            if tag == PRESET_SLEEP_500MS && catalog.kind_of(tag).is_none() {
                return Ok(quick_sleep(
                    catalog,
                    sequence_tag,
                    rule_index,
                    exec_index,
                    Duration::from_millis(500),
                ));
            }
            match catalog.kind_of(tag) {
                Some(PluginKind::Sequence) => Ok(ExecutableSpec::Call {
                    target: SequenceRef::new(tag),
                }),
                Some(PluginKind::Forward) => {
                    catalog.forwards.borrow_mut().invocation(tag, args, path)
                }
                Some(
                    PluginKind::Cache
                    | PluginKind::Fallback
                    | PluginKind::Hosts
                    | PluginKind::Ecs
                    | PluginKind::Redirect
                    | PluginKind::Sleep
                    | PluginKind::Rewrite
                    | PluginKind::DomainOutput,
                ) => Ok(ExecutableSpec::External {
                    target: ExternalRef::new(tag),
                }),
                Some(
                    PluginKind::FastMark
                    | PluginKind::FlowSetter
                    | PluginKind::TagSetter
                    | PluginKind::Switch
                    | PluginKind::DomainMapper,
                ) => Ok(ExecutableSpec::Fixture {
                    target: mosdns_sequence_core::FixtureRef::new(plugin_fixture_name(tag)),
                }),
                Some(
                    PluginKind::DomainSet
                    | PluginKind::Listener
                    | PluginKind::IpSet
                    | PluginKind::SiSet
                    | PluginKind::DomainSetLight
                    | PluginKind::SdSetLight
                    | PluginKind::AdguardRule
                    | PluginKind::WebInfo
                    | PluginKind::Requery
                    | PluginKind::Socks5Proxy
                    | PluginKind::EcsAddress,
                ) => Err(ConfigError::new(
                    path,
                    format!("`{name}` is not executable"),
                )),
                None => Err(ConfigError::unknown_reference(
                    path,
                    format!("unknown executable reference `{name}`"),
                    tag,
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
        None => Err(ConfigError::unknown_reference(
            path,
            format!("unknown sequence reference `${tag}`"),
            tag,
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
    let listen = parse_listen_addr(&listen, &format!("{path}.listen"))?;
    let enable_audit = match args.get("enable_audit") {
        Some(value) => expect_bool(value, &format!("{path}.enable_audit"))?,
        None => false,
    };
    let idle_timeout = if is_tcp {
        let timeout = match args.get("idle_timeout") {
            Some(value) => expect_positive_integer(value, &format!("{path}.idle_timeout"))?,
            None => DEFAULT_TCP_IDLE_TIMEOUT_SECS,
        };
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

/// The per-upstream options a forward target is built from, already merged
/// with any plugin-level defaults.
pub(crate) struct ForwardTargetOptions {
    /// Numeric `dial_addr` text; the port defaults to the service port.
    pub(crate) dial_addr: Option<String>,
    pub(crate) bootstrap: Option<SocketAddr>,
    pub(crate) bootstrap_version: Option<u8>,
    pub(crate) query_timeout: Duration,
    pub(crate) insecure_skip_verify: bool,
    pub(crate) socks5: Option<String>,
}

fn parse_forward_target(
    value: &str,
    item: &RawMap,
    global_bootstrap: Option<SocketAddr>,
    global_bootstrap_version: Option<u8>,
    path: &str,
) -> Result<ForwardTargetConfig, ConfigError> {
    let options = ForwardTargetOptions {
        dial_addr: item
            .get("dial_addr")
            .map(|value| expect_string(value, &format!("{path}.dial_addr")))
            .transpose()?,
        bootstrap: item
            .get("bootstrap")
            .map(|value| parse_bootstrap(value, &format!("{path}.bootstrap")))
            .transpose()?
            .or(global_bootstrap),
        bootstrap_version: item
            .get("bootstrap_version")
            .map(|value| parse_bootstrap_version(value, &format!("{path}.bootstrap_version")))
            .transpose()?
            .or(global_bootstrap_version),
        query_timeout: parse_query_timeout(item.get("upstream_query_timeout"), path)?,
        insecure_skip_verify: item
            .get("insecure_skip_verify")
            .map(|value| expect_bool(value, &format!("{path}.insecure_skip_verify")))
            .transpose()?
            .unwrap_or(false),
        socks5: None,
    };
    validate_inactive_forward_options(item, path)?;
    build_forward_target(value, options, path)
}

/// Builds one validated target from typed options. A proxied TCP-based target
/// may name a host without a numeric address: the proxy resolves it.
pub(crate) fn build_forward_target(
    value: &str,
    options: ForwardTargetOptions,
    path: &str,
) -> Result<ForwardTargetConfig, ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.addr"),
            "addr must not be empty",
        ));
    }
    let (scheme, host, port, service) = parse_service_address(value, path)?;
    let dial_addr = options
        .dial_addr
        .as_deref()
        .map(|text| parse_numeric_text(text, port, &format!("{path}.dial_addr")))
        .transpose()?;
    let dial_addr = dial_addr.or_else(|| {
        host.parse::<std::net::IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, port))
    });
    let ForwardTargetOptions {
        bootstrap,
        bootstrap_version,
        query_timeout,
        insecure_skip_verify,
        socks5,
        ..
    } = options;
    let socks5 = socks5.filter(|_| scheme != ForwardScheme::Udp);

    if host.parse::<std::net::IpAddr>().is_err()
        && dial_addr.is_none()
        && bootstrap.is_none()
        && socks5.is_none()
    {
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
        socks5,
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
    parse_numeric_text(&expect_string(value, path)?, inherited_port, path)
}

fn parse_numeric_text(
    text: &str,
    inherited_port: u16,
    path: &str,
) -> Result<SocketAddr, ConfigError> {
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

/// The reference `tcp_server` default when `idle_timeout` is not configured.
const DEFAULT_TCP_IDLE_TIMEOUT_SECS: u64 = 10;

/// A listener address: a numeric `ip:port`, or `:port` for every address,
/// which binds the IPv6 wildcard (dual-stack, see `bind.rs`).
fn parse_listen_addr(value: &str, path: &str) -> Result<SocketAddr, ConfigError> {
    match value.strip_prefix(':') {
        Some(port) => match port.parse::<u16>() {
            Ok(port) if port != 0 => Ok(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port)),
            _ => Err(ConfigError::new(path, "port must be a nonzero number")),
        },
        None => parse_socket_addr(value, path),
    }
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

/// Reads one signed integer. Cache options accept non-positive values and clamp
/// them to their documented default, so the raw sign must survive parsing.
fn expect_integer(value: &RawValue, path: &str) -> Result<i64, ConfigError> {
    match value {
        RawValue::Number(RawNumber::Unsigned(value)) => {
            i64::try_from(*value).map_err(|_| ConfigError::new(path, "integer is out of range"))
        }
        RawValue::Number(RawNumber::Signed(value)) => Ok(*value),
        _ => Err(ConfigError::new(path, "expected an integer")),
    }
}

#[derive(Clone, Debug)]
struct RawPlugin {
    tag: String,
    kind: String,
    args: RawValue,
    source_path: String,
    /// The configuration root every relative plugin-argument path resolves
    /// against (`-d`, else the main configuration's directory), regardless of
    /// which file declared the plugin.
    root_dir: PathBuf,
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
                return Err(ConfigError {
                    kind: ConfigErrorKind::UnsupportedField,
                    ..ConfigError::new(format!("{path}.{key}"), "unsupported field")
                });
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

fn policy_strings(value: Option<&RawValue>, path: &str) -> Result<Vec<String>, ConfigError> {
    if matches!(value, None | Some(RawValue::Null)) {
        Ok(Vec::new())
    } else {
        string_list(value, path)
    }
}
fn policy_sources(
    plugin: &RawPlugin,
    field: &str,
    hosts_allowed: &[&str],
    redirect_allowed: &[&str],
) -> Result<(Vec<String>, Vec<PathBuf>), ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "policy args must be a mapping")?;
    args.reject_unknown(
        if field == "rules" {
            redirect_allowed
        } else {
            hosts_allowed
        },
        &path,
    )?;
    let inline = policy_strings(args.get(field), &format!("{path}.{field}"))?;
    let files = policy_strings(args.get("files"), &format!("{path}.files"))?
        .into_iter()
        .map(|file| resolve_rule_path(&file, &plugin.root_dir))
        .collect();
    Ok((inline, files))
}
fn compile_response_ip(
    args: &str,
    path: &str,
    catalog: &PluginCatalog<'_>,
    base: &Path,
) -> Result<Vec<Rc<crate::policy::IpListHandle>>, ConfigError> {
    if args.is_empty() {
        return Err(ConfigError::new(
            path,
            "resp_ip requires at least one IP/CIDR/provider/file",
        ));
    }
    let mut lists = Vec::new();
    let mut inline = Vec::new();
    let mut files = Vec::new();
    for token in args.split_whitespace() {
        if let Some(tag) = token.strip_prefix('$') {
            let set = catalog
                .ip_sets
                .iter()
                .find(|set| set.tag == tag)
                .ok_or_else(|| {
                    let reason = format!("unknown or wrong-type ip_set `{tag}`");
                    if catalog.kind_of(tag).is_none() {
                        ConfigError::unknown_reference(path, reason, tag)
                    } else {
                        ConfigError::new(path, reason)
                    }
                })?;
            lists.push(Rc::clone(&set.prefixes));
        } else if let Some(file) = token.strip_prefix('&') {
            if file.is_empty() {
                return Err(ConfigError::new(path, "empty IP file reference"));
            }
            files.push(resolve_rule_path(file, base));
        } else {
            inline.push(token.to_owned());
        }
    }
    let mut prefixes = mosdns_matcher_core::IpPrefixList::new();
    let mut budgets = catalog.ip_budgets.borrow_mut();
    let mut inputs = catalog.inputs.borrow_mut();
    let key = path.split(".args[").next().unwrap_or(path).to_owned();
    budgets.entry(key).or_default().load_with_inputs(
        &inline,
        &files,
        true,
        path,
        inputs.as_mut(),
        |line, location| {
            let (ip, bits) =
                policy::ip_prefix(line.split_whitespace().next().unwrap_or(""), location)?;
            prefixes.append(ip, bits);
            Ok(())
        },
    )?;
    prefixes.rebuild();
    lists.push(Rc::new(crate::policy::IpListHandle::new(Rc::new(prefixes))));
    Ok(lists)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mosdns_sequence_core::{ExecutionControl, ExecutionState};
    use mosdns_upstream_core::Transport;

    use super::{ListenerKind, LogLevel, PreferenceFamily, compile_yaml, compile_yaml_with_base};

    const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");
    const TCP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-tcp.yaml");

    #[test]
    fn cache_dependency_fingerprint_fails_closed_for_file_backed_ip_matchers() {
        let root =
            std::env::temp_dir().join(format!("native-cache-ip-dependency-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        for (index, matcher) in ["client_ip", "resp_ip"].into_iter().enumerate() {
            let rules = root.join(format!("rules-{index}.txt"));
            std::fs::write(&rules, b"192.0.2.0/24\n").unwrap();
            let rules_yaml = if matcher == "client_ip" {
                format!(
                    "      - matches: client_ip &rules-{index}.txt\n        exec: $cache\n      - exec: $upstream\n"
                )
            } else {
                format!(
                    "      - exec: $cache\n      - exec: $upstream\n      - matches: resp_ip &rules-{index}.txt\n        exec: reject\n"
                )
            };
            let yaml = format!(
                r#"log: {{level: error}}
plugins:
  - tag: upstream
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:15453"}}]}}
  - tag: cache
    type: cache
    args: {{size: 64}}
  - tag: entry
    type: sequence
    args:
{rules_yaml}
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:15353", enable_audit: false}}
"#
            );

            let before = compile_yaml_with_base(&yaml, &root).unwrap();
            let cache = before
                .caches
                .iter()
                .find(|cache| cache.tag == "cache")
                .unwrap();
            let before = before.cache_dependencies[cache.id.0].clone();
            std::fs::write(&rules, b"203.0.113.0/24\n").unwrap();
            let after = compile_yaml_with_base(&yaml, &root).unwrap();
            let cache = after
                .caches
                .iter()
                .find(|cache| cache.tag == "cache")
                .unwrap();
            let after = after.cache_dependencies[cache.id.0].clone();

            assert!(
                before.is_none() && after.is_none(),
                "{matcher} file-backed IP rules lack a complete reusable fingerprint"
            );
        }

        std::fs::remove_dir_all(root).unwrap();
    }

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
        let absolute_overflow = Instant::now()
            .checked_add(Duration::from_millis(u64::MAX))
            .is_none();
        assert_eq!(
            overflow.is_err(),
            absolute_overflow,
            "load validation must mirror platform monotonic deadline arithmetic"
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
  - { tag: l2, type: udp_server, args: { entry: s, listen: "127.0.0.1:53", enable_audit: false } }
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
            "resp_ip ::1/129",
            "resp_ip 192.0.2.1/33",
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
    #[test]
    fn request_primitives_compile_in_every_reference_form() {
        // Quick forms, named plugins, and the `_sleep_500ms` preset.
        let yaml = native_plugin_yaml(
            "      - matches: rcode 0 3\n        exec: tag_setter quick label\n      - exec: drop_resp\n      - exec: sleep 25\n      - exec: $named_setter\n      - exec: $named_sleep\n      - exec: $_sleep_500ms\n      - exec: sleep -3\n      - exec: $forward\n",
            "  - tag: named_setter\n    type: tag_setter\n    args: { tag: named }\n  - tag: named_sleep\n    type: sleep\n    args: { duration: 40 }\n",
        );
        let config = compile_yaml(&yaml).expect("request primitives compile");
        let mut durations: Vec<_> = config
            .sleeps
            .iter()
            .map(|sleep| sleep.duration.as_millis())
            .collect();
        durations.sort_unstable();
        // `sleep -3` is accepted and does not wait, as in the reference.
        assert_eq!(durations, [0, 25, 40, 500]);
        assert!(config.sleeps.iter().any(|sleep| sleep.tag == "named_sleep"));

        // A bare `tag_setter` and a named one without a tag are accepted.
        let bare = native_plugin_yaml(
            "      - exec: tag_setter\n      - exec: $empty_setter\n      - exec: $forward\n",
            "  - tag: empty_setter\n    type: tag_setter\n    args: {}\n",
        );
        compile_yaml(&bare).expect("clearing tag setters compile");
        // `rcode` without a code is accepted by the reference and never matches.
        compile_yaml(&native_plugin_yaml(
            "      - matches: rcode\n        exec: reject 3\n      - exec: $forward\n",
            "",
        ))
        .expect("an empty rcode list compiles");
    }

    #[test]
    fn request_primitives_reject_bad_values_with_paths() {
        for (sequence, expected) in [
            (
                "      - matches: rcode 2 x\n        exec: reject 3\n",
                "$.plugins[0].args[0].matches[0]",
            ),
            (
                "      - matches: rcode 1.5\n        exec: reject 3\n",
                "$.plugins[0].args[0].matches[0]",
            ),
            ("      - exec: sleep\n", "$.plugins[0].args[0].exec"),
            ("      - exec: sleep soon\n", "$.plugins[0].args[0].exec"),
            ("      - exec: sleep 1 2\n", "$.plugins[0].args[0].exec"),
            ("      - exec: drop_resp now\n", "$.plugins[0].args[0].exec"),
        ] {
            let error = compile_yaml(&native_plugin_yaml(
                &format!("{sequence}      - exec: $forward\n"),
                "",
            ))
            .err()
            .unwrap_or_else(|| panic!("`{sequence}` must be rejected"));
            assert_eq!(error.path, expected, "{sequence}: {error}");
        }
        for (definition, expected) in [
            (
                "  - tag: p\n    type: sleep\n    args: { duration: -1 }\n",
                "$.plugins[1].args.duration",
            ),
            (
                "  - tag: p\n    type: sleep\n    args: { duration: soon }\n",
                "$.plugins[1].args.duration",
            ),
            (
                "  - tag: p\n    type: sleep\n    args: { millis: 5 }\n",
                "$.plugins[1].args.millis",
            ),
            (
                "  - tag: p\n    type: tag_setter\n    args: { tag: [a] }\n",
                "$.plugins[1].args.tag",
            ),
            (
                "  - tag: p\n    type: tag_setter\n    args: { label: a }\n",
                "$.plugins[1].args.label",
            ),
        ] {
            let error = compile_yaml(&native_plugin_yaml(
                "      - exec: $p\n      - exec: $forward\n",
                definition,
            ))
            .err()
            .unwrap_or_else(|| panic!("`{definition}` must be rejected"));
            assert_eq!(error.path, expected, "{definition}: {error}");
        }
        // `drop_resp` exists only as a quick form in the reference.
        let error = compile_yaml(&native_plugin_yaml(
            "      - exec: $forward\n",
            "  - tag: p\n    type: drop_resp\n    args: {}\n",
        ))
        .err()
        .expect("no named drop_resp");
        assert!(error.reason.contains("unsupported plugin type"), "{error}");
    }

    #[test]
    fn fallback_targets_accept_bare_and_dollar_tags() {
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
    args: { primary: PRIMARY, secondary: SECONDARY }
  - tag: forward
    type: forward
    args: { upstreams: [ { addr: "udp://127.0.0.1:15453" } ] }
  - tag: listener
    type: udp_server
    args: { entry: entry, listen: "127.0.0.1:15353", enable_audit: false }
"#;
        let compile = |primary: &str, secondary: &str| {
            compile_yaml(
                &yaml
                    .replace("PRIMARY", primary)
                    .replace("SECONDARY", secondary),
            )
        };
        let bare = compile("primary", "secondary").expect("bare tags");
        let dollar = compile("$primary", "$secondary").expect("dollar tags");
        let mixed = compile("primary", "$secondary").expect("mixed tags");
        for config in [&bare, &dollar, &mixed] {
            assert_eq!(config.fallbacks.len(), 1);
            assert_eq!(config.fallbacks[0].primary, bare.fallbacks[0].primary);
            assert_eq!(config.fallbacks[0].secondary, bare.fallbacks[0].secondary);
        }
        for (primary, secondary, field) in [
            ("missing", "secondary", "primary"),
            ("primary", "\"$\"", "secondary"),
            ("\"two words\"", "secondary", "primary"),
        ] {
            let error = compile(primary, secondary).err().expect("invalid target");
            assert!(error.path.contains(field), "{error}");
        }
        // A named sleep is a legal (if pointless) fallback target.
        let sleepy = yaml
            .replace("PRIMARY", "nap")
            .replace("SECONDARY", "secondary")
            .replace(
                "  - tag: forward\n",
                "  - tag: nap\n    type: sleep\n    args: { duration: 1 }\n  - tag: forward\n",
            );
        compile_yaml(&sleepy).expect("sleep as fallback target");
    }
}
