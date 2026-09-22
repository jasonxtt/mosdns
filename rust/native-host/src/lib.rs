#![forbid(unsafe_code)]
#![allow(clippy::pedantic)]

mod assembly;
mod cache;
mod cli;
mod config;
mod execution;
mod matchers;
mod tcp;
mod udp;

pub use assembly::{
    AssemblyError, ForwardAdapter, HostAssembly, HostOptions, HostRunError, HostRuntime,
};
pub use cache::{CacheAdapterError, CacheClock, CacheTestClock, NativeCacheAdapter, PendingStore};
pub use cli::{CliCommand, CliError, parse_args};
pub use config::{
    CachePluginConfig, CompiledConfig, ConfigError, ForwardConfig, ListenerConfig, ListenerKind,
    LogLevel, SequenceConfig, compile_yaml, load_yaml,
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
            let yaml = load_yaml(&config).map_err(HostError::Config)?;
            HostAssembly::from_yaml(&yaml).map_err(HostError::Assembly)
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
