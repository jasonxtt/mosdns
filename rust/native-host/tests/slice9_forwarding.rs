use mosdns_native_host::compile_yaml;

fn yaml(entries: &str, args: &str, exec: &str) -> String {
    format!(
        "log:\n  level: error\nplugins:\n  - tag: upstream\n    type: forward\n    args:\n{args}      upstreams:\n{entries}  - tag: entry\n    type: sequence\n    args:\n      - exec: '{exec}'\n  - tag: listener\n    type: udp_server\n    args:\n      entry: entry\n      listen: 127.0.0.1:15400\n      enable_audit: true\n"
    )
}

#[test]
fn ordered_tagged_subset_compiles_without_extending_sequence_external() {
    let source = yaml(
        "        - {tag: a, addr: '127.0.0.1:15453'}\n        - {tag: b, addr: 'tcp://127.0.0.1:15454'}\n",
        "      concurrent: 3\n",
        "$upstream b a",
    );
    let config = compile_yaml(&source).expect("multi-entry ordered subset must compile before I/O");
    assert_eq!(config.forward_definitions.len(), 1);
    assert_eq!(config.forward_invocations.len(), 1);
    assert_eq!(config.forward_invocations[0].entries, [1, 0]);
    assert_eq!(config.forward_definitions[0].entries[1].identity, "b");
    let external = config.forward_invocations[0].executable;
    assert!(
        config
            .program
            .externals
            .iter()
            .any(|(id, _)| *id == external)
    );
}

#[test]
fn quick_forward_uses_the_same_descriptor_and_three_entry_cap() {
    let source = "log:\n  level: error\nplugins:\n  - tag: configured\n    type: forward\n    args:\n      upstreams:\n        - {addr: '127.0.0.1:15453'}\n  - tag: entry\n    type: sequence\n    args:\n      - exec: [\"forward 127.0.0.1:15454 127.0.0.1:15455 127.0.0.1:15456 127.0.0.1:15457\", \"$configured\"]\n  - tag: listener\n    type: udp_server\n    args:\n      entry: entry\n      listen: 127.0.0.1:15400\n      enable_audit: true\n";
    let config = compile_yaml(source).expect("quick forward must compile");
    assert_eq!(config.forward_definitions.len(), 2);
    assert_eq!(config.forward_definitions[1].concurrent, 3);
    assert_eq!(config.forward_definitions[1].entries.len(), 4);
    assert_eq!(
        config.forward_definitions[1].entries[0].identity,
        "@native-quick:656e747279:0:0:0"
    );
    assert_eq!(config.forward_invocations[0].entries, [0, 1, 2, 3]);
    assert_eq!(
        config.forward_invocations[0].entries,
        (0..4).collect::<Vec<_>>()
    );
}

fn query() -> Vec<u8> {
    let mut wire = vec![0x91, 0x01, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    wire.extend_from_slice(b"\x04race\x04test\x00\x00\x01\x00\x01");
    wire
}

fn response_base(mut wire: Vec<u8>) -> Vec<u8> {
    let mut position = 12;
    while wire[position] != 0 {
        position += 1 + usize::from(wire[position]);
    }
    wire.truncate(position + 5);
    wire[4..6].copy_from_slice(&1u16.to_be_bytes());
    wire[6..12].fill(0);
    wire
}

fn answer(wire: Vec<u8>, ip: bool) -> Vec<u8> {
    let mut wire = response_base(wire);
    wire[2] = 0x81;
    wire[3] = 0x80;
    if ip {
        wire[7] = 1;
        wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 19]);
    }
    wire
}

fn answer_ip(mut wire: Vec<u8>, address: [u8; 4]) -> Vec<u8> {
    wire = response_base(wire);
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
    wire.extend_from_slice(&address);
    wire
}

fn no_answer(mut wire: Vec<u8>) -> Vec<u8> {
    wire = response_base(wire);
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire
}

fn truncated(mut wire: Vec<u8>) -> Vec<u8> {
    wire = response_base(wire);
    wire[2] = 0x83;
    wire[3] = 0x80;
    wire
}

