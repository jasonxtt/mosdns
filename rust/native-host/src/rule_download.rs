//! Bounded HTTP(S) client for rule-source downloads and plugin URL actions.
//!
//! The reference providers use Go's `http.Client`: verified TLS, redirects
//! followed (at most 10), an overall timeout, and an optional SOCKS5 dialer.
//! This client keeps those semantics over hyper HTTP/1.1 and rustls, adds a
//! body size limit, and stops at once when the shutdown token is cancelled.

use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// The reference `http.Client` follows at most this many redirects.
const MAX_REDIRECTS: usize = 10;

/// One request: a download (`fetch`) or a call to a URL (`call`).
pub(crate) struct Call<'a> {
    pub(crate) method: &'static str,
    pub(crate) url: &'a str,
    /// Sent with `Content-Type: application/json` when present.
    pub(crate) body: Option<&'a [u8]>,
    /// `host:port` of a SOCKS5 proxy, or empty for a direct connection.
    pub(crate) socks5: &'a str,
    /// Overall deadline: connect, TLS, redirects and the whole body.
    pub(crate) timeout: Duration,
    pub(crate) max_bytes: usize,
    /// Trust roots; `None` loads the system store.
    pub(crate) roots: Option<Arc<rustls::RootCertStore>>,
    /// Follow redirects like the reference client (GET only).
    pub(crate) follow: bool,
    /// Read the body of a non-200 answer; a download refuses it by status alone.
    pub(crate) error_body: bool,
}

/// One download request.
pub(crate) struct Fetch<'a> {
    pub(crate) url: &'a str,
    /// `host:port` of a SOCKS5 proxy, or empty for a direct connection.
    pub(crate) socks5: &'a str,
    /// Overall deadline: connect, TLS, redirects and the whole body.
    pub(crate) timeout: Duration,
    pub(crate) max_bytes: usize,
    /// Trust roots; `None` loads the system store.
    pub(crate) roots: Option<Arc<rustls::RootCertStore>>,
}

/// Downloads `fetch.url` and returns the body of the final 200 response.
///
/// # Errors
///
/// A message for the log: bad URL, connect/proxy/TLS failure, non-200 status,
/// too many redirects, a body over the limit, the deadline, or shutdown.
pub(crate) async fn fetch(
    fetch: Fetch<'_>,
    cancel: &TransportCancellation,
) -> Result<Vec<u8>, String> {
    let (status, body) = call(
        Call {
            method: "GET",
            url: fetch.url,
            body: None,
            socks5: fetch.socks5,
            timeout: fetch.timeout,
            max_bytes: fetch.max_bytes,
            roots: fetch.roots,
            follow: true,
            error_body: false,
        },
        cancel,
    )
    .await?;
    if status == 200 {
        Ok(body)
    } else {
        Err(format!("bad status: {status}"))
    }
}

/// Performs `call` and returns the final status and body (at most
/// `max_bytes`; a longer body is an error).
///
/// # Errors
///
/// A message for the log: bad URL, connect/proxy/TLS failure, too many
/// redirects, a body over the limit, the deadline, or shutdown.
pub(crate) async fn call(
    call: Call<'_>,
    cancel: &TransportCancellation,
) -> Result<(u16, Vec<u8>), String> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err("canceled by shutdown".to_owned()),
        result = tokio::time::timeout(call.timeout, follow(&call)) => match result {
            Ok(result) => result,
            Err(_) => Err(format!("timed out after {}s", call.timeout.as_secs())),
        },
    }
}

async fn follow(call: &Call<'_>) -> Result<(u16, Vec<u8>), String> {
    let mut url = url::Url::parse(call.url).map_err(|error| format!("invalid url: {error}"))?;
    for _ in 0..=MAX_REDIRECTS {
        match call_once(&url, call).await? {
            Reply::Done(status, body) => return Ok((status, body)),
            Reply::Redirect(location) => {
                url = url
                    .join(&location)
                    .map_err(|error| format!("invalid redirect location: {error}"))?;
            }
        }
    }
    Err(format!("stopped after {MAX_REDIRECTS} redirects"))
}

