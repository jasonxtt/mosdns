use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{
    QuestionInfo, inspect_response_header, observe_answer_addresses, observe_response_metadata,
    parse_query, validate_response,
};
use mosdns_native_host::{HostAssembly, HostOptions, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use serde::Deserialize;

const ROUTING_CONFIG: &str = include_str!("../../../tests/phase5a-baseline/configs/routing.yaml");
const ROUTING_WORKLOAD: &str =
    include_str!("../../../tests/phase5a-baseline/workloads/routing.jsonl");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct RoutingCase {
    case_id: String,
    scenario: String,
    transport: String,
    qname: String,
    qtype: String,
    expected_rcode: u8,
    expected_answer_class: String,
    expected_answer: String,
    expected_route_class: String,
    request_deadline_ms: u64,
    weight: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteEvent {
    id: u16,
    route: &'static str,
    qname: Vec<u8>,
}

struct RouteFixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RouteFixture {
    fn start(route: &'static str, events: &Arc<Mutex<Vec<RouteEvent>>>) -> Self {
        Self::start_with_delay(route, events, Duration::ZERO)
    }

    fn start_with_delay(
        route: &'static str,
        events: &Arc<Mutex<Vec<RouteEvent>>>,
        response_delay: Duration,
    ) -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").expect("route fixture bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("route fixture timeout");
        let address = socket.local_addr().expect("route fixture address");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_events = Arc::clone(events);
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                let query = &packet[..length];
                let Ok((header, question)) = parse_query(query) else {
                    continue;
                };
                thread_events
                    .lock()
                    .expect("route event lock")
                    .push(RouteEvent {
                        id: header.id,
                        route,
                        qname: question.qname_wire.clone(),
                    });
                if !response_delay.is_zero() {
                    thread::sleep(response_delay);
                }
                if let Some(response) = response_for(route, query, &question) {
                    socket
                        .send_to(&response, peer)
                        .expect("route fixture response");
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("route fixture thread");
        }
    }
}

fn response_for(route: &'static str, query: &[u8], question: &QuestionInfo) -> Option<Vec<u8>> {
    if route == "b" && question.qname_wire == qname_wire("malformed.test.") {
        return Some(vec![1, 2, 3]);
    }
    if route == "b" && question.qname_wire == qname_wire("mismatch.test.") {
        let mut wrong_question = question.clone();
        wrong_question.qname_wire = qname_wire("other.test.");
        return Some(response_with_rcode(
            query,
            &wrong_question,
            0,
            Some([192, 0, 2, 30]),
        ));
    }
    if route == "b" && question.qname_wire == qname_wire("b-fail.test.") {
        return None;
    }
    if route == "a" && question.qname_wire == qname_wire("a-fail.test.") {
        return None;
    }
    if route == "c" && question.qname_wire == qname_wire("c-fail.test.") {
        return None;
    }
    if question.qtype != 1 || question.qclass != 1 {
        return Some(response_with_rcode(query, question, 3, None));
    }
    let negative = route == "b" && question.qname_wire == qname_wire("negative.test.");
    if negative {
        return Some(response_with_rcode(query, question, 3, None));
    }
    let address = match route {
        "a" => [192, 0, 2, 11],
        "b" if question.qname_wire == qname_wire("ip-hit.test.")
            || question.qname_wire == qname_wire("a-fail.test.") =>
        {
            [192, 0, 2, 10]
        }
        "b" => [192, 0, 2, 30],
        "c" => [192, 0, 2, 12],
        _ => return None,
    };
    Some(response_with_rcode(query, question, 0, Some(address)))
}

fn response_with_rcode(
    query: &[u8],
    question: &QuestionInfo,
    rcode: u8,
    address: Option<[u8; 4]>,
) -> Vec<u8> {
    let (header, _) = parse_query(query).expect("fixture query");
    let mut response = Vec::with_capacity(query.len() + 16);
    response.extend_from_slice(&header.id.to_be_bytes());
    response.extend_from_slice(&(0x8180_u16 | u16::from(rcode)).to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&u16::from(address.is_some()).to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&question.qname_wire);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&question.qclass.to_be_bytes());
    if let Some(address) = address {
        response.extend_from_slice(&[
            0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1e, 0x00, 0x04,
        ]);
        response.extend_from_slice(&address);
    }
    response
}

