//! Native rule-source management (M1-a2): the online sources of
//! `sd_set_light`, `si_set` and `adguard_rule`, the local list of
//! `domain_set_light`, and the hot rebuild of the `domain_mapper`s and direct
//! matchers that consume them.
//!
//! One `RuleSources` belongs to each compiled configuration. It keeps every
//! provider's manifest in memory (as the reference providers do), serializes
//! each provider's file and manifest writes, downloads with a bounded HTTP
//! client, and republishes query-side state as whole generations built on the
//! blocking pool. Background work runs as tracked jobs that shutdown cancels
//! and joins. Behavior follows the reference handlers; deliberate differences
//! are listed in `docs/rust/contracts/native-rule-providers.md`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use mosdns_matcher_core::{IpPrefixList, ValuedDomainMatcher, decode_domain_srs, decode_ip_srs};
use mosdns_upstream_core::TransportCancellation;
use serde::Serialize;

use crate::config::providers::{
    self, Counts, Filtered, LightSpec, ManifestSpec, MapperSpec, RuleEntry, Schema, SourceEntry,
};
use crate::managed::PersistGate;
use crate::policy::IpListHandle;

/// A value read whole at each use and replaced whole on publication. A reader
/// holding the previous `Rc` keeps using that generation until it drops it.
pub(crate) struct Live<T>(RefCell<Rc<T>>);

impl<T> Live<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(RefCell::new(Rc::new(value)))
    }

    pub(crate) fn get(&self) -> Rc<T> {
        Rc::clone(&self.0.borrow())
    }

    /// Publishes `value` and returns the replaced generation.
    pub(crate) fn replace(&self, value: Rc<T>) -> Rc<T> {
        std::mem::replace(&mut *self.0.borrow_mut(), value)
    }
}

/// Download and scheduler timings. The defaults are the reference values;
/// tests shorten them.
#[derive(Clone, Debug)]
pub struct RuleTiming {
    /// Delay before the first auto-update check after the host starts.
    pub first_check: Duration,
    /// `sd_set_light`/`si_set` check period.
    pub source_period: Duration,
    /// `adguard_rule` check period.
    pub adguard_period: Duration,
    /// Overall download deadline for `sd_set_light`/`si_set`.
    pub source_timeout: Duration,
    /// Overall download deadline for `adguard_rule`.
    pub adguard_timeout: Duration,
    /// Largest accepted download body.
    pub max_download_bytes: usize,
}

impl Default for RuleTiming {
    fn default() -> Self {
        Self {
            first_check: Duration::from_secs(60),
            source_period: Duration::from_secs(600),
            adguard_period: Duration::from_secs(60),
            source_timeout: Duration::from_secs(60),
            adguard_timeout: Duration::from_secs(30),
            max_download_bytes: 64 * 1024 * 1024,
        }
    }
}

/// The reference zero `time.Time`, as Go prints it.
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";
/// Seconds of the zero time relative to the Unix epoch.
const ZERO_UNIX: i64 = -62_135_596_800;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Light,
    Sd,
    Si,
    Adguard,
}

/// One rebuild target's state: a request while `Running` marks it `Dirty`
/// and the running job builds once more with fresh inputs.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Build {
    Idle,
    Running,
    Dirty,
}

struct ManifestOwner {
    spec: ManifestSpec,
    sources: RefCell<Vec<SourceEntry>>,
    unreadable: bool,
    /// Counts read at compile time, reconciled into the manifest at start.
    loaded_counts: RefCell<Counts>,
    /// Single flight per source key: `true` asks the running download to run
    /// once more when it finishes.
    flights: RefCell<std::collections::BTreeMap<String, bool>>,
}

/// One rule provider and its management state.
pub(crate) struct Provider {
    pub(crate) tag: String,
    pub(crate) kind: Kind,
    light: Option<LightSpec>,
    manifest: Option<ManifestOwner>,
    /// Serializes every manifest and rule-file write of this provider. The
    /// lock is shared by every generation that owns the same file (see
    /// `write_lock`), so a retired owner can never interleave with its successor.
    mutate: Arc<tokio::sync::Mutex<()>>,
    ip: Option<Rc<IpListHandle>>,
    filtered: RefCell<Option<Rc<Live<Filtered>>>>,
    reload: Cell<Build>,
}

impl Provider {
    pub(crate) fn light(spec: LightSpec) -> Self {
        let lock = write_lock(spec.writable_file().map_or_else(
            || PathBuf::from(format!("light:{}", spec.tag)),
            Path::to_path_buf,
        ));
        Self::new(spec.tag.clone(), Kind::Light, Some(spec), None, None, lock)
    }

    pub(crate) fn manifest(
        spec: ManifestSpec,
        manifest: providers::Manifest,
        counts: Counts,
        ip: Option<Rc<IpListHandle>>,
    ) -> Self {
        let kind = match spec.schema {
            Schema::Sd => Kind::Sd,
            Schema::Si => Kind::Si,
            Schema::Adguard => Kind::Adguard,
        };
        let owner = ManifestOwner {
            spec: spec.clone(),
            sources: RefCell::new(manifest.entries),
            unreadable: manifest.unreadable,
            loaded_counts: RefCell::new(counts),
            flights: RefCell::new(std::collections::BTreeMap::new()),
        };
        let lock = write_lock(spec.manifest.clone());
        Self::new(spec.tag, kind, None, Some(owner), ip, lock)
    }

    fn new(
        tag: String,
        kind: Kind,
        light: Option<LightSpec>,
        manifest: Option<ManifestOwner>,
        ip: Option<Rc<IpListHandle>>,
        mutate: Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        Self {
            tag,
            kind,
            light,
            manifest,
            mutate,
            ip,
            filtered: RefCell::new(None),
            reload: Cell::new(Build::Idle),
        }
    }

    pub(crate) fn attach_filtered(&self, live: Rc<Live<Filtered>>) {
        *self.filtered.borrow_mut() = Some(live);
    }

    /// True when `/post` and `/save` can write this `domain_set_light`.
    pub(crate) fn postable(&self) -> bool {
        self.light
            .as_ref()
            .is_some_and(|spec| spec.writable_file().is_some())
    }

    fn owner(&self) -> &ManifestOwner {
        self.manifest.as_ref().expect("manifest provider")
    }

    fn key(&self, entry: &SourceEntry) -> String {
        if self.kind == Kind::Adguard {
            entry.id.clone()
        } else {
            entry.name.clone()
        }
    }

    fn find(&self, key: &str) -> Option<SourceEntry> {
        self.owner()
            .sources
            .borrow()
            .iter()
            .find(|entry| self.key(entry) == key)
            .cloned()
    }

    fn load_plan(&self) -> Plan {
        match (&self.light, &self.manifest) {
            (Some(spec), _) => Plan::Light(spec.clone()),
            (None, Some(owner)) => {
                Plan::Manifest(owner.spec.clone(), owner.sources.borrow().clone())
            }
            (None, None) => unreachable!("a provider is a list or a manifest"),
        }
    }
}

/// The `Send` inputs of one provider for a rebuild on the blocking pool.
enum Plan {
    Light(LightSpec),
    Manifest(ManifestSpec, Vec<SourceEntry>),
}

impl Plan {
    /// The rules a `domain_mapper` consumes from this provider.
    fn rules(&self) -> Result<providers::ProviderRules, String> {
        match self {
            Self::Light(spec) => providers::load_light(spec, None)
                .map(Rc::new)
                .map_err(|e| e.to_string()),
            Self::Manifest(spec, sources) => Ok(match spec.schema {
                Schema::Sd => Rc::new(providers::load_sd(spec, sources, None).0),
                Schema::Adguard => providers::load_adguard(spec, sources, false, None).0.deny,
                Schema::Si => Rc::new(Vec::new()),
            }),
        }
    }
}

