use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use mosdns_dns_core::{FrameMode, frame_response};
use mosdns_upstream_core::TransportCancellation;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;

use crate::assembly::{ForwardCatalog, HostAssembly, HostOptions};
use crate::cache::NativeCacheAdapter;
use crate::config::{CompiledConfig, ListenerKind};
use crate::execution::{ExecutionRequest, execute_request};
use crate::observer::{
    AdmittedQueryGuard, QueryObserver, QueryTerminalOutcome, QueryTransport, TerminalObservation,
};

const MAX_UDP_PACKET: usize = 65535;
#[cfg(test)]
const REFUSED: u8 = 5;

/// A local UDP listener owned by the native host. Its request tasks run on the
/// host's current-thread local task set because sequence executors are not
/// widened to `Send + Sync` by this migration slice.
pub struct UdpServer {
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    options: HostOptions,
    socket: Arc<UdpSocket>,
    observer: Arc<QueryObserver>,
}

impl UdpServer {
    /// Binds a UDP listener after strict configuration compilation. The
    /// supplied address is testable with port zero; the compiled graph still
    /// determines that the selected listener role is UDP.
    pub async fn bind(assembly: &HostAssembly, listen: SocketAddr) -> Result<Self, UdpServerError> {
        if assembly.config().listener.kind != ListenerKind::Udp {
            return Err(UdpServerError::WrongListener(
                assembly.config().listener.kind,
            ));
        }
        let socket = UdpSocket::bind(listen)
            .await
            .map_err(UdpServerError::Bind)?;
        Ok(Self {
            config: assembly.config_handle(),
            forwards: assembly.forwards_handle(),
            cache: assembly.cache_handle(),
            options: assembly.options().clone(),
            socket: Arc::new(socket),
            observer: assembly.observer_handle(),
        })
    }

    /// Binds the listener address declared by the compiled W1 YAML.
    pub async fn bind_configured(assembly: &HostAssembly) -> Result<Self, UdpServerError> {
        Self::bind(assembly, assembly.config().listener.listen).await
    }

    pub fn local_addr(&self) -> Result<SocketAddr, UdpServerError> {
        self.socket.local_addr().map_err(UdpServerError::LocalAddr)
    }

    /// Receives datagrams concurrently, stops admission on cancellation, then
    /// cancels/joins every request task before draining the upstream owner.
    pub async fn serve(self, shutdown: TransportCancellation) -> Result<(), UdpServerError> {
        let mut tasks = JoinSet::new();
        let mut packet = vec![0_u8; MAX_UDP_PACKET];
        let mut receive_error = None;
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
                received = self.socket.recv_from(&mut packet) => {
                    match received {
                        Ok((length, peer)) => {
                            let raw = packet[..length].to_vec();
                            let socket = Arc::clone(&self.socket);
                            let config = Rc::clone(&self.config);
                            let forwards = Rc::clone(&self.forwards);
                            let cache = Rc::clone(&self.cache);
                            let observer = Arc::clone(&self.observer);
                            let mut options = self.options.clone();
                            options.admission_deadline = Some(Instant::now() + options.request_deadline);
                            let request_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                process_request(RequestTask {
                                    socket,
                                    config,
                                    forwards,
                                    cache,
                                    observer,
                                    options,
                                    raw,
                                    peer,
                                    request_shutdown,
                                })
                                .await;
                            });
                        }
                        Err(error) => {
                            receive_error = Some(error);
                            shutdown.cancel();
                            break;
                        }
                    }
                }
            }
        }

        shutdown.cancel();
        finish_server(&self.forwards, &mut tasks, &mut task_error, receive_error).await
    }
}

struct RequestTask {
    socket: Arc<UdpSocket>,
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<NativeCacheAdapter>,
    observer: Arc<QueryObserver>,
    options: HostOptions,
    raw: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
}

