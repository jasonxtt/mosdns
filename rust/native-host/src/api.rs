//! The scoped management HTTP surface for the local-rule editing workflow.
//!
//! This listener is owned by the top-level host supervisor alongside the DNS
//! listener. It serves only the bounded routes needed to show, save and post
//! one eligible `domain_set`, plus the read-only special-group list. It is not
//! a general API host, it never owns the upstream catalog, and it never closes
//! shared upstream state.

use std::cell::Cell;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use mosdns_upstream_core::TransportCancellation;

use crate::config::CompiledConfig;
use crate::managed::ManagedDomainSet;
use crate::observer::{
    AnswerDetailsStatus, AuditClock, AuditReadSnapshot, AuditRecord, AuditStatsSnapshot,
    AuditTimingSnapshot, QueryObserver, ResponseState, UpstreamAttemptOutcome, UpstreamDiagnostics,
};
use crate::udp::{drain_tasks, reap_one_task};

pub(crate) const AUDIT_SETTINGS_FILENAME: &str = "audit_settings.json";
const MAX_AUDIT_CAPACITY: usize = 400_000;
static AUDIT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
pub(crate) struct AuditPersistenceFaults {
    fail_temp_write: Arc<AtomicBool>,
    fail_final_replace: Arc<AtomicBool>,
}

impl AuditPersistenceFaults {
    pub(crate) fn fail_next_temp_write(&self) {
        self.fail_temp_write.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_next_final_replace(&self) {
        self.fail_final_replace.store(true, Ordering::Relaxed);
    }

    fn take_temp_write_failure(&self) -> bool {
        self.fail_temp_write.swap(false, Ordering::Relaxed)
    }

    fn take_final_replace_failure(&self) -> bool {
        self.fail_final_replace.swap(false, Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
struct AuditSettings {
    #[serde(default)]
    capacity: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SavedAuditSettings {
    capacity: usize,
}

fn clamp_audit_capacity(value: i64) -> usize {
    if value < 0 {
        0
    } else {
        usize::try_from(value)
            .unwrap_or(MAX_AUDIT_CAPACITY)
            .min(MAX_AUDIT_CAPACITY)
    }
}

fn parse_audit_capacity(bytes: &[u8]) -> Result<usize, String> {
    let settings: AuditSettings =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    Ok(clamp_audit_capacity(settings.capacity.unwrap_or(0)))
}

fn audit_settings_path(root: &Path) -> PathBuf {
    root.join("webinfo").join(AUDIT_SETTINGS_FILENAME)
}

fn legacy_audit_settings_paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join("state").join(AUDIT_SETTINGS_FILENAME),
        root.join(AUDIT_SETTINGS_FILENAME),
    ]
}

fn audit_settings_diagnostic(path: &Path, message: impl AsRef<str>) {
    eprintln!(
        "native audit settings diagnostic: {} ({})",
        message.as_ref(),
        path.display()
    );
}

/// Loads the canonical settings first. A present but malformed canonical file
/// wins over legacy locations and falls back to the caller's default.
pub(crate) fn load_audit_capacity(root: &Path, fallback: usize) -> usize {
    let canonical = audit_settings_path(root);
    match std::fs::read(&canonical) {
        Ok(bytes) => match parse_audit_capacity(&bytes) {
            Ok(capacity) => capacity,
            Err(error) => {
                audit_settings_diagnostic(&canonical, format!("using default: {error}"));
                fallback
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            for legacy in legacy_audit_settings_paths(root) {
                match std::fs::read(&legacy) {
                    Ok(bytes) => {
                        let capacity = match parse_audit_capacity(&bytes) {
                            Ok(capacity) => capacity,
                            Err(error) => {
                                audit_settings_diagnostic(
                                    &legacy,
                                    format!("using default: {error}"),
                                );
                                return fallback;
                            }
                        };
                        if let Err(error) = migrate_legacy_settings(&canonical, &legacy, &bytes) {
                            audit_settings_diagnostic(
                                &legacy,
                                format!("migration failed: {error}"),
                            );
                        };
                        return capacity;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        audit_settings_diagnostic(&legacy, format!("using default: {error}"));
                        return fallback;
                    }
                }
            }
            audit_settings_diagnostic(&canonical, "using default: settings file is missing");
            fallback
        }
        Err(error) => {
            audit_settings_diagnostic(&canonical, format!("using default: {error}"));
            fallback
        }
    }
}

fn migrate_legacy_settings(canonical: &Path, legacy: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = canonical
        .parent()
        .ok_or_else(|| io::Error::other("canonical settings path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let counter = AUDIT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{AUDIT_SETTINGS_FILENAME}.migration.{}.{}",
        std::process::id(),
        counter
    ));
    let result = (|| {
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, canonical)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return result;
    }
    if let Err(error) = std::fs::remove_file(legacy) {
        audit_settings_diagnostic(
            legacy,
            format!("canonicalized but could not remove source: {error}"),
        );
    }
    Ok(())
}

fn write_audit_settings(
    root: &Path,
    capacity: usize,
    faults: &AuditPersistenceFaults,
) -> io::Result<()> {
    let directory = root.join("webinfo");
    std::fs::create_dir_all(&directory)?;
    let bytes =
        serde_json::to_vec_pretty(&SavedAuditSettings { capacity }).map_err(io::Error::other)?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    let counter = AUDIT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".{AUDIT_SETTINGS_FILENAME}.tmp.{}.{}",
        std::process::id(),
        counter
    ));
    let result = (|| {
        if faults.take_temp_write_failure() {
            return Err(io::Error::other(
                "injected audit settings temp-write failure",
            ));
        }
        std::fs::write(&temporary, &bytes)?;
        if faults.take_final_replace_failure() {
            return Err(io::Error::other(
                "injected audit settings final-replace failure",
            ));
        }
        std::fs::rename(&temporary, directory.join(AUDIT_SETTINGS_FILENAME))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

async fn persist_and_publish_audit_capacity(
    root: Option<PathBuf>,
    lock: Arc<AsyncMutex<()>>,
    observer: &QueryObserver,
    faults: AuditPersistenceFaults,
    capacity: usize,
) -> Result<(), &'static str> {
    let Some(root) = root else {
        return Err("audit settings state root is unavailable");
    };
    let _guard = lock.lock().await;
    tokio::task::spawn_blocking(move || write_audit_settings(&root, capacity, &faults))
        .await
        .map_err(|_| "audit settings persistence failed")?
        .map_err(|_| "audit settings persistence failed")
        .map(|()| observer.set_audit_capacity(capacity))
}

/// The largest request head accepted before the connection is rejected.
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// The largest request body accepted before the connection is rejected.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The host-owned management listener.
pub struct ApiServer {
    listener: TcpListener,
    config: Rc<CompiledConfig>,
    observer: Arc<QueryObserver>,
    state_root: Option<PathBuf>,
    audit_persist_lock: Arc<AsyncMutex<()>>,
    audit_persistence_faults: AuditPersistenceFaults,
    audit_clock: Arc<dyn AuditClock>,
    audit_read_slots: Arc<Semaphore>,
    accept_fault_after: Cell<Option<usize>>,
}

impl ApiServer {
    /// Binds the configured management address. Binding is separate from
    /// serving so the supervisor can bind every listener before any of them
    /// starts accepting.
    pub(crate) async fn bind(
        config: Rc<CompiledConfig>,
        observer: Arc<QueryObserver>,
        state_root: Option<PathBuf>,
        audit_persistence_faults: AuditPersistenceFaults,
        audit_clock: Arc<dyn AuditClock>,
        address: SocketAddr,
    ) -> Result<Self, ApiServerError> {
        let listener = TcpListener::bind(address)
            .await
            .map_err(ApiServerError::Bind)?;
        Ok(Self {
            listener,
            config,
            observer,
            state_root,
            audit_persist_lock: Arc::new(AsyncMutex::new(())),
            audit_persistence_faults,
            audit_clock,
            audit_read_slots: Arc::new(Semaphore::new(2)),
            accept_fault_after: Cell::new(None),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, ApiServerError> {
        self.listener
            .local_addr()
            .map_err(ApiServerError::LocalAddr)
    }

    /// Arms one narrow running-side failure. Only tests call this.
    #[doc(hidden)]
    pub fn inject_accept_fault_after(&self, connections: usize) {
        self.accept_fault_after.set(Some(connections));
    }

    /// Accepts connections until the shared scope is cancelled or the listener
    /// fails, then joins every request task before returning.
    pub async fn serve(self, shutdown: TransportCancellation) -> Result<(), ApiServerError> {
        let mut tasks = JoinSet::new();
        let mut task_error = None;
        let mut accept_error = None;
        let mut accepted = 0_usize;

        loop {
            let has_tasks = !tasks.is_empty();
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                joined = reap_one_task(&mut tasks, &mut task_error), if has_tasks => {
                    if joined && task_error.is_some() {
                        shutdown.cancel();
                    }
                }
                incoming = self.listener.accept() => {
                    match incoming {
                        Ok((stream, _peer)) => {
                            accepted += 1;
                            if self
                                .accept_fault_after
                                .get()
                                .is_some_and(|after| accepted > after)
                            {
                                accept_error = Some(io::Error::other(
                                    "injected management accept failure",
                                ));
                                shutdown.cancel();
                                break;
                            }
                            let config = Rc::clone(&self.config);
                            let observer = Arc::clone(&self.observer);
                            let state_root = self.state_root.clone();
                            let audit_persist_lock = Arc::clone(&self.audit_persist_lock);
                            let audit_persistence_faults = self.audit_persistence_faults.clone();
                            let audit_clock = Arc::clone(&self.audit_clock);
                            let audit_read_slots = Arc::clone(&self.audit_read_slots);
                            let connection_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                let _ = process_connection(
                                    stream,
                                    config,
                                    observer,
                                    state_root,
                                    audit_persist_lock,
                                    audit_persistence_faults,
                                    audit_clock,
                                    audit_read_slots,
                                    connection_shutdown,
                                )
                                .await;
                            });
                        }
                        Err(error) => {
                            accept_error = Some(error);
                            shutdown.cancel();
                            break;
                        }
                    }
                }
            }
        }

        shutdown.cancel();
        drain_tasks(&mut tasks, &mut task_error).await;
        if let Some(error) = task_error {
            return Err(ApiServerError::Task(error));
        }
        if let Some(error) = accept_error {
            return Err(ApiServerError::Accept(error));
        }
        Ok(())
    }
}

/// Failures that can occur while owning the management listener.
#[derive(Debug)]
pub enum ApiServerError {
    Bind(io::Error),
    LocalAddr(io::Error),
    Accept(io::Error),
    Task(String),
}

impl fmt::Display for ApiServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(error) => write!(formatter, "cannot bind the management listener: {error}"),
            Self::LocalAddr(error) => write!(formatter, "management listener address: {error}"),
            Self::Accept(error) => write!(formatter, "management accept failed: {error}"),
            Self::Task(error) => write!(formatter, "management request task failed: {error}"),
        }
    }
}

impl std::error::Error for ApiServerError {}

/// One parsed request. The body is already bounded and read.
struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    content_type: Option<&'static str>,
    body: Vec<u8>,
}

