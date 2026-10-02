use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{Name, RecordType};
use mosdns_native_host::{HostAssembly, TcpServer, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

fn host(audit: bool) -> HostAssembly {
    host_kind(audit, false)
}
fn host_kind(audit: bool, tcp: bool) -> HostAssembly {
    let yaml = format!(
        r"
log: {{level: error}}
plugins:
  - tag: unused
    type: forward
    args: {{upstreams: [{{addr: udp://127.0.0.1:19902}}]}}
  - tag: networks
    type: ip_set
    args: {{ips: ['127.0.0.1/32', '::1/128']}}
  - tag: main
    type: sequence
    args:
      - matches: client_ip $networks
        exec: reject 3
      - matches: '!client_ip 127.0.0.2/32'
        exec: reject 5
      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {{entry: main, listen: '127.0.0.1:19901', enable_audit: {audit}}}
"
    );
    let yaml = if tcp {
        yaml.replace("type: udp_server", "type: tcp_server")
            .replace("entry: main", "idle_timeout: 2, entry: main")
    } else {
        yaml
    };
    HostAssembly::from_config(compile_yaml(&yaml).expect("client_ip config")).unwrap()
}
fn query() -> Vec<u8> {
    let mut message = Message::new();
    message.set_id(4321).add_query(Query::query(
        Name::from_ascii("client.example.").unwrap(),
        RecordType::A,
    ));
    // Forge an unrelated incoming ECS. It must never become socket identity.
    let mut wire = message.to_vec().unwrap();
    wire[11] = 1;
    wire.extend_from_slice(&[
        0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 11, 0, 8, 0, 7, 0, 1, 24, 0, 203, 0, 113,
    ]);
    wire
}
fn udp(addr: SocketAddr, bind: &str) -> Message {
    let socket = UdpSocket::bind(bind).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket.send_to(&query(), addr).unwrap();
    let mut buf = [0; 4096];
    let n = socket.recv(&mut buf).unwrap();
    Message::from_vec(&buf[..n]).unwrap()
}
fn tcp(addr: SocketAddr) -> Message {
    let mut socket = TcpStream::connect(addr).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let wire = query();
    socket
        .write_all(&u16::try_from(wire.len()).unwrap().to_be_bytes())
        .unwrap();
    socket.write_all(&wire).unwrap();
    let mut length = [0; 2];
    socket.read_exact(&mut length).unwrap();
    let mut wire = vec![0; u16::from_be_bytes(length) as usize];
    socket.read_exact(&mut wire).unwrap();
    Message::from_vec(&wire).unwrap()
}
#[test]
fn real_udp_trusted_peers_forged_ecs_and_audit_off() {
    for audit in [true, false] {
        let host = host(audit);
        let server = host
            .block_on(UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap()))
            .unwrap();
        let addr = server.local_addr().unwrap();
        let stop = TransportCancellation::new();
        host.block_on(async {
            let task = tokio::task::spawn_local(server.serve(stop.clone()));
            for (bind, rcode) in [("127.0.0.1:0", 3), ("127.0.0.2:0", 0), ("127.0.0.3:0", 5)] {
                let response = tokio::task::spawn_blocking(move || udp(addr, bind))
                    .await
                    .unwrap();
                assert_eq!(response.response_code().low(), rcode);
                assert_eq!(response.id(), 4321);
            }
            stop.cancel();
            task.await.unwrap().unwrap();
        });
    }
}
#[test]
fn real_ipv6_udp_and_tcp_context() {
    let host = host(false);
    let stop = TransportCancellation::new();
    let server = host
        .block_on(UdpServer::bind(&host, "[::1]:0".parse().unwrap()))
        .unwrap();
    let addr = server.local_addr().unwrap();
    host.block_on(async {
        let task = tokio::task::spawn_local(server.serve(stop.clone()));
        assert_eq!(
            tokio::task::spawn_blocking(move || udp(addr, "[::1]:0"))
                .await
                .unwrap()
                .response_code()
                .low(),
            3
        );
        stop.cancel();
        task.await.unwrap().unwrap();
    });
    for audit in [true, false] {
        let host = host_kind(audit, true);
        let stop = TransportCancellation::new();
        let server = host
            .block_on(TcpServer::bind(&host, "[::]:0".parse().unwrap()))
            .unwrap();
        let port = server.local_addr().unwrap().port();
        host.block_on(async {
            let task = tokio::task::spawn_local(server.serve(stop.clone()));
            for addr in [
                SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
                SocketAddr::from(([127, 0, 0, 1], port)),
            ] {
                assert_eq!(
                    tokio::task::spawn_blocking(move || tcp(addr))
                        .await
                        .unwrap()
                        .response_code()
                        .low(),
                    3
                );
            }
            stop.cancel();
            task.await.unwrap().unwrap();
        });
    }
}

#[test]
fn unknown_and_mapped_embedding_identity_with_file_or_and_negation() {
    use mosdns_sequence_core::{
        ClientContext, ClientTransport, ExecutionControl, ExecutionState, MachineStep,
        ResponseState,
    };
    let root = std::env::temp_dir().join(format!("ecs-client-context-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("clients.txt"), "192.0.2.0/24 trailing comment\n").unwrap();
    let yaml = r"
log: {level: error}
plugins:
  - tag: unused
    type: forward
    args: {upstreams: [{addr: udp://127.0.0.1:19902}]}
  - tag: v6
    type: ip_set
    args: {ips: ['2001:db8::/32']}
  - tag: main
    type: sequence
    args:
      - matches: client_ip $v6 &clients.txt 198.51.100.1
        exec: reject 3
      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {entry: main, listen: '127.0.0.1:19901', enable_audit: false}
";
    let config = mosdns_native_host::compile_yaml_with_base(yaml, &root).unwrap();
    std::fs::remove_file(root.join("clients.txt")).unwrap();
    for (client, rcode) in [
        (ClientContext::default(), 0),
        (
            ClientContext::from_peer("::ffff:192.0.2.9".parse().unwrap(), ClientTransport::Tcp),
            3,
        ),
        (
            ClientContext::from_peer("2001:db8::9".parse().unwrap(), ClientTransport::Udp),
            3,
        ),
        (
            ClientContext::from_peer("198.51.100.1".parse().unwrap(), ClientTransport::Udp),
            3,
        ),
        (
            ClientContext::from_peer("203.0.113.9".parse().unwrap(), ClientTransport::Udp),
            0,
        ),
    ] {
        let (header, question) = mosdns_dns_core::parse_query(&query()).unwrap();
        let mut state = ExecutionState::new(header, question);
        state.query.client = client;
        let mut machine = config
            .new_machine(state, ExecutionControl::with_fuel(64))
            .unwrap();
        assert!(matches!(machine.step().unwrap(), MachineStep::Complete(_)));
        let ResponseState::Synthesized(response) = &machine.state().response else {
            panic!("reject response")
        };
        assert_eq!(response.rcode(), rcode);
        assert_eq!(machine.state().query.client, client);
        assert_eq!(machine.state().snapshot().query.client, client);
    }
    assert_eq!(
        ClientContext::from_peer("::ffff:192.0.2.9".parse().unwrap(), ClientTransport::Tcp)
            .peer_ip(),
        Some("192.0.2.9".parse().unwrap())
    );
    for expression in [
        "client_ip",
        "client_ip ::1/129",
        "client_ip $missing",
        "client_ip &",
    ] {
        assert!(
            compile_yaml(&yaml.replace("client_ip $v6 &clients.txt 198.51.100.1", expression))
                .is_err(),
            "{expression}"
        );
    }
    std::fs::remove_dir(root).unwrap();
}
