//! Bounded HTTP(S) GET for rule-source downloads.
//!
//! The reference providers use Go's `http.Client`: verified TLS, redirects
//! followed (at most 10), an overall timeout, and an optional SOCKS5 dialer.
//! This client keeps those semantics over hyper HTTP/1.1 and rustls, adds a
//! body size limit, and stops at once when the shutdown token is cancelled.

use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// The reference `http.Client` follows at most this many redirects.
const MAX_REDIRECTS: usize = 10;

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
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err("canceled by shutdown".to_owned()),
        result = tokio::time::timeout(fetch.timeout, follow(&fetch)) => match result {
            Ok(result) => result,
            Err(_) => Err(format!("timed out after {}s", fetch.timeout.as_secs())),
        },
    }
}

async fn follow(fetch: &Fetch<'_>) -> Result<Vec<u8>, String> {
    let mut url = url::Url::parse(fetch.url).map_err(|error| format!("invalid url: {error}"))?;
    for _ in 0..=MAX_REDIRECTS {
        match get_once(&url, fetch).await? {
            Reply::Body(body) => return Ok(body),
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
    Body(Vec<u8>),
    Redirect(String),
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn get_once(url: &url::Url, fetch: &Fetch<'_>) -> Result<Reply, String> {
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
    let tcp = if fetch.socks5.is_empty() {
        TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|error| format!("connect {host}:{port}: {error}"))?
    } else {
        mosdns_upstream_core::socks5::connect(fetch.socks5, &host, port)
            .await
            .map_err(|error| format!("proxy {}: {error}", fetch.socks5))?
    };
    let io: Box<dyn Io> = if https {
        Box::new(tls(tcp, &host, fetch.roots.clone()).await?)
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
    let request = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(target)
        .header(hyper::header::HOST, authority)
        .header(hyper::header::USER_AGENT, "Go-http-client/1.1")
        .body(Empty::<Bytes>::new())
        .map_err(|error| format!("build request: {error}"))?;
    let exchange = async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|error| format!("http request: {error}"))?;
        let status = response.status();
        if status.is_redirection()
            && matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.headers().get(hyper::header::LOCATION)
        {
            let location = location
                .to_str()
                .map_err(|_| "redirect location is not text".to_owned())?;
            return Ok(Reply::Redirect(location.to_owned()));
        }
        if status != hyper::StatusCode::OK {
            return Err(format!("bad status: {}", status.as_u16()));
        }
        let body = Limited::new(response.into_body(), fetch.max_bytes)
            .collect()
            .await
            .map_err(|error| {
                if error.is::<http_body_util::LengthLimitError>() {
                    format!("body exceeds {} bytes", fetch.max_bytes)
                } else {
                    format!("read body: {error}")
                }
            })?;
        Ok(Reply::Body(body.to_bytes().to_vec()))
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

/// POSTs `body` as JSON to `url` (HTTP or HTTPS, no redirects, no proxy). The
/// reply is read and dropped; only a transport failure is an error, as with
/// the reference publisher, which ignores the response status.
///
/// # Errors
///
/// A message for the log: bad URL, connect/TLS failure, the deadline, or shutdown.
pub(crate) async fn post_json(
    url: &str,
    body: &[u8],
    timeout: Duration,
    roots: Option<Arc<rustls::RootCertStore>>,
    cancel: &TransportCancellation,
) -> Result<(), String> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err("canceled by shutdown".to_owned()),
        result = tokio::time::timeout(timeout, post_once(url, body, roots)) => match result {
            Ok(result) => result,
            Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
        },
    }
}

async fn post_once(
    url: &str,
    body: &[u8],
    roots: Option<Arc<rustls::RootCertStore>>,
) -> Result<(), String> {
    let url = url::Url::parse(url).map_err(|error| format!("invalid url: {error}"))?;
    let https = match url.scheme() {
        "http" => false,
        "https" => true,
        other => return Err(format!("unsupported url scheme `{other}`")),
    };
    let host = url
        .host_str()
        .ok_or_else(|| "url has no host".to_owned())?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "url has no port".to_owned())?;
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|error| format!("connect {host}:{port}: {error}"))?;
    let io: Box<dyn Io> = if https {
        Box::new(tls(tcp, &host, roots).await?)
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
    let request = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(target)
        .header(hyper::header::HOST, authority)
        .header(hyper::header::USER_AGENT, "Go-http-client/1.1")
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::copy_from_slice(body)))
        .map_err(|error| format!("build request: {error}"))?;
    let exchange = async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|error| format!("http request: {error}"))?;
        // Drained and dropped; a body over the limit only ends the read.
        let _ = Limited::new(response.into_body(), 1024 * 1024)
            .collect()
            .await;
        Ok(())
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
