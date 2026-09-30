use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::{CacheAdapterError, CacheClock, NativeCacheAdapter};
use mosdns_dns_core::parse_query;
use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, DohEndpoint, DohReuseOwner, DotEndpoint,
    Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse, ResolutionMode, ResolutionPolicy,
    ResolutionTarget, ReuseOwner, SecureReuseOwner, ServerIdentity, SideEffectState, SystemClock,
    TlsPolicy, Transport, TransportCancellation, UdpTcpPolicy, Upstream, UpstreamError,
};
use tokio::task::JoinSet;

use crate::api::{ApiServer, ApiServerError, AuditPersistenceFaults};
use crate::config::{
    CompiledConfig, ConfigError, ForwardScheme, ForwardTargetConfig, compile_yaml, load_and_compile,
};
use crate::execution::{ExchangeError, ExchangeExecutor, InvocationExchange};
use crate::observer::{
    AuditClock, AuditSnapshot, MetricsSnapshot, QueryObserver, UpstreamAttemptLedger,
    UpstreamAttemptOutcome, UpstreamAttemptTracker, UpstreamTransport,
};
use crate::tcp::{TcpServer, TcpServerError};
use crate::udp::{UdpServer, UdpServerError};

pub(crate) const DEFAULT_AUDIT_CAPACITY: usize = 100_000;

/// Host-side options reserved for tests and the later request runner.
/// Configuration files cannot override these values in this task.
#[derive(Clone)]
pub struct HostOptions {
    pub request_deadline: Duration,
    pub cancellation: Option<TransportCancellation>,
    pub cache_clock: Rc<dyn CacheClock>,
    pub(crate) audit_clock: Arc<dyn AuditClock>,
    pub(crate) admission_deadline: Option<std::time::Instant>,
    pub(crate) audit_capacity: usize,
    pub(crate) tls_roots: Option<Arc<rustls::RootCertStore>>,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            request_deadline: Duration::from_secs(5),
            cancellation: None,
            cache_clock: Rc::new(crate::cache::MonotonicCacheClock::new()),
            audit_clock: crate::observer::default_audit_clock(),
            admission_deadline: None,
            audit_capacity: DEFAULT_AUDIT_CAPACITY,
            tls_roots: None,
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
            audit_clock: crate::observer::default_audit_clock(),
            admission_deadline: None,
            audit_capacity: DEFAULT_AUDIT_CAPACITY,
            tls_roots: None,
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

    /// Injects the wall-clock source used by audit admission and windows.
    #[must_use]
    pub fn with_audit_clock(mut self, audit_clock: Arc<dyn AuditClock>) -> Self {
        self.audit_clock = audit_clock;
        self
    }

    /// Sets a focused-test audit retention capacity without changing YAML.
    #[must_use]
    pub fn with_audit_capacity(mut self, audit_capacity: usize) -> Self {
        self.audit_capacity = audit_capacity;
        self
    }