fn query_qtype(packet: &[u8]) -> u16 {
    let mut position = 12;
    while packet[position] != 0 {
        position += 1 + usize::from(packet[position]);
    }
    position += 1;
    u16::from_be_bytes([packet[position], packet[position + 1]])
}

#[test]
fn multi_entry_listener_selects_ip_answer_and_accounts_each_started_entry() {
    use mosdns_native_host::{HostAssembly, UdpServer};
    use mosdns_upstream_core::TransportCancellation;
    use std::time::Duration;
    let a = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let b = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    a.set_nonblocking(true).unwrap();
    b.set_nonblocking(true).unwrap();
    let b_address = b.local_addr().unwrap();
    let source = yaml(
        &format!(
            "        - {{tag: a, addr: 'udp://{}'}}\n        - {{tag: b, addr: 'udp://{}'}}\n",
            a.local_addr().unwrap(),
            b.local_addr().unwrap()
        ),
        "      concurrent: 3\n",
        "$upstream",
    );
    let host = HostAssembly::from_yaml(&source).unwrap();
    host.block_on(async {
        let a = tokio::net::UdpSocket::from_std(a).unwrap();
        let b = tokio::net::UdpSocket::from_std(b).unwrap();
        let peers = [a, b]
            .into_iter()
            .enumerate()
            .map(|(index, peer)| {
                tokio::task::spawn_local(async move {
                    let mut buffer = [0; 512];
                    let (n, client) = peer.recv_from(&mut buffer).await.unwrap();
                    peer.send_to(&answer(buffer[..n].to_vec(), index == 1), client)
                        .await
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let address = server.local_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let service = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(&query(), address).await.unwrap();
        let mut buffer = [0; 512];
        let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        shutdown.cancel();
        service.await.unwrap().unwrap();
        for peer in peers {
            peer.abort();
            let _ = peer.await;
        }
        assert_eq!(
            mosdns_dns_core::observe_answer_addresses(&buffer[..n]).unwrap(),
            vec!["192.0.2.19".parse::<std::net::IpAddr>().unwrap()]
        );
    });
    let snapshot = host.audit_snapshot();
    assert_eq!(
        snapshot.records[0].selected_upstream.as_deref(),
        Some(b_address.to_string().as_str())
    );
    assert_eq!(snapshot.records[0].upstream_attempts.len(), 2);
    let diagnostics = snapshot.records[0]
        .upstream_diagnostics
        .as_ref()
        .expect("native schema 1 diagnostics");
    assert_eq!(diagnostics.schema_version, 1);
    assert_eq!(diagnostics.attempts.len(), 2);
    assert_eq!(diagnostics.attempts[0].ordinal, 0);
    assert_eq!(diagnostics.attempts[0].entry, "a");
    assert_eq!(diagnostics.attempts[1].entry, "b");
    assert_eq!(diagnostics.selected.as_ref().unwrap().entry, "b");
    assert_eq!(diagnostics.selected.as_ref().unwrap().peer, b_address);
    assert_eq!(
        diagnostics.selected.as_ref().unwrap().transport,
        mosdns_native_host::UpstreamTransport::Udp
    );
}

#[test]
fn udp_truncation_reuses_one_entry_and_finishes_over_tcp() {
    use mosdns_native_host::{HostAssembly, UdpServer};
    use mosdns_upstream_core::TransportCancellation;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = udp.local_addr().unwrap();
    let tcp = std::net::TcpListener::bind(address).unwrap();
    udp.set_nonblocking(true).unwrap();
    tcp.set_nonblocking(true).unwrap();
    let source = yaml(
        &format!("        - {{tag: tc, addr: 'udp://{address}'}}\n"),
        "",
        "$upstream",
    );
    let host = HostAssembly::from_yaml(&source).unwrap();
    host.block_on(async {
        let udp = tokio::net::UdpSocket::from_std(udp).unwrap();
        let tcp = tokio::net::TcpListener::from_std(tcp).unwrap();
        let udp_task = tokio::task::spawn_local(async move {
            let mut packet = [0; 512];
            let (length, peer) = udp.recv_from(&mut packet).await.unwrap();
            udp.send_to(&truncated(packet[..length].to_vec()), peer)
                .await
                .unwrap();
        });
        let tcp_task = tokio::task::spawn_local(async move {
            let (mut stream, _) = tcp.accept().await.unwrap();
            let length = stream.read_u16().await.unwrap() as usize;
            let mut packet = vec![0; length];
            stream.read_exact(&mut packet).await.unwrap();
            let response = answer(packet, true);
            stream
                .write_u16(u16::try_from(response.len()).unwrap())
                .await
                .unwrap();
            stream.write_all(&response).await.unwrap();
        });
        let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let listen = server.local_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let service = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(&query(), listen).await.unwrap();
        let mut buffer = [0; 512];
        let length = tokio::time::timeout(Duration::from_secs(3), client.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(3), service)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        udp_task.await.unwrap();
        tcp_task.await.unwrap();
        assert_eq!(
            mosdns_dns_core::observe_answer_addresses(&buffer[..length]).unwrap(),
            vec!["192.0.2.19".parse::<std::net::IpAddr>().unwrap()]
        );
    });
    let snapshot = host.audit_snapshot();
    let diagnostics = snapshot.records[0]
        .upstream_diagnostics
        .as_ref()
        .expect("native diagnostics");
    assert_eq!(diagnostics.attempts.len(), 1);
    assert_eq!(
        diagnostics.attempts[0].transport,
        Some(mosdns_native_host::UpstreamTransport::Tcp)
    );
    assert_eq!(
        diagnostics.selected.as_ref().unwrap().transport,
        mosdns_native_host::UpstreamTransport::Tcp
    );
}

#[test]
fn native_http_log_exposes_versioned_upstream_diagnostics() {
    use mosdns_native_host::{HostAssembly, HostOptions};
    use mosdns_upstream_core::TransportCancellation;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let upstream = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    upstream.set_nonblocking(true).unwrap();
    let api_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dns_port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let source = format!(
        "log:\n  level: error\napi:\n  http: 127.0.0.1:{api_port}\nplugins:\n  - tag: upstream\n    type: forward\n    args:\n      upstreams:\n        - {{tag: primary, addr: 'udp://{upstream_address}'}}\n  - tag: entry\n    type: sequence\n    args:\n      - exec: $upstream\n  - tag: listener\n    type: udp_server\n    args:\n      entry: entry\n      listen: 127.0.0.1:{dns_port}\n      enable_audit: true\n"
    );
    let host = HostAssembly::with_options(
        mosdns_native_host::compile_yaml(&source).unwrap(),
        HostOptions::default(),
    )
    .unwrap();
    let bound = host.block_on(host.bind_host()).unwrap();
    let dns = bound.dns_addr();
    let api = bound.api_addr().unwrap();
    let shutdown = TransportCancellation::new();
    host.block_on(async {
        let upstream = tokio::net::UdpSocket::from_std(upstream).unwrap();
        let upstream_task = tokio::task::spawn_local(async move {
            let mut packet = [0; 512];
            let (length, peer) = upstream.recv_from(&mut packet).await.unwrap();
            upstream
                .send_to(&answer(packet[..length].to_vec(), true), peer)
                .await
                .unwrap();
        });
        let service = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(&query(), dns).await.unwrap();
        let mut response = [0; 512];
        tokio::time::timeout(Duration::from_secs(2), client.recv(&mut response))
            .await
            .unwrap()
            .unwrap();

        let mut http = tokio::net::TcpStream::connect(api).await.unwrap();
        http.write_all(
            b"GET /api/v2/audit/logs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut body = Vec::new();
        http.read_to_end(&mut body).await.unwrap();
        shutdown.cancel();
        service.await.unwrap().unwrap();
        upstream_task.await.unwrap();
        let body = String::from_utf8(body).unwrap();
        let json = body.split("\r\n\r\n").nth(1).unwrap();
        let payload: serde_json::Value = serde_json::from_str(json).unwrap();
        let diagnostics = &payload["logs"][0]["upstream_diagnostics"];
        assert_eq!(diagnostics["schema_version"], 1);
        assert_eq!(diagnostics["selected"]["entry"], "primary");
        assert_eq!(
            diagnostics["selected"]["peer"],
            upstream_address.to_string()
        );
        assert_eq!(diagnostics["selected"]["transport"], "udp");
        assert_eq!(diagnostics["attempts"][0]["ordinal"], 0);
        assert_eq!(diagnostics["attempts"][0]["outcome"], "response");
    });
}

#[test]
fn endpoint_descriptors_preserve_defaults_inheritance_and_secure_identity() {
    let source = yaml(
        "        - {tag: dot, addr: 'tls://dns.example', dial_addr: '127.0.0.1:1853', bootstrap_version: 6, insecure_skip_verify: true}\n        - {tag: doh, addr: 'https://dns.example:9443/dns-query?edns=1', dial_addr: '127.0.0.1:19443', insecure_skip_verify: true}\n",
        "      bootstrap: 127.0.0.1\n      bootstrap_version: 4\n      concurrent: 9\n",
        "$upstream dot",
    );
    let config = compile_yaml(&source).expect("secure descriptor must compile");
    let definition = &config.forward_definitions[0];
    assert_eq!(definition.concurrent, 3);
    assert_eq!(definition.entries[0].target.port, 853);
    assert_eq!(definition.entries[0].target.bootstrap_version, Some(6));
    assert_eq!(
        definition.entries[0].target.dial_addr,
        Some("127.0.0.1:1853".parse().unwrap())
    );
    assert_eq!(definition.entries[1].target.port, 9443);
    assert_eq!(
        definition.entries[1].target.service,
        "https://dns.example:9443/dns-query?edns=1"
    );
    assert_eq!(definition.entries[1].target.bootstrap_version, Some(4));
}

#[test]
fn hostname_forward_requires_numeric_bootstrap_or_dial_and_rejects_unsupported_options() {
    let missing = yaml("        - {addr: 'dns.example'}\n", "", "$upstream");
    let Err(error) = compile_yaml(&missing) else {
        panic!("hostname without bootstrap must fail")
    };
    assert!(error.reason.contains("numeric dial_addr or bootstrap"));

    let unsupported = yaml(
        "        - {addr: '127.0.0.1:53', max_conns: 2}\n",
        "",
        "$upstream",
    );
    let Err(error) = compile_yaml(&unsupported) else {
        panic!("unsupported options must fail")
    };
    assert!(error.reason.contains("unsupported"));
}

#[test]
fn hostname_forward_uses_controlled_bootstrap_and_target_peers() {
    use mosdns_native_host::{HostAssembly, UdpServer};
    use mosdns_upstream_core::TransportCancellation;
    use std::time::Duration;

    let bootstrap = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let target = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    bootstrap.set_nonblocking(true).unwrap();
    target.set_nonblocking(true).unwrap();
    let bootstrap_address = bootstrap.local_addr().unwrap();
    let target_address = target.local_addr().unwrap();
    let source = yaml(
        &format!(
            "        - {{tag: host, addr: 'dns.example:{}'}}\n",
            target_address.port()
        ),
        &format!("      bootstrap: {bootstrap_address}\n"),
        "$upstream",
    );
    let host = HostAssembly::from_yaml(&source).unwrap();
    host.block_on(async {
        let bootstrap = tokio::net::UdpSocket::from_std(bootstrap).unwrap();
        let target = tokio::net::UdpSocket::from_std(target).unwrap();
        let bootstrap_task = tokio::task::spawn_local(async move {
            let mut packet = [0; 512];
            for _ in 0..2 {
                let (length, peer) = bootstrap.recv_from(&mut packet).await.unwrap();
                let is_a = query_qtype(&packet[..length]) == 1;
                let response = if is_a {
                    answer_ip(packet[..length].to_vec(), [127, 0, 0, 1])
                } else {
                    no_answer(packet[..length].to_vec())
                };
                bootstrap.send_to(&response, peer).await.unwrap();
            }
        });
        let target_task = tokio::task::spawn_local(async move {
            let mut packet = [0; 512];
            let (length, peer) = target.recv_from(&mut packet).await.unwrap();
            target
                .send_to(&answer(packet[..length].to_vec(), true), peer)
                .await
                .unwrap();
        });
        let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let address = server.local_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let service = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(&query(), address).await.unwrap();
        let mut buffer = [0; 512];
        let length = tokio::time::timeout(Duration::from_secs(3), client.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(3), service)
            .await
            .expect("server drain")
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), bootstrap_task)
            .await
            .expect("bootstrap fixture drain")
            .unwrap();
        assert_eq!(
            mosdns_dns_core::observe_answer_addresses(&buffer[..length]).unwrap(),
            vec!["192.0.2.19".parse::<std::net::IpAddr>().unwrap()]
        );
        tokio::time::timeout(Duration::from_secs(3), target_task)
            .await
            .expect("target fixture drain")
            .unwrap();
    });
    let snapshot = host.audit_snapshot();
    assert_eq!(
        snapshot.records[0].selected_upstream.as_deref(),
        Some(target_address.to_string().as_str())
    );
}

struct SecureFixture {
    root: rustls::pki_types::CertificateDer<'static>,
    leaf: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
}

fn secure_fixture() -> SecureFixture {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
        KeyPair, KeyUsagePurpose, SanType, date_time_ymd,
    };

    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut root_params = CertificateParams::default();
    root_params
        .distinguished_name
        .push(DnType::CommonName, "native-forwarding-test-root");
    root_params.not_before = date_time_ymd(2024, 1, 1);
    root_params.not_after = date_time_ymd(2036, 1, 1);
    root_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let root = root_params.self_signed(&root_key).unwrap();
    let issuer = Issuer::from_params(&root_params, &root_key);

    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut leaf_params = CertificateParams::default();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "dns.example");
    leaf_params.subject_alt_names = vec![SanType::DnsName("dns.example".try_into().unwrap())];
    leaf_params.not_before = date_time_ymd(2024, 1, 1);
    leaf_params.not_after = date_time_ymd(2036, 1, 1);
    leaf_params.is_ca = IsCa::ExplicitNoCa;
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

    SecureFixture {
        root: root.der().clone(),
        leaf: leaf.der().clone(),
        key: rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
    }
}

fn secure_server_config(
    fixture: &SecureFixture,
    alpn_protocols: &[&[u8]],
) -> std::sync::Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![fixture.leaf.clone(), fixture.root.clone()],
        fixture.key.clone_key(),
    )
    .unwrap();
    config.alpn_protocols = alpn_protocols
        .iter()
        .map(|protocol| (*protocol).to_vec())
        .collect();
    std::sync::Arc::new(config)
}

