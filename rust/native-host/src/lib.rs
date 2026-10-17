#![deny(unsafe_code)]
#![allow(clippy::pedantic)]

mod api;
mod appearance;
mod assembly;
mod bind;
pub mod build_identity;
mod cache;
mod cache_dump;
mod cache_placement;
mod cli;
mod config;
mod diag;
mod domain_output;
mod ecs;
mod execution;
mod external_ui;
mod local_time;
mod local_url;
mod managed;
mod matchers;
mod memory;
mod observer;
mod plugins;
mod policy;
mod process_metrics;
mod requery;
mod restart;
mod rewrite;
mod rule_download;
mod rule_sources;
mod runtime_snapshot;
mod settings_api;
#[doc(hidden)]
pub mod settings_migrate;
mod special_groups;
mod standard_groups_api;
mod static_ui;
mod switch;
#[cfg(test)]
mod switch_admission_tests;
mod switch_state;
mod system_api;
mod tcp;
mod transaction;
mod udp;
mod webinfo;

pub use api::{ApiServer, ApiServerError};
pub use assembly::{
    AssemblyError, BoundHost, DnsServer, ForwardAdapter, HostAssembly, HostOptions, HostRunError,
    HostRuntime,
};
pub use cache::{
    CacheAdapterError, CacheCatalog, CacheClock, CacheTestClock, NativeCacheAdapter, PendingStore,
};
pub use cli::{CliCommand, CliError, parse_args, resolve_config_paths};
pub use config::{
    ApiConfig, CacheId, CacheKind, CachePluginConfig, CheckFinding, CheckFindingKind, CheckReport,
    CompiledConfig, ConfigError, DomainMapperConfig, DomainSetConfig, ForwardConfig,
    ForwardDefinitionConfig, ForwardEntryConfig, ForwardInvocationConfig, ForwardScheme,
    ForwardTargetConfig, ListenerConfig, ListenerKind, LogLevel, ManagedRouterConfig,
    ManagedRouterGroup, MapperOutcome, ResponseIpRuleConfig, SequenceConfig, check_config,
    check_config_in, compile_yaml, compile_yaml_with_base, compile_yaml_with_roots,
    load_and_compile, load_yaml,
};
pub use diag::record_fatal;
pub use managed::{ManagedDomainSet, ManagedSetError, PersistFault, PersistGate};
pub use memory::configure_process_memory;
pub use observer::{
    AdmissionError, AnswerDetailsStatus, AuditAnswer, AuditClock, AuditRecord, AuditSnapshot,
    AuditTestClock, CacheStatus, DurationHistogramBucket, DurationHistogramSnapshot,
    FailureProvenance, LocalFailureKind, MetricsSnapshot, QueryTerminalOutcome, QueryTransport,
    ResponseDetails, ResponseFlags, ResponseSource, ResponseState, UpstreamAttemptMetricsSnapshot,
    UpstreamAttemptOutcome, UpstreamAttemptRecord, UpstreamDiagnosticAttempt,
    UpstreamDiagnosticSelected, UpstreamDiagnostics, UpstreamTransport,
};
pub use restart::{RestartTarget, capture as capture_restart_target, captured as restart_target};
pub use special_groups::{ManagedProfile, SpecialGroup, forward_entries, normalize_groups};
pub use switch::SwitchDeclaration;
pub use tcp::{TcpServer, TcpServerError};
pub use udp::{UdpServer, UdpServerError};

/// Parses and prepares a native host without opening a listener or an
/// upstream socket.
pub fn prepare_from_args<I, S>(args: I) -> Result<HostAssembly, HostError>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let command = parse_args(args).map_err(HostError::Cli)?;
    match command {
        CliCommand::Version => Err(HostError::Cli(CliError::new(
            "version does not prepare a host",
        ))),
        CliCommand::Check { .. } => Err(HostError::Cli(CliError::new(
            "check does not prepare a host",
        ))),
        CliCommand::Start { config, dir } => {
            let (config, root) =
                resolve_config_paths(&config, dir.as_deref()).map_err(HostError::Cli)?;
            HostAssembly::from_config_file_in(&config, root.as_deref()).map_err(HostError::Assembly)
        }
    }
}

/// Top-level preparation errors returned by the small CLI entrypoint.
#[derive(Debug)]
pub enum HostError {
    Cli(CliError),
    Config(ConfigError),
    Assembly(AssemblyError),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cli(error) => error.fmt(formatter),
            Self::Config(error) => error.fmt(formatter),
            Self::Assembly(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HostError {}

pub use policy::{
    DomainPayload, HostAddresses, IpListHandle, IpSetConfig, POLICY_BYTES_LIMIT, POLICY_LINE_LIMIT,
    POLICY_RULE_LIMIT, ResponsePolicy, ResponsePolicyConfig, TtlPolicy,
};

pub use rule_sources::{RuleSources, RuleTiming};
pub use runtime_snapshot::{PreparedSnapshot, RuntimeControl};

pub use transaction::{CandidatePersistence, CompiledCandidate, ManagedStore, TransactionError};