struct FallibleJsonWriter {
    body: Vec<u8>,
}

impl io::Write for FallibleJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.body
            .try_reserve(bytes.len())
            .map_err(|_| io::Error::other("JSON response allocation failed"))?;
        self.body.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Response {
    fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
        }
    }

    /// Mirrors Go's `http.Error`: one line, plain text, trailing newline.
    fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            content_type: Some("text/plain; charset=utf-8"),
            body: format!("{message}\n").into_bytes(),
        }
    }

    fn text(body: String) -> Self {
        Self {
            status: 200,
            content_type: Some("text/plain; charset=utf-8"),
            body: body.into_bytes(),
        }
    }

    fn success(body: &str) -> Self {
        Self {
            status: 200,
            content_type: None,
            body: body.as_bytes().to_vec(),
        }
    }

    fn json<T: Serialize>(value: &T) -> Self {
        let mut body = serde_json::to_vec(value).expect("small API response serializes");
        body.push(b'\n');
        Self {
            status: 200,
            content_type: Some("application/json"),
            body,
        }
    }

    fn json_compact<T: Serialize>(value: &T) -> Self {
        Self {
            status: 200,
            content_type: Some("application/json"),
            body: serde_json::to_vec(value).expect("small API response serializes"),
        }
    }

    fn try_json_compact<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        let mut writer = FallibleJsonWriter { body: Vec::new() };
        serde_json::to_writer(&mut writer, value)?;
        Ok(Self {
            status: 200,
            content_type: Some("application/json"),
            body: writer.body,
        })
    }

    fn method_not_allowed() -> Self {
        Self::error(405, "method not allowed")
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_connection(
    mut stream: TcpStream,
    config: Rc<CompiledConfig>,
    observer: Arc<QueryObserver>,
    state_root: Option<PathBuf>,
    audit_persist_lock: Arc<AsyncMutex<()>>,
    audit_persistence_faults: AuditPersistenceFaults,
    audit_clock: Arc<dyn AuditClock>,
    audit_read_slots: Arc<Semaphore>,
    shutdown: TransportCancellation,
) -> io::Result<()> {
    let request = tokio::select! {
        biased;
        () = shutdown.cancelled() => return Ok(()),
        request = read_request(&mut stream) => match request {
            Ok(Some(request)) => request,
            // The client closed before sending a complete request.
            Ok(None) => return Ok(()),
            Err(_) => {
                let response = Response::error(400, "bad request");
                let (_reader, mut writer) = stream.into_split();
                return write_response(&mut writer, &response).await;
            }
        },
    };
    let (_reader, mut writer) = stream.into_split();
    let request_shutdown = shutdown.child_token();
    let dispatch = dispatch(
        &config,
        &observer,
        state_root.as_deref(),
        &audit_persist_lock,
        &audit_persistence_faults,
        &audit_clock,
        &audit_read_slots,
        &request_shutdown,
        &request,
    );
    tokio::pin!(dispatch);
    let disconnect = watch_client_disconnect(_reader, request_shutdown.clone());
    tokio::pin!(disconnect);
    let response = tokio::select! {
        biased;
        () = shutdown.cancelled() => {
            request_shutdown.cancel();
            dispatch.await
        }
        () = &mut disconnect => {
            // Keep awaiting the worker after the peer closes. The worker owns
            // the expensive-read permit until it observes cancellation and
            // exits, so a disconnect cannot create an unbounded admission path.
            request_shutdown.cancel();
            dispatch.await
        }
        response = &mut dispatch => response,
    };
    if request_shutdown.is_cancelled() {
        return Ok(());
    }
    tokio::select! {
        biased;
        () = shutdown.cancelled() => Ok(()),
        written = write_response(&mut writer, &response) => written,
    }
}

/// Watches the request-side half after the complete request has been read.
/// EOF or a socket error is the client-disconnect signal for the in-flight
/// dispatch; the caller still awaits that dispatch before dropping its permit.
async fn watch_client_disconnect(mut stream: OwnedReadHalf, cancellation: TransportCancellation) {
    let mut buffer = [0_u8; 1024];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => {
                cancellation.cancel();
                return;
            }
            Ok(_) => {}
        }
    }
}

/// Reads one complete request head and its declared body. Returns `None` when
/// the peer closed first, and an error when the request is unusable.
async fn read_request(stream: &mut TcpStream) -> io::Result<Option<Request>> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let head_end = loop {
        if let Some(position) = head_end(&buffer) {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(io::Error::other("request head too large"));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let (method, target, content_length) = {
        let head = std::str::from_utf8(&buffer[..head_end])
            .map_err(|_| io::Error::other("request head is not UTF-8"))?;
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split(' ');
        let method = parts
            .next()
            .filter(|part| !part.is_empty())
            .ok_or_else(|| io::Error::other("missing request method"))?;
        let target = parts
            .next()
            .filter(|part| !part.is_empty())
            .ok_or_else(|| io::Error::other("missing request target"))?;

        let mut content_length = 0_usize;
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case("content-length") {
                    content_length = value
                        .trim()
                        .parse()
                        .map_err(|_| io::Error::other("invalid content length"))?;
                }
            }
        }
        (method.to_owned(), target.to_owned(), content_length)
    };
    if content_length > MAX_BODY_BYTES {
        return Err(io::Error::other("request body too large"));
    }

    let body_start = head_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer
        .get(body_start..body_start + content_length)
        .unwrap_or_default()
        .to_vec();
    Ok(Some(Request {
        method,
        target,
        body,
    }))
}

fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_response<W: AsyncWrite + Unpin>(
    stream: &mut W,
    response: &Response,
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason_phrase(response.status)
    );
    if let Some(content_type) = response.content_type {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response.body.len()
    ));
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.flush().await
}

const fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

/// Which configured operation one request addresses. The HTTP method is
/// resolved by the dispatcher, because a tag that is not mounted has no route
/// at all and must fail as 404 before any method decision.
enum Route<'a> {
    /// A registered plugin route shape: `/plugins/{tag}/{show|save|post}`.
    Plugin {
        tag: &'a str,
        action: &'a str,
    },
    Audit(AuditRoute),
    AuditV2(AuditV2Route),
    SpecialGroups,
    Unknown,
}

#[derive(Clone, Copy)]
enum AuditRoute {
    Status,
    Start,
    Stop,
    Clear,
    Capacity,
}

#[derive(Clone, Copy)]
enum AuditV2Route {
    Stats,
    Windows,
    Logs,
    LogsDomain,
    RankDomain,
    RankClient,
    RankDomainSet,
    RankEffective,
    RankSlowest,
}

fn route(target: &str) -> Route<'_> {
    // Go's `/show` ignores the query string; the UI sends `?limit=10000`.
    let path = target.split_once('?').map_or(target, |(path, _query)| path);
    if path == "/api/v1/special-groups" {
        return Route::SpecialGroups;
    }
    if let Some(suffix) = path.strip_prefix("/api/v2/audit/") {
        return match suffix {
            "stats" => Route::AuditV2(AuditV2Route::Stats),
            "stats/windows" => Route::AuditV2(AuditV2Route::Windows),
            "logs" => Route::AuditV2(AuditV2Route::Logs),
            "logs/domain" => Route::AuditV2(AuditV2Route::LogsDomain),
            "rank/domain" => Route::AuditV2(AuditV2Route::RankDomain),
            "rank/client" => Route::AuditV2(AuditV2Route::RankClient),
            "rank/domain_set" => Route::AuditV2(AuditV2Route::RankDomainSet),
            "rank/effective" => Route::AuditV2(AuditV2Route::RankEffective),
            "rank/slowest" => Route::AuditV2(AuditV2Route::RankSlowest),
            _ => Route::Unknown,
        };
    }
    if let Some(suffix) = path.strip_prefix("/api/v1/audit/") {
        return match suffix {
            "status" => Route::Audit(AuditRoute::Status),
            "start" => Route::Audit(AuditRoute::Start),
            "stop" => Route::Audit(AuditRoute::Stop),
            "clear" => Route::Audit(AuditRoute::Clear),
            "capacity" => Route::Audit(AuditRoute::Capacity),
            _ => Route::Unknown,
        };
    }
    let Some(rest) = path.strip_prefix("/plugins/") else {
        return Route::Unknown;
    };
    let Some((tag, action)) = rest.split_once('/') else {
        return Route::Unknown;
    };
    if tag.is_empty() {
        return Route::Unknown;
    }
    match action {
        "show" | "save" | "post" => Route::Plugin { tag, action },
        _ => Route::Unknown,
    }
}

#[derive(Serialize)]
struct AuditStatusResponse {
    capturing: bool,
}

#[derive(Serialize)]
struct AuditCapacityResponse {
    capacity: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditCapacityRequest {
    capacity: i64,
}

#[derive(Serialize)]
struct AuditStatsResponse {
    total_queries: usize,
    average_duration_ms: f64,
}

#[derive(Serialize)]
struct AuditWindowResponse {
    key: &'static str,
    label: &'static str,
    window_seconds: u64,
    request_count: usize,
    average_duration_ms: f64,
    complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_start: Option<String>,
}

#[derive(Serialize)]
struct AuditWindowsResponse {
    generated_at: String,
    items: Vec<AuditWindowResponse>,
}

#[derive(Serialize)]
struct AuditLogResponse {
    query_time: String,
    query_name: String,
    query_type: String,
    query_class: String,
    client_ip: String,
    trace_id: String,
    duration_ms: f64,
    response_code: String,
    response_flags: AuditResponseFlags,
    answers: Vec<AuditAnswerResponse>,
    answer_details_status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer_decode_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_targets: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_diagnostics: Option<AuditUpstreamDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_rule_source: Option<String>,
}

#[derive(Serialize)]
struct AuditUpstreamDiagnostics {
    schema_version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected: Option<AuditUpstreamSelected>,
    attempts: Vec<AuditUpstreamAttempt>,
}

#[derive(Serialize)]
struct AuditUpstreamSelected {
    entry: String,
    peer: String,
    transport: &'static str,
}

#[derive(Serialize)]
struct AuditUpstreamAttempt {
    ordinal: usize,
    entry: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<&'static str>,
    outcome: &'static str,
}

#[derive(Serialize)]
struct AuditResponseFlags {
    #[serde(rename = "AA")]
    aa: bool,
    #[serde(rename = "TC")]
    tc: bool,
    #[serde(rename = "RA")]
    ra: bool,
}

#[derive(Serialize)]
struct AuditAnswerResponse {
    #[serde(rename = "type")]
    rrtype: String,
    ttl: u32,
    data: String,
}

#[derive(Serialize)]
struct AuditLogPagination {
    total_items: usize,
    total_pages: usize,
    current_page: usize,
    items_per_page: usize,
}

#[derive(Serialize)]
struct AuditLogsResponse {
    pagination: AuditLogPagination,
    logs: Vec<AuditLogResponse>,
}

#[derive(Serialize)]
struct AuditRankItem {
    key: String,
    count: usize,
}

fn format_rfc3339(timestamp: SystemTime, seconds_only: bool) -> String {
    let timestamp = if seconds_only {
        timestamp
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| UNIX_EPOCH.checked_add(Duration::from_secs(duration.as_secs())))
            .unwrap_or(UNIX_EPOCH)
    } else {
        timestamp
    };
    let value = time::OffsetDateTime::from(timestamp).to_offset(time::UtcOffset::UTC);
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn query_type_name(qtype: u16) -> &'static str {
    match qtype {
        1 => "A",
        2 => "NS",
        3 => "MD",
        4 => "MF",
        5 => "CNAME",
        6 => "SOA",
        7 => "MB",
        8 => "MG",
        9 => "MR",
        10 => "NULL",
        11 => "WKS",
        12 => "PTR",
        13 => "HINFO",
        14 => "MINFO",
        15 => "MX",
        16 => "TXT",
        17 => "RP",
        18 => "AFSDB",
        19 => "X25",
        20 => "ISDN",
        21 => "RT",
        22 => "NSAP",
        23 => "NSAP-PTR",
        24 => "SIG",
        25 => "KEY",
        26 => "PX",
        27 => "GPOS",
        28 => "AAAA",
        29 => "LOC",
        30 => "NXT",
        31 => "EID",
        32 => "NIMLOC",
        33 => "SRV",
        34 => "ATMA",
        35 => "NAPTR",
        36 => "KX",
        37 => "CERT",
        38 => "A6",
        39 => "DNAME",
        40 => "SINK",
        41 => "OPT",
        42 => "APL",
        43 => "DS",
        44 => "SSHFP",
        45 => "IPSECKEY",
        46 => "RRSIG",
        47 => "NSEC",
        48 => "DNSKEY",
        49 => "DHCID",
        50 => "NSEC3",
        51 => "NSEC3PARAM",
        52 => "TLSA",
        53 => "SMIMEA",
        55 => "HIP",
        56 => "NINFO",
        57 => "RKEY",
        58 => "TALINK",
        59 => "CDS",
        60 => "CDNSKEY",
        61 => "OPENPGPKEY",
        62 => "CSYNC",
        63 => "ZONEMD",
        64 => "SVCB",
        65 => "HTTPS",
        99 => "SPF",
        100 => "UINFO",
        101 => "UID",
        102 => "GID",
        103 => "UNSPEC",
        104 => "NID",
        105 => "L32",
        106 => "L64",
        107 => "LP",
        108 => "EUI48",
        109 => "EUI64",
        249 => "TKEY",
        250 => "TSIG",
        251 => "IXFR",
        252 => "AXFR",
        253 => "MAILB",
        254 => "MAILA",
        255 => "ANY",
        256 => "URI",
        257 => "CAA",
        258 => "AVC",
        259 => "DOA",
        260 => "AMTRELAY",
        32768 => "TA",
        32769 => "DLV",
        _ => "",
    }
}