/// One `domain_mapper` and the generation its executor reads.
pub(crate) struct MapperSlot {
    spec: MapperSpec,
    live: Rc<Live<ValuedDomainMatcher>>,
    build: Cell<Build>,
}

impl MapperSlot {
    pub(crate) fn new(spec: MapperSpec, live: Rc<Live<ValuedDomainMatcher>>) -> Self {
        Self {
            spec,
            live,
            build: Cell::new(Build::Idle),
        }
    }
}

struct Inner {
    providers: Vec<Rc<Provider>>,
    mappers: Vec<Rc<MapperSlot>>,
    build_gate: RefCell<Option<PersistGate>>,
    builds: AtomicUsize,
}

#[derive(Clone, Default)]
struct Env {
    roots: Option<Arc<rustls::RootCertStore>>,
    timing: RuleTiming,
}

/// What one background job needs.
#[derive(Clone)]
struct Ctx {
    inner: Rc<Inner>,
    cancel: TransportCancellation,
    env: Env,
}

/// The management state of every rule provider of one compiled configuration.
pub struct RuleSources {
    inner: Rc<Inner>,
    env: RefCell<Env>,
    jobs: RefCell<tokio::task::JoinSet<()>>,
    cancel: TransportCancellation,
    closed: Cell<bool>,
}

impl Default for RuleSources {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

/// An HTTP reply for the management API.
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) content_type: Option<&'static str>,
    pub(crate) body: Vec<u8>,
    /// Headers beyond `Content-Type` and `Content-Length`.
    pub(crate) headers: Vec<(&'static str, String)>,
}

const JSON_UTF8: &str = "application/json; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

impl Reply {
    fn json<T: Serialize>(status: u16, content_type: &'static str, value: &T) -> Self {
        let mut body = serde_json::to_vec(value).expect("small API response serializes");
        body.push(b'\n');
        Self {
            status,
            content_type: Some(content_type),
            body,
            headers: Vec::new(),
        }
    }

    fn json_error(status: u16, message: &str) -> Self {
        Self::json(status, JSON_UTF8, &serde_json::json!({ "error": message }))
    }

    fn text(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: Some(TEXT),
            body: body.into_bytes(),
            headers: Vec::new(),
        }
    }

    /// Go's `http.Error`.
    fn error(status: u16, message: &str) -> Self {
        Self::text(status, format!("{message}\n"))
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    fn not_found() -> Self {
        Self::error(404, "404 page not found")
    }
}

impl RuleSources {
    pub(crate) fn new(providers: Vec<Rc<Provider>>, mappers: Vec<Rc<MapperSlot>>) -> Self {
        Self {
            inner: Rc::new(Inner {
                providers,
                mappers,
                build_gate: RefCell::new(None),
                builds: AtomicUsize::new(0),
            }),
            env: RefCell::new(Env::default()),
            jobs: RefCell::new(tokio::task::JoinSet::new()),
            cancel: TransportCancellation::new(),
            closed: Cell::new(false),
        }
    }

    /// Applies the host's trust roots and timings.
    pub(crate) fn configure(&self, roots: Option<Arc<rustls::RootCertStore>>, timing: RuleTiming) {
        *self.env.borrow_mut() = Env { roots, timing };
    }

    pub(crate) fn timing(&self) -> RuleTiming {
        self.env.borrow().timing.clone()
    }

    pub(crate) fn roots(&self) -> Option<Arc<rustls::RootCertStore>> {
        self.env.borrow().roots.clone()
    }

    pub(crate) fn provider(&self, tag: &str) -> Option<Rc<Provider>> {
        self.inner
            .providers
            .iter()
            .find(|provider| provider.tag == tag)
            .cloned()
    }

    pub(crate) fn has_kind(&self, kinds: &[Kind]) -> bool {
        self.inner
            .providers
            .iter()
            .any(|provider| kinds.contains(&provider.kind))
    }

    /// Every rule file a provider of this configuration still reads.
    fn files_in_use(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for provider in &self.inner.providers {
            if let Some(spec) = &provider.light {
                files.extend(spec.files.iter().map(|(_, path)| path.clone()));
                continue;
            }
            let owner = provider.owner();
            files.extend(
                owner
                    .sources
                    .borrow()
                    .iter()
                    .filter(|entry| provider.kind == Kind::Adguard || !entry.files.is_empty())
                    .map(|entry| owner.spec.source_file(entry)),
            );
        }
        files
    }

