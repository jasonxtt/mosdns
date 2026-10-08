//! Minimal SOCKS5 (RFC 1928) TCP CONNECT client.
//!
//! It mirrors what the reference host gets from `golang.org/x/net/proxy`
//! `SOCKS5(network, addr, nil, proxy.Direct)`: no authentication, and the
//! target host is sent as a domain name (remote resolution) unless it is an IP
//! literal. Rule downloads and the `aliapi` TCP/TLS/HTTPS upstreams use it.
//! Deadlines and cancellation belong to the caller (wrap the future).

use std::fmt;
use std::io;
use std::net::IpAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Why a SOCKS5 CONNECT failed.
#[derive(Debug)]
pub enum Socks5Error {
    /// Reaching or talking to the proxy failed.
    Io(io::Error),
    /// The proxy answered with something that is not SOCKS5.
    Protocol(&'static str),
    /// The proxy requires authentication, which is not supported.
    AuthRequired,
    /// The proxy refused the CONNECT with this reply code.
    Refused(u8),
    /// The target host name cannot be sent (longer than 255 bytes or empty).
    InvalidTarget,
}

impl fmt::Display for Socks5Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "socks5: {error}"),
            Self::Protocol(message) => write!(formatter, "socks5: {message}"),
            Self::AuthRequired => formatter.write_str("socks5: proxy requires authentication"),
            Self::Refused(code) => write!(formatter, "socks5: connect refused (reply {code})"),
            Self::InvalidTarget => formatter.write_str("socks5: invalid target host"),
        }
    }
}

impl std::error::Error for Socks5Error {}

impl From<io::Error> for Socks5Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// How a TCP-based upstream reaches its server through a SOCKS5 proxy.
///
/// `domain` is the target host name sent to the proxy for remote resolution
/// (the reference host does this when the upstream has no numeric `dial_addr`);
/// without it the numeric dial address of the endpoint is sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Route {
    proxy: std::sync::Arc<str>,
    domain: Option<std::sync::Arc<str>>,
}

impl Route {
    /// `proxy` is `host:port`; `domain` the optional remote-resolved target.
    #[must_use]
    pub fn new(proxy: &str, domain: Option<&str>) -> Self {
        Self {
            proxy: proxy.into(),
            domain: domain.map(Into::into),
        }
    }

    /// The proxy address (`host:port`).
    #[must_use]
    pub fn proxy(&self) -> &str {
        &self.proxy
    }

    /// The target host name the proxy resolves, when one is configured.
    #[must_use]
    pub fn domain(&self) -> Option<&str> {
        self.domain.as_deref()
    }
}

/// Opens the TCP connection to `address`, directly or through `route`.
pub(crate) async fn dial(
    address: std::net::SocketAddr,
    route: Option<&Route>,
) -> io::Result<TcpStream> {
    let Some(route) = route else {
        return TcpStream::connect(address).await;
    };
    let host = route
        .domain
        .as_deref()
        .map_or_else(|| address.ip().to_string(), str::to_owned);
    connect(&route.proxy, &host, address.port())
        .await
        .map_err(io::Error::other)
}

/// Connects to `proxy` (`host:port`, resolved by the system resolver) and asks
/// it to CONNECT to `host:port`. The returned stream carries the tunnelled
/// connection.
///
/// # Errors
///
/// Any proxy I/O failure, a non-SOCKS5 or authenticating proxy, or a refused
/// CONNECT.
pub async fn connect(proxy: &str, host: &str, port: u16) -> Result<TcpStream, Socks5Error> {
    let mut stream = TcpStream::connect(proxy).await?;
    handshake(&mut stream, host, port).await?;
    Ok(stream)
}