fn query_class_name(qclass: u16) -> &'static str {
    match qclass {
        1 => "IN",
        3 => "CH",
        4 => "HS",
        _ => "",
    }
}

fn response_code_name(response: &ResponseState, code: u16) -> String {
    if matches!(response, ResponseState::NoResponse) {
        return "NO_RESPONSE".to_owned();
    }
    match code {
        0 => "NOERROR",
        1 => "FORMERR",
        2 => "SERVFAIL",
        3 => "NXDOMAIN",
        4 => "NOTIMP",
        5 => "REFUSED",
        6 => "YXDOMAIN",
        7 => "YXRRSET",
        8 => "NXRRSET",
        9 => "NOTAUTH",
        10 => "NOTZONE",
        16 => "BADVERS",
        17 => "BADKEY",
        18 => "BADTIME",
        19 => "BADMODE",
        20 => "BADNAME",
        21 => "BADALG",
        22 => "BADTRUNC",
        23 => "BADCOOKIE",
        _ => return String::new(),
    }
    .to_owned()
}

fn answer_type_name(rrtype: u16) -> String {
    let name = query_type_name(rrtype);
    if name.is_empty() {
        format!("TYPE{rrtype}")
    } else {
        name.to_owned()
    }
}

fn answer_details_status_name(status: AnswerDetailsStatus) -> &'static str {
    match status {
        AnswerDetailsStatus::Complete => "complete",
        AnswerDetailsStatus::RawRdata => "raw_rdata",
        AnswerDetailsStatus::DecodeError => "decode_error",
    }
}

fn project_log(record: &AuditRecord) -> AuditLogResponse {
    let query_name = if record.qname == "." {
        ".".to_owned()
    } else {
        record
            .qname
            .strip_suffix('.')
            .unwrap_or(&record.qname)
            .to_owned()
    };
    AuditLogResponse {
        query_time: format_rfc3339(record.timestamp, false),
        query_name,
        query_type: query_type_name(record.qtype).to_owned(),
        query_class: query_class_name(record.qclass).to_owned(),
        client_ip: record.client_addr.ip().to_string(),
        trace_id: record.trace_id.clone(),
        duration_ms: record.elapsed.as_secs_f64() * 1_000.0,
        response_code: response_code_name(&record.response, record.response_details.rcode),
        response_flags: AuditResponseFlags {
            aa: record.response_details.flags.aa,
            tc: record.response_details.flags.tc,
            ra: record.response_details.flags.ra,
        },
        answers: record
            .response_details
            .answers
            .iter()
            .map(|answer| AuditAnswerResponse {
                rrtype: answer_type_name(answer.rrtype),
                ttl: answer.ttl,
                data: answer.data.clone(),
            })
            .collect(),
        answer_details_status: answer_details_status_name(
            record.response_details.answer_details_status,
        ),
        answer_decode_error: record.response_details.answer_decode_error.clone(),
        domain_set: record.domain_set.clone(),
        effective_tag: record.effective_tag.clone(),
        matched_group: record.matched_group.clone(),
        final_sequence: record.final_sequence.clone(),
        final_upstream: record.final_upstream.clone(),
        upstream_targets: record.upstream_targets.clone(),
        selected_upstream: record.selected_upstream.clone(),
        upstream_diagnostics: record
            .upstream_diagnostics
            .as_ref()
            .map(project_upstream_diagnostics),
        matched_rule_source: record.matched_rule_source.clone(),
    }
}

fn project_upstream_diagnostics(diagnostics: &UpstreamDiagnostics) -> AuditUpstreamDiagnostics {
    AuditUpstreamDiagnostics {
        schema_version: diagnostics.schema_version,
        selected: diagnostics
            .selected
            .as_ref()
            .map(|selected| AuditUpstreamSelected {
                entry: selected.entry.clone(),
                peer: selected.peer.to_string(),
                transport: selected.transport.as_str(),
            }),
        attempts: diagnostics
            .attempts
            .iter()
            .map(|attempt| AuditUpstreamAttempt {
                ordinal: attempt.ordinal,
                entry: attempt.entry.clone(),
                peer: attempt.peer.map(|peer| peer.to_string()),
                transport: attempt.transport.map(|transport| transport.as_str()),
                outcome: match attempt.outcome {
                    UpstreamAttemptOutcome::Response => "response",
                    UpstreamAttemptOutcome::TimedOut => "timed_out",
                    UpstreamAttemptOutcome::Failed => "failed",
                    UpstreamAttemptOutcome::Canceled => "canceled",
                    UpstreamAttemptOutcome::Interrupted => "interrupted",
                },
            })
            .collect(),
    }
}

fn audit_stats(snapshot: &AuditStatsSnapshot) -> AuditStatsResponse {
    let total_queries = snapshot.total_queries;
    let average_duration_ms = if total_queries == 0 {
        0.0
    } else {
        snapshot.elapsed_micros as f64 / 1_000.0 / total_queries as f64
    };
    AuditStatsResponse {
        total_queries,
        average_duration_ms,
    }
}

fn audit_windows(records: &[AuditTimingSnapshot], now: SystemTime) -> AuditWindowsResponse {
    const WINDOWS: [(&str, &str, u64); 5] = [
        ("1h", "1小时内", 3_600),
        ("6h", "最近6小时", 21_600),
        ("24h", "24小时内", 86_400),
        ("3d", "最近3天", 259_200),
        ("7d", "最近7天", 604_800),
    ];
    let oldest = records.iter().map(|record| record.timestamp).fold(
        None,
        |oldest: Option<SystemTime>, timestamp| match oldest {
            None => Some(timestamp),
            Some(current) if timestamp.duration_since(current).is_err() => Some(timestamp),
            Some(current) => Some(current),
        },
    );
    let coverage_start = oldest.map(|timestamp| format_rfc3339(timestamp, true));
    let items = WINDOWS
        .into_iter()
        .map(|(key, label, window_seconds)| {
            let cutoff = now
                .checked_sub(Duration::from_secs(window_seconds))
                .unwrap_or(UNIX_EPOCH);
            let (request_count, duration_total) = records
                .iter()
                .filter(|record| {
                    record.timestamp.duration_since(cutoff).is_ok()
                        && now.duration_since(record.timestamp).is_ok()
                })
                .fold((0_usize, 0.0_f64), |(count, total), record| {
                    (
                        count.saturating_add(1),
                        total + record.elapsed.as_secs_f64() * 1_000.0,
                    )
                });
            let average_duration_ms = if request_count == 0 {
                0.0
            } else {
                duration_total / request_count as f64
            };
            AuditWindowResponse {
                key,
                label,
                window_seconds,
                request_count,
                average_duration_ms,
                complete: oldest.is_some_and(|timestamp| cutoff.duration_since(timestamp).is_ok()),
                coverage_start: coverage_start.clone(),
            }
        })
        .collect();
    AuditWindowsResponse {
        generated_at: format_rfc3339(now, true),
        items,
    }
}

