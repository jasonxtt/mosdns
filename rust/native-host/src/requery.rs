//! The `requery` plugin: a scheduled (or triggered) job that saves the cache and
//! name-list state, collects the recently seen names from the statistics files,
//! flushes, asks the host's own resolver for every name again at a fixed rate
//! (rebuilding caches and lists), and saves again.
//!
//! The job's settings and progress live in one JSON file the Vue UI also edits
//! through the plugin's API (`file`, usually `webinfo/requeryconfig.json`); its
//! format is the reference one. See `docs/rust/contracts/native-requery.md`.

use std::cell::{Cell, RefCell};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mosdns_upstream_core::TransportCancellation;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;

use crate::local_url::{self, LocalApi, Target};
use crate::rule_sources::Reply;

const JSON_UTF8: &str = "application/json; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";
/// Deadline of one URL action (the reference client's timeout).
const URL_TIMEOUT: Duration = Duration::from_secs(30);
/// Deadline of one re-query.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// Re-queries in flight at once (the reference does not bound them).
const MAX_IN_FLIGHT: usize = 1024;
/// The longest single sleep of the scheduler.
const MAX_SLEEP: Duration = Duration::from_secs(24 * 3600);

// ----- the JSON document ---------------------------------------------------------

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A Go `time.Time`: RFC 3339 in UTC with the shortest sub-second part; the
/// zero time prints as `0001-01-01T00:00:00Z`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GoTime(Option<OffsetDateTime>);

const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

impl Serialize for GoTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            None => serializer.serialize_str(ZERO_TIME),
            Some(moment) => serializer.serialize_str(
                &moment
                    .to_offset(time::UtcOffset::UTC)
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(serde::ser::Error::custom)?,
            ),
        }
    }
}

