use std::fmt;
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

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
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
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                return Err(UdpServerError::Task(error.to_string()));
            }
        }
        let _ = self.forward.upstream().close().await;
        if let Some(error) = receive_error {
            return Err(UdpServerError::Receive(error));
        }
        Ok(())
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
    let _ = socket.send_to(&framed, peer).await;
}

async fn execute_request(
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
    use mosdns_dns_core::{parse_query, validate_response};
    use mosdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};

    use super::{REFUSED, response_from_state};
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
}