fn root_store(fixture: &SecureFixture) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(fixture.root.clone()).unwrap();
    roots
}

fn secure_yaml(scheme: &str, dial: std::net::SocketAddr) -> String {
    let addr = if scheme == "tls" {
        "tls://dns.example".to_owned()
    } else {
        "https://dns.example/dns-query".to_owned()
    };
    yaml(
        &format!("        - {{tag: secure, addr: '{addr}', dial_addr: '{dial}'}}\n"),
        "",
        "$upstream",
    )
}

async fn query_host(host: &mosdns_native_host::HostAssembly) -> Vec<u8> {
    use mosdns_native_host::UdpServer;
    use mosdns_upstream_core::TransportCancellation;
    use std::time::Duration;

    let server = UdpServer::bind(host, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let listen = server.local_addr().unwrap();
    let shutdown = TransportCancellation::new();
    let service = tokio::task::spawn_local(server.serve(shutdown.clone()));
    let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.send_to(&query(), listen).await.unwrap();
    let mut response = [0; 512];
    let length = tokio::time::timeout(Duration::from_secs(3), client.recv(&mut response))
        .await
        .unwrap()
        .unwrap();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(3), service)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    response[..length].to_vec()
}

async fn serve_dot(listener: std::net::TcpListener, config: std::sync::Arc<rustls::ServerConfig>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsAcceptor;

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let mut stream = TlsAcceptor::from(config).accept(stream).await.unwrap();
    let length = stream.read_u16().await.unwrap() as usize;
    let mut packet = vec![0; length];
    stream.read_exact(&mut packet).await.unwrap();
    let response = answer_ip(packet, [192, 0, 2, 53]);
    stream
        .write_u16(u16::try_from(response.len()).unwrap())
        .await
        .unwrap();
    stream.write_all(&response).await.unwrap();
}