    pub(crate) fn has_postable_light(&self) -> bool {
        self.inner.providers.iter().any(|p| p.postable())
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            inner: Rc::clone(&self.inner),
            cancel: self.cancel.clone(),
            env: self.env.borrow().clone(),
        }
    }

    fn spawn(&self, job: impl std::future::Future<Output = ()> + 'static) {
        if self.closed.get() {
            return;
        }
        let mut jobs = self.jobs.borrow_mut();
        while jobs.try_join_next().is_some() {}
        jobs.spawn_local(job);
    }

    /// Stops this generation from starting new jobs or writes, at once. Called
    /// when a newer view is installed, before it admits any request; `close`
    /// later joins what is still running.
    pub(crate) fn fence(&self) {
        self.closed.set(true);
        self.cancel.cancel();
    }

    /// Cancels every background job (in-flight downloads stop at once) and
    /// joins them; a build already running on the blocking pool is awaited.
    pub(crate) async fn close(&self) {
        self.closed.set(true);
        self.cancel.cancel();
        let mut jobs = std::mem::take(&mut *self.jobs.borrow_mut());
        while jobs.join_next().await.is_some() {}
    }

    /// Start-up work the reference does while loading: rewrite stale rule
    /// counts, and download enabled AdGuard lists that are not on disk yet.
    pub(crate) fn start(&self) {
        let providers: Vec<_> = self
            .inner
            .providers
            .iter()
            .filter(|provider| provider.manifest.is_some())
            .cloned()
            .collect();
        if providers.is_empty() {
            return;
        }
        let ctx = self.ctx();
        self.spawn(async move {
            for provider in providers {
                let counts = std::mem::take(&mut *provider.owner().loaded_counts.borrow_mut());
                apply_counts(&ctx, &provider, counts).await;
                if provider.kind != Kind::Adguard {
                    continue;
                }
                let owner = provider.owner();
                let candidates: Vec<(String, PathBuf)> = owner
                    .sources
                    .borrow()
                    .iter()
                    .filter(|entry| entry.enabled)
                    .map(|entry| (entry.id.clone(), owner.spec.source_file(entry)))
                    .collect();
                let mut missing = Vec::new();
                for (id, file) in candidates {
                    let exists = tokio::task::spawn_blocking(move || file.exists())
                        .await
                        .unwrap_or(true);
                    if !exists {
                        missing.push(id);
                    }
                }
                if !missing.is_empty() && download_many(&ctx, &provider, missing).await {
                    changed(&ctx, &provider).await;
                }
            }
        });
    }

    /// Runs the reference auto-update predicate for the providers of `kinds`
    /// at `now` (Unix seconds) and downloads every due source.
    pub(crate) fn auto_update(&self, kinds: &[Kind], now: i64) {
        for provider in &self.inner.providers {
            if !kinds.contains(&provider.kind) || provider.manifest.is_none() {
                continue;
            }
            let due: Vec<String> = provider
                .owner()
                .sources
                .borrow()
                .iter()
                .filter(|entry| is_due(entry, now))
                .map(|entry| provider.key(entry))
                .collect();
            if due.is_empty() {
                continue;
            }
            let ctx = self.ctx();
            let provider = Rc::clone(provider);
            self.spawn(async move {
                if download_many(&ctx, &provider, due).await {
                    changed(&ctx, &provider).await;
                }
            });
        }
    }

    /// Arms a test-only gate that parks every rebuild on the blocking pool.
    #[doc(hidden)]
    pub fn inject_build_gate(&self, gate: PersistGate) {
        *self.inner.build_gate.borrow_mut() = Some(gate);
    }

    /// The number of mapper/provider builds completed so far (tests).
    #[doc(hidden)]
    #[must_use]
    pub fn completed_builds(&self) -> usize {
        self.inner.builds.load(Ordering::SeqCst)
    }

    /// Waits until no background job is left (tests).
    #[doc(hidden)]
    pub async fn settle(&self) {
        loop {
            {
                let mut jobs = self.jobs.borrow_mut();
                while jobs.try_join_next().is_some() {}
                if jobs.is_empty() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Serves `/plugins/{tag}/{rest}` for a rule provider. `rest` is the raw
    /// path after the tag (no query); `query` is the raw query string.
    pub(crate) async fn handle(
        &self,
        provider: &Rc<Provider>,
        method: &str,
        rest: &str,
        query: &str,
        body: &[u8],
    ) -> Reply {
        // A view retired by a managed apply no longer changes anything.
        if self.closed.get() && method != "GET" {
            return Reply::error(503, "host is applying configuration");
        }
        let (action, item) = match rest.split_once('/') {
            Some((action, item)) => (action, Some(item)),
            None => (rest, None),
        };
        let item = match item {
            None => None,
            Some(raw) if raw.is_empty() || raw.contains('/') => return Reply::not_found(),
            Some(raw) => match crate::api::decode_plugin_tag(raw) {
                Some(decoded) => Some(decoded.into_owned()),
                None => return Reply::not_found(),
            },
        };
        match (provider.kind, action, item) {
            (Kind::Light, "show", None) => match method {
                "GET" => self.light_show(provider, query).await,
                _ => Reply::empty(405),
            },
            (Kind::Light, "save", None) => match method {
                "GET" => self.light_save(provider).await,
                _ => Reply::empty(405),
            },
            (Kind::Light, "post", None) => match method {
                "POST" => self.light_post(provider, body).await,
                _ => Reply::empty(405),
            },
            (Kind::Sd | Kind::Si, "config", None) => match method {
                "GET" => self.list(provider),
                _ => Reply::empty(405),
            },
            (Kind::Sd | Kind::Si, "config", Some(name)) => match method {
                "PUT" => self.put_source(provider, name, body).await,
                "DELETE" => self.delete_source(provider, &name).await,
                _ => Reply::empty(405),
            },
            (Kind::Sd | Kind::Si, "update", Some(name)) => match method {
                "POST" => self.update_source(provider, name),
                _ => Reply::empty(405),
            },
            (Kind::Adguard, "rules", None) => match method {
                "GET" => self.list(provider),
                "POST" => self.create_rule(provider, body).await,
                _ => Reply::empty(405),
            },
            (Kind::Adguard, "rules", Some(id)) => match method {
                "PUT" => self.put_rule(provider, &id, body).await,
                "DELETE" => self.delete_source(provider, &id).await,
                _ => Reply::empty(405),
            },
            (Kind::Adguard, "update", None) => match method {
                "POST" => self.update_all_rules(provider),
                _ => Reply::empty(405),
            },
            (Kind::Adguard, "update", Some(id)) => match method {
                "POST" => self.update_rule(provider, id),
                _ => Reply::empty(405),
            },
            _ => Reply::not_found(),
        }
    }

    // ----- domain_set_light -------------------------------------------------

    async fn light_show(&self, provider: &Provider, query: &str) -> Reply {
        let spec = provider.light.clone().expect("light provider");
        let rules = match on_build_thread(move || providers::load_light(&spec, None)).await {
            Ok(Ok(rules)) => rules,
            Ok(Err(error)) => return Reply::error(500, &error.to_string()),
            Err(error) => return Reply::error(500, &error.to_string()),
        };
        let mut search = String::new();
        let (mut limit, mut offset) = (0_i64, 0_i64);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode_query(value);
            match key {
                "q" if search.is_empty() => search = value.to_lowercase(),
                "limit" if limit == 0 => limit = value.parse().unwrap_or(0),
                "offset" if offset == 0 => offset = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        let limit = if limit <= 0 { 100 } else { limit };
        let offset = offset.max(0);
        let mut body = String::new();
        let (mut matched, mut sent) = (0_i64, 0_i64);
        for entry in &rules {
            if !search.is_empty() && !entry.rule.to_lowercase().contains(&search) {
                continue;
            }
            matched += 1;
            if matched <= offset {
                continue;
            }
            body.push_str(&entry.rule);
            body.push('\n');
            sent += 1;
            if sent >= limit {
                break;
            }
        }
        Reply::text(200, body)
    }

    async fn light_save(&self, provider: &Provider) -> Reply {
        let spec = provider.light.clone().expect("light provider");
        if spec.files.is_empty() || spec.files[0].0 != 0 {
            return Reply::error(500, "no file configured");
        }
        if spec.writable_file().is_none() {
            return Reply::error(400, "no txt file configured, cannot post");
        }
        let _guard = provider.mutate.lock().await;
        let result = on_build_thread(move || {
            let rules = providers::load_light(&spec, None).map_err(|e| e.to_string())?;
            let file = spec.writable_file().expect("checked").to_path_buf();
            write_atomically(&file, &rule_lines(rules.iter().map(|e| e.rule.as_str())))
                .map_err(|e| e.to_string())
        })
        .await;
        match result {
            Ok(Ok(())) => Reply::empty(200),
            Ok(Err(error)) => Reply::error(500, &error),
            Err(error) => Reply::error(500, &error.to_string()),
        }
    }

    async fn light_post(&self, provider: &Rc<Provider>, body: &[u8]) -> Reply {
        #[derive(serde::Deserialize)]
        struct Payload {
            #[serde(default)]
            values: Option<Vec<String>>,
        }
        let Ok(payload) = serde_json::from_slice::<Payload>(body) else {
            return Reply::error(400, "invalid JSON");
        };
        let spec = provider.light.clone().expect("light provider");
        let Some(file) = spec.writable_file().map(Path::to_path_buf) else {
            return Reply::error(400, "no txt file configured, cannot post");
        };
        // Values are normalized like file lines (the approved `domain_set` A1
        // rule), so the published list equals what a restart reads back.
        let values: Vec<String> = payload
            .values
            .unwrap_or_default()
            .into_iter()
            .filter(|value| !value.contains('\n') && !value.contains('\r'))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty() && !value.starts_with('#'))
            .collect();
        if let Some(reason) = values
            .iter()
            .find_map(|rule| providers::rule_refusal(&provider.tag, rule))
        {
            return Reply::error(400, &reason);
        }
        let count = values.len();
        let guard = provider.mutate.lock().await;
        let bytes = rule_lines(values.iter().map(String::as_str));
        match tokio::task::spawn_blocking(move || write_atomically(&file, &bytes)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Reply::error(500, &error.to_string()),
            Err(error) => return Reply::error(500, &error.to_string()),
        }
        drop(guard);
        let ctx = self.ctx();
        let provider = Rc::clone(provider);
        self.spawn(async move { changed(&ctx, &provider).await });
        Reply::text(
            200,
            format!("domain_set_light replaced with {count} entries"),
        )
    }

    // ----- sd_set_light / si_set / adguard_rule -----------------------------

    fn list(&self, provider: &Provider) -> Reply {
        let mut sources = provider.owner().sources.borrow().clone();
        sources.sort_by(|a, b| a.name.cmp(&b.name));
        if provider.kind == Kind::Adguard {
            let views: Vec<_> = sources.iter().map(|e| rule_view(e, false)).collect();
            Reply::json(200, "application/json", &views)
        } else {
            let views: Vec<_> = sources
                .iter()
                .map(|e| source_view(e, provider.kind))
                .collect();
            Reply::json(200, JSON_UTF8, &views)
        }
    }

    async fn put_source(&self, provider: &Rc<Provider>, name: String, body: &[u8]) -> Reply {
        let schema = provider.owner().spec.schema;
        let Ok(request) = providers::parse_source_object(body, schema) else {
            return Reply::json_error(400, "invalid request body");
        };
        if provider.kind == Kind::Sd
            && (request.files.trim().is_empty() || request.url.trim().is_empty())
        {
            return Reply::json_error(400, "'files' and 'url' fields are required");
        }
        if provider.kind == Kind::Sd && request.enable_regexp {
            // Turning regexps on must not load a rule this host refuses at
            // start (the download check only covered what was enabled then).
            let spec = provider.owner().spec.clone();
            let source = SourceEntry {
                enabled: true,
                ..request.clone()
            };
            let tag = provider.tag.clone();
            let refusal = on_build_thread(move || {
                providers::load_sd(&spec, std::slice::from_ref(&source), None)
                    .0
                    .iter()
                    .find_map(|entry| providers::rule_refusal(&tag, &entry.rule))
            })
            .await;
            if let Ok(Some(reason)) = refusal {
                return Reply::json_error(400, &reason);
            }
        }
        let guard = provider.mutate.lock().await;
        let mut sources = provider.owner().sources.borrow().clone();
        let (stored, status) = match sources.iter_mut().find(|e| e.name == name) {
            Some(existing) => {
                existing.kind = request.kind;
                existing.files = request.files;
                existing.url = request.url;
                existing.enabled = request.enabled;
                if provider.kind == Kind::Sd {
                    existing.enable_regexp = request.enable_regexp;
                }
                existing.auto_update = request.auto_update;
                existing.update_interval_hours = request.update_interval_hours;
                (existing.clone(), 200)
            }
            None => {
                let created = SourceEntry {
                    name: name.clone(),
                    rule_count: 0,
                    last_updated: None,
                    enable_regexp: provider.kind == Kind::Sd && request.enable_regexp,
                    ..request
                };
                sources.push(created.clone());
                (created, 201)
            }
        };
        if let Err(reply) = save_manifest(provider, sources, "failed to save config").await {
            return reply;
        }
        drop(guard);
        let fetch = !stored.files.trim().is_empty() && !stored.url.trim().is_empty();
        let ctx = self.ctx();
        let job_provider = Rc::clone(provider);
        self.spawn(async move {
            if fetch {
                download_many(&ctx, &job_provider, vec![name]).await;
            }
            changed(&ctx, &job_provider).await;
        });
        Reply::json(status, JSON_UTF8, &source_view(&stored, provider.kind))
    }

    async fn delete_source(&self, provider: &Rc<Provider>, key: &str) -> Reply {
        let adguard = provider.kind == Kind::Adguard;
        let guard = provider.mutate.lock().await;
        let mut sources = provider.owner().sources.borrow().clone();
        let Some(index) = sources.iter().position(|e| provider.key(e) == key) else {
            return if adguard {
                Reply::json_error(404, "Rule not found")
            } else {
                Reply::json_error(404, "source not found")
            };
        };
        let removed = sources.remove(index);
        let message = if adguard {
            "Failed to save config"
        } else {
            "failed to save config"
        };
        if let Err(reply) = save_manifest(provider, sources, message).await {
            return reply;
        }
        // The manifest no longer names the file, so removing it last can
        // never leave a manifest entry without its rules. A file another
        // source still reads is kept (the reference would delete it).
        let file = provider.owner().spec.source_file(&removed);
        if adguard || !removed.files.is_empty() {
            let tag = provider.tag.clone();
            let in_use = self.files_in_use();
            let _ = tokio::task::spawn_blocking(move || {
                if in_use.iter().any(|other| same_file(other, &file)) {
                    return;
                }
                if let Err(error) = std::fs::remove_file(&file)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    crate::diag::warn_line!(
                        "{tag}: failed to delete rule file {}: {error}",
                        file.display()
                    );
                }
            })
            .await;
        }
        drop(guard);
        let ctx = self.ctx();
        let provider = Rc::clone(provider);
        self.spawn(async move { changed(&ctx, &provider).await });
        Reply::empty(204)
    }

    fn update_source(&self, provider: &Rc<Provider>, name: String) -> Reply {
        if provider.find(&name).is_none() {
            return Reply::json_error(404, &format!("source '{name}' not found"));
        }
        let message = format!("update process for '{name}' started in the background");
        let ctx = self.ctx();
        let provider = Rc::clone(provider);
        self.spawn(async move {
            if download_many(&ctx, &provider, vec![name]).await {
                changed(&ctx, &provider).await;
            }
        });
        Reply::json(202, JSON_UTF8, &serde_json::json!({ "message": message }))
    }

    fn adguard_request(body: &[u8]) -> Result<SourceEntry, Reply> {
        let Ok(mut request) = providers::parse_source_object(body, Schema::Adguard) else {
            return Err(Reply::json_error(400, "Invalid request body"));
        };
        request.name = request.name.trim().to_owned();
        request.url = request.url.trim().to_owned();
        if request.name.is_empty() || request.url.is_empty() {
            return Err(Reply::json_error(400, "Name and URL are required"));
        }
        if request.update_interval_hours < 0 {
            return Err(Reply::json_error(
                400,
                "UpdateIntervalHours cannot be negative",
            ));
        }
        Ok(request)
    }

    async fn create_rule(&self, provider: &Rc<Provider>, body: &[u8]) -> Reply {
        let mut rule = match Self::adguard_request(body) {
            Ok(rule) => rule,
            Err(reply) => return reply,
        };
        rule.id = match new_uuid() {
            Ok(id) => id,
            Err(error) => return Reply::json_error(500, &error),
        };
        rule.last_updated = None;
        let guard = provider.mutate.lock().await;
        let mut sources = provider.owner().sources.borrow().clone();
        sources.push(rule.clone());
        if let Err(reply) = save_manifest(provider, sources, "Failed to save config").await {
            return reply;
        }
        drop(guard);
        if rule.enabled {
            let ctx = self.ctx();
            let job_provider = Rc::clone(provider);
            let id = rule.id.clone();
            self.spawn(async move {
                download_many(&ctx, &job_provider, vec![id]).await;
                changed(&ctx, &job_provider).await;
            });
        }
        // The reference encodes the new rule by value, so the zero time is
        // printed here (unlike the list and the manifest).
        Reply::json(201, "application/json", &rule_view(&rule, true))
    }

    async fn put_rule(&self, provider: &Rc<Provider>, id: &str, body: &[u8]) -> Reply {
        let request = match Self::adguard_request(body) {
            Ok(rule) => rule,
            Err(reply) => return reply,
        };
        let guard = provider.mutate.lock().await;
        let mut sources = provider.owner().sources.borrow().clone();
        let Some(rule) = sources.iter_mut().find(|e| e.id == id) else {
            return Reply::json_error(404, "Rule not found");
        };
        rule.name = request.name;
        rule.url = request.url;
        rule.enabled = request.enabled;
        rule.auto_update = request.auto_update;
        rule.update_interval_hours = request.update_interval_hours;
        let stored = rule.clone();
        if let Err(reply) = save_manifest(provider, sources, "Failed to save config").await {
            return reply;
        }
        drop(guard);
        let ctx = self.ctx();
        let job_provider = Rc::clone(provider);
        self.spawn(async move { changed(&ctx, &job_provider).await });
        Reply::json(200, "application/json", &rule_view(&stored, false))
    }

    fn update_all_rules(&self, provider: &Rc<Provider>) -> Reply {
        let ids: Vec<String> = provider
            .owner()
            .sources
            .borrow()
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.id.clone())
            .collect();
        let ctx = self.ctx();
        let provider = Rc::clone(provider);
        self.spawn(async move {
            download_many(&ctx, &provider, ids).await;
            changed(&ctx, &provider).await;
        });
        Reply::text(
            202,
            "Update process for enabled rules has been started in the background.\n".to_owned(),
        )
    }

    fn update_rule(&self, provider: &Rc<Provider>, id: String) -> Reply {
        if provider.find(&id).is_none() {
            return Reply::json_error(404, "Rule not found");
        }
        let ctx = self.ctx();
        let provider = Rc::clone(provider);
        self.spawn(async move {
            if download_many(&ctx, &provider, vec![id]).await {
                changed(&ctx, &provider).await;
            }
        });
        Reply::text(
            202,
            "Update process for rule has been started in the background.\n".to_owned(),
        )
    }
}