#[derive(Clone, Debug, Default)]
struct AuditFilter {
    q: Option<String>,
    exact: bool,
    domain: Option<String>,
    client_ips: Vec<String>,
    answer_ip: Option<String>,
    cname: Option<String>,
    domain_set: Option<String>,
    effective_tag: Option<String>,
    exact_domain: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct AuditPage {
    page: usize,
    limit: usize,
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_query_component(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or(())?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or(())?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn query_pairs(target: &str) -> Result<Vec<(String, String)>, String> {
    let Some((_, query)) = target.split_once('?') else {
        return Ok(Vec::new());
    };
    if query.contains(';') {
        return Err("invalid audit query encoding".to_owned());
    }
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((
                decode_query_component(key).map_err(|_| "invalid audit query encoding")?,
                decode_query_component(value).map_err(|_| "invalid audit query encoding")?,
            ))
        })
        .collect()
}

fn first_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
}

fn positive_or_default(value: Option<&String>, default: usize) -> usize {
    value
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

fn parse_audit_filter(
    target: &str,
    domain_route: bool,
) -> Result<(AuditFilter, AuditPage), String> {
    let pairs = query_pairs(target).map_err(|message| message.to_owned())?;
    let allowed = [
        "page",
        "limit",
        "q",
        "exact",
        "domain",
        "client_ip",
        "answer_ip",
        "cname",
        "domain_set",
        "effective_tag",
    ];
    for (key, _) in &pairs {
        if domain_route && !["domain", "page", "limit"].contains(&key.as_str()) {
            return Err("unsupported audit query parameter".to_owned());
        }
        if !allowed.contains(&key.as_str()) {
            return Err("unsupported audit query parameter".to_owned());
        }
    }
    let limit_value = first_value(&pairs, "limit");
    let parsed_limit = positive_or_default(limit_value.as_ref(), 50);
    if limit_value
        .as_ref()
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|value| value > 500)
    {
        return Err("audit log limit must be between 1 and 500".to_owned());
    }
    let mut filter = AuditFilter {
        q: first_value(&pairs, "q").filter(|value| !value.is_empty()),
        exact: first_value(&pairs, "exact")
            .as_deref()
            .is_some_and(parse_bool_like),
        domain: first_value(&pairs, "domain").filter(|value| !value.is_empty()),
        client_ips: pairs
            .iter()
            .filter(|(key, _)| key == "client_ip")
            .map(|(_, value)| value.clone())
            .collect(),
        answer_ip: first_value(&pairs, "answer_ip").filter(|value| !value.is_empty()),
        cname: first_value(&pairs, "cname").filter(|value| !value.is_empty()),
        domain_set: first_value(&pairs, "domain_set").filter(|value| !value.is_empty()),
        effective_tag: first_value(&pairs, "effective_tag").filter(|value| !value.is_empty()),
        exact_domain: None,
    };
    if domain_route {
        let domain = first_value(&pairs, "domain").filter(|value| !value.is_empty());
        if domain.is_none() {
            return Err("exact domain is required".to_owned());
        }
        filter.exact_domain = domain;
        filter.domain = None;
    }
    Ok((
        filter,
        AuditPage {
            page: positive_or_default(first_value(&pairs, "page").as_ref(), 1),
            limit: parsed_limit,
        },
    ))
}

fn parse_bool_like(value: &str) -> bool {
    matches!(value, "1" | "t" | "T" | "TRUE" | "true" | "True")
}

fn normalized_ip(value: &str) -> Option<String> {
    let address = if let Ok(address) = value.parse::<SocketAddr>() {
        address.ip()
    } else if let Ok(address) = value.parse::<std::net::IpAddr>() {
        address
    } else {
        value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<std::net::IpAddr>().ok())?
    };
    let address = match address {
        std::net::IpAddr::V4(address) => std::net::IpAddr::V4(address),
        std::net::IpAddr::V6(address) => canonical_ip(std::net::IpAddr::V6(address)),
    };
    Some(address.to_string())
}

fn canonical_ip(address: std::net::IpAddr) -> std::net::IpAddr {
    match address {
        std::net::IpAddr::V4(address) => std::net::IpAddr::V4(address),
        std::net::IpAddr::V6(address) => {
            let segments = address.segments();
            if segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
                let octets = address.octets();
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(
                    octets[12], octets[13], octets[14], octets[15],
                ))
            } else {
                std::net::IpAddr::V6(address)
            }
        }
    }
}

fn text_matches(value: &str, query: &str, exact: bool) -> bool {
    if exact {
        value == query
    } else {
        value.to_lowercase().contains(&query.to_lowercase())
    }
}

fn projected_query_name(record: &AuditRecord) -> String {
    if record.qname == "." {
        ".".to_owned()
    } else {
        record.qname.trim_end_matches('.').to_owned()
    }
}

fn audit_record_matches(record: &AuditRecord, filter: &AuditFilter) -> bool {
    let query_match = filter.q.as_ref().is_none_or(|query| {
        let exact = filter.exact;
        let ip_query = normalized_ip(query);
        let mut values = vec![
            projected_query_name(record),
            canonical_ip(record.client_addr.ip()).to_string(),
            record.trace_id.clone(),
        ];
        values.extend(record.domain_set.clone());
        values.extend(record.effective_tag.clone());
        values.extend(record.matched_rule_source.clone());
        values.extend(record.selected_upstream.clone());
        values.extend(
            record
                .response_details
                .answers
                .iter()
                .map(|answer| answer.data.clone()),
        );
        values.iter().any(|value| {
            let ip_matches = ip_query
                .as_deref()
                .is_some_and(|ip| normalized_ip(value).as_deref() == Some(ip));
            ip_matches || text_matches(value, query, exact)
        })
    });
    let domain_match = filter
        .domain
        .as_ref()
        .is_none_or(|domain| projected_query_name(record).contains(domain));
    let client_match = if filter.client_ips.is_empty() {
        true
    } else {
        let record_ip = canonical_ip(record.client_addr.ip()).to_string();
        filter
            .client_ips
            .iter()
            .any(|candidate| normalized_ip(candidate).as_deref() == Some(record_ip.as_str()))
    };
    let answer_ip_match = filter.answer_ip.as_ref().is_none_or(|ip| {
        normalized_ip(ip).is_some_and(|ip| {
            record.response_details.answers.iter().any(|answer| {
                matches!(answer.rrtype, 1 | 28)
                    && normalized_ip(&answer.data).as_deref() == Some(&ip)
            })
        })
    });
    let cname_match = filter.cname.as_ref().is_none_or(|cname| {
        record
            .response_details
            .answers
            .iter()
            .any(|answer| answer.rrtype == 5 && answer.data.contains(cname))
    });
    let domain_set_match = filter
        .domain_set
        .as_ref()
        .is_none_or(|value| record.domain_set.as_deref() == Some(value));
    let effective_match = filter
        .effective_tag
        .as_ref()
        .is_none_or(|value| record.effective_tag.as_deref() == Some(value));
    let exact_domain_match = filter
        .exact_domain
        .as_ref()
        .is_none_or(|value| projected_query_name(record) == value.as_str());
    query_match
        && domain_match
        && client_match
        && answer_ip_match
        && cname_match
        && domain_set_match
        && effective_match
        && exact_domain_match
}

fn audit_logs_from_records(
    records: Vec<std::sync::Arc<AuditRecord>>,
    page: usize,
    limit: usize,
) -> Result<AuditLogsResponse, ()> {
    let total_items = records.len();
    let total_pages = if total_items == 0 {
        0
    } else {
        (total_items - 1) / limit + 1
    };
    let start = page.saturating_sub(1).saturating_mul(limit);
    let mut logs = Vec::new();
    logs.try_reserve(limit.min(total_items)).map_err(|_| ())?;
    logs.extend(
        records
            .iter()
            .rev()
            .skip(start)
            .take(limit)
            .map(|record| project_log(record)),
    );
    Ok(AuditLogsResponse {
        pagination: AuditLogPagination {
            total_items,
            total_pages,
            current_page: page,
            items_per_page: limit,
        },
        logs,
    })
}