async fn handshake(stream: &mut TcpStream, host: &str, port: u16) -> Result<(), Socks5Error> {
    // Greeting: version 5, one method, "no authentication".
    stream.write_all(&[5, 1, 0]).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != 5 {
        return Err(Socks5Error::Protocol("unexpected greeting version"));
    }
    if choice[1] != 0 {
        return Err(Socks5Error::AuthRequired);
    }
    let mut request = vec![5, 1, 0];
    let literal = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    match literal.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            let name = host.as_bytes();
            let length = u8::try_from(name.len()).map_err(|_| Socks5Error::InvalidTarget)?;
            if length == 0 {
                return Err(Socks5Error::InvalidTarget);
            }
            request.push(3);
            request.push(length);
            request.extend_from_slice(name);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 {
        return Err(Socks5Error::Protocol("unexpected reply version"));
    }
    if reply[1] != 0 {
        return Err(Socks5Error::Refused(reply[1]));
    }
    // Discard the bound address the proxy reports.
    let remaining = match reply[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await?;
            usize::from(length[0]) + 2
        }
        _ => return Err(Socks5Error::Protocol("unknown bound address type")),
    };
    let mut bound = vec![0u8; remaining];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// A one-shot proxy that records the CONNECT request and answers `reply`.
    async fn proxy(reply: Vec<u8>, method: u8) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            socket.write_all(&[5, method]).await.unwrap();
            if method != 0 {
                return Vec::new();
            }
            let mut head = [0u8; 4];
            socket.read_exact(&mut head).await.unwrap();
            let mut request = head.to_vec();
            let rest = match head[3] {
                1 => 6,
                4 => 18,
                _ => {
                    let mut length = [0u8; 1];
                    socket.read_exact(&mut length).await.unwrap();
                    request.push(length[0]);
                    usize::from(length[0]) + 2
                }
            };
            let mut tail = vec![0u8; rest];
            socket.read_exact(&mut tail).await.unwrap();
            request.extend(tail);
            socket.write_all(&reply).await.unwrap();
            socket.write_all(b"tunnel").await.unwrap();
            request
        });
        (address, task)
    }

    #[test]
    fn connect_sends_a_domain_name_and_returns_the_tunnel() {
        runtime().block_on(async {
            let (address, task) = proxy(vec![5, 0, 0, 1, 127, 0, 0, 1, 0, 80], 0).await;
            let mut stream = connect(&address, "example.com", 443).await.unwrap();
            let mut tunnel = [0u8; 6];
            stream.read_exact(&mut tunnel).await.unwrap();
            assert_eq!(&tunnel, b"tunnel");
            let request = task.await.unwrap();
            let mut expected = vec![5, 1, 0, 3, 11];
            expected.extend_from_slice(b"example.com");
            expected.extend_from_slice(&443u16.to_be_bytes());
            assert_eq!(request, expected);
        });
    }

    #[test]
    fn ip_literals_use_address_types_and_domain_bound_addresses_are_skipped() {
        runtime().block_on(async {
            let (address, task) =
                proxy(vec![5, 0, 0, 3, 4, b'h', b'o', b's', b't', 0, 80], 0).await;
            let mut stream = connect(&address, "[2001:db8::1]", 80).await.unwrap();
            let mut tunnel = [0u8; 6];
            stream.read_exact(&mut tunnel).await.unwrap();
            assert_eq!(&tunnel, b"tunnel");
            let request = task.await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 4]);
            let (address, task) = proxy(vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0], 0).await;
            connect(&address, "10.0.0.1", 53).await.unwrap();
            assert_eq!(task.await.unwrap(), vec![5, 1, 0, 1, 10, 0, 0, 1, 0, 53]);
        });
    }

    #[test]
    fn a_route_sends_its_domain_or_the_numeric_address() {
        runtime().block_on(async {
            let target: std::net::SocketAddr = "192.0.2.7:853".parse().unwrap();
            let (address, task) = proxy(vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0], 0).await;
            let route = Route::new(&address, None);
            dial(target, Some(&route)).await.unwrap();
            assert_eq!(task.await.unwrap(), vec![5, 1, 0, 1, 192, 0, 2, 7, 3, 85]);
            let (address, task) = proxy(vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0], 0).await;
            let route = Route::new(&address, Some("dns.example"));
            dial(target, Some(&route)).await.unwrap();
            let mut expected = vec![5, 1, 0, 3, 11];
            expected.extend_from_slice(b"dns.example");
            expected.extend_from_slice(&853u16.to_be_bytes());
            assert_eq!(task.await.unwrap(), expected);
            // Without a route the address is dialed directly.
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            dial(listener.local_addr().unwrap(), None).await.unwrap();
        });
    }

    #[test]
    fn refusals_and_authentication_are_errors() {
        runtime().block_on(async {
            let (address, _task) = proxy(vec![5, 5, 0, 1, 0, 0, 0, 0, 0, 0], 0).await;
            assert!(matches!(
                connect(&address, "example.com", 80).await,
                Err(Socks5Error::Refused(5))
            ));
            let (address, _task) = proxy(Vec::new(), 2).await;
            assert!(matches!(
                connect(&address, "example.com", 80).await,
                Err(Socks5Error::AuthRequired)
            ));
            let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = closed.local_addr().unwrap().to_string();
            drop(closed);
            assert!(matches!(
                connect(&address, "example.com", 80).await,
                Err(Socks5Error::Io(_))
            ));
            assert!(matches!(
                connect("127.0.0.1:1", &"a".repeat(256), 80).await,
                Err(Socks5Error::Io(_) | Socks5Error::InvalidTarget)
            ));
        });
    }
}
