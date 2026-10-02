use hickory_proto::op::Message;
use mosdns_matcher_core::IpPrefixList;
use mosdns_upstream_core::TransportCancellation;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::future::Future;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use mosdns_cache_core::{CacheConfig, CacheError, NativeCache};
use mosdns_dns_core::{
    QueryError, ResponseError, ResponseMetadata, ResponseQuestion, observe_response_metadata,
    patch_response_id_ra, validate_response,
};

use crate::config::CacheId;

const CACHE_CAPACITY: u64 = 64;
const CACHE_LAZY_TTL_SECS: u32 = 0;
const CLASS_IN: u16 = 1;
const RCODE_NOERROR: u16 = 0;
const RCODE_SERVFAIL: u16 = 2;
const RCODE_NXDOMAIN: u16 = 3;
const RETENTION_FALLBACK_SECS: u32 = 5;
const RETENTION_NXDOMAIN_SECS: u32 = 30;
const RETENTION_MAX_NOERROR_SECS: u32 = 300;

/// A monotonic clock expressed in the same elapsed-second epoch for every
/// operation on one host-owned cache.
pub trait CacheClock {
    /// Returns elapsed whole seconds since the clock's private epoch.
    fn now_seconds(&self) -> Result<i64, CacheAdapterError>;
    fn wall_seconds(&self) -> Result<i64, CacheAdapterError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CacheAdapterError::ClockOverflow)?;
        i64::try_from(elapsed.as_secs()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
}

#[derive(Clone)]
pub(crate) struct MonotonicCacheClock {
    epoch: Instant,
}

impl MonotonicCacheClock {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl CacheClock for MonotonicCacheClock {
    fn now_seconds(&self) -> Result<i64, CacheAdapterError> {
        i64::try_from(self.epoch.elapsed().as_secs()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
}

/// A deterministic whole-second clock for adapter and request-driver tests.
#[derive(Clone)]
pub struct CacheTestClock {
    seconds: Rc<Cell<u64>>,
    wall: Rc<Cell<u64>>,
}

impl CacheTestClock {
    #[must_use]
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds: Rc::new(Cell::new(seconds)),
            wall: Rc::new(Cell::new(seconds)),
        }
    }

    pub fn set_wall(&self, seconds: u64) {
        self.wall.set(seconds);
    }

    pub fn set(&self, seconds: u64) {
        self.seconds.set(seconds);
    }

    pub fn advance(&self, seconds: u64) {
        self.seconds.set(self.seconds.get().saturating_add(seconds));
    }
}

impl CacheClock for CacheTestClock {
    fn wall_seconds(&self) -> Result<i64, CacheAdapterError> {
        i64::try_from(self.wall.get()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
    fn now_seconds(&self) -> Result<i64, CacheAdapterError> {
        i64::try_from(self.seconds.get()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
}

/// Errors from the native adapter boundary. A malformed query is a caller
/// error; malformed responses are admission misses rather than a reason to
/// alter the W1 forwarding result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheAdapterError {
    InvalidQuery,
    Cache(CacheError),
    ClockOverflow,
    ExpiryOverflow,
    Dump(String),
    Io(String),
    Closed,
}

impl std::fmt::Display for CacheAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dump(error) => write!(formatter, "invalid cache dump: {error}"),
            Self::Io(error) => write!(formatter, "cache persistence: {error}"),
            Self::Closed => formatter.write_str("cache owner is closed"),
            Self::InvalidQuery => formatter.write_str("invalid cache query"),
            Self::Cache(error) => error.fmt(formatter),
            Self::ClockOverflow => formatter.write_str("cache clock overflow"),
            Self::ExpiryOverflow => formatter.write_str("cache expiry overflow"),
        }
    }
}

impl std::error::Error for CacheAdapterError {}

impl From<CacheError> for CacheAdapterError {
    fn from(error: CacheError) -> Self {
        Self::Cache(error)
    }
}

/// One host-owned adapter around one owned cache-core store.
#[derive(Clone)]
pub struct NativeCacheAdapter {
    cache: Rc<NativeCache>,
    clock: Rc<dyn CacheClock>,
    lazy_ttl: u32,
    enable_ecs: bool,
    exclusions: Rc<IpPrefixList>,
    owner: Rc<CacheOwner>,
    dump_file: Option<std::path::PathBuf>,
    dump_interval: u64,
}

impl NativeCacheAdapter {
    /// Creates the native Phase 5A cache with the reviewed fixed W2 capacity.
    pub fn new() -> Result<Self, CacheAdapterError> {
        Self::with_clock(Rc::new(MonotonicCacheClock::new()))
    }

    /// Creates a cache with an injected clock. The clock is private to this
    /// cache instance, so stored and lookup timestamps cannot mix epochs.
    pub fn with_clock(clock: Rc<dyn CacheClock>) -> Result<Self, CacheAdapterError> {
        Self::with_capacity_and_clock(CACHE_CAPACITY, clock)
    }

    /// Creates a cache with the injected clock and the reviewed lazy window.
    pub fn with_options_and_clock(
        capacity: u64,
        lazy_cache_ttl_secs: u32,
        clock: Rc<dyn CacheClock>,
    ) -> Result<Self, CacheAdapterError> {
        let cache = NativeCache::new(CacheConfig {
            capacity,
            lazy_cache_ttl_secs,
            flags: 0,
        })?;
        Ok(Self {
            cache: Rc::new(cache),
            clock,
            lazy_ttl: lazy_cache_ttl_secs,
            enable_ecs: false,
            owner: Rc::new(CacheOwner::new()),
            dump_file: None,
            dump_interval: 600,
            exclusions: Rc::new({
                let mut list = IpPrefixList::new();
                list.rebuild();
                list
            }),
        })
    }

