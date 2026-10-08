//! The `domain_output` plugin: counts the query names that pass through a
//! sequence, persists them (`file_stat`, `file_rule`, `gen_rule`), publishes
//! the generated rule list to a `domain_set_light` and injects each new name
//! into a `domain_mapper` right away (`target_mapper`).
//!
//! Recording is a hash-map update on the DNS thread; every write works on a
//! snapshot and runs on the blocking pool through a same-directory temporary
//! file, so a failed write keeps the previous file. See
//! `docs/rust/contracts/native-domain-output.md`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mosdns_dns_core::QuestionInfo;
use mosdns_upstream_core::TransportCancellation;

use crate::config::providers::{HotLayer, presentation_name};
use crate::rule_sources::{Reply, RuleSources};

/// Distinct keys held at once. The reference grows without bound; new keys
/// beyond this are not recorded (existing keys keep counting).
pub(crate) const MAX_KEYS: usize = 262_144;
/// How long a cached calendar date may be stale.
const DATE_REFRESH: Duration = Duration::from_secs(60);
/// Deadline of one `domain_set_url` push.
const PUSH_TIMEOUT: Duration = Duration::from_secs(5);
const TEXT: &str = "text/plain; charset=utf-8";

/// The validated arguments of one plugin.
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub(crate) file_stat: Option<PathBuf>,
    pub(crate) file_rule: Option<PathBuf>,
    pub(crate) gen_rule: Option<PathBuf>,
    pub(crate) pattern: String,
    pub(crate) appended_string: String,
    /// Records since the last write that trigger an early write; `<= 0` never.
    pub(crate) max_entries: i64,
    pub(crate) dump_interval: Duration,
    pub(crate) domain_set_url: String,
    pub(crate) enable_flags: bool,
    /// Most distinct keys held at once ([`MAX_KEYS`] unless a test lowers it).
    pub(crate) max_keys: usize,
}

impl Settings {
    /// The files this plugin writes.
    pub(crate) fn files(&self) -> Vec<PathBuf> {
        [&self.file_stat, &self.file_rule, &self.gen_rule]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }
}

/// `target_mapper` and what to inject into it.
pub(crate) struct MapperTarget {
    pub(crate) layer: Rc<HotLayer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Periodic,
    Flush,
    Save,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Running,
    /// A newer view owns the files: nothing is recorded or written any more.
    Fenced,
    Closed,
}

struct Entry {
    count: u64,
    date: Arc<str>,
}

struct State {
    stats: HashMap<Box<str>, Entry>,
    /// Records since the last write (the early-write trigger).
    since_write: i64,
    today: Arc<str>,
    today_at: std::time::Instant,
    warned_cap: bool,
}

impl State {
    fn today(&mut self) -> Arc<str> {
        if self.today_at.elapsed() >= DATE_REFRESH {
            self.refresh_date();
        }
        Arc::clone(&self.today)
    }

    fn refresh_date(&mut self) {
        let date = crate::local_time::local_date();
        if *self.today != *date {
            self.today = Arc::from(date);
        }
        self.today_at = std::time::Instant::now();
    }
}

/// One row of a snapshot.
#[derive(Clone)]
struct Row {
    key: String,
    count: u64,
    date: Arc<str>,
}

/// Where the generated list goes.
enum Push {
    Off,
    /// This host's own `domain_set_light` `/post`, called in-process.
    Local(String),
    Http(String),
}

struct Env {
    sources: Rc<RuleSources>,
    push: Push,
    roots: Option<Arc<rustls::RootCertStore>>,
}

/// The runtime owner of one `domain_output` plugin.
pub struct DomainOutput {
    pub tag: String,
    settings: Arc<Settings>,
    target: Option<MapperTarget>,
    /// Always on until the settings layer (M1-f) drives it.
    enabled: Cell<bool>,
    state: RefCell<State>,
    seq: Cell<u64>,
    last_written: Arc<Mutex<u64>>,
    wake: tokio::sync::Notify,
    phase: Cell<Phase>,
    started: Cell<bool>,
    cancel: TransportCancellation,
    jobs: RefCell<tokio::task::JoinSet<()>>,
    env: RefCell<Option<Env>>,
    pending_push: RefCell<Option<Vec<String>>>,
    pushing: Cell<bool>,
}