// ----- JSON views ------------------------------------------------------------

#[derive(Serialize)]
struct SourceView<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    files: &'a str,
    url: &'a str,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_regexp: Option<bool>,
    auto_update: bool,
    update_interval_hours: i64,
    rule_count: i64,
    last_updated: &'a str,
}

fn source_view(entry: &SourceEntry, kind: Kind) -> SourceView<'_> {
    SourceView {
        name: &entry.name,
        kind: &entry.kind,
        files: &entry.files,
        url: &entry.url,
        enabled: entry.enabled,
        // `omitempty` in `sd_set_light`; `si_set` has no such field.
        enable_regexp: (kind == Kind::Sd && entry.enable_regexp).then_some(true),
        auto_update: entry.auto_update,
        update_interval_hours: entry.update_interval_hours,
        rule_count: entry.rule_count,
        last_updated: entry.last_updated.as_deref().unwrap_or(ZERO_TIME),
    }
}

#[derive(Serialize)]
struct RuleView<'a> {
    id: &'a str,
    name: &'a str,
    url: &'a str,
    enabled: bool,
    auto_update: bool,
    update_interval_hours: i64,
    rule_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_updated: Option<&'a str>,
}

/// The reference `MarshalJSON` omits a zero `last_updated`.
fn rule_view(entry: &SourceEntry, print_zero: bool) -> RuleView<'_> {
    let last_updated = match entry.last_updated.as_deref() {
        Some(text) if unix_seconds(text) != Some(ZERO_UNIX) => Some(text),
        _ if print_zero => Some(ZERO_TIME),
        _ => None,
    };
    RuleView {
        id: &entry.id,
        name: &entry.name,
        url: &entry.url,
        enabled: entry.enabled,
        auto_update: entry.auto_update,
        update_interval_hours: entry.update_interval_hours,
        rule_count: entry.rule_count,
        last_updated,
    }
}