    /// Injects a caller-owned trust store for offline secure-transport tests
    /// and embedders. Production construction leaves this unset and loads the
    /// host system trust store instead.
    #[doc(hidden)]
    #[must_use]
    pub fn with_tls_roots(mut self, roots: rustls::RootCertStore) -> Self {
        self.tls_roots = Some(Arc::new(roots));
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

/// Pre-I/O native host graph. It owns the single runtime and immutable
/// executable-to-upstream catalog, but deliberately does not bind the
/// configured listener.
pub struct HostAssembly {
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    observer: Arc<QueryObserver>,
    runtime: HostRuntime,
    options: HostOptions,
    state_root: Option<PathBuf>,
    audit_persistence_faults: AuditPersistenceFaults,
}

impl HostAssembly {
    /// Compiles YAML and constructs only resources that are safe before I/O.
    pub fn from_yaml(yaml: &str) -> Result<Self, AssemblyError> {
        let config = compile_yaml(yaml).map_err(AssemblyError::Config)?;
        Self::from_config(config)
    }

    /// Reads and compiles one configuration file, resolving relative include
    /// and rule-file paths against that file's directory.
    pub fn from_config_file(path: &std::path::Path) -> Result<Self, AssemblyError> {
        let config = load_and_compile(path).map_err(AssemblyError::Config)?;
        let state_root = path.parent().unwrap_or_else(|| Path::new(""));
        Self::with_options_and_state_root(config, HostOptions::default(), state_root)
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
        Self::with_options_and_optional_state_root(config, options, None)
    }

    /// Constructs an in-memory graph with an explicit directory for managed
    /// runtime state. This is the test and embedding seam for persistent API
    /// settings; it never changes YAML path resolution.
    pub fn with_options_and_state_root(
        config: CompiledConfig,
        options: HostOptions,
        state_root: impl AsRef<Path>,
    ) -> Result<Self, AssemblyError> {
        Self::with_options_and_optional_state_root(
            config,
            options,
            Some(state_root.as_ref().to_path_buf()),
        )
    }

    fn with_options_and_optional_state_root(
        config: CompiledConfig,
        options: HostOptions,
        state_root: Option<PathBuf>,
    ) -> Result<Self, AssemblyError> {
        let config = Rc::new(config);
        let forwards = Rc::new(
            ForwardCatalog::from_compiled_config(&config, options.tls_roots.clone())
                .map_err(AssemblyError::Catalog)?,
        );
        let cache = Rc::new(
            NativeCacheAdapter::with_capacity_and_clock(
                config.cache.as_ref().map_or(1, |cache| cache.capacity),
                options.cache_clock.clone(),
            )
            .map_err(AssemblyError::Cache)?,
        );
        let upstream_identities = config
            .forwards
            .iter()
            .map(|forward| {
                forward
                    .upstream_tag
                    .as_deref()
                    .unwrap_or(&forward.tag)
                    .to_owned()
            })
            .collect::<Vec<_>>();
        let metric_registry = config
            .forward_invocations
            .iter()
            .flat_map(|invocation| {
                config
                    .forward_definitions
                    .get(invocation.definition)
                    .into_iter()
                    .flat_map(move |definition| {
                        invocation.entries.iter().filter_map(move |&entry_index| {
                            definition.entries.get(entry_index).map(|entry| {
                                (invocation.executable, entry_index, entry.identity.clone())
                            })
                        })
                    })
            })
            .collect::<Vec<_>>();
        let audit_capacity = state_root
            .as_deref()
            .map(|root| crate::api::load_audit_capacity(root, options.audit_capacity))
            .unwrap_or(options.audit_capacity);
        let observer = Arc::new(
            QueryObserver::try_with_clock_and_registry(
                config.listener.enable_audit,
                upstream_identities,
                metric_registry,
                audit_capacity,
                options.audit_clock.clone(),
            )
            .map_err(|error| AssemblyError::Runtime(error.to_string()))?,
        );
        Ok(Self {
            config,
            forwards,
            cache,
            observer,
            runtime: HostRuntime::new()?,
            options,
            state_root,
            audit_persistence_faults: AuditPersistenceFaults::default(),
        })
    }

    #[must_use]
    pub fn config(&self) -> &CompiledConfig {
        &self.config
    }

    #[must_use]
    pub fn forward(&self) -> Option<&ForwardAdapter> {
        let executable = self.config.forward.as_ref()?.executable;
        self.forwards.forward(executable)
    }

    #[must_use]
    pub fn cache(&self) -> &NativeCacheAdapter {
        &self.cache
    }

    /// Copies the host's fixed-cardinality metrics at one consistent snapshot boundary.
    #[must_use]
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.observer.metrics_snapshot()
    }

    /// Copies the bounded audit ring in oldest-to-newest order.
    #[must_use]
    pub fn audit_snapshot(&self) -> AuditSnapshot {
        self.observer.audit_snapshot()
    }

    /// Starts terminal-time audit capture when the listener's static audit
    /// gate is enabled. The return value reports whether capture is active.
    pub fn start_audit(&self) -> bool {
        self.observer.start_capture()
    }

    /// Stops terminal-time audit capture without resetting lifetime metrics.
    pub fn stop_audit(&self) -> bool {
        self.observer.stop_capture()
    }

    /// Removes retained audit records without resetting lifetime metrics.
    pub fn clear_audit(&self) {
        self.observer.clear_audit();
    }

    /// Changes the bounded audit retention capacity, evicting oldest records
    /// immediately when the new capacity is smaller than the current ring.
    pub fn set_audit_capacity(&self, capacity: usize) {
        self.observer.set_audit_capacity(capacity);
    }

    #[must_use]
    pub fn audit_capacity(&self) -> usize {
        self.observer.audit_capacity()
    }

    #[must_use]
    pub fn audit_capturing(&self) -> bool {
        self.observer.is_capturing()
    }

    /// Arms a one-shot persistence failure before writing the temporary file.
    #[doc(hidden)]
    pub fn inject_audit_temp_write_failure(&self) {
        self.audit_persistence_faults.fail_next_temp_write();
    }

    /// Arms a one-shot persistence failure before replacing the canonical file.
    #[doc(hidden)]
    pub fn inject_audit_final_replace_failure(&self) {
        self.audit_persistence_faults.fail_next_final_replace();
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
    pub fn endpoint(&self) -> Option<Endpoint> {
        self.forward().map(ForwardAdapter::endpoint)
    }

    pub(crate) fn config_handle(&self) -> Rc<CompiledConfig> {
        Rc::clone(&self.config)
    }

    pub(crate) fn forwards_handle(&self) -> Rc<ForwardCatalog> {
        Rc::clone(&self.forwards)
    }

    pub(crate) fn cache_handle(&self) -> Rc<NativeCacheAdapter> {
        Rc::clone(&self.cache)
    }

    pub(crate) fn observer_handle(&self) -> Arc<QueryObserver> {
        Arc::clone(&self.observer)
    }

    /// Binds every configured listener without serving any of them. A failed
    /// bind drops the listeners that already succeeded, so no socket survives a
    /// failed startup.
    pub async fn bind_host(&self) -> Result<BoundHost, HostRunError> {
        let dns = match self.config.listener.kind {
            crate::config::ListenerKind::Udp => {
                let server = UdpServer::bind_configured(self)
                    .await
                    .map_err(HostRunError::Udp)?;
                DnsServer::Udp(server)
            }
            crate::config::ListenerKind::Tcp => {
                let server = TcpServer::bind_configured(self)
                    .await
                    .map_err(HostRunError::Tcp)?;
                DnsServer::Tcp(server)
            }
        };
        let dns_addr = dns.local_addr()?;
        let api = match &self.config.api {
            Some(config) => {
                let server = ApiServer::bind(
                    self.config_handle(),
                    self.observer_handle(),
                    self.state_root.clone(),
                    self.audit_persistence_faults.clone(),
                    self.options.audit_clock.clone(),
                    config.http,
                )
                .await
                .map_err(HostRunError::Api)?;
                let address = server.local_addr().map_err(HostRunError::Api)?;
                Some((server, address))
            }
            None => None,
        };
        let (api, api_addr) = match api {
            Some((server, address)) => (Some(server), Some(address)),
            None => (None, None),
        };
        Ok(BoundHost {
            dns,
            dns_addr,
            api,
            api_addr,
        })
    }

    /// The top-level supervisor entrypoint: binds DNS and the optional
    /// management listener, then serves both under one shutdown scope. A
    /// failure on either side cancels and joins the other before returning.
    pub async fn serve_host(&self) -> Result<(), HostRunError> {
        self.bind_host()
            .await?
            .serve(TransportCancellation::new())
            .await
    }

    /// Binds and serves the configured UDP listener without opening any
    /// listener socket during assembly.
    pub fn run_udp(&self) -> Result<(), UdpServerError> {
        let server = self.block_on(UdpServer::bind_configured(self))?;
        self.block_on(server.serve(TransportCancellation::new()))
    }

    /// Binds and serves the configured TCP listener.
    pub fn run_tcp(&self) -> Result<(), TcpServerError> {
        let server = self.block_on(TcpServer::bind_configured(self))?;
        self.block_on(server.serve(TransportCancellation::new()))
    }

    /// Runs the configured host: one DNS listener plus the optional scoped
    /// management listener, owned by one shutdown scope.
    pub fn run(&self) -> Result<(), HostRunError> {
        self.block_on(self.serve_host())
    }
}

/// One bound DNS listener, before any request is served.
pub enum DnsServer {
    Udp(UdpServer),
    Tcp(TcpServer),
}

impl DnsServer {
    fn local_addr(&self) -> Result<std::net::SocketAddr, HostRunError> {
        match self {
            Self::Udp(server) => server.local_addr().map_err(HostRunError::Udp),
            Self::Tcp(server) => server.local_addr().map_err(HostRunError::Tcp),
        }
    }

    async fn serve(self, shutdown: TransportCancellation) -> Result<(), HostRunError> {
        match self {
            Self::Udp(server) => server.serve(shutdown).await.map_err(HostRunError::Udp),
            Self::Tcp(server) => server.serve(shutdown).await.map_err(HostRunError::Tcp),
        }
    }
}

/// Every listener the supervisor bound, before any of them started serving.
/// The sockets are released when this value is dropped or its `serve` returns.
pub struct BoundHost {
    dns: DnsServer,
    dns_addr: std::net::SocketAddr,
    api: Option<ApiServer>,
    api_addr: Option<std::net::SocketAddr>,
}

impl BoundHost {
    #[must_use]
    pub const fn dns_addr(&self) -> std::net::SocketAddr {
        self.dns_addr
    }

    #[must_use]
    pub const fn api_addr(&self) -> Option<std::net::SocketAddr> {
        self.api_addr
    }

    /// Arms one narrow running-side failure on the management listener. Only
    /// tests call this.
    #[doc(hidden)]
    pub fn inject_api_accept_fault_after(&self, connections: usize) {
        if let Some(api) = &self.api {
            api.inject_accept_fault_after(connections);
        }
    }

    /// Drives both listeners under one shared shutdown scope. Whichever side
    /// fails first cancels the other; every listener task is joined and both
    /// sockets are closed before this returns. The DNS side owns the one
    /// upstream-catalog close, so the management side never closes it.
    pub async fn serve(self, shutdown: TransportCancellation) -> Result<(), HostRunError> {
        let Self {
            dns,
            dns_addr: _,
            api,
            api_addr: _,
        } = self;
        let mut tasks: JoinSet<Result<(), HostRunError>> = JoinSet::new();
        {
            let scope = shutdown.child_token();
            tasks.spawn_local(async move { dns.serve(scope).await });
        }
        if let Some(api) = api {
            let scope = shutdown.child_token();
            tasks.spawn_local(async move { api.serve(scope).await.map_err(HostRunError::Api) });
        }

        let mut failure: Option<HostRunError> = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                    // A running-side failure stops the other listener; the
                    // loop keeps joining until every task has finished.
                    shutdown.cancel();
                }
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(HostRunError::Task(error.to_string()));
                    }
                    shutdown.cancel();
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Listener failures returned by the native host's small runtime entrypoint.
#[derive(Debug)]
pub enum HostRunError {
    Udp(UdpServerError),
    Tcp(TcpServerError),
    Api(ApiServerError),
    Task(String),
}

impl std::fmt::Display for HostRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Udp(error) => error.fmt(formatter),
            Self::Tcp(error) => error.fmt(formatter),
            Self::Api(error) => error.fmt(formatter),
            Self::Task(error) => write!(formatter, "listener task failed: {error}"),
        }
    }
}

