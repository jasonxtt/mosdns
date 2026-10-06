//! Request-owned immutable graphs. Executable IDs never cross generations.
use crate::assembly::{AssemblyError, ForwardCatalog, HostOptions, build_cache_catalog};
use crate::cache::CacheCatalog;
use crate::config::{CompiledConfig, ListenerKind};
use crate::observer::QueryObserver;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
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
    state_writer: RefCell<Option<Arc<std::sync::Mutex<crate::transaction::ManagedStore>>>>,
    updates: Rc<ListenerMailbox>,
    retiring_bindings: RefCell<BTreeSet<BindingKey>>,
    listeners_drained: Notify,
    current: RefCell<Rc<RuntimeSnapshot>>,
    retired: RefCell<Option<Rc<RuntimeSnapshot>>>,
    preparing: Cell<bool>,
    // Bounds transaction filesystem work submitted to the owned I/O worker.
    store_io_gate: Arc<tokio::sync::Semaphore>,
    retirement_gate: tokio::sync::Mutex<()>,
    admitting: Cell<bool>,
    started: Cell<bool>,
    active_management: Cell<usize>,
    management_drained: Notify,
    recovery_required: Cell<bool>,
    closed: Cell<bool>,
    observer: Arc<QueryObserver>,
    shutdown: RefCell<Option<mosdns_upstream_core::TransportCancellation>>,
    #[cfg(test)]
    apply_commit_test_gate: RefCell<Option<Arc<ApplyCommitTestGate>>>,
    #[cfg(test)]
    apply_marker_test_gate: RefCell<Option<Arc<ApplyCommitTestGate>>>,
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

#[cfg(test)]
pub(crate) struct ApplyCommitTestGate {
    reached: Notify,
    release: Notify,
}

#[cfg(test)]
impl ApplyCommitTestGate {
    pub(crate) fn new() -> Self {
        Self {
            reached: Notify::new(),
            release: Notify::new(),
        }
    }