enum Reply {
    Done(u16, Vec<u8>),
    Redirect(String),
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn call_once(url: &url::Url, call: &Call<'_>) -> Result<Reply, String> {
    let https = match url.scheme() {
        "http" => false,
        "https" => true,
        other => return Err(format!("unsupported url scheme `{other}`")),
    };
    let host = match url.host() {
        Some(url::Host::Domain(name)) => name.to_owned(),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Ipv6(address)) => address.to_string(),
        None => return Err("url has no host".to_owned()),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "url has no port".to_owned())?;
    let tcp = if call.socks5.is_empty() {
        TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|error| format!("connect {host}:{port}: {error}"))?
    } else {
        mosdns_upstream_core::socks5::connect(call.socks5, &host, port)
            .await
            .map_err(|error| format!("proxy {}: {error}", call.socks5))?
    };
    let io: Box<dyn Io> = if https {
        Box::new(tls(tcp, &host, call.roots.clone()).await?)
    } else {
        Box::new(tcp)
    };
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|error| format!("http handshake: {error}"))?;
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    let authority = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_owned(),
    };
    let mut request = hyper::Request::builder()
        .method(call.method)
        .uri(target)
        .header(hyper::header::HOST, authority)
        .header(hyper::header::USER_AGENT, "Go-http-client/1.1");
    if call.body.is_some() {
        request = request.header(hyper::header::CONTENT_TYPE, "application/json");
    }
    let request = request
        .body(Full::new(Bytes::copy_from_slice(
            call.body.unwrap_or_default(),
        )))
        .map_err(|error| format!("build request: {error}"))?;
    let exchange = async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|error| format!("http request: {error}"))?;
        let status = response.status();
        if call.follow
            && status.is_redirection()
            && matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.headers().get(hyper::header::LOCATION)
        {
            let location = location
                .to_str()
                .map_err(|_| "redirect location is not text".to_owned())?;
            return Ok(Reply::Redirect(location.to_owned()));
        }
        if status != hyper::StatusCode::OK && !call.error_body {
            return Ok(Reply::Done(status.as_u16(), Vec::new()));
        }
        let body = Limited::new(response.into_body(), call.max_bytes)
            .collect()
            .await
            .map_err(|error| {
                if error.is::<http_body_util::LengthLimitError>() {
                    format!("body exceeds {} bytes", call.max_bytes)
                } else {
                    format!("read body: {error}")
                }
            })?;
        Ok(Reply::Done(status.as_u16(), body.to_bytes().to_vec()))
    };
    tokio::pin!(exchange);
    tokio::pin!(connection);
    let mut connection_done = false;
    loop {
        tokio::select! {
            result = &mut exchange => return result,
            outcome = &mut connection, if !connection_done => {
                connection_done = true;
                if let Err(error) = outcome {
                    return Err(format!("http connection: {error}"));
                }
            }
        }
    }
}

async fn tls(
    tcp: TcpStream,
    host: &str,
    roots: Option<Arc<rustls::RootCertStore>>,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let roots = match roots {
        Some(roots) => roots,
        None => system_roots()?,
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("tls config: {error}"))?
        .with_root_certificates(Arc::unwrap_or_clone(roots))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|error| format!("tls server name {host}: {error}"))?;
    tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|error| format!("tls handshake with {host}: {error}"))
}

/// The system trust store, loaded once per process.
fn system_roots() -> Result<Arc<rustls::RootCertStore>, String> {
    static ROOTS: std::sync::OnceLock<Result<Arc<rustls::RootCertStore>, String>> =
        std::sync::OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let loaded = rustls_native_certs::load_native_certs();
            let mut roots = rustls::RootCertStore::empty();
            for certificate in loaded.certs {
                let _ = roots.add(certificate);
            }
            if roots.is_empty() {
                Err("system trust store has no usable roots".to_owned())
            } else {
                Ok(Arc::new(roots))
            }
        })
        .clone()
}
