use hickory_proto::op::{Message, MessageType, Query};
use hickory_proto::rr::{Name, RData, Record, RecordType, rdata::A};
use mosdns_native_host::{HostAssembly, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use std::net::UdpSocket;
use std::time::Duration;

fn query(ecs: Option<&[u8]>, opt: bool) -> Vec<u8> {
    let mut message = Message::new();
    message
        .set_id(45)
        .set_recursion_desired(true)
        .add_query(Query::query(
            Name::from_ascii("ecs.example.").unwrap(),
            RecordType::A,
        ));
    let mut wire = message.to_vec().unwrap();
    if opt {
        let mut body = vec![0, 10, 0, 2, 5, 6]; // permitted non-ECS option
        if let Some(ecs) = ecs {
            body.extend_from_slice(&[0, 8]);
            body.extend_from_slice(&u16::try_from(ecs.len()).unwrap().to_be_bytes());
            body.extend_from_slice(ecs);
        }
        wire[11] = 1;
        wire.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 128, 0]);
        wire.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        wire.extend_from_slice(&body);
    }
    wire
}
fn config(args: &str, quick: Option<&str>, peer_addr: &str) -> String {
    let declaration = if quick.is_none() {
        format!("  - tag: ecs\n    type: ecs_handler\n    args: {args}\n")
    } else {
        String::new()
    };
    let exec = quick.unwrap_or("$ecs");
    format!(
        "log: {{level: error}}\nplugins:\n{declaration}  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: udp://{peer_addr}}}]}}\n  - tag: main\n    type: sequence\n    args:\n      - exec: {exec}\n      - exec: $upstream\n  - tag: listener\n    type: udp_server\n    args: {{entry: main, listen: '127.0.0.1:19903', enable_audit: false}}\n"
    )
}
fn run(args: &str, quick: Option<&str>, raw: Vec<u8>) -> Vec<u8> {
    let (response, wire) = run_response(args, quick, raw);
    assert_eq!(response.answers()[0].data().to_string(), "192.0.2.1");
    wire.expect("upstream request")
}
fn run_response(args: &str, quick: Option<&str>, raw: Vec<u8>) -> (Message, Option<Vec<u8>>) {
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let peer_addr = peer.local_addr().unwrap();
    let peer_task = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        let (len, addr) = match peer.recv_from(&mut buf) {
            Ok(result) => result,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                return None;
            }
            Err(error) => panic!("{error}"),
        };
        let request = Message::from_vec(&buf[..len]).unwrap();
        let mut response = Message::new();
        response
            .set_id(request.id())
            .set_message_type(MessageType::Response)
            .add_queries(request.queries().to_vec())
            .add_answer(Record::from_rdata(
                request.queries()[0].name().clone(),
                60,
                RData::A(A("192.0.2.1".parse().unwrap())),
            ));
        peer.send_to(&response.to_vec().unwrap(), addr).unwrap();
        Some(buf[..len].to_vec())
    });
    let yaml = config(args, quick, &peer_addr.to_string());
    let host = HostAssembly::from_config(compile_yaml(&yaml).expect("ECS config")).unwrap();
    let server = host
        .block_on(UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = server.local_addr().unwrap();
    let stop = TransportCancellation::new();
    let response = host.block_on(async {
        let task = tokio::task::spawn_local(server.serve(stop.clone()));
        let response = tokio::task::spawn_blocking(move || {
            let client = UdpSocket::bind("127.0.0.1:0").unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client.send_to(&raw, address).unwrap();
            let mut buf = [0; 4096];
            let len = client.recv(&mut buf).unwrap();
            Message::from_vec(&buf[..len]).unwrap()
        })
        .await
        .unwrap();
        assert_eq!(response.id(), 45);
        stop.cancel();
        task.await.unwrap().unwrap();
        response
    });
    (response, peer_task.join().unwrap())
}
fn ecs(wire: &[u8]) -> Option<mosdns_dns_core::EcsInfo> {
    let (_, q) = mosdns_dns_core::parse_query(wire).unwrap();
    let offset = 12 + q.qname_wire.len() + 4;
    if wire[11] == 0 {
        None
    } else {
        mosdns_dns_core::extract_edns_at(wire, offset)
            .unwrap()
            .unwrap()
            .ecs
    }
}
#[test]
fn named_handler_selection_and_masked_wire() {
    let incoming = [0, 1, 24, 0, 203, 0, 113];
    for (args, expected) in [
        ("{}", None),
        (
            "{forward: true, preset: '192.0.2.199', send: true}",
            Some((1, 24, vec![203, 0, 113, 0])),
        ),
        (
            "{preset: '192.0.2.199', send: true}",
            Some((1, 24, vec![192, 0, 2, 0])),
        ),
        ("{send: true}", Some((1, 24, vec![127, 0, 0, 0]))),
        (
            "{preset: '2001:db8:1234:5678::1'}",
            Some((
                2,
                48,
                vec![
                    0x20, 1, 0x0d, 0xb8, 0x12, 0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
            )),
        ),
        (
            "{preset: '::ffff:192.0.2.199', mask4: 32}",
            Some((1, 32, vec![192, 0, 2, 199])),
        ),
        (
            "{preset: '192.0.2.199', mask4: 0}",
            Some((1, 24, vec![192, 0, 2, 0])),
        ),
    ] {
        let wire = run(args, None, query(Some(&incoming), true));
        let actual = ecs(&wire);
        assert_eq!(
            actual.map(|e| (e.family, e.source_netmask, e.address)),
            expected,
            "{args}"
        );
        assert_eq!(wire[7], 0);
        assert_eq!(wire[9], 0);
    }
    let wire = run("{send: true}", None, query(None, false));
    assert_eq!(ecs(&wire).unwrap().address, vec![127, 0, 0, 0]);
    assert_eq!(
        mosdns_dns_core::extract_edns_at(
            &wire,
            12 + mosdns_dns_core::parse_query(&wire)
                .unwrap()
                .1
                .qname_wire
                .len()
                + 4
        )
        .unwrap()
        .unwrap()
        .udp_size,
        1232
    );
    let wire = run("{}", Some("ecs 192.0.2.199/8 ignored"), query(None, false));
    assert_eq!(ecs(&wire).unwrap().source_netmask, 24);
}

#[test]
fn current_policy_ecs_survives_later_handler() {
    let wire = run(
        "{}",
        Some("ecs 192.0.2.199\n      - exec: ecs 203.0.113.55"),
        query(None, false),
    );
    assert_eq!(ecs(&wire).unwrap().address, vec![192, 0, 2, 0]);
}

#[test]
fn handler_configuration_validation() {
    for args in [
        "{mask4: -1}",
        "{mask4: 33}",
        "{mask6: 129}",
        "{mask6: -1}",
        "{mask4: true}",
        "{forward: 1}",
        "{send: 'yes'}",
        "{preset: '192.0.2.1/24'}",
        "{preset: 'bad'}",
        "{extra: 1}",
    ] {
        assert!(
            compile_yaml(&config(args, None, "127.0.0.1:19905")).is_err(),
            "{args}"
        );
    }
    for args in [
        "{}",
        "null",
        "{mask4: 32, mask6: 128}",
        "{mask4: 0, mask6: 0}",
        "{forward: false, send: false, preset: ''}",
    ] {
        assert!(
            compile_yaml(&config(args, None, "127.0.0.1:19905")).is_ok(),
            "{args}"
        );
    }
    assert!(compile_yaml(&config("{}", Some("ecs invalid"), "127.0.0.1:19905")).is_err());
    assert!(compile_yaml(&config("{}", Some("ecs"), "127.0.0.1:19905")).is_ok());
}
#[test]
fn handler_preserves_other_opt_flags_and_empty_legacy_and_non_in() {
    let incoming = [0, 1, 24, 0, 203, 0, 113];
    let original = query(Some(&incoming), true);
    let wire = run("{preset: '192.0.2.199', mask4: 25}", None, original.clone());
    let (_, question) = mosdns_dns_core::parse_query(&wire).unwrap();
    let offset = 12 + question.qname_wire.len() + 4;
    assert_eq!(&wire[..offset], &original[..offset]);
    assert_eq!(&wire[offset..offset + 9], &original[offset..offset + 9]);
    assert_eq!(&wire[offset + 11..offset + 17], &[0, 10, 0, 2, 5, 6]);
    assert_eq!(ecs(&wire).unwrap().address, vec![192, 0, 2, 128]);
    assert_eq!(run("{}", Some("ecs"), original.clone()), original);
    let mut non_in = original;
    non_in[offset - 2..offset].copy_from_slice(&3u16.to_be_bytes());
    assert_eq!(run("{send: true}", None, non_in.clone()), non_in);
}
#[test]
fn malformed_incoming_ecs_fails_locally_without_forwarding() {
    for data in [
        vec![0, 0, 0, 0],
        vec![0, 1, 24, 1, 203, 0, 113],
        vec![0, 1, 25, 0, 203, 0, 113, 1],
        vec![0, 1, 32, 0, 203],
        vec![0, 3, 0, 0],
    ] {
        let (response, wire) =
            run_response("{preset: '192.0.2.1'}", None, query(Some(&data), true));
        assert_eq!(response.response_code().low(), 2);
        assert!(wire.is_none());
    }
    let mut duplicate = query(Some(&[0, 1, 24, 0, 203, 0, 113]), true);
    let (_, q) = mosdns_dns_core::parse_query(&duplicate).unwrap();
    let offset = 12 + q.qname_wire.len() + 4;
    let old = u16::from_be_bytes([duplicate[offset + 9], duplicate[offset + 10]]);
    duplicate[offset + 9..offset + 11].copy_from_slice(&(old + 11).to_be_bytes());
    duplicate.extend_from_slice(&[0, 8, 0, 7, 0, 1, 24, 0, 203, 0, 113]);
    let (response, wire) = run_response("{forward: true}", None, duplicate);
    assert_eq!(response.response_code().low(), 2);
    assert!(wire.is_none());
}