fn qname_wire(name: &str) -> Vec<u8> {
    let mut wire = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        wire.push(u8::try_from(label.len()).expect("DNS label length"));
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = Vec::from([
        (id >> 8) as u8,
        id.to_be_bytes()[1],
        0x01,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ]);
    packet.extend_from_slice(&qname_wire(name));
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet
}

fn client_request(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

fn routing_yaml(a: SocketAddr, b: SocketAddr, c: SocketAddr) -> String {
    ROUTING_CONFIG
        .replace("udp://127.0.0.1:15456", &format!("udp://{a}"))
        .replace("udp://127.0.0.1:15457", &format!("udp://{b}"))
        .replace("udp://127.0.0.1:15458", &format!("udp://{c}"))
}

fn route_sequence(events: &[RouteEvent], id: u16) -> Vec<&'static str> {
    events
        .iter()
        .filter(|event| event.id == id)
        .map(|event| event.route)
        .collect()
}

fn routing_cases() -> Vec<RoutingCase> {
    ROUTING_WORKLOAD
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| yaml_serde::from_str(line).expect("routing JSONL row"))
        .collect()
}

fn expected_routes(case: &RoutingCase) -> &'static [&'static str] {
    match case.expected_route_class.as_str() {
        "DOMAIN_HIT" => &["a"],
        "IP_RULE_HIT" => &["b", "a"],
        "IP_RULE_MISS" => &["b", "c"],
        other => panic!("unknown route class {other}"),
    }
}

fn verify_route_evidence(cases: &[RoutingCase], events: &[RouteEvent]) -> Result<(), String> {
    for (index, case) in cases.iter().enumerate() {
        let id = 0x7101_u16 + u16::try_from(index).map_err(|_| "too many cases".to_owned())?;
        let actual = route_sequence(events, id);
        let expected = expected_routes(case);
        if actual != expected {
            return Err(format!(
                "route evidence mismatch for {}: expected={expected:?} actual={actual:?}",
                case.case_id
            ));
        }
        let expected_qname = qname_wire(&case.qname);
        if events
            .iter()
            .filter(|event| event.id == id)
            .any(|event| event.qname != expected_qname)
        {
            return Err(format!(
                "route evidence question mismatch for {}",
                case.case_id
            ));
        }
    }
    Ok(())
}

fn assert_original_question(response: &[u8], qname: &str) {
    let (_, expected) = parse_query(&query(0, qname)).expect("original query");
    let question = observe_response_metadata(response)
        .expect("response metadata")
        .question
        .expect("one response question");
    assert_eq!(question.qname_wire, expected.qname_wire);
    assert_eq!(question.qtype, expected.qtype);
    assert_eq!(question.qclass, expected.qclass);
}

fn assert_answer(response: &[u8], id: u16, qname: &str, expected_ip: [u8; 4]) {
    assert_eq!(
        inspect_response_header(response)
            .expect("response header")
            .id,
        id
    );
    assert_eq!(response[3] & 0x0f, 0);
    validate_response(response).expect("valid final response");
    assert_original_question(response, qname);
    assert_eq!(
        observe_answer_addresses(response).expect("answer addresses"),
        vec![std::net::IpAddr::V4(std::net::Ipv4Addr::from(expected_ip))]
    );
}

fn assert_servfail(response: &[u8], id: u16, qname: &str) {
    assert_eq!(
        inspect_response_header(response)
            .expect("response header")
            .id,
        id
    );
    assert_eq!(response[3] & 0x0f, 2);
    validate_response(response).expect("valid SERVFAIL response");
    assert_original_question(response, qname);
}

