//! Request-owned immutable graphs. Executable IDs never cross generations.
use crate::assembly::{AssemblyError, ForwardCatalog, HostOptions, build_cache_catalog};
use crate::cache::CacheCatalog;
use crate::config::{CompiledConfig, ListenerKind};
use crate::observer::QueryObserver;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Notify;

pub(crate) struct RuntimeSnapshot {
    pub config: Rc<CompiledConfig>,
    pub forwards: Rc<ForwardCatalog>,
    pub cache: Rc<CacheCatalog>,
    /// The switch owners admitted with this committed view.
    pub switches: crate::switch_state::SwitchRegistry,
    pub options: HostOptions,
    active: Cell<usize>,
    drained: Notify,
}

/// Stable host control handle; query graphs are captured at packet/frame admission.
#[derive(Clone)]
pub struct RuntimeControl(Rc<Control>);
pub(crate) type BindingKey = (String, ListenerKind, std::net::SocketAddr);
pub(crate) struct ListenerUpdate {
    pub add: Vec<(BindingKey, crate::assembly::DnsServer)>,
    pub remove: Vec<BindingKey>,
}
// One bounded update slot: a second preparation is fenced until retirement.
// Publication moves a prebuilt value into this slot without queue allocation.
struct ListenerMailbox {
    open: Cell<bool>,
    pending: RefCell<Option<ListenerUpdate>>,
    ready: Notify,
}
pub(crate) struct ListenerUpdates(Rc<ListenerMailbox>);
impl ListenerUpdates {
    pub(crate) async fn recv(&mut self) -> Option<ListenerUpdate> {
        loop {
            let ready = self.0.ready.notified();
            if let Some(update) = self.0.pending.borrow_mut().take() {
                return Some(update);
            }
            if !self.0.open.get() {
                return None;
            }
            ready.await;
        }
    }
}
impl Drop for ListenerUpdates {
    fn drop(&mut self) {
        self.0.open.set(false);
        self.0.pending.borrow_mut().take();
        self.0.ready.notify_waiters();
    }
}

fn bindings(config: &CompiledConfig) -> BTreeSet<BindingKey> {
    config
        .listeners
        .iter()
        .map(|l| (l.tag.clone(), l.kind, l.listen))
        .collect()
}
struct Control {
    updates: Rc<ListenerMailbox>,
    retiring_bindings: RefCell<BTreeSet<BindingKey>>,
    listeners_drained: Notify,
    current: RefCell<Rc<RuntimeSnapshot>>,
    retired: RefCell<Option<Rc<RuntimeSnapshot>>>,
    preparing: Cell<bool>,
    retirement_gate: tokio::sync::Mutex<()>,
    admitting: Cell<bool>,
    /// Set while only management is paused (a settings apply): DNS keeps
    /// being admitted, because the apply changes nothing a query could
    /// observe half way (each query keeps the generation it was admitted to).
    management_only: Cell<bool>,
    started: Cell<bool>,
    active_management: Cell<usize>,
    management_drained: Notify,
    recovery_required: Cell<bool>,
    closed: Cell<bool>,
    /// Deleted special-group artifacts that could not be removed yet, relative
    /// to the configuration root. Non-fatal; the next sweep retries them.
    pending_cleanup: RefCell<Vec<String>>,
    /// The receive buffer the kernel granted each live UDP listener (`None`
    /// when it could not be read); shown by the health endpoint.
    udp_receive_buffers: RefCell<BTreeMap<BindingKey, Option<usize>>>,
    /// The management listener's actual address (set when it is bound).
    api_listen: Cell<Option<std::net::SocketAddr>>,
    /// An in-place restart was scheduled / has been requested of the supervisor.
    restart_scheduled: Cell<bool>,
    restart_requested: Cell<bool>,
    /// The stop came from a termination signal, which overrides a pending restart.
    signal_stop: Cell<bool>,
    observer: Arc<QueryObserver>,
    shutdown: RefCell<Option<mosdns_upstream_core::TransportCancellation>>,
}

pub(crate) struct ManagementLease(RuntimeControl);

/// A weak control handle for switch owners. The registry must never keep the
/// committed view alive: a strong cycle would leak the managed-state writer.
#[derive(Clone)]
pub(crate) struct RuntimeControlWeak(std::rc::Weak<Control>);

impl RuntimeControlWeak {
    pub(crate) fn from_control(control: &RuntimeControl) -> Self {
        Self(Rc::downgrade(&control.0))
    }

    /// The control, unless the host has been dropped.
    pub(crate) fn upgrade(&self) -> Option<RuntimeControl> {
        self.0.upgrade().map(RuntimeControl)
    }

    /// Acquires the management lease, or `None` while admission is paused,
    /// closed, or recovering.
    pub(crate) fn begin_management_mutation(&self) -> Option<ManagementLease> {
        let control = self.0.upgrade().map(RuntimeControl)?;
        control.begin_management_mutation().ok()
    }

    /// Fences admission and stops the host after a fatal ambiguity.
    pub(crate) fn fatal_recovery(&self) {
        if let Some(control) = self.0.upgrade().map(RuntimeControl) {
            control.enter_recovery_required();
            control.stop_host();
        }
    }
}

impl Drop for ManagementLease {
    fn drop(&mut self) {
        let active = self.0.0.active_management.get();
        assert!(active > 0, "management lease count underflow");
        self.0.0.active_management.set(active - 1);
        if active == 1 {
            self.0.0.management_drained.notify_waiters();
        }
    }
}

// Wait until host shutdown is requested; an absent token cannot cancel work.
async fn wait_for_shutdown(token: Option<mosdns_upstream_core::TransportCancellation>) {
    if let Some(token) = token {
        token.cancelled().await;
    } else {
        std::future::pending::<()>().await;
    }
}

pub struct PreparedSnapshot {
    snapshot: Option<Rc<RuntimeSnapshot>>,
    owner: RuntimeControl,
    base_generation: u64,
    listener_update: Option<ListenerUpdate>,
    listeners_ready: bool,
    added_metric_labels: Vec<String>,
    periodic: Option<crate::cache::PreparedPeriodic>,
    retiring_bindings: Option<BTreeSet<BindingKey>>,
}

/// Invalidated cache dumps are emptied first, followed by optional derived
/// group YAML and the authoritative settings/group state rename.
pub(crate) struct FileCommit {
    path: std::path::PathBuf,
    /// Authority bytes when the candidate was built (`None`: absent).
    expected: Option<Vec<u8>>,
    bytes: Vec<u8>,
    cache_dumps: Vec<std::path::PathBuf>,
    derived: Option<DerivedFile>,
    inputs: crate::special_groups::CandidateInputSet,
}

