use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use mosdns_native_host::{HostAssembly, ResponseSource, ResponseState, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use std::net::UdpSocket;
use std::time::Duration;

fn assembly(rules: &str, entries: &str) -> HostAssembly {
    assembly_for(rules, entries, "127.0.0.1:19000")
}
fn assembly_for(rules: &str, entries: &str, upstream: &str) -> HostAssembly {
    assembly_extra(rules, entries, upstream, "")
}
fn assembly_extra(rules: &str, entries: &str, upstream: &str, extra: &str) -> HostAssembly {
    let yaml = format!(
        "log: {{level: error}}\nplugins:\n  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: udp://{upstream}}}]}}\n  - tag: local\n    type: hosts\n    args:\n      entries: {entries}\n{extra}  - tag: main\n    type: sequence\n    args:\n{rules}\n  - tag: listener\n    type: udp_server\n    args:\n      entry: main\n      listen: 127.0.0.1:19100\n      enable_audit: true\n"
    );
    HostAssembly::from_config(compile_yaml(&yaml).expect("compile")).expect("policy runtime")
}
fn query(kind: RecordType, class: DNSClass, name: &str) -> Vec<u8> {
    let mut message = Message::new();
    let mut q = Query::query(Name::from_ascii(name).unwrap(), kind);
    q.set_query_class(class);
    message.set_id(42).set_recursion_desired(true).add_query(q);
    message.to_vec().unwrap()
}
fn request(assembly: &HostAssembly, query: Vec<u8>) -> Message {
    let server = assembly
        .block_on(UdpServer::bind(assembly, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = server.local_addr().unwrap();
    let shutdown = TransportCancellation::new();
    let wire = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket.send_to(&query, address).unwrap();
            let mut wire = vec![0; 65535];
            let len = socket.recv(&mut wire).unwrap();
            wire.truncate(len);
            wire
        })
        .await
        .unwrap();
        shutdown.cancel();
        task.await.unwrap().unwrap();
        response
    });
    Message::from_vec(&wire).unwrap()
}
#[test]
fn hosts_dual_stack_order_ttl_and_continue_into_quick_ttl() {
    let host = assembly(
        "      - exec: $local\n      - exec: ttl 300-10\n      - matches: has_resp\n        exec: accept",
        "['a.example 192.0.2.2 192.0.2.1 ::2 ::1']",
    );
    for kind in [RecordType::A, RecordType::AAAA] {
        let response = request(&host, query(kind, DNSClass::IN, "a.example."));
        assert_eq!(response.answers().len(), 2);
        assert!(response.answers().iter().all(|r| r.ttl() == 10));
        let first = response.answers()[0].data().to_string();
        assert_eq!(
            first,
            if kind == RecordType::A {
                "192.0.2.2"
            } else {
                "::2"
            }
        );
    }
}
#[test]
fn hosts_empty_family_has_complete_fake_soa() {
    let host = assembly(
        "      - exec: $local\n      - exec: accept",
        "['a.example 192.0.2.1']",
    );
    let response = request(&host, query(RecordType::AAAA, DNSClass::IN, "a.example."));
    assert_eq!(response.response_code().low(), 0);
    assert!(response.answers().is_empty());
    assert_eq!(response.name_servers().len(), 1);
    let record = &response.name_servers()[0];
    assert_eq!(record.name().to_ascii(), "a.example.");
    assert_eq!(record.ttl(), 300);
    let RData::SOA(soa) = record.data() else {
        panic!("SOA required")
    };
    assert_eq!(soa.mname().to_ascii(), "fake-ns.mosdns.fake.root.");
    assert_eq!(soa.rname().to_ascii(), "fake-mbox.mosdns.fake.root.");
    assert_eq!(
        (
            soa.serial(),
            soa.refresh(),
            soa.retry(),
            soa.expire(),
            soa.minimum()
        ),
        (2_021_110_400, 1800, 900, 604_800, 86400)
    );
}
#[test]
fn hosts_no_hit_no_addresses_non_in_and_non_address_continue() {
    let host = assembly(
        "      - exec: $local\n      - exec: reject 3",
        "['a.example 192.0.2.1', 'empty.example']",
    );
    for (name, kind, class) in [
        ("missing.example.", RecordType::A, DNSClass::IN),
        ("empty.example.", RecordType::A, DNSClass::IN),
        ("a.example.", RecordType::MX, DNSClass::IN),
        ("a.example.", RecordType::A, DNSClass::CH),
    ] {
        let response = request(&host, query(kind, class, name));
        assert_eq!(response.response_code().low(), 3);
    }
}
#[test]
fn ttl_fixed_zero_is_noop_and_range_zero_is_unbounded() {
    for (policy, expected) in [
        ("0", 10),
        ("42", 42),
        ("20-0", 20),
        ("0-5", 5),
        ("0-0", 10),
        ("300-10", 10),
    ] {
        let host = assembly(
            &format!("      - exec: $local\n      - exec: ttl {policy}\n      - exec: accept"),
            "['a.example 192.0.2.1']",
        );
        let response = request(&host, query(RecordType::A, DNSClass::IN, "a.example."));
        assert_eq!(response.answers()[0].ttl(), expected);
    }
}

#[test]
fn hosts_replaces_upstream_supplier_but_preserves_actual_attempts() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let addr = socket.local_addr().unwrap();
    let peer = std::thread::spawn(move || {
        let mut wire = vec![0; 65535];
        let (len, peer) = socket.recv_from(&mut wire).unwrap();
        let mut response = Message::from_vec(&wire[..len]).unwrap();
        response.set_message_type(hickory_proto::op::MessageType::Response);
        response.add_answer(hickory_proto::rr::Record::from_rdata(
            Name::from_ascii("a.example.").unwrap(),
            60,
            RData::A(hickory_proto::rr::rdata::A("198.51.100.1".parse().unwrap())),
        ));
        socket.send_to(&response.to_vec().unwrap(), peer).unwrap();
    });
    let host = assembly_for(
        "      - exec: $upstream\n      - exec: $local\n      - exec: accept",
        "['a.example 192.0.2.1']",
        &addr.to_string(),
    );
    let response = request(&host, query(RecordType::A, DNSClass::IN, "a.example."));
    peer.join().unwrap();
    assert_eq!(response.answers()[0].data().to_string(), "192.0.2.1");
    assert!(response.recursion_desired());
    let audit = host.audit_snapshot();
    let record = &audit.records[0];
    assert_eq!(record.upstream_attempts.len(), 1);
    assert!(record.selected_upstream.is_none());
    assert!(record.final_upstream.is_none());
    assert_eq!(
        record.response,
        ResponseState::Dns {
            rcode: 0,
            source: ResponseSource::Local
        }
    );
    assert!(
        record
            .upstream_diagnostics
            .as_ref()
            .unwrap()
            .selected
            .is_none()
    );
}

#[test]
fn direct_fallback_hosts_target_continues_into_branch_ttl() {
    let host = assembly_extra(
        "      - exec: $choice\n      - exec: ttl 77\n      - exec: accept",
        "['a.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: choice\n    type: fallback\n    args:\n      primary: $local\n      secondary: $upstream\n      threshold: 100\n",
    );
    let response = request(&host, query(RecordType::A, DNSClass::IN, "a.example."));
    assert_eq!(response.answers()[0].ttl(), 77);
    assert_eq!(
        host.audit_snapshot().records[0].response,
        ResponseState::Dns {
            rcode: 0,
            source: ResponseSource::Local
        }
    );
    assert!(
        host.audit_snapshot().records[0]
            .upstream_attempts
            .is_empty()
    );
}