#[test]
fn tampered_route_evidence_fails_even_with_the_expected_final_answers() {
    let cases = routing_cases();
    let expected_answers = cases
        .iter()
        .map(|case| case.expected_answer.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        expected_answers,
        vec!["192.0.2.11", "192.0.2.11", "192.0.2.12"]
    );
    let mut tampered = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let id = 0x7101_u16 + u16::try_from(index).expect("corpus index");
        for route in expected_routes(case) {
            let route = if case.case_id == "ip-rule-hit" && *route == "a" {
                "c"
            } else {
                *route
            };
            tampered.push(RouteEvent {
                id,
                route,
                qname: qname_wire(&case.qname),
            });
        }
    }
    assert!(verify_route_evidence(&cases, &tampered).is_err());
}

#[test]
fn frozen_w3_corpus_routes_over_real_udp_and_preserves_order() {
    let cases = routing_cases();
    assert_eq!(cases.len(), 3, "the frozen W3 corpus row count changed");
    assert_eq!(
        cases
            .iter()
            .map(|case| case.case_id.as_str())
            .collect::<Vec<_>>(),
        vec!["domain-hit", "ip-rule-hit", "ip-rule-miss"]
    );
    for case in &cases {
        assert_eq!(case.scenario, "w3");
        assert_eq!(case.transport, "udp");
        assert_eq!(case.qtype, "A");
        assert_eq!(case.expected_rcode, 0);
        assert_eq!(case.expected_answer_class, "A");
        assert_eq!(case.request_deadline_ms, 500);
        assert_eq!(case.weight, 1);
        case.expected_answer
            .parse::<std::net::Ipv4Addr>()
            .expect("frozen W3 answer address");
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let route_a = RouteFixture::start("a", &events);
    let route_b = RouteFixture::start("b", &events);
    let route_c = RouteFixture::start("c", &events);
    let yaml = routing_yaml(route_a.address, route_b.address, route_c.address);
    let config = compile_yaml(&yaml).expect("W3 config");
    let assembly =
        HostAssembly::with_options(config, HostOptions::with_deadline(Duration::from_secs(1)))
            .expect("W3 assembly");
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("listener"),
        ))
        .expect("W3 listener bind");
    let listener = server.local_addr().expect("W3 listener address");
    let shutdown = TransportCancellation::new();
    let barrier = Arc::new(Barrier::new(cases.len()));
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut clients = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            let id = 0x7101_u16 + u16::try_from(index).expect("corpus index");
            let request = query(id, &case.qname);
            let barrier = Arc::clone(&barrier);
            clients.push(tokio::task::spawn_blocking(move || {
                barrier.wait();
                (index, id, client_request(listener, &request))
            }));
        }
        let mut responses = Vec::new();
        for client in clients {
            responses.push(client.await.expect("routing client"));
        }
        shutdown.cancel();
        let server_result = server_task.await.expect("W3 server task");
        (responses, server_result)
    });
    result.1.expect("W3 server shutdown");
    for (index, id, response) in result.0 {
        let case = &cases[index];
        let expected_ip = case
            .expected_answer
            .parse::<std::net::Ipv4Addr>()
            .expect("frozen W3 answer address")
            .octets();
        assert_answer(&response, id, &case.qname, expected_ip);
    }
    let snapshot = events.lock().expect("route events").clone();
    verify_route_evidence(&cases, &snapshot).expect("frozen route evidence");
    route_a.stop();
    route_b.stop();
    route_c.stop();
}