#[derive(Clone, Copy)]
enum AuditReadKind {
    Logs { domain_route: bool },
    RankDomain,
    RankClient,
    RankDomainSet,
    RankEffective,
    RankSlowest,
}

fn parse_rank_limit(target: &str, default: usize) -> Result<usize, String> {
    let pairs = query_pairs(target).map_err(|message| message.to_owned())?;
    for (key, _) in &pairs {
        if key != "limit" {
            return Err("unsupported audit query parameter".to_owned());
        }
    }
    let value = first_value(&pairs, "limit");
    if value
        .as_ref()
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|value| value > 500)
    {
        return Err("audit rank limit must be between 1 and 500".to_owned());
    }
    Ok(positive_or_default(value.as_ref(), default).min(500))
}

enum AuditReadFailure {
    Canceled,
    Allocation,
}

fn rank_items(
    records: &[std::sync::Arc<AuditRecord>],
    key: impl Fn(&AuditRecord) -> Option<String>,
    shutdown: &TransportCancellation,
) -> Result<Vec<AuditRankItem>, AuditReadFailure> {
    let mut keys = Vec::new();
    keys.try_reserve(records.len())
        .map_err(|_| AuditReadFailure::Allocation)?;
    for (index, record) in records.iter().enumerate() {
        if index % 1024 == 0 && shutdown.is_cancelled() {
            return Err(AuditReadFailure::Canceled);
        }
        if let Some(value) = key(record) {
            keys.push(value);
        }
    }
    keys.sort_unstable();
    let mut items = Vec::new();
    items
        .try_reserve(keys.len())
        .map_err(|_| AuditReadFailure::Allocation)?;
    let mut keys = keys.into_iter().peekable();
    while let Some(key) = keys.next() {
        let mut count = 1;
        while keys.peek().is_some_and(|next| next == &key) {
            let _ = keys.next();
            count += 1;
        }
        items.push(AuditRankItem { key, count });
    }
    items.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    if shutdown.is_cancelled() {
        Err(AuditReadFailure::Canceled)
    } else {
        Ok(items)
    }
}

fn take_rank_items<T>(items: Vec<T>, limit: usize) -> Result<Vec<T>, AuditReadFailure> {
    let mut selected = Vec::new();
    selected
        .try_reserve(limit.min(items.len()))
        .map_err(|_| AuditReadFailure::Allocation)?;
    selected.extend(items.into_iter().take(limit));
    Ok(selected)
}

fn rank_response(
    snapshot: AuditReadSnapshot,
    kind: AuditReadKind,
    limit: usize,
    filter: Option<AuditFilter>,
    shutdown: &TransportCancellation,
) -> Result<Response, AuditReadFailure> {
    let records = match filter {
        None => snapshot.records,
        Some(filter) => {
            let mut records = Vec::new();
            records
                .try_reserve(snapshot.records.len())
                .map_err(|_| AuditReadFailure::Allocation)?;
            for (index, record) in snapshot.records.into_iter().enumerate() {
                if index % 1024 == 0 && shutdown.is_cancelled() {
                    return Err(AuditReadFailure::Canceled);
                }
                if audit_record_matches(&record, &filter) {
                    records.push(record);
                }
            }
            records
        }
    };
    match kind {
        AuditReadKind::Logs { .. } => unreachable!("logs use their own projection"),
        AuditReadKind::RankDomain => Response::try_json_compact(&take_rank_items(
            rank_items(
                &records,
                |record| Some(projected_query_name(record)),
                shutdown,
            )?,
            limit,
        )?)
        .map_err(|_| AuditReadFailure::Allocation),
        AuditReadKind::RankClient => Response::try_json_compact(&take_rank_items(
            rank_items(
                &records,
                |record| Some(canonical_ip(record.client_addr.ip()).to_string()),
                shutdown,
            )?,
            limit,
        )?)
        .map_err(|_| AuditReadFailure::Allocation),
        AuditReadKind::RankDomainSet => Response::try_json_compact(&take_rank_items(
            rank_items(&records, |record| record.domain_set.clone(), shutdown)?,
            limit,
        )?)
        .map_err(|_| AuditReadFailure::Allocation),
        AuditReadKind::RankEffective => Response::try_json_compact(&take_rank_items(
            rank_items(&records, |record| record.effective_tag.clone(), shutdown)?,
            limit,
        )?)
        .map_err(|_| AuditReadFailure::Allocation),
        AuditReadKind::RankSlowest => {
            if shutdown.is_cancelled() {
                return Err(AuditReadFailure::Canceled);
            }
            let mut records = Vec::new();
            records
                .try_reserve(limit.min(snapshot.slowest.len()))
                .map_err(|_| AuditReadFailure::Allocation)?;
            records.extend(
                snapshot
                    .slowest
                    .iter()
                    .take(limit)
                    .map(|record| project_log(record)),
            );
            Response::try_json_compact(&records).map_err(|_| AuditReadFailure::Allocation)
        }
    }
}