    /// Creates a cache with the configured positive size. Only the entry
    /// capacity is configurable here; the lazy TTL and eligibility model stay
    /// the reviewed W2 contract.
    pub fn with_capacity_and_clock(
        capacity: u64,
        clock: Rc<dyn CacheClock>,
    ) -> Result<Self, CacheAdapterError> {
        Self::with_options_and_clock(capacity, CACHE_LAZY_TTL_SECS, clock)
    }

    #[must_use]
    pub fn with_ecs(mut self, enabled: bool) -> Self {
        self.enable_ecs = enabled;
        self
    }
    pub fn with_exclusions(mut self, prefixes: &[String]) -> Self {
        let mut list = IpPrefixList::new();
        for prefix in prefixes {
            let valid = prefix.split_once('/').and_then(|(ip, bits)| {
                let ip = ip.parse::<std::net::IpAddr>().ok()?;
                let bits = bits.parse::<u8>().ok()?;
                (bits <= if ip.is_ipv4() { 32 } else { 128 }).then_some((ip, bits))
            });
            if let Some((ip, bits)) = valid {
                list.append(ip, bits);
            } else {
                eprintln!("cache: skipping invalid exclude_ip CIDR {prefix:?}");
            }
        }
        list.rebuild();
        self.exclusions = Rc::new(list);
        self
    }

    /// Wraps this store as a one-entry catalog at [`CacheId(0)`](CacheId).
    ///
    /// This is the single-cache seam used by the request driver unit tests and
    /// by embedders that compile exactly one cache. Production hosts build the
    /// catalog from the compiled configuration instead.
    #[must_use]
    pub fn catalog(&self) -> CacheCatalog {
        CacheCatalog::from_adapters(vec![self.clone()])
    }

    /// Test constructor using a deterministic clock and a fresh store.
    pub fn for_test(clock: CacheTestClock) -> Result<Self, CacheAdapterError> {
        Self::with_clock(Rc::new(clock))
    }

    /// Returns the key bytes used by the in-memory native cache.
    pub fn key_for_query(&self, query: &[u8]) -> Result<Option<Vec<u8>>, CacheAdapterError> {
        key_for_query(query, self.enable_ecs)
    }

    /// Looks up an eligible query and patches the response copy for its ID.
    pub fn lookup(&self, query: &[u8]) -> Result<Option<Vec<u8>>, CacheAdapterError> {
        Ok(self.lookup_entry(query)?.map(|entry| entry.response))
    }

    pub fn lookup_entry(&self, query: &[u8]) -> Result<Option<CacheHit>, CacheAdapterError> {
        self.lookup_entry_counted(query, true)
    }
    pub(crate) fn lookup_entry_counted(
        &self,
        query: &[u8],
        foreground: bool,
    ) -> Result<Option<CacheHit>, CacheAdapterError> {
        if foreground {
            self.owner
                .queries
                .set(self.owner.queries.get().saturating_add(1));
        }

        if self.owner.failed.get() {
            return Err(CacheAdapterError::Closed);
        }
        let Some((key, request_id)) = query_key_and_id(query, self.enable_ecs)? else {
            return Ok(None);
        };
        let now = self.clock.now_seconds()?;
        let Some(lookup) = self.cache.lookup(&key, now)? else {
            return Ok(None);
        };
        if foreground {
            self.owner.hits.set(self.owner.hits.get().saturating_add(1));
            if lookup.state == mosdns_cache_core::LookupState::Lazy {
                self.owner
                    .lazy_hits
                    .set(self.owner.lazy_hits.get().saturating_add(1));
            }
        }
        let response = patch_response_id_ra(&lookup.response, request_id)
            .map_err(|_| CacheAdapterError::InvalidQuery)?;
        Ok(Some(CacheHit {
            response,
            domain_set: String::from_utf8(lookup.domain_set)
                .map_err(|_| CacheAdapterError::InvalidQuery)?,
            state: lookup.state,
        }))
    }

    /// Arms one request-local publication token after an eligible query miss.
    pub fn begin_store(&self, query: &[u8]) -> Result<Option<PendingStore>, CacheAdapterError> {
        let Some((key, _, question)) = query_key_question(query, self.enable_ecs)? else {
            return Ok(None);
        };
        Ok(Some(PendingStore {
            adapter: self.clone(),
            key,
            question,
            generation: self.owner.generation.get(),
        }))
    }

    /// Admits an owner-owned refresh without waiting or queueing. A follower
    /// does not build an execution future or take another concurrency slot.
    pub fn start_refresh<F: Future<Output = ()> + 'static>(
        &self,
        query: &[u8],
        build: impl FnOnce(PendingStore, TransportCancellation, Instant) -> F,
    ) -> Result<bool, CacheAdapterError> {
        let Some(token) = self.begin_store(query)? else {
            return Ok(false);
        };
        if self.owner.stopped.get() || self.owner.failed.get() {
            return Ok(false);
        }
        let key = token.key.clone();
        {
            let mut active = self.owner.active.borrow_mut();
            if active.contains(&key) || active.len() >= 256 {
                return Ok(false);
            }
            active.insert(key.clone());
        }
        let guard = RefreshGuard {
            owner: self.owner.clone(),
            key,
        };
        let cancellation = self.owner.cancellation.child_token();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let work = build(token, cancellation.clone(), deadline);
        let mut tasks = self.owner.tasks.borrow_mut();
        while let Some(result) = tasks.try_join_next() {
            if result.is_err() {
                self.owner.failed.set(true);
            }
        }
        tasks.spawn_local(async move {
            let _guard = guard;
            tokio::select! {
                () = cancellation.cancelled() => {},
                () = tokio::time::sleep_until(deadline.into()) => {},
                () = work => {},
            }
        });
        Ok(true)
    }