/// The manifest bytes the reference writes: indented, sorted by name
/// (`sd_set_light`/`si_set`) or id (`adguard_rule`).
fn manifest_bytes(kind: Kind, sources: &mut [SourceEntry]) -> Vec<u8> {
    let bytes = if kind == Kind::Adguard {
        sources.sort_by(|a, b| a.id.cmp(&b.id));
        let views: Vec<_> = sources.iter().map(|e| rule_view(e, false)).collect();
        serde_json::to_vec_pretty(&views)
    } else {
        sources.sort_by(|a, b| a.name.cmp(&b.name));
        let views: Vec<_> = sources.iter().map(|e| source_view(e, kind)).collect();
        serde_json::to_vec_pretty(&views)
    };
    bytes.expect("manifest serializes")
}

/// Writes `sources` as the manifest, then adopts them in memory. On failure
/// neither the file nor memory changes. The caller holds the provider mutex.
async fn save_manifest(
    provider: &Provider,
    mut sources: Vec<SourceEntry>,
    failure: &str,
) -> Result<(), Reply> {
    let owner = provider.owner();
    if owner.unreadable {
        crate::diag::error_line!(
            "{}: source manifest {} could not be read at start; refusing to overwrite it",
            provider.tag,
            owner.spec.manifest.display()
        );
        return Err(Reply::json_error(500, failure));
    }
    let bytes = manifest_bytes(provider.kind, &mut sources);
    let path = owner.spec.manifest.clone();
    match tokio::task::spawn_blocking(move || write_atomically(&path, &bytes)).await {
        Ok(Ok(())) => {
            *owner.sources.borrow_mut() = sources;
            Ok(())
        }
        Ok(Err(error)) => {
            crate::diag::error_line!("{}: failed to save config: {error}", provider.tag);
            Err(Reply::json_error(500, failure))
        }
        Err(error) => {
            crate::diag::error_line!("{}: failed to save config: {error}", provider.tag);
            Err(Reply::json_error(500, failure))
        }
    }
}

/// Updates the in-memory counts and saves the manifest when any changed; a
/// failed save is logged and the counts stay in memory (as the reference).
async fn apply_counts(ctx: &Ctx, provider: &Provider, counts: Counts) {
    if counts.is_empty() || provider.owner().unreadable {
        return;
    }
    let _guard = provider.mutate.lock().await;
    // A fenced (retired) generation never writes after its successor exists.
    if ctx.cancel.is_cancelled() {
        return;
    }
    let mut sources = provider.owner().sources.borrow().clone();
    let mut changed = false;
    for (key, count) in counts {
        if let Some(entry) = sources.iter_mut().find(|e| provider.key(e) == key)
            && entry.rule_count != count
        {
            entry.rule_count = count;
            changed = true;
        }
    }
    if changed && save_manifest(provider, sources.clone(), "").await.is_err() {
        *provider.owner().sources.borrow_mut() = sources;
    }
}

// ----- downloads -------------------------------------------------------------

/// Downloads every source in `keys` concurrently; true when at least one
/// download replaced its file.
async fn download_many(ctx: &Ctx, provider: &Rc<Provider>, keys: Vec<String>) -> bool {
    let mut tasks = tokio::task::JoinSet::new();
    for key in keys {
        let ctx = ctx.clone();
        let provider = Rc::clone(provider);
        tasks.spawn_local(async move { download_source(&ctx, &provider, key).await });
    }
    let mut any = false;
    while let Some(result) = tasks.join_next().await {
        any |= result.unwrap_or(false);
    }
    any
}

/// Removes a source's single-flight entry however the download ends.
struct Flight<'a> {
    provider: &'a Provider,
    key: String,
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        self.provider.owner().flights.borrow_mut().remove(&self.key);
    }
}