impl std::error::Error for HostRunError {}

/// One forward adapter owned by the native host catalog. It delegates request
/// validation and exchange execution to `upstream-core`; callers supply the
/// runtime, deadline, and cancellation scope.
pub struct ForwardAdapter {
    executable: mosdns_sequence_core::ExecutableId,
    entry_index: usize,
    target: ForwardTargetConfig,
    resolver: Option<Rc<BootstrapResolver>>,
    owner: RefCell<Option<Rc<ForwardOwner>>>,
    active_peer: RefCell<Option<std::net::SocketAddr>>,
    tls_roots: Option<Arc<rustls::RootCertStore>>,
    /// Compatibility handle for existing host tests/introspection. The
    /// transport owner above remains the runtime source of truth.
    legacy_upstream: Option<Upstream>,
}

enum ForwardOwner {
    Udp {
        policy: UdpTcpPolicy,
        legacy: Upstream,
    },
    Tcp {
        reuse: ReuseOwner,
        fallback: Upstream,
    },
    Dot(SecureReuseOwner),
    Doh(DohReuseOwner),
}

impl ForwardOwner {
    async fn exchange(
        &self,
        query: &[u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
        tracker: &mut UpstreamAttemptTracker,
    ) -> Result<ExchangeResponse, UpstreamError> {
        let request = ExchangeRequest::new(query)?;
        let context = ExchangeContext::new(deadline, cancellation);
        match self {
            Self::Udp { policy, .. } => {
                policy
                    .exchange_with_phase_hook(request, context, |transport| {
                        tracker.set_transport(match transport {
                            Transport::Udp => UpstreamTransport::Udp,
                            Transport::Tcp => UpstreamTransport::Tcp,
                            Transport::Quic => UpstreamTransport::Tcp,
                        });
                    })
                    .await
            }
            Self::Tcp { reuse, fallback } => match reuse.exchange(request, context.clone()).await {
                Err(UpstreamError::Backpressure(SideEffectState::NotSent)) => {
                    fallback.exchange(request, context).await
                }
                result => result,
            },
            Self::Dot(owner) => owner
                .exchange(request, context)
                .await
                .map(secure_response)
                .map_err(secure_error),
            Self::Doh(owner) => owner
                .exchange(request, context)
                .await
                .map(secure_response)
                .map_err(secure_error),
        }
    }

    async fn close(&self) {
        match self {
            Self::Udp { policy, legacy } => {
                let _ = tokio::join!(policy.close(), legacy.close());
            }
            Self::Tcp { reuse, fallback } => {
                let _ = tokio::join!(reuse.close(), fallback.close());
            }
            Self::Dot(owner) => {
                let _ = owner.close().await;
            }
            Self::Doh(owner) => {
                let _ = owner.close().await;
            }
        }
    }
}

fn secure_response(response: mosdns_upstream_core::secure::SecureResponse) -> ExchangeResponse {
    let request_id = response.request_id();
    let response_id = response.response_id();
    let truncated = response.truncated();
    ExchangeResponse::new(
        response.into_wire(),
        request_id,
        response_id,
        Transport::Tcp,
        truncated,
    )
}

fn secure_error(error: mosdns_upstream_core::SecureError) -> UpstreamError {
    match error {
        mosdns_upstream_core::SecureError::Transport(error) => error,
        mosdns_upstream_core::SecureError::DohProtocol(_)
        | mosdns_upstream_core::SecureError::DohRequest(_) => UpstreamError::MalformedResponse,
        mosdns_upstream_core::SecureError::Tls(_)
        | mosdns_upstream_core::SecureError::TlsConfig(_)
        | mosdns_upstream_core::SecureError::InvalidIdentity(_)
        | mosdns_upstream_core::SecureError::InvalidServiceUrl(_)
        | mosdns_upstream_core::SecureError::ZeroDialPort => UpstreamError::Connect,
        mosdns_upstream_core::SecureError::DoqProtocolTrailingResponse
        | mosdns_upstream_core::SecureError::DoqProtocolMissingResponseFin
        | mosdns_upstream_core::SecureError::DoqProtocolNonzeroResponseId => {
            UpstreamError::MalformedResponse
        }
    }
}

fn attempt_outcome(error: &UpstreamError) -> UpstreamAttemptOutcome {
    match error {
        UpstreamError::DeadlineExceeded(_) => UpstreamAttemptOutcome::TimedOut,
        UpstreamError::Cancelled(_) | UpstreamError::Closed(_) => UpstreamAttemptOutcome::Canceled,
        _ => UpstreamAttemptOutcome::Failed,
    }
}

fn tls_policy(
    insecure: bool,
    custom_roots: Option<&rustls::RootCertStore>,
) -> Result<TlsPolicy, String> {
    if insecure {
        return Ok(TlsPolicy::insecure_skip_verify());
    }
    let mut roots = custom_roots
        .cloned()
        .unwrap_or_else(rustls::RootCertStore::empty);
    if custom_roots.is_none() {
        let loaded = rustls_native_certs::load_native_certs();
        for certificate in loaded.certs {
            roots
                .add(certificate)
                .map_err(|_| "system trust contains an unusable certificate".to_owned())?;
        }
        if !loaded.errors.is_empty() && roots.is_empty() {
            return Err("system trust store could not provide usable roots".to_owned());
        }
    }
    TlsPolicy::verified(roots).map_err(|error| error.to_string())
}

/// Immutable executable-ID to upstream-owner catalog. W1/W2 populate one
/// entry; W3 populates one entry per validated route without introducing a
/// fallback-to-first-upstream path.
pub struct ForwardCatalog {
    owners: BTreeMap<mosdns_sequence_core::ExecutableId, Vec<Rc<ForwardAdapter>>>,
    concurrent: BTreeMap<mosdns_sequence_core::ExecutableId, usize>,
    rotation: RefCell<u64>,
}

impl ForwardCatalog {
    #[cfg(test)]
    fn from_configs(configs: &[crate::config::ForwardConfig]) -> Result<Self, String> {
        Self::from_configs_with_seed(configs, 1)
    }