async fn dispatch_audit_read(
    observer: Arc<QueryObserver>,
    slots: &Arc<Semaphore>,
    target: &str,
    kind: AuditReadKind,
    shutdown: TransportCancellation,
) -> Response {
    let (filter, page, limit) = match kind {
        AuditReadKind::Logs { domain_route } => match parse_audit_filter(target, domain_route) {
            Ok((filter, page)) => (Some(filter), page.page, page.limit),
            Err(message) => return Response::error(400, &message),
        },
        AuditReadKind::RankSlowest
        | AuditReadKind::RankDomain
        | AuditReadKind::RankClient
        | AuditReadKind::RankDomainSet
        | AuditReadKind::RankEffective => match parse_rank_limit(
            target,
            if matches!(kind, AuditReadKind::RankSlowest) {
                100
            } else {
                20
            },
        ) {
            Ok(limit) => (None, 1, limit),
            Err(message) => return Response::error(400, &message),
        },
    };
    let Ok(permit) = Arc::clone(slots).try_acquire_owned() else {
        return Response::error(503, "audit read capacity exhausted");
    };
    tokio::task::spawn_blocking(move || {
        let _permit: OwnedSemaphorePermit = permit;
        let snapshot = match observer.audit_read_snapshot() {
            Ok(snapshot) => snapshot,
            Err(()) => return Response::error(500, "audit read failed"),
        };
        if let AuditReadKind::Logs { .. } = kind {
            let mut records = Vec::new();
            if records.try_reserve(snapshot.records.len()).is_err() {
                return Response::error(500, "audit read failed");
            }
            for (index, record) in snapshot.records.into_iter().enumerate() {
                if index % 1024 == 0 && shutdown.is_cancelled() {
                    return Response::empty(499);
                }
                if filter
                    .as_ref()
                    .is_none_or(|filter| audit_record_matches(&record, filter))
                {
                    records.push(record);
                }
            }
            if shutdown.is_cancelled() {
                return Response::empty(499);
            }
            match audit_logs_from_records(records, page, limit)
                .ok()
                .and_then(|logs| Response::try_json_compact(&logs).ok())
            {
                Some(response) => response,
                None => Response::error(500, "audit read failed"),
            }
        } else {
            match rank_response(snapshot, kind, limit, filter, &shutdown) {
                Ok(response) => response,
                Err(AuditReadFailure::Canceled) => Response::empty(499),
                Err(AuditReadFailure::Allocation) => Response::error(500, "audit read failed"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| Response::error(500, "audit read failed"))
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    config: &CompiledConfig,
    observer: &Arc<QueryObserver>,
    state_root: Option<&Path>,
    audit_persist_lock: &Arc<AsyncMutex<()>>,
    audit_persistence_faults: &AuditPersistenceFaults,
    audit_clock: &Arc<dyn AuditClock>,
    audit_read_slots: &Arc<Semaphore>,
    shutdown: &TransportCancellation,
    request: &Request,
) -> Response {
    match route(&request.target) {
        Route::AuditV2(AuditV2Route::Stats) => match request.method.as_str() {
            "GET" => {
                let snapshot = observer.audit_stats_snapshot();
                Response::json_compact(&audit_stats(&snapshot))
            }
            _ => Response::method_not_allowed(),
        },
        Route::AuditV2(AuditV2Route::Windows) => match request.method.as_str() {
            "GET" => {
                let snapshot = observer.audit_timing_snapshot();
                Response::json_compact(&audit_windows(&snapshot, audit_clock.now()))
            }
            _ => Response::method_not_allowed(),
        },
        Route::AuditV2(AuditV2Route::Logs) => match request.method.as_str() {
            "GET" => {
                dispatch_audit_read(
                    Arc::clone(observer),
                    audit_read_slots,
                    &request.target,
                    AuditReadKind::Logs {
                        domain_route: false,
                    },
                    shutdown.clone(),
                )
                .await
            }
            _ => Response::method_not_allowed(),
        },
        Route::AuditV2(AuditV2Route::LogsDomain) => match request.method.as_str() {
            "GET" => {
                dispatch_audit_read(
                    Arc::clone(observer),
                    audit_read_slots,
                    &request.target,
                    AuditReadKind::Logs { domain_route: true },
                    shutdown.clone(),
                )
                .await
            }
            _ => Response::method_not_allowed(),
        },
        Route::AuditV2(AuditV2Route::RankDomain)
        | Route::AuditV2(AuditV2Route::RankClient)
        | Route::AuditV2(AuditV2Route::RankDomainSet)
        | Route::AuditV2(AuditV2Route::RankEffective)
        | Route::AuditV2(AuditV2Route::RankSlowest)
            if request.method.as_str() == "GET" =>
        {
            let kind = match route(&request.target) {
                Route::AuditV2(AuditV2Route::RankDomain) => AuditReadKind::RankDomain,
                Route::AuditV2(AuditV2Route::RankClient) => AuditReadKind::RankClient,
                Route::AuditV2(AuditV2Route::RankDomainSet) => AuditReadKind::RankDomainSet,
                Route::AuditV2(AuditV2Route::RankEffective) => AuditReadKind::RankEffective,
                Route::AuditV2(AuditV2Route::RankSlowest) => AuditReadKind::RankSlowest,
                _ => unreachable!("route arm is exhaustive"),
            };
            dispatch_audit_read(
                Arc::clone(observer),
                audit_read_slots,
                &request.target,
                kind,
                shutdown.clone(),
            )
            .await
        }
        Route::AuditV2(AuditV2Route::RankDomain)
        | Route::AuditV2(AuditV2Route::RankClient)
        | Route::AuditV2(AuditV2Route::RankDomainSet)
        | Route::AuditV2(AuditV2Route::RankEffective)
        | Route::AuditV2(AuditV2Route::RankSlowest) => Response::method_not_allowed(),
        Route::Audit(AuditRoute::Status) => match request.method.as_str() {
            "GET" => Response::json(&AuditStatusResponse {
                capturing: observer.is_capturing(),
            }),
            _ => Response::method_not_allowed(),
        },
        Route::Audit(AuditRoute::Start) => match request.method.as_str() {
            "POST" => {
                observer.start_capture();
                Response::success("Audit log collection started.")
            }
            _ => Response::method_not_allowed(),
        },
        Route::Audit(AuditRoute::Stop) => match request.method.as_str() {
            "POST" => {
                observer.stop_capture();
                Response::success("Audit log collection stopped.")
            }
            _ => Response::method_not_allowed(),
        },
        Route::Audit(AuditRoute::Clear) => match request.method.as_str() {
            "POST" => {
                observer.clear_audit();
                Response::success("In-memory audit logs cleared.")
            }
            _ => Response::method_not_allowed(),
        },
        Route::Audit(AuditRoute::Capacity) => match request.method.as_str() {
            "GET" => Response::json(&AuditCapacityResponse {
                capacity: observer.audit_capacity(),
            }),
            "POST" => {
                let payload: AuditCapacityRequest = match serde_json::from_slice::<
                    AuditCapacityRequest,
                >(&request.body)
                {
                    Ok(payload) if (0..=MAX_AUDIT_CAPACITY as i64).contains(&payload.capacity) => {
                        payload
                    }
                    _ => return Response::error(400, "invalid audit capacity request"),
                };
                let capacity =
                    usize::try_from(payload.capacity).expect("validated audit capacity fits usize");
                if let Err(message) = persist_and_publish_audit_capacity(
                    state_root.map(Path::to_path_buf),
                    Arc::clone(audit_persist_lock),
                    observer,
                    audit_persistence_faults.clone(),
                    capacity,
                )
                .await
                {
                    return Response::error(500, message);
                }
                Response::success(&format!(
                    "Audit log capacity set to {capacity}. Existing logs have been cleared."
                ))
            }
            _ => Response::method_not_allowed(),
        },
        // Go mounts handlers per existing plugin tag, so a tag that is not
        // mounted has no route: 404 before any method or eligibility decision.
        Route::Plugin { tag, .. } if config.domain_set(tag).is_none() => {
            Response::error(404, "404 page not found")
        }
        Route::Plugin { tag, action } => match (request.method.as_str(), action) {
            ("GET", "show") => match eligible(config, tag) {
                Err(response) => response,
                Ok(provider) => {
                    let mut body = String::new();
                    for rule in provider.rules() {
                        body.push_str(&rule);
                        body.push('\n');
                    }
                    Response::text(body)
                }
            },
            ("GET", "save") => match eligible(config, tag) {
                Err(response) => response,
                Ok(provider) => match provider.save().await {
                    Ok(()) => Response::empty(200),
                    Err(error) => Response::error(500, &error.to_string()),
                },
            },
            ("POST", "post") => match eligible(config, tag) {
                Err(response) => response,
                Ok(provider) => {
                    let payload: PostPayload = match serde_json::from_slice(&request.body) {
                        Ok(payload) => payload,
                        Err(_) => return Response::error(400, "invalid JSON"),
                    };
                    match provider.replace(&payload.values).await {
                        Ok(count) => {
                            Response::text(format!("domain_set replaced with {count} entries"))
                        }
                        Err(error) => Response::error(500, &error.to_string()),
                    }
                }
            },
            // Any other method on a registered, mounted path, as chi replies.
            _ => Response::empty(405),
        },
        // The strict native subset cannot configure dedicated routing groups, so
        // an empty list is the true state and no group mutation route exists.
        Route::SpecialGroups => match request.method.as_str() {
            "GET" => Response {
                status: 200,
                content_type: Some("application/json"),
                body: b"[]\n".to_vec(),
            },
            _ => Response::empty(405),
        },
        // Mirrors Go's `http.NotFound` for an unmatched route.
        Route::Unknown => Response::error(404, "404 page not found"),
    }
}

/// Looks up one management-eligible provider, or the explicit failure.
fn eligible<'a>(config: &'a CompiledConfig, tag: &str) -> Result<&'a ManagedDomainSet, Response> {
    let Some(set) = config.domain_set(tag) else {
        return Err(Response::error(404, "404 page not found"));
    };
    match set.managed.as_deref() {
        Some(provider) => Ok(provider),
        None => {
            let reason = set
                .ineligible_reason
                .clone()
                .unwrap_or_else(|| "not a single-file .txt managed profile".to_owned());
            Err(Response::error(
                400,
                &format!("domain_set `{tag}` is not manageable: {reason}"),
            ))
        }
    }
}

/// The UI payload: `{ "values": ["...", ...] }`.
#[derive(Deserialize)]
struct PostPayload {
    #[serde(default)]
    values: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use mosdns_upstream_core::TransportCancellation;
    use tokio::sync::Semaphore;

    use super::{
        AuditReadKind, QueryObserver, ResponseState, dispatch_audit_read, load_audit_capacity,
        migrate_legacy_settings, normalized_ip, parse_audit_filter, parse_rank_limit, query_pairs,
        query_type_name, response_code_name,
    };

    fn test_root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("mosdns-native-audit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test root");
        root
    }

    #[test]
    fn canonical_settings_win_before_legacy_and_clamp_saved_values() {
        let root = test_root("precedence");
        fs::create_dir_all(root.join("webinfo")).expect("webinfo");
        fs::create_dir_all(root.join("state")).expect("state");
        fs::write(root.join("webinfo/audit_settings.json"), b"not-json").expect("canonical");
        fs::write(root.join("state/audit_settings.json"), b"{\"capacity\":2}").expect("legacy");
        assert_eq!(load_audit_capacity(&root, 100_000), 100_000);

        fs::remove_file(root.join("webinfo/audit_settings.json")).expect("remove canonical");
        assert_eq!(load_audit_capacity(&root, 100_000), 2);
        assert!(root.join("webinfo/audit_settings.json").is_file());
        assert!(!root.join("state/audit_settings.json").exists());

        fs::write(
            root.join("webinfo/audit_settings.json"),
            b"{\"capacity\":999999}",
        )
        .expect("above-range canonical");
        assert_eq!(load_audit_capacity(&root, 100_000), 400_000);

        fs::remove_file(root.join("webinfo/audit_settings.json")).expect("remove canonical");
        fs::write(root.join("audit_settings.json"), b"{\"capacity\":3}")
            .expect("root-level legacy");
        assert_eq!(load_audit_capacity(&root, 100_000), 3);
        assert!(root.join("webinfo/audit_settings.json").is_file());
        assert!(!root.join("audit_settings.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn null_or_missing_capacity_is_zero_and_failed_migration_keeps_source() {
        let root = test_root("migration");
        fs::create_dir_all(root.join("webinfo")).expect("webinfo");
        fs::write(root.join("webinfo/audit_settings.json"), b"{}").expect("missing capacity");
        assert_eq!(load_audit_capacity(&root, 100_000), 0);
        fs::write(
            root.join("webinfo/audit_settings.json"),
            b"{\"capacity\":null}",
        )
        .expect("null capacity");
        assert_eq!(load_audit_capacity(&root, 100_000), 0);

        fs::remove_file(root.join("webinfo/audit_settings.json")).expect("remove canonical");
        fs::create_dir(root.join("webinfo/audit_settings.json")).expect("blocking target");
        fs::create_dir_all(root.join("state")).expect("state");
        let legacy = root.join("state/audit_settings.json");
        let bytes = b"{\"capacity\":7}";
        fs::write(&legacy, bytes).expect("legacy");
        migrate_legacy_settings(&root.join("webinfo/audit_settings.json"), &legacy, bytes)
            .expect_err("blocked migration");
        assert!(legacy.is_file());
        fs::remove_dir(root.join("webinfo/audit_settings.json")).expect("remove blocking target");
        assert_eq!(load_audit_capacity(&root, 100_000), 7);
        assert!(!legacy.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn known_dns_types_keep_mnemonics_and_unknown_types_are_empty() {
        assert_eq!(query_type_name(13), "HINFO");
        assert_eq!(query_type_name(44), "SSHFP");
        assert_eq!(query_type_name(65535), "");
    }

    #[test]
    fn audit_query_decoding_is_form_strict_and_rank_limits_are_bounded() {
        assert_eq!(
            query_pairs("/api/v2/audit/logs?q=one+two&domain=%E4%B8%AD").expect("query pairs"),
            vec![
                ("q".to_owned(), "one two".to_owned()),
                ("domain".to_owned(), "中".to_owned()),
            ]
        );
        assert_eq!(
            query_pairs("/api/v2/audit/logs?q=%ff").expect_err("invalid UTF-8"),
            "invalid audit query encoding"
        );
        assert_eq!(
            parse_rank_limit("/api/v2/audit/rank/domain?limit=501", 20)
                .expect_err("bounded rank limit"),
            "audit rank limit must be between 1 and 500"
        );
        assert_eq!(
            parse_audit_filter("/api/v2/audit/logs/domain", true)
                .expect_err("missing exact domain"),
            "exact domain is required"
        );
        assert_eq!(
            normalized_ip("127.0.0.1:5353"),
            Some("127.0.0.1".to_owned())
        );
        assert_eq!(normalized_ip("[::1]:5353"), Some("::1".to_owned()));
        assert_eq!(
            normalized_ip("::ffff:192.0.2.7"),
            Some("192.0.2.7".to_owned())
        );
        assert_eq!(
            response_code_name(
                &ResponseState::Dns {
                    rcode: 16,
                    source: crate::observer::ResponseSource::Local,
                },
                16
            ),
            "BADVERS"
        );
        let (_, page) = parse_audit_filter(
            "/api/v2/audit/logs?page=9223372036854775808&limit=9223372036854775808",
            false,
        )
        .expect("signed-64 overflow uses defaults");
        assert_eq!(page.page, 1);
        assert_eq!(page.limit, 50);
        assert_eq!(
            parse_rank_limit("/api/v2/audit/rank/domain?limit=9223372036854775808", 20,)
                .expect("rank signed-64 overflow uses default"),
            20
        );
    }

    #[test]
    fn canceled_audit_read_releases_its_slot_only_after_worker_exit() {
        let observer = Arc::new(QueryObserver::new(true, [], 20_000));
        let question = mosdns_dns_core::QuestionInfo {
            qname_wire: vec![3, b'r', b'e', b'c', 0],
            qtype: 1,
            qclass: 1,
        };
        for _ in 0..20_000 {
            drop(observer.admit(
                "127.0.0.1:53000".parse().expect("client"),
                crate::observer::QueryTransport::Udp,
                &question,
                TransportCancellation::new(),
            ));
        }
        let slots = Arc::new(Semaphore::new(1));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("audit read runtime");
        let canceled = TransportCancellation::new();
        runtime.block_on(async {
            let first_slots = Arc::clone(&slots);
            let first_observer = observer;
            let first_shutdown = canceled.clone();
            let first = tokio::spawn(async move {
                dispatch_audit_read(
                    first_observer,
                    &first_slots,
                    "/api/v2/audit/logs?limit=50",
                    AuditReadKind::Logs {
                        domain_route: false,
                    },
                    first_shutdown,
                )
                .await
            });
            for _ in 0..100 {
                if slots.available_permits() == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert_eq!(
                slots.available_permits(),
                0,
                "worker must own the read slot"
            );
            assert!(
                !first.is_finished(),
                "the expensive worker must still be active"
            );
            canceled.cancel();

            let rejected = dispatch_audit_read(
                Arc::new(QueryObserver::new(true, [], 4)),
                &slots,
                "/api/v2/audit/logs?limit=50",
                AuditReadKind::Logs {
                    domain_route: false,
                },
                TransportCancellation::new(),
            )
            .await;
            assert_eq!(rejected.status, 503);
            assert_eq!(rejected.body, b"audit read capacity exhausted\n");

            let first_response = first.await.expect("canceled audit worker");
            assert_eq!(first_response.status, 499);
            assert_eq!(slots.available_permits(), 1);

            let rebound = dispatch_audit_read(
                Arc::new(QueryObserver::new(true, [], 4)),
                &slots,
                "/api/v2/audit/logs?limit=50",
                AuditReadKind::Logs {
                    domain_route: false,
                },
                TransportCancellation::new(),
            )
            .await;
            assert_eq!(rebound.status, 200);
        });
    }

    #[test]
    fn windows_use_the_oldest_admission_time_not_terminal_order() {
        use std::time::{Duration, UNIX_EPOCH};

        let now = UNIX_EPOCH + Duration::from_secs(7_200);
        let older = now - Duration::from_secs(7_200);
        let newer = now - Duration::from_secs(1_800);
        let windows = super::audit_windows(
            &[
                super::AuditTimingSnapshot {
                    timestamp: newer,
                    elapsed: Duration::from_millis(1),
                },
                super::AuditTimingSnapshot {
                    timestamp: older,
                    elapsed: Duration::from_millis(2),
                },
            ],
            now,
        );
        assert_eq!(
            windows.items[0].coverage_start,
            Some("1970-01-01T00:00:00Z".to_owned())
        );
        assert!(windows.items[0].complete);
    }
}
