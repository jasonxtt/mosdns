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
use std::rc::Rc;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use mosdns_upstream_core::TransportCancellation;

use crate::config::CompiledConfig;
use crate::managed::ManagedDomainSet;
use crate::udp::{drain_tasks, reap_one_task};

/// The largest request head accepted before the connection is rejected.
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// The largest request body accepted before the connection is rejected.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The host-owned management listener.
pub struct ApiServer {
    listener: TcpListener,
    config: Rc<CompiledConfig>,
    accept_fault_after: Cell<Option<usize>>,
}

impl ApiServer {
    /// Binds the configured management address. Binding is separate from
    /// serving so the supervisor can bind every listener before any of them
    /// starts accepting.
    pub async fn bind(
        config: Rc<CompiledConfig>,
        address: SocketAddr,
    ) -> Result<Self, ApiServerError> {
        let listener = TcpListener::bind(address)
            .await
            .map_err(ApiServerError::Bind)?;
        Ok(Self {
            listener,
            config,
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
                            let connection_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                let _ = process_connection(stream, config, connection_shutdown).await;
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
}

async fn process_connection(
    mut stream: TcpStream,
    config: Rc<CompiledConfig>,
    shutdown: TransportCancellation,
) -> io::Result<()> {
    let response = tokio::select! {
        biased;
        () = shutdown.cancelled() => return Ok(()),
        request = read_request(&mut stream) => match request {
            Ok(Some(request)) => dispatch(&config, &request),
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

/// Which configured operation one request addresses.
enum Route<'a> {
    Show(&'a str),
    Save(&'a str),
    Post(&'a str),
    SpecialGroups,
    Unknown,
    WrongMethod,
}

fn route<'a>(method: &str, target: &'a str) -> Route<'a> {
    // Go's `/show` ignores the query string; the UI sends `?limit=10000`.
    let path = target.split_once('?').map_or(target, |(path, _query)| path);
    if path == "/api/v1/special-groups" {
        return if method == "GET" {
            Route::SpecialGroups
        } else {
            Route::WrongMethod
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
        "show" | "save" | "post" => match (method, action) {
            ("GET", "show") => Route::Show(tag),
            ("GET", "save") => Route::Save(tag),
            ("POST", "post") => Route::Post(tag),
            // Any other method on a registered path, as chi replies.
            _ => Route::WrongMethod,
        },
        _ => Route::Unknown,
    }
}

fn dispatch(config: &CompiledConfig, request: &Request) -> Response {
    match route(&request.method, &request.target) {
        Route::Show(tag) => match eligible(config, tag) {
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
        Route::Save(tag) => match eligible(config, tag) {
            Err(response) => response,
            Ok(provider) => match provider.save() {
                Ok(()) => Response::empty(200),
                Err(error) => Response::error(500, &error.to_string()),
            },
        },
        Route::Post(tag) => match eligible(config, tag) {
            Err(response) => response,
            Ok(provider) => {
                let payload: PostPayload = match serde_json::from_slice(&request.body) {
                    Ok(payload) => payload,
                    Err(_) => return Response::error(400, "invalid JSON"),
                };
                match provider.replace(&payload.values) {
                    Ok(count) => {
                        Response::text(format!("domain_set replaced with {count} entries"))
                    }
                    Err(error) => Response::error(500, &error.to_string()),
                }
            }
        },
        // The strict native subset cannot configure dedicated routing groups, so
        // an empty list is the true state and no group mutation route exists.
        Route::SpecialGroups => Response {
            status: 200,
            content_type: Some("application/json"),
            body: b"[]\n".to_vec(),
        },
        // Mirrors Go's `http.NotFound` for an unmatched route or unmounted tag.
        Route::Unknown => Response::error(404, "404 page not found"),
        // Mirrors chi's method-not-allowed reply: status only, no body.
        Route::WrongMethod => Response::empty(405),
    }
}

/// Looks up one management-eligible provider, or the explicit failure.
fn eligible<'a>(config: &'a CompiledConfig, tag: &str) -> Result<&'a ManagedDomainSet, Response> {
    let Some(set) = config.domain_set(tag) else {
        return Err(Response::error(404, "404 page not found"));
    };
    set.managed.as_deref().ok_or_else(|| {
        Response::error(
            400,
            &format!("domain_set `{tag}` is not a single-file .txt managed profile"),
        )
    })
}

/// The UI payload: `{ "values": ["...", ...] }`.
#[derive(Deserialize)]
struct PostPayload {
    #[serde(default)]
    values: Vec<String>,
}
