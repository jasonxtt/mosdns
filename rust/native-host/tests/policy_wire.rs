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
    request_optional(assembly, query).expect("DNS response")
}
fn request_optional(assembly: &HostAssembly, query: Vec<u8>) -> Option<Message> {
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
            let len = match socket.recv(&mut wire) {
                Ok(len) => len,
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
            wire.truncate(len);
            Some(wire)
        })
        .await
        .unwrap();
        shutdown.cancel();
        task.await.unwrap().unwrap();
        response
    });
    wire.map(|wire| Message::from_vec(&wire).unwrap())
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

fn redirect_peer(negative: bool) -> (String, std::thread::JoinHandle<Message>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let address = socket.local_addr().unwrap().to_string();
    let task = std::thread::spawn(move || {
        let mut wire = vec![0; 65535];
        let (len, peer) = socket.recv_from(&mut wire).unwrap();
        let query = Message::from_vec(&wire[..len]).unwrap();
        let mut response = query.clone();
        response.set_message_type(hickory_proto::op::MessageType::Response);
        response.set_authoritative(true);
        if negative {
            response.set_response_code(hickory_proto::op::ResponseCode::NXDomain);
            response.add_name_server(hickory_proto::rr::Record::from_rdata(
                Name::from_ascii("target.example.").unwrap(),
                321,
                RData::SOA(hickory_proto::rr::rdata::SOA::new(
                    Name::from_ascii("ns.example.").unwrap(),
                    Name::from_ascii("mb.example.").unwrap(),
                    1,
                    2,
                    3,
                    4,
                    5,
                )),
            ));
        } else {
            response.add_answer(hickory_proto::rr::Record::from_rdata(
                query.queries()[0].name().clone(),
                60,
                RData::CNAME(hickory_proto::rr::rdata::CNAME(
                    Name::from_ascii("final.example.").unwrap(),
                )),
            ));
            response.add_answer(hickory_proto::rr::Record::from_rdata(
                Name::from_ascii("final.example.").unwrap(),
                70,
                RData::A(hickory_proto::rr::rdata::A("192.0.2.1".parse().unwrap())),
            ));
        }
        socket.send_to(&response.to_vec().unwrap(), peer).unwrap();
        query
    });
    (address, task)
}
#[test]
fn redirect_sends_actual_target_preserves_existing_chain_and_admission_audit() {
    let (address, peer) = redirect_peer(false);
    let host = assembly_extra(
        "      - exec: $rewrite\n      - exec: $upstream",
        "[]",
        &address,
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n",
    );
    let response = request(
        &host,
        query(RecordType::A, DNSClass::IN, "original.example."),
    );
    assert_eq!(
        peer.join().unwrap().queries()[0].name().to_ascii(),
        "target.example."
    );
    assert_eq!(response.queries()[0].name().to_ascii(), "original.example.");
    assert_eq!(response.answers().len(), 3);
    assert_eq!(response.answers()[0].name().to_ascii(), "original.example.");
    assert_eq!(response.answers()[0].data().to_string(), "target.example.");
    assert_eq!(response.answers()[0].ttl(), 1);
    assert_eq!(response.answers()[1].name().to_ascii(), "target.example.");
    assert_eq!(response.answers()[1].ttl(), 60);
    assert_eq!(response.answers()[2].ttl(), 70);
    assert!(response.authoritative());
    let record = &host.audit_snapshot().records[0];
    assert_eq!(record.qname, "original.example.");
    assert!(record.selected_upstream.is_some());
    assert_eq!(record.upstream_attempts.len(), 1);
    assert!(
        record
            .upstream_diagnostics
            .as_ref()
            .unwrap()
            .selected
            .is_some()
    );
}
#[test]
fn nested_redirect_restores_each_question_and_negative_soa() {
    let (address, peer) = redirect_peer(true);
    let host = assembly_extra(
        "      - exec: $first\n      - exec: $second\n      - exec: $upstream",
        "[]",
        &address,
        "  - tag: first\n    type: redirect\n    args: {rules: ['original.example middle.example']}\n  - tag: second\n    type: redirect\n    args: {rules: ['middle.example target.example']}\n",
    );
    let response = request(
        &host,
        query(RecordType::A, DNSClass::IN, "original.example."),
    );
    assert_eq!(
        peer.join().unwrap().queries()[0].name().to_ascii(),
        "target.example."
    );
    assert_eq!(
        response.response_code(),
        hickory_proto::op::ResponseCode::NXDomain
    );
    assert_eq!(response.queries()[0].name().to_ascii(), "original.example.");
    assert_eq!(
        response
            .answers()
            .iter()
            .map(|r| (r.name().to_ascii(), r.data().to_string(), r.ttl()))
            .collect::<Vec<_>>(),
        vec![
            ("original.example.".into(), "middle.example.".into(), 1),
            ("middle.example.".into(), "target.example.".into(), 1)
        ]
    );
    assert_eq!(
        response.name_servers()[0].name().to_ascii(),
        "target.example."
    );
    assert_eq!(response.name_servers()[0].ttl(), 321);
}

