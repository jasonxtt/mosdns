/// Appended to `rust/native-host/tests/slice3_composition.rs` for the isolated
/// Linux A6 exact-rule verification. Keep this source with its evidence so the
/// temporary test can be independently audited and reproduced.
fn exact_rule_assembly(
    fixture: &Fixture,
    local: SocketAddr,
    default: SocketAddr,
    tcp_listener: bool,
) -> HostAssembly {
    fixture.write("sub_config/rules/local.txt", "full:local.only.test\n");
    // This invalid decoy makes resolving files relative to the root config fail.
    fixture.write("rules/local.txt", "not-a-valid-rule\n");
    fixture.write(
        "sub_config/routes.yaml",
        &ROUTES_CONFIG
            .replace("RULES_PATH", "rules/local.txt")
            .replace("udp://127.0.0.1:26361", &format!("udp://{local}"))
            .replace("tcp://127.0.0.1:26362", &format!("udp://{default}")),
    );
    let root = if tcp_listener {
        ROOT_CONFIG
            .replace("type: udp_server", "type: tcp_server")
            .replace(
                "      enable_audit: true",
                "      idle_timeout: 5\n      enable_audit: false",
            )
    } else {
        ROOT_CONFIG.to_owned()
    };
    fixture.write("config.yaml", &root);
    HostAssembly::from_config_file(&fixture.config()).expect("exact-rule chain assembly")
}

#[test]
fn file_backed_full_rule_matches_only_the_exact_name_over_udp_and_tcp() {
    let local = Peer::start(LOCAL_ANSWER);
    let default = Peer::start(DEFAULT_ANSWER);

    let udp_fixture = Fixture::new("exact-rule-udp");
    let udp_assembly = exact_rule_assembly(&udp_fixture, local.address, default.address, false);
    let udp_server = udp_assembly
        .block_on(UdpServer::bind(
            &udp_assembly,
            "127.0.0.1:0".parse().expect("UDP bind address"),
        ))
        .expect("UDP listener bind");
    let udp_listener = udp_server.local_addr().expect("UDP listener address");
    eprintln!(
        "A6_EXACT_ENDPOINTS udp listener={udp_listener} local={} default={}",
        local.address, default.address
    );
    let udp_shutdown = TransportCancellation::new();
    let udp_responses = udp_assembly.block_on(async {
        let task = tokio::task::spawn_local(udp_server.serve(udp_shutdown.clone()));
        let mut responses = Vec::new();
        for (id, name) in [
            (0x7101_u16, "local.only.test."),
            (0x7102_u16, "sub.local.only.test."),
        ] {
            let request = query(id, name, 1);
            responses.push(
                tokio::task::spawn_blocking(move || {
                    client_request(udp_listener, &request, Duration::from_secs(2))
                })
                .await
                .expect("UDP client"),
            );
        }
        udp_shutdown.cancel();
        task.await.expect("UDP server task").expect("UDP shutdown");
        responses
    });
    for (index, response) in udp_responses.iter().enumerate() {
        let header = inspect_response_header(response).expect("UDP response header");
        assert_eq!(header.id, 0x7101 + u16::try_from(index).expect("UDP index"));
        validate_response(response).expect("valid UDP response");
    }
    assert_eq!(
        answer_addresses(&udp_responses[0]),
        vec![std::net::IpAddr::V4(LOCAL_ANSWER.into())],
        "the exact full rule must reach the local peer"
    );
    assert_eq!(
        answer_addresses(&udp_responses[1]),
        vec![std::net::IpAddr::V4(DEFAULT_ANSWER.into())],
        "a subdomain must not match the exact full rule"
    );
    assert_eq!(local.requests(), 1, "only the exact UDP name uses local");
    assert_eq!(default.requests(), 1, "the UDP subdomain uses default");
    assert_eq!(udp_assembly.audit_snapshot().records.len(), 2);

    let tcp_fixture = Fixture::new("exact-rule-tcp");
    let tcp_assembly = exact_rule_assembly(&tcp_fixture, local.address, default.address, true);
    let tcp_server = tcp_assembly
        .block_on(mosdns_native_host::TcpServer::bind(
            &tcp_assembly,
            "127.0.0.1:0".parse().expect("TCP bind address"),
        ))
        .expect("TCP listener bind");
    let tcp_listener = tcp_server.local_addr().expect("TCP listener address");
    eprintln!(
        "A6_EXACT_ENDPOINTS tcp listener={tcp_listener} local={} default={}",
        local.address, default.address
    );
    let tcp_shutdown = TransportCancellation::new();
    let tcp_responses = tcp_assembly.block_on(async {
        let task = tokio::task::spawn_local(tcp_server.serve(tcp_shutdown.clone()));
        let mut responses = Vec::new();
        for (id, name) in [
            (0x7201_u16, "local.only.test."),
            (0x7202_u16, "sub.local.only.test."),
        ] {
            let request = query(id, name, 1);
            responses.push(
                tokio::task::spawn_blocking(move || {
                    tcp_request(tcp_listener, &request, Duration::from_secs(3))
                })
                .await
                .expect("TCP client"),
            );
        }
        tcp_shutdown.cancel();
        task.await.expect("TCP server task").expect("TCP shutdown");
        responses
    });
    for (index, response) in tcp_responses.iter().enumerate() {
        let header = inspect_response_header(response).expect("TCP response header");
        assert_eq!(header.id, 0x7201 + u16::try_from(index).expect("TCP index"));
        validate_response(response).expect("valid TCP response");
    }
    assert_eq!(
        answer_addresses(&tcp_responses[0]),
        vec![std::net::IpAddr::V4(LOCAL_ANSWER.into())],
        "the exact full TCP rule must reach the local peer"
    );
    assert_eq!(
        answer_addresses(&tcp_responses[1]),
        vec![std::net::IpAddr::V4(DEFAULT_ANSWER.into())],
        "a TCP subdomain must use the default peer"
    );
    assert_eq!(local.requests(), 2, "one local request per listener");
    assert_eq!(default.requests(), 2, "one default request per listener");
    assert!(
        tcp_assembly.audit_snapshot().records.is_empty(),
        "audit off must retain no per-query record"
    );

    local.stop();
    default.stop();
}