fn doh_query(target: &str) -> Vec<u8> {
    use base64::Engine as _;

    let encoded = target
        .split_once("dns=")
        .unwrap()
        .1
        .split('&')
        .next()
        .unwrap();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .unwrap()
}

async fn serve_doh_h1(
    listener: std::net::TcpListener,
    config: std::sync::Arc<rustls::ServerConfig>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsAcceptor;

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let mut stream = TlsAcceptor::from(config).accept(stream).await.unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0; 512];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let length = stream.read(&mut chunk).await.unwrap();
        assert_ne!(length, 0);
        buffer.extend_from_slice(&chunk[..length]);
    }
    let head = String::from_utf8(buffer).unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let response = answer_ip(doh_query(target), [192, 0, 2, 54]);
    let header = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/dns-message\r\ncontent-length: {}\r\n\r\n",
        response.len()
    );
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.write_all(&response).await.unwrap();
    stream.flush().await.unwrap();
    let mut scratch = [0; 64];
    let _ =
        tokio::time::timeout(std::time::Duration::from_secs(3), stream.read(&mut scratch)).await;
}

async fn serve_doh_h2(
    listener: std::net::TcpListener,
    config: std::sync::Arc<rustls::ServerConfig>,
) {
    use bytes::Bytes;
    use h2::server;
    use tokio_rustls::TlsAcceptor;

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let tls = TlsAcceptor::from(config).accept(stream).await.unwrap();
    let mut connection = server::handshake(tls).await.unwrap();
    let Some(Ok((request, mut respond))) = connection.accept().await else {
        panic!("DoH H2 request was not received")
    };
    let target = request.uri().path_and_query().unwrap().as_str().to_owned();
    let response = answer_ip(doh_query(&target), [192, 0, 2, 55]);
    let head = http::Response::builder()
        .status(200)
        .header("content-type", "application/dns-message")
        .body(())
        .unwrap();
    let mut send = respond.send_response(head, false).unwrap();
    send.send_data(Bytes::from(response), true).unwrap();
    let _ = connection.accept().await;
}

