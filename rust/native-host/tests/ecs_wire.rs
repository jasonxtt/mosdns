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
    run_with_supplier(args, quick, raw, None)
}
fn run_with_supplier(
    args: &str,
    quick: Option<&str>,
    raw: Vec<u8>,
    supplier_options: Option<Vec<u8>>,
) -> (Message, Option<Vec<u8>>) {
    run_scenario(
        args,
        quick,
        raw,
        supplier_options,
        Duration::ZERO,
        1,
        |yaml| yaml,
    )
}
fn run_scenario(
    args: &str,
    quick: Option<&str>,
    raw: Vec<u8>,
    supplier_options: Option<Vec<u8>>,
    delay: Duration,
    count: usize,
    rewrite: impl FnOnce(String) -> String,
) -> (Message, Option<Vec<u8>>) {
    run_scenario_requests(
        args,
        quick,
        raw,
        supplier_options,
        delay,
        (count, 1),
        rewrite,
    )
}
fn run_scenario_requests(
    args: &str,
    quick: Option<&str>,
    raw: Vec<u8>,
    supplier_options: Option<Vec<u8>>,
    delay: Duration,
    counts: (usize, usize),
    rewrite: impl FnOnce(String) -> String,
) -> (Message, Option<Vec<u8>>) {
    let (count, requests) = counts;
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let peer_addr = peer.local_addr().unwrap();
    let peer_task = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        let mut captured = None;
        for _ in 0..count {
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
            let mut response_wire = response.to_vec().unwrap();
            if let Some(options) = &supplier_options {
                response_wire[11] = 1;
                response_wire.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 128, 0]);
                response_wire
                    .extend_from_slice(&u16::try_from(options.len()).unwrap().to_be_bytes());
                response_wire.extend_from_slice(options);
            }
            std::thread::sleep(delay);
            peer.send_to(&response_wire, addr).unwrap();
            captured = Some(buf[..len].to_vec());
        }
        captured
    });
    let yaml = rewrite(config(args, quick, &peer_addr.to_string()));
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
            let mut result = None;
            for _ in 0..requests {
                client.send_to(&raw, address).unwrap();
                let mut buf = [0; 4096];
                let len = client.recv(&mut buf).unwrap();
                result = Some(Message::from_vec(&buf[..len]).unwrap());
            }
            result.unwrap()
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

fn supplier_ecs(data: &[u8]) -> Vec<u8> {
    let mut options = vec![0, 10, 0, 2, 7, 8, 0, 8];
    options.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
    options.extend_from_slice(data);
    options
}
#[test]
fn generated_policy_cannot_echo_supplier_ecs() {
    let (response, _) = run_with_supplier(
        "{preset: '203.0.113.9'}",
        None,
        query(None, true),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
    );
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(hickory_proto::rr::rdata::opt::EdnsCode::Subnet)
            .is_none()
    );
    assert_eq!(response.answers()[0].data().to_string(), "192.0.2.1");
}

#[test]
fn only_legal_matching_supplier_ecs_can_echo() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let incoming = [0, 1, 24, 0, 203, 0, 113];
    for (payload, allowed) in [
        (vec![0, 1, 24, 20, 203, 0, 113], true),
        (vec![0, 1, 24, 0, 203, 0, 113], true),
        (vec![0, 1, 24, 25, 203, 0, 113], false),
        (vec![0, 1, 24, 20, 192, 0, 2], false),
        (vec![0, 1, 16, 16, 203, 0], false),
        (vec![0, 2, 24, 0, 203, 0, 113], false),
        (vec![0, 0, 0, 0], false),
        (vec![0, 1, 32, 0, 203], false),
        (vec![0, 1, 25, 0, 203, 0, 113, 1], false),
    ] {
        let (response, _) = run_with_supplier(
            "{forward: true}",
            None,
            query(Some(&incoming), true),
            Some(supplier_ecs(&payload)),
        );
        let opt = response.extensions().as_ref().unwrap();
        assert_eq!(
            opt.options().get(EdnsCode::Subnet).is_some(),
            allowed,
            "{payload:?}"
        );
        assert!(opt.flags().dnssec_ok);
        assert_eq!(opt.max_payload(), 1232);
        assert_eq!(response.id(), 45);
        assert_eq!(response.queries()[0].name().to_ascii(), "ecs.example.");
        assert_eq!(response.answers()[0].data().to_string(), "192.0.2.1");
        assert!(opt.options().get(EdnsCode::from(10)).is_some());
    }
    let mut duplicate = supplier_ecs(&[0, 1, 24, 20, 203, 0, 113]);
    duplicate.extend_from_slice(&supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])[6..]);
    let (response, _) = run_with_supplier(
        "{forward: true}",
        None,
        query(Some(&incoming), true),
        Some(duplicate),
    );
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_none()
    );
    let (response, _) =
        run_with_supplier("{forward: true}", None, query(Some(&incoming), true), None);
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_none()
    );
    let (response, _) = run_with_supplier(
        "{preset: '203.0.113.9'}",
        None,
        query(None, false),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
    );
    assert!(response.extensions().is_none());
}