struct DerivedFile {
    path: std::path::PathBuf,
    expected: Option<Vec<u8>>,
    bytes: Vec<u8>,
}

impl DerivedFile {
    fn of(config: &CompiledConfig) -> Result<Option<Self>, ApplyFailure> {
        let Some(profile) = &config.standard_groups else {
            return Ok(None);
        };
        let path = profile.base.join(crate::config::standard_groups::GENERATED);
        let expected = read_optional(&path)?;
        Ok(Some(Self {
            path,
            expected,
            bytes: profile.generated.clone(),
        }))
    }
}

fn read_optional(path: &std::path::Path) -> Result<Option<Vec<u8>>, ApplyFailure> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApplyFailure::Failed(format!("{}: {error}", path.display()))),
    }
}

impl FileCommit {
    fn write(self) -> Result<(), ApplyFailure> {
        for (path, expected) in &self.inputs.files {
            if read_optional(path)?.map(|bytes| crate::special_groups::sha256(&bytes)) != *expected
            {
                return Err(ApplyFailure::Conflict(path.clone()));
            }
        }
        if let Some(derived) = &self.derived {
            if read_optional(&derived.path)? != derived.expected {
                return Err(ApplyFailure::Conflict(derived.path.clone()));
            }
        }
        let current = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ApplyFailure::Failed(error.to_string())),
        };
        if current != self.expected {
            return Err(ApplyFailure::Conflict(self.path));
        }
        if !self.cache_dumps.is_empty() {
            let empty = crate::cache_dump::encode(&[])
                .map_err(|error| ApplyFailure::Failed(error.to_string()))?;
            for dump in &self.cache_dumps {
                write_durably(dump, &empty).map_err(|error| {
                    ApplyFailure::Failed(format!("{}: {error}", dump.display()))
                })?;
            }
        }
        if let Some(derived) = &self.derived {
            write_durably(&derived.path, &derived.bytes)
                .map_err(|error| ApplyFailure::Failed(error.to_string()))?;
        }
        if let Err(error) = write_durably(&self.path, &self.bytes) {
            if let Some(derived) = &self.derived {
                let rollback = match &derived.expected {
                    Some(bytes) => write_durably(&derived.path, bytes),
                    None => std::fs::remove_file(&derived.path),
                };
                if let Err(rollback) = rollback {
                    return Err(ApplyFailure::RecoveryRequired(format!(
                        "state commit failed: {error}; derived rollback failed: {rollback}"
                    )));
                }
            }
            return Err(ApplyFailure::Failed(format!(
                "{}: {error}",
                self.path.display()
            )));
        }
        Ok(())
    }
}

/// Replaces `path` so that after a crash it holds either the old or the new
/// bytes. The renamed file is the commit point: everything that can fail
/// happens before it (the temporary file is flushed, a directory created for
/// the first save is made durable in its parent), and flushing the directory
/// entry afterwards is best effort, because a failure there cannot be undone
/// and the running generation must agree with the file that now holds the
/// new bytes.
pub(crate) fn write_durably(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if !parent.exists() {
        std::fs::create_dir_all(parent)?;
        if let Some(grandparent) = parent
            .parent()
            .filter(|grandparent| !grandparent.as_os_str().is_empty())
        {
            std::fs::File::open(grandparent)?.sync_all()?;
        }
    }
    let mut name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("path has no file name"))?
        .to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temporary = parent.join(name);
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return written;
    }
    let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
    Ok(())
}

#[derive(Debug)]
pub(crate) enum ApplyFailure {
    Busy(String),
    Conflict(std::path::PathBuf),
    ResourceConflict(String),
    Invalid(String),
    Failed(String),
    Shutdown,
    RecoveryRequired(String),
    CommittedCleanup { generation: u64, reason: String },
}

impl std::fmt::Display for ApplyFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(reason) => write!(formatter, "managed apply is busy: {reason}"),
            Self::Conflict(path) => {
                write!(
                    formatter,
                    "managed input changed externally: {}",
                    path.display()
                )
            }
            Self::ResourceConflict(reason) => {
                write!(formatter, "managed resource conflict: {reason}")
            }
            Self::Invalid(reason) => write!(formatter, "invalid managed candidate: {reason}"),
            Self::Failed(reason) => write!(formatter, "managed apply failed: {reason}"),
            Self::Shutdown => formatter.write_str("host shutdown interrupted managed apply"),
            Self::RecoveryRequired(reason) => {
                write!(formatter, "managed state requires recovery: {reason}")
            }
            Self::CommittedCleanup { generation, reason } => write!(
                formatter,
                "generation {generation} committed, but retirement cleanup failed: {reason}"
            ),
        }
    }
}
impl Drop for PreparedSnapshot {
    fn drop(&mut self) {
        if let Some(snapshot) = &self.snapshot {
            self.owner
                .0
                .observer
                .discard_generation(snapshot.config.generation, &self.added_metric_labels);
            self.owner.0.preparing.set(false);
        }
    }
}

pub(crate) struct AdmittedSnapshot {
    pub snapshot: Rc<RuntimeSnapshot>,
    pub options: HostOptions,
    pub enable_audit: bool,
}
impl Drop for AdmittedSnapshot {
    fn drop(&mut self) {
        self.snapshot.active.set(self.snapshot.active.get() - 1);
        if self.snapshot.active.get() == 0 {
            self.snapshot.drained.notify_waiters();
        }
    }
}