    #[cfg(test)]
    fn from_configs_with_seed(
        configs: &[crate::config::ForwardConfig],
        seed: u64,
    ) -> Result<Self, String> {
        let mut owners = BTreeMap::new();
        for config in configs {
            if owners
                .insert(
                    config.executable,
                    vec![Rc::new(ForwardAdapter::new(
                        config.executable,
                        config.endpoint,
                    ))],
                )
                .is_some()
            {
                return Err(format!(
                    "duplicate forward executable {:?}",
                    config.executable
                ));
            }
        }
        let concurrent = owners.keys().map(|executable| (*executable, 1)).collect();
        Ok(Self {
            owners,
            concurrent,
            rotation: RefCell::new(seed),
        })
    }

    fn from_compiled_config(
        config: &CompiledConfig,
        tls_roots: Option<Arc<rustls::RootCertStore>>,
    ) -> Result<Self, String> {
        let mut owners: BTreeMap<mosdns_sequence_core::ExecutableId, Vec<Rc<ForwardAdapter>>> =
            BTreeMap::new();
        let mut shared: BTreeMap<(usize, usize), Rc<ForwardAdapter>> = BTreeMap::new();
        let mut concurrent = BTreeMap::new();
        for invocation in &config.forward_invocations {
            let definition = config
                .forward_definitions
                .get(invocation.definition)
                .ok_or_else(|| {
                    format!("forward definition {} is missing", invocation.definition)
                })?;
            concurrent.insert(invocation.executable, definition.concurrent);
            let mut invocation_owners = Vec::with_capacity(invocation.entries.len());
            for &entry_index in &invocation.entries {
                let entry = definition.entries.get(entry_index).ok_or_else(|| {
                    format!(
                        "forward invocation {:?} references missing entry {}",
                        invocation.executable, entry_index
                    )
                })?;
                let owner = if let Some(owner) = shared.get(&(invocation.definition, entry_index)) {
                    Rc::clone(owner)
                } else {
                    let owner = Rc::new(ForwardAdapter::new_entry(
                        invocation.executable,
                        entry_index,
                        entry.target.clone(),
                        tls_roots.clone(),
                    )?);
                    shared.insert((invocation.definition, entry_index), Rc::clone(&owner));
                    owner
                };
                invocation_owners.push(owner);
            }
            owners.insert(invocation.executable, invocation_owners);
        }
        let mut seed_bytes = [0_u8; 8];
        let seed = if getrandom::fill(&mut seed_bytes).is_ok() {
            u64::from_ne_bytes(seed_bytes)
        } else {
            0x9e37_79b9_7f4a_7c15
        };
        Ok(Self {
            owners,
            concurrent,
            rotation: RefCell::new(seed),
        })
    }