#[test]
fn nested_redirect_and_handler_inherit_echo_permission() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let (response, wire) = run_scenario(
        "{forward: true}",
        None,
        query(Some(&[0, 1, 24, 0, 203, 0, 113]), true),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
        Duration::ZERO,
        1,
        |yaml| {
            yaml.replace("  - tag: main", "  - tag: nested\n    type: ecs_handler\n    args: {preset: '192.0.2.55'}\n  - tag: rewrite\n    type: redirect\n    args: {rules: ['ecs.example target.example']}\n  - tag: main").replace("      - exec: $upstream", "      - exec: $nested\n      - exec: $rewrite\n      - exec: $upstream")
        },
    );
    assert_eq!(ecs(&wire.unwrap()).unwrap().address, vec![203, 0, 113, 0]);
    assert_eq!(response.queries()[0].name().to_ascii(), "ecs.example.");
    assert_eq!(response.answers()[0].record_type(), RecordType::CNAME);
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_some()
    );
}
#[test]
fn preference_local_suppression_has_no_supplier_echo() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let mut raw = query(Some(&[0, 1, 24, 0, 203, 0, 113]), true);
    let offset = 12
        + mosdns_dns_core::parse_query(&raw)
            .unwrap()
            .1
            .qname_wire
            .len();
    raw[offset..offset + 2].copy_from_slice(&28u16.to_be_bytes());
    let (response, _) = run_scenario(
        "{forward: true}",
        None,
        raw,
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
        Duration::ZERO,
        2,
        |yaml| {
            yaml.replace(
                "      - exec: $upstream",
                "      - exec: prefer_ipv4\n      - exec: $upstream",
            )
        },
    );
    assert!(response.answers().is_empty());
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_none()
    );
}

#[test]
fn cache_hit_reconstructs_client_opt_without_inventing_ecs() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let (response, _) = run_scenario_requests(
        "{}",
        None,
        query(Some(&[0, 1, 24, 0, 203, 0, 113]), true),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
        Duration::ZERO,
        (1, 2),
        |yaml| {
            yaml.replace(
                "  - tag: main",
                "  - tag: cache\n    type: cache\n    args: {}\n  - tag: main",
            )
            .replace(
                "      - exec: $upstream",
                "      - exec: $cache\n      - exec: $upstream",
            )
        },
    );
    assert_eq!(response.answers()[0].data().to_string(), "192.0.2.1");
    let opt = response.extensions().as_ref().unwrap();
    assert!(opt.flags().dnssec_ok);
    assert!(opt.options().get(EdnsCode::Subnet).is_none());
}
#[test]
fn fallback_echo_uses_only_winning_supplier() {
    use hickory_proto::rr::rdata::opt::{EdnsCode, EdnsOption};
    let backup = UdpSocket::bind("127.0.0.1:0").unwrap();
    backup
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let addr = backup.local_addr().unwrap();
    let secondary = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        let (len, client) = backup.recv_from(&mut buf).unwrap();
        let query = Message::from_vec(&buf[..len]).unwrap();
        assert_eq!(ecs(&buf[..len]).unwrap().address, vec![203, 0, 113, 0]);
        let mut answer = Message::new();
        answer
            .set_id(query.id())
            .set_message_type(MessageType::Response)
            .add_queries(query.queries().to_vec())
            .add_answer(Record::from_rdata(
                query.queries()[0].name().clone(),
                60,
                RData::A(A("192.0.2.2".parse().unwrap())),
            ));
        answer
            .extensions_mut()
            .get_or_insert_with(hickory_proto::op::Edns::new)
            .options_mut()
            .insert(EdnsOption::Unknown(8, vec![0, 1, 24, 12, 203, 0, 113]));
        backup.send_to(&answer.to_vec().unwrap(), client).unwrap();
    });
    let (response, _) = run_scenario(
        "{forward: true}",
        None,
        query(Some(&[0, 1, 24, 0, 203, 0, 113]), true),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
        Duration::from_millis(150),
        1,
        |yaml| {
            yaml.replace("  - tag: main", &format!("  - tag: backup\n    type: forward\n    args: {{upstreams: [{{addr: udp://{addr}}}]}}\n  - tag: choice\n    type: fallback\n    args: {{primary: '$upstream', secondary: '$backup', threshold: 1, always_standby: true}}\n  - tag: main")).replace("      - exec: $upstream", "      - exec: $choice")
        },
    );
    secondary.join().unwrap();
    assert_eq!(response.answers()[0].data().to_string(), "192.0.2.2");
    let EdnsOption::Subnet(subnet) = response
        .extensions()
        .as_ref()
        .unwrap()
        .options()
        .get(EdnsCode::Subnet)
        .unwrap()
    else {
        panic!("ECS");
    };
    assert_eq!(subnet.scope_prefix(), 12);
}