#[test]
fn valid_negative_b_answer_falls_through_to_c_without_a() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let route_a = RouteFixture::start("a", &events);
    let route_b = RouteFixture::start("b", &events);
    let route_c = RouteFixture::start("c", &events);
    let assembly = HostAssembly::with_options(
        compile_yaml(&routing_yaml(
            route_a.address,
            route_b.address,
            route_c.address,
        ))
        .expect("W3 config"),
        HostOptions::with_deadline(Duration::from_secs(1)),
    )
    .expect("W3 assembly");
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("listener"),
        ))
        .expect("W3 listener bind");
    let listener = server.local_addr().expect("W3 listener address");
    let shutdown = TransportCancellation::new();
    let response = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            client_request(listener, &query(0x7201, "negative.test."))
        })
        .await
        .expect("negative client");
        shutdown.cancel();
        server_task
            .await
            .expect("negative server task")
            .expect("shutdown");
        response
    });
    assert_answer(&response, 0x7201, "negative.test.", [192, 0, 2, 12]);
    let snapshot = events.lock().expect("route events").clone();
    assert_eq!(route_sequence(&snapshot, 0x7201), vec!["b", "c"]);
    route_a.stop();
    route_b.stop();
    route_c.stop();
}

#[test]
fn w3_malformed_mismatch_and_leg_failures_stop_without_stale_or_extra_routes() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let route_a = RouteFixture::start("a", &events);
    let route_b = RouteFixture::start("b", &events);
    let route_c = RouteFixture::start("c", &events);
    let assembly = HostAssembly::with_options(
        compile_yaml(&routing_yaml(
            route_a.address,
            route_b.address,
            route_c.address,
        ))
        .expect("W3 config"),
        HostOptions::with_deadline(Duration::from_millis(80)),
    )
    .expect("W3 assembly");
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("listener"),
        ))
        .expect("W3 listener bind");
    let listener = server.local_addr().expect("W3 listener address");
    let shutdown = TransportCancellation::new();
    let cases = [
        (0x7301_u16, "b-fail.test."),
        (0x7302_u16, "malformed.test."),
        (0x7303_u16, "mismatch.test."),
        (0x7304_u16, "a-fail.test."),
        (0x7305_u16, "c-fail.test."),
    ];
    let responses = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut responses = Vec::new();
        for (id, name) in cases {
            let response = tokio::task::spawn_blocking(move || {
                (id, client_request(listener, &query(id, name)))
            })
            .await
            .expect("failure-path client");
            responses.push((id, name, response.1));
        }
        shutdown.cancel();
        server_task
            .await
            .expect("failure server task")
            .expect("shutdown");
        responses
    });
    for (id, name, response) in &responses {
        assert_servfail(response, *id, name);
    }
    let snapshot = events.lock().expect("route events").clone();
    assert_eq!(route_sequence(&snapshot, 0x7301), vec!["b"]);
    assert_eq!(route_sequence(&snapshot, 0x7302), vec!["b"]);
    assert_eq!(route_sequence(&snapshot, 0x7303), vec!["b"]);
    assert_eq!(route_sequence(&snapshot, 0x7304), vec!["b", "a"]);
    assert_eq!(route_sequence(&snapshot, 0x7305), vec!["b", "c"]);
    route_a.stop();
    route_b.stop();
    route_c.stop();
}