    fn rotation_start(&self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let mut state = self.rotation.borrow_mut();
        let value = *state;
        *state = value
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (value as usize) % len
    }

    #[must_use]
    pub fn forward(
        &self,
        executable: mosdns_sequence_core::ExecutableId,
    ) -> Option<&ForwardAdapter> {
        self.owners
            .get(&executable)
            .and_then(|owners| (owners.len() == 1).then(|| owners[0].as_ref()))
    }

    pub async fn close_all(&self) {
        let mut closed = BTreeSet::new();
        for owners in self.owners.values() {
            for owner in owners {
                let key = Rc::as_ptr(owner) as usize;
                if closed.insert(key) {
                    owner.close().await;
                }
            }
        }
    }
}

impl ExchangeExecutor for ForwardCatalog {
    fn exchange<'a>(
        &'a self,
        executable: mosdns_sequence_core::ExecutableId,
        query: &'a [u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ExchangeResponse, ExchangeError>> + 'a>,
    > {
        let Some(owner) = self
            .owners
            .get(&executable)
            .and_then(|owners| owners.first())
        else {
            return Box::pin(async move { Err(ExchangeError::UnknownExecutable(executable)) });
        };
        Box::pin(async move {
            owner
                .exchange(query, deadline, cancellation)
                .await
                .map_err(ExchangeError::Upstream)
        })
    }