/// One source's download with single flight: a request for a source that is
/// already downloading makes that download run once more and returns false.
async fn download_source(ctx: &Ctx, provider: &Provider, key: String) -> bool {
    {
        let mut flights = provider.owner().flights.borrow_mut();
        if let Some(again) = flights.get_mut(&key) {
            *again = true;
            return false;
        }
        flights.insert(key.clone(), false);
    }
    let _flight = Flight {
        provider,
        key: key.clone(),
    };
    let mut replaced = false;
    loop {
        if ctx.cancel.is_cancelled() {
            return replaced;
        }
        match attempt(ctx, provider, &key).await {
            Ok(()) => replaced = true,
            Err(error) => {
                crate::diag::error_line!("{}: failed to update '{key}': {error}", provider.tag);
            }
        }
        let again = provider
            .owner()
            .flights
            .borrow_mut()
            .get_mut(&key)
            .map(std::mem::take)
            .unwrap_or(false);
        if !again {
            return replaced;
        }
    }
}

async fn attempt(ctx: &Ctx, provider: &Provider, key: &str) -> Result<(), String> {
    let owner = provider.owner();
    let entry = provider
        .find(key)
        .ok_or_else(|| format!("source '{key}' not found"))?;
    if entry.url.is_empty() {
        return Err("no URL configured".to_owned());
    }
    if provider.kind != Kind::Adguard && entry.files.is_empty() {
        return Err("no local file path configured".to_owned());
    }
    let file = owner.spec.source_file(&entry);
    let timeout = if provider.kind == Kind::Adguard {
        ctx.env.timing.adguard_timeout
    } else {
        ctx.env.timing.source_timeout
    };
    let bytes = crate::rule_download::fetch(
        crate::rule_download::Fetch {
            url: &entry.url,
            socks5: &owner.spec.socks5,
            timeout,
            max_bytes: ctx.env.timing.max_download_bytes,
            roots: ctx.env.roots.clone(),
        },
        &ctx.cancel,
    )
    .await?;
    let _guard = provider.mutate.lock().await;
    if ctx.cancel.is_cancelled() {
        return Err("canceled by shutdown".to_owned());
    }
    // The source may have been removed or repointed while downloading.
    let current = provider
        .find(key)
        .ok_or_else(|| "source was removed during the download".to_owned())?;
    if current.url != entry.url || owner.spec.source_file(&current) != file {
        return Err("source changed during the download; result discarded".to_owned());
    }
    let (kind, tag, regexp) = (provider.kind, provider.tag.clone(), current.enable_regexp);
    crate::diag::debug_line!("{tag}: '{key}' downloaded {} bytes", bytes.len());
    let count = on_build_thread(move || store_download(kind, &tag, &bytes, &file, regexp))
        .await
        .map_err(|error| error.to_string())??;
    let mut sources = owner.sources.borrow().clone();
    if let Some(entry) = sources.iter_mut().find(|e| provider.key(e) == key) {
        if let Some(count) = count {
            entry.rule_count = count;
        }
        entry.last_updated = Some(now_text());
    }
    if save_manifest(provider, sources.clone(), "").await.is_err() {
        // As the reference: the new metadata stays in memory.
        *owner.sources.borrow_mut() = sources;
    }
    crate::diag::info_line!("{}: updated '{key}'", provider.tag);
    Ok(())
}

/// Validates downloaded content and replaces the rule file. Returns the
/// reference rule count for SRS sources.
fn store_download(
    kind: Kind,
    tag: &str,
    bytes: &[u8],
    file: &Path,
    enable_regexp: bool,
) -> Result<Option<i64>, String> {
    let count = match kind {
        Kind::Sd => {
            let srs = decode_domain_srs(bytes, enable_regexp)
                .map_err(|error| format!("invalid SRS data: {error}"))?;
            if let Some(reason) = srs
                .rules
                .iter()
                .find_map(|rule| providers::rule_refusal(tag, rule))
            {
                return Err(format!("content refused: {reason}"));
            }
            Some(srs.rules.len() as i64)
        }
        Kind::Si => {
            let srs =
                decode_ip_srs(bytes).map_err(|error| format!("not a valid SRS file: {error}"))?;
            Some(srs.prefixes.len() as i64)
        }
        Kind::Adguard => {
            let (mut allow, mut deny) = (Vec::new(), Vec::<RuleEntry>::new());
            providers::parse_adguard_list(tag, "download", bytes, &mut allow, &mut deny);
            if let Some(reason) = allow
                .iter()
                .map(String::as_str)
                .chain(deny.iter().map(|e| e.rule.as_str()))
                .find_map(|rule| providers::rule_refusal(tag, rule))
            {
                return Err(format!("content refused: {reason}"));
            }
            None
        }
        Kind::Light => return Err("not a downloadable provider".to_owned()),
    };
    write_atomically(file, bytes).map_err(|error| format!("write {}: {error}", file.display()))?;
    Ok(count)
}

// ----- rebuilds --------------------------------------------------------------

/// Resets a build state when a job ends early (shutdown), so it never stays
/// `Running` without a runner.
struct BuildGuard<'a>(&'a Cell<Build>);

impl Drop for BuildGuard<'_> {
    fn drop(&mut self) {
        self.0.set(Build::Idle);
    }
}

/// Runs `work` unless a build of this target is already running, in which
/// case that build is asked to run once more. Never two at a time.
async fn coalesced<F, Fut>(state: &Cell<Build>, mut work: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if state.get() != Build::Idle {
        state.set(Build::Dirty);
        return;
    }
    let _guard = BuildGuard(state);
    loop {
        state.set(Build::Running);
        work().await;
        if state.get() != Build::Dirty {
            return;
        }
    }
}

/// After a provider changed: its own state (counts, `si_set` list, AdGuard
/// direct matcher), then every mapper that names it.
async fn changed(ctx: &Ctx, provider: &Rc<Provider>) {
    if ctx.cancel.is_cancelled() {
        return;
    }
    coalesced(&provider.reload, || reload_provider(ctx, provider)).await;
    let dependents: Vec<Rc<MapperSlot>> = ctx
        .inner
        .mappers
        .iter()
        .filter(|slot| slot.spec.rules.iter().any(|rule| rule.tag == provider.tag))
        .cloned()
        .collect();
    for slot in dependents {
        if ctx.cancel.is_cancelled() {
            return;
        }
        coalesced(&slot.build, || rebuild_mapper(ctx, &slot)).await;
    }
    // The replaced generations are dropped by now.
    let _ = on_build_thread(crate::memory::release_free_memory).await;
}

/// Frees a replaced generation off the DNS thread, where dropping a large
/// matcher would stall queries for milliseconds. Not on the build thread
/// either: there it could wait behind another queued build and keep both
/// generations alive meanwhile. A generation a query still holds (none is
/// held across an await) is dropped by its last holder instead.
async fn drop_off_thread<T: Send + 'static>(replaced: Rc<T>) {
    if let Ok(replaced) = Rc::try_unwrap(replaced) {
        let _ = tokio::task::spawn_blocking(move || drop(replaced)).await;
    }
}

fn gate(ctx: &Ctx) -> Option<PersistGate> {
    ctx.inner.build_gate.borrow().clone()
}

enum Reloaded {
    Counts(Counts),
    Si(IpPrefixList, Counts),
    Adguard(Result<Box<Filtered>, String>, Counts),
}

