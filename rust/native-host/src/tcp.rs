use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use mosdns_dns_core::{FrameMode, frame_response, parse_query};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::assembly::{ForwardCatalog, HostAssembly, HostOptions};
use crate::cache::NativeCacheAdapter;
use crate::config::{CompiledConfig, ListenerKind};
use crate::execution::{ExecutionRequest, execute_request};
use crate::observer::{
    FailureProvenance, LocalFailureKind, QueryObserver, QueryTerminalOutcome, QueryTransport,
    ResponseState, TerminalObservation,
};
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
    observer: Arc<QueryObserver>,
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
            observer: assembly.observer_handle(),
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
                        Ok((stream, peer)) => {
                            let config = Rc::clone(&self.config);
                            let forwards = Rc::clone(&self.forwards);
                            let cache = Rc::clone(&self.cache);
                            let observer = Arc::clone(&self.observer);
                            let options = self.options.clone();
                            let idle_timeout = self.idle_timeout;
                            let connection_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                process_connection(ConnectionTask {
                                    stream,
                                    config,
                                    forwards,
                                    cache,
                                    observer,
                                    client_addr: peer,
                                    options,
                                    idle_timeout,
                                    connection_shutdown,
                                }).await;
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

struct ConnectionTask {
    stream: TcpStream,
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    observer: Arc<QueryObserver>,
    client_addr: SocketAddr,
    options: HostOptions,
    idle_timeout: Duration,
    connection_shutdown: TransportCancellation,
}

async fn process_connection(task: ConnectionTask) {
    let ConnectionTask {
        mut stream,
        config,
        forwards,
        cache,
        observer,
        client_addr,
        options,
        idle_timeout,
        connection_shutdown,
    } = task;
    loop {
        let frame =
            match read_frame_with_control(&mut stream, idle_timeout, &connection_shutdown).await {
                Ok(Some(frame)) => frame,
                Ok(None) => return,
                Err(error) => {
                    if matches!(
                        error.kind(),
                        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                    ) {
                        observer.record_malformed();
                    }
                    return;
                }
            };
        let Ok((header, question)) = parse_query(&frame) else {
            // A malformed or partial DNS message closes only this connection.
            observer.record_malformed();
            return;
        };
        let admitted = observer.admit(
            client_addr,
            QueryTransport::Tcp,
            &question,
            connection_shutdown.clone(),
        );
        let mut request_options = options.clone();
        request_options.admission_deadline = Some(Instant::now() + options.request_deadline);
        let mut execution = execute_request(
            ExecutionRequest {
                config: &config,
                cache: &cache,
                options: &request_options,
                raw: &frame,
                header,
                question,
            },
            &forwards,
            connection_shutdown.clone(),
        )
        .await;
        let terminal = if connection_shutdown.is_cancelled() {
            QueryTerminalOutcome::Canceled
        } else if matches!(execution.response, ResponseState::NoResponse) {
            QueryTerminalOutcome::NoResponse
        } else {
            match frame_response(&execution.response_wire, FrameMode::Stream) {
                Ok(framed) => write_response(&mut stream, &framed, &connection_shutdown).await,
                Err(_) => {
                    execution.failure_provenance = Some(FailureProvenance::LocalFailure(
                        LocalFailureKind::ResponseConstruction,
                    ));
                    QueryTerminalOutcome::SendFailed
                }
            }
        };
        admitted.finish(TerminalObservation {
            outcome: terminal,
            response: execution.response,
            cache_status: execution.cache_status,
            final_sequence: execution.final_sequence,
            final_upstream: execution.final_upstream,
            upstream_attempts: execution.upstream_attempts,
            failure_provenance: execution.failure_provenance,
            elapsed: Duration::ZERO,
        });
        if matches!(
            terminal,
            QueryTerminalOutcome::SendFailed
                | QueryTerminalOutcome::Canceled
                | QueryTerminalOutcome::NoResponse
        ) {
            return;
        }
    }
}

async fn write_response(
    stream: &mut TcpStream,
    response: &[u8],
    cancellation: &TransportCancellation,
) -> QueryTerminalOutcome {
    write_with_control(cancellation, stream.write_all(response)).await
}

async fn write_with_control<F>(
    cancellation: &TransportCancellation,
    write: F,
) -> QueryTerminalOutcome
where
    F: std::future::Future<Output = io::Result<()>>,
{
    tokio::select! {
        biased;
        () = cancellation.cancelled() => QueryTerminalOutcome::Canceled,
        result = write => if result.is_ok() {
            QueryTerminalOutcome::SendSucceeded
        } else {
            QueryTerminalOutcome::SendFailed
        },
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
        let read = match reader.read(&mut buffer[filled..]).await {
            Ok(read) => read,
            Err(error) if filled > 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("truncated DNS frame: {error}"),
                ));
            }
            Err(error) => return Err(error),
        };
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use mosdns_dns_core::{
        FrameMode, frame_response, parse_query, synthesize_response, validate_response,
    };
    use mosdns_upstream_core::TransportCancellation;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    use super::{read_frame, write_with_control};
    use crate::assembly::HostAssembly;
    use crate::assembly::HostRuntime;
    use crate::observer::{
        CacheStatus, QueryTerminalOutcome, QueryTransport, ResponseSource, ResponseState,
    };

    const TCP_CONFIG: &str =
        include_str!("../../../tests/phase5a-baseline/configs/forward-tcp.yaml");

    #[test]
    fn tcp_write_failure_and_pending_write_cancellation_are_distinct() {
        let runtime = HostRuntime::new().expect("test runtime");
        runtime.block_on(async {
            let cancellation = TransportCancellation::new();
            let failure = write_with_control(&cancellation, async {
                Err(std::io::Error::other("injected TCP write failure"))
            })
            .await;
            assert_eq!(failure, QueryTerminalOutcome::SendFailed);

            let cancellation = TransportCancellation::new();
            cancellation.cancel();
            let canceled = write_with_control(&cancellation, std::future::pending()).await;
            assert_eq!(canceled, QueryTerminalOutcome::Canceled);
        });
    }

    #[test]
    fn tcp_connection_records_multiple_queries_and_excludes_partial_input() {
        let upstream = std::net::TcpListener::bind("127.0.0.1:0").expect("upstream listener");
        upstream
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let upstream_addr = upstream.local_addr().expect("upstream address");
        let yaml = TCP_CONFIG
            .replace("127.0.0.1:15454", &upstream_addr.to_string())
            .replace("enable_audit: false", "enable_audit: true");
        let assembly = HostAssembly::from_yaml(&yaml).expect("native host");

        assembly.block_on(async {
            let upstream = TcpListener::from_std(upstream).expect("Tokio upstream listener");
            let upstream_task = tokio::task::spawn_local(async move {
                let mut served = 0;
                while served < 2 {
                    let (mut stream, _) = upstream.accept().await.expect("accept upstream");
                    while let Some(query) =
                        read_frame(&mut stream).await.expect("read upstream query")
                    {
                        let (header, question) = parse_query(&query).expect("upstream DNS query");
                        let response =
                            synthesize_response(&header, &question, 0).expect("DNS response");
                        let framed =
                            frame_response(&response, FrameMode::Stream).expect("framed response");
                        stream.write_all(&framed).await.expect("upstream write");
                        served += 1;
                        if served == 2 {
                            return;
                        }
                    }
                }
            });

            let server =
                super::TcpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("TCP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let mut client = TcpStream::connect(server_addr)
                .await
                .expect("connect client");
            let client_addr: SocketAddr = client.local_addr().expect("client address");
            let first = query(0x4101, "one.example");
            let second = query(0x4102, "two.example");
            let first_frame = frame_response(&first, FrameMode::Stream).expect("first frame");
            let second_frame = frame_response(&second, FrameMode::Stream).expect("second frame");
            client
                .write_all(&[first_frame, second_frame].concat())
                .await
                .expect("write pipelined requests");

            let first_response = read_frame(&mut client)
                .await
                .expect("read first response")
                .expect("first response exists");
            let second_response = read_frame(&mut client)
                .await
                .expect("read second response")
                .expect("second response exists");
            validate_response(&first_response).expect("first DNS response");
            validate_response(&second_response).expect("second DNS response");
            assert_eq!(&first_response[..2], &[0x41, 0x01]);
            assert_eq!(&second_response[..2], &[0x41, 0x02]);

            while assembly.metrics_snapshot().completed_total != 2 {
                tokio::task::yield_now().await;
            }
            let records = assembly.audit_snapshot().records;
            assert_eq!(records.len(), 2);
            assert_eq!(records[0].client_addr, client_addr);
            assert_eq!(records[0].transport, QueryTransport::Tcp);
            assert_eq!(records[0].qname, "one.example.");
            assert_eq!(records[1].qname, "two.example.");
            assert!(
                records.iter().all(|record| {
                    record.terminal_outcome == QueryTerminalOutcome::SendSucceeded
                        && record.response
                            == ResponseState::Dns {
                                rcode: 0,
                                source: ResponseSource::Upstream("phase5a_forward".to_owned()),
                            }
                        && record.cache_status == CacheStatus::NotApplicable
                        && record.final_sequence.as_deref() == Some("phase5a_entry")
                        && record.final_upstream.as_deref() == Some("phase5a_forward")
                }),
                "unexpected TCP audit records: {records:#?}"
            );

            client
                .write_all(&[0, 10, 1, 2])
                .await
                .expect("partial frame prefix");
            client.shutdown().await.expect("half-close client");
            while assembly.metrics_snapshot().malformed_total != 1 {
                tokio::task::yield_now().await;
            }
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.admitted_total, 2);
            assert_eq!(metrics.completed_total, 2);
            assert_eq!(metrics.in_flight, 0);
            assert_eq!(metrics.send_succeeded_total, 2);
            assert_eq!(metrics.malformed_total, 1);
            assert_eq!(assembly.audit_snapshot().records.len(), 2);

            shutdown.cancel();
            assert!(server_task.await.expect("server task").is_ok());
            upstream_task.await.expect("upstream task");
        });
    }

    fn query(id: u16, name: &str) -> Vec<u8> {
        let mut query = Vec::from(id.to_be_bytes());
        query.extend_from_slice(&[1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        for label in name.split('.') {
            query.push(u8::try_from(label.len()).expect("test label length"));
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        query
    }
}