#[test]
fn cache_before_and_inside_redirect_owns_correct_wire_and_key() {
    for outer in [true, false] {
        let (address, peer) = redirect_peer(false);
        let rules = if outer {
            "      - exec: $stored\n      - exec: $rewrite\n      - exec: $upstream"
        } else {
            "      - exec: $rewrite\n      - exec: $stored\n      - exec: $upstream"
        };
        let host = assembly_extra(
            rules,
            "[]",
            &address,
            "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: stored\n    type: cache\n    args: {size: 64, lazy_cache_ttl: 0}\n",
        );
        let original = query(RecordType::A, DNSClass::IN, "original.example.");
        let target = query(RecordType::A, DNSClass::IN, "target.example.");
        let first = request(&host, original.clone());
        assert_eq!(
            peer.join().unwrap().queries()[0].name().to_ascii(),
            "target.example."
        );
        let second = request(&host, original.clone());
        assert_eq!(first.answers(), second.answers());
        let cache = host.cache().get(mosdns_native_host::CacheId(0)).unwrap();
        let (stored, absent) = if outer {
            (&original, &target)
        } else {
            (&target, &original)
        };
        let stored_wire = cache.lookup(stored).unwrap().expect("correct cache key");
        let stored_message = Message::from_vec(&stored_wire).unwrap();
        assert_eq!(stored_message.answers().len(), if outer { 3 } else { 2 });
        assert!(cache.lookup(absent).unwrap().is_none());
        assert_eq!(
            host.audit_snapshot().records[1].response,
            ResponseState::Dns {
                rcode: 0,
                source: ResponseSource::Cache
            }
        );
    }
}
#[test]
fn redirect_exit_decorates_response_but_never_publishes_outer_cache() {
    let host = assembly_extra(
        "      - exec: $stored\n      - exec: $rewrite\n      - exec: $local\n      - exec: exit",
        "['target.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: stored\n    type: cache\n    args: {size: 64, lazy_cache_ttl: 0}\n",
    );
    let original = query(RecordType::A, DNSClass::IN, "original.example.");
    let response = request(&host, original.clone());
    assert_eq!(response.answers().len(), 2);
    assert_eq!(response.queries()[0].name().to_ascii(), "original.example.");
    assert!(
        host.cache()
            .get(mosdns_native_host::CacheId(0))
            .unwrap()
            .lookup(&original)
            .unwrap()
            .is_none()
    );
}
#[test]
fn redirect_cycle_shares_root_fuel_and_cannot_manufacture_success() {
    let host = assembly_extra(
        "      - exec: $rewrite\n      - exec: goto $main",
        "[]",
        "127.0.0.1:19000",
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example original.example']}\n",
    );
    let response = request_optional(
        &host,
        query(RecordType::A, DNSClass::IN, "original.example."),
    );
    assert!(response.is_none());
    assert_eq!(
        host.audit_snapshot().records[0].response,
        ResponseState::NoResponse
    );
}

#[test]
fn redirect_query_view_preserves_flags_qtype_and_opt() {
    let (address, peer) = redirect_peer(false);
    let host = assembly_extra(
        "      - exec: $rewrite\n      - exec: $upstream",
        "[]",
        &address,
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n",
    );
    let mut original =
        Message::from_vec(&query(RecordType::AAAA, DNSClass::IN, "original.example.")).unwrap();
    original.set_id(0xaa55).set_checking_disabled(true);
    original
        .extensions_mut()
        .get_or_insert_with(hickory_proto::op::Edns::new)
        .set_max_payload(1232)
        .set_dnssec_ok(true);
    let response = request(&host, original.to_vec().unwrap());
    let target = peer.join().unwrap();
    assert_eq!(target.id(), original.id());
    assert_eq!(target.queries()[0].query_type(), RecordType::AAAA);
    assert_eq!(target.queries()[0].query_class(), DNSClass::IN);
    assert!(target.recursion_desired());
    assert!(target.checking_disabled());
    assert_eq!(target.extensions(), original.extensions());
    assert_eq!(response.extensions(), original.extensions());
    assert_eq!(response.id(), original.id());
}
#[test]
fn redirect_non_in_is_noop_and_direct_fallback_redirect_uses_current_target() {
    for class in [DNSClass::IN, DNSClass::CH] {
        let (address, peer) = redirect_peer(false);
        let host = assembly_extra(
            "      - exec: $choice\n      - exec: $upstream",
            "[]",
            &address,
            "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: choice\n    type: fallback\n    args: {primary: '$rewrite', secondary: '$upstream', threshold: 100}\n",
        );
        let response = request(&host, query(RecordType::A, class, "original.example."));
        let target = peer.join().unwrap();
        assert_eq!(
            target.queries()[0].name().to_ascii(),
            if class == DNSClass::IN {
                "target.example."
            } else {
                "original.example."
            }
        );
        assert_eq!(
            response.answers().len(),
            if class == DNSClass::IN { 3 } else { 2 }
        );
        assert_eq!(response.queries()[0].query_class(), class);
    }
}

#[test]
fn redirect_exit_through_named_fallback_target_keeps_abort_and_skips_parent_tail() {
    let host = assembly_extra(
        "      - exec: $stored\n      - exec: $choice\n      - exec: ttl 42\n      - exec: accept",
        "['target.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: routed\n    type: sequence\n    args:\n      - exec: $rewrite\n      - exec: $local\n      - exec: exit\n  - tag: choice\n    type: fallback\n    args: {primary: '$routed', secondary: '$upstream', threshold: 100}\n  - tag: stored\n    type: cache\n    args: {size: 64, lazy_cache_ttl: 0}\n",
    );
    let original = query(RecordType::A, DNSClass::IN, "original.example.");
    let response = request(&host, original.clone());
    assert_eq!(response.answers()[0].ttl(), 1);
    assert_eq!(response.answers()[1].ttl(), 10);
    assert!(
        host.cache()
            .get(mosdns_native_host::CacheId(0))
            .unwrap()
            .lookup(&original)
            .unwrap()
            .is_none()
    );
}