async fn process_request(task: RequestTask) {
    let RequestTask {
        socket,
        config,
        forwards,
        cache,
        observer,
        options,
        raw,
        peer,
        request_shutdown,
    } = task;
    let Ok((header, question)) = mosdns_dns_core::parse_query(&raw) else {
        // Malformed UDP input is isolated to this datagram and produces no
        // response, matching the frozen server behavior.
        observer.record_malformed();
        return;
    };

    let admitted = observer.admit(
        peer,
        QueryTransport::Udp,
        &question,
        request_shutdown.clone(),
    );
    let progress = admitted.execution_progress();

    let mut execution = execute_request(
        ExecutionRequest {
            config: &config,
            cache: &cache,
            options: &options,
            raw: &raw,
            header,
            question,
        },
        &forwards,
        request_shutdown.clone(),
        progress,
    )
    .await;
    admitted.capture_execution(&TerminalObservation {
        outcome: QueryTerminalOutcome::NoResponse,
        response: execution.response.clone(),
        cache_status: execution.cache_status,
        final_sequence: execution.final_sequence.clone(),
        final_upstream: execution.final_upstream.clone(),
        upstream_attempts: execution.upstream_attempts.clone(),
        failure_provenance: execution.failure_provenance.clone(),
        elapsed: std::time::Duration::ZERO,
    });
    let terminal = if request_shutdown.is_cancelled() {
        QueryTerminalOutcome::Canceled
    } else if matches!(
        execution.response,
        crate::observer::ResponseState::NoResponse
    ) {
        QueryTerminalOutcome::NoResponse
    } else {
        match frame_response(&execution.response_wire, FrameMode::Udp) {
            Ok(framed) => send_response(socket, framed, peer, request_shutdown).await,
            Err(_) => {
                execution.failure_provenance =
                    Some(crate::observer::FailureProvenance::LocalFailure(
                        crate::observer::LocalFailureKind::ResponseConstruction,
                    ));
                QueryTerminalOutcome::SendFailed
            }
        }
    };
    finalize_admitted(admitted, terminal, execution);
}

fn finalize_admitted(
    admitted: AdmittedQueryGuard,
    outcome: QueryTerminalOutcome,
    execution: crate::execution::ExecutionResult,
) {
    admitted.finish(TerminalObservation {
        outcome,
        response: execution.response,
        cache_status: execution.cache_status,
        final_sequence: execution.final_sequence,
        final_upstream: execution.final_upstream,
        upstream_attempts: execution.upstream_attempts,
        failure_provenance: execution.failure_provenance,
        elapsed: std::time::Duration::ZERO,
    });
}

async fn send_response(
    socket: Arc<UdpSocket>,
    response: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
) -> QueryTerminalOutcome {
    send_response_after_gate(
        socket,
        response,
        peer,
        request_shutdown,
        std::future::ready(()),
    )
    .await
}

async fn send_response_after_gate<F>(
    socket: Arc<UdpSocket>,
    response: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
    before_send: F,
) -> QueryTerminalOutcome
where
    F: Future<Output = ()>,
{
    before_send.await;
    send_with_control(&request_shutdown, socket.send_to(&response, peer)).await
}

async fn send_with_control<F>(
    request_shutdown: &TransportCancellation,
    send: F,
) -> QueryTerminalOutcome
where
    F: Future<Output = io::Result<usize>>,
{
    tokio::select! {
        biased;
        () = request_shutdown.cancelled() => QueryTerminalOutcome::Canceled,
        result = send => if result.is_ok() {
            QueryTerminalOutcome::SendSucceeded
        } else {
            QueryTerminalOutcome::SendFailed
        },
    }
}

fn record_task_result(result: Result<(), tokio::task::JoinError>, task_error: &mut Option<String>) {
    if let Err(error) = result {
        if task_error.is_none() {
            *task_error = Some(error.to_string());
        }
    }
}

pub(crate) async fn reap_one_task(
    tasks: &mut JoinSet<()>,
    task_error: &mut Option<String>,
) -> bool {
    let Some(result) = tasks.join_next().await else {
        return false;
    };
    record_task_result(result, task_error);
    true
}

pub(crate) async fn drain_tasks(tasks: &mut JoinSet<()>, task_error: &mut Option<String>) {
    while reap_one_task(tasks, task_error).await {}
}

async fn finish_server(
    forwards: &ForwardCatalog,
    tasks: &mut JoinSet<()>,
    task_error: &mut Option<String>,
    receive_error: Option<std::io::Error>,
) -> Result<(), UdpServerError> {
    drain_tasks(tasks, task_error).await;
    forwards.close_all().await;
    if let Some(error) = task_error {
        return Err(UdpServerError::Task(error.clone()));
    }
    if let Some(error) = receive_error {
        return Err(UdpServerError::Receive(error));
    }
    Ok(())
}

/// Failures that can occur while owning the UDP listener.
#[derive(Debug)]
pub enum UdpServerError {
    WrongListener(ListenerKind),
    Bind(std::io::Error),
    LocalAddr(std::io::Error),
    Receive(std::io::Error),
    Task(String),
}

impl fmt::Display for UdpServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongListener(kind) => write!(formatter, "listener is not UDP: {kind:?}"),
            Self::Bind(error) => write!(formatter, "UDP bind failed: {error}"),
            Self::LocalAddr(error) => write!(formatter, "UDP local address failed: {error}"),
            Self::Receive(error) => write!(formatter, "UDP receive failed: {error}"),
            Self::Task(error) => write!(formatter, "UDP request task failed: {error}"),
        }
    }
}