async fn reload_provider(ctx: &Ctx, provider: &Rc<Provider>) {
    let Some(owner) = provider.manifest.as_ref() else {
        return;
    };
    let spec = owner.spec.clone();
    let sources = owner.sources.borrow().clone();
    let kind = provider.kind;
    let direct = provider.filtered.borrow().is_some();
    let gate = gate(ctx);
    let result = on_build_thread(move || {
        if let Some(gate) = gate {
            gate.wait();
        }
        match kind {
            Kind::Sd => Reloaded::Counts(count_sd(&spec, &sources)),
            Kind::Si => {
                let (list, counts) = providers::load_si(&spec, &sources, None);
                Reloaded::Si(list, counts)
            }
            Kind::Adguard => {
                let (rules, counts) = providers::load_adguard(&spec, &sources, true, None);
                let filtered = if direct {
                    rules.filtered(&spec.tag).map(Box::new)
                } else {
                    Err(String::new())
                };
                Reloaded::Adguard(filtered, counts)
            }
            Kind::Light => Reloaded::Counts(Vec::new()),
        }
    })
    .await;
    let counts = match result {
        Ok(Reloaded::Counts(counts)) => counts,
        Ok(Reloaded::Si(list, counts)) => {
            if let Some(handle) = &provider.ip {
                handle.publish(Rc::new(list));
            }
            counts
        }
        Ok(Reloaded::Adguard(filtered, counts)) => {
            let live = provider.filtered.borrow().clone();
            if let Some(live) = live {
                match filtered {
                    Ok(filtered) => drop_off_thread(live.replace(Rc::new(*filtered))).await,
                    Err(error) => crate::diag::error_line!(
                        "{}: rebuild failed, keeping the previous rules: {error}",
                        provider.tag
                    ),
                }
            }
            counts
        }
        Err(error) => {
            crate::diag::error_line!("{}: reload failed: {error}", provider.tag);
            return;
        }
    };
    ctx.inner.builds.fetch_add(1, Ordering::SeqCst);
    apply_counts(ctx, provider, counts).await;
}

/// The reference reload count of `sd_set_light`: each decodable enabled
/// source, one file at a time (only the count is kept).
fn count_sd(spec: &ManifestSpec, sources: &[SourceEntry]) -> Counts {
    let mut counts = Vec::new();
    for source in sources {
        if !source.enabled || source.files.is_empty() {
            continue;
        }
        let (_, mut one) = providers::load_sd(spec, std::slice::from_ref(source), None);
        counts.append(&mut one);
    }
    counts
}

async fn rebuild_mapper(ctx: &Ctx, slot: &Rc<MapperSlot>) {
    let mut plans: Vec<(String, Plan)> = Vec::new();
    for rule in &slot.spec.rules {
        if plans.iter().any(|(tag, _)| *tag == rule.tag) {
            continue;
        }
        if let Some(provider) = ctx.inner.providers.iter().find(|p| p.tag == rule.tag) {
            plans.push((rule.tag.clone(), provider.load_plan()));
        }
    }
    let spec = slot.spec.clone();
    let gate = gate(ctx);
    let built = on_build_thread(move || {
        if let Some(gate) = gate {
            gate.wait();
        }
        let mut rules: Vec<(String, providers::ProviderRules)> = Vec::with_capacity(plans.len());
        for (tag, plan) in plans {
            rules.push((tag, plan.rules()?));
        }
        providers::build_mapper_matcher(&spec, &|tag| {
            rules
                .iter()
                .find(|(name, _)| name == tag)
                .map(|(_, entries)| entries.as_slice())
        })
        .map_err(|error| error.to_string())
    })
    .await;
    match built {
        Ok(Ok(matcher)) => {
            let replaced = slot.live.replace(Rc::new(matcher));
            ctx.inner.builds.fetch_add(1, Ordering::SeqCst);
            drop_off_thread(replaced).await;
        }
        Ok(Err(error)) => crate::diag::error_line!(
            "{}: rebuild failed, keeping the previous rules: {error}",
            slot.spec.tag
        ),
        Err(error) => crate::diag::error_line!("{}: rebuild failed: {error}", slot.spec.tag),
    }
}

// ----- scheduler -------------------------------------------------------------

/// The host-lifetime auto-update loop: start-up work once, then the
/// reference cadence. It always acts on the currently committed view.
pub(crate) async fn supervise(
    control: crate::runtime_snapshot::RuntimeControl,
    shutdown: TransportCancellation,
) {
    // What compiling the configuration freed (rule text, build temporaries).
    let _ = on_build_thread(crate::memory::release_free_memory).await;
    let sources = Rc::clone(&control.view().config.rule_sources);
    sources.start();
    let timing = sources.timing();
    drop(sources);
    let start = tokio::time::Instant::now();
    let mut next_source = start + timing.first_check;
    let mut next_adguard = start + timing.first_check;
    loop {
        let next = next_source.min(next_adguard);
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep_until(next) => {}
        }
        let now = tokio::time::Instant::now();
        let mut kinds = Vec::new();
        if now >= next_source {
            kinds.extend([Kind::Sd, Kind::Si]);
            next_source = now + timing.source_period;
        }
        if now >= next_adguard {
            kinds.push(Kind::Adguard);
            next_adguard = now + timing.adguard_period;
        }
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        control.view().config.rule_sources.auto_update(&kinds, unix);
    }
}

/// The reference predicate: enabled, auto-update on, a positive interval and
/// at least that many hours since `last_updated` (the zero time is due).
pub(crate) fn is_due(entry: &SourceEntry, now: i64) -> bool {
    if !entry.enabled || !entry.auto_update || entry.update_interval_hours <= 0 {
        return false;
    }
    let last = entry
        .last_updated
        .as_deref()
        .and_then(unix_seconds)
        .unwrap_or(ZERO_UNIX);
    i128::from(now) - i128::from(last) >= i128::from(entry.update_interval_hours) * 3600
}

// ----- helpers ---------------------------------------------------------------

/// The process-wide write lock of one manifest or rule file. Every
/// generation's provider for the same file gets the same lock.
fn write_lock(path: PathBuf) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(locks.entry(path).or_default())
}

type BuildJob = Box<dyn FnOnce() + Send>;

/// Runs allocation-heavy rule work (decoding, building matchers) on one
/// long-lived thread instead of the blocking pool. Every rebuild then reuses
/// the same allocator arena, which bounds the memory a series of rebuilds
/// keeps resident; a pool thread per rebuild would leave each arena at its
/// own peak. The DNS runtime thread never runs this work.
pub(crate) async fn on_build_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    static WORKER: std::sync::OnceLock<Result<std::sync::mpsc::Sender<BuildJob>, String>> =
        std::sync::OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<BuildJob>();
        std::thread::Builder::new()
            .name("mosdns-rule-build".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job();
                }
            })
            .map(|_| sender)
            .map_err(|error| error.to_string())
    });
    let sender = worker.as_ref().map_err(Clone::clone)?;
    let (reply, result) = tokio::sync::oneshot::channel();
    sender
        .send(Box::new(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .map_err(|_| "rule build panicked".to_owned());
            let _ = reply.send(outcome);
        }))
        .map_err(|_| "rule build thread stopped".to_owned())?;
    result
        .await
        .map_err(|_| "rule build thread stopped".to_owned())?
}

/// Whether two paths name the same file: the same inode when both exist
/// (any spelling, symlink or hard link), otherwise the same normalized path.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    if let (Ok(left), Ok(right)) = (std::fs::metadata(a), std::fs::metadata(b)) {
        use std::os::unix::fs::MetadataExt;
        return left.dev() == right.dev() && left.ino() == right.ino();
    }
    normalized(a) == normalized(b)
}

/// `.` and `..` removed lexically.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn rule_lines<'a>(rules: impl Iterator<Item = &'a str>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for rule in rules {
        bytes.extend_from_slice(rule.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Replaces `path` through a same-directory temporary file and a rename,
/// creating the directory when needed. A failure removes the temporary file
/// and leaves `path` untouched.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = directory.join(temporary);
    let outcome =
        std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

fn decode_query(value: &str) -> String {
    let value = value.replace('+', " ");
    crate::api::decode_plugin_tag(&value).map_or(value.clone(), std::borrow::Cow::into_owned)
}

fn now_text() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| ZERO_TIME.to_owned())
}