impl RuntimeControl {
    pub(crate) fn new(
        config: Rc<CompiledConfig>,
        forwards: Rc<ForwardCatalog>,
        cache: Rc<CacheCatalog>,
        options: HostOptions,
        observer: Arc<QueryObserver>,
    ) -> Result<Self, AssemblyError> {
        let switches =
            crate::switch_state::SwitchRegistry::build(&config.switches).map_err(|error| {
                AssemblyError::Runtime(format!(
                    "switch `{}` state file `{}` failed admission: {}",
                    error.tag,
                    error.path.display(),
                    error.reason
                ))
            })?;
        let control = Self(Rc::new(Control {
            updates: Rc::new(ListenerMailbox {
                open: Cell::new(false),
                pending: RefCell::new(None),
                ready: Notify::new(),
            }),
            retiring_bindings: RefCell::new(BTreeSet::new()),
            listeners_drained: Notify::new(),
            current: RefCell::new(Rc::new(RuntimeSnapshot {
                config,
                forwards,
                cache,
                switches,
                options,
                active: Cell::new(0),
                drained: Notify::new(),
            })),
            retired: RefCell::new(None),
            preparing: Cell::new(false),
            retirement_gate: tokio::sync::Mutex::new(()),
            admitting: Cell::new(true),
            management_only: Cell::new(false),
            started: Cell::new(false),
            active_management: Cell::new(0),
            management_drained: Notify::new(),
            recovery_required: Cell::new(false),
            closed: Cell::new(false),
            pending_cleanup: RefCell::new(Vec::new()),
            udp_receive_buffers: RefCell::new(BTreeMap::new()),
            api_listen: Cell::new(None),
            restart_scheduled: Cell::new(false),
            restart_requested: Cell::new(false),
            signal_stop: Cell::new(false),
            observer,
            shutdown: RefCell::new(None),
        }));
        control
            .0
            .current
            .borrow()
            .switches
            .attach_control(RuntimeControlWeak::from_control(&control));
        Ok(control)
    }

    pub fn generation(&self) -> u64 {
        self.0.current.borrow().config.generation
    }

    /// Closes DNS and management admission before a durable transaction.
    pub(crate) fn pause_admission(&self) -> Result<(), AssemblyError> {
        if !self.0.admitting.get() || self.0.closed.get() || self.0.recovery_required.get() {
            return Err(AssemblyError::Runtime(
                "host admission is already paused".into(),
            ));
        }
        self.0.admitting.set(false);
        Ok(())
    }

    /// Closes management admission only; queries keep being admitted. Used by
    /// the apply of a settings change (no durable transaction around queries).
    pub(crate) fn pause_management(&self) -> Result<(), AssemblyError> {
        self.pause_admission()?;
        self.0.management_only.set(true);
        Ok(())
    }

    /// Reopens admission after an ordinary rollback.
    pub(crate) fn resume_admission(&self) {
        self.0.management_only.set(false);
        if !self.0.closed.get()
            && !self.0.recovery_required.get()
            && !self
                .0
                .shutdown
                .borrow()
                .as_ref()
                .is_some_and(mosdns_upstream_core::TransportCancellation::is_cancelled)
        {
            self.0.admitting.set(true);
        }
    }