impl std::error::Error for UdpServerError {}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::observer::{
        CacheStatus, QueryObserver, QueryTerminalOutcome, QueryTransport, ResponseSource,
        ResponseState, TerminalObservation,
    };
    use mosdns_dns_core::{parse_query, synthesize_response, validate_response};
    use mosdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};
    use mosdns_upstream_core::{LifecycleState, TransportCancellation};
    use tokio::net::UdpSocket;
    use tokio::sync::Notify;
    use tokio::task::JoinSet;

    use super::{
        REFUSED, drain_tasks, finish_server, reap_one_task, send_response_after_gate,
        send_with_control,
    };
    use crate::assembly::{HostAssembly, HostRuntime};
    use crate::config::compile_yaml;
    use crate::execution::response_from_state;

    const UDP_CONFIG: &str =
        include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

    #[test]
    fn a_completed_sequence_without_response_maps_to_refused() {
        let config = compile_yaml(UDP_CONFIG).expect("frozen UDP config");
        let query = [
            0x50, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'n',
            b'o', b'p', 0x00, 0x00, 0x01, 0x00, 0x01,
        ];
        let (header, question) = parse_query(&query).expect("valid query");
        let state = ExecutionState::new(header, question.clone());
        let mut machine = config
            .new_machine(state, ExecutionControl::with_fuel(8))
            .expect("compiled sequence");
        let dispatch = match machine.step().expect("forward dispatch") {
            MachineStep::Dispatch(dispatch) => dispatch,
            MachineStep::Complete(_) => panic!("forward must dispatch"),
        };
        let completion = machine
            .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("resume without a response");
        assert!(matches!(completion, MachineStep::Complete(_)));

        let response = response_from_state(&machine, &header, &question);
        validate_response(&response).expect("REFUSED response must be valid");
        assert_eq!(response[3] & 0x0f, REFUSED);
    }

    #[test]
    fn completed_tasks_are_reaped_and_failures_do_not_skip_remaining_tasks() {
        let runtime = HostRuntime::new().expect("test runtime");
        let observed = Arc::new(AtomicBool::new(false));
        runtime.block_on(async {
            let mut tasks = JoinSet::new();
            for _ in 0..128 {
                tasks.spawn_local(async {});
            }
            let mut task_error = None;
            let mut reaped = 0;
            while reap_one_task(&mut tasks, &mut task_error).await {
                reaped += 1;
            }
            assert_eq!(reaped, 128);
            assert!(tasks.is_empty());
            assert!(task_error.is_none());

            let panic_task = tasks.spawn_local(async {
                panic!("intentional task failure");
            });
            let observed_by_task = Arc::clone(&observed);
            tasks.spawn_local(async move {
                observed_by_task.store(true, Ordering::SeqCst);
            });
            let mut task_error = None;
            drain_tasks(&mut tasks, &mut task_error).await;
            assert!(panic_task.is_finished());
            assert!(observed.load(Ordering::SeqCst));
            assert!(task_error.is_some());
            assert!(tasks.is_empty());
        });
    }

    #[test]
    fn task_failure_still_closes_upstream_after_full_drain() {
        let runtime = HostRuntime::new().expect("test runtime");
        let host = HostAssembly::from_yaml(UDP_CONFIG).expect("frozen UDP config");
        runtime.block_on(async {
            let mut tasks = JoinSet::new();
            tasks.spawn_local(async {
                panic!("intentional task failure");
            });
            let mut task_error = None;
            let result =
                finish_server(&host.forwards_handle(), &mut tasks, &mut task_error, None).await;
            assert!(matches!(result, Err(super::UdpServerError::Task(_))));
            assert!(tasks.is_empty());
            assert_eq!(
                host.forward().upstream().lifecycle_state(),
                LifecycleState::Closed
            );
        });
    }

    #[test]
    fn cancellation_wins_at_the_pre_send_commit_gate() {
        let runtime = HostRuntime::new().expect("test runtime");
        runtime.block_on(async {
            let sender = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("sender socket"));
            let receiver = UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("receiver socket");
            let peer = receiver.local_addr().expect("receiver address");
            let cancellation = TransportCancellation::new();
            let observer = Arc::new(QueryObserver::new(
                true,
                ["phase5a_forward".to_owned()],
                std::num::NonZeroUsize::new(2).expect("audit capacity"),
            ));
            let question = mosdns_dns_core::QuestionInfo {
                qname_wire: vec![3, b'n', b'o', b'p', 0],
                qtype: 1,
                qclass: 1,
            };
            let admitted =
                observer.admit(peer, QueryTransport::Udp, &question, cancellation.clone());
            let gate = Arc::new(Notify::new());
            let send = tokio::task::spawn_local(send_response_after_gate(
                sender,
                vec![0xaa, 0xbb],
                peer,
                cancellation.clone(),
                Arc::clone(&gate).notified_owned(),
            ));

            cancellation.cancel();
            gate.notify_one();
            let outcome = send.await.expect("send task");
            assert_eq!(outcome, QueryTerminalOutcome::Canceled);
            admitted.finish(TerminalObservation {
                outcome,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::NotApplicable,
                final_sequence: None,
                final_upstream: None,
                upstream_attempts: Vec::new(),
                failure_provenance: None,
                elapsed: std::time::Duration::ZERO,
            });
            let metrics = observer.metrics_snapshot();
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.canceled_total, 1);
            assert_eq!(metrics.send_succeeded_total, 0);
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(20),
                    receiver.recv_from(&mut [0_u8; 8]),
                )
                .await
                .is_err()
            );
        });
    }

    #[test]
    fn udp_send_failure_and_pending_send_cancellation_are_distinct() {
        let runtime = HostRuntime::new().expect("test runtime");
        runtime.block_on(async {
            let cancellation = TransportCancellation::new();
            let failure = send_with_control(&cancellation, async {
                Err(std::io::Error::other("injected UDP send failure"))
            })
            .await;
            assert_eq!(failure, QueryTerminalOutcome::SendFailed);

            let cancellation = TransportCancellation::new();
            cancellation.cancel();
            let canceled = send_with_control(&cancellation, std::future::pending()).await;
            assert_eq!(canceled, QueryTerminalOutcome::Canceled);
        });
    }

    #[test]
    fn udp_listener_records_one_terminal_audit_for_the_actual_request() {
        let upstream = std::net::UdpSocket::bind("127.0.0.1:0").expect("upstream socket");
        upstream.set_nonblocking(true).expect("nonblocking socket");
        let upstream_addr = upstream.local_addr().expect("upstream address");
        let yaml = UDP_CONFIG
            .replace("127.0.0.1:15453", &upstream_addr.to_string())
            .replace("enable_audit: false", "enable_audit: true");
        let assembly = HostAssembly::from_yaml(&yaml).expect("native host");

        assembly.block_on(async {
            let upstream = UdpSocket::from_std(upstream).expect("Tokio upstream socket");
            let upstream_task = tokio::task::spawn_local(async move {
                let mut request = [0_u8; 2048];
                let (length, client) = upstream
                    .recv_from(&mut request)
                    .await
                    .expect("receive query");
                let (header, question) = parse_query(&request[..length]).expect("upstream query");
                let response = synthesize_response(&header, &question, 0).expect("DNS response");
                upstream
                    .send_to(&response, client)
                    .await
                    .expect("send response");
            });

            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
            let client_addr = client.local_addr().expect("client address");
            let query = [
                0x31, 0x41, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'w',
                b'w', b'w', 0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x00, 0x00, 0x01, 0x00,
                0x01,
            ];
            client
                .send_to(&query, server_addr)
                .await
                .expect("send query");
            let mut response = [0_u8; 2048];
            let (length, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.recv_from(&mut response),
            )
            .await
            .expect("listener response timeout")
            .expect("receive response");
            validate_response(&response[..length]).expect("valid response");

            while assembly.metrics_snapshot().completed_total != 1 {
                tokio::task::yield_now().await;
            }
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.admitted_total, 1);
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.in_flight, 0);
            assert_eq!(metrics.send_succeeded_total, 1);
            assert_eq!(metrics.malformed_total, 0);
            assert_eq!(
                metrics.forward_attempts_by_upstream["phase5a_forward"].responses_total,
                1
            );

            let audit = assembly.audit_snapshot();
            assert_eq!(audit.records.len(), 1);
            let record = &audit.records[0];
            assert_eq!(record.client_addr, client_addr);
            assert_eq!(record.transport, QueryTransport::Udp);
            assert_eq!(record.qname, "www.example.");
            assert_eq!(record.qtype, 1);
            assert_eq!(record.qclass, 1);
            assert_eq!(record.terminal_outcome, QueryTerminalOutcome::SendSucceeded);
            assert_eq!(
                record.response,
                ResponseState::Dns {
                    rcode: 0,
                    source: ResponseSource::Upstream("phase5a_forward".to_owned())
                }
            );
            assert_eq!(record.cache_status, CacheStatus::NotApplicable);
            assert_eq!(record.final_sequence.as_deref(), Some("phase5a_entry"));
            assert_eq!(record.final_upstream.as_deref(), Some("phase5a_forward"));
            assert_eq!(record.upstream_attempts.len(), 1);

            client
                .send_to(&[0_u8, 0_u8], server_addr)
                .await
                .expect("send malformed datagram");
            while assembly.metrics_snapshot().malformed_total != 1 {
                tokio::task::yield_now().await;
            }
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.admitted_total, 1);
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.malformed_total, 1);
            assert_eq!(assembly.audit_snapshot().records.len(), 1);

            shutdown.cancel();
            assert!(server_task.await.expect("server task").is_ok());
            upstream_task.await.expect("upstream task");
        });
    }
}