#[test]
fn w3_shutdown_after_b_is_observed_prevents_late_response_and_rebinds() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let route_a = RouteFixture::start("a", &events);
    let route_b = RouteFixture::start_with_delay("b", &events, Duration::from_millis(200));
    let route_c = RouteFixture::start("c", &events);
    let assembly = HostAssembly::with_options(
        compile_yaml(&routing_yaml(
            route_a.address,
            route_b.address,
            route_c.address,
        ))
        .expect("W3 config"),
        HostOptions::default(),
    )
    .expect("W3 assembly");
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("listener"),
        ))
        .expect("W3 listener bind");
    let listener = server.local_addr().expect("W3 listener address");
    let shutdown = TransportCancellation::new();
    let (received, server_result) = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::task::spawn_blocking(move || {
            let socket = StdUdpSocket::bind("127.0.0.1:0").expect("cancel client bind");
            socket
                .set_read_timeout(Some(Duration::from_millis(500)))
                .expect("cancel client timeout");
            socket
                .send_to(&query(0x7401, "cancel.test."), listener)
                .expect("cancel client send");
            let mut response = [0_u8; 512];
            socket.recv_from(&mut response).is_ok()
        });
        let mut observed_b = false;
        for _ in 0..200 {
            observed_b = events
                .lock()
                .expect("route events")
                .iter()
                .any(|event| event.id == 0x7401 && event.route == "b");
            if observed_b {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(observed_b, "B must observe the request before shutdown");
        shutdown.cancel();
        let received = client.await.expect("cancel client task");
        let server_result = server_task.await.expect("cancel server task");
        (received, server_result)
    });
    assert!(!received, "shutdown must prevent a late W3 response");
    server_result.expect("W3 server shutdown");
    let snapshot = events.lock().expect("route events").clone();
    assert_eq!(route_sequence(&snapshot, 0x7401), vec!["b"]);
    let rebound = assembly
        .block_on(UdpServer::bind(&assembly, listener))
        .expect("W3 listener must rebind after shutdown");
    drop(rebound);
    route_a.stop();
    route_b.stop();
    route_c.stop();
}

#[test]
fn w3_shutdown_cancels_a_and_c_in_flight_without_late_responses() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let route_a = RouteFixture::start_with_delay("a", &events, Duration::from_millis(200));
    let route_b = RouteFixture::start("b", &events);
    let route_c = RouteFixture::start_with_delay("c", &events, Duration::from_millis(200));
    let assembly = HostAssembly::with_options(
        compile_yaml(&routing_yaml(
            route_a.address,
            route_b.address,
            route_c.address,
        ))
        .expect("W3 config"),
        HostOptions::default(),
    )
    .expect("W3 assembly");
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("listener"),
        ))
        .expect("W3 listener bind");
    let listener = server.local_addr().expect("W3 listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let barrier = Arc::new(Barrier::new(2));
        let mut clients = Vec::new();
        for (id, name) in [(0x7501_u16, "ip-hit.test."), (0x7502, "ip-miss.test.")] {
            let barrier = Arc::clone(&barrier);
            clients.push(tokio::task::spawn_blocking(move || {
                barrier.wait();
                let socket = StdUdpSocket::bind("127.0.0.1:0").expect("in-flight client bind");
                socket
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .expect("in-flight client timeout");
                let request = query(id, name);
                socket
                    .send_to(&request, listener)
                    .expect("in-flight client send");
                let mut response = [0_u8; 512];
                socket.recv_from(&mut response).is_ok()
            }));
        }
        let mut observed_a = false;
        let mut observed_c = false;
        for _ in 0..300 {
            (observed_a, observed_c) = {
                let snapshot = events.lock().expect("route events");
                (
                    snapshot
                        .iter()
                        .any(|event| event.id == 0x7501 && event.route == "a"),
                    snapshot
                        .iter()
                        .any(|event| event.id == 0x7502 && event.route == "c"),
                )
            };
            if observed_a && observed_c {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(observed_a && observed_c, "A and C must both be in flight");
        shutdown.cancel();
        let first_received = clients.remove(0).await.expect("A client");
        let second_received = clients.remove(0).await.expect("C client");
        let server_result = server_task.await.expect("in-flight server task");
        ((first_received, second_received), server_result)
    });
    assert!(
        !result.0.0 && !result.0.1,
        "shutdown must prevent late A/C responses"
    );
    result.1.expect("W3 in-flight shutdown");
    let snapshot = events.lock().expect("route events").clone();
    assert_eq!(route_sequence(&snapshot, 0x7501), vec!["b", "a"]);
    assert_eq!(route_sequence(&snapshot, 0x7502), vec!["b", "c"]);
    let rebound = assembly
        .block_on(UdpServer::bind(&assembly, listener))
        .expect("W3 listener must rebind after A/C shutdown");
    drop(rebound);
    route_a.stop();
    route_b.stop();
    route_c.stop();
}