#[test]
fn ecs_scope_preserves_exit_and_local_terminal_completion() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let (response, _) = run_scenario(
        "{forward: true}",
        None,
        query(Some(&[0, 1, 24, 0, 203, 0, 113]), true),
        Some(supplier_ecs(&[0, 1, 24, 20, 203, 0, 113])),
        Duration::ZERO,
        1,
        |yaml| {
            yaml.replace(
                "      - exec: $upstream",
                "      - exec: $upstream\n      - exec: exit",
            )
        },
    );
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_some()
    );
    let (response, _) = run_scenario(
        "{forward: true}",
        None,
        query(Some(&[0, 1, 24, 0, 203, 0, 113]), true),
        None,
        Duration::ZERO,
        0,
        |yaml| {
            yaml.replace(
                "      - exec: $upstream",
                "      - exec: reject 2\n      - exec: $upstream",
            )
        },
    );
    assert_eq!(u16::from(response.response_code()), 2);
    assert!(
        response
            .extensions()
            .as_ref()
            .unwrap()
            .options()
            .get(EdnsCode::Subnet)
            .is_none()
    );
}

#[test]
fn ecs_scope_shutdown_cancels_without_fabricated_response() {
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let yaml = config(
        "{forward: true}",
        None,
        &peer.local_addr().unwrap().to_string(),
    )
    .replace("enable_audit: false", "enable_audit: true");
    let (observed, received) = std::sync::mpsc::channel();
    let upstream = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        let (len, _) = peer.recv_from(&mut buf).unwrap();
        assert_eq!(ecs(&buf[..len]).unwrap().address, vec![203, 0, 113, 0]);
        observed.send(()).unwrap();
    });
    let host = HostAssembly::from_config(compile_yaml(&yaml).unwrap()).unwrap();
    let server = host
        .block_on(UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = server.local_addr().unwrap();
    let stop = TransportCancellation::new();
    host.block_on(async {
        let serving = tokio::task::spawn_local(server.serve(stop.clone()));
        let client = tokio::task::spawn_blocking(move || {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            socket
                .send_to(&query(Some(&[0, 1, 24, 0, 203, 0, 113]), true), address)
                .unwrap();
            let mut buf = [0; 4096];
            assert!(socket.recv(&mut buf).is_err());
        });
        tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(2)).unwrap())
            .await
            .unwrap();
        stop.cancel();
        serving.await.unwrap().unwrap();
        client.await.unwrap();
    });
    upstream.join().unwrap();
    assert_eq!(
        host.audit_snapshot().records[0].response,
        mosdns_native_host::ResponseState::NoResponse
    );
}

#[test]
fn forwarded_ipv6_supplier_has_legal_echo() {
    use hickory_proto::rr::rdata::opt::{EdnsCode, EdnsOption};
    let incoming = [0, 2, 48, 0, 0x20, 1, 0x0d, 0xb8, 0x12, 0x34];
    let actual = [0, 2, 48, 32, 0x20, 1, 0x0d, 0xb8, 0x12, 0x34];
    let (response, _) = run_with_supplier(
        "{forward: true}",
        None,
        query(Some(&incoming), true),
        Some(supplier_ecs(&actual)),
    );
    let EdnsOption::Subnet(subnet) = response
        .extensions()
        .as_ref()
        .unwrap()
        .options()
        .get(EdnsCode::Subnet)
        .unwrap()
    else {
        panic!("ECS");
    };
    assert_eq!(subnet.scope_prefix(), 32);
    assert_eq!(subnet.source_prefix(), 48);
}