    #[must_use]
    pub fn pending_refreshes(&self) -> usize {
        self.owner.active.borrow().len()
    }

    pub fn stop_admission(&self) {
        self.owner.stopped.set(true);
    }

    pub async fn stop_refreshes(&self) -> Result<(), CacheAdapterError> {
        self.owner.stopped.set(true);
        self.owner.cancellation.cancel();
        let mut tasks = std::mem::take(&mut *self.owner.tasks.borrow_mut());
        let mut failed = self.owner.failed.get();
        while let Some(result) = tasks.join_next().await {
            if result.is_err() {
                failed = true;
            }
        }
        if failed {
            return Err(CacheAdapterError::Cache(CacheError::Internal));
        }
        Ok(())
    }

    #[must_use]
    pub fn with_persistence(mut self, path: Option<std::path::PathBuf>, interval: u64) -> Self {
        self.dump_file = path;
        self.dump_interval = interval;
        self
    }

    #[doc(hidden)]
    pub fn inject_persist_fault(&self, fault: crate::managed::PersistFault) {
        self.owner.fault.set(fault);
    }
    #[doc(hidden)]
    pub fn inject_persist_gate(&self, gate: crate::managed::PersistGate) {
        *self.owner.gate.borrow_mut() = Some(gate);
    }
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.owner.revision.get() != self.owner.clean_revision.get()
    }
    fn next_versions(&self) -> Result<(u64, u64), CacheAdapterError> {
        Ok((
            self.owner
                .generation
                .get()
                .checked_add(1)
                .ok_or(CacheAdapterError::ExpiryOverflow)?,
            self.owner
                .revision
                .get()
                .checked_add(1)
                .ok_or(CacheAdapterError::ExpiryOverflow)?,
        ))
    }
    fn prepare_import(
        &self,
        bytes: &[u8],
    ) -> Result<Vec<mosdns_cache_core::PreparedNativeEntry>, CacheAdapterError> {
        let wall = self.clock.wall_seconds()?;
        let now = self.clock.now_seconds()?;
        if wall < 0 {
            return Err(CacheAdapterError::ClockOverflow);
        }
        let mut prepared = Vec::new();
        for mut entry in crate::cache_dump::decode(bytes)? {
            let [stored, msg, cache] = entry.wall_times.ok_or(CacheAdapterError::InvalidQuery)?;
            if stored < 0 || msg < stored || cache < stored || stored > wall {
                return Err(CacheAdapterError::Dump("invalid timestamps".into()));
            }
            let metadata = observe_response_metadata(&entry.response)
                .map_err(|_| CacheAdapterError::InvalidQuery)?;
            let question = metadata
                .question
                .as_ref()
                .ok_or(CacheAdapterError::InvalidQuery)?;
            let flags = *entry.key.first().ok_or(CacheAdapterError::InvalidQuery)?;
            let mut flags_wire = vec![0; 4];
            flags_wire[3] = ((flags & 1) << 5) | ((flags & 2) << 3);
            let base = encode_key(
                &flags_wire,
                &question.qname_wire,
                question.qtype,
                flags & 4 != 0,
            )
            .ok_or(CacheAdapterError::InvalidQuery)?;
            let normalized = normalize_dump_key(&entry.key, &base, self.enable_ecs);
            if flags & !7 != 0
                || question.qclass != CLASS_IN
                || metadata.opcode != 0
                || metadata.truncated
                || metadata.has_opt
                || normalized.is_none()
                || validate_response(&entry.response).is_err()
                || std::str::from_utf8(&entry.domain_set).is_err()
            {
                return Err(CacheAdapterError::Dump(
                    "ineligible key, response or domain_set".into(),
                ));
            }
            entry.key = normalized.ok_or(CacheAdapterError::InvalidQuery)?;
            // Validate even expired entries: malformed data never hides behind expiry.
            let convert = |at: i64| {
                now.checked_add(
                    at.checked_sub(wall)
                        .ok_or(CacheAdapterError::ExpiryOverflow)?,
                )
                .ok_or(CacheAdapterError::ExpiryOverflow)
            };
            entry.times = [convert(stored)?, convert(msg)?, convert(cache)?];
            let expired = cache <= wall || (msg <= wall && self.lazy_ttl == 0);
            let item = mosdns_cache_core::PreparedNativeEntry::new(entry)?;
            if !expired {
                prepared.push(item);
            }
        }
        Ok(prepared)
    }
    pub fn dump(&self) -> Result<Vec<u8>, CacheAdapterError> {
        let entries =
            self.cache
                .snapshot_bounded(self.clock.now_seconds()?, 100_000, 64 * 1024 * 1024)?;
        crate::cache_dump::encode(&entries)
    }

    async fn transaction(&self, operation: ManagementOperation) -> Result<(), CacheAdapterError> {
        if self.owner.stopped.get() || self.owner.failed.get() {
            return Err(CacheAdapterError::Closed);
        }
        let (send, receive) = tokio::sync::oneshot::channel();
        let adapter = self.clone();
        {
            let mut tasks = self.owner.management.borrow_mut();
            while let Some(result) = tasks.try_join_next() {
                if result.is_err() {
                    self.owner.failed.set(true);
                }
            }
            tasks.spawn_local(async move {
                let _guard = adapter.owner.management_gate.lock().await;
                let _fatal = CommitGuard {
                    owner: adapter.owner.clone(),
                };
                let result = adapter.commit_operation(operation).await;
                let _ = send.send(result);
            });
        }
        receive.await.map_err(|_| {
            self.owner.failed.set(true);
            self.owner.stopped.set(true);
            CacheAdapterError::Closed
        })?
    }
    pub async fn import_dump(&self, bytes: Vec<u8>) -> Result<(), CacheAdapterError> {
        self.transaction(ManagementOperation::Import(bytes)).await
    }
    pub async fn flush(&self) -> Result<(), CacheAdapterError> {
        self.transaction(ManagementOperation::Flush).await
    }
    pub async fn save(&self) -> Result<(), CacheAdapterError> {
        self.transaction(ManagementOperation::Save).await
    }
    async fn commit_operation(
        &self,
        operation: ManagementOperation,
    ) -> Result<(), CacheAdapterError> {
        if self.owner.failed.get() {
            return Err(CacheAdapterError::Closed);
        }
        match operation {
            ManagementOperation::Import(bytes) => {
                let entries = self.prepare_import(&bytes)?;
                let (generation, revision) = self.next_versions()?;
                let _guard = CommitGuard {
                    owner: self.owner.clone(),
                };
                self.cache.merge_prepared(entries);
                self.owner.generation.set(generation);
                self.owner.revision.set(revision);
            }
            ManagementOperation::Flush => {
                let (generation, revision) = self.next_versions()?;
                let bytes = crate::cache_dump::encode(&[])?;
                self.owner.blocked.set(true);
                let _guard = CommitGuard {
                    owner: self.owner.clone(),
                };
                if let Some(path) = &self.dump_file {
                    self.persist(path.clone(), bytes).await?;
                }
                // All recoverable operations precede rename. No await/error follows this commit.
                self.cache.flush();
                self.owner.generation.set(generation);
                self.owner.revision.set(revision);
                if self.dump_file.is_some() {
                    self.owner.clean_revision.set(revision);
                }
            }
            ManagementOperation::Save => {
                let path = self
                    .dump_file
                    .clone()
                    .ok_or_else(|| CacheAdapterError::Dump("no dump_file configured".into()))?;
                let revision = self.owner.revision.get();
                let bytes = self.dump()?;
                self.persist(path, bytes).await?;
                // A publication during disk I/O remains dirty.
                self.owner.clean_revision.set(revision);
            }
        }
        Ok(())
    }
    async fn persist(
        &self,
        path: std::path::PathBuf,
        bytes: Vec<u8>,
    ) -> Result<(), CacheAdapterError> {
        let fault = self.owner.fault.replace(crate::managed::PersistFault::None);
        let gate = self.owner.gate.borrow_mut().take();
        tokio::task::spawn_blocking(move || {
            if let Some(gate) = gate {
                gate.wait();
            }
            persist_dump(&path, &bytes, fault)
        })
        .await
        .map_err(|e| CacheAdapterError::Io(e.to_string()))?
        .map_err(|e| CacheAdapterError::Io(e.to_string()))
    }
    async fn load_startup(&self) {
        if self.owner.loaded.replace(true) {
            return;
        }
        let Some(path) = self.dump_file.clone() else {
            return;
        };
        let display = path.display().to_string();
        let read = tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take((crate::cache_dump::MAX_COMPRESSED + 1) as u64)
                .read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        })
        .await;
        let result = match read {
            Ok(Ok(bytes)) => self.prepare_import(&bytes).map(|entries| {
                self.cache.merge_prepared(entries);
            }),
            Ok(Err(error)) => Err(CacheAdapterError::Io(error.to_string())),
            Err(error) => Err(CacheAdapterError::Io(error.to_string())),
        };
        if let Err(error) = result {
            eprintln!("cache startup {display}: {error}; starting empty");
        }
    }
    fn start_periodic(&self) {
        if self.dump_file.is_none()
            || !self.owner.periodic.borrow().is_empty()
            || self.owner.stopped.get()
        {
            return;
        }
        let adapter = self.clone();
        let cancellation = self.owner.cancellation.clone();
        self.owner.periodic.borrow_mut().spawn_local(async move {
            loop {
                tokio::select! {
                    () = cancellation.cancelled() => break,
                    () = async {
                        if let Some(deadline) = Instant::now().checked_add(std::time::Duration::from_secs(adapter.dump_interval)) {
                            tokio::time::sleep_until(deadline.into()).await;
                        } else { std::future::pending::<()>().await; }
                    } => {},
                }
                let _guard = adapter.owner.management_gate.lock().await;
                if adapter.is_dirty() {
                    if let Err(error) = adapter.commit_operation(ManagementOperation::Save).await { eprintln!("cache periodic save: {error}"); }
                }
            }
        });
    }
    async fn finish_persistence(&self) -> Result<(), CacheAdapterError> {
        self.stop_admission();
        self.owner.cancellation.cancel();
        let mut periodic = std::mem::take(&mut *self.owner.periodic.borrow_mut());
        let mut management = std::mem::take(&mut *self.owner.management.borrow_mut());
        let mut failed = self.owner.failed.get();
        while let Some(result) = periodic.join_next().await {
            failed |= result.is_err();
        }
        while let Some(result) = management.join_next().await {
            failed |= result.is_err();
        }
        if failed {
            self.owner.failed.set(true);
            return Err(CacheAdapterError::Closed);
        }
        if self.dump_file.is_some() {
            self.commit_operation(ManagementOperation::Save).await?;
        }
        Ok(())
    }

    /// Returns an owned live snapshot with original wall and runtime timestamps.
    pub fn snapshot(
        &self,
    ) -> Result<Vec<mosdns_cache_core::NativeSnapshotEntry>, CacheAdapterError> {
        Ok(self.cache.snapshot(self.clock.now_seconds()?))
    }

    pub(crate) fn metrics(&self) -> Result<[u64; 4], CacheAdapterError> {
        Ok([
            self.owner.queries.get(),
            self.owner.hits.get(),
            self.owner.lazy_hits.get(),
            self.cache.live_len(self.clock.now_seconds()?) as u64,
        ])
    }
    pub(crate) fn has_dump_file(&self) -> bool {
        self.dump_file.is_some()
    }

    /// Returns the underlying store count after cache-core maintenance.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.cache.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn normalize_dump_key(key: &[u8], base: &[u8], enable_ecs: bool) -> Option<Vec<u8>> {
    if key == base {
        return Some(base.to_vec());
    }
    if !enable_ecs {
        return None;
    }
    let suffix = key.strip_prefix(base)?;
    let (&length, text) = suffix.split_first()?;
    if length == 0 || usize::from(length) != text.len() {
        return None;
    }
    let subnet = crate::ecs::Subnet::dump_string(std::str::from_utf8(text).ok()?)?;
    let text = subnet.key_string();
    let mut normalized = base.to_vec();
    normalized.push(u8::try_from(text.len()).ok()?);
    normalized.extend_from_slice(text.as_bytes());
    Some(normalized)
}