    pub(crate) async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub(crate) fn release(&self) {
        self.release.notify_one();
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

#[cfg(test)]
fn coordinator_crash_boundary(name: &str) {
    if std::env::var("MOSDNS_TEST_APPLY_CRASH_AFTER").as_deref() != Ok(name) {
        return;
    }
    let ready = std::path::PathBuf::from(
        std::env::var_os("MOSDNS_TEST_APPLY_READY").expect("crash probe ready path"),
    );
    std::fs::write(ready, name).expect("publish apply crash boundary");
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
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
            state_writer: RefCell::new(None),
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
            store_io_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            retirement_gate: tokio::sync::Mutex::new(()),
            admitting: Cell::new(true),
            started: Cell::new(false),
            active_management: Cell::new(0),
            management_drained: Notify::new(),
            recovery_required: Cell::new(false),
            closed: Cell::new(false),
            observer,
            shutdown: RefCell::new(None),
            #[cfg(test)]
            apply_commit_test_gate: RefCell::new(None),
            #[cfg(test)]
            apply_marker_test_gate: RefCell::new(None),
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

    /// Compiles a complete candidate from the current managed files plus staged writes.
    pub(crate) fn compile_managed_candidate(
        &self,
        config_path: &std::path::Path,
        changes: Vec<(std::path::PathBuf, Option<Vec<u8>>)>,
    ) -> Result<crate::transaction::CompiledCandidate, crate::transaction::TransactionError> {
        let writer = self.0.state_writer.borrow().clone().ok_or_else(|| {
            crate::transaction::TransactionError::Invalid(
                "managed state writer is not attached".into(),
            )
        })?;
        let writer = writer.try_lock().map_err(|error| {
            crate::transaction::TransactionError::Busy(std::io::Error::other(error.to_string()))
        })?;
        writer.compile_candidate(config_path, changes)
    }

    #[cfg(test)]
    pub(crate) fn inject_apply_commit_test_gate(&self, gate: Arc<ApplyCommitTestGate>) {
        *self.0.apply_commit_test_gate.borrow_mut() = Some(gate);
    }

    #[cfg(test)]
    pub(crate) fn inject_apply_marker_test_gate(&self, gate: Arc<ApplyCommitTestGate>) {
        *self.0.apply_marker_test_gate.borrow_mut() = Some(gate);
    }

    pub(crate) fn attach_managed_store(
        &self,
        store: crate::transaction::ManagedStore,
    ) -> Result<(), AssemblyError> {
        let current = self.0.current.borrow();
        if self.0.state_writer.borrow().is_some()
            || current
                .config
                .managed_profile
                .as_ref()
                .is_none_or(|profile| profile.base_dir != store.root())
        {
            return Err(AssemblyError::Runtime(
                "managed writer root or owner mismatch".into(),
            ));
        }
        *self.0.state_writer.borrow_mut() = Some(Arc::new(std::sync::Mutex::new(store)));
        Ok(())
    }

    /// Runs durable-store work on the bounded worker without blocking the host runtime.
    pub(crate) async fn with_managed_store<T, F>(
        &self,
        operation: F,
    ) -> Result<T, crate::transaction::TransactionError>
    where
        T: Send + 'static,
        F: FnOnce(
                &mut crate::transaction::ManagedStore,
            ) -> Result<T, crate::transaction::TransactionError>
            + Send
            + 'static,
    {
        let writer = self.0.state_writer.borrow().clone().ok_or_else(|| {
            crate::transaction::TransactionError::Invalid(
                "managed state writer is not attached".into(),
            )
        })?;
        let permit = self
            .0
            .store_io_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                crate::transaction::TransactionError::Invalid("managed I/O worker is closed".into())
            })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::transaction::blocking_io(move || {
                let mut writer = writer.lock().map_err(|error| {
                    crate::transaction::TransactionError::Invalid(format!(
                        "managed writer lock poisoned: {error}"
                    ))
                })?;
                operation(&mut writer)
            })
            .map_err(crate::transaction::TransactionError::Io)?
        })
        .await
        .map_err(|error| {
            crate::transaction::TransactionError::Io(std::io::Error::other(format!(
                "managed I/O worker join failed: {error}"
            )))
        })?
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

    /// Reopens admission after an ordinary rollback.
    pub(crate) fn resume_admission(&self) {
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
        match (&current.config.managed_profile, &config.managed_profile) {
            (Some(previous), Some(next)) if previous.base_dir == next.base_dir => {}
            _ => {
                return Err(AssemblyError::Runtime(
                    "managed opt-in and canonical root must remain frozen".into(),
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
        let recovery = self.with_managed_store(|store| store.recover()).await;
        drop(prepared);
        drop(cache_gate);
        match recovery {
            Ok(()) => {
                self.resume_admission();
                original
            }
            Err(error) => {
                self.enter_recovery_required();
                self.stop_host();
                ApplyFailure::RecoveryRequired(format!(
                    "{original}; rollback/recovery failed: {error}"
                ))
            }
        }
    }

    fn map_transaction_error(error: crate::transaction::TransactionError) -> ApplyFailure {
        use crate::transaction::TransactionError;
        match error {
            TransactionError::Busy(error) => ApplyFailure::Busy(error.to_string()),
            TransactionError::Conflict(path) => ApplyFailure::Conflict(path),
            TransactionError::Invalid(reason) => ApplyFailure::Invalid(reason),
            TransactionError::CommitAmbiguous(reason) => ApplyFailure::RecoveryRequired(reason),
            TransactionError::Io(error) => ApplyFailure::Failed(error.to_string()),
        }
    }

    /// Applies an already compiled candidate under the host-owned transaction
    /// lifecycle. The caller's HTTP cancellation is intentionally not passed
    /// here: only root host shutdown can abort work before the commit marker.
    /// Applies a compiled candidate under the host-owned transaction lifecycle.
    pub(crate) async fn apply_candidate(
        &self,
        candidate: crate::transaction::CompiledCandidate,
    ) -> Result<u64, ApplyFailure> {
        let (config, mut persistence) = candidate.into_parts();
        let cache_dump_replacements = {
            let current = self.0.current.borrow();
            CacheCatalog::invalidated_dump_paths(&current.config, &config)
        };
        persistence.replace_cache_dumps(cache_dump_replacements);
        let old_generation = self.generation();
        let new_generation = old_generation
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

        if let Err(error) = self.pause_admission() {
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
        let stage = self
            .with_managed_store(move |store| {
                store.prepare_candidate(old_generation, new_generation, &persistence)
            })
            .await;
        if let Err(error) = stage {
            return Err(self
                .abort_precommit(Self::map_transaction_error(error), prepared, cache_gate)
                .await);
        }
        if self.shutdown_requested() {
            return Err(self
                .abort_precommit(ApplyFailure::Shutdown, prepared, cache_gate)
                .await);
        }

        let replacement = self
            .with_managed_store(|store| store.replace_prepared())
            .await;
        if let Err(error) = replacement {
            return Err(self
                .abort_precommit(Self::map_transaction_error(error), prepared, cache_gate)
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

        #[cfg(test)]
        let test_gate = self.0.apply_commit_test_gate.borrow_mut().take();
        #[cfg(test)]
        if let Some(gate) = test_gate {
            gate.reached.notify_one();
            gate.release.notified().await;
        }
        if self.shutdown_requested() {
            return Err(self
                .abort_precommit(ApplyFailure::Shutdown, prepared, cache_gate)
                .await);
        }

        let marker = self
            .with_managed_store(|store| store.mark_committed())
            .await;
        if let Err(error) = marker {
            let failure = Self::map_transaction_error(error);
            if let ApplyFailure::RecoveryRequired(reason) = failure {
                self.enter_recovery_required();
                self.stop_host();
                drop(cache_gate);
                drop(prepared);
                return Err(ApplyFailure::RecoveryRequired(format!(
                    "commit marker outcome is ambiguous: {reason}"
                )));
            }
            return Err(self.abort_precommit(failure, prepared, cache_gate).await);
        }

        #[cfg(test)]
        coordinator_crash_boundary("marker");

        #[cfg(test)]
        let marker_test_gate = self.0.apply_marker_test_gate.borrow_mut().take();
        #[cfg(test)]
        if let Some(gate) = marker_test_gate {
            gate.reached.notify_one();
            gate.release.notified().await;
        }

        // A panic here is an invariant failure after the point of no return.
        // Keep the journal and stop; startup will complete the committed state.
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

        #[cfg(test)]
        coordinator_crash_boundary("swap");

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

        #[cfg(test)]
        coordinator_crash_boundary("retirement");

        if let Err(error) = self
            .with_managed_store(|store| store.finish_retirement())
            .await
        {
            self.enter_recovery_required();
            self.stop_host();
            return Err(ApplyFailure::CommittedCleanup {
                generation,
                reason: error.to_string(),
            });
        }

        #[cfg(test)]
        coordinator_crash_boundary("cleanup");
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
        if !self.admission_open() {
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
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    static MANAGED_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn managed_fixture() -> (crate::HostAssembly, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "native-managed-apply-{}-{}",
            std::process::id(),
            MANAGED_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        let path = root.join("config.yaml");
        std::fs::write(
            &path,
            include_str!("../../../docs/rust/examples/native-managed-empty/config.yaml"),
        )
        .unwrap();
        std::fs::write(root.join("webinfo/special_upstream_groups.json"), b"[]\n").unwrap();
        std::fs::write(root.join("webinfo/upstream_overrides.json"), b"{}\n").unwrap();
        (crate::HostAssembly::from_config_file(&path).unwrap(), root)
    }

    fn managed_cache_fixture() -> (crate::HostAssembly, std::path::PathBuf) {
        managed_cache_fixture_with(15455, 15456, 15457, 15458, 0, None)
    }

    fn managed_cache_fixture_with(
        default_upstream: u16,
        sibling_upstream: u16,
        main_listener: u16,
        sibling_listener: u16,
        group_listener: u16,
        group_upstream: Option<u16>,
    ) -> (crate::HostAssembly, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "native-managed-cache-{}-{}",
            std::process::id(),
            MANAGED_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::create_dir_all(root.join("rule")).unwrap();
        std::fs::write(
            root.join("webinfo/special_upstream_groups.json"),
            format!(r#"[{{"slot":50,"name":"managed","listen_port":{group_listener}}}]"#)
                .into_bytes(),
        )
        .unwrap();
        let overrides = group_upstream.map_or_else(
            || "{}\n".to_owned(),
            |port| {
                format!(
                    r#"{{"special_upstream_50":[{{"tag":"local","enabled":true,"protocol":"udp","addr":"127.0.0.1:{port}"}}]}}"#
                )
            },
        );
        std::fs::write(root.join("webinfo/upstream_overrides.json"), overrides).unwrap();
        std::fs::write(root.join("rule/special_50.txt"), b"full:example.test\n").unwrap();
        let path = root.join("config.yaml");
        let yaml = r#"log: {level: error}
native_management: {special_groups: true}
plugins:
  - tag: default_upstream
    type: forward
    args: {upstreams: [{addr: "udp://127.0.0.1:15455"}]}
  - tag: cache_all
    type: cache
    args: {size: 64, lazy_cache_ttl: 0, dump_file: "cache/cache_all.dump"}
  - tag: sibling_cache
    type: cache
    args: {size: 64, lazy_cache_ttl: 0, dump_file: "cache/sibling.dump"}
  - tag: sibling_upstream
    type: forward
    args: {upstreams: [{addr: "udp://127.0.0.1:15456"}]}
  - tag: main_entry
    type: sequence
    args: [{exec: $cache_all}, {exec: 'cache 64'}, {exec: $special_upstream_matcher}, {exec: $default_upstream}]
  - tag: sibling_entry
    type: sequence
    args: [{exec: $sibling_cache}, {exec: $sibling_upstream}]
  - tag: main_udp
    type: udp_server
    args: {entry: main_entry, listen: "127.0.0.1:15457", enable_audit: false}
  - tag: sibling_udp
    type: udp_server
    args: {entry: sibling_entry, listen: "127.0.0.1:15458", enable_audit: false}
"#
        .replace("127.0.0.1:15455", &format!("127.0.0.1:{default_upstream}"))
        .replace("127.0.0.1:15456", &format!("127.0.0.1:{sibling_upstream}"))
        .replace("127.0.0.1:15457", &format!("127.0.0.1:{main_listener}"))
        .replace("127.0.0.1:15458", &format!("127.0.0.1:{sibling_listener}"));
        std::fs::write(&path, yaml).unwrap();
        (crate::HostAssembly::from_config_file(&path).unwrap(), root)
    }

    fn seed_cache(adapter: &crate::cache::NativeCacheAdapter, name: &str) {
        let wire = cache_query(name);
        let response = cache_response(&wire);
        assert!(
            adapter
                .begin_store(&wire)
                .unwrap()
                .unwrap()
                .publish(&response)
                .unwrap()
        );
    }

    fn cache_query(name: &str) -> Vec<u8> {
        let mut query = hickory_proto::op::Message::new();
        query
            .set_id(71)
            .set_message_type(hickory_proto::op::MessageType::Query)
            .add_query(hickory_proto::op::Query::query(
                hickory_proto::rr::Name::from_ascii(name).unwrap(),
                hickory_proto::rr::RecordType::A,
            ));
        query.to_vec().unwrap()
    }

    fn cache_response(wire: &[u8]) -> Vec<u8> {
        let (header, question) = mosdns_dns_core::parse_query(wire).unwrap();
        mosdns_dns_core::synthesize_response(&header, &question, 0).unwrap()
    }

    fn start_controlled_peer(
        socket: std::net::UdpSocket,
        rcode: u8,
        requests: std::sync::Arc<AtomicUsize>,
        stop: mosdns_upstream_core::TransportCancellation,
    ) -> tokio::task::JoinHandle<()> {
        socket.set_nonblocking(true).unwrap();
        let socket = tokio::net::UdpSocket::from_std(socket).unwrap();
        tokio::task::spawn_local(async move {
            let mut wire = [0; 512];
            loop {
                let received = tokio::select! {
                    () = stop.cancelled() => break,
                    received = socket.recv_from(&mut wire) => received,
                };
                let (length, address) = received.unwrap();
                requests.fetch_add(1, Ordering::Relaxed);
                let (header, question) = mosdns_dns_core::parse_query(&wire[..length]).unwrap();
                let response =
                    mosdns_dns_core::synthesize_response(&header, &question, rcode).unwrap();
                socket.send_to(&response, address).await.unwrap();
            }
        })
    }

    async fn dns_exchange(socket: &tokio::net::UdpSocket, port: u16, query: &[u8]) -> u16 {
        socket.send_to(query, ("127.0.0.1", port)).await.unwrap();
        let mut response = [0; 512];
        let (_length, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            socket.recv_from(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        response[3] as u16 & 0x0f
    }

    #[test]
    fn managed_policy_change_replaces_only_affected_cache_owners() {
        let (host, root) = managed_cache_fixture();
        let control = host.control();
        let old = control.view();
        let id = |config: &crate::config::CompiledConfig, tag: &str| {
            config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id
        };
        let all_id = id(&old.config, "cache_all");
        let sibling_id = id(&old.config, "sibling_cache");
        let group_id = id(&old.config, "cache_special_50");
        let quick_tag = old
            .config
            .caches
            .iter()
            .find(|cache| cache.kind == crate::config::CacheKind::Quick)
            .unwrap()
            .tag
            .clone();
        let all = old.cache.get(all_id).unwrap();
        let sibling = old.cache.get(sibling_id).unwrap();
        let group = old.cache.get(group_id).unwrap();
        seed_cache(all, "example.test.");
        seed_cache(sibling, "sibling.test.");
        seed_cache(group, "example.test.");

        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("rule/special_50.txt"),
                    Some(b"full:other.test\n".to_vec()),
                )],
            )
            .unwrap();
        let (config, _persistence) = candidate.into_parts();
        let prepared = control.prepare(config).unwrap();
        let next = prepared.snapshot.as_ref().unwrap();
        let next_all = next.cache.get(id(&next.config, "cache_all")).unwrap();
        let next_sibling = next.cache.get(id(&next.config, "sibling_cache")).unwrap();
        let next_group = next
            .cache
            .get(id(&next.config, "cache_special_50"))
            .unwrap();
        let next_quick = next.cache.get(id(&next.config, &quick_tag)).unwrap();

        assert!(
            next_all.is_empty(),
            "the router-wrapping cache is policy-dependent"
        );
        assert_eq!(
            next_sibling.len(),
            1,
            "the disjoint sibling owner must survive"
        );
        assert!(
            next_group.is_empty(),
            "the changed group route closure must be fenced"
        );
        assert!(
            next_quick.is_empty(),
            "quick caches depend on their downstream policy closure"
        );
        drop(prepared);
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_group_upstream_edit_invalidates_its_cache_closure_only() {
        let (host, root) = managed_cache_fixture();
        let control = host.control();
        let old = control.view();
        let quick_tag = old
            .config
            .caches
            .iter()
            .find(|cache| cache.kind == crate::config::CacheKind::Quick)
            .unwrap()
            .tag
            .clone();
        for tag in [
            "cache_all",
            "sibling_cache",
            "cache_special_50",
            quick_tag.as_str(),
        ] {
            let id = old
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            seed_cache(old.cache.get(id).unwrap(), "example.test.");
        }

        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                managed_changes("127.0.0.1:15459"),
            )
            .unwrap();
        let (config, _persistence) = candidate.into_parts();
        let prepared = control.prepare(config).unwrap();
        let next = prepared.snapshot.as_ref().unwrap();
        for tag in ["cache_all", "cache_special_50", quick_tag.as_str()] {
            let id = next
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            assert!(next.cache.get(id).unwrap().is_empty(), "{tag}");
        }
        let sibling_id = next
            .config
            .caches
            .iter()
            .find(|cache| cache.tag == "sibling_cache")
            .unwrap()
            .id;
        assert_eq!(next.cache.get(sibling_id).unwrap().len(), 1);

        drop(prepared);
        host.block_on(control.close()).unwrap();
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_port_only_membership_change_invalidates_group_and_ancestor_caches() {
        let (host, root) = managed_cache_fixture_with(15455, 15456, 15457, 15458, 15575, None);
        let control = host.control();
        let old = control.view();
        for tag in ["cache_all", "sibling_cache", "cache_special_50"] {
            let id = old
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            seed_cache(old.cache.get(id).unwrap(), "example.test.");
        }

        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("webinfo/special_upstream_groups.json"),
                    Some(br#"[{"slot":50,"name":"managed","listen_port":15575,"custom_port_only":true}]"#.to_vec()),
                )],
            )
            .unwrap();
        let (config, _persistence) = candidate.into_parts();
        let prepared = control.prepare(config).unwrap();
        let next = prepared.snapshot.as_ref().unwrap();
        for tag in ["cache_all", "cache_special_50"] {
            let id = next
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            assert!(next.cache.get(id).unwrap().is_empty(), "{tag}");
        }
        let sibling_id = next
            .config
            .caches
            .iter()
            .find(|cache| cache.tag == "sibling_cache")
            .unwrap()
            .id;
        assert_eq!(next.cache.get(sibling_id).unwrap().len(), 1);

        drop(prepared);
        host.block_on(control.close()).unwrap();
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_policy_commit_replaces_affected_dumps_and_keeps_sibling_dump() {
        let (host, root) = managed_cache_fixture();
        let control = host.control();
        let old = control.view();
        let cache = |tag: &str| {
            let id = old
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            old.cache.get(id).unwrap()
        };
        let all = cache("cache_all");
        let sibling = cache("sibling_cache");
        let group = cache("cache_special_50");
        seed_cache(all, "example.test.");
        seed_cache(sibling, "sibling.test.");
        seed_cache(group, "example.test.");
        host.block_on(all.save()).unwrap();
        host.block_on(sibling.save()).unwrap();
        host.block_on(group.save()).unwrap();
        let sibling_dump = std::fs::read(root.join("cache/sibling.dump")).unwrap();
        assert_eq!(crate::cache_dump::decode(&sibling_dump).unwrap().len(), 1);

        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("rule/special_50.txt"),
                    Some(b"full:other.test\n".to_vec()),
                )],
            )
            .unwrap();
        host.block_on(control.apply_candidate(candidate)).unwrap();

        for path in ["cache/cache_all.dump", "cache/cache_special_50.dump"] {
            let bytes = std::fs::read(root.join(path)).unwrap();
            assert!(
                crate::cache_dump::decode(&bytes).unwrap().is_empty(),
                "impacted durable cache must be replaced with a valid empty dump: {path}"
            );
        }
        assert_eq!(
            std::fs::read(root.join("cache/sibling.dump")).unwrap(),
            sibling_dump,
            "a disjoint owner dump must remain byte-for-byte unchanged"
        );
        host.block_on(control.close()).unwrap();
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_policy_commit_fences_changed_misses_but_keeps_disjoint_miss() {
        let (host, root) = managed_cache_fixture();
        let control = host.control();
        let old = control.view();
        let cache = |tag: &str| {
            let id = old
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == tag)
                .unwrap()
                .id;
            old.cache.get(id).unwrap()
        };
        let all_query = cache_query("example.test.");
        let sibling_query = cache_query("sibling.test.");
        let group_query = cache_query("example.test.");
        let all_miss = cache("cache_all").begin_store(&all_query).unwrap().unwrap();
        let sibling_miss = cache("sibling_cache")
            .begin_store(&sibling_query)
            .unwrap()
            .unwrap();
        let group_miss = cache("cache_special_50")
            .begin_store(&group_query)
            .unwrap()
            .unwrap();
        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("rule/special_50.txt"),
                    Some(b"full:other.test\n".to_vec()),
                )],
            )
            .unwrap();
        host.block_on(control.apply_candidate(candidate)).unwrap();

        assert!(!all_miss.publish(&cache_response(&all_query)).unwrap());
        assert!(!group_miss.publish(&cache_response(&group_query)).unwrap());
        assert!(
            sibling_miss
                .publish(&cache_response(&sibling_query))
                .unwrap()
        );
        host.block_on(control.close()).unwrap();
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_rule_edit_changes_real_dns_and_preserves_disjoint_warm_answer() {
        let group_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let default_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let sibling_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let group_port = group_socket.local_addr().unwrap().port();
        let default_port = default_socket.local_addr().unwrap().port();
        let sibling_port = sibling_socket.local_addr().unwrap().port();
        let main_port = free_pair();
        let sibling_listener = free_pair();
        let (host, root) = managed_cache_fixture_with(
            default_port,
            sibling_port,
            main_port,
            sibling_listener,
            0,
            Some(group_port),
        );

        host.block_on(async {
            let group_requests = std::sync::Arc::new(AtomicUsize::new(0));
            let default_requests = std::sync::Arc::new(AtomicUsize::new(0));
            let sibling_requests = std::sync::Arc::new(AtomicUsize::new(0));
            let peer_stop = mosdns_upstream_core::TransportCancellation::new();
            let group_task = start_controlled_peer(
                group_socket,
                0,
                group_requests.clone(),
                peer_stop.child_token(),
            );
            let default_task = start_controlled_peer(
                default_socket,
                3,
                default_requests.clone(),
                peer_stop.child_token(),
            );
            let sibling_task = start_controlled_peer(
                sibling_socket,
                0,
                sibling_requests.clone(),
                peer_stop.child_token(),
            );
            let bound = host.bind_host().await.unwrap();
            let stop = mosdns_upstream_core::TransportCancellation::new();
            let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
            let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let group_query = cache_query("example.test.");
            let sibling_query = cache_query("sibling.test.");

            assert_eq!(dns_exchange(&client, main_port, &group_query).await, 0);
            assert_eq!(dns_exchange(&client, main_port, &group_query).await, 0);
            assert_eq!(group_requests.load(Ordering::Relaxed), 1);
            assert_eq!(
                dns_exchange(&client, sibling_listener, &sibling_query).await,
                0
            );
            assert_eq!(
                dns_exchange(&client, sibling_listener, &sibling_query).await,
                0
            );
            assert_eq!(sibling_requests.load(Ordering::Relaxed), 1);

            let candidate = host
                .control()
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    vec![(
                        std::path::PathBuf::from("rule/special_50.txt"),
                        Some(b"full:other.test\n".to_vec()),
                    )],
                )
                .unwrap();
            host.control().apply_candidate(candidate).await.unwrap();

            assert_eq!(dns_exchange(&client, main_port, &group_query).await, 3);
            assert_eq!(default_requests.load(Ordering::Relaxed), 1);
            assert_eq!(
                dns_exchange(&client, sibling_listener, &sibling_query).await,
                0
            );
            assert_eq!(sibling_requests.load(Ordering::Relaxed), 1);

            stop.cancel();
            serving.await.unwrap().unwrap();
            peer_stop.cancel();
            group_task.await.unwrap();
            default_task.await.unwrap();
            sibling_task.await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn group_rename_and_port_only_change_retain_wire_cache_owners() {
        let old_listener = free_pair();
        let new_listener = loop {
            let candidate = free_pair();
            if candidate != old_listener {
                break candidate;
            }
        };
        let (host, root) =
            managed_cache_fixture_with(15455, 15456, 15457, 15458, old_listener, None);
        let control = host.control();
        let old = control.view();
        let quick_tag = old
            .config
            .caches
            .iter()
            .find(|cache| cache.kind == crate::config::CacheKind::Quick)
            .unwrap()
            .tag
            .clone();
        for cache in [
            "cache_all",
            "sibling_cache",
            "cache_special_50",
            quick_tag.as_str(),
        ] {
            let id = old
                .config
                .caches
                .iter()
                .find(|item| item.tag == cache)
                .unwrap()
                .id;
            seed_cache(old.cache.get(id).unwrap(), "example.test.");
        }

        let candidate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("webinfo/special_upstream_groups.json"),
                    Some(
                        format!(
                            r#"[{{"slot":50,"name":"renamed","listen_port":{}}}]"#,
                            new_listener
                        )
                        .into_bytes(),
                    ),
                )],
            )
            .unwrap();
        let (config, _persistence) = candidate.into_parts();
        let prepared = control.prepare(config).unwrap();
        let next = prepared.snapshot.as_ref().unwrap();
        for tag in ["cache_all", "sibling_cache", "cache_special_50"] {
            let id = next
                .config
                .caches
                .iter()
                .find(|item| item.tag == tag)
                .unwrap()
                .id;
            assert_eq!(
                next.cache.get(id).unwrap().len(),
                1,
                "display rename and listener port do not change wire cache policy: {tag}"
            );
        }
        drop(prepared);
        host.block_on(control.close()).unwrap();
        drop(old);
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deleting_and_reusing_a_slot_does_not_reimport_its_old_cache_dump() {
        let (host, root) = managed_cache_fixture();
        let control = host.control();
        let old = control.view();
        let group_id = old
            .config
            .caches
            .iter()
            .find(|cache| cache.tag == "cache_special_50")
            .unwrap()
            .id;
        let old_group_cache = old.cache.get(group_id).unwrap();
        seed_cache(old_group_cache, "example.test.");
        host.block_on(old_group_cache.save()).unwrap();
        assert_eq!(
            crate::cache_dump::decode(
                &std::fs::read(root.join("cache/cache_special_50.dump")).unwrap()
            )
            .unwrap()
            .len(),
            1
        );

        let delete = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("webinfo/special_upstream_groups.json"),
                    Some(b"[]\n".to_vec()),
                )],
            )
            .unwrap();
        host.block_on(control.apply_candidate(delete)).unwrap();
        assert!(
            crate::cache_dump::decode(
                &std::fs::read(root.join("cache/cache_special_50.dump")).unwrap()
            )
            .unwrap()
            .is_empty()
        );

        let recreate = control
            .compile_managed_candidate(
                &root.join("config.yaml"),
                vec![(
                    std::path::PathBuf::from("webinfo/special_upstream_groups.json"),
                    Some(br#"[{"slot":50,"name":"managed","listen_port":0}]"#.to_vec()),
                )],
            )
            .unwrap();
        host.block_on(control.apply_candidate(recreate)).unwrap();
        let current = control.view();
        let current_group = current
            .config
            .caches
            .iter()
            .find(|cache| cache.tag == "cache_special_50")
            .unwrap()
            .id;
        assert!(current.cache.get(current_group).unwrap().is_empty());
        host.block_on(control.close()).unwrap();
        drop(current);
        drop(old);
        drop(control);
        drop(host);

        let restarted = crate::HostAssembly::from_config_file(&root.join("config.yaml")).unwrap();
        let _bound = restarted.block_on(restarted.bind_host()).unwrap();
        let current_group = restarted
            .config()
            .caches
            .iter()
            .find(|cache| cache.tag == "cache_special_50")
            .unwrap()
            .id;
        assert!(restarted.cache().get(current_group).unwrap().is_empty());
        drop(_bound);
        restarted.block_on(restarted.control().close()).unwrap();
        drop(restarted);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn managed_changes(address: &str) -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
        vec![
            (
                std::path::PathBuf::from("webinfo/special_upstream_groups.json"),
                Some(br#"[{"slot":50,"name":"managed","listen_port":0}]"#.to_vec()),
            ),
            (
                std::path::PathBuf::from("webinfo/upstream_overrides.json"),
                Some(
                    format!(
                        r#"{{"special_upstream_50":[{{"tag":"local","enabled":true,"protocol":"udp","addr":"{address}"}}]}}"#
                    )
                    .into_bytes(),
                ),
            ),
        ]
    }

    #[test]
    fn managed_apply_replaces_files_swaps_runtime_and_cleans_after_retirement() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let generation = control.apply_candidate(candidate).await.unwrap();
            assert_eq!(generation, 1);
            assert_eq!(control.generation(), 1);
            assert_eq!(
                control
                    .view()
                    .config
                    .managed_profile
                    .as_ref()
                    .unwrap()
                    .groups[0]
                    .name,
                "managed"
            );
            assert!(root.join("sub_config/special_groups.yaml").is_file());
            assert!(
                !crate::transaction::ManagedStore::has_pending(&root),
                "journal cleanup follows retirement"
            );
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_apply_conflict_keeps_external_edit_and_reopens_old_generation() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let initial = control.view();
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let external = br#"{"external":true}"#;
            std::fs::write(root.join("webinfo/upstream_overrides.json"), external).unwrap();
            let error = control.apply_candidate(candidate).await.unwrap_err();
            assert!(matches!(error, super::ApplyFailure::Conflict(_)));
            assert_eq!(control.generation(), 0);
            assert!(Rc::ptr_eq(&initial, &control.view()));
            assert!(control.admission_open());
            assert_eq!(
                std::fs::read(root.join("webinfo/upstream_overrides.json")).unwrap(),
                external
            );
            assert!(!crate::transaction::ManagedStore::has_pending(&root));
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_cache_apply_conflict_preserves_memory_dumps_and_admission() {
        let (host, root) = managed_cache_fixture();
        host.block_on(async {
            let control = host.control();
            let initial = control.view();
            let adapter = |tag: &str| {
                let id = initial
                    .config
                    .caches
                    .iter()
                    .find(|cache| cache.tag == tag)
                    .unwrap()
                    .id;
                initial.cache.get(id).unwrap().clone()
            };
            let all = adapter("cache_all");
            let sibling = adapter("sibling_cache");
            let group = adapter("cache_special_50");
            seed_cache(&all, "example.test.");
            seed_cache(&sibling, "sibling.test.");
            seed_cache(&group, "example.test.");
            all.save().await.unwrap();
            sibling.save().await.unwrap();
            group.save().await.unwrap();
            let old_dumps = [
                "cache/cache_all.dump",
                "cache/sibling.dump",
                "cache/cache_special_50.dump",
            ]
            .map(|path| std::fs::read(root.join(path)).unwrap());
            let pending = all
                .begin_store(&cache_query("pending.example."))
                .unwrap()
                .unwrap();

            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    vec![(
                        std::path::PathBuf::from("rule/special_50.txt"),
                        Some(b"full:other.test\n".to_vec()),
                    )],
                )
                .unwrap();
            let external = b"full:external.test\n";
            std::fs::write(root.join("rule/special_50.txt"), external).unwrap();

            assert!(matches!(
                control.apply_candidate(candidate).await,
                Err(super::ApplyFailure::Conflict(_))
            ));
            assert_eq!(control.generation(), 0);
            assert!(Rc::ptr_eq(&initial, &control.view()));
            assert!(control.admission_open());
            assert_eq!(all.len(), 1);
            assert_eq!(sibling.len(), 1);
            assert_eq!(group.len(), 1);
            for (path, expected) in [
                "cache/cache_all.dump",
                "cache/sibling.dump",
                "cache/cache_special_50.dump",
            ]
            .into_iter()
            .zip(old_dumps)
            {
                assert_eq!(std::fs::read(root.join(path)).unwrap(), expected, "{path}");
            }
            assert_eq!(
                std::fs::read(root.join("rule/special_50.txt")).unwrap(),
                external
            );
            assert!(!crate::transaction::ManagedStore::has_pending(&root));
            assert!(
                pending
                    .publish(&cache_response(&cache_query("pending.example.")))
                    .unwrap()
            );

            control.close().await.unwrap();
            drop(initial);
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_apply_closes_admission_then_waits_for_existing_management_write() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let lease = control.begin_management_mutation().unwrap();
            let task_control = control.clone();
            let apply =
                tokio::task::spawn_local(
                    async move { task_control.apply_candidate(candidate).await },
                );
            tokio::task::yield_now().await;

            assert!(!control.admission_open());
            assert_eq!(control.generation(), 0);
            assert!(
                !crate::transaction::ManagedStore::has_pending(&root),
                "persistent preparation waits until admitted management writes drain"
            );
            drop(lease);
            assert_eq!(apply.await.unwrap().unwrap(), 1);
            assert_eq!(control.generation(), 1);
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_shutdown_before_marker_rolls_back_and_does_not_reopen_admission() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let shutdown = mosdns_upstream_core::TransportCancellation::new();
            let updates = control.start(&shutdown);
            let gate = std::sync::Arc::new(super::ApplyCommitTestGate::new());
            control.inject_apply_commit_test_gate(gate.clone());
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let task_control = control.clone();
            let apply =
                tokio::task::spawn_local(
                    async move { task_control.apply_candidate(candidate).await },
                );
            gate.wait_until_reached().await;
            assert_eq!(control.generation(), 0);
            assert!(!control.admission_open());
            assert_ne!(
                std::fs::read(root.join("webinfo/special_upstream_groups.json")).unwrap(),
                b"[]\n",
                "candidate files are replaced before the marker"
            );
            shutdown.cancel();
            gate.release();
            assert!(matches!(
                apply.await.unwrap(),
                Err(super::ApplyFailure::Shutdown)
            ));
            assert_eq!(control.generation(), 0);
            assert!(!control.admission_open());
            assert_eq!(
                std::fs::read(root.join("webinfo/special_upstream_groups.json")).unwrap(),
                b"[]\n"
            );
            assert!(!crate::transaction::ManagedStore::has_pending(&root));
            drop(updates);
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_failure_enters_recovery_required_and_retains_journal() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let shutdown = mosdns_upstream_core::TransportCancellation::new();
            let updates = control.start(&shutdown);
            let gate = std::sync::Arc::new(super::ApplyCommitTestGate::new());
            control.inject_apply_commit_test_gate(gate.clone());
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let task_control = control.clone();
            let apply =
                tokio::task::spawn_local(
                    async move { task_control.apply_candidate(candidate).await },
                );
            gate.wait_until_reached().await;

            let journal = root.join("webinfo/native_management.transaction");
            let backup = std::fs::read_dir(&journal)
                .unwrap()
                .map(Result::unwrap)
                .map(|entry| entry.path())
                .find(|path| path.extension().is_some_and(|extension| extension == "old"))
                .expect("transaction contains an old-state backup");
            std::fs::write(&backup, b"tampered rollback artifact").unwrap();
            shutdown.cancel();
            gate.release();

            assert!(matches!(
                apply.await.unwrap(),
                Err(super::ApplyFailure::RecoveryRequired(_))
            ));
            assert!(control.recovery_required());
            assert!(!control.admission_open());
            assert_eq!(control.generation(), 0);
            assert!(crate::transaction::ManagedStore::has_pending(&root));
            drop(updates);
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_apply_finishes_after_client_disconnect_and_host_shutdown() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let shutdown = mosdns_upstream_core::TransportCancellation::new();
            let updates = control.start(&shutdown);
            let gate = std::sync::Arc::new(super::ApplyCommitTestGate::new());
            control.inject_apply_marker_test_gate(gate.clone());
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (server, _) = listener.accept().await.unwrap();
            let (reader, _writer) = server.into_split();
            let request_shutdown = shutdown.child_token();
            let task_control = control.clone();
            let task_shutdown = shutdown.clone();
            let task_request_shutdown = request_shutdown.clone();
            let apply = tokio::task::spawn_local(async move {
                let operation = async move { task_control.apply_candidate(candidate).await };
                crate::api::await_dispatch_after_disconnect(
                    &task_shutdown,
                    &task_request_shutdown,
                    reader,
                    operation,
                )
                .await
            });

            gate.wait_until_reached().await;
            assert!(
                root.join("webinfo/native_management.transaction/commit")
                    .is_file(),
                "the durable commit marker precedes the disconnect"
            );
            client.shutdown().await.unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                request_shutdown.cancelled(),
            )
            .await
            .expect("API observes client disconnect");
            assert!(!apply.is_finished(), "the transaction remains host-owned");

            shutdown.cancel();
            gate.release();
            assert_eq!(apply.await.unwrap().unwrap(), 1);
            assert_eq!(control.generation(), 1);
            assert!(!control.admission_open());
            assert!(!crate::transaction::ManagedStore::has_pending(&root));
            assert!(
                String::from_utf8_lossy(
                    &std::fs::read(root.join("webinfo/special_upstream_groups.json")).unwrap()
                )
                .contains("managed")
            );
            drop(updates);
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ambiguous_marker_failure_fails_closed_and_retains_transaction_evidence() {
        let (host, root) = managed_fixture();
        host.block_on(async {
            let control = host.control();
            let shutdown = mosdns_upstream_core::TransportCancellation::new();
            let updates = control.start(&shutdown);
            let gate = std::sync::Arc::new(super::ApplyCommitTestGate::new());
            control.inject_apply_commit_test_gate(gate.clone());
            let candidate = control
                .compile_managed_candidate(
                    &root.join("config.yaml"),
                    managed_changes("127.0.0.1:15455"),
                )
                .unwrap();
            let task_control = control.clone();
            let apply =
                tokio::task::spawn_local(
                    async move { task_control.apply_candidate(candidate).await },
                );
            gate.wait_until_reached().await;
            let journal = root.join("webinfo/native_management.transaction");
            let external = root.join("outside-marker-target");
            std::fs::write(&external, b"sentinel").unwrap();
            std::os::unix::fs::symlink(&external, journal.join("commit.tmp")).unwrap();
            gate.release();

            assert!(matches!(
                apply.await.unwrap(),
                Err(super::ApplyFailure::RecoveryRequired(_))
            ));
            assert!(control.recovery_required());
            assert!(!control.admission_open());
            assert_eq!(control.generation(), 0);
            assert!(crate::transaction::ManagedStore::has_pending(&root));
            assert_eq!(std::fs::read(&external).unwrap(), b"sentinel");
            drop(updates);
            control.close().await.unwrap();
        });
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sigkill_after_marker_swap_retirement_and_cleanup_recovers_committed_candidate() {
        use std::os::unix::process::ExitStatusExt;

        for phase in ["marker", "swap", "retirement", "cleanup"] {
            let root = std::env::temp_dir().join(format!(
                "native-managed-apply-kill-{}-{phase}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("webinfo")).unwrap();
            std::fs::create_dir_all(root.join("cache")).unwrap();
            let path = root.join("config.yaml");
            std::fs::write(
                &path,
                r#"log: {level: error}
native_management: {special_groups: true}
api: {http: "127.0.0.1:15380"}
plugins:
  - tag: default_upstream
    type: forward
    args: {upstreams: [{tag: local_default, addr: "udp://127.0.0.1:15455"}]}
  - tag: cache_all
    type: cache
    args: {size: 64, lazy_cache_ttl: 0, dump_file: "cache/cache_all.dump"}
  - tag: main_entry
    type: sequence
    args:
      - exec: $cache_all
      - exec: $special_upstream_matcher
      - exec: $default_upstream
  - tag: main_udp
    type: udp_server
    args: {entry: main_entry, listen: "127.0.0.1:15355", enable_audit: false}
"#,
            )
            .unwrap();
            std::fs::write(root.join("webinfo/special_upstream_groups.json"), b"[]\n").unwrap();
            std::fs::write(root.join("webinfo/upstream_overrides.json"), b"{}\n").unwrap();
            let ready = root.join("ready");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "runtime_snapshot::admission_tests::managed_apply_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MOSDNS_TEST_APPLY_ROOT", &root)
                .env("MOSDNS_TEST_APPLY_READY", &ready)
                .env("MOSDNS_TEST_APPLY_CRASH_AFTER", phase)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                if ready.exists() {
                    break;
                }
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("apply child exited before {phase}: {status}");
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("apply child did not reach {phase}");
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(std::fs::read(&ready).unwrap(), phase.as_bytes());
            assert!(
                std::process::Command::new("kill")
                    .args(["-KILL", &child.id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
            assert_eq!(child.wait().unwrap().signal(), Some(9));

            let mut restarted = crate::transaction::ManagedStore::open(&root).unwrap();
            restarted.recover().unwrap();
            let groups = std::fs::read(root.join("webinfo/special_upstream_groups.json")).unwrap();
            assert!(
                String::from_utf8_lossy(&groups).contains("managed"),
                "committed group file recovers after {phase}"
            );
            let generated =
                std::fs::read_to_string(root.join("sub_config/special_groups.yaml")).unwrap();
            assert!(generated.contains("sequence_special_50"));
            let dump_path = root.join("cache/cache_all.dump");
            let dump = std::fs::read(&dump_path).unwrap();
            assert!(
                crate::cache_dump::decode(&dump).unwrap().is_empty(),
                "old cache data and final snapshots cannot overwrite the committed empty dump after {phase}"
            );
            assert!(!crate::transaction::ManagedStore::has_pending(&root));
            drop(restarted);

            let restarted_host = crate::HostAssembly::from_config_file(&path).unwrap();
            let restarted_control = restarted_host.control();
            let restarted_view = restarted_control.view();
            let cache_id = restarted_view
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == "cache_all")
                .unwrap()
                .id;
            assert!(
                restarted_view.cache.get(cache_id).unwrap().is_empty(),
                "committed cache invalidation remains empty after host restart at {phase}"
            );
            restarted_host.block_on(restarted_control.close()).unwrap();
            drop(restarted_view);
            drop(restarted_control);
            drop(restarted_host);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess crash probe invoked by the parent recovery test"]
    fn managed_apply_crash_child() {
        let Some(root) = std::env::var_os("MOSDNS_TEST_APPLY_ROOT").map(std::path::PathBuf::from)
        else {
            return;
        };
        let config = root.join("config.yaml");
        let host = crate::HostAssembly::from_config_file(&config).unwrap();
        host.block_on(async {
            let control = host.control();
            let initial = control.view();
            let cache_id = initial
                .config
                .caches
                .iter()
                .find(|cache| cache.tag == "cache_all")
                .unwrap()
                .id;
            let cache = initial.cache.get(cache_id).unwrap();
            seed_cache(cache, "example.test.");
            cache.save().await.unwrap();
            seed_cache(cache, "pending.test.");
            assert_eq!(
                crate::cache_dump::decode(
                    &std::fs::read(root.join("cache/cache_all.dump")).unwrap()
                )
                .unwrap()
                .len(),
                1,
                "the transaction begins with a durable prior-generation entry"
            );
            drop(initial);
            let candidate = control
                .compile_managed_candidate(&config, managed_changes("127.0.0.1:15455"))
                .unwrap();
            control.apply_candidate(candidate).await.unwrap();
        });
        panic!("requested coordinator crash boundary was not reached");
    }

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
        let groups = root.join("webinfo/special_upstream_groups.json");
        let write_group = |port| {
            std::fs::write(
                &groups,
                format!(
                    r#"[{{"slot":50,"name":"group","listen_port":{port},"custom_port_only":true}}]"#
                ),
            )
            .unwrap()
        };
        write_group(old_port);
        std::fs::write(root.join("webinfo/upstream_overrides.json"),r#"{"special_upstream_50":[{"tag":"controlled","protocol":"udp","addr":"127.0.0.1:15999","enabled":true}]}"#).unwrap();
        let path = root.join("config.yaml");
        std::fs::write(&path,format!("log: {{level: error}}\nnative_management: {{special_groups: true}}\nplugins:\n  - tag: entry\n    type: sequence\n    args: [{{exec: $special_upstream_matcher}}, {{exec: reject 3}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{}', enable_audit: true}}\n",free_pair())).unwrap();
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
            write_group(free_pair());
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
        let old_port = free_pair();
        let groups = root.join("webinfo/special_upstream_groups.json");
        let write_group = |port| {
            std::fs::write(
                &groups,
                format!(
                    r#"[{{"slot":50,"name":"group","listen_port":{port},"custom_port_only":true}}]"#
                ),
            )
            .unwrap()
        };
        write_group(old_port);
        std::fs::write(root.join("webinfo/upstream_overrides.json"),r#"{"special_upstream_50":[{"tag":"controlled","protocol":"udp","addr":"127.0.0.1:15999","enabled":true}]}"#).unwrap();
        let path = root.join("config.yaml");
        std::fs::write(&path,format!("log: {{level: error}}\nnative_management: {{special_groups: true}}\nplugins:\n  - tag: entry\n    type: sequence\n    args: [{{exec: $special_upstream_matcher}}, {{exec: reject 3}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{}', enable_audit: true}}\n",free_pair())).unwrap();
        let host =
            crate::HostAssembly::from_config(crate::load_and_compile(&path).unwrap()).unwrap();
        host.block_on(async {
            let control = host.control();
            let old = control.view();
            std::fs::write(root.join("webinfo/upstream_overrides.json"),r#"{"special_upstream_50":[{"tag":"replacement","protocol":"udp","addr":"127.0.0.1:16000","enabled":true}]}"#).unwrap();
            let prepared = control.prepare(crate::load_and_compile(&path).unwrap()).unwrap();
            control.install(prepared).unwrap();
            let current = control.view();
            old.cache.get(crate::config::CacheId(0)).unwrap().inject_refresh_join_failure();
            current.cache.get(crate::config::CacheId(0)).unwrap().inject_persist_fault(crate::managed::PersistFault::Rename);
            assert!(control.close().await.is_err(), "cleanup errors must remain observable");
            for snapshot in [old,current] {
                let executable = snapshot.config.forward_invocations[0].executable;
                let owner = snapshot.forwards.forward(executable).unwrap();
                assert_eq!(owner.upstream().lifecycle_state(),mosdns_upstream_core::LifecycleState::Closed,"both generations must close transports despite cache cleanup failures");
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