/// The compiled plugin: the external it is dispatched as, and its owner.
pub struct DomainOutputConfig {
    pub tag: String,
    pub executable: mosdns_sequence_core::ExecutableId,
    pub runtime: Rc<DomainOutput>,
}

impl DomainOutput {
    pub(crate) fn new(tag: &str, settings: Settings, target: Option<MapperTarget>) -> Self {
        Self {
            tag: tag.to_owned(),
            settings: Arc::new(settings),
            target,
            enabled: Cell::new(true),
            state: RefCell::new(State {
                stats: HashMap::new(),
                since_write: 0,
                today: Arc::from(crate::local_time::local_date()),
                today_at: std::time::Instant::now(),
                warned_cap: false,
            }),
            seq: Cell::new(0),
            last_written: Arc::new(Mutex::new(0)),
            wake: tokio::sync::Notify::new(),
            phase: Cell::new(Phase::Running),
            started: Cell::new(false),
            cancel: TransportCancellation::new(),
            jobs: RefCell::new(tokio::task::JoinSet::new()),
            env: RefCell::new(None),
            pending_push: RefCell::new(None),
            pushing: Cell::new(false),
        }
    }

    // ----- recording ---------------------------------------------------------

    /// Counts the question of one query. `raw` is the query as it is now.
    pub(crate) fn record(&self, raw: &[u8], question: &QuestionInfo) {
        if !self.enabled.get() || self.phase.get() != Phase::Running {
            return;
        }
        let Some(presentation) = presentation_name(&question.qname_wire) else {
            return;
        };
        let name = presentation.strip_suffix('.').unwrap_or(&presentation);
        if name.is_empty() {
            return;
        }
        let mut key = name.to_ascii_lowercase();
        let plain = key.len();
        if self.settings.enable_flags {
            let (ad, cd) = query_flags(raw);
            for (set, flag) in [(ad, "|AD"), (cd, "|CD")] {
                if set {
                    key.push_str(flag);
                }
            }
        }
        let mut state = self.state.borrow_mut();
        let today = state.today();
        if let Some(entry) = state.stats.get_mut(key.as_str()) {
            entry.count += 1;
            if *entry.date != *today {
                entry.date = today;
            }
        } else {
            if state.stats.len() >= self.settings.max_keys {
                if !state.warned_cap {
                    state.warned_cap = true;
                    crate::diag::warn_line!(
                        "{}: {} distinct names recorded; new names are not recorded",
                        self.tag,
                        self.settings.max_keys
                    );
                }
                return;
            }
            if let Some(target) = &self.target {
                target.layer.add(&key[..plain]);
            }
            state.stats.insert(
                key.into_boxed_str(),
                Entry {
                    count: 1,
                    date: today,
                },
            );
        }
        state.since_write += 1;
        let trigger =
            self.settings.max_entries > 0 && state.since_write >= self.settings.max_entries;
        drop(state);
        if trigger {
            self.wake.notify_one();
        }
    }

    // ----- lifecycle ---------------------------------------------------------

    /// Loads the statistics file and starts the periodic writer. Called when
    /// the host (or a newly installed view) starts serving.
    pub(crate) fn start(self: &Rc<Self>, sources: Rc<RuleSources>, api: Option<SocketAddr>) {
        if self.started.replace(true) || self.phase.get() != Phase::Running {
            return;
        }
        self.load();
        let push = classify(&self.settings.domain_set_url, api, &sources);
        let roots = sources.roots();
        *self.env.borrow_mut() = Some(Env {
            sources,
            push,
            roots,
        });
        let this = Rc::clone(self);
        self.jobs.borrow_mut().spawn_local(async move {
            this.run().await;
        });
    }

