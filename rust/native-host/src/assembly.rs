use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use crate::cache::{CacheAdapterError, CacheClock, NativeCacheAdapter};
use mosdns_upstream_core::{
    Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse, TransportCancellation, Upstream,
    UpstreamError,
};

use crate::config::{CompiledConfig, ConfigError, compile_yaml};
use crate::tcp::{TcpServer, TcpServerError};
use crate::udp::{UdpServer, UdpServerError};

/// Host-side options reserved for tests and the later request runner.
/// Configuration files cannot override these values in this task.
#[derive(Clone)]
pub struct HostOptions {
    pub request_deadline: Duration,
    pub cancellation: Option<TransportCancellation>,
    pub cache_clock: Rc<dyn CacheClock>,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            request_deadline: Duration::from_secs(5),
            cancellation: None,
            cache_clock: Rc::new(crate::cache::MonotonicCacheClock::new()),
        }
    }
}

impl HostOptions {
    /// Sets a short deadline for a focused request test without adding a YAML
    /// timeout field to the accepted product configuration.
    #[must_use]
    pub fn with_deadline(request_deadline: Duration) -> Self {
        Self {
            request_deadline,
            cancellation: None,
            cache_clock: Rc::new(crate::cache::MonotonicCacheClock::new()),
        }
    }

    /// Injects a caller-owned cancellation scope for a focused request test.
    #[must_use]
    pub fn with_cancellation(mut self, cancellation: TransportCancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    #[must_use]
    pub fn with_cache_clock(mut self, cache_clock: Rc<dyn CacheClock>) -> Self {
        self.cache_clock = cache_clock;
        self
    }
}

/// The one runtime owner used by the native host.
pub struct HostRuntime {
    runtime: tokio::runtime::Runtime,
}

impl HostRuntime {
    /// Builds the current-thread runtime owned by the host. No listener or
    /// socket is opened by this constructor.
    pub fn new() -> Result<Self, AssemblyError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|runtime| Self { runtime })
            .map_err(|error| AssemblyError::Runtime(error.to_string()))
    }

    #[must_use]
    pub fn as_runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }

    /// Runs a future with a local task set on the host-owned runtime.
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        tokio::task::LocalSet::new().block_on(&self.runtime, future)
    }
}

/// Pre-I/O native host graph. It owns the single runtime and one async
/// upstream owner, but deliberately does not bind the configured listener.
pub struct HostAssembly {
    config: Rc<CompiledConfig>,
    forward: Rc<ForwardAdapter>,
    cache: Rc<NativeCacheAdapter>,
    runtime: HostRuntime,
    options: HostOptions,
}

impl HostAssembly {
    /// Compiles YAML and constructs only resources that are safe before I/O.
    pub fn from_yaml(yaml: &str) -> Result<Self, AssemblyError> {
        let config = compile_yaml(yaml).map_err(AssemblyError::Config)?;
        Self::from_config(config)
    }

    /// Constructs a pre-I/O graph from an already compiled configuration.
    pub fn from_config(config: CompiledConfig) -> Result<Self, AssemblyError> {
        Self::with_options(config, HostOptions::default())
    }

    /// Constructs a graph with test-only request options. The options do not
    /// alter YAML acceptance and are used by the later request runner.
    pub fn with_options(
        config: CompiledConfig,
        options: HostOptions,
    ) -> Result<Self, AssemblyError> {
        let endpoint = config.forward.endpoint;
        let config = Rc::new(config);
        let cache = Rc::new(
            NativeCacheAdapter::with_clock(options.cache_clock.clone())
                .map_err(AssemblyError::Cache)?,
        );
        Ok(Self {
            config,
            forward: Rc::new(ForwardAdapter::new(endpoint)),
            cache,
            runtime: HostRuntime::new()?,
            options,
        })
    }

    #[must_use]
    pub fn config(&self) -> &CompiledConfig {
        &self.config
    }

    #[must_use]
    pub fn forward(&self) -> &ForwardAdapter {
        &self.forward
    }

    #[must_use]
    pub fn cache(&self) -> &NativeCacheAdapter {
        &self.cache
    }

