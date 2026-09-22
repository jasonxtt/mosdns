use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use mosdns_dns_core::{FrameMode, frame_response, parse_query};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::assembly::{ForwardCatalog, HostAssembly, HostOptions};
use crate::cache::NativeCacheAdapter;
use crate::config::{CompiledConfig, ListenerKind};
use crate::execution::{ExecutionRequest, execute_request};
use crate::udp::{drain_tasks, reap_one_task};

const MAX_TCP_FRAME: usize = u16::MAX as usize;

/// A local TCP listener for the supported W1 DNS-over-TCP path.
pub struct TcpServer {
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    options: HostOptions,
    listener: Arc<TcpListener>,
    idle_timeout: Duration,
}

impl TcpServer {
    /// Binds a TCP listener after strict configuration compilation.
    pub async fn bind(assembly: &HostAssembly, listen: SocketAddr) -> Result<Self, TcpServerError> {
        if assembly.config().listener.kind != ListenerKind::Tcp {
            return Err(TcpServerError::WrongListener(
                assembly.config().listener.kind,
            ));
        }
        let idle_timeout = assembly
            .config()
            .listener
            .idle_timeout
            .ok_or(TcpServerError::MissingIdleTimeout)?;
        let listener = TcpListener::bind(listen)
            .await
            .map_err(TcpServerError::Bind)?;
        Ok(Self {
            config: assembly.config_handle(),
            forwards: assembly.forwards_handle(),
            cache: assembly.cache_handle(),
            options: assembly.options().clone(),
            listener: Arc::new(listener),
            idle_timeout,
        })
    }

    /// Binds the listener address declared by the compiled W1 YAML.
    pub async fn bind_configured(assembly: &HostAssembly) -> Result<Self, TcpServerError> {
        Self::bind(assembly, assembly.config().listener.listen).await
    }

    pub fn local_addr(&self) -> Result<SocketAddr, TcpServerError> {
        self.listener
            .local_addr()
            .map_err(TcpServerError::LocalAddr)
    }

    /// Accepts connections concurrently and processes each connection's DNS
    /// frames sequentially. Shutdown stops admission, cancels and joins every
    /// connection task, then closes the one upstream owner.
    pub async fn serve(self, shutdown: TransportCancellation) -> Result<(), TcpServerError> {
        let mut tasks = JoinSet::new();
        let mut accept_error = None;
        let mut task_error = None;

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
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let config = Rc::clone(&self.config);
                            let forwards = Rc::clone(&self.forwards);
                            let cache = Rc::clone(&self.cache);
                            let options = self.options.clone();
                            let idle_timeout = self.idle_timeout;
                            let connection_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                process_connection(
                                    stream,
                                    config,
                                    forwards,
                                    cache,
                                    options,
                                    idle_timeout,
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
        self.forwards.close_all().await;
        if let Some(error) = task_error {
            return Err(TcpServerError::Task(error));
        }
        if let Some(error) = accept_error {
            return Err(TcpServerError::Accept(error));
        }
        Ok(())
    }
}

async fn process_connection(
    mut stream: TcpStream,
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    options: HostOptions,
    idle_timeout: Duration,
    connection_shutdown: TransportCancellation,
) {
    loop {
        let frame =
            match read_frame_with_control(&mut stream, idle_timeout, &connection_shutdown).await {
                Ok(Some(frame)) => frame,
                Ok(None) | Err(_) => return,
            };
        let Ok((header, question)) = parse_query(&frame) else {
            // A malformed or partial DNS message closes only this connection.
            return;
        };
        let response = execute_request(
            ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &options,
                raw: &frame,
                header,
                question,
            },
            &forwards,
            connection_shutdown.clone(),
        )
        .await;
        if connection_shutdown.is_cancelled() {
            return;
        }
        let Ok(framed) = frame_response(&response, FrameMode::Stream) else {
            return;
        };
        if !write_response(&mut stream, &framed, &connection_shutdown).await {
            return;
        }
    }
}

async fn write_response(
    stream: &mut TcpStream,
    response: &[u8],
    cancellation: &TransportCancellation,
) -> bool {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => false,
        result = stream.write_all(response) => result.is_ok(),
    }
}

async fn read_frame_with_control(
    stream: &mut TcpStream,
    idle_timeout: Duration,
    cancellation: &TransportCancellation,
) -> io::Result<Option<Vec<u8>>> {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
        result = tokio::time::timeout(idle_timeout, read_frame(stream)) => {
            match result {
                Ok(result) => result,
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "idle timeout")),
            }
        }
    }
}

async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0_u8; 2];
    if read_exact_or_eof(reader, &mut prefix).await? {
        return Ok(None);
    }
    let length = usize::from(u16::from_be_bytes(prefix));
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero-length DNS frame",
        ));
    }
    let mut body = vec![0_u8; length.min(MAX_TCP_FRAME)];
    if read_exact_or_eof(reader, &mut body).await? {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated DNS frame",
        ));
    }
    Ok(Some(body))
}

async fn read_exact_or_eof<R>(reader: &mut R, buffer: &mut [u8]) -> io::Result<bool>
where
    R: AsyncRead + Unpin,
{
    let mut filled = 0;
    while filled < buffer.len() {
        let read = reader.read(&mut buffer[filled..]).await?;
        if read == 0 {
            if filled == 0 {
                return Ok(true);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated DNS frame",
            ));
        }
        filled += read;
    }
    Ok(false)
}

/// Failures that can occur while owning the TCP listener.
#[derive(Debug)]
pub enum TcpServerError {
    WrongListener(ListenerKind),
    MissingIdleTimeout,
    Bind(io::Error),
    LocalAddr(io::Error),
    Accept(io::Error),
    Task(String),
}

impl fmt::Display for TcpServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongListener(kind) => write!(formatter, "listener is not TCP: {kind:?}"),
            Self::MissingIdleTimeout => write!(formatter, "TCP listener is missing idle timeout"),
            Self::Bind(error) => write!(formatter, "TCP bind failed: {error}"),
            Self::LocalAddr(error) => write!(formatter, "TCP local address failed: {error}"),
            Self::Accept(error) => write!(formatter, "TCP accept failed: {error}"),
            Self::Task(error) => write!(formatter, "TCP connection task failed: {error}"),
        }
    }
}

impl std::error::Error for TcpServerError {}