    /// One coherent lifecycle observation from the supervisor and its admission owner.
    pub(crate) fn lifecycle_state(&self) -> &'static str {
        if self.0.closed.get() {
            "closed"
        } else if self.0.recovery_required.get() {
            "recovery_required"
        } else if self
            .0
            .shutdown
            .borrow()
            .as_ref()
            .is_some_and(mosdns_upstream_core::TransportCancellation::is_cancelled)
        {
            "stopping"
        } else if !self.0.started.get() {
            "starting"
        } else if !self.admission_open() {
            "applying"
        } else {
            "ready"
        }
    }

    pub(crate) fn admission_open(&self) -> bool {
        self.0.admitting.get() && !self.0.closed.get() && !self.0.recovery_required.get()
    }

    pub(crate) fn begin_management_mutation(&self) -> Result<ManagementLease, AssemblyError> {
        if !self.admission_open() {
            return Err(AssemblyError::Runtime(
                "management admission is paused".into(),
            ));
        }
        let active = self
            .0
            .active_management
            .get()
            .checked_add(1)
            .ok_or_else(|| AssemblyError::Runtime("management lease count exhausted".into()))?;
        self.0.active_management.set(active);
        Ok(ManagementLease(self.clone()))
    }

    /// Waits for previously admitted cache-management requests to finish.
    async fn wait_management_drain(&self) {
        loop {
            let notified = self.0.management_drained.notified();
            if self.0.active_management.get() == 0 {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn enter_recovery_required(&self) {
        self.0.recovery_required.set(true);
        self.0.admitting.set(false);
    }

    pub(crate) fn recovery_required(&self) -> bool {
        self.0.recovery_required.get()
    }

    /// Replaces the list of deleted-group artifacts still waiting to be removed
    /// (relative to the configuration root; empty when nothing is pending).
    pub(crate) fn set_pending_cleanup(&self, pending: Vec<String>) {
        *self.0.pending_cleanup.borrow_mut() = pending;
    }

    pub(crate) fn pending_cleanup(&self) -> Vec<String> {
        self.0.pending_cleanup.borrow().clone()
    }

    /// Records the receive buffer a freshly bound UDP listener was granted.
    pub(crate) fn set_udp_receive_buffer(&self, key: BindingKey, bytes: Option<usize>) {
        self.0.udp_receive_buffers.borrow_mut().insert(key, bytes);
    }

    pub(crate) fn set_api_listen(&self, address: std::net::SocketAddr) {
        self.0.api_listen.set(Some(address));
    }

    /// The management listener's actual address, once bound.
    pub(crate) fn api_listen(&self) -> Option<std::net::SocketAddr> {
        self.0.api_listen.get()
    }

    /// Schedules one in-place restart after `delay`; false when one is already scheduled.
    /// The restart itself is the supervisor's ordinary graceful stop, after which `main`
    /// replaces the process image (see `restart.rs`).
    pub(crate) fn schedule_restart(&self, delay: std::time::Duration) -> bool {
        if self.0.restart_scheduled.replace(true) {
            return false;
        }
        let control = self.clone();
        tokio::task::spawn_local(async move {
            tokio::time::sleep(delay).await;
            control.0.restart_requested.set(true);
            control.stop_host();
        });
        true
    }

    /// Records that a termination signal started the shutdown.
    pub(crate) fn note_signal_stop(&self) {
        self.0.signal_stop.set(true);
    }

    /// Whether the host stopped to be restarted in place (a signal always wins).
    #[must_use]
    pub fn restart_requested(&self) -> bool {
        self.0.restart_requested.get() && !self.0.signal_stop.get()
    }

    /// Forgets a UDP listener that is gone (served to the end, or staged and dropped unused).
    pub(crate) fn forget_udp_receive_buffer(&self, key: &BindingKey) {
        self.0.udp_receive_buffers.borrow_mut().remove(key);
    }

    /// `(listen address, granted bytes)` of every live UDP listener, in address order.
    pub(crate) fn udp_receive_buffers(&self) -> Vec<(std::net::SocketAddr, Option<usize>)> {
        let mut buffers: Vec<_> = self
            .0
            .udp_receive_buffers
            .borrow()
            .iter()
            .map(|((_, _, address), bytes)| (*address, *bytes))
            .collect();
        buffers.sort();
        buffers
    }

    pub(crate) fn stop_host(&self) {
        if let Some(shutdown) = self.0.shutdown.borrow().as_ref() {
            shutdown.cancel();
        }
    }

    /// Pre-I/O graph preparation. Changed bindings additionally require
    /// `prepare_host` to stage sockets before publication.
    pub fn prepare(&self, mut config: CompiledConfig) -> Result<PreparedSnapshot, AssemblyError> {
        if self.0.closed.get() || self.0.preparing.get() || self.0.retired.borrow().is_some() {
            return Err(AssemblyError::Runtime(
                "snapshot preparation is closed or busy".into(),
            ));
        }
        let current = self.0.current.borrow().clone();
        match (&current.config.standard_groups, &config.standard_groups) {
            (Some(old), Some(next)) if old.base == next.base => {}
            (None, None) => {}
            _ => {
                return Err(AssemblyError::Runtime(
                    "standard special-group root must remain frozen".into(),
                ));
            }
        }
        if config.api != current.config.api {
            return Err(AssemblyError::Runtime("API bind must remain stable".into()));
        }
        let listeners_ready = bindings(&config) == bindings(&current.config);
        config.generation = current
            .config
            .generation
            .checked_add(1)
            .ok_or_else(|| AssemblyError::Runtime("snapshot generation exhausted".into()))?;
        config.rule_sources.configure(
            current.options.tls_roots.clone(),
            current.options.rule_timing.clone(),
        );
        let config = Rc::new(config);
        let forwards = Rc::new(
            ForwardCatalog::from_compiled_reusing(
                &config,
                current.options.tls_roots.clone(),
                Some((&current.config, &current.forwards)),
            )
            .map_err(AssemblyError::Catalog)?,
        );
        let fresh_cache = build_cache_catalog(&config, &current.options)?;
        let cache = Rc::new(fresh_cache.reuse_matching(&current.cache, &current.config, &config));
        let mut options = current.options.clone();
        options.entry_sequence = None;
        options.refresh_environment = Some(Rc::new(crate::execution::RefreshEnvironment {
            config: config.clone(),
            forwards: forwards.clone(),
            cache: Rc::downgrade(&cache),
            observer: self.0.observer.clone(),
        }));
        // Unchanged switch owners carry over as live objects so their values
        // stay whatever the latest publication left; new or moved owners read
        // their bounded file here and abort the candidate on failure.
        let switches =
            crate::switch_state::SwitchRegistry::rebind(Some(&current.switches), &config.switches)
                .map_err(|error| {
                    AssemblyError::Runtime(format!(
                        "switch `{}` state file `{}` failed candidate admission: {}",
                        error.tag,
                        error.path.display(),
                        error.reason
                    ))
                })?;
        // Allocate the generation-qualified registry before publication.
        let added_metric_labels = self.0.observer.register_generation(&config);
        let snapshot = Rc::new(RuntimeSnapshot {
            config,
            forwards,
            cache,
            switches,
            options,
            active: Cell::new(0),
            drained: Notify::new(),
        });
        let periodic = self
            .0
            .shutdown
            .borrow()
            .as_ref()
            .map(|shutdown| snapshot.cache.prepare_periodic(shutdown));
        self.0.preparing.set(true);
        Ok(PreparedSnapshot {
            snapshot: Some(snapshot),
            owner: self.clone(),
            base_generation: current.config.generation,
            listener_update: None,
            listeners_ready,
            added_metric_labels,
            periodic,
            retiring_bindings: None,
        })
    }

    /// Prebind both transport halves before publication. Failed preparation
    /// drops every candidate socket; existing accept loops are untouched.
    pub async fn prepare_host(
        &self,
        config: CompiledConfig,
    ) -> Result<PreparedSnapshot, AssemblyError> {
        let mut prepared = self.prepare(config)?;
        if prepared.listeners_ready {
            return Ok(prepared);
        }
        if !self.0.updates.open.get() {
            return Err(AssemblyError::Runtime(
                "listener supervisor is not running".into(),
            ));
        }
        let previous = bindings(&self.0.current.borrow().config);
        let next = prepared.snapshot.as_ref().expect("prepared graph");
        let next_bindings = bindings(&next.config);
        let mut add = Vec::new();
        for listener in &next.config.listeners {
            let key = (listener.tag.clone(), listener.kind, listener.listen);
            if previous.contains(&key) {
                continue;
            }
            let server = match listener.kind {
                ListenerKind::Udp => crate::assembly::DnsServer::Udp(
                    crate::udp::UdpServer::bind_snapshot(
                        self.clone(),
                        next,
                        self.0.observer.clone(),
                        listener,
                    )
                    .await
                    .map_err(|error| match error {
                        crate::udp::UdpServerError::Bind(source)
                            if source.kind() == std::io::ErrorKind::AddrInUse =>
                        {
                            AssemblyError::ListenerConflict(format!(
                                "UDP listener {} is already in use: {source}",
                                listener.listen
                            ))
                        }
                        error => AssemblyError::Runtime(error.to_string()),
                    })?,
                ),
                ListenerKind::Tcp => crate::assembly::DnsServer::Tcp(
                    crate::tcp::TcpServer::bind_snapshot(
                        self.clone(),
                        next,
                        self.0.observer.clone(),
                        listener,
                    )
                    .await
                    .map_err(|error| match error {
                        crate::tcp::TcpServerError::Bind(source)
                            if source.kind() == std::io::ErrorKind::AddrInUse =>
                        {
                            AssemblyError::ListenerConflict(format!(
                                "TCP listener {} is already in use: {source}",
                                listener.listen
                            ))
                        }
                        error => AssemblyError::Runtime(error.to_string()),
                    })?,
                ),
            };
            add.push((key, server));
        }
        prepared.retiring_bindings = Some(previous.difference(&next_bindings).cloned().collect());
        prepared.listener_update = Some(ListenerUpdate {
            add,
            remove: previous.difference(&next_bindings).cloned().collect(),
        });
        prepared.listeners_ready = true;
        Ok(prepared)
    }

    /// No await during publication; staged graph and ID tables already exist.
    pub(crate) fn validate_install(
        &self,
        prepared: &PreparedSnapshot,
    ) -> Result<(), AssemblyError> {
        if !prepared.listeners_ready
            || self
                .0
                .shutdown
                .borrow()
                .as_ref()
                .is_some_and(|s| s.is_cancelled())
            || !Rc::ptr_eq(&self.0, &prepared.owner.0)
            || self.0.closed.get()
            || self.generation() != prepared.base_generation
        {
            return Err(AssemblyError::Runtime(
                "stale or foreign prepared snapshot".into(),
            ));
        }
        if prepared.listener_update.is_some()
            && (!self.0.updates.open.get() || self.0.updates.pending.borrow().is_some())
        {
            return Err(AssemblyError::Runtime(
                "listener supervisor is closed".into(),
            ));
        }
        Ok(())
    }

    /// The post-marker publication path. All allocation, resource construction,
    /// binding and fallible validation happened before the durable marker.
    fn install_committed(&self, mut prepared: PreparedSnapshot) -> (u64, bool) {
        assert!(
            prepared.listeners_ready,
            "validated snapshot is listener-ready"
        );
        assert!(
            Rc::ptr_eq(&self.0, &prepared.owner.0),
            "validated snapshot owner"
        );
        assert_eq!(
            self.generation(),
            prepared.base_generation,
            "validated base generation"
        );
        let next = prepared
            .snapshot
            .take()
            .expect("prepared graph is consumed once");
        let generation = next.config.generation;
        // Carried switch owners hold the latest surviving values; the
        // aggregate is rebuilt synchronously here, after every accepted
        // mutation of the previous view has drained.
        next.switches.refresh_aggregate();
        next.switches
            .attach_control(RuntimeControlWeak::from_control(&self.clone()));
        self.0.observer.enable_snapshot_audit(&next.config);
        if let Some(periodic) = &prepared.periodic {
            periodic.activate();
        }
        let previous = self.0.current.replace(next);
        let live = self.0.current.borrow().clone();
        previous.cache.fence_exclusive_to(&live.cache);
        // The retired view stops writing rule sources before the new view
        // admits anything; both share each file's write lock.
        previous.config.rule_sources.fence();
        live.config.rule_sources.start();
        // The retired view saves its statistics before the new view loads them.
        previous.config.local.fence();
        live.config.local.start(&crate::local_url::LocalApi {
            control: RuntimeControlWeak::from_control(&self.clone()),
            api: live.config.api.as_ref().map(|api| api.http),
            roots: live.config.rule_sources.roots(),
        });
        assert!(
            self.0.retired.borrow_mut().replace(previous).is_none(),
            "only one retired snapshot"
        );
        let mut listener_published = true;
        if let Some(update) = prepared.listener_update.take() {
            let mut pending = self.0.updates.pending.borrow_mut();
            if self.0.updates.open.get() && pending.is_none() {
                *self.0.retiring_bindings.borrow_mut() = prepared
                    .retiring_bindings
                    .take()
                    .expect("prepared retirement set");
                *pending = Some(update);
                self.0.updates.ready.notify_one();
            } else {
                // The durable config is authoritative now. A closing listener
                // supervisor cannot accept a new binding, so stop and let
                // startup recovery rebuild sockets from the committed files.
                listener_published = false;
            }
        }
        self.0.preparing.set(false);
        (generation, listener_published)
    }

    pub fn install(&self, prepared: PreparedSnapshot) -> Result<u64, AssemblyError> {
        self.validate_install(&prepared)?;
        // Pending switch owners are admitted before publication; the
        // production path does this after the management drain in
        // `apply_candidate`.
        if let Err(error) = prepared
            .snapshot
            .as_ref()
            .expect("prepared graph")
            .switches
            .complete_pending_admissions()
        {
            return Err(AssemblyError::Runtime(format!(
                "switch `{}` state file `{}` failed candidate admission: {}",
                error.tag,
                error.path.display(),
                error.reason
            )));
        }
        let (generation, listener_published) = self.install_committed(prepared);
        if !listener_published {
            self.enter_recovery_required();
            self.stop_host();
            return Err(AssemblyError::Runtime(
                "listener supervisor closed during publication".into(),
            ));
        }
        Ok(generation)
    }

    fn shutdown_token(&self) -> Option<mosdns_upstream_core::TransportCancellation> {
        self.0.shutdown.borrow().clone()
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown_token()
            .is_some_and(|shutdown| shutdown.is_cancelled())
    }

    async fn abort_precommit(
        &self,
        original: ApplyFailure,
        prepared: PreparedSnapshot,
        cache_gate: crate::cache::PolicyTransactionGate,
    ) -> ApplyFailure {
        // Nothing durable was staged: only admission needs to resume.
        drop(prepared);
        drop(cache_gate);
        self.resume_admission();
        original
    }

    /// Recompiles the running configuration with `settings` and publishes it as
    /// the next generation; the settings file is written at the commit point
    /// (see `docs/rust/contracts/native-settings.md`). Nothing changes on failure.
    pub(crate) async fn apply_settings(
        &self,
        settings: crate::config::settings::Settings,
    ) -> Result<u64, ApplyFailure> {
        let reload = self.0.current.borrow().config.reload.clone();
        let Some(reload) = reload else {
            return Err(ApplyFailure::Invalid(
                "this configuration was not loaded from a file".into(),
            ));
        };
        let path = crate::config::settings::Settings::path(&reload.root_dir);
        let expected = read_optional(&path)?;
        let config = crate::config::recompile(&reload, &settings)
            .map_err(|error| ApplyFailure::Invalid(error.to_string()))?;
        let commit = FileCommit {
            path,
            expected,
            bytes: settings.to_bytes(),
            cache_dumps: Vec::new(),
            derived: DerivedFile::of(&config)?,
            inputs: config.compile_inputs.clone(),
        };
        self.apply_config(config, Some(commit)).await
    }

    pub(crate) async fn apply_groups(
        &self,
        groups: Vec<crate::config::standard_groups::Group>,
    ) -> Result<u64, ApplyFailure> {
        let current = self.view();
        let profile = current.config.standard_groups.as_ref().ok_or_else(|| {
            ApplyFailure::Invalid("standard special groups are unavailable".into())
        })?;
        let reload = current
            .config
            .reload
            .as_ref()
            .ok_or_else(|| ApplyFailure::Invalid("file configuration is required".into()))?;
        let expected = read_optional(&profile.base.join(crate::config::standard_groups::STATE))?;
        if expected != profile.expected_state {
            return Err(ApplyFailure::Conflict(
                profile.base.join(crate::config::standard_groups::STATE),
            ));
        }
        let mut config = crate::config::recompile_groups(reload, &current.config.settings, &groups)
            .map_err(|error| ApplyFailure::Invalid(error.to_string()))?;
        let bytes = crate::config::standard_groups::state_bytes(
            &config
                .standard_groups
                .as_ref()
                .expect("candidate groups")
                .groups,
        );
        let commit = FileCommit {
            path: profile.base.join(crate::config::standard_groups::STATE),
            expected,
            bytes: bytes.clone(),
            cache_dumps: Vec::new(),
            derived: DerivedFile::of(&config)?,
            inputs: config.compile_inputs.clone(),
        };
        config
            .standard_groups
            .as_mut()
            .expect("candidate groups")
            .expected_state = Some(bytes);
        drop(current);
        self.apply_config(config, Some(commit)).await
    }

    /// The shared whole-generation publication path; `files` are the plain files written at
    /// the commit point.
    async fn apply_config(
        &self,
        config: crate::config::CompiledConfig,
        mut files: Option<FileCommit>,
    ) -> Result<u64, ApplyFailure> {
        let cache_dump_replacements = {
            let current = self.0.current.borrow();
            CacheCatalog::invalidated_dump_paths(&current.config, &config)
        };
        if let Some(commit) = &mut files {
            commit.cache_dumps = cache_dump_replacements;
        }
        self.generation()
            .checked_add(1)
            .ok_or_else(|| ApplyFailure::Invalid("generation exhausted".into()))?;
        let prepared = self
            .prepare_host(config)
            .await
            .map_err(|error| match error {
                AssemblyError::ListenerConflict(reason) => ApplyFailure::ResourceConflict(reason),
                error => ApplyFailure::Invalid(error.to_string()),
            })?;
        self.validate_install(&prepared)
            .map_err(|error| ApplyFailure::Busy(error.to_string()))?;

        if let Err(error) = self.pause_management() {
            drop(prepared);
            return Err(ApplyFailure::Busy(error.to_string()));
        }

        tokio::select! {
            biased;
            () = wait_for_shutdown(self.shutdown_token()) => {
                drop(prepared);
                self.resume_admission();
                return Err(ApplyFailure::Shutdown);
            }
            () = self.wait_management_drain() => {},
        }
        if self.shutdown_requested() {
            drop(prepared);
            self.resume_admission();
            return Err(ApplyFailure::Shutdown);
        }
        let previous = self.0.current.borrow().clone();
        prepared
            .snapshot
            .as_ref()
            .expect("prepared graph")
            .switches
            .rebase_carried_values(&previous.switches);
        // The old generation must be freed by retirement, not by the end of this function.
        drop(previous);
        // Pending switch owners are admitted only now, after every accepted
        // mutation of the previous view drained, so a late old-owner write
        // cannot be overwritten by a stale preparation-time read.
        if let Err(error) = prepared
            .snapshot
            .as_ref()
            .expect("prepared graph")
            .switches
            .complete_pending_admissions()
        {
            drop(prepared);
            self.resume_admission();
            return Err(ApplyFailure::Invalid(format!(
                "switch `{}` state file `{}` failed candidate admission: {}",
                error.tag,
                error.path.display(),
                error.reason
            )));
        }

        let old_cache = self.0.current.borrow().cache.clone();
        let cache_gate = tokio::select! {
            biased;
            () = wait_for_shutdown(self.shutdown_token()) => {
                drop(prepared);
                self.resume_admission();
                return Err(ApplyFailure::Shutdown);
            }
            gate = old_cache.acquire_policy_transaction_gate() => gate,
        };

        if self.shutdown_requested() {
            drop(cache_gate);
            drop(prepared);
            self.resume_admission();
            return Err(ApplyFailure::Shutdown);
        }
        drop(old_cache);
        if self.shutdown_requested() {
            return Err(self
                .abort_precommit(ApplyFailure::Shutdown, prepared, cache_gate)
                .await);
        }

        if self.shutdown_requested() {
            return Err(self
                .abort_precommit(ApplyFailure::Shutdown, prepared, cache_gate)
                .await);
        }
        if let Err(error) = self.validate_install(&prepared) {
            return Err(self
                .abort_precommit(ApplyFailure::Busy(error.to_string()), prepared, cache_gate)
                .await);
        }

        if self.shutdown_requested() {
            return Err(self
                .abort_precommit(ApplyFailure::Shutdown, prepared, cache_gate)
                .await);
        }

        if let Some(commit) = files.take() {
            // The commit point of a settings apply: the invalidated cache dumps,
            // then the settings file. A failure here publishes nothing.
            let outcome = tokio::task::spawn_blocking(move || commit.write())
                .await
                .map_err(|error| ApplyFailure::Failed(error.to_string()))
                .and_then(|outcome| outcome);
            if let Err(failure) = outcome {
                if matches!(failure, ApplyFailure::RecoveryRequired(_)) {
                    self.enter_recovery_required();
                    self.stop_host();
                    return Err(failure);
                }
                return Err(self.abort_precommit(failure, prepared, cache_gate).await);
            }
        }

        // A panic here is an invariant failure after the point of no return.
        let publication = catch_unwind(AssertUnwindSafe(|| self.install_committed(prepared)));
        let (generation, listener_published) = match publication {
            Ok(value) => value,
            Err(_) => {
                self.enter_recovery_required();
                self.stop_host();
                drop(cache_gate);
                return Err(ApplyFailure::RecoveryRequired(
                    "runtime publication panicked after the durable marker".into(),
                ));
            }
        };
        if !listener_published {
            self.enter_recovery_required();
            self.stop_host();
            drop(cache_gate);
            return Err(ApplyFailure::RecoveryRequired(
                "listener supervisor closed after the durable marker".into(),
            ));
        }

        self.resume_admission();
        drop(cache_gate);
        if let Err(error) = self.retire().await {
            self.enter_recovery_required();
            self.stop_host();
            return Err(ApplyFailure::CommittedCleanup {
                generation,
                reason: error.to_string(),
            });
        }
        // Artifacts of deleted groups are removed after publication. The
        // committed generation is correct without that, so a failure is only
        // reported (log and health) and retried by the next sweep.
        let pending = self
            .view()
            .config
            .standard_groups
            .as_ref()
            .map(crate::config::standard_groups::Profile::sweep_orphans);
        if let Some(pending) = pending {
            self.set_pending_cleanup(pending);
        }
        // The retired generation is freed once the last query admitted to it has
        // finished; hand its pages back right away and again after those queries
        // are certainly done (Mem-1).
        let _ = crate::rule_sources::on_build_thread(crate::memory::release_free_memory).await;
        let shutdown = self.shutdown_token();
        tokio::task::spawn_local(async move {
            tokio::select! {
                biased;
                () = wait_for_shutdown(shutdown) => {}
                () = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                    let _ = crate::rule_sources::on_build_thread(crate::memory::release_free_memory).await;
                }
            }
        });

        Ok(generation)
    }

    /// Bounded old request deadlines drain before a second preparation. A
    /// canceled join leaves retirement registered for supervisor shutdown/retry.
    pub async fn retire(&self) -> Result<(), AssemblyError> {
        let _gate = self.0.retirement_gate.lock().await;
        let retired = self.0.retired.borrow().clone();
        if let Some(old) = retired {
            while old.active.get() != 0 {
                let notified = old.drained.notified();
                if old.active.get() == 0 {
                    break;
                }
                notified.await;
            }
            while !self.0.retiring_bindings.borrow().is_empty() {
                let notified = self.0.listeners_drained.notified();
                if self.0.retiring_bindings.borrow().is_empty() {
                    break;
                }
                notified.await;
            }
            let live = self.0.current.borrow().clone();
            // The retired view's rule-source jobs stop before it is dropped;
            // the committed view re-read the manifests when it compiled.
            old.config.local.close().await;
            old.config.rule_sources.close().await;
            let failure = old.cache.retire_exclusive_to(&live.cache).await.err();
            old.forwards.close_exclusive_to(&live.forwards).await;
            self.0.observer.retire_generation(old.config.generation);
            self.0.retired.borrow_mut().take();
            if let Some(error) = failure {
                return Err(AssemblyError::Cache(error));
            }
        }
        Ok(())
    }

    pub(crate) fn view(&self) -> Rc<RuntimeSnapshot> {
        self.0.current.borrow().clone()
    }

    pub(crate) fn capture(
        &self,
        tag: &str,
        kind: ListenerKind,
        address: std::net::SocketAddr,
    ) -> Option<AdmittedSnapshot> {
        let dns_open = (self.0.admitting.get() || self.0.management_only.get())
            && !self.0.closed.get()
            && !self.0.recovery_required.get();
        if !dns_open {
            return None;
        }
        let snapshot = self.0.current.borrow().clone();
        let binding = snapshot
            .config
            .listeners
            .iter()
            .find(|l| l.tag == tag && l.kind == kind && l.listen == address)?;
        let mut options = snapshot.options.clone();
        options.entry_sequence = Some(
            snapshot
                .config
                .program
                .sequences
                .iter()
                .find(|s| !s.synthetic && s.name == binding.entry)?
                .id,
        );
        let enable_audit = binding.enable_audit;
        snapshot.active.set(snapshot.active.get() + 1);
        Some(AdmittedSnapshot {
            snapshot,
            options,
            enable_audit,
        })
    }

    pub(crate) fn start(
        &self,
        shutdown: &mosdns_upstream_core::TransportCancellation,
    ) -> ListenerUpdates {
        *self.0.shutdown.borrow_mut() = Some(shutdown.clone());
        self.0.started.set(true);
        self.0.current.borrow().cache.start_periodic(shutdown);
        self.0.updates.open.set(true);
        ListenerUpdates(self.0.updates.clone())
    }

    pub(crate) fn listener_finished(&self, key: &BindingKey) {
        if self.0.retiring_bindings.borrow_mut().remove(key) {
            self.0.listeners_drained.notify_waiters();
        }
    }

    pub(crate) async fn close(&self) -> Result<(), AssemblyError> {
        self.0.closed.set(true);
        self.0.admitting.set(false);
        self.0.current.borrow().switches.close();
        self.0.updates.open.set(false);
        self.0.updates.pending.borrow_mut().take();
        self.0.updates.ready.notify_waiters();
        self.0.retiring_bindings.borrow_mut().clear();
        self.0.listeners_drained.notify_waiters();
        let mut failure = self.retire().await.err();
        let current = self.0.current.borrow().clone();
        current.cache.stop_admission();
        if let Err(error) = current.cache.stop_refreshes().await {
            failure.get_or_insert(AssemblyError::Cache(error));
        }
        if let Err(error) = current.cache.finish_persistence().await {
            failure.get_or_insert(AssemblyError::Cache(error));
        }
        // Rule-source jobs: downloads stop at once, a running build is awaited.
        current.config.local.close().await;
        current.config.rule_sources.close().await;
        current.forwards.close_all().await;
        // Accepted switch mutations complete their durable write and
        // publication even during shutdown.
        current.switches.drain().await;
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod admission_tests {
    use std::rc::Rc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn apply_admission_gate_closes_and_recovers_without_touching_the_snapshot() {
        let config = crate::compile_yaml(include_str!(
            "../../../tests/phase5a-baseline/configs/forward-udp.yaml"
        ))
        .unwrap();
        let host = crate::HostAssembly::from_config(config).unwrap();
        host.block_on(async {
            let control = host.control();
            let initial = control.view();
            let listener = &initial.config.listeners[0];
            assert!(
                control
                    .capture(&listener.tag, listener.kind, listener.listen)
                    .is_some()
            );
            control.pause_admission().unwrap();
            assert!(!control.admission_open());
            assert!(
                control
                    .capture(&listener.tag, listener.kind, listener.listen)
                    .is_none()
            );
            assert!(Rc::ptr_eq(&initial, &control.view()));
            control.resume_admission();
            assert!(control.admission_open());
            assert!(
                control
                    .capture(&listener.tag, listener.kind, listener.listen)
                    .is_some()
            );
            control.enter_recovery_required();
            assert!(!control.admission_open());
            control.resume_admission();
            assert!(!control.admission_open());
            control.close().await.unwrap();
        });
    }

    fn free_pair() -> u16 {
        loop {
            let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = tcp.local_addr().unwrap().port();
            if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
                return port;
            }
        }
    }

    #[test]
    fn publication_rejects_removed_bindings_before_supervisor_cancellation() {
        let root =
            std::env::temp_dir().join(format!("native-removed-admission-{}", std::process::id()));
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        let old_port = free_pair();
        let path = root.join("config.yaml");
        let main_port = free_pair();
        // `main` plus an extra UDP/TCP pair on `port`; moving the pair removes the old bindings.
        let write_pair = |port: u16| {
            std::fs::write(&path, format!("log: {{level: error}}\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:15999'}}]}}\n  - tag: entry\n    type: sequence\n    args: [{{exec: reject 3}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{main_port}', enable_audit: true}}\n  - tag: extra_udp\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{port}', enable_audit: true}}\n  - tag: extra_tcp\n    type: tcp_server\n    args: {{entry: entry, listen: '127.0.0.1:{port}', enable_audit: true}}\n")).unwrap()
        };
        write_pair(old_port);
        let host =
            crate::HostAssembly::from_config(crate::load_and_compile(&path).unwrap()).unwrap();
        host.block_on(async {
            let control = host.control();
            let stop = mosdns_upstream_core::TransportCancellation::new();
            let updates = control.start(&stop);
            // Hold the mailbox unpolled to expose the publication/cancel window.
            let old_config = control.view().config.clone();
            let old_bindings: Vec<_> = old_config.listeners.iter().filter(|l| l.listen.port()==old_port).cloned().collect();
            let mut servers = tokio::task::JoinSet::new();
            for binding in &old_bindings {
                let scope = stop.child_token();
                match binding.kind {
                    crate::config::ListenerKind::Udp => {
                        let server = crate::udp::UdpServer::bind_listener(&host,binding,binding.listen,true).await.unwrap();
                        servers.spawn_local(async move { server.serve(scope).await.map_err(|e|e.to_string()) });
                    }
                    crate::config::ListenerKind::Tcp => {
                        let server = crate::tcp::TcpServer::bind_listener(&host,binding,binding.listen,true).await.unwrap();
                        servers.spawn_local(async move { server.serve(scope).await.map_err(|e|e.to_string()) });
                    }
                }
            }
            let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1",old_port)).await.unwrap();
            let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let in_flight: Vec<_> = old_bindings.iter().map(|l| control.capture(&l.tag,l.kind,l.listen).unwrap()).collect();
            write_pair(free_pair());
            let prepared = control.prepare_host(crate::load_and_compile(&path).unwrap()).await.unwrap();
            control.install(prepared).unwrap();
            for binding in &old_bindings {
                assert!(control.capture(&binding.tag,binding.kind,binding.listen).is_none(),"old socket must not capture a new-generation datagram/frame before asynchronous cancellation");
            }
            let query = [1,2,1,0,0,1,0,0,0,0,0,0,1,b'x',0,0,1,0,1];
            udp.send_to(&query,("127.0.0.1",old_port)).await.unwrap();
            tcp.write_all(&u16::try_from(query.len()).unwrap().to_be_bytes()).await.unwrap();
            tcp.write_all(&query).await.unwrap();
            assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1),tcp.read(&mut [0;1])).await.unwrap().unwrap(),0,"old TCP frame must close without execution");
            assert!(tokio::time::timeout(std::time::Duration::from_millis(100),udp.recv_from(&mut [0;512])).await.is_err(),"old UDP binding must not answer");
            assert_eq!(host.metrics_snapshot().admitted_total,0,"neither removed binding may execute the newly received DNS message");
            assert!(in_flight.iter().all(|lease| lease.snapshot.config.generation==0));
            stop.cancel();
            while let Some(result) = servers.join_next().await { result.unwrap().unwrap(); }
            drop(in_flight);
            // The held receiver owns staged sockets; no candidate accept loop ran.
            drop(updates);
            for binding in old_bindings { control.listener_finished(&(binding.tag,binding.kind,binding.listen)); }
            control.close().await.unwrap();
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_failures_still_close_retired_and_current_transports() {
        let root =
            std::env::temp_dir().join(format!("native-failed-cleanup-{}", std::process::id()));
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        let path = root.join("config.yaml");
        let main_port = free_pair();
        let write_config = |upstream: u16| {
            std::fs::write(&path, format!("log: {{level: error}}\nplugins:\n  - tag: cache\n    type: cache\n    args: {{dump_file: cache/c.dump}}\n  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{upstream}'}}]}}\n  - tag: entry\n    type: sequence\n    args: [{{exec: $cache}}, {{exec: $fwd}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{main_port}', enable_audit: true}}\n")).unwrap()
        };
        write_config(15999);
        let host =
            crate::HostAssembly::from_config(crate::load_and_compile(&path).unwrap()).unwrap();
        host.block_on(async {
            let control = host.control();
            let old = control.view();
            write_config(16000);
            let prepared = control
                .prepare(crate::load_and_compile(&path).unwrap())
                .unwrap();
            control.install(prepared).unwrap();
            let current = control.view();
            old.cache
                .get(crate::config::CacheId(0))
                .unwrap()
                .inject_refresh_join_failure();
            current
                .cache
                .get(crate::config::CacheId(0))
                .unwrap()
                .inject_persist_fault(crate::managed::PersistFault::Rename);
            assert!(
                control.close().await.is_err(),
                "cleanup errors must remain observable"
            );
            for snapshot in [old, current] {
                let executable = snapshot.config.forward_invocations[0].executable;
                let owner = snapshot.forwards.forward(executable).unwrap();
                assert_eq!(
                    owner.upstream().lifecycle_state(),
                    mosdns_upstream_core::LifecycleState::Closed,
                    "both generations must close transports despite cache cleanup failures"
                );
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod file_commit_tests {
    use super::{ApplyFailure, FileCommit};

    fn commit(dir: &std::path::Path, expected: Option<&[u8]>) -> FileCommit {
        FileCommit {
            path: dir.join("webinfo/settings.json"),
            expected: expected.map(<[u8]>::to_vec),
            bytes: b"new".to_vec(),
            cache_dumps: vec![dir.join("cache/a.gz")],
            derived: None,
            inputs: crate::special_groups::CandidateInputSet::default(),
        }
    }

    #[test]
    fn the_commit_writes_dumps_and_settings_only_when_the_file_is_unchanged() {
        let dir = std::env::temp_dir().join(format!("file-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Absent as expected: the dumps are emptied, then the file appears.
        commit(&dir, None).write().unwrap();
        assert_eq!(
            std::fs::read(dir.join("webinfo/settings.json")).unwrap(),
            b"new"
        );
        assert!(dir.join("cache/a.gz").exists());
        // The file changed after the candidate was built: nothing is written.
        std::fs::write(dir.join("webinfo/settings.json"), b"edited").unwrap();
        std::fs::remove_file(dir.join("cache/a.gz")).unwrap();
        let outcome = commit(&dir, Some(b"new")).write();
        assert!(matches!(outcome, Err(ApplyFailure::Conflict(_))));
        assert_eq!(
            std::fs::read(dir.join("webinfo/settings.json")).unwrap(),
            b"edited"
        );
        assert!(!dir.join("cache/a.gz").exists());
        // Matching bytes: written.
        commit(&dir, Some(b"edited")).write().unwrap();
        assert_eq!(
            std::fs::read(dir.join("webinfo/settings.json")).unwrap(),
            b"new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
