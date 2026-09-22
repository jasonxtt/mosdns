use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use mosdns_dns_core::{
    FrameMode, QueryHeader, QuestionInfo, frame_response, inspect_response_header,
    patch_response_id_ra, synthesize_response, validate_response,
};
use mosdns_sequence_core::{
    ExecutionControl, ExecutionMachine, ExecutionState, ExecutorOutcome, MachineStep, ResponseState,
};
use mosdns_upstream_core::TransportCancellation;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;

use crate::assembly::{ForwardAdapter, HostAssembly, HostOptions};
use crate::config::{CompiledConfig, ListenerKind};

const MAX_UDP_PACKET: usize = 65535;
const DEFAULT_FUEL: u64 = 64;
const SERVFAIL: u8 = 2;
const REFUSED: u8 = 5;

/// A local UDP listener owned by the native host. Its request tasks run on the
/// host's current-thread local task set because sequence executors are not
/// widened to `Send + Sync` by this migration slice.
pub struct UdpServer {
    config: Rc<CompiledConfig>,
    forward: Rc<ForwardAdapter>,
    options: HostOptions,
    socket: Arc<UdpSocket>,
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
            forward: assembly.forward_handle(),
            options: assembly.options().clone(),
            socket: Arc::new(socket),
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
                            let forward = Rc::clone(&self.forward);
                            let options = self.options.clone();
                            let request_shutdown = shutdown.child_token();
                            tasks.spawn_local(async move {
                                process_request(
                                    socket,
                                    config,
                                    forward,
                                    options,
                                    raw,
                                    peer,
                                    request_shutdown,
                                )
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
        finish_server(&self.forward, &mut tasks, &mut task_error, receive_error).await
    }
}

async fn process_request(
    socket: Arc<UdpSocket>,
    config: Rc<CompiledConfig>,
    forward: Rc<ForwardAdapter>,
    options: HostOptions,
    raw: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
) {
    let Ok((header, question)) = mosdns_dns_core::parse_query(&raw) else {
        // Malformed UDP input is isolated to this datagram and produces no
        // response, matching the frozen server behavior.
        return;
    };

    let response = execute_request(
        &config,
        &forward,
        &options,
        &raw,
        header,
        question,
        request_shutdown.clone(),
    )
    .await;
    if request_shutdown.is_cancelled() {
        return;
    }
    let Ok(framed) = frame_response(&response, FrameMode::Udp) else {
        return;
    };
    let _ = send_response(socket, framed, peer, request_shutdown).await;
}

async fn send_response(
    socket: Arc<UdpSocket>,
    response: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
) -> bool {
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
) -> bool
where
    F: Future<Output = ()>,
{
    before_send.await;
    tokio::select! {
        biased;
        () = request_shutdown.cancelled() => false,
        result = socket.send_to(&response, peer) => result.is_ok(),
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
    forward: &ForwardAdapter,
    tasks: &mut JoinSet<()>,
    task_error: &mut Option<String>,
    receive_error: Option<std::io::Error>,
) -> Result<(), UdpServerError> {
    drain_tasks(tasks, task_error).await;
    let _ = forward.upstream().close().await;
    if let Some(error) = task_error {
        return Err(UdpServerError::Task(error.clone()));
    }
    if let Some(error) = receive_error {
        return Err(UdpServerError::Receive(error));
    }
    Ok(())
}

pub(crate) async fn execute_request(
    config: &CompiledConfig,
    forward: &ForwardAdapter,
    options: &HostOptions,
    raw: &[u8],
    header: QueryHeader,
    question: QuestionInfo,
    request_shutdown: TransportCancellation,
) -> Vec<u8> {
    let state = ExecutionState::new(header, question.clone());
    let mut machine = match config.new_machine(state, ExecutionControl::with_fuel(DEFAULT_FUEL)) {
        Ok(machine) => machine,
        Err(_) => return protocol_error(&header, &question, SERVFAIL),
    };

    let mut step = match machine.step() {
        Ok(step) => step,
        Err(_) => return protocol_error(&header, &question, SERVFAIL),
    };
    loop {
        match step {
            MachineStep::Complete(_) => return response_from_state(&machine, &header, &question),
            MachineStep::Dispatch(dispatch) => {
                if dispatch.executable() != config.forward.executable {
                    return protocol_error(&header, &question, SERVFAIL);
                }
                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                let deadline = Instant::now() + options.request_deadline;
                let exchange = forward
                    .exchange(raw, deadline, request_shutdown.clone())
                    .await;
                if request_shutdown.is_cancelled() {
                    return Vec::new();
                }
                match exchange {
                    Ok(response) => {
                        let accepted = patch_response_id_ra(response.wire(), header.id)
                            .ok()
                            .filter(|wire| {
                                inspect_response_header(wire).is_ok()
                                    && validate_response(wire).is_ok()
                            });
                        match accepted {
                            Some(wire) => machine.state_mut().set_raw_response(wire),
                            None => set_servfail(&mut machine),
                        }
                    }
                    Err(_) => set_servfail(&mut machine),
                }
                step = match machine.resume(dispatch.executable(), Ok(ExecutorOutcome::Continue)) {
                    Ok(step) => step,
                    Err(_) => return response_from_state(&machine, &header, &question),
                };
            }
        }
    }
}

fn set_servfail(machine: &mut ExecutionMachine<'_>) {
    let _ = machine
        .state_mut()
        .set_synthesized_response(u16::from(SERVFAIL));
}

fn response_from_state(
    machine: &ExecutionMachine<'_>,
    header: &QueryHeader,
    question: &QuestionInfo,
) -> Vec<u8> {
    match &machine.state().response {
        ResponseState::Raw(wire) => wire.0.clone(),
        ResponseState::Synthesized(response) => {
            let rcode = u8::try_from(response.rcode()).unwrap_or(SERVFAIL);
            protocol_error(header, question, rcode)
        }
        ResponseState::None => protocol_error(header, question, REFUSED),
    }
}

fn protocol_error(header: &QueryHeader, question: &QuestionInfo, rcode: u8) -> Vec<u8> {
    synthesize_response(header, question, rcode).unwrap_or_else(|_| {
        synthesize_response(header, question, SERVFAIL).unwrap_or_else(|_| Vec::new())
    })
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

    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};
    use mosdns_upstream_core::{LifecycleState, TransportCancellation};
    use tokio::net::UdpSocket;
    use tokio::sync::Notify;
    use tokio::task::JoinSet;

    use super::{
        REFUSED, drain_tasks, finish_server, reap_one_task, response_from_state,
        send_response_after_gate,
    };
    use crate::assembly::{HostAssembly, HostRuntime};
    use crate::config::compile_yaml;

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
            let result = finish_server(host.forward(), &mut tasks, &mut task_error, None).await;
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
            assert!(!send.await.expect("send task"));
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
}