    /// A newer view takes over: save what this one holds, synchronously, and
    /// never write again. Called before the new view starts, so the new view
    /// loads the saved file.
    pub(crate) fn fence(&self) {
        if self.phase.get() != Phase::Running {
            return;
        }
        self.phase.set(Phase::Fenced);
        self.cancel.cancel();
        if self.started.get() {
            let rows = self.snapshot();
            let seq = self.next_seq();
            // A publication still in flight writes the provider's file (usually
            // `file_rule` too) after this save; leave the rule files to it.
            let rules = !self.pushing.get() && self.pending_push.borrow().is_none();
            if let Err(error) = persist(&self.settings, &self.last_written, seq, &rows, rules) {
                crate::diag::error_line!("{}: final save failed: {error}", self.tag);
            }
        }
    }

    /// Stops the writer and, unless a newer view took over, saves once more.
    pub(crate) async fn close(&self) {
        let was = self.phase.replace(Phase::Closed);
        self.cancel.cancel();
        let mut jobs = std::mem::take(&mut *self.jobs.borrow_mut());
        while jobs.join_next().await.is_some() {}
        if was == Phase::Running && self.started.get() {
            let rows = self.snapshot();
            let seq = self.next_seq();
            let settings = Arc::clone(&self.settings);
            let last = Arc::clone(&self.last_written);
            let tag = self.tag.clone();
            match tokio::task::spawn_blocking(move || persist(&settings, &last, seq, &rows, true))
                .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => crate::diag::error_line!("{tag}: final save failed: {error}"),
                Err(error) => crate::diag::error_line!("{tag}: final save failed: {error}"),
            }
        }
    }

    async fn run(self: Rc<Self>) {
        let period = self.settings.dump_interval;
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => return,
                _ = ticker.tick() => {}
                () = self.wake.notified() => {}
            }
            if let Err(error) = self.write(Mode::Periodic).await {
                crate::diag::error_line!("{}: periodic write failed: {error}", self.tag);
            }
        }
    }

    // ----- persistence -------------------------------------------------------

    fn next_seq(&self) -> u64 {
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        seq
    }

    fn snapshot(&self) -> Vec<Row> {
        let mut state = self.state.borrow_mut();
        state.since_write = 0;
        state
            .stats
            .iter()
            .map(|(key, entry)| Row {
                key: key.to_string(),
                count: entry.count,
                date: Arc::clone(&entry.date),
            })
            .collect()
    }

    /// One write in `mode`: snapshot on this thread, files on the blocking
    /// pool, then (except at shutdown) publish the name list.
    async fn write(self: &Rc<Self>, mode: Mode) -> Result<(), String> {
        let rows = {
            let mut state = self.state.borrow_mut();
            state.refresh_date();
            match mode {
                Mode::Periodic => {
                    if !self.enabled.get() || state.stats.is_empty() {
                        return Ok(());
                    }
                }
                Mode::Flush => {
                    state.stats = HashMap::new();
                    state.warned_cap = false;
                    if let Some(target) = &self.target {
                        target.layer.clear();
                    }
                }
                Mode::Save | Mode::Shutdown => {}
            }
            drop(state);
            self.snapshot()
        };
        let seq = self.next_seq();
        let settings = Arc::clone(&self.settings);
        let last = Arc::clone(&self.last_written);
        let names =
            tokio::task::spawn_blocking(move || persist(&settings, &last, seq, &rows, true))
                .await
                .map_err(|error| error.to_string())??;
        if mode != Mode::Shutdown
            && let Some(names) = names
        {
            self.schedule_push(names);
        }
        Ok(())
    }

    fn load(&self) {
        let Some(path) = &self.settings.file_stat else {
            return;
        };
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut state = self.state.borrow_mut();
        let today = state.today();
        let mut dates: HashMap<String, Arc<str>> = HashMap::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (count, date, key) = match fields.as_slice() {
                [count, key] => (*count, None, *key),
                [count, date, key, ..] => (*count, Some(*date), *key),
                _ => continue,
            };
            let date = date.map_or_else(
                || Arc::clone(&today),
                |date| {
                    Arc::clone(
                        dates
                            .entry(date.to_owned())
                            .or_insert_with(|| Arc::from(date)),
                    )
                },
            );
            // What was learned before keeps its routing even when the rule
            // file is older than the statistics (a managed apply, a crash).
            if let Some(target) = &self.target {
                target.layer.add(key.split('|').next().unwrap_or_default());
            }
            state.stats.insert(
                key.into(),
                Entry {
                    count: count.parse().unwrap_or(0),
                    date,
                },
            );
        }
    }

    // ----- rule publication --------------------------------------------------

    /// Queues `names` for publication; a newer list replaces one still waiting.
    fn schedule_push(self: &Rc<Self>, names: Vec<String>) {
        if self.phase.get() != Phase::Running || self.env.borrow().is_none() {
            return;
        }
        if matches!(
            self.env.borrow().as_ref().map(|env| &env.push),
            Some(Push::Off)
        ) {
            return;
        }
        *self.pending_push.borrow_mut() = Some(names);
        if self.pushing.replace(true) {
            return;
        }
        let this = Rc::clone(self);
        let mut jobs = self.jobs.borrow_mut();
        while jobs.try_join_next().is_some() {}
        jobs.spawn_local(async move {
            loop {
                let next = this.pending_push.borrow_mut().take();
                let Some(names) = next else { break };
                if this.cancel.is_cancelled() {
                    break;
                }
                this.push(&names).await;
            }
            this.pushing.set(false);
        });
    }

    async fn push(&self, names: &[String]) {
        let values: Vec<String> = names.iter().map(|name| format!("full:{name}")).collect();
        let body = serde_json::to_vec(&serde_json::json!({ "values": values })).unwrap_or_default();
        let (sources, roots, local, http) = {
            let env = self.env.borrow();
            let Some(env) = env.as_ref() else { return };
            let (local, http) = match &env.push {
                Push::Off => return,
                Push::Local(tag) => (Some(tag.clone()), None),
                Push::Http(url) => (None, Some(url.clone())),
            };
            (Rc::clone(&env.sources), env.roots.clone(), local, http)
        };
        // The in-process call is never dropped half way (its file write runs on
        // the blocking pool and the provider lock is held across it); the HTTP
        // one is bounded.
        let outcome = if let Some(tag) = local {
            Ok(self.publish_local(&sources, &tag, &body).await)
        } else if let Some(url) = http {
            tokio::time::timeout(
                PUSH_TIMEOUT,
                crate::rule_download::post_json(&url, &body, PUSH_TIMEOUT, roots, &self.cancel),
            )
            .await
        } else {
            return;
        };
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                crate::diag::error_line!("{}: publishing the rules failed: {error}", self.tag);
            }
            Err(_) => crate::diag::error_line!("{}: publishing the rules timed out", self.tag),
        }
    }

    async fn publish_local(
        &self,
        sources: &RuleSources,
        tag: &str,
        body: &[u8],
    ) -> Result<(), String> {
        let Some(provider) = sources.provider(tag) else {
            return Err(format!("no rule provider `{tag}` to publish to"));
        };
        let reply = sources.handle(&provider, "POST", "post", "", body).await;
        if reply.status >= 400 {
            return Err(format!(
                "publishing to `{tag}` answered {}: {}",
                reply.status,
                String::from_utf8_lossy(&reply.body).trim()
            ));
        }
        Ok(())
    }

    // ----- HTTP ----------------------------------------------------------------

    /// `GET /flush`, `/save`, `/show`, `/restartall` on `/plugins/<tag>/`.
    pub(crate) async fn handle(self: &Rc<Self>, method: &str, action: &str, query: &str) -> Reply {
        match (action, method) {
            ("flush" | "save" | "show" | "restartall", "GET") => {}
            ("flush" | "save" | "show" | "restartall", _) => return reply(405, None, Vec::new()),
            _ => return text(404, "404 page not found\n"),
        }
        match action {
            "show" => self.show(query),
            "restartall" => text(
                501,
                "restart is not supported by this host; restart the service instead\n",
            ),
            _ if self.phase.get() != Phase::Running || !self.started.get() => {
                text(503, "host is applying configuration\n")
            }
            "flush" => match self.write(Mode::Flush).await {
                Ok(()) => reply(200, Some(TEXT), b"domain_output flushed".to_vec()),
                Err(error) => text(500, &format!("{error}\n")),
            },
            _ => match self.write(Mode::Save).await {
                Ok(()) => reply(200, Some(TEXT), b"domain_output saved".to_vec()),
                Err(error) => text(500, &format!("{error}\n")),
            },
        }
    }

    fn show(&self, query: &str) -> Reply {
        let (mut search, mut limit, mut offset) = (None, None, None);
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let slot = match name {
                "q" => &mut search,
                "limit" => &mut limit,
                "offset" => &mut offset,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(decode(value));
            }
        }
        let search = search.unwrap_or_default().to_lowercase();
        let limit = match limit.and_then(|value| value.parse::<i64>().ok()) {
            Some(limit) if limit > 0 => limit,
            _ => 100,
        };
        let offset = offset
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0);
        let window = usize::try_from(offset.saturating_add(limit)).unwrap_or(usize::MAX);
        let mut rows: Vec<(u64, Arc<str>, String)> = Vec::new();
        let mut total = 0_usize;
        {
            let state = self.state.borrow();
            for (key, entry) in &state.stats {
                if !search.is_empty() && !key.to_lowercase().contains(&search) {
                    continue;
                }
                total += 1;
                rows.push((entry.count, Arc::clone(&entry.date), key.to_string()));
            }
        }
        let order = |a: &(u64, Arc<str>, String), b: &(u64, Arc<str>, String)| {
            b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2))
        };
        if rows.len() > window {
            rows.select_nth_unstable_by(window, order);
            rows.truncate(window);
        }
        rows.sort_by(order);
        let skip = usize::try_from(offset).unwrap_or(usize::MAX);
        let mut body = String::new();
        for (count, date, key) in rows.iter().skip(skip) {
            body.push_str(&format!("{count:010} {date} {key}\n"));
        }
        let mut response = reply(200, Some(TEXT), body.into_bytes());
        response.headers = vec![
            ("X-Total-Count", total.to_string()),
            ("Access-Control-Expose-Headers", "X-Total-Count".to_owned()),
        ];
        response
    }

    /// Test access: the number of distinct keys held.
    #[doc(hidden)]
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.state.borrow().stats.len()
    }

    /// Test access: waits until no publication job is left.
    #[doc(hidden)]
    pub async fn settle(&self) {
        loop {
            if !self.pushing.get() && self.pending_push.borrow().is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn reply(status: u16, content_type: Option<&'static str>, body: Vec<u8>) -> Reply {
    Reply {
        status,
        content_type,
        body,
        headers: Vec::new(),
    }
}

fn text(status: u16, body: &str) -> Reply {
    reply(status, Some(TEXT), body.as_bytes().to_vec())
}

fn decode(value: &str) -> String {
    let value = value.replace('+', " ");
    crate::api::decode_plugin_tag(&value).map_or(value.clone(), std::borrow::Cow::into_owned)
}

/// AD and CD of a query wire. The reference also tests EDNS DO, but it does so
/// on a query whose OPT the server has already replaced by a fresh one without
/// DO, so DO never appears in a key; neither does it here.
fn query_flags(raw: &[u8]) -> (bool, bool) {
    raw.get(3).map_or((false, false), |flags| {
        (flags & 0x20 != 0, flags & 0x10 != 0)
    })
}

/// Where `domain_set_url` points: this host's own `/plugins/<tag>/post` is
/// called in-process, anything else over HTTP.
fn classify(url: &str, api: Option<SocketAddr>, sources: &RuleSources) -> Push {
    if url.is_empty() {
        return Push::Off;
    }
    if let (Ok(parsed), Some(api)) = (url::Url::parse(url), api)
        && parsed.scheme() == "http"
        && parsed.port_or_known_default() == Some(api.port())
        && let Some(tag) = parsed
            .path()
            .strip_prefix("/plugins/")
            .and_then(|rest| rest.strip_suffix("/post"))
            .filter(|tag| !tag.is_empty() && !tag.contains('/'))
    {
        let own = match parsed.host() {
            Some(url::Host::Domain(name)) => {
                name.eq_ignore_ascii_case("localhost")
                    && (api.ip().is_loopback() || api.ip().is_unspecified())
            }
            Some(url::Host::Ipv4(ip)) => is_own(IpAddr::V4(ip), api.ip()),
            Some(url::Host::Ipv6(ip)) => is_own(IpAddr::V6(ip), api.ip()),
            None => false,
        };
        if own
            && let Some(tag) = crate::api::decode_plugin_tag(tag)
            && sources.provider(&tag).is_some_and(|p| p.postable())
        {
            return Push::Local(tag.into_owned());
        }
    }
    Push::Http(url.to_owned())
}

/// Whether `host` reaches this host's API socket: its own address, or any
/// loopback address when the API listens on the wildcard.
fn is_own(host: IpAddr, api: IpAddr) -> bool {
    host == api || (api.is_unspecified() && host.is_loopback())
}

/// Sorts a snapshot and writes the three files. Returns the distinct names in
/// file order, or `None` when a newer snapshot was already written.
fn persist(
    settings: &Settings,
    last_written: &Mutex<u64>,
    seq: u64,
    rows: &[Row],
    rules: bool,
) -> Result<Option<Vec<String>>, String> {
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in &sorted {
        let name = row.key.split('|').next().unwrap_or_default();
        if seen.insert(name) {
            names.push(name.to_owned());
        }
    }
    let mut guard = last_written
        .lock()
        .map_err(|_| "write lock poisoned".to_owned())?;
    if seq <= *guard {
        return Ok(None);
    }
    let mut failure = None;
    let mut write = |path: Option<&Path>, render: &dyn Fn() -> Vec<u8>| {
        if let Some(path) = path
            && let Err(error) = write_file(path, &render())
        {
            failure.get_or_insert(format!("{}: {error}", path.display()));
        }
    };
    write(settings.file_stat.as_deref(), &|| {
        let mut out = String::new();
        for row in &sorted {
            out.push_str(&format!("{:010} {} {}\n", row.count, row.date, row.key));
        }
        out.into_bytes()
    });
    write(settings.file_rule.as_deref().filter(|_| rules), &|| {
        let mut out = String::new();
        for name in &names {
            out.push_str("full:");
            out.push_str(name);
            out.push('\n');
        }
        out.into_bytes()
    });
    write(settings.gen_rule.as_deref().filter(|_| rules), &|| {
        let mut out = String::new();
        if !settings.pattern.is_empty() {
            if !settings.appended_string.is_empty() {
                out.push_str(&settings.appended_string);
                out.push('\n');
            }
            for name in &names {
                out.push_str(&settings.pattern.replace("DOMAIN", name));
                out.push('\n');
            }
        }
        out.into_bytes()
    });
    *guard = seq;
    drop(guard);
    failure.map_or(Ok(Some(names)), Err)
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    crate::rule_sources::write_atomically(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(name: &str, flags: u8) -> Vec<u8> {
        let mut out = vec![0, 1, 0, flags, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            out.push(u8::try_from(label.len()).unwrap());
            out.extend(label.as_bytes());
        }
        out.push(0);
        out.extend([0, 1, 0, 1]);
        out
    }

    #[test]
    fn flags_come_from_the_header() {
        assert_eq!(query_flags(&wire("a.example", 0x20)), (true, false));
        assert_eq!(query_flags(&wire("a.example", 0x10)), (false, true));
        assert_eq!(query_flags(&wire("a.example", 0)), (false, false));
        assert_eq!(query_flags(&[0, 1]), (false, false));
    }

    #[test]
    fn a_stale_snapshot_never_overwrites_a_newer_one() {
        let dir = std::env::temp_dir().join(format!("domain-output-seq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            file_stat: Some(dir.join("stat.txt")),
            file_rule: Some(dir.join("rule.txt")),
            gen_rule: None,
            pattern: String::new(),
            appended_string: String::new(),
            max_entries: 0,
            dump_interval: Duration::from_secs(60),
            domain_set_url: String::new(),
            enable_flags: false,
            max_keys: MAX_KEYS,
        };
        let last = Mutex::new(0);
        let row = |key: &str, count| Row {
            key: key.to_owned(),
            count,
            date: Arc::from("2026-10-08"),
        };
        persist(&settings, &last, 2, &[row("new.example", 9)], true).unwrap();
        assert!(
            persist(&settings, &last, 1, &[row("old.example", 1)], true)
                .unwrap()
                .is_none()
        );
        let stat = std::fs::read_to_string(dir.join("stat.txt")).unwrap();
        assert_eq!(stat, "0000000009 2026-10-08 new.example\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_are_sorted_deduplicated_and_formatted_like_the_reference() {
        let dir = std::env::temp_dir().join(format!("domain-output-fmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = Settings {
            file_stat: Some(dir.join("stat.txt")),
            file_rule: Some(dir.join("rule.txt")),
            gen_rule: Some(dir.join("gen.txt")),
            pattern: "[/DOMAIN/]127.0.0.1".to_owned(),
            appended_string: "udp://1".to_owned(),
            max_entries: 0,
            dump_interval: Duration::from_secs(60),
            domain_set_url: String::new(),
            enable_flags: true,
            max_keys: MAX_KEYS,
        };
        let row = |key: &str, count| Row {
            key: key.to_owned(),
            count,
            date: Arc::from("2026-10-08"),
        };
        let names = persist(
            &settings,
            &Mutex::new(0),
            1,
            &[
                row("b.example", 2),
                row("a.example|AD|CD", 7),
                row("a.example", 1),
            ],
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(names, ["a.example", "b.example"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("stat.txt")).unwrap(),
            "0000000007 2026-10-08 a.example|AD|CD\n0000000002 2026-10-08 b.example\n0000000001 2026-10-08 a.example\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("rule.txt")).unwrap(),
            "full:a.example\nfull:b.example\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("gen.txt")).unwrap(),
            "udp://1\n[/a.example/]127.0.0.1\n[/b.example/]127.0.0.1\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn question(name: &str) -> QuestionInfo {
        let packet = wire(name, 0);
        mosdns_dns_core::parse_query(&packet).unwrap().1
    }

    fn quiet(max_keys: usize) -> Settings {
        Settings {
            file_stat: None,
            file_rule: None,
            gen_rule: None,
            pattern: String::new(),
            appended_string: String::new(),
            max_entries: 0,
            dump_interval: Duration::from_secs(60),
            domain_set_url: String::new(),
            enable_flags: true,
            max_keys,
        }
    }

    fn rows(output: &DomainOutput) -> Vec<(String, u64)> {
        let mut rows: Vec<(String, u64)> = output
            .state
            .borrow()
            .stats
            .iter()
            .map(|(key, entry)| (key.to_string(), entry.count))
            .collect();
        rows.sort();
        rows
    }

    #[test]
    fn names_are_lowercased_flagged_counted_and_the_root_is_skipped() {
        let output = DomainOutput::new("t", quiet(MAX_KEYS), None);
        for packet in [
            wire("Mixed.Example", 0x20),
            wire("mixed.example", 0x20),
            wire("mixed.example", 0x30),
            wire("mixed.example", 0),
        ] {
            let question = mosdns_dns_core::parse_query(&packet).unwrap().1;
            output.record(&packet, &question);
        }
        let root = wire("a", 0);
        let root_question = QuestionInfo {
            qname_wire: vec![0],
            qtype: 1,
            qclass: 1,
        };
        output.record(&root, &root_question);
        assert_eq!(
            rows(&output),
            [
                ("mixed.example".to_owned(), 1),
                ("mixed.example|AD".to_owned(), 2),
                ("mixed.example|AD|CD".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn the_key_cap_stops_new_names_but_not_counting() {
        let output = DomainOutput::new("t", quiet(2), None);
        for name in ["a.example", "b.example", "c.example", "a.example"] {
            let packet = wire(name, 0);
            output.record(&packet, &question(name));
        }
        assert_eq!(
            rows(&output),
            [("a.example".to_owned(), 2), ("b.example".to_owned(), 1)]
        );
    }

    #[test]
    fn a_new_name_is_injected_into_the_mapper_overlay_once() {
        let overlay = crate::config::providers::HotOverlay::default();
        let target = MapperTarget {
            layer: overlay.layer(4, "special"),
        };
        let output = DomainOutput::new("t", quiet(MAX_KEYS), Some(target));
        let packet = wire("New.Example", 0);
        output.record(&packet, &question("new.example"));
        output.record(&packet, &question("new.example"));
        assert_eq!(
            overlay.lookup("NEW.example."),
            Some((1 << 4, "special".to_owned()))
        );
        assert!(overlay.lookup("other.example.").is_none());
    }
}