impl<'de> Deserialize<'de> for GoTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Some(text) = Option::<String>::deserialize(deserializer)? else {
            return Ok(Self(None));
        };
        if text == ZERO_TIME {
            return Ok(Self(None));
        }
        OffsetDateTime::parse(&text, &time::format_description::well_known::Rfc3339)
            .map(|moment| Self(Some(moment)))
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SourceFile {
    #[serde(default, deserialize_with = "null_default")]
    alias: String,
    #[serde(default, deserialize_with = "null_default")]
    path: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct DomainProcessing {
    /// `None` is a JSON `null`, which is what the reference writes for a list
    /// that was never set.
    #[serde(default)]
    source_files: Option<Vec<SourceFile>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct UrlActions {
    #[serde(default)]
    save_rules: Option<Vec<String>>,
    #[serde(default)]
    flush_rules: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct SchedulerConfig {
    #[serde(default, deserialize_with = "null_default")]
    enabled: bool,
    #[serde(default, deserialize_with = "null_default")]
    start_datetime: String,
    #[serde(default, deserialize_with = "null_default")]
    interval_minutes: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ExecutionSettings {
    #[serde(default, deserialize_with = "null_default")]
    queries_per_second: i64,
    #[serde(default, deserialize_with = "null_default")]
    resolver_address: String,
    #[serde(default, deserialize_with = "null_default")]
    url_call_delay_ms: i64,
    #[serde(default, deserialize_with = "null_default")]
    date_range_days: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
struct Progress {
    #[serde(default, deserialize_with = "null_default")]
    processed: i64,
    #[serde(default, deserialize_with = "null_default")]
    total: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Status {
    #[serde(default, deserialize_with = "null_default")]
    task_state: String,
    #[serde(default, deserialize_with = "null_default")]
    last_run_start_time: GoTime,
    #[serde(default, deserialize_with = "null_default")]
    last_run_end_time: GoTime,
    #[serde(default, deserialize_with = "null_default")]
    last_run_domain_count: i64,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    last_error: String,
    #[serde(default, deserialize_with = "null_default")]
    progress: Progress,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Config {
    #[serde(default, deserialize_with = "null_default")]
    domain_processing: DomainProcessing,
    #[serde(default, deserialize_with = "null_default")]
    url_actions: UrlActions,
    #[serde(default, deserialize_with = "null_default")]
    scheduler: SchedulerConfig,
    #[serde(default, deserialize_with = "null_default")]
    execution_settings: ExecutionSettings,
    #[serde(default, deserialize_with = "null_default")]
    status: Status,
}

/// Go's `json.MarshalIndent` output: two-space indent, HTML characters
/// escaped, no trailing newline.
fn marshal_indent<T: Serialize>(value: &T) -> Vec<u8> {
    crate::webinfo::escape_html(&serde_json::to_string_pretty(value).unwrap_or_default())
        .into_bytes()
}

/// Go's `json.Encoder` output: compact, HTML characters escaped, one newline.
fn encode_line<T: Serialize>(value: &T) -> Vec<u8> {
    let mut text = crate::webinfo::escape_html(&serde_json::to_string(value).unwrap_or_default());
    text.push('\n');
    text.into_bytes()
}

fn json_reply<T: Serialize>(status: u16, value: &T) -> Reply {
    Reply {
        status,
        content_type: Some(JSON_UTF8),
        body: encode_line(value),
        headers: Vec::new(),
    }
}

fn error_reply(status: u16, message: &str) -> Reply {
    json_reply(
        status,
        &serde_json::json!({ "status": "error", "message": message }),
    )
}

fn success_reply(message: &str) -> Reply {
    json_reply(
        200,
        &serde_json::json!({ "status": "success", "message": message }),
    )
}

fn plain_reply(status: u16, content_type: Option<&'static str>, body: &str) -> Reply {
    Reply {
        status,
        content_type,
        body: body.as_bytes().to_vec(),
        headers: Vec::new(),
    }
}

/// Reads the state file as the reference does at start: a missing file is
/// `None`, an unreadable or invalid one an error.
fn read_config(path: &Path) -> Result<Option<Config>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            format!(
                "failed to parse json from config file {}: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

// ----- scheduling ----------------------------------------------------------------

/// The next run of a scheduler configuration: the start time itself while it
/// is ahead, else the next multiple of the interval after it. `None` when the
/// scheduler is off, has no (valid) start time, or has no positive interval.
fn next_run(scheduler: &SchedulerConfig, now: OffsetDateTime) -> Option<OffsetDateTime> {
    if !scheduler.enabled || scheduler.interval_minutes <= 0 || scheduler.start_datetime.is_empty()
    {
        return None;
    }
    let start = OffsetDateTime::parse(
        &scheduler.start_datetime,
        &time::format_description::well_known::Rfc3339,
    )
    .ok()?;
    if start > now {
        return Some(start);
    }
    let interval = i128::from(scheduler.interval_minutes).checked_mul(60_000_000_000)?;
    let elapsed = (now - start).whole_nanoseconds();
    let cycles = elapsed / interval;
    let offset = (cycles + 1).checked_mul(interval)?;
    let offset = time::Duration::new(
        i64::try_from(offset / 1_000_000_000).ok()?,
        i32::try_from(offset % 1_000_000_000).ok()?,
    );
    start.checked_add(offset)
}

// ----- the plugin ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Running,
    /// A newer view owns the file: nothing runs or writes any more.
    Fenced,
    Closed,
}

enum End {
    Done,
    Failed(String),
    Cancelled,
}

struct Env {
    api: LocalApi,
}

/// The runtime owner of one `requery` plugin.
pub struct Requery {
    pub tag: String,
    file: PathBuf,
    /// The configuration root; relative source paths are read from here (the
    /// reference runs in it).
    root: PathBuf,
    config: RefCell<Config>,
    seq: Cell<u64>,
    last_written: Arc<Mutex<u64>>,
    phase: Cell<Phase>,
    started: Cell<bool>,
    /// Cancels everything this plugin runs (host stop, newer view).
    cancel: TransportCancellation,
    /// The running task's number and own token (a child of `cancel`).
    task: RefCell<Option<(u64, TransportCancellation)>>,
    /// Numbers the tasks, so a finished one never clears its successor.
    task_seq: Cell<u64>,
    jobs: RefCell<tokio::task::JoinSet<()>>,
    wake: tokio::sync::Notify,
    env: RefCell<Option<Env>>,
    fenced: Cell<bool>,
}

/// The compiled plugin: the executable-less handle the API looks up by tag.
pub struct RequeryConfig {
    pub tag: String,
    pub runtime: Rc<Requery>,
}

impl Requery {
    /// Validates `file` the way the reference does at load (a present file must
    /// parse); nothing is written until the host starts serving.
    pub(crate) fn load(tag: &str, file: PathBuf, root: PathBuf) -> Result<Self, String> {
        let existing = read_config(&file)?;
        Ok(Self::new(tag, file, root, existing.unwrap_or_default()))
    }

    fn new(tag: &str, file: PathBuf, root: PathBuf, config: Config) -> Self {
        Self {
            tag: tag.to_owned(),
            file,
            root,
            config: RefCell::new(config),
            seq: Cell::new(0),
            last_written: Arc::new(Mutex::new(0)),
            phase: Cell::new(Phase::Running),
            started: Cell::new(false),
            cancel: TransportCancellation::new(),
            task: RefCell::new(None),
            task_seq: Cell::new(0),
            jobs: RefCell::new(tokio::task::JoinSet::new()),
            wake: tokio::sync::Notify::new(),
            env: RefCell::new(None),
            fenced: Cell::new(false),
        }
    }

    /// A source path as the reference reads it: relative to the working
    /// directory, which is the configuration root.
    fn source_path(&self, path: &str) -> String {
        crate::matchers::resolve_rule_path(path, &self.root)
            .display()
            .to_string()
    }

    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    // ----- lifecycle -----------------------------------------------------------

    /// Loads the state file (creating it, applying defaults, recovering an
    /// interrupted run) and starts the scheduler. Called when the host (or a
    /// newly installed view) starts serving.
    pub(crate) fn start(self: &Rc<Self>, api: &LocalApi) {
        if self.started.replace(true) || self.phase.get() != Phase::Running {
            return;
        }
        if let Err(error) = self.load_for_service() {
            crate::diag::error_line!("{}: {error}", self.tag);
        }
        *self.env.borrow_mut() = Some(Env { api: api.clone() });
        let this = Rc::clone(self);
        self.jobs.borrow_mut().spawn_local(async move {
            this.scheduler().await;
        });
    }

    fn load_for_service(&self) -> Result<(), String> {
        let mut changed = false;
        let mut config = match read_config(&self.file)? {
            Some(config) => config,
            None => {
                // The reference writes an empty configuration when none exists.
                let mut config = Config::default();
                config.status.task_state = "idle".to_owned();
                config.execution_settings.date_range_days = 30;
                *self.config.borrow_mut() = config;
                return self.persist_blocking();
            }
        };
        if config.status.task_state.is_empty() {
            config.status.task_state = "idle".to_owned();
            changed = true;
        }
        if config.execution_settings.url_call_delay_ms == 0 {
            config.execution_settings.url_call_delay_ms = 50;
            changed = true;
        }
        if config.execution_settings.queries_per_second == 0 {
            config.execution_settings.queries_per_second = 100;
            changed = true;
        }
        if config.execution_settings.date_range_days <= 0 {
            config.execution_settings.date_range_days = 30;
            changed = true;
        }
        // A task that was running when the host stopped did not finish.
        if config.status.task_state == "running" {
            crate::diag::warn_line!(
                "{}: a task was running when the host stopped; marking it failed",
                self.tag
            );
            config.status.task_state = "failed".to_owned();
            config.status.last_run_end_time = GoTime(Some(self.now()));
            changed = true;
        }
        *self.config.borrow_mut() = config;
        if changed {
            self.persist_blocking()?;
        }
        Ok(())
    }

    /// A newer view takes over: stop the task and the scheduler, record the
    /// interruption synchronously and never write again, so the new view
    /// loads a settled file.
    pub(crate) fn fence(&self) {
        if self.phase.get() != Phase::Running {
            return;
        }
        self.phase.set(Phase::Fenced);
        self.fenced.set(true);
        self.cancel.cancel();
        if !self.started.get() {
            return;
        }
        let interrupted = self.task.borrow_mut().take().is_some();
        {
            let mut config = self.config.borrow_mut();
            if interrupted || config.status.task_state == "running" {
                config.status.task_state = "cancelled".to_owned();
                config.status.last_error = "task cancelled by host shutdown".to_owned();
                config.status.last_run_end_time = GoTime(Some(self.now()));
            }
        }
        if let Err(error) = self.persist_blocking() {
            crate::diag::error_line!("{}: final save failed: {error}", self.tag);
        }
    }

    /// Stops the scheduler and a running task (joined; the task records its
    /// interruption).
    pub(crate) async fn close(&self) {
        self.phase.set(Phase::Closed);
        self.cancel.cancel();
        let mut jobs = std::mem::take(&mut *self.jobs.borrow_mut());
        while jobs.join_next().await.is_some() {}
    }

    // ----- persistence ---------------------------------------------------------

    fn encoded(&self) -> (u64, Vec<u8>) {
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        (seq, marshal_indent(&*self.config.borrow()))
    }

    fn write_snapshot(
        file: &Path,
        last_written: &Mutex<u64>,
        seq: u64,
        bytes: &[u8],
    ) -> Result<(), String> {
        let mut guard = last_written
            .lock()
            .map_err(|_| "write lock poisoned".to_owned())?;
        if seq <= *guard {
            return Ok(());
        }
        crate::rule_sources::write_atomically(file, bytes)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        *guard = seq;
        Ok(())
    }

    fn persist_blocking(&self) -> Result<(), String> {
        let (seq, bytes) = self.encoded();
        Self::write_snapshot(&self.file, &self.last_written, seq, &bytes)
    }

    /// Writes the current state on the blocking pool; a failure keeps the
    /// previous file.
    async fn persist(&self) -> Result<(), String> {
        // A newer view owns the file now.
        if self.fenced.get() {
            return Ok(());
        }
        let (seq, bytes) = self.encoded();
        let file = self.file.clone();
        let last = Arc::clone(&self.last_written);
        tokio::task::spawn_blocking(move || Self::write_snapshot(&file, &last, seq, &bytes))
            .await
            .map_err(|error| error.to_string())?
    }

    // ----- the task ------------------------------------------------------------

    fn running(&self) -> bool {
        self.config.borrow().status.task_state == "running"
    }

    /// Starts a task unless one is running. `Err(true)` is the conflict,
    /// `Err(false)` a failure (the message is logged or returned by the caller).
    async fn begin(self: &Rc<Self>) -> Result<(), BeginError> {
        if self.phase.get() != Phase::Running {
            return Err(BeginError::Failed("the host is stopping".to_owned()));
        }
        if self.running() {
            return Err(BeginError::Conflict);
        }
        let token = self.cancel.child_token();
        let id = self.task_seq.get() + 1;
        self.task_seq.set(id);
        let before = self.config.borrow().status.clone();
        {
            let mut config = self.config.borrow_mut();
            config.status.task_state = "running".to_owned();
            config.status.last_run_start_time = GoTime(Some(self.now()));
            config.status.last_run_end_time = GoTime(None);
            config.status.last_run_domain_count = 0;
            config.status.last_error.clear();
            config.status.progress = Progress::default();
        }
        *self.task.borrow_mut() = Some((id, token.clone()));
        if let Err(error) = self.persist().await {
            self.release_task(id);
            let mut config = self.config.borrow_mut();
            config.status = before;
            config.status.task_state = "idle".to_owned();
            return Err(BeginError::Failed(format!(
                "failed to persist running state: {error}"
            )));
        }
        // The host may have stopped while the state was being written; nothing
        // may start then (`close` has already joined its jobs).
        if self.phase.get() != Phase::Running {
            self.release_task(id);
            {
                let mut config = self.config.borrow_mut();
                config.status.task_state = "cancelled".to_owned();
                config.status.last_error = "task cancelled by host shutdown".to_owned();
                config.status.last_run_end_time = GoTime(Some(self.now()));
            }
            let _ = self.persist().await;
            return Err(BeginError::Failed("the host is stopping".to_owned()));
        }
        let this = Rc::clone(self);
        let mut jobs = self.jobs.borrow_mut();
        while jobs.try_join_next().is_some() {}
        jobs.spawn_local(async move { this.run_task(id, token).await });
        Ok(())
    }

    /// Forgets task `id` unless a newer task already replaced it.
    fn release_task(&self, id: u64) {
        let mut task = self.task.borrow_mut();
        if task.as_ref().is_some_and(|(current, _)| *current == id) {
            *task = None;
        }
    }

    async fn run_task(self: Rc<Self>, id: u64, token: TransportCancellation) {
        let mut end = self.task_steps(&token).await;
        // A host stop outranks whatever step it interrupted.
        if self.cancel.is_cancelled() && matches!(end, End::Failed(_)) {
            end = End::Cancelled;
        }
        {
            let mut config = self.config.borrow_mut();
            match end {
                End::Failed(message) => {
                    crate::diag::error_line!("{}: task failed: {message}", self.tag);
                    config.status.task_state = "failed".to_owned();
                    config.status.last_error = message;
                }
                End::Cancelled => {
                    config.status.task_state = "cancelled".to_owned();
                    config.status.last_error = if self.cancel.is_cancelled() {
                        "task cancelled by host shutdown".to_owned()
                    } else {
                        "task cancelled by user".to_owned()
                    };
                }
                End::Done => {
                    if config.status.task_state == "running" {
                        config.status.task_state = "idle".to_owned();
                    }
                }
            }
            config.status.last_run_end_time = GoTime(Some(self.now()));
        }
        if let Err(error) = self.persist().await {
            crate::diag::error_line!("{}: saving the task state failed: {error}", self.tag);
        }
        self.release_task(id);
    }

    async fn task_steps(self: &Rc<Self>, token: &TransportCancellation) -> End {
        let (save, flush, sources, limit_days, qps, resolver, delay) = {
            let config = self.config.borrow();
            let settings = &config.execution_settings;
            (
                config.url_actions.save_rules.clone().unwrap_or_default(),
                config.url_actions.flush_rules.clone().unwrap_or_default(),
                config
                    .domain_processing
                    .source_files
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|file| self.source_path(&file.path))
                    .collect::<Vec<_>>(),
                if settings.date_range_days > 0 {
                    settings.date_range_days
                } else {
                    30
                },
                if settings.queries_per_second > 0 {
                    settings.queries_per_second
                } else {
                    100
                },
                settings.resolver_address.clone(),
                Duration::from_millis(u64::try_from(settings.url_call_delay_ms).unwrap_or(0)),
            )
        };
        crate::diag::info_line!("{}: starting a task", self.tag);
        if let Err(error) = self.call_urls(token, &save, delay).await {
            return End::Failed(format!("failed during save_rules step: {error}"));
        }
        let domains = match self.merge_domains(token, sources, limit_days).await {
            Ok(domains) => domains,
            Err(error) => return End::Failed(format!("failed during domain merge: {error}")),
        };
        if domains.is_empty() {
            crate::diag::info_line!("{}: no domains found to process", self.tag);
            return End::Done;
        }
        if let Err(error) = self.call_urls(token, &flush, delay).await {
            return End::Failed(format!("failed during flush_rules step: {error}"));
        }
        {
            let mut config = self.config.borrow_mut();
            config.status.last_run_domain_count = i64::try_from(domains.len()).unwrap_or(i64::MAX);
            config.status.progress.total = i64::try_from(domains.len()).unwrap_or(i64::MAX);
        }
        crate::diag::info_line!("{}: re-querying {} domains", self.tag, domains.len());
        if self.resend(token, &domains, qps, &resolver).await.is_err() {
            return End::Cancelled;
        }
        if let Err(error) = self.call_urls(token, &save, delay).await {
            return End::Failed(format!("failed during final save_rules step: {error}"));
        }
        End::Done
    }

    /// Calls each URL with GET in order, `delay` apart; every answer must be 2xx.
    async fn call_urls(
        &self,
        token: &TransportCancellation,
        urls: &[String],
        delay: Duration,
    ) -> Result<(), String> {
        let api = self
            .env
            .borrow()
            .as_ref()
            .map(|env| env.api.clone())
            .ok_or_else(|| "the plugin is not running".to_owned())?;
        for (index, url) in urls.iter().enumerate() {
            if token.is_cancelled() {
                return Err("context canceled".to_owned());
            }
            let target = local_url::normalized(url, api.api);
            let shown = match &target {
                Target::Local(path) => format!(
                    "http://127.0.0.1:{}{path}",
                    api.api.map_or(0, |address| address.port())
                ),
                Target::Http(url) => url.clone(),
            };
            let (status, body) = api
                .call("GET", &target, None, URL_TIMEOUT, token)
                .await
                .map_err(|error| format!("failed to call URL {shown}: {error}"))?;
            if !(200..300).contains(&status) {
                return Err(format!(
                    "bad response from URL {shown}: status {status}, body: {}",
                    String::from_utf8_lossy(&body)
                ));
            }
            if index + 1 < urls.len() {
                tokio::select! {
                    biased;
                    () = token.cancelled() => return Err("context canceled".to_owned()),
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
        Ok(())
    }

    /// The distinct names of the source files: `full:` lines, and statistics
    /// lines (`count date name`) whose date is within `limit_days`.
    async fn merge_domains(
        &self,
        token: &TransportCancellation,
        sources: Vec<String>,
        limit_days: i64,
    ) -> Result<Vec<String>, String> {
        let now = self.now();
        let token = token.clone();
        tokio::task::spawn_blocking(move || {
            merge_source_files(&sources, limit_days, now, || token.is_cancelled())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Step 6: asks `resolver` for the A and AAAA records of every name at
    /// `qps` names per second. Failures of single queries are ignored.
    async fn resend(
        &self,
        token: &TransportCancellation,
        domains: &[String],
        qps: i64,
        resolver: &str,
    ) -> Result<(), ()> {
        let address = tokio::net::lookup_host(resolver)
            .await
            .ok()
            .and_then(|mut addresses| addresses.next());
        let period = Duration::from_secs(1) / u32::try_from(qps.clamp(1, 1_000_000)).unwrap_or(1);
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));
        let mut queries = tokio::task::JoinSet::new();
        let total = domains.len();
        let mut outcome = Ok(());
        for (index, raw) in domains.iter().enumerate() {
            tokio::select! {
                biased;
                () = token.cancelled() => {
                    outcome = Err(());
                    break;
                }
                _ = ticker.tick() => {}
            }
            while queries.try_join_next().is_some() {}
            let (name, ad, cd, dnssec) = split_flags(raw);
            if let Some(address) = address {
                for qtype in [1_u16, 28] {
                    let permit = tokio::select! {
                        biased;
                        () = token.cancelled() => {
                            outcome = Err(());
                            break;
                        }
                        permit = Arc::clone(&slots).acquire_owned() => match permit {
                            Ok(permit) => permit,
                            Err(_) => break,
                        },
                    };
                    let Some(packet) = query_packet(name, qtype, ad, cd, dnssec) else {
                        continue;
                    };
                    let token = token.clone();
                    queries.spawn_local(async move {
                        let _permit = permit;
                        let _ = tokio::select! {
                            biased;
                            () = token.cancelled() => None,
                            reply = exchange(address, &packet) => reply,
                        };
                    });
                }
            }
            if outcome.is_err() {
                break;
            }
            let processed = index + 1;
            self.config.borrow_mut().status.progress.processed =
                i64::try_from(processed).unwrap_or(i64::MAX);
            // Fewer writes: every hundredth name and the last.
            if processed % 100 == 0 || processed == total {
                let _ = self.persist().await;
            }
        }
        if outcome.is_err() {
            queries.abort_all();
        }
        while queries.join_next().await.is_some() {}
        outcome
    }

    // ----- the scheduler -------------------------------------------------------

    async fn scheduler(self: Rc<Self>) {
        loop {
            let planned = next_run(&self.config.borrow().scheduler, self.now());
            let Some(planned) = planned else {
                tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => return,
                    () = self.wake.notified() => continue,
                }
            };
            let wait = (planned - self.now())
                .try_into()
                .unwrap_or(Duration::ZERO)
                .clamp(Duration::from_millis(1), MAX_SLEEP);
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => return,
                () = self.wake.notified() => continue,
                () = tokio::time::sleep(wait) => {}
            }
            if self.now() < planned {
                continue;
            }
            crate::diag::info_line!("{}: the scheduler triggers a task", self.tag);
            match self.begin().await {
                Ok(()) => {}
                Err(BeginError::Conflict) => {
                    crate::diag::info_line!(
                        "{}: the scheduler skipped: a task is running",
                        self.tag
                    );
                }
                Err(BeginError::Failed(error)) => {
                    crate::diag::error_line!(
                        "{}: the scheduler failed to start a task: {error}",
                        self.tag
                    );
                }
            }
            // Guard against re-firing for the same instant.
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    // ----- HTTP ----------------------------------------------------------------

    /// `/plugins/<tag>/` + `action`: `GET ""`, `GET status`, `POST trigger`,
    /// `POST cancel`, `POST scheduler/config`, `GET stats/source_file_counts`.
    pub(crate) async fn handle(self: &Rc<Self>, method: &str, action: &str, body: &[u8]) -> Reply {
        let allowed = match action {
            "" | "status" | "stats/source_file_counts" => "GET",
            "trigger" | "cancel" | "scheduler/config" => "POST",
            _ => return plain_reply(404, Some(TEXT), "404 page not found\n"),
        };
        if method != allowed {
            return plain_reply(405, None, "");
        }
        if method == "POST" && self.phase.get() != Phase::Running {
            return plain_reply(503, Some(TEXT), "host is applying configuration\n");
        }
        match action {
            "" => json_reply(200, &*self.config.borrow()),
            "status" => json_reply(200, &self.config.borrow().status),
            "trigger" => match self.begin().await {
                Ok(()) => success_reply("A new task has been started."),
                Err(BeginError::Conflict) => error_reply(409, "A task is already running."),
                Err(BeginError::Failed(error)) => {
                    error_reply(500, &format!("Failed to start task: {error}"))
                }
            },
            "cancel" => {
                let token = self.task.borrow().as_ref().map(|(_, token)| token.clone());
                match token {
                    Some(token) if self.running() => {
                        token.cancel();
                        success_reply("Task cancellation initiated.")
                    }
                    _ => error_reply(404, "No running task to cancel."),
                }
            }
            "scheduler/config" => self.update_scheduler(body).await,
            _ => self.source_file_counts().await,
        }
    }

    async fn update_scheduler(&self, body: &[u8]) -> Reply {
        #[derive(Deserialize)]
        struct Payload {
            #[serde(default, deserialize_with = "null_default")]
            enabled: bool,
            #[serde(default, deserialize_with = "null_default")]
            start_datetime: String,
            #[serde(default, deserialize_with = "null_default")]
            interval_minutes: i64,
            #[serde(default, deserialize_with = "null_default")]
            date_range_days: i64,
        }
        let Ok(payload) = serde_json::from_slice::<Payload>(body) else {
            return error_reply(400, "Invalid JSON body");
        };
        let before = self.config.borrow().clone();
        {
            let mut config = self.config.borrow_mut();
            config.scheduler = SchedulerConfig {
                enabled: payload.enabled,
                start_datetime: payload.start_datetime,
                interval_minutes: payload.interval_minutes,
            };
            // Only a positive value changes it, so a missing field never zeroes it.
            if payload.date_range_days > 0 {
                config.execution_settings.date_range_days = payload.date_range_days;
            }
        }
        if self.persist().await.is_err() {
            // The file keeps the previous settings, and so does the memory.
            let status = self.config.borrow().status.clone();
            *self.config.borrow_mut() = Config { status, ..before };
            return error_reply(500, "Failed to save updated config");
        }
        self.wake.notify_one();
        success_reply("Scheduler configuration updated successfully.")
    }

    async fn source_file_counts(self: &Rc<Self>) -> Reply {
        let (save, sources, delay) = {
            let config = self.config.borrow();
            (
                config.url_actions.save_rules.clone().unwrap_or_default(),
                config
                    .domain_processing
                    .source_files
                    .clone()
                    .unwrap_or_default(),
                Duration::from_millis(
                    u64::try_from(config.execution_settings.url_call_delay_ms).unwrap_or(0),
                ),
            )
        };
        if let Err(error) = self.call_urls(&self.cancel, &save, delay).await {
            return error_reply(
                500,
                &format!("Failed to save rules before counting: {error}"),
            );
        }
        let sources: Vec<(String, String)> = sources
            .into_iter()
            .map(|source| (source.alias, self.source_path(&source.path)))
            .collect();
        let counted = tokio::task::spawn_blocking(move || {
            let mut counts = Vec::new();
            for (alias, path) in sources {
                let count = count_full_lines(&path)
                    .map_err(|error| format!("Failed to read source file {path}: {error}"))?;
                counts.push(serde_json::json!({ "alias": alias, "count": count }));
            }
            Ok::<_, String>(counts)
        })
        .await;
        match counted {
            Ok(Ok(counts)) => json_reply(
                200,
                &serde_json::json!({ "status": "success", "data": counts }),
            ),
            Ok(Err(message)) => error_reply(500, &message),
            Err(error) => error_reply(500, &error.to_string()),
        }
    }

    /// Test access: the settled state (`task_state`).
    #[doc(hidden)]
    #[must_use]
    pub fn task_state(&self) -> String {
        self.config.borrow().status.task_state.clone()
    }
}

enum BeginError {
    Conflict,
    Failed(String),
}

// ----- source files and queries ---------------------------------------------------

#[allow(clippy::cast_precision_loss)]
/// Reads the source files into the distinct names to ask for. A missing file
/// is skipped; `cancelled` is checked between files.
fn merge_source_files(
    paths: &[String],
    limit_days: i64,
    now: OffsetDateTime,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut names = Vec::new();
    for path in paths {
        if cancelled() {
            return Err("context canceled".to_owned());
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("failed to open source file {path}: {error}")),
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let name = if let Some(rest) = line.strip_prefix("full:") {
                let name = rest.trim();
                (!name.is_empty()).then_some(name)
            } else if line.as_bytes()[0].is_ascii_digit() {
                // `count date name`: kept while the date is within the range.
                let mut fields = line.split_whitespace();
                let (_, date, name) = (fields.next(), fields.next(), fields.next());
                match (date.and_then(parse_date), name) {
                    (Some(date), Some(name))
                        if (now - date).as_seconds_f64() / 86400.0 <= limit_days as f64 =>
                    {
                        Some(name)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(name) = name
                && seen.insert(name.to_owned())
            {
                names.push(name.to_owned());
            }
        }
    }
    Ok(names)
}

/// `2006-01-02` at 00:00 UTC.
fn parse_date(text: &str) -> Option<OffsetDateTime> {
    let mut parts = text.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    if !(year.bytes().chain(month.bytes()).chain(day.bytes())).all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let date = time::Date::from_calendar_date(
        year.parse().ok()?,
        time::Month::try_from(month.parse::<u8>().ok()?).ok()?,
        day.parse().ok()?,
    )
    .ok()?;
    Some(date.midnight().assume_utc())
}

/// The lines of a source file that start with `full:` and carry a name.
fn count_full_lines(path: &str) -> Result<usize, std::io::Error> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| {
                line.strip_prefix("full:")
                    .is_some_and(|rest| !rest.is_empty())
            })
            .count()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

/// `name|AD|CD|DO` -> the name and which flags it carries.
fn split_flags(raw: &str) -> (&str, bool, bool, bool) {
    let mut parts = raw.split('|');
    let name = parts.next().unwrap_or_default();
    let (mut ad, mut cd, mut dnssec) = (false, false, false);
    for flag in parts {
        match flag {
            "AD" => ad = true,
            "CD" => cd = true,
            "DO" => dnssec = true,
            _ => {}
        }
    }
    (name, ad, cd, dnssec)
}

/// A recursive IN query for `name`: AD/CD as flags, DO through a 4096-byte OPT.
fn query_packet(name: &str, qtype: u16, ad: bool, cd: bool, dnssec: bool) -> Option<Vec<u8>> {
    let mut id = [0_u8; 2];
    getrandom::fill(&mut id).ok()?;
    let mut flags = 0x0100_u16;
    if ad {
        flags |= 0x0020;
    }
    if cd {
        flags |= 0x0010;
    }
    let mut packet = Vec::with_capacity(64);
    packet.extend_from_slice(&id);
    packet.extend_from_slice(&flags.to_be_bytes());
    packet.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, u8::from(dnssec)]);
    packet.extend_from_slice(&wire_name(name)?);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&[0, 1]);
    if dnssec {
        packet.extend_from_slice(&[0, 0, 41, 0x10, 0x00, 0, 0, 0x80, 0, 0, 0]);
    }
    Some(packet)
}

/// A presentation-form name (with `\.` and `\DDD` escapes) in wire form.
fn wire_name(name: &str) -> Option<Vec<u8>> {
    let name = name.strip_suffix('.').unwrap_or(name);
    let mut wire = Vec::with_capacity(name.len() + 2);
    let mut label: Vec<u8> = Vec::new();
    let bytes = name.as_bytes();
    let mut index = 0;
    let flush = |label: &mut Vec<u8>, wire: &mut Vec<u8>| -> Option<()> {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        wire.push(u8::try_from(label.len()).ok()?);
        wire.append(label);
        Some(())
    };
    while index < bytes.len() {
        match bytes[index] {
            b'.' => flush(&mut label, &mut wire)?,
            b'\\' => {
                index += 1;
                let next = *bytes.get(index)?;
                if next.is_ascii_digit() {
                    let digits = bytes.get(index..index + 3)?;
                    let value = std::str::from_utf8(digits).ok()?.parse::<u16>().ok()?;
                    label.push(u8::try_from(value).ok()?);
                    index += 2;
                } else {
                    label.push(next);
                }
            }
            byte => label.push(byte),
        }
        index += 1;
    }
    if !label.is_empty() {
        flush(&mut label, &mut wire)?;
    }
    wire.push(0);
    (wire.len() <= 255).then_some(wire)
}

/// Sends `packet` to `address` from a fresh socket and waits for one datagram.
async fn exchange(address: SocketAddr, packet: &[u8]) -> Option<Vec<u8>> {
    let bind: SocketAddr = if address.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0_u16; 8], 0).into()
    };
    let socket = tokio::net::UdpSocket::bind(bind).await.ok()?;
    socket.connect(address).await.ok()?;
    socket.send(packet).await.ok()?;
    let mut reply = vec![0_u8; 4096];
    let length = tokio::time::timeout(QUERY_TIMEOUT, socket.recv(&mut reply))
        .await
        .ok()?
        .ok()?;
    reply.truncate(length);
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).unwrap()
    }

    fn scheduler(start: &str, minutes: i64) -> SchedulerConfig {
        SchedulerConfig {
            enabled: true,
            start_datetime: start.to_owned(),
            interval_minutes: minutes,
        }
    }

    #[test]
    fn the_next_run_follows_the_reference_rule() {
        let start = "2025-10-14T20:30:00.000Z";
        // Start ahead of now: the start itself.
        assert_eq!(
            next_run(&scheduler(start, 60), at("2025-10-14T20:00:00Z")),
            Some(at(start))
        );
        // Start passed: the next multiple of the interval after it.
        assert_eq!(
            next_run(&scheduler(start, 60), at("2025-10-14T21:29:59Z")),
            Some(at("2025-10-14T21:30:00Z"))
        );
        // Exactly on a boundary: the following one.
        assert_eq!(
            next_run(&scheduler(start, 60), at("2025-10-14T21:30:00Z")),
            Some(at("2025-10-14T22:30:00Z"))
        );
        // Years later: a multiple of the interval after the start, ahead of now.
        let now = at("2026-10-08T00:00:00Z");
        let next = next_run(&scheduler(start, 4320), now).unwrap();
        assert!(next > now && next - now <= time::Duration::minutes(4320));
        assert_eq!(((next - at(start)).whole_minutes()) % 4320, 0);
        // Off, no start, bad start, no interval.
        let mut off = scheduler(start, 60);
        off.enabled = false;
        assert_eq!(next_run(&off, at("2025-10-14T20:00:00Z")), None);
        assert_eq!(
            next_run(&scheduler("", 60), at("2025-10-14T20:00:00Z")),
            None
        );
        assert_eq!(
            next_run(&scheduler("tomorrow", 60), at("2025-10-14T20:00:00Z")),
            None
        );
        assert_eq!(
            next_run(&scheduler(start, 0), at("2025-10-14T20:00:00Z")),
            None
        );
        assert_eq!(
            next_run(&scheduler(start, -5), at("2025-10-14T20:00:00Z")),
            None
        );
    }

    #[test]
    fn times_print_like_go() {
        let text = r#"{"status":{"last_run_start_time":"2026-10-03T20:30:00.001381517Z","last_run_end_time":"2026-10-03T20:46:39.422409373+08:00"}}"#;
        let config: Config = serde_json::from_str(text).unwrap();
        let out = String::from_utf8(marshal_indent(&config)).unwrap();
        assert!(
            out.contains("\"last_run_start_time\": \"2026-10-03T20:30:00.001381517Z\""),
            "{out}"
        );
        assert!(
            out.contains("\"last_run_end_time\": \"2026-10-03T12:46:39.422409373Z\""),
            "{out}"
        );
        assert!(out.contains("\"source_files\": null"), "{out}");
        assert!(!out.contains("last_error"));
        let zero = Config::default();
        assert!(
            String::from_utf8(marshal_indent(&zero))
                .unwrap()
                .contains("\"last_run_start_time\": \"0001-01-01T00:00:00Z\"")
        );
        let whole = GoTime(Some(at("2026-01-02T03:04:05Z")));
        assert_eq!(
            serde_json::to_string(&whole).unwrap(),
            "\"2026-01-02T03:04:05Z\""
        );
        let fraction = GoTime(Some(at("2026-01-02T03:04:05.120Z")));
        assert_eq!(
            serde_json::to_string(&fraction).unwrap(),
            "\"2026-01-02T03:04:05.12Z\""
        );
    }

    #[test]
    fn source_lines_are_merged_filtered_and_deduplicated() {
        let dir = std::env::temp_dir().join(format!("requery-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("a.txt");
        std::fs::write(
            &first,
            "full:rule.example\n  full:  spaced.example  \n0000000004 2026-10-05 fresh.example\n0000000001 2026-08-01 stale.example\n0000000002 2026-10-05 fresh.example|AD\n0000000002 2026-10-05 fresh.example\nnot-a-line\n\n7 bad-date x.example\n3 2026-13-40 y.example\n",
        )
        .unwrap();
        let names = merge_source_files(
            &[
                first.display().to_string(),
                dir.join("missing.txt").display().to_string(),
            ],
            30,
            at("2026-10-08T12:00:00Z"),
            || false,
        )
        .unwrap();
        assert_eq!(
            names,
            [
                "rule.example",
                "spaced.example",
                "fresh.example",
                "fresh.example|AD"
            ]
        );
        // The reference counts untrimmed lines that start with `full:`.
        assert_eq!(count_full_lines(&first.display().to_string()).unwrap(), 1);
        assert_eq!(
            count_full_lines(&dir.join("none").display().to_string()).unwrap(),
            0
        );
        assert!(
            merge_source_files(
                &[first.display().to_string()],
                30,
                at("2026-10-08T12:00:00Z"),
                || true
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_packets_carry_the_flags() {
        assert_eq!(
            split_flags("a.example|AD|DO"),
            ("a.example", true, false, true)
        );
        let plain = query_packet("a.example", 1, false, false, false).unwrap();
        assert_eq!(plain[2..4], [0x01, 0x00]);
        assert_eq!(plain[10..12], [0, 0]);
        let full = query_packet("a.example", 28, true, true, true).unwrap();
        assert_eq!(full[2..4], [0x01, 0x30]);
        assert_eq!(full[10..12], [0, 1]);
        assert!(full.ends_with(&[0, 0, 41, 0x10, 0, 0, 0, 0x80, 0, 0, 0]));
        assert_eq!(
            wire_name("a\\.b.example."),
            Some(b"\x03a.b\x07example\0".to_vec())
        );
        assert_eq!(
            wire_name("a\\032b.example"),
            Some(b"\x03a b\x07example\0".to_vec())
        );
        assert_eq!(wire_name("a..example"), None);
        assert_eq!(wire_name(&format!("{}.example", "x".repeat(64))), None);
    }
}
