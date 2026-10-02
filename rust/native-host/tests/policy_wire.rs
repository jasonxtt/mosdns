use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use mosdns_native_host::{
    CacheTestClock, HostAssembly, HostOptions, ResponseSource, ResponseState, UdpServer,
    compile_yaml,
};
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
    assembly_options(rules, entries, upstream, extra, HostOptions::default())
}
fn assembly_options(
    rules: &str,
    entries: &str,
    upstream: &str,
    extra: &str,
    options: HostOptions,
) -> HostAssembly {
    let yaml = format!(
        "log: {{level: error}}\nplugins:\n  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: udp://{upstream}}}]}}\n  - tag: local\n    type: hosts\n    args:\n      entries: {entries}\n{extra}  - tag: main\n    type: sequence\n    args:\n{rules}\n  - tag: listener\n    type: udp_server\n    args:\n      entry: main\n      listen: 127.0.0.1:19100\n      enable_audit: true\n"
    );
    HostAssembly::with_options(compile_yaml(&yaml).expect("compile"), options)
        .expect("policy runtime")
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
    assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = ask(address, query).await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        response
    })
}
async fn ask(address: std::net::SocketAddr, query: Vec<u8>) -> Option<Message> {
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
    response.map(|wire| Message::from_vec(&wire).unwrap())
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
    redirect_peer_ip(negative, "192.0.2.1")
}
fn redirect_peer_ip(negative: bool, ip: &str) -> (String, std::thread::JoinHandle<Message>) {
    let ip: std::net::Ipv4Addr = ip.parse().unwrap();
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
                RData::A(hickory_proto::rr::rdata::A(ip)),
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

#[test]
fn redirect_consumes_jump_continuation_exactly_once_before_restoration() {
    let host = assembly_extra(
        "      - exec: jump $inner\n      - exec: ttl 42\n      - exec: accept",
        "['target.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: inner\n    type: sequence\n    args:\n      - exec: $rewrite\n      - exec: $local\n",
    );
    let response = request(
        &host,
        query(RecordType::A, DNSClass::IN, "original.example."),
    );
    assert_eq!(
        response.answers()[0].ttl(),
        1,
        "caller tail runs once in captured target scope, before CNAME restoration"
    );
    assert_eq!(response.answers()[1].ttl(), 42);
}

#[test]
fn response_ip_named_cidr_or_ipv6_and_mapped_addresses_drive_real_sequence() {
    for (address, kind, expression, hit) in [
        ("192.0.2.1", RecordType::A, "$networks", true),
        ("192.0.3.1", RecordType::A, "$networks", false),
        (
            "2001:db8::1",
            RecordType::AAAA,
            "$networks 198.51.100.0/24",
            true,
        ),
        (
            "2001:db9::1",
            RecordType::AAAA,
            "$networks 198.51.100.0/24",
            false,
        ),
        ("::ffff:192.0.2.1", RecordType::AAAA, "192.0.2.0/24", true),
        ("2001:db9::1", RecordType::AAAA, "::/0", true),
        ("198.51.100.1", RecordType::A, "$empty 198.51.100.1", true),
    ] {
        let rules = format!(
            "      - exec: $local\n      - matches: resp_ip {expression}\n        exec: ttl 99\n      - exec: accept"
        );
        let entries = format!("['a.example {address}']");
        let host = assembly_extra(
            &rules,
            &entries,
            "127.0.0.1:19000",
            "  - tag: networks\n    type: ip_set\n    args: {ips: ['192.0.2.0/24', '2001:db8::/32']}\n  - tag: empty\n    type: ip_set\n",
        );
        let response = request(&host, query(kind, DNSClass::IN, "a.example."));
        assert_eq!(
            response.answers()[0].ttl(),
            if hit { 99 } else { 10 },
            "{address} / {expression}"
        );
    }
}

#[test]
fn response_ip_text_file_is_or_snapshot_not_request_time_io() {
    let path = std::env::temp_dir().join(format!(
        "mosdns-ip-wire-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, "# fixture\n192.0.2.0/24 trailing columns\n").unwrap();
    let rules = format!(
        "      - exec: $local\n      - matches: resp_ip $empty &{} 2001:db8::/32\n        exec: ttl 99\n      - exec: accept",
        path.display()
    );
    let host = assembly_extra(
        &rules,
        "['a.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: empty\n    type: ip_set\n",
    );
    let original = query(RecordType::A, DNSClass::IN, "a.example.");
    assert_eq!(request(&host, original.clone()).answers()[0].ttl(), 99);
    std::fs::write(&path, "198.51.100.0/24\n").unwrap();
    assert_eq!(request(&host, original.clone()).answers()[0].ttl(), 99);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(request(&host, original).answers()[0].ttl(), 99);
}

#[test]
fn composition_ttl_inside_and_outside_cache_keep_distinct_stored_ttls() {
    for inside in [true, false] {
        let clock = CacheTestClock::new(10);
        let rules = if inside {
            "      - exec: $stored\n      - exec: $local\n      - exec: ttl 30\n      - exec: accept"
        } else {
            "      - exec: $child\n      - exec: ttl 30\n      - exec: accept"
        };
        let host = assembly_options(
            rules,
            "['a.example 192.0.2.1']",
            "127.0.0.1:19000",
            "  - tag: stored\n    type: cache\n    args: {size: 64, lazy_cache_ttl: 0}\n  - tag: child\n    type: sequence\n    args:\n      - exec: $stored\n      - exec: $local\n      - exec: accept\n",
            HostOptions::default().with_cache_clock(std::rc::Rc::new(clock.clone())),
        );
        let raw = query(RecordType::A, DNSClass::IN, "a.example.");
        assert_eq!(request(&host, raw.clone()).answers()[0].ttl(), 30);
        let cache = host.cache().get(mosdns_native_host::CacheId(0)).unwrap();
        assert_eq!(
            Message::from_vec(&cache.lookup(&raw).unwrap().unwrap())
                .unwrap()
                .answers()[0]
                .ttl(),
            if inside { 30 } else { 10 }
        );
        clock.advance(3);
        assert_eq!(
            request(&host, raw.clone()).answers()[0].ttl(),
            if inside { 27 } else { 30 }
        );
        assert_eq!(
            Message::from_vec(&cache.lookup(&raw).unwrap().unwrap())
                .unwrap()
                .answers()[0]
                .ttl(),
            if inside { 27 } else { 7 }
        );
    }
}
#[test]
fn composition_retained_dump_can_hit_old_policy_until_quiescent_durable_flush() {
    let path = std::env::temp_dir().join(format!(
        "mosdns-policy-dump-{}-{}.gz",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let extra = format!(
        "  - tag: stored\n    type: cache\n    args: {{size: 64, lazy_cache_ttl: 0, dump_file: '{}'}}\n",
        path.display()
    );
    let rules = "      - exec: $stored\n      - exec: $local\n      - exec: accept";
    let raw = query(RecordType::A, DNSClass::IN, "a.example.");
    let old = assembly_extra(rules, "['a.example 192.0.2.1']", "127.0.0.1:19000", &extra);
    let server = old
        .block_on(UdpServer::bind(&old, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let addr = server.local_addr().unwrap();
    let stop = TransportCancellation::new();
    old.block_on(async {
        let task = tokio::task::spawn_local(server.serve(stop.clone()));
        assert_eq!(
            ask(addr, raw.clone()).await.unwrap().answers()[0]
                .data()
                .to_string(),
            "192.0.2.1"
        );
        old.cache()
            .get(mosdns_native_host::CacheId(0))
            .unwrap()
            .save()
            .await
            .unwrap();
        stop.cancel();
        task.await.unwrap().unwrap();
    });
    drop(old);
    let changed = assembly_extra(rules, "['a.example 192.0.2.2']", "127.0.0.1:19000", &extra);
    let cache = changed.cache().get(mosdns_native_host::CacheId(0)).unwrap();
    changed
        .block_on(cache.import_dump(std::fs::read(&path).unwrap()))
        .unwrap();
    let server = changed
        .block_on(UdpServer::bind(&changed, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let addr = server.local_addr().unwrap();
    let stop = TransportCancellation::new();
    changed.block_on(async {
        let task = tokio::task::spawn_local(server.serve(stop.clone()));
        assert_eq!(
            ask(addr, raw.clone()).await.unwrap().answers()[0]
                .data()
                .to_string(),
            "192.0.2.1",
            "retained v2 dump has no policy generation"
        );
        // The only producer has completed. No query is submitted during flush.
        cache.flush().await.unwrap();
        stop.cancel();
        task.await.unwrap().unwrap();
    });
    drop(changed);
    let after_flush = assembly_extra(rules, "['a.example 192.0.2.2']", "127.0.0.1:19000", &extra);
    after_flush
        .block_on(
            after_flush
                .cache()
                .get(mosdns_native_host::CacheId(0))
                .unwrap()
                .import_dump(std::fs::read(&path).unwrap()),
        )
        .unwrap();
    assert_eq!(
        request(&after_flush, raw).answers()[0].data().to_string(),
        "192.0.2.2"
    );
    drop(after_flush);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn composition_lazy_refresh_uses_same_immutable_hosts_snapshot() {
    let clock = CacheTestClock::new(10);
    let path = std::env::temp_dir().join(format!("mosdns-hosts-lazy-{}.txt", std::process::id()));
    std::fs::write(&path, "a.example 192.0.2.1\n").unwrap();
    let extra = format!(
        "  - tag: source_hosts\n    type: hosts\n    args: {{files: ['{}']}}\n  - tag: stored\n    type: cache\n    args: {{size: 64, lazy_cache_ttl: 90}}\n",
        path.display()
    );
    let host = assembly_options(
        "      - exec: $stored\n      - exec: $source_hosts\n      - exec: accept",
        "[]",
        "127.0.0.1:19000",
        &extra,
        HostOptions::default().with_cache_clock(std::rc::Rc::new(clock.clone())),
    );
    let raw = query(RecordType::A, DNSClass::IN, "a.example.");
    let server = host
        .block_on(UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = server.local_addr().unwrap();
    let shutdown = TransportCancellation::new();
    host.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        assert_eq!(
            ask(address, raw.clone()).await.unwrap().answers()[0].ttl(),
            10
        );
        std::fs::write(&path, "a.example 192.0.2.2\n").unwrap();
        clock.advance(11);
        let lazy = ask(address, raw.clone()).await.unwrap();
        assert_eq!(lazy.answers()[0].data().to_string(), "192.0.2.1");
        let cache = host.cache().get(mosdns_native_host::CacheId(0)).unwrap();
        let limit = std::time::Instant::now() + Duration::from_secs(1);
        while cache.lookup_entry(&raw).unwrap().unwrap().state
            != mosdns_cache_core::LookupState::Fresh
            && std::time::Instant::now() < limit
        {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            cache.lookup_entry(&raw).unwrap().unwrap().state,
            mosdns_cache_core::LookupState::Fresh
        );
        assert_eq!(
            ask(address, raw.clone()).await.unwrap().answers()[0].ttl(),
            10
        );
        shutdown.cancel();
        task.await.unwrap().unwrap();
    });
    drop(host);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn composition_ip_condition_selects_fallback_supplier_and_keeps_both_attempts() {
    let (primary, first) = redirect_peer_ip(false, "192.0.2.1");
    let (backup, second) = redirect_peer_ip(false, "198.51.100.1");
    let extra = format!(
        "  - tag: backup\n    type: forward\n    args: {{upstreams: [{{addr: udp://{backup}}}]}}\n  - tag: choice\n    type: fallback\n    args: {{primary: '$backup', secondary: '$local', threshold: 100}}\n"
    );
    let host = assembly_extra(
        "      - exec: $upstream\n      - matches: resp_ip 192.0.2.0/24\n        exec: $choice\n      - exec: accept",
        "[]",
        &primary,
        &extra,
    );
    let response = request(&host, query(RecordType::A, DNSClass::IN, "a.example."));
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(response.answers()[1].data().to_string(), "198.51.100.1");
    let audit = host.audit_snapshot();
    let record = &audit.records[0];
    assert_eq!(record.upstream_attempts.len(), 2);
    assert_eq!(record.selected_upstream.as_deref(), Some(backup.as_str()));
    assert_eq!(
        record
            .upstream_diagnostics
            .as_ref()
            .unwrap()
            .selected
            .as_ref()
            .unwrap()
            .entry,
        "backup"
    );
}
#[test]
fn composition_redirect_preference_probe_uses_current_question_and_qtype_matchers() {
    let host = assembly_extra(
        "      - exec: $rewrite\n      - exec: prefer_ipv4\n      - matches: qtype 1\n        exec: $local\n      - matches: qtype 28\n        exec: reject 0",
        "['target.example 192.0.2.1']",
        "127.0.0.1:19000",
        "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n",
    );
    let response = request(
        &host,
        query(RecordType::AAAA, DNSClass::IN, "original.example."),
    );
    assert_eq!(response.queries()[0].query_type(), RecordType::AAAA);
    assert_eq!(response.answers().len(), 1);
    assert_eq!(response.answers()[0].data().to_string(), "target.example.");
    let audit = host.audit_snapshot();
    assert!(
        audit.records[0]
            .upstream_diagnostics
            .as_ref()
            .unwrap()
            .branches
            .iter()
            .any(|branch| branch.decision == "suppressed")
    );
    assert!(audit.records[0].selected_upstream.is_none());
}

fn held_peer() -> (
    String,
    std::thread::JoinHandle<Message>,
    std::sync::mpsc::Sender<()>,
) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let address = socket.local_addr().unwrap().to_string();
    let (stop, wait) = std::sync::mpsc::channel();
    let task = std::thread::spawn(move || {
        let mut wire = vec![0; 65535];
        let (len, _) = socket.recv_from(&mut wire).unwrap();
        let query = Message::from_vec(&wire[..len]).unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(2));
        query
    });
    (address, task, stop)
}
#[test]
fn composition_fallback_redirect_siblings_are_isolated_and_cancelled_supplier_never_wins() {
    let (primary, first, release) = held_peer();
    let (backup, second) = redirect_peer_ip(false, "198.51.100.1");
    let extra = format!(
        "  - tag: backup\n    type: forward\n    args: {{upstreams: [{{addr: udp://{backup}}}]}}\n  - tag: first_rewrite\n    type: redirect\n    args: {{rules: ['original.example primary.example']}}\n  - tag: second_rewrite\n    type: redirect\n    args: {{rules: ['original.example secondary.example']}}\n  - tag: first_path\n    type: sequence\n    args: [{{exec: '$first_rewrite'}}, {{exec: '$upstream'}}]\n  - tag: second_path\n    type: sequence\n    args: [{{exec: '$second_rewrite'}}, {{exec: '$backup'}}]\n  - tag: choice\n    type: fallback\n    args: {{primary: '$first_path', secondary: '$second_path', threshold: 100, always_standby: true}}\n"
    );
    let host = assembly_extra(
        "      - exec: $choice\n      - exec: accept",
        "[]",
        &primary,
        &extra,
    );
    let response = request(
        &host,
        query(RecordType::A, DNSClass::IN, "original.example."),
    );
    release.send(()).unwrap();
    assert_eq!(
        first.join().unwrap().queries()[0].name().to_ascii(),
        "primary.example."
    );
    assert_eq!(
        second.join().unwrap().queries()[0].name().to_ascii(),
        "secondary.example."
    );
    assert_eq!(response.queries()[0].name().to_ascii(), "original.example.");
    assert_eq!(
        response.answers()[0].data().to_string(),
        "secondary.example."
    );
    assert!(
        response
            .answers()
            .iter()
            .all(|answer| !answer.data().to_string().contains("primary.example"))
    );
    let audit = host.audit_snapshot();
    let record = &audit.records[0];
    assert_eq!(record.upstream_attempts.len(), 2);
    assert_eq!(record.selected_upstream.as_deref(), Some(backup.as_str()));
    let diagnostics = record.upstream_diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics.selected.as_ref().unwrap().entry, "backup");
    assert!(
        diagnostics
            .attempts
            .iter()
            .any(|attempt| attempt.entry == "upstream"
                && attempt.outcome == mosdns_native_host::UpstreamAttemptOutcome::Canceled)
    );
}
#[test]
fn composition_ip_matchers_see_target_and_restored_cached_answers_at_their_own_boundaries() {
    let extra = "  - tag: rewrite\n    type: redirect\n    args: {rules: ['original.example target.example']}\n  - tag: stored\n    type: cache\n    args: {size: 64, lazy_cache_ttl: 0}\n  - tag: child\n    type: sequence\n    args:\n      - exec: $stored\n      - exec: $rewrite\n      - matches: qname target.example\n        exec: $local\n      - matches: resp_ip 192.0.2.0/24\n        exec: ttl 20\n      - exec: accept\n";
    let host = assembly_extra(
        "      - exec: $child\n      - matches: resp_ip 192.0.2.0/24\n        exec: ttl 30\n      - exec: accept",
        "['target.example 192.0.2.1']",
        "127.0.0.1:19000",
        extra,
    );
    let raw = query(RecordType::A, DNSClass::IN, "original.example.");
    for _ in 0..2 {
        let response = request(&host, raw.clone());
        assert_eq!(response.answers().len(), 2);
        assert!(response.answers().iter().all(|answer| answer.ttl() == 30));
    }
    let stored = Message::from_vec(
        &host
            .cache()
            .get(mosdns_native_host::CacheId(0))
            .unwrap()
            .lookup(&raw)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stored.answers()[0].ttl(), 1);
    assert_eq!(stored.answers()[1].ttl(), 20);
}