    fn exchange_invocation<'a>(
        &'a self,
        executable: mosdns_sequence_core::ExecutableId,
        query: &'a [u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
        ledger: &'a mut UpstreamAttemptLedger,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InvocationExchange, ExchangeError>> + 'a>,
    > {
        let Some(owners) = self.owners.get(&executable) else {
            return Box::pin(async move { Err(ExchangeError::UnknownExecutable(executable)) });
        };
        let concurrent = self.concurrent.get(&executable).copied().unwrap_or(1);
        let query = query.to_vec();
        let (header, question) = match parse_query(&query) {
            Ok(parsed) => parsed,
            Err(_) => {
                return Box::pin(async move {
                    Err(ExchangeError::Upstream(UpstreamError::InvalidRequest(
                        mosdns_upstream_core::RequestError::Malformed,
                    )))
                });
            }
        };
        if owners.len() <= 1 || concurrent <= 1 {
            let owner = Rc::clone(&owners[0]);
            let shared = std::mem::take(ledger).into_shared();
            let slot = shared.borrow_mut().start(owner.entry_index());
            let mut tracker =
                UpstreamAttemptTracker::new(Rc::clone(&shared), slot, cancellation.clone());
            return Box::pin(async move {
                let result = owner
                    .exchange_tracked(&query, deadline, cancellation, &mut tracker)
                    .await;
                let peer = tracker.peer();
                let transport = tracker.transport();
                let outcome = result
                    .as_ref()
                    .map(|_| UpstreamAttemptOutcome::Response)
                    .unwrap_or_else(attempt_outcome);
                tracker.finish(peer, transport, outcome);
                drop(tracker);
                let response = result
                    .map(|response| InvocationExchange {
                        selected_entry: Some(owner.entry_index()),
                        selected_peer: peer,
                        response,
                    })
                    .map_err(ExchangeError::Upstream);
                *ledger = UpstreamAttemptLedger::restore_from_shared(shared);
                response
            });
        }

        let scope = cancellation.child_token();
        let limit = concurrent.min(3).min(owners.len());
        let start = self.rotation_start(owners.len());
        let owners = (0..limit)
            .map(|offset| Rc::clone(&owners[(start + offset) % owners.len()]))
            .collect::<Vec<_>>();
        let shared = std::mem::take(ledger).into_shared();
        let slots = owners
            .iter()
            .map(|owner| {
                (
                    owner.entry_index(),
                    shared.borrow_mut().start(owner.entry_index()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Box::pin(async move {
            let mut tasks = JoinSet::new();
            for owner in owners {
                let entry_index = owner.entry_index();
                let slot = slots[&entry_index];
                let leg_cancellation = scope.child_token();
                let leg_query = query.clone();
                let shared = Rc::clone(&shared);
                tasks.spawn_local(async move {
                    let mut tracker =
                        UpstreamAttemptTracker::new(shared, slot, leg_cancellation.clone());
                    let result = owner
                        .exchange_tracked(&leg_query, deadline, leg_cancellation, &mut tracker)
                        .await;
                    let peer = tracker.peer();
                    let transport = tracker.transport();
                    let outcome = result
                        .as_ref()
                        .map(|_| UpstreamAttemptOutcome::Response)
                        .unwrap_or_else(attempt_outcome);
                    tracker.finish(peer, transport, outcome);
                    (entry_index, peer, transport, result)
                });
            }

            let mut winner: Option<(usize, ExchangeResponse, u8)> = None;
            let mut last_error = None;
            while let Some(result) = tasks.join_next().await {
                match result {
                    Ok((entry_index, peer, transport, Ok(response))) => {
                        let Some(wire) = crate::execution::qualify_response(
                            response.wire(),
                            header.id,
                            &question,
                        ) else {
                            shared.borrow_mut().finish(
                                slots[&entry_index],
                                peer,
                                transport,
                                UpstreamAttemptOutcome::Failed,
                            );
                            last_error.get_or_insert(UpstreamError::MalformedResponse);
                            continue;
                        };
                        let priority = crate::execution::response_priority(&wire);
                        let replace = winner
                            .as_ref()
                            .is_none_or(|(_, _, current_priority)| priority < *current_priority);
                        if replace {
                            winner = Some((
                                entry_index,
                                ExchangeResponse::new(
                                    wire,
                                    response.request_id(),
                                    response.response_id(),
                                    response.transport(),
                                    response.truncated(),
                                ),
                                priority,
                            ));
                        }
                        if priority == 0 {
                            scope.cancel();
                        }
                    }
                    Ok((_entry_index, _peer, _transport, Err(error))) => {
                        last_error.get_or_insert(error);
                    }
                    Err(_) => {
                        last_error.get_or_insert(UpstreamError::Runtime(
                            mosdns_upstream_core::SideEffectState::NotSent,
                        ));
                    }
                }
            }

            for slot in slots.values() {
                if shared.borrow().slots()[*slot].outcome.is_none() {
                    shared.borrow_mut().finish(
                        *slot,
                        None,
                        None,
                        UpstreamAttemptOutcome::Interrupted,
                    );
                    last_error.get_or_insert(UpstreamError::Runtime(
                        mosdns_upstream_core::SideEffectState::NotSent,
                    ));
                }
            }

            let Some((selected_entry, response, _)) = winner else {
                *ledger = UpstreamAttemptLedger::restore_from_shared(shared);
                return Err(ExchangeError::Upstream(
                    last_error.unwrap_or(UpstreamError::MalformedResponse),
                ));
            };
            let selected_peer = shared
                .borrow()
                .slots()
                .iter()
                .find(|attempt| {
                    attempt.entry_index == selected_entry
                        && attempt.outcome == Some(UpstreamAttemptOutcome::Response)
                })
                .and_then(|attempt| attempt.peer);
            *ledger = UpstreamAttemptLedger::restore_from_shared(shared);
            Ok(InvocationExchange {
                response,
                selected_entry: Some(selected_entry),
                selected_peer,
            })
        })
    }
}

impl ForwardAdapter {
    #[cfg(test)]
    pub(crate) fn new(executable: mosdns_sequence_core::ExecutableId, endpoint: Endpoint) -> Self {
        let scheme = match endpoint.transport() {
            Transport::Udp => ForwardScheme::Udp,
            Transport::Tcp | Transport::Quic => ForwardScheme::Tcp,
        };
        Self::new_entry(
            executable,
            0,
            ForwardTargetConfig {
                scheme,
                service: endpoint.address().to_string(),
                host: endpoint.address().ip().to_string(),
                port: endpoint.address().port(),
                dial_addr: Some(endpoint.address()),
                bootstrap: None,
                bootstrap_version: None,
                query_timeout: Duration::from_secs(5),
                insecure_skip_verify: false,
            },
            None,
        )
        .expect("legacy numeric endpoint must build")
    }

    pub(crate) fn new_entry(
        executable: mosdns_sequence_core::ExecutableId,
        entry_index: usize,
        target: ForwardTargetConfig,
        tls_roots: Option<Arc<rustls::RootCertStore>>,
    ) -> Result<Self, String> {
        let legacy_upstream = target.dial_addr.map(|dial| {
            Upstream::new(
                Endpoint::new(dial, target.scheme.endpoint_transport())
                    .expect("validated numeric dial"),
            )
        });
        let owner = target
            .dial_addr
            .map(|dial| build_owner(&target, dial, tls_roots.as_deref()).map(Rc::new))
            .transpose()?;
        let resolver = if target.dial_addr.is_none() {
            let bootstrap = target
                .bootstrap
                .ok_or_else(|| "hostname forward is missing numeric bootstrap".to_owned())?;
            let version = target.bootstrap_version.unwrap_or(0);
            let mode = match version {
                0 => ResolutionMode::PreferIpv4Dual,
                4 => ResolutionMode::Ipv4,
                6 => ResolutionMode::Ipv6,
                _ => return Err("unsupported bootstrap_version".to_owned()),
            };
            let family = if mode == ResolutionMode::Ipv6 {
                AddressFamily::Ipv6
            } else {
                AddressFamily::Ipv4
            };
            let target_input = ResolutionTarget::new(&target.host, target.port, family)
                .map_err(|error| format!("invalid resolver target: {error}"))?;
            let bootstrap_input =
                BootstrapEndpoint::new(&bootstrap.ip().to_string(), bootstrap.port())
                    .map_err(|error| format!("invalid resolver bootstrap: {error}"))?;
            Some(Rc::new(
                BootstrapResolver::with_mode(
                    target_input,
                    bootstrap_input,
                    ResolutionPolicy::default(),
                    Arc::new(SystemClock),
                    mode,
                )
                .map_err(|error| format!("resolver setup failed: {error}"))?,
            ))
        } else {
            None
        };
        let initial_peer = target.dial_addr;
        Ok(Self {
            executable,
            entry_index,
            target,
            resolver,
            owner: RefCell::new(owner),
            active_peer: RefCell::new(initial_peer),
            tls_roots,
            legacy_upstream,
        })
    }

    #[must_use]
    pub const fn executable(&self) -> mosdns_sequence_core::ExecutableId {
        self.executable
    }

    #[must_use]
    pub const fn entry_index(&self) -> usize {
        self.entry_index
    }

    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        let dial = self
            .target
            .dial_addr
            .expect("resolver-backed forwards have no legacy endpoint");
        Endpoint::new(dial, self.target.scheme.endpoint_transport()).expect("validated endpoint")
    }

    #[must_use]
    pub fn upstream(&self) -> &Upstream {
        self.legacy_upstream
            .as_ref()
            .expect("resolver-backed forwards have no legacy upstream")
    }

    /// Performs one caller-owned exchange on the caller's Tokio runtime.
    /// Assembly remains pre-I/O; the request driver invokes this through the
    /// executable catalog after a listener admits a request.
    pub async fn exchange(
        &self,
        query: &[u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
    ) -> Result<ExchangeResponse, UpstreamError> {
        let shared = UpstreamAttemptLedger::default().into_shared();
        let slot = shared.borrow_mut().start(self.entry_index);
        let mut tracker = UpstreamAttemptTracker::new(shared, slot, cancellation.clone());
        self.exchange_tracked(query, deadline, cancellation, &mut tracker)
            .await
    }

    async fn exchange_tracked(
        &self,
        query: &[u8],
        deadline: std::time::Instant,
        cancellation: TransportCancellation,
        tracker: &mut UpstreamAttemptTracker,
    ) -> Result<ExchangeResponse, UpstreamError> {
        let deadline = deadline.min(std::time::Instant::now() + self.target.query_timeout);
        let owner = if let Some(resolver) = &self.resolver {
            let context = ExchangeContext::new(deadline, cancellation.clone());
            let published = resolver
                .resolve(context)
                .await
                .map_err(|_| UpstreamError::Connect)?;
            let peer = published.dial();
            let current = self.owner.borrow().clone();
            if self.active_peer.borrow().as_ref() != Some(&peer) || current.is_none() {
                let next = Rc::new(
                    build_owner(&self.target, peer, self.tls_roots.as_deref())
                        .map_err(|_| UpstreamError::Connect)?,
                );
                let _previous = self.owner.borrow_mut().replace(Rc::clone(&next));
                *self.active_peer.borrow_mut() = Some(peer);
                next
            } else {
                current.ok_or(UpstreamError::Connect)?
            }
        } else {
            self.owner.borrow().clone().ok_or(UpstreamError::Connect)?
        };
        let peer = if self.resolver.is_some() {
            *self.active_peer.borrow()
        } else {
            self.target.dial_addr
        };
        let transport = match self.target.scheme {
            ForwardScheme::Udp => UpstreamTransport::Udp,
            ForwardScheme::Tcp => UpstreamTransport::Tcp,
            ForwardScheme::Tls => UpstreamTransport::Tls,
            ForwardScheme::Https => UpstreamTransport::Https,
        };
        tracker.phase(peer, Some(transport));
        owner.exchange(query, deadline, cancellation, tracker).await
    }

    async fn close(&self) {
        let owner = self.owner.borrow_mut().take();
        if let Some(owner) = owner {
            owner.close().await;
        }
        if let Some(legacy) = &self.legacy_upstream {
            let _ = legacy.close().await;
        }
        if let Some(resolver) = &self.resolver {
            let _ = resolver.close().await;
        }
    }
}

fn build_owner(
    target: &ForwardTargetConfig,
    dial: std::net::SocketAddr,
    tls_roots: Option<&rustls::RootCertStore>,
) -> Result<ForwardOwner, String> {
    match target.scheme {
        ForwardScheme::Udp => {
            let endpoint =
                Endpoint::new(dial, Transport::Udp).map_err(|error| error.to_string())?;
            Ok(ForwardOwner::Udp {
                policy: UdpTcpPolicy::new(endpoint),
                legacy: Upstream::new(endpoint),
            })
        }
        ForwardScheme::Tcp => {
            let endpoint =
                Endpoint::new(dial, Transport::Tcp).map_err(|error| error.to_string())?;
            Ok(ForwardOwner::Tcp {
                reuse: ReuseOwner::new(endpoint),
                fallback: Upstream::new(endpoint),
            })
        }
        ForwardScheme::Tls => {
            let identity = ServerIdentity::new(&target.host).map_err(|error| error.to_string())?;
            let endpoint = DotEndpoint::new(dial, identity).map_err(|error| error.to_string())?;
            let policy = tls_policy(target.insecure_skip_verify, tls_roots)?;
            SecureReuseOwner::new(endpoint, policy)
                .map(ForwardOwner::Dot)
                .map_err(|error| error.to_string())
        }
        ForwardScheme::Https => {
            let endpoint =
                DohEndpoint::new(&target.service, dial).map_err(|error| error.to_string())?;
            let policy = tls_policy(target.insecure_skip_verify, tls_roots)?;
            DohReuseOwner::new(endpoint, policy)
                .map(ForwardOwner::Doh)
                .map_err(|error| error.to_string())
        }
    }
}

/// Failures that can occur before the native host owns a listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblyError {
    Config(ConfigError),
    Cache(CacheAdapterError),
    Catalog(String),
    Runtime(String),
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Cache(error) => write!(formatter, "cache setup failed: {error}"),
            Self::Catalog(message) => write!(formatter, "forward catalog setup failed: {message}"),
            Self::Runtime(message) => write!(formatter, "runtime setup failed: {message}"),
        }
    }
}

impl std::error::Error for AssemblyError {}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mosdns_sequence_core::ExecutableId;
    use mosdns_upstream_core::{
        Endpoint, LifecycleState, Transport, TransportCancellation, UpstreamError,
    };

