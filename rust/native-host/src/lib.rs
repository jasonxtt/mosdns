#![forbid(unsafe_code)]
#![allow(clippy::pedantic)]

mod api;
mod assembly;
mod cache;
mod cli;
mod config;
mod execution;
mod managed;
mod matchers;
mod observer;
mod plugins;
mod tcp;
mod udp;

pub use api::{ApiServer, ApiServerError};
pub use assembly::{
    AssemblyError, BoundHost, DnsServer, ForwardAdapter, HostAssembly, HostOptions, HostRunError,
    HostRuntime,
};
pub use cache::{CacheAdapterError, CacheClock, CacheTestClock, NativeCacheAdapter, PendingStore};
pub use cli::{CliCommand, CliError, parse_args};
pub use config::{
    ApiConfig, CachePluginConfig, CompiledConfig, ConfigError, DomainSetConfig, ForwardConfig,
    ListenerConfig, ListenerKind, LogLevel, SequenceConfig, compile_yaml, compile_yaml_with_base,
    load_and_compile, load_yaml,
};
pub use managed::{ManagedDomainSet, ManagedSetError, PersistFault, PersistGate};
pub use observer::{
    AuditRecord, AuditSnapshot, CacheStatus, DurationHistogramBucket, DurationHistogramSnapshot,
    FailureProvenance, LocalFailureKind, MetricsSnapshot, QueryTerminalOutcome, QueryTransport,
    ResponseSource, ResponseState, UpstreamAttemptMetricsSnapshot, UpstreamAttemptOutcome,
    UpstreamAttemptRecord,
};
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
        CliCommand::Start { config } => {
            HostAssembly::from_config_file(&config).map_err(HostError::Assembly)
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