#[test]
#[allow(clippy::too_many_lines)]
fn native_secure_forwarding_proves_dot_and_doh_h1_h2_with_synthetic_ca() {
    use mosdns_native_host::{HostAssembly, HostOptions};

    let fixture = std::sync::Arc::new(secure_fixture());
    let cases = [
        ("tls", "dot", 53u8),
        ("https", "h1", 54u8),
        ("https", "h2", 55u8),
    ];
    for (scheme, protocol, marker) in cases {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dial = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = match protocol {
            "dot" => secure_server_config(&fixture, &[]),
            "h1" => secure_server_config(&fixture, &[b"http/1.1"]),
            "h2" => secure_server_config(&fixture, &[b"h2"]),
            _ => unreachable!(),
        };
        let host = HostAssembly::with_options(
            mosdns_native_host::compile_yaml(&secure_yaml(scheme, dial)).unwrap(),
            HostOptions::default().with_tls_roots(root_store(&fixture)),
        )
        .unwrap();
        host.block_on(async {
            let fixture_task = match protocol {
                "dot" => tokio::task::spawn_local(serve_dot(listener, config)),
                "h1" => tokio::task::spawn_local(serve_doh_h1(listener, config)),
                "h2" => tokio::task::spawn_local(serve_doh_h2(listener, config)),
                _ => unreachable!(),
            };
            let response = query_host(&host).await;
            fixture_task.await.unwrap();
            assert_eq!(
                mosdns_dns_core::observe_answer_addresses(&response).unwrap(),
                vec![
                    format!("192.0.2.{marker}")
                        .parse::<std::net::IpAddr>()
                        .unwrap()
                ]
            );
        });
        let expected_transport = if protocol == "dot" {
            mosdns_native_host::UpstreamTransport::Tls
        } else {
            mosdns_native_host::UpstreamTransport::Https
        };
        let snapshot = host.audit_snapshot();
        let diagnostics = snapshot.records[0].upstream_diagnostics.as_ref().unwrap();
        assert_eq!(
            diagnostics.selected.as_ref().unwrap().transport,
            expected_transport
        );
        assert_eq!(
            diagnostics.attempts[0].outcome,
            mosdns_native_host::UpstreamAttemptOutcome::Response
        );
    }
}