/// One owner for native publication and refresh lifetime. No client token is
/// retained here; each task has an owner child cancellation and independent root.
struct CacheOwner {
    generation: Cell<u64>,
    revision: Cell<u64>,
    queries: Cell<u64>,
    hits: Cell<u64>,
    lazy_hits: Cell<u64>,
    clean_revision: Cell<u64>,
    blocked: Cell<bool>,
    loaded: Cell<bool>,
    fault: Cell<crate::managed::PersistFault>,
    gate: RefCell<Option<crate::managed::PersistGate>>,
    management_gate: Rc<tokio::sync::Mutex<()>>,
    management: RefCell<tokio::task::JoinSet<()>>,
    periodic: RefCell<tokio::task::JoinSet<()>>,
    stopped: Cell<bool>,
    failed: Cell<bool>,
    cancellation: TransportCancellation,
    host_shutdown: RefCell<Option<TransportCancellation>>,
    active: RefCell<BTreeSet<Vec<u8>>>,
    tasks: RefCell<tokio::task::JoinSet<()>>,
}
impl CacheOwner {
    fn new() -> Self {
        Self {
            generation: Cell::new(0),
            revision: Cell::new(0),
            queries: Cell::new(0),
            hits: Cell::new(0),
            lazy_hits: Cell::new(0),
            clean_revision: Cell::new(0),
            blocked: Cell::new(false),
            loaded: Cell::new(false),
            fault: Cell::new(crate::managed::PersistFault::None),
            gate: RefCell::new(None),
            management_gate: Rc::new(tokio::sync::Mutex::new(())),
            management: RefCell::new(tokio::task::JoinSet::new()),
            periodic: RefCell::new(tokio::task::JoinSet::new()),
            stopped: Cell::new(false),
            failed: Cell::new(false),
            cancellation: TransportCancellation::new(),
            host_shutdown: RefCell::new(None),
            active: RefCell::new(BTreeSet::new()),
            tasks: RefCell::new(tokio::task::JoinSet::new()),
        }
    }
}
enum ManagementOperation {
    Import(Vec<u8>),
    Flush,
    Save,
}
struct CommitGuard {
    owner: Rc<CacheOwner>,
}
impl Drop for CommitGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.owner.failed.set(true);
            self.owner.stopped.set(true);
            self.owner.cancellation.cancel();
            if let Some(shutdown) = self.owner.host_shutdown.borrow().as_ref() {
                shutdown.cancel();
            }
        }
        self.owner.blocked.set(false);
    }
}
static DUMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn persist_dump(
    path: &std::path::Path,
    bytes: &[u8],
    fault: crate::managed::PersistFault,
) -> std::io::Result<()> {
    use crate::managed::PersistFault;
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("dump target has no parent"))?;
    let number = DUMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(".mosdns-cache-{}-{number}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| {
        if fault == PersistFault::WriteTemp {
            return Err(std::io::Error::other("injected temporary write failure"));
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        if fault == PersistFault::Rename {
            return Err(std::io::Error::other("injected rename failure"));
        }
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() && created {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
struct RefreshGuard {
    owner: Rc<CacheOwner>,
    key: Vec<u8>,
}
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.owner.active.borrow_mut().remove(&self.key);
    }
}

/// Every host-owned cache store, indexed by the compile-time [`CacheId`].
///
/// One compiled cache owns exactly one store: several named caches never share
/// an instance, and every inline `exec: cache` callsite gets its own. The
/// catalog is immutable once assembly finishes, so a request can hold a shared
/// reference for its whole lifetime without any interior locking.
#[derive(Clone)]
pub struct CacheCatalog {
    entries: Rc<Vec<NativeCacheAdapter>>,
}

impl CacheCatalog {
    /// Builds a catalog whose index is the [`CacheId`] of each adapter.
    #[must_use]
    pub fn from_adapters(entries: Vec<NativeCacheAdapter>) -> Self {
        Self {
            entries: Rc::new(entries),
        }
    }

    /// Resolves one compiled cache identity to its store.
    #[must_use]
    pub fn get(&self, id: CacheId) -> Option<&NativeCacheAdapter> {
        self.entries.get(id.0)
    }

    pub fn stop_admission(&self) {
        for owner in self.entries.iter() {
            owner.stop_admission();
        }
    }

    pub async fn stop_refreshes(&self) -> Result<(), CacheAdapterError> {
        let mut error = None;
        for owner in self.entries.iter() {
            if let Err(failure) = owner.stop_refreshes().await {
                error = Some(failure);
            }
        }
        error.map_or(Ok(()), Err)
    }

    pub(crate) async fn load_startup(&self) {
        for owner in self.entries.iter() {
            owner.load_startup().await;
        }
    }
    pub(crate) fn start_periodic(&self, shutdown: &TransportCancellation) {
        for owner in self.entries.iter() {
            *owner.owner.host_shutdown.borrow_mut() = Some(shutdown.clone());
            owner.start_periodic();
        }
    }
    pub(crate) async fn finish_persistence(&self) -> Result<(), CacheAdapterError> {
        let mut failures = Vec::new();
        for (id, owner) in self.entries.iter().enumerate() {
            if let Err(error) = owner.finish_persistence().await {
                failures.push(format!("cache {id}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CacheAdapterError::Io(failures.join("; ")))
        }
    }
    /// The summed raw store length. This is a maintenance count, not the
    /// live-entry count that management surfaces must report.
    #[must_use]
    pub fn raw_entry_count(&self) -> u64 {
        self.entries.iter().map(NativeCacheAdapter::len).sum()
    }
}

/// One request-owned cache publication token. Dropping it is the normal path
/// for cancellation, failed execution, malformed responses, and local errors.
pub struct CacheHit {
    pub response: Vec<u8>,
    pub domain_set: String,
    pub state: mosdns_cache_core::LookupState,
}

pub struct PendingStore {
    adapter: NativeCacheAdapter,
    key: Vec<u8>,
    question: ResponseQuestion,
    generation: u64,
}

impl PendingStore {
    /// Validates and publishes one successful upstream response. Consuming the
    /// token makes duplicate publication impossible at the API boundary.
    pub fn publish(self, response: &[u8]) -> Result<bool, CacheAdapterError> {
        self.publish_with_domain(response, "")
    }

    pub fn publish_with_domain(
        self,
        response: &[u8],
        domain_set: &str,
    ) -> Result<bool, CacheAdapterError> {
        if self.adapter.owner.blocked.get()
            || self.adapter.owner.failed.get()
            || self.adapter.owner.stopped.get()
            || self.generation != self.adapter.owner.generation.get()
        {
            return Ok(false);
        }
        let next_revision = self
            .adapter
            .owner
            .revision
            .get()
            .checked_add(1)
            .ok_or(CacheAdapterError::ExpiryOverflow)?;
        let Some(metadata) = response_metadata(response) else {
            return Ok(false);
        };
        if !eligible_response(&metadata, &self.question) || validate_response(response).is_err() {
            return Ok(false);
        }
        let addresses = mosdns_dns_core::observe_answer_addresses(response)?;
        if addresses
            .iter()
            .any(|ip| self.adapter.exclusions.contains(*ip))
        {
            return Ok(false);
        }
        // Decode/re-encode when removing OPT: names and known name-bearing RDATA
        // are rebuilt with new compression offsets, never sliced from a packet.
        let clean = if metadata.has_opt {
            let mut message = match Message::from_vec(response) {
                Ok(msg) => msg,
                Err(_) => return Ok(false),
            };
            if message
                .extensions()
                .as_ref()
                .is_some_and(|edns| edns.version() != 0 || edns.rcode_high() != 0)
            {
                return Ok(false);
            }
            *message.extensions_mut() = None;
            match message.to_vec() {
                Ok(wire) => wire,
                Err(_) => return Ok(false),
            }
        } else {
            response.to_vec()
        };
        let ttl = mosdns_dns_core::observe_response_ttl(&clean)?;
        let retention = retention_seconds(&metadata, ttl.minimal_ttl, ttl.record_count);
        let stored_at = self.adapter.clock.now_seconds()?;
        let wall = self.adapter.clock.wall_seconds()?;
        if wall < 0 {
            return Err(CacheAdapterError::ClockOverflow);
        }
        let cache_ttl = if metadata.rcode == RCODE_NOERROR && self.adapter.lazy_ttl > 0 {
            self.adapter.lazy_ttl
        } else {
            retention
        };
        let add = |at: i64, ttl: u32| {
            at.checked_add(i64::from(ttl))
                .ok_or(CacheAdapterError::ExpiryOverflow)
        };
        self.adapter.cache.store_timed(
            &self.key,
            &clean,
            domain_set.as_bytes(),
            [
                stored_at,
                add(stored_at, retention)?,
                add(stored_at, cache_ttl)?,
            ],
            Some([wall, add(wall, retention)?, add(wall, cache_ttl)?]),
        )?;
        self.adapter.owner.revision.set(next_revision);
        Ok(true)
    }
}

fn query_key_and_id(
    query: &[u8],
    enabled: bool,
) -> Result<Option<(Vec<u8>, u16)>, CacheAdapterError> {
    let Some((key, header, _)) = query_key_question(query, enabled)? else {
        return Ok(None);
    };
    Ok(Some((key, header.id)))
}

fn query_key_question(
    query: &[u8],
    enabled: bool,
) -> Result<Option<(Vec<u8>, mosdns_dns_core::QueryHeader, ResponseQuestion)>, CacheAdapterError> {
    let (header, question) = mosdns_dns_core::parse_query(query).map_err(|error| match error {
        QueryError::Parse(_) | QueryError::Unsupported(_) => CacheAdapterError::InvalidQuery,
    })?;
    if question.qclass != CLASS_IN {
        return Ok(None);
    }
    let opt = match crate::ecs::query_opt(query) {
        Ok(opt) => opt,
        Err(_) => return Ok(None),
    };
    if !enabled && opt.as_ref().is_some_and(|opt| opt.ecs.is_some()) {
        return Ok(None);
    }
    let do_bit = opt.as_ref().is_some_and(|opt| opt.fixed[7] & 0x80 != 0);
    let Some(mut key) = encode_key(query, &question.qname_wire, question.qtype, do_bit) else {
        return Ok(None);
    };
    if let Some(ecs) = opt.and_then(|opt| opt.ecs) {
        let suffix = ecs.key_string();
        key.push(u8::try_from(suffix.len()).map_err(|_| CacheAdapterError::InvalidQuery)?);
        key.extend_from_slice(suffix.as_bytes());
    }
    Ok(Some((
        key,
        header,
        ResponseQuestion {
            qname_wire: question.qname_wire,
            qtype: question.qtype,
            qclass: question.qclass,
        },
    )))
}

fn key_for_query(query: &[u8], enabled: bool) -> Result<Option<Vec<u8>>, CacheAdapterError> {
    Ok(query_key_question(query, enabled)?.map(|(key, _, _)| key))
}

/// Product text form follows miekg/dns label escaping, retaining case and root dot.
fn product_name(wire: &[u8]) -> Option<String> {
    let mut name = String::new();
    let mut pos = 0;
    loop {
        let length = usize::from(*wire.get(pos)?);
        pos += 1;
        if length == 0 {
            if name.is_empty() {
                name.push('.');
            }
            return Some(name);
        }
        for &byte in wire.get(pos..pos + length)? {
            if b". '@;()\"\\".contains(&byte) {
                name.push('\\');
                name.push(char::from(byte));
            } else if !(b' '..=b'~').contains(&byte) {
                use std::fmt::Write;
                let _ = write!(name, "\\{byte:03}");
            } else {
                name.push(char::from(byte));
            }
        }
        name.push('.');
        pos += length;
    }
}

fn encode_key(query: &[u8], qname_wire: &[u8], qtype: u16, do_bit: bool) -> Option<Vec<u8>> {
    let name = product_name(qname_wire)?;
    let length = u8::try_from(name.len()).ok()?;
    let flags = u8::from(query[3] & 0x20 != 0)
        | (u8::from(query[3] & 0x10 != 0) << 1)
        | (u8::from(do_bit) << 2);
    let mut key = vec![flags];
    key.extend_from_slice(&qtype.to_be_bytes());
    key.push(length);
    key.extend_from_slice(name.as_bytes());
    Some(key)
}

fn response_metadata(response: &[u8]) -> Option<ResponseMetadata> {
    observe_response_metadata(response).ok()
}

fn eligible_response(metadata: &ResponseMetadata, question: &ResponseQuestion) -> bool {
    metadata.opcode == 0 && !metadata.truncated && metadata.question.as_ref() == Some(question)
}

fn retention_seconds(metadata: &ResponseMetadata, minimal_ttl: u32, record_count: u32) -> u32 {
    match metadata.rcode {
        RCODE_NXDOMAIN => RETENTION_NXDOMAIN_SECS,
        RCODE_SERVFAIL => RETENTION_FALLBACK_SECS,
        RCODE_NOERROR if record_count > 0 && minimal_ttl > 0 => {
            if metadata.answer_count == 0 {
                minimal_ttl.min(RETENTION_MAX_NOERROR_SECS)
            } else {
                minimal_ttl
            }
        }
        _ => RETENTION_FALLBACK_SECS,
    }
}

impl From<ResponseError> for CacheAdapterError {
    fn from(_: ResponseError) -> Self {
        Self::InvalidQuery
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    fn query_response() -> (Vec<u8>, Vec<u8>) {
        let query = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 0, 0, 1, 0, 1];
        let mut response = query.clone();
        response[2] = 0x81;
        response[3] = 0x80;
        response[7] = 1;
        response.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1]);
        (query, response)
    }
    #[test]
    fn ecs_suffix_validation_is_atomic_even_for_expired_last_entries() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, async {
            let clock = CacheTestClock::new(1000);
            let cache = NativeCacheAdapter::for_test(clock).unwrap().with_ecs(true);
            let (query, response) = query_response();
            cache
                .begin_store(&query)
                .unwrap()
                .unwrap()
                .publish_with_domain(&response, "unchanged")
                .unwrap();
            let original = cache.snapshot().unwrap().pop().unwrap();
            for text in [
                "192.0.2.1/33/0",
                "192.0.2.1/24/1",
                "[::1]/129/0",
                "::1/64/0",
                "192.0.2.1/24/0/junk",
                "192.0.2.1/+24/0",
                "[invalid]/64/0",
                "",
                "192.0.2.1/95/0",
            ] {
                let mut bad = original.clone();
                bad.wall_times = Some([1, 2, 3]);
                bad.key.push(u8::try_from(text.len()).unwrap());
                bad.key.extend_from_slice(text.as_bytes());
                let bytes = crate::cache_dump::encode(&[original.clone(), bad]).unwrap();
                assert!(cache.import_dump(bytes).await.is_err(), "{text}");
                assert_eq!(cache.snapshot().unwrap()[0].domain_set, b"unchanged");
            }
            for length in [0, 1, 255] {
                let mut bad = original.clone();
                bad.key.push(length);
                bad.key.extend_from_slice(b"192.0.2.1/24/0");
                assert!(
                    cache
                        .import_dump(crate::cache_dump::encode(&[original.clone(), bad]).unwrap())
                        .await
                        .is_err()
                );
            }
            let old = cache.begin_store(&query).unwrap().unwrap();
            cache.flush().await.unwrap();
            assert!(!old.publish(&response).unwrap());
            assert!(cache.snapshot().unwrap().is_empty());
        });
    }
    #[test]
    fn all_entry_validation_precedes_merge_including_expired_and_ecs_keys() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, async {
            let clock = CacheTestClock::new(1000);
            let cache = NativeCacheAdapter::for_test(clock).unwrap();
            let (query, response) = query_response();
            cache
                .begin_store(&query)
                .unwrap()
                .unwrap()
                .publish_with_domain(&response, "unchanged")
                .unwrap();
            let original = cache.snapshot().unwrap().pop().unwrap();
            for variant in 0..7 {
                let mut bad = original.clone();
                match variant {
                    0 => bad.key.push(0),                           // ECS suffix/extra key bytes
                    1 => bad.key[0] = 128,                          // reserved flags
                    2 => bad.key[4] = b'b',                         // mismatched question
                    3 => bad.wall_times = Some([1001, 1060, 1060]), // future stored
                    4 => bad.wall_times = Some([1000, 999, 1060]),  // inverted expiry
                    5 => {
                        bad.wall_times = Some([1, 2, 3]);
                        bad.response.clear();
                    } // expired malformed wire
                    _ => bad.domain_set = vec![255],
                }
                let old = cache.begin_store(&query).unwrap().unwrap();
                let bytes = crate::cache_dump::encode(&[original.clone(), bad]).unwrap();
                assert!(cache.import_dump(bytes).await.is_err());
                assert_eq!(
                    cache.lookup_entry(&query).unwrap().unwrap().domain_set,
                    "unchanged"
                );
                assert!(old.publish_with_domain(&response, "unchanged").unwrap());
            }
        });
    }
    #[test]
    fn post_commit_invariant_failure_stops_host_and_never_saves_old_memory() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, async {
            let path =
                std::env::temp_dir().join(format!("native-owner-fatal-{}.gz", std::process::id()));
            let cache = NativeCacheAdapter::for_test(CacheTestClock::new(1000))
                .unwrap()
                .with_persistence(Some(path.clone()), 600);
            let (query, response) = query_response();
            cache
                .begin_store(&query)
                .unwrap()
                .unwrap()
                .publish(&response)
                .unwrap();
            cache.save().await.unwrap();
            let shutdown = TransportCancellation::new();
            let catalog = cache.catalog();
            catalog.start_periodic(&shutdown);
            let owner = cache.clone();
            cache.owner.management.borrow_mut().spawn_local(async move {
                let _fatal = CommitGuard {
                    owner: owner.owner.clone(),
                };
                owner
                    .persist(
                        owner.dump_file.clone().unwrap(),
                        crate::cache_dump::encode(&[]).unwrap(),
                    )
                    .await
                    .unwrap();
                panic!("injected post-commit invariant failure");
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), shutdown.cancelled())
                .await
                .unwrap();
            assert!(cache.lookup(&query).is_err());
            assert!(catalog.finish_persistence().await.is_err());
            assert!(
                crate::cache_dump::decode(&std::fs::read(&path).unwrap())
                    .unwrap()
                    .is_empty()
            );
            std::fs::remove_file(path).unwrap();
        });
    }
    #[test]
    fn startup_periodic_shutdown_snapshot_and_all_owner_errors_are_observable() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, async {
            let path = std::env::temp_dir()
                .join(format!("native-owner-persist-{}.gz", std::process::id()));
            let owner = NativeCacheAdapter::for_test(CacheTestClock::new(1000))
                .unwrap()
                .with_persistence(Some(path.clone()), 1);
            let (query, response) = query_response();
            let catalog = owner.catalog();
            catalog.load_startup().await;
            catalog.start_periodic(&TransportCancellation::new());
            owner
                .begin_store(&query)
                .unwrap()
                .unwrap()
                .publish_with_domain(&response, "persisted-domain")
                .unwrap();
            for _ in 0..1500 {
                if path.exists() && !owner.is_dirty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            assert!(path.exists());
            assert!(!owner.is_dirty());
            let pending = owner.begin_store(&query).unwrap().unwrap();
            catalog.stop_admission();
            catalog.stop_refreshes().await.unwrap();
            catalog.finish_persistence().await.unwrap();
            assert!(!pending.publish(&response).unwrap());
            let restart = NativeCacheAdapter::for_test(CacheTestClock::new(1010))
                .unwrap()
                .with_persistence(Some(path.clone()), 600);
            restart.catalog().load_startup().await;
            assert_eq!(
                restart.lookup_entry(&query).unwrap().unwrap().domain_set,
                "persisted-domain"
            );
            assert_eq!(
                restart.snapshot().unwrap()[0].wall_times,
                Some([1000, 1060, 1060])
            );
            std::fs::write(&path, b"corrupt").unwrap();
            let bad = NativeCacheAdapter::for_test(CacheTestClock::new(1000))
                .unwrap()
                .with_persistence(Some(path.clone()), 600);
            bad.catalog().load_startup().await;
            assert!(bad.snapshot().unwrap().is_empty());
            let first = owner
                .clone()
                .with_persistence(Some(path.with_extension("missing").join("one.gz")), 600);
            let second = restart
                .clone()
                .with_persistence(Some(path.with_extension("missing").join("two.gz")), 600);
            let failures = CacheCatalog::from_adapters(vec![first, second])
                .finish_persistence()
                .await
                .unwrap_err()
                .to_string();
            assert!(failures.contains("cache 0:"));
            assert!(failures.contains("cache 1:"));
            std::fs::remove_file(path).unwrap();
        });
    }
}