    use super::{AssemblyError, ForwardCatalog, HostAssembly, HostOptions, HostRuntime};
    use crate::config::{ConfigError, ForwardConfig};

    const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

    #[test]
    fn valid_graph_is_pre_io_and_invalid_graph_never_reaches_assembly() {
        let host = HostAssembly::from_yaml(UDP).expect("valid graph must assemble");
        assert_eq!(host.config().listener.listen.port(), 15353);
        assert_eq!(
            host.endpoint().expect("primary endpoint").address().port(),
            15453
        );
        assert_eq!(
            host.forward()
                .expect("primary forward")
                .upstream()
                .lifecycle_state(),
            LifecycleState::Open
        );

        let options = HostOptions::with_deadline(Duration::from_millis(1))
            .with_cancellation(TransportCancellation::new());
        assert_eq!(options.request_deadline, Duration::from_millis(1));
        assert!(options.cancellation.is_some());

        let error = host
            .runtime()
            .block_on(host.forward().expect("primary forward").exchange(
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

    #[test]
    fn audit_retention_defaults_to_the_frozen_hundred_thousand_record_limit() {
        assert_eq!(HostOptions::default().audit_capacity, 100_000);
    }

    #[test]
    fn catalog_closes_all_distinct_forward_owners_and_rejects_duplicates() {
        let endpoint_a = Endpoint::new("127.0.0.1:1".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        let endpoint_b = Endpoint::new("127.0.0.1:2".parse().expect("endpoint"), Transport::Udp)
            .expect("endpoint");
        let configs = vec![
            ForwardConfig {
                tag: "a".to_owned(),
                upstream_tag: None,
                endpoint: endpoint_a,
                executable: ExecutableId(1),
            },
            ForwardConfig {
                tag: "b".to_owned(),
                upstream_tag: None,
                endpoint: endpoint_b,
                executable: ExecutableId(2),
            },
        ];
        let catalog = ForwardCatalog::from_configs(&configs).expect("distinct catalog");
        assert_eq!(
            catalog.forward(ExecutableId(1)).expect("a").endpoint(),
            endpoint_a
        );
        assert_eq!(
            catalog.forward(ExecutableId(2)).expect("b").endpoint(),
            endpoint_b
        );
        let duplicate = vec![configs[0].clone(), configs[0].clone()];
        assert!(ForwardCatalog::from_configs(&duplicate).is_err());

        let runtime = HostRuntime::new().expect("runtime");
        runtime.block_on(catalog.close_all());
        assert_eq!(
            catalog
                .forward(ExecutableId(1))
                .expect("a")
                .upstream()
                .lifecycle_state(),
            LifecycleState::Closed
        );
        assert_eq!(
            catalog
                .forward(ExecutableId(2))
                .expect("b")
                .upstream()
                .lifecycle_state(),
            LifecycleState::Closed
        );
    }
}
