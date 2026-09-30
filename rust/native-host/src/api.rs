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

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use mosdns_upstream_core::TransportCancellation;

use crate::config::CompiledConfig;
use crate::managed::ManagedDomainSet;
use crate::observer::QueryObserver;
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
                            let connection_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                let _ = process_connection(
                                    stream,
                                    config,
                                    observer,
                                    state_root,
                                    audit_persist_lock,
                                    audit_persistence_faults,
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

    fn method_not_allowed() -> Self {
        Self::error(405, "method not allowed")
    }
}

async fn process_connection(
    mut stream: TcpStream,
    config: Rc<CompiledConfig>,
    observer: Arc<QueryObserver>,
    state_root: Option<PathBuf>,
    audit_persist_lock: Arc<AsyncMutex<()>>,
    audit_persistence_faults: AuditPersistenceFaults,
    shutdown: TransportCancellation,
) -> io::Result<()> {
    let response = tokio::select! {
        biased;
        () = shutdown.cancelled() => return Ok(()),
        request = read_request(&mut stream) => match request {
            Ok(Some(request)) => {
                dispatch(
                    &config,
                    &observer,
                    state_root.as_deref(),
                    &audit_persist_lock,
                    &audit_persistence_faults,
                    &request,
                )
                .await
            }
            // The client closed before sending a complete request.
            Ok(None) => return Ok(()),
            Err(_) => Response::error(400, "bad request"),
        },
    };
    tokio::select! {
        biased;
        () = shutdown.cancelled() => Ok(()),
        written = write_response(&mut stream, &response) => written,
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

async fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
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

fn route(target: &str) -> Route<'_> {
    // Go's `/show` ignores the query string; the UI sends `?limit=10000`.
    let path = target.split_once('?').map_or(target, |(path, _query)| path);
    if path == "/api/v1/special-groups" {
        return Route::SpecialGroups;
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

async fn dispatch(
    config: &CompiledConfig,
    observer: &QueryObserver,
    state_root: Option<&Path>,
    audit_persist_lock: &Arc<AsyncMutex<()>>,
    audit_persistence_faults: &AuditPersistenceFaults,
    request: &Request,
) -> Response {
    match route(&request.target) {
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

    use super::{load_audit_capacity, migrate_legacy_settings};

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
}