    #[must_use]
    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.as_runtime()
    }

    /// Runs a future on the host's current-thread runtime and local task set.
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        self.runtime.block_on(future)
    }

    #[must_use]
    pub const fn options(&self) -> &HostOptions {
        &self.options
    }

    /// The configured upstream endpoint, exposed without exposing the owner
    /// internals or creating a second transport adapter.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.forward.endpoint()
    }

    pub(crate) fn config_handle(&self) -> Rc<CompiledConfig> {
        Rc::clone(&self.config)
    }

    pub(crate) fn forward_handle(&self) -> Rc<ForwardAdapter> {
        Rc::clone(&self.forward)
    }

    pub(crate) fn cache_handle(&self) -> Rc<NativeCacheAdapter> {
        Rc::clone(&self.cache)
    }

    /// Binds and serves the configured UDP listener. TCP remains a later
    /// slice, so a TCP configuration is rejected without binding anything.
    pub fn run_udp(&self) -> Result<(), UdpServerError> {
        let server = self.block_on(UdpServer::bind_configured(self))?;
        self.block_on(server.serve(TransportCancellation::new()))
    }

    /// Binds and serves the configured TCP listener.
    pub fn run_tcp(&self) -> Result<(), TcpServerError> {
        let server = self.block_on(TcpServer::bind_configured(self))?;
        self.block_on(server.serve(TransportCancellation::new()))
    }

    /// Runs the listener selected by the strictly compiled configuration.
    pub fn run(&self) -> Result<(), HostRunError> {
        match self.config.listener.kind {
            crate::config::ListenerKind::Udp => self.run_udp().map_err(HostRunError::Udp),
            crate::config::ListenerKind::Tcp => self.run_tcp().map_err(HostRunError::Tcp),
        }
    }
}

/// Listener failures returned by the native host's small runtime entrypoint.
#[derive(Debug)]
pub enum HostRunError {
    Udp(UdpServerError),
    Tcp(TcpServerError),
}

impl std::fmt::Display for HostRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Udp(error) => error.fmt(formatter),
            Self::Tcp(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HostRunError {}

/// The only forward adapter owned by the native host. It delegates request
/// validation and exchange execution to `upstream-core`; callers supply the
/// runtime, deadline, and cancellation scope.
pub struct ForwardAdapter {
    upstream: Upstream,
}

impl ForwardAdapter {
    fn new(endpoint: Endpoint) -> Self {
        Self {
            upstream: Upstream::new(endpoint),
        }
    }

    #[must_use]
    pub const fn endpoint(&self) -> Endpoint {
        self.upstream.endpoint()
    }

    #[must_use]
    pub fn upstream(&self) -> &Upstream {
        &self.upstream
    }

    /// Performs one caller-owned exchange on the caller's Tokio runtime.
    /// This method is not invoked by Slice 2, so assembly remains pre-I/O.
    pub async fn exchange(
        &self,
        query: &[u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
    ) -> Result<ExchangeResponse, UpstreamError> {
        let request = ExchangeRequest::new(query)?;
        let context = ExchangeContext::new(deadline, cancellation);
        self.upstream.exchange(request, context).await
    }
}

/// Failures that can occur before the native host owns a listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblyError {
    Config(ConfigError),
    Cache(CacheAdapterError),
    Runtime(String),
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Cache(error) => write!(formatter, "cache setup failed: {error}"),
            Self::Runtime(message) => write!(formatter, "runtime setup failed: {message}"),
        }
    }
}

impl std::error::Error for AssemblyError {}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mosdns_upstream_core::{LifecycleState, TransportCancellation, UpstreamError};

    use super::{AssemblyError, HostAssembly, HostOptions};
    use crate::config::ConfigError;

    const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

    #[test]
    fn valid_graph_is_pre_io_and_invalid_graph_never_reaches_assembly() {
        let host = HostAssembly::from_yaml(UDP).expect("valid graph must assemble");
        assert_eq!(host.config().listener.listen.port(), 15353);
        assert_eq!(host.endpoint().address().port(), 15453);
        assert_eq!(
            host.forward().upstream().lifecycle_state(),
            LifecycleState::Open
        );

        let options = HostOptions::with_deadline(Duration::from_millis(1))
            .with_cancellation(TransportCancellation::new());
        assert_eq!(options.request_deadline, Duration::from_millis(1));
        assert!(options.cancellation.is_some());

        let error = host.runtime().block_on(host.forward().exchange(
            &[],
            Instant::now() + Duration::from_secs(1),
            TransportCancellation::new(),
        ));
        assert!(matches!(error, Err(UpstreamError::InvalidRequest(_))));

        let invalid = UDP.replace("level: error", "level: info");
        assert!(matches!(
            HostAssembly::from_yaml(&invalid),
            Err(AssemblyError::Config(ConfigError { .. }))
        ));
    }
}