/// A random (version 4) UUID, as the reference `uuid.New()`.
fn new_uuid() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Seconds since the Unix epoch of a manifest time (the RFC 3339 forms the
/// manifest decoder accepts), or `None` when it is not one.
pub(crate) fn unix_seconds(text: &str) -> Option<i64> {
    let mut padded = text.as_bytes().to_vec();
    if padded.len() > 12 && padded[11].is_ascii_digit() && padded[12] == b':' {
        padded.insert(11, b'0');
    }
    let b = padded.as_slice();
    let digits = |from: usize, n: usize| -> Option<i64> {
        let part = b.get(from..from + n)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0, |acc, d| acc * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    let mut i = 19;
    if matches!(b.get(i), Some(b'.' | b',')) {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    let offset = match b.get(i)? {
        b'Z' => 0,
        sign @ (b'+' | b'-') => {
            let value = digits(i + 1, 2)? * 3600 + digits(i + 4, 2)? * 60;
            if *sign == b'-' { -value } else { value }
        }
        _ => return None,
    };
    // Days from the civil date (proleptic Gregorian), Howard Hinnant's method.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(enabled: bool, auto: bool, hours: i64, last: Option<&str>) -> SourceEntry {
        SourceEntry {
            name: "s".into(),
            enabled,
            auto_update: auto,
            update_interval_hours: hours,
            last_updated: last.map(str::to_owned),
            ..SourceEntry::default()
        }
    }

    #[test]
    fn times_parse_like_the_manifest_decoder() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds(ZERO_TIME), Some(ZERO_UNIX));
        assert_eq!(unix_seconds("0001-01-01T08:00:00+08:00"), Some(ZERO_UNIX));
        assert_eq!(
            unix_seconds("2025-12-19T16:32:42.281233973+08:00"),
            Some(1_766_133_162)
        );
        assert_eq!(unix_seconds("2025-12-19T8:32:42Z"), Some(1_766_133_162));
        assert_eq!(unix_seconds("2025-12-19T08:32:42,5Z"), Some(1_766_133_162));
        assert_eq!(unix_seconds("not a time"), None);
    }

    #[test]
    fn the_due_predicate_follows_the_reference() {
        let now = unix_seconds("2026-10-08T12:00:00Z").unwrap();
        // Never updated (zero time or no field) is due.
        assert!(is_due(&entry(true, true, 24, None), now));
        assert!(is_due(&entry(true, true, 24, Some(ZERO_TIME)), now));
        // Exactly the interval is due; one second less is not.
        assert!(is_due(
            &entry(true, true, 24, Some("2026-10-07T12:00:00Z")),
            now
        ));
        assert!(!is_due(
            &entry(true, true, 24, Some("2026-10-07T12:00:01Z")),
            now
        ));
        // Offsets are honored.
        assert!(is_due(
            &entry(true, true, 1, Some("2026-10-08T19:00:00+08:00")),
            now
        ));
        assert!(!is_due(
            &entry(true, true, 1, Some("2026-10-08T19:30:00+08:00")),
            now
        ));
        // Disabled, auto-update off, or a non-positive interval is never due.
        assert!(!is_due(&entry(false, true, 24, None), now));
        assert!(!is_due(&entry(true, false, 24, None), now));
        assert!(!is_due(&entry(true, true, 0, None), now));
        assert!(!is_due(&entry(true, true, -1, None), now));
    }

    #[test]
    fn manifests_are_written_like_the_reference() {
        let mut sources = vec![
            SourceEntry {
                name: "b".into(),
                kind: "geositecn".into(),
                files: "srs/b.srs".into(),
                url: "https://x/b.srs".into(),
                enabled: true,
                update_interval_hours: 24,
                rule_count: 3,
                last_updated: Some("2025-12-19T16:32:42.281233973+08:00".into()),
                ..SourceEntry::default()
            },
            SourceEntry {
                name: "a".into(),
                enable_regexp: true,
                ..SourceEntry::default()
            },
        ];
        let sd = String::from_utf8(manifest_bytes(Kind::Sd, &mut sources.clone())).unwrap();
        assert_eq!(
            sd,
            "[\n  {\n    \"name\": \"a\",\n    \"type\": \"\",\n    \"files\": \"\",\n    \"url\": \"\",\n    \"enabled\": false,\n    \"enable_regexp\": true,\n    \"auto_update\": false,\n    \"update_interval_hours\": 0,\n    \"rule_count\": 0,\n    \"last_updated\": \"0001-01-01T00:00:00Z\"\n  },\n  {\n    \"name\": \"b\",\n    \"type\": \"geositecn\",\n    \"files\": \"srs/b.srs\",\n    \"url\": \"https://x/b.srs\",\n    \"enabled\": true,\n    \"auto_update\": false,\n    \"update_interval_hours\": 24,\n    \"rule_count\": 3,\n    \"last_updated\": \"2025-12-19T16:32:42.281233973+08:00\"\n  }\n]"
        );
        let si = String::from_utf8(manifest_bytes(Kind::Si, &mut sources)).unwrap();
        assert!(!si.contains("enable_regexp"));
        let mut rules = vec![
            SourceEntry {
                id: "2".into(),
                name: "z".into(),
                last_updated: Some(ZERO_TIME.into()),
                ..SourceEntry::default()
            },
            SourceEntry {
                id: "1".into(),
                name: "y".into(),
                last_updated: Some("2026-03-24T13:10:52.253656114+08:00".into()),
                ..SourceEntry::default()
            },
        ];
        let adguard = String::from_utf8(manifest_bytes(Kind::Adguard, &mut rules)).unwrap();
        assert!(adguard.find("\"id\": \"1\"").unwrap() < adguard.find("\"id\": \"2\"").unwrap());
        assert_eq!(adguard.matches("last_updated").count(), 1, "{adguard}");
    }

    #[test]
    fn one_file_has_one_write_lock_across_generations() {
        let a = write_lock(PathBuf::from("/x/srs/a.json"));
        let b = write_lock(PathBuf::from("/x/srs/a.json"));
        let c = write_lock(PathBuf::from("/x/srs/c.json"));
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[test]
    fn same_file_sees_through_spellings_and_links() {
        let root = std::env::temp_dir().join(format!("rule-sources-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("srs")).unwrap();
        let file = root.join("srs/cn.srs");
        std::fs::write(&file, b"x").unwrap();
        assert!(same_file(&file, &root.join("srs/../srs/./cn.srs")));
        std::fs::hard_link(&file, root.join("srs/link.srs")).unwrap();
        assert!(same_file(&file, &root.join("srs/link.srs")));
        std::fs::write(root.join("srs/other.srs"), b"x").unwrap();
        assert!(!same_file(&file, &root.join("srs/other.srs")));
        // Missing files compare by normalized spelling.
        assert!(same_file(
            &root.join("a/../gone.srs"),
            &root.join("gone.srs")
        ));
        assert!(!same_file(&root.join("gone.srs"), &root.join("gone2.srs")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uuids_are_version_4() {
        let id = new_uuid().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(id, new_uuid().unwrap());
    }

    #[test]
    fn atomic_writes_replace_or_leave_the_target() {
        let root = std::env::temp_dir().join(format!("rule-sources-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join("deep/dir/file.json");
        write_atomically(&target, b"one").unwrap();
        write_atomically(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        // A directory in the way of the rename fails without touching it.
        let blocked = root.join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(write_atomically(&blocked, b"x").is_err());
        assert!(blocked.join("child").is_dir());
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
