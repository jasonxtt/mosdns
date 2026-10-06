use mosdns_native_host::{HostAssembly, load_and_compile};
use mosdns_upstream_core::TransportCancellation;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture(PathBuf);
impl Fixture {
    fn new(port: u16, upstream: u16) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "managed-supervisor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for sub in ["webinfo", "rule", "cache"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(
            root.join("webinfo/special_upstream_groups.json"),
            serde_json::to_vec(
                &json!([{"slot":50,"name":"group","listen_port":port,"custom_port_only":true}]),
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("webinfo/upstream_overrides.json"),serde_json::to_vec(&json!({"special_upstream_50":[{"tag":"controlled","protocol":"udp","addr":format!("127.0.0.1:{upstream}"),"enabled":true}]})).unwrap()).unwrap();
        std::fs::write(root.join("config.yaml"),"log: {level: error}\nnative_management: {special_groups: true}\nplugins:\n  - tag: main_entry\n    type: sequence\n    args: [{exec: $special_upstream_matcher}, {exec: reject 3}]\n  - tag: main\n    type: udp_server\n    args: {entry: main_entry, listen: '127.0.0.1:0', enable_audit: true}\n").unwrap();
        let config_path = root.join("config.yaml");
        let yaml = std::fs::read_to_string(&config_path)
            .unwrap()
            .replace("127.0.0.1:0", &format!("127.0.0.1:{}", free_pair()));
        std::fs::write(config_path, yaml).unwrap();
        Self(root)
    }
    fn assembly(&self) -> HostAssembly {
        HostAssembly::from_config(load_and_compile(&self.0.join("config.yaml")).unwrap()).unwrap()
    }

    fn enable_api(&self, port: u16) {
        let config = self.0.join("config.yaml");
        let contents = std::fs::read_to_string(&config).unwrap();
        std::fs::write(
            config,
            format!("api: {{http: '127.0.0.1:{port}'}}\n{contents}"),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn query() -> Vec<u8> {
    vec![1, 2, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'x', 0, 0, 1, 0, 1]
}
fn free_pair() -> u16 {
    loop {
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

async fn http_request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, String) {
    let (status, body) = http_request_bytes(address, method, path, body.as_bytes()).await;
    (status, String::from_utf8(body).unwrap())
}

async fn http_request_bytes(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = std::str::from_utf8(&bytes[..split]).unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, bytes[split + 4..].to_vec())
}

#[test]
fn custom_udp_tcp_pair_serves_the_group_without_main_matching() {
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let port = free_pair();
    let fixture = Fixture::new(port, peer.local_addr().unwrap().port());
    let host = fixture.assembly();
    host.block_on(async {
        let peer=tokio::net::UdpSocket::from_std(peer).unwrap();
        let peer_stop = TransportCancellation::new();
        let peer_scope = peer_stop.clone();
        let upstream=tokio::task::spawn_local(async move {
            loop { let mut packet=vec![0;4096]; let received = tokio::select! { biased; () = peer_scope.cancelled() => break, received = peer.recv_from(&mut packet) => received }; let (n,addr)=received.unwrap(); let (h,q)=mosdns_dns_core::parse_query(&packet[..n]).unwrap(); let wire=mosdns_dns_core::synthesize_response(&h,&q,0).unwrap(); peer.send_to(&wire,addr).await.unwrap(); }
        });
        let bound=host.bind_host().await.unwrap(); let main=bound.dns_addr(); let shutdown=TransportCancellation::new();
        let serving=tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let udp=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap(); let mut packet=vec![0;4096];
        udp.send_to(&query(),("127.0.0.1",port)).await.unwrap();
        let (n,_)=tokio::time::timeout(Duration::from_secs(2),udp.recv_from(&mut packet)).await.unwrap().unwrap(); assert_eq!(packet[3]&15,0); mosdns_dns_core::validate_response(&packet[..n]).unwrap();
        let mut tcp=tokio::net::TcpStream::connect(("127.0.0.1",port)).await.unwrap(); let wire=query(); tcp.write_all(&u16::try_from(wire.len()).unwrap().to_be_bytes()).await.unwrap();tcp.write_all(&wire).await.unwrap();
        let mut prefix=[0;2]; tokio::time::timeout(Duration::from_secs(2),tcp.read_exact(&mut prefix)).await.unwrap().unwrap(); let mut response=vec![0;u16::from_be_bytes(prefix) as usize];tcp.read_exact(&mut response).await.unwrap();assert_eq!(response[3]&15,0);
        udp.send_to(&query(),main).await.unwrap();udp.recv_from(&mut packet).await.unwrap();assert_eq!(packet[3]&15,3);
        while host.metrics_snapshot().completed_total<3 {tokio::task::yield_now().await;}
        let audit=host.audit_snapshot(); assert_eq!(audit.records.iter().filter(|r|r.matched_group.as_deref()==Some("special_50")).count(),2);
        shutdown.cancel();assert!(serving.await.unwrap().is_ok());peer_stop.cancel();upstream.await.unwrap();
        assert!(std::net::TcpListener::bind(("127.0.0.1",port)).is_ok());assert!(std::net::UdpSocket::bind(("127.0.0.1",port)).is_ok());
    });
}

#[test]
fn occupied_tcp_rolls_back_the_prebound_udp_half() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    let fixture = Fixture::new(port, 15999);
    let host = fixture.assembly();
    host.block_on(async {
        assert!(host.bind_host().await.is_err());
        assert!(std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok());
    });
}

#[test]
fn in_flight_uses_old_snapshot_and_next_tcp_frame_uses_new_snapshot() {
    let old_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let new_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    old_peer.set_nonblocking(true).unwrap();
    new_peer.set_nonblocking(true).unwrap();
    let new_addr = new_peer.local_addr().unwrap();
    let port = free_pair();
    let fixture = Fixture::new(port, old_peer.local_addr().unwrap().port());
    let host = fixture.assembly();
    host.block_on(async {
        let old_peer = tokio::net::UdpSocket::from_std(old_peer).unwrap();
        let new_peer = tokio::net::UdpSocket::from_std(new_peer).unwrap();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, paused) = tokio::sync::oneshot::channel();
        let old_task = tokio::task::spawn_local(async move {
            let mut packet = vec![0; 4096]; let (n, addr) = old_peer.recv_from(&mut packet).await.unwrap();
            let (header, question) = mosdns_dns_core::parse_query(&packet[..n]).unwrap();
            started.send(()).unwrap(); paused.await.unwrap();
            old_peer.send_to(&mosdns_dns_core::synthesize_response(&header, &question, 0).unwrap(), addr).await.unwrap();
        });
        let new_task = tokio::task::spawn_local(async move {
            let mut packet = vec![0; 4096]; let (n, addr) = new_peer.recv_from(&mut packet).await.unwrap();
            let (header, question) = mosdns_dns_core::parse_query(&packet[..n]).unwrap();
            new_peer.send_to(&mosdns_dns_core::synthesize_response(&header, &question, 2).unwrap(), addr).await.unwrap();
        });
        let bound = host.bind_host().await.unwrap(); let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let wire = query(); let length = u16::try_from(wire.len()).unwrap().to_be_bytes();
        stream.write_all(&length).await.unwrap(); stream.write_all(&wire).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
        std::fs::write(fixture.0.join("webinfo/upstream_overrides.json"), serde_json::to_vec(&json!({"special_upstream_50":[{"tag":"replacement","protocol":"udp","addr":new_addr.to_string(),"enabled":true}]})).unwrap()).unwrap();
        let config_path = fixture.0.join("config.yaml");
        let yaml = std::fs::read_to_string(&config_path).unwrap().replace("plugins:\n", "plugins:\n  - tag: inserted\n    type: sequence\n    args: [{exec: return}]\n");
        std::fs::write(&config_path, yaml).unwrap();
        let control = host.control(); let candidate = control.prepare(load_and_compile(&config_path).unwrap()).unwrap();
        assert_eq!(control.install(candidate).unwrap(), 1);
        assert!(control.prepare(load_and_compile(&config_path).unwrap()).is_err());
        release.send(()).unwrap();
        let mut prefix = [0;2]; stream.read_exact(&mut prefix).await.unwrap();
        let mut response = vec![0;usize::from(u16::from_be_bytes(prefix))]; stream.read_exact(&mut response).await.unwrap(); assert_eq!(response[3]&15,0);
        stream.write_all(&length).await.unwrap(); stream.write_all(&wire).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut prefix)).await.unwrap().unwrap();
        response.resize(usize::from(u16::from_be_bytes(prefix)),0);stream.read_exact(&mut response).await.unwrap();assert_eq!(response[3]&15,2);
        control.retire().await.unwrap();
        while host.metrics_snapshot().completed_total != 2 {tokio::task::yield_now().await;}
        let audit = host.audit_snapshot(); assert_eq!(audit.records[0].final_upstream.as_deref(),Some("controlled"));assert_eq!(audit.records[1].final_upstream.as_deref(),Some("replacement"));
        let metrics = host.metrics_snapshot(); assert_eq!(metrics.forward_attempts_by_upstream["controlled"].responses_total,1);assert_eq!(metrics.forward_attempts_by_upstream["replacement"].responses_total,1);
        shutdown.cancel(); assert!(serving.await.unwrap().is_ok()); old_task.await.unwrap();new_task.await.unwrap();
    });
}

#[test]
fn running_port_change_prebinds_both_halves_and_drains_only_removed_sockets() {
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let old_port = free_pair();
    let fixture = Fixture::new(old_port, peer.local_addr().unwrap().port());
    let host = fixture.assembly();
    host.block_on(async {
        let peer = tokio::net::UdpSocket::from_std(peer).unwrap(); let stop_peer = TransportCancellation::new();let peer_scope=stop_peer.clone();
        let queries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_queries = queries.clone();
        let upstream = tokio::task::spawn_local(async move { loop {
            let mut packet = vec![0;4096]; let received=tokio::select! { ()=peer_scope.cancelled()=>break, r=peer.recv_from(&mut packet)=>r };let (n,addr)=received.unwrap();
            observed_queries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (h,q)=mosdns_dns_core::parse_query(&packet[..n]).unwrap();peer.send_to(&mosdns_dns_core::synthesize_response(&h,&q,0).unwrap(),addr).await.unwrap();
        }});
        let bound=host.bind_host().await.unwrap();let main=bound.dns_addr();let shutdown=TransportCancellation::new();let serving=tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let udp=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let mut response=vec![0;4096];
        udp.send_to(&query(),("127.0.0.1",old_port)).await.unwrap();tokio::time::timeout(Duration::from_secs(2),udp.recv_from(&mut response)).await.unwrap().unwrap();assert_eq!(response[3]&15,0);
        let mut idle=tokio::net::TcpStream::connect(("127.0.0.1",old_port)).await.unwrap();
        let occupied=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let blocked_port=occupied.local_addr().unwrap().port();
        let update=|port| std::fs::write(fixture.0.join("webinfo/special_upstream_groups.json"),serde_json::to_vec(&json!([{"slot":50,"name":"group","listen_port":port,"custom_port_only":true}])).unwrap()).unwrap();
        let control=host.control();update(blocked_port);
        assert!(control.prepare_host(load_and_compile(&fixture.0.join("config.yaml")).unwrap()).await.is_err());assert_eq!(control.generation(),0);
        assert!(std::net::UdpSocket::bind(("127.0.0.1",blocked_port)).is_ok());
        udp.send_to(&query(),("127.0.0.1",old_port)).await.unwrap();udp.recv_from(&mut response).await.unwrap();assert_eq!(response[3]&15,0);
        let new_port=free_pair();update(new_port);
        let candidate=control.prepare_host(load_and_compile(&fixture.0.join("config.yaml")).unwrap()).await.unwrap();assert_eq!(control.generation(),0);
        assert!(std::net::TcpListener::bind(("127.0.0.1",new_port)).is_err());assert!(std::net::UdpSocket::bind(("127.0.0.1",new_port)).is_err());
        control.install(candidate).unwrap();control.retire().await.unwrap();
        assert!(std::net::TcpListener::bind(("127.0.0.1",old_port)).is_ok());assert!(std::net::UdpSocket::bind(("127.0.0.1",old_port)).is_ok());
        assert_eq!(tokio::time::timeout(Duration::from_secs(2),idle.read(&mut [0;1])).await.unwrap().unwrap(),0);
        udp.send_to(&query(),("127.0.0.1",new_port)).await.unwrap();tokio::time::timeout(Duration::from_secs(2),udp.recv_from(&mut response)).await.unwrap().unwrap();assert_eq!(response[3]&15,0);
        let mut tcp=tokio::net::TcpStream::connect(("127.0.0.1",new_port)).await.unwrap();let q=query();tcp.write_all(&u16::try_from(q.len()).unwrap().to_be_bytes()).await.unwrap();tcp.write_all(&q).await.unwrap();let mut prefix=[0;2];tokio::time::timeout(Duration::from_secs(2),tcp.read_exact(&mut prefix)).await.unwrap().unwrap();let mut wire=vec![0;usize::from(u16::from_be_bytes(prefix))];tcp.read_exact(&mut wire).await.unwrap();assert_eq!(wire[3]&15,0);
        udp.send_to(&query(),main).await.unwrap();udp.recv_from(&mut response).await.unwrap();assert_eq!(response[3]&15,3);
        assert_eq!(queries.load(std::sync::atomic::Ordering::SeqCst), 1, "port-only change must preserve warmed cache owner");
        shutdown.cancel();assert!(serving.await.unwrap().is_ok());stop_peer.cancel();upstream.await.unwrap();
    });
}

#[test]
fn audit_can_be_enabled_by_a_later_managed_snapshot() {
    let fixture = Fixture::new(free_pair(), 15999);
    std::fs::write(fixture.0.join("webinfo/special_upstream_groups.json"), "[]").unwrap();
    std::fs::write(fixture.0.join("webinfo/upstream_overrides.json"), "{}").unwrap();
    let path = fixture.0.join("config.yaml");
    let original = format!(
        "{}  - tag: unused_forward\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:15999'}}]}}\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(
        &path,
        original.replace("enable_audit: true", "enable_audit: false"),
    )
    .unwrap();
    let host = fixture.assembly();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let addr = bound.dns_addr();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        tokio::task::yield_now().await;
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut answer = [0; 512];
        client.send_to(&query(), addr).await.unwrap();
        client.recv_from(&mut answer).await.unwrap();
        assert!(host.audit_snapshot().records.is_empty());
        std::fs::write(&path, original).unwrap();
        let control = host.control();
        let candidate = control
            .prepare_host(load_and_compile(&path).unwrap())
            .await
            .unwrap();
        control.install(candidate).unwrap();
        control.retire().await.unwrap();
        client.send_to(&query(), addr).await.unwrap();
        client.recv_from(&mut answer).await.unwrap();
        assert_eq!(host.audit_snapshot().records.len(), 1);
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn discarded_candidate_does_not_change_upstream_inventory() {
    let fixture = Fixture::new(free_pair(), 15999);
    let host = fixture.assembly();
    let before = host.metrics_snapshot().forward_attempts_by_upstream;
    let path = fixture.0.join("webinfo/upstream_overrides.json");
    let overrides = std::fs::read_to_string(&path)
        .unwrap()
        .replace("controlled", "candidate_only");
    std::fs::write(path, overrides).unwrap();
    let control = host.control();
    let candidate = control
        .prepare(load_and_compile(&fixture.0.join("config.yaml")).unwrap())
        .unwrap();
    drop(candidate);
    assert_eq!(control.generation(), 0);
    assert_eq!(host.metrics_snapshot().forward_attempts_by_upstream, before);
}

#[test]
fn stable_api_captures_the_snapshot_after_the_complete_http_request() {
    let fixture = Fixture::new(free_pair(), 15999);
    let path = fixture.0.join("config.yaml");
    let original = format!(
        "api: {{http: '127.0.0.1:{}'}}\n{}",
        free_pair(),
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &original).unwrap();
    let host = fixture.assembly();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let mut client = tokio::net::TcpStream::connect(api).await.unwrap();
        client
            .write_all(b"GET /api/v1/cache/inventory HTTP/1.1\r\nHost: localhost\r\n")
            .await
            .unwrap();
        tokio::task::yield_now().await;
        std::fs::write(
            &path,
            original.replace(
                "plugins:\n",
                "plugins:\n  - tag: candidate_cache\n    type: cache\n    args: {}\n",
            ),
        )
        .unwrap();
        let control = host.control();
        let candidate = control
            .prepare_host(load_and_compile(&path).unwrap())
            .await
            .unwrap();
        control.install(candidate).unwrap();
        control.retire().await.unwrap();
        client
            .write_all(b"Connection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("candidate_cache"), "{response}");
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn removing_another_group_preserves_the_live_tcp_upstream_owner() {
    let peer = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let peer_addr = peer.local_addr().unwrap();
    let port = free_pair();
    let fixture = Fixture::new(port, peer_addr.port());
    let groups = fixture.0.join("webinfo/special_upstream_groups.json");
    let overrides = fixture.0.join("webinfo/upstream_overrides.json");
    std::fs::write(
        &groups,
        serde_json::to_vec(&json!([
            {"slot":50,"name":"group","listen_port":port,"custom_port_only":true},
            {"slot":51,"name":"removed","listen_port":free_pair(),"custom_port_only":true}
        ]))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(&overrides, serde_json::to_vec(&json!({
        "special_upstream_50":[{"tag":"controlled","protocol":"tcp","addr":peer_addr.to_string(),"enabled":true}],
        "special_upstream_51":[{"tag":"other","protocol":"tcp","addr":peer_addr.to_string(),"enabled":true}]
    })).unwrap()).unwrap();
    let host = fixture.assembly();
    host.block_on(async {
        let peer = tokio::net::TcpListener::from_std(peer).unwrap();
        let accepts = std::rc::Rc::new(std::cell::Cell::new(0));
        let closed = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen_accepts = accepts.clone();
        let seen_closed = closed.clone();
        let peer_stop = TransportCancellation::new();
        let scope = peer_stop.clone();
        let upstream = tokio::task::spawn_local(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    () = scope.cancelled() => break,
                    accepted = peer.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        seen_accepts.set(seen_accepts.get()+1);
                        let closed = seen_closed.clone();
                        connections.spawn_local(async move {
                            loop {
                                let mut length = [0;2];
                                if stream.read_exact(&mut length).await.is_err() { closed.set(closed.get()+1); break; }
                                let mut wire = vec![0;usize::from(u16::from_be_bytes(length))];
                                stream.read_exact(&mut wire).await.unwrap();
                                let (header, question) = mosdns_dns_core::parse_query(&wire).unwrap();
                                let response = mosdns_dns_core::synthesize_response(&header, &question, 0).unwrap();
                                stream.write_all(&u16::try_from(response.len()).unwrap().to_be_bytes()).await.unwrap();
                                stream.write_all(&response).await.unwrap();
                            }
                        });
                    }
                }
            }
            while let Some(result) = connections.join_next().await { result.unwrap(); }
        });
        let bound = host.bind_host().await.unwrap();
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut response = [0;512];
        client.send_to(&query(), ("127.0.0.1",port)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2),client.recv_from(&mut response)).await.unwrap().unwrap();
        assert_eq!(accepts.get(),1);
        std::fs::write(&groups,serde_json::to_vec(&json!([{"slot":50,"name":"group","listen_port":port,"custom_port_only":true}])).unwrap()).unwrap();
        let control = host.control();
        let prepared = control.prepare_host(load_and_compile(&fixture.0.join("config.yaml")).unwrap()).await.unwrap();
        control.install(prepared).unwrap();
        control.retire().await.unwrap();
        let mut next = query(); next[13] = b'y';
        client.send_to(&next,("127.0.0.1",port)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2),client.recv_from(&mut response)).await.unwrap().unwrap();
        assert_eq!(response[3]&15,0);
        assert_eq!(accepts.get(),1,"unchanged transport must reuse its existing connection");
        assert_eq!(closed.get(),0,"retiring the other group must not close this owner");
        shutdown.cancel(); serving.await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(2),async { while closed.get()!=1 { tokio::task::yield_now().await; } }).await.unwrap();
        peer_stop.cancel(); upstream.await.unwrap();
        assert_eq!(closed.get(),1,"shared owner closes once at supervisor shutdown");
    });
}

#[test]
fn managed_publication_cannot_disable_the_frozen_opt_in() {
    let fixture = Fixture::new(free_pair(), 15999);
    let host = fixture.assembly();
    let path = fixture.0.join("config.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap()
        .replace("special_groups: true", "special_groups: false")
        .replace("{exec: $special_upstream_matcher}, ", "");
    std::fs::write(&path, format!("{yaml}  - tag: unused_forward\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:15999'}}]}}\n")).unwrap();
    let candidate = load_and_compile(&path).unwrap();
    assert!(host.control().prepare(candidate).is_err());
    assert_eq!(host.control().generation(), 0);
}

#[test]
fn api_failure_releases_all_main_and_custom_listener_sockets() {
    let port = free_pair();
    let fixture = Fixture::new(port, 15999);
    let path = fixture.0.join("config.yaml");
    std::fs::write(
        &path,
        format!(
            "api: {{http: '127.0.0.1:{}'}}\n{}",
            free_pair(),
            std::fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    let host = fixture.assembly();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let main = bound.dns_addr();
        let api = bound.api_addr().unwrap();
        bound.inject_api_accept_fault_after(0);
        let serving = tokio::task::spawn_local(bound.serve(TransportCancellation::new()));
        if let Ok(mut client) = tokio::net::TcpStream::connect(api).await {
            let _ = client
                .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await;
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(2), serving)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(std::net::UdpSocket::bind(main).is_ok());
        assert!(std::net::TcpListener::bind(api).is_ok());
        assert!(std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok());
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    });
}

#[test]
fn changing_listener_entry_invalidates_a_shared_wire_cache() {
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let fixture = Fixture::new(free_pair(), peer.local_addr().unwrap().port());
    let path = fixture.0.join("config.yaml");
    let yaml = std::fs::read_to_string(&path).unwrap()
        .replace("plugins:\n", "plugins:\n  - tag: entry_a\n    type: sequence\n    args: [{exec: $special_upstream_matcher}, {exec: $cache_special_50}, {exec: $special_upstream_50}]\n  - tag: entry_b\n    type: sequence\n    args: [{exec: $special_upstream_matcher}, {exec: $cache_special_50}, {exec: reject 2}]\n")
        .replace("entry: main_entry", "entry: entry_a");
    std::fs::write(&path, &yaml).unwrap();
    let host = fixture.assembly();
    host.block_on(async {
        let peer = tokio::net::UdpSocket::from_std(peer).unwrap();
        let peer_stop = TransportCancellation::new();
        let peer_scope = peer_stop.clone();
        let upstream = tokio::task::spawn_local(async move {
            loop {
                let mut wire = [0;512];
                let received = tokio::select! { () = peer_scope.cancelled() => break, result = peer.recv_from(&mut wire) => result };
                let (n, addr) = received.unwrap();
                let (header,question) = mosdns_dns_core::parse_query(&wire[..n]).unwrap();
                peer.send_to(&mosdns_dns_core::synthesize_response(&header,&question,0).unwrap(),addr).await.unwrap();
            }
        });
        let bound = host.bind_host().await.unwrap();
        let main = bound.dns_addr();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut response = [0;512];
        for _ in 0..2 {
            client.send_to(&query(),main).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2),client.recv_from(&mut response)).await.unwrap().unwrap();
            assert_eq!(response[3]&15,0);
        }
        std::fs::write(&path,yaml.replace("entry: entry_a", "entry: entry_b")).unwrap();
        let control = host.control();
        let prepared = control.prepare_host(load_and_compile(&path).unwrap()).await.unwrap();
        control.install(prepared).unwrap();
        control.retire().await.unwrap();
        client.send_to(&query(),main).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2),client.recv_from(&mut response)).await.unwrap().unwrap();
        assert_eq!(response[3]&15,2,"entry B must not hit entry A's cached answer");
        stop.cancel();serving.await.unwrap().unwrap();peer_stop.cancel();upstream.await.unwrap();
    });
}

#[test]
fn special_groups_http_view_reflects_the_committed_profile() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = fixture.assembly();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let (status, body) = http_request(api, "GET", "/api/v1/special-groups", "").await;
        assert_eq!(status, 200, "{body}");
        let groups: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(groups[0]["slot"], 50);
        assert_eq!(groups[0]["name"], "group");
        assert_eq!(groups[0]["listen_port"], group_port);
        assert_eq!(groups[0]["custom_port_only"], true);
        assert_eq!(groups[0]["upstream_plugin_tag"], "special_upstream_50");

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn special_groups_http_crud_commits_and_removes_owned_state() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    let config_path = fixture.0.join("config.yaml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        config.replace(
            "plugins:\n",
            "plugins:\n  - tag: default_forward\n    type: forward\n    args: {upstreams: [{tag: fallback, addr: 'udp://127.0.0.1:15999'}]}\n",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(fixture.0.join("srs")).unwrap();
    std::fs::write(
        fixture.0.join("srs/special_50.json"),
        br#"{"opaque":{"enabled":false,"unsupported":"retained"}}"#,
    )
    .unwrap();
    std::fs::write(fixture.0.join("rule/special_50.txt"), "owned.example\n").unwrap();
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let duplicate_port = format!(
            r#"{{"slot":51,"name":"other","listen_port":{group_port},"custom_port_only":false}}"#
        );
        let (status, _) = http_request(api, "POST", "/api/v1/special-groups", &duplicate_port).await;
        assert_eq!(status, 409);

        let (status, _) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":51,"name":"bad","listen_port":53,"custom_port_only":false}"#,
        )
        .await;
        assert_eq!(status, 400);

        let (status, _) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":51,"name":"bad","listen_port":0,"custom_port_only":false,"unexpected":true}"#,
        )
        .await;
        assert_eq!(status, 400);

        let (status, _) = http_request(api, "DELETE", "/api/v1/special-groups/99", "").await;
        assert_eq!(status, 404);
        let (status, _) =
            http_request(api, "DELETE", "/api/v1/special-groups/not-a-slot", "").await;
        assert_eq!(status, 400);

        let update = format!(
            r#"{{"slot":50,"name":"renamed","listen_port":{group_port},"custom_port_only":false}}"#
        );
        let (status, body) = http_request(api, "POST", "/api/v1/special-groups", &update).await;
        assert_eq!(status, 200, "{body}");
        let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(saved["name"], "renamed");
        assert_eq!(saved["slot"], 50);
        let catalog: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.0.join("srs/special_50.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(catalog["opaque"]["unsupported"], "retained");

        let (status, body) = http_request(api, "GET", "/api/v1/special-groups", "").await;
        assert_eq!(status, 200);
        let groups: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(groups[0]["name"], "renamed");

        let (status, body) = http_request(api, "DELETE", "/api/v1/special-groups/50", "").await;
        assert_eq!(status, 204, "{body}");
        assert!(!fixture.0.join("srs/special_50.json").exists());
        assert!(!fixture.0.join("rule/special_50.txt").exists());
        let (status, body) = http_request(api, "GET", "/api/v1/special-groups", "").await;
        assert_eq!(status, 200);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap(), json!([]));

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn special_group_http_mutations_follow_nonstandard_root_config_across_snapshots() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    let api_port = free_pair();
    fixture.enable_api(api_port);

    let root_config = fixture.0.join("gateway.yaml");
    std::fs::rename(fixture.0.join("config.yaml"), &root_config).unwrap();
    let decoy = fixture.0.join("config.yaml");
    std::fs::write(
        &decoy,
        "log: {level: error}\nplugins:\n  - tag: main_entry\n    type: sequence\n    args: [{exec: reject 3}]\n  - tag: main\n    type: udp_server\n    args: {entry: main_entry, listen: '127.0.0.1:0'}\n",
    )
    .unwrap();
    let decoy_before = std::fs::read(&decoy).unwrap();

    let host = HostAssembly::from_config_file(&root_config).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let initial_generation = host.control().generation();

        let (status, body) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":51,"name":"created","listen_port":0,"custom_port_only":false}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(host.control().generation(), initial_generation + 1);

        let (status, body) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":51,"name":"updated","listen_port":0,"custom_port_only":false}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(host.control().generation(), initial_generation + 2);

        let groups: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.0.join("webinfo/special_upstream_groups.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(groups[1]["slot"], 51);
        assert_eq!(groups[1]["name"], "updated");

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
    drop(host);

    assert_eq!(std::fs::read(&decoy).unwrap(), decoy_before);
    let restarted = HostAssembly::from_config_file(&root_config).unwrap();
    assert_eq!(
        restarted.config().managed_profile.as_ref().unwrap().groups[1].name,
        "updated"
    );
}

#[test]
fn special_group_zero_or_missing_slot_allocates_the_first_unused_slot() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let (status, body) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":0,"name":"automatic","listen_port":0}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let first: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(first["slot"], 51);

        let (status, body) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"name":"omitted slot","listen_port":0}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let second: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(second["slot"], 52);

        let (status, body) = http_request(api, "GET", "/api/v1/special-groups", "").await;
        assert_eq!(status, 200);
        let groups: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            groups
                .as_array()
                .unwrap()
                .iter()
                .map(|group| group["slot"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![50, 51, 52]
        );

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn occupied_listener_update_returns_conflict_and_preserves_active_generation() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let config_path = fixture.0.join("config.yaml");
    let host = HostAssembly::from_config_file(&config_path).unwrap();
    let occupied_tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let occupied_port = occupied_tcp.local_addr().unwrap().port();
    host.block_on(async {
        let initial_generation = host.control().generation();
        let original_groups =
            std::fs::read(fixture.0.join("webinfo/special_upstream_groups.json")).unwrap();
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let update = json!({
            "slot": 50,
            "name": "group",
            "listen_port": occupied_port,
            "custom_port_only": true
        })
        .to_string();
        let (status, body) = http_request(api, "POST", "/api/v1/special-groups", &update).await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(host.control().generation(), initial_generation);
        assert_eq!(
            std::fs::read(fixture.0.join("webinfo/special_upstream_groups.json")).unwrap(),
            original_groups,
            "a bind conflict must preserve the committed groups file"
        );
        assert!(std::net::TcpListener::bind(("127.0.0.1", group_port)).is_err());
        assert!(std::net::UdpSocket::bind(("127.0.0.1", group_port)).is_err());
        assert!(
            std::net::UdpSocket::bind(("127.0.0.1", occupied_port)).is_ok(),
            "failed candidate preparation must release its prebound UDP half"
        );

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)] // One serialized HTTP/cache concurrency scenario.
fn concurrent_managed_writes_and_cache_management_preserve_committed_state() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    std::fs::write(fixture.0.join("rule/special_50.txt"), "before.example\n").unwrap();
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let initial_generation = host.control().generation();
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let (dump_status, dump) = http_request_bytes(
            api,
            "GET",
            "/plugins/cache_special_50/dump",
            b"",
        )
        .await;
        assert_eq!(dump_status, 200);
        assert_eq!(&dump[..2], &[31, 139]);

        let upstream_body = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [{
                "tag": "parallel",
                "enabled": true,
                "protocol": "udp",
                "addr": "udp://127.0.0.1:15999"
            }]
        })
        .to_string();
        let manual_body = r#"{"values":["parallel.example"]}"#;
        let ((upstream_status, upstream_response), (manual_status, manual_response), (flush_status, _), (import_status, _)) = tokio::join!(
            http_request(api, "POST", "/api/v1/upstream/config", &upstream_body),
            http_request(api, "POST", "/plugins/special_manual_50/post", manual_body),
            http_request(api, "GET", "/plugins/cache_special_50/flush", ""),
            http_request_bytes(api, "POST", "/plugins/cache_special_50/load_dump", &dump),
        );
        assert!(
            matches!(upstream_status, 200 | 409 | 503),
            "unexpected upstream result {upstream_status}: {upstream_response}"
        );
        assert!(
            matches!(manual_status, 200 | 409 | 503),
            "unexpected manual result {manual_status}: {manual_response}"
        );
        assert!(matches!(flush_status, 200 | 503), "flush returned {flush_status}");
        assert!(matches!(import_status, 200 | 503), "import returned {import_status}");

        if upstream_status != 200 {
            let (status, body) =
                http_request(api, "POST", "/api/v1/upstream/config", &upstream_body).await;
            assert_eq!(status, 200, "{body}");
        }
        if manual_status != 200 {
            let (status, body) = http_request(
                api,
                "POST",
                "/plugins/special_manual_50/post",
                manual_body,
            )
            .await;
            assert_eq!(status, 200, "{body}");
        }
        if flush_status != 200 {
            assert_eq!(
                http_request(api, "GET", "/plugins/cache_special_50/flush", "").await.0,
                200
            );
        }
        if import_status != 200 {
            assert_eq!(
                http_request_bytes(
                    api,
                    "POST",
                    "/plugins/cache_special_50/load_dump",
                    &dump,
                )
                .await
                .0,
                200
            );
        }

        assert_eq!(host.control().generation(), initial_generation + 2);
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("rule/special_50.txt")).unwrap(),
            "parallel.example\n"
        );
        let overrides: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(overrides["special_upstream_50"][0]["tag"], "parallel");
        let (status, body) =
            http_request(api, "GET", "/api/v1/upstream/config", "").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()
                ["special_upstream_50"][0]["tag"],
            "parallel"
        );

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)] // One live upstream-save, DNS, and restart proof.
fn upstream_http_snapshot_save_changes_generation_and_dns_supplier() {
    let old_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let new_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    old_peer.set_nonblocking(true).unwrap();
    new_peer.set_nonblocking(true).unwrap();
    let old_addr = old_peer.local_addr().unwrap();
    let new_addr = new_peer.local_addr().unwrap();
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, old_addr.port());
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let old_peer = tokio::net::UdpSocket::from_std(old_peer).unwrap();
        let new_peer = tokio::net::UdpSocket::from_std(new_peer).unwrap();
        let peer_stop = TransportCancellation::new();
        let old_scope = peer_stop.clone();
        let old_task = tokio::task::spawn_local(async move {
            loop {
                let mut packet = vec![0; 4096];
                let received = tokio::select! {
                    () = old_scope.cancelled() => break,
                    received = old_peer.recv_from(&mut packet) => received,
                };
                let (length, address) = received.unwrap();
                let (header, question) = mosdns_dns_core::parse_query(&packet[..length]).unwrap();
                let response = mosdns_dns_core::synthesize_response(&header, &question, 3).unwrap();
                old_peer.send_to(&response, address).await.unwrap();
            }
        });
        let new_scope = peer_stop.clone();
        let new_task = tokio::task::spawn_local(async move {
            loop {
                let mut packet = vec![0; 4096];
                let received = tokio::select! {
                    () = new_scope.cancelled() => break,
                    received = new_peer.recv_from(&mut packet) => received,
                };
                let (length, address) = received.unwrap();
                let (header, question) = mosdns_dns_core::parse_query(&packet[..length]).unwrap();
                let response = mosdns_dns_core::synthesize_response(&header, &question, 0).unwrap();
                new_peer.send_to(&response, address).await.unwrap();
            }
        });

        let initial_generation = host.control().generation();
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let (status, body) = http_request(api, "GET", "/api/v1/upstream/tags", "").await;
        assert_eq!(status, 200, "{body}");
        let tags: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(tags, json!(["special_upstream_50"]));

        let (status, body) = http_request(api, "GET", "/api/v1/capabilities", "").await;
        assert_eq!(status, 200);
        let capabilities: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(capabilities["schema_version"], 1);
        assert_eq!(capabilities["runtime"], "rust");
        assert_eq!(capabilities["special_groups"]["enabled"], true);
        assert_eq!(
            capabilities["special_groups"]["profile"],
            "local_text_dns_v1"
        );
        assert_eq!(
            capabilities["upstream_protocols"],
            json!(["udp", "tcp", "dot", "doh"])
        );
        assert_eq!(capabilities["endpoints"]["upstream"]["config_post"], true);
        assert!(
            capabilities["unsupported_features"]
                .as_array()
                .unwrap()
                .iter()
                .any(|feature| feature == "quic")
        );

        let (status, body) = http_request(api, "GET", "/api/v1/upstream/config", "").await;
        assert_eq!(status, 200);
        let overrides: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(overrides["special_upstream_50"][0]["tag"], "controlled");
        let original_override_file =
            std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap();

        let (status, _) = http_request(api, "POST", "/api/v1/upstream/tags", "{}").await;
        assert_eq!(status, 405);
        let (status, _) = http_request(api, "GET", "/api/v1/upstream/runtime/missing", "").await;
        assert_eq!(status, 404);
        let unknown_tag = json!({
            "plugin_tag": "missing_forward",
            "upstreams": [{
                "tag": "candidate",
                "enabled": true,
                "protocol": "udp",
                "addr": "udp://127.0.0.1:9999"
            }]
        })
        .to_string();
        let (status, _) = http_request(api, "POST", "/api/v1/upstream/config", &unknown_tag).await;
        assert_eq!(status, 404);
        assert_eq!(
            std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
            original_override_file,
            "unknown upstream tags must not create persisted overrides"
        );

        for upstreams in [
            json!([]),
            json!([{
                "tag": "disabled-only",
                "enabled": false,
                "protocol": "quic",
                "addr": "quic://127.0.0.1:9999"
            }]),
            json!([
                {
                    "tag": "duplicate",
                    "enabled": true,
                    "protocol": "udp",
                    "addr": "udp://127.0.0.1:9999"
                },
                {
                    "tag": "duplicate",
                    "enabled": true,
                    "protocol": "udp",
                    "addr": "udp://127.0.0.1:10000"
                }
            ]),
        ] {
            let request = json!({
                "plugin_tag": "special_upstream_50",
                "upstreams": upstreams
            })
            .to_string();
            let (status, body) =
                http_request(api, "POST", "/api/v1/upstream/config", &request).await;
            assert_eq!(status, 400, "{body}");
            assert_eq!(
                std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
                original_override_file,
                "empty, disabled-only and duplicate-tag configurations must not persist"
            );
        }

        for (option, value) in [
            ("idle_timeout", json!(1)),
            ("enable_pipeline", json!(true)),
            ("enable_http3", json!(true)),
            ("use_socks_proxy", json!(true)),
            ("so_mark", json!(7)),
            ("bind_to_device", json!("lo")),
            ("max_conns", json!(1)),
        ] {
            let entry = json!({
                "tag": "unsupported",
                "enabled": true,
                "protocol": "udp",
                "addr": "udp://127.0.0.1:9999",
                (option): value
            });
            let request = json!({
                "plugin_tag": "special_upstream_50",
                "upstreams": [entry]
            })
            .to_string();
            let (status, body) =
                http_request(api, "POST", "/api/v1/upstream/config", &request).await;
            assert_eq!(status, 400, "{option}: {body}");
            assert_eq!(
                std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
                original_override_file,
                "unsupported {option} must not alter persisted overrides"
            );
        }

        let signed_protocol = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [{
                "tag": "unsupported",
                "enabled": true,
                "protocol": "aliapi",
                "addr": "aliapi://127.0.0.1:9999"
            }]
        })
        .to_string();
        let (status, body) =
            http_request(api, "POST", "/api/v1/upstream/config", &signed_protocol).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
            original_override_file
        );
        let unsupported = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [{
                "tag": "unsupported",
                "enabled": true,
                "protocol": "quic",
                "addr": "quic://127.0.0.1:9999"
            }]
        })
        .to_string();
        let (status, _) = http_request(api, "POST", "/api/v1/upstream/config", &unsupported).await;
        assert_eq!(status, 400);
        assert_eq!(
            std::fs::read(fixture.0.join("webinfo/upstream_overrides.json")).unwrap(),
            original_override_file,
            "unsupported saves must leave persisted state untouched"
        );

        let new_upstreams = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [
                {
                    "tag": "replacement",
                    "enabled": true,
                    "protocol": "udp",
                    "addr": format!("udp://{new_addr}"),
                    "idle_timeout": 0,
                    "enable_pipeline": false,
                    "enable_http3": false,
                    "use_socks_proxy": false,
                    "account_id": ""
                },
                {
                    "tag": "disabled-unsupported",
                    "enabled": false,
                    "protocol": "quic",
                    "addr": "quic://127.0.0.1:9999",
                    "enable_http3": true,
                    "opaque": {"future": true}
                }
            ]
        })
        .to_string();
        let (status, body) =
            http_request(api, "POST", "/api/v1/upstream/config", &new_upstreams).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["message"],
            "Upstream configuration saved."
        );
        assert_eq!(host.control().generation(), initial_generation + 1);

        let (status, body) = http_request(
            api,
            "GET",
            "/api/v1/upstream/runtime/special_upstream_50",
            "",
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let runtime: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(runtime["tag"], "special_upstream_50");
        assert_eq!(runtime["override_config"][0]["tag"], "replacement");
        assert_eq!(runtime["override_config"][1]["opaque"]["future"], true);
        assert_eq!(runtime["runtime_targets"], json!([new_addr.to_string()]));

        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut response = vec![0; 4096];
        client
            .send_to(&query(), ("127.0.0.1", group_port))
            .await
            .unwrap();
        let (length, _) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut response))
                .await
                .unwrap()
                .unwrap();
        mosdns_dns_core::validate_response(&response[..length]).unwrap();
        assert_eq!(response[3] & 15, 0, "DNS must use the newly committed peer");
        while host.metrics_snapshot().completed_total == 0 {
            tokio::task::yield_now().await;
        }
        let audit = host.audit_snapshot();
        assert_eq!(
            audit.records.last().unwrap().final_upstream.as_deref(),
            Some("replacement")
        );

        stop.cancel();
        serving.await.unwrap().unwrap();
        peer_stop.cancel();
        old_task.await.unwrap();
        new_task.await.unwrap();
    });

    drop(host);
    let restarted = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    restarted.block_on(async {
        let bound = restarted.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let (status, body) = http_request(
            api,
            "GET",
            "/api/v1/upstream/runtime/special_upstream_50",
            "",
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let runtime: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(runtime["override_config"][0]["tag"], "replacement");
        assert_eq!(runtime["override_config"][1]["opaque"]["future"], true);
        assert_eq!(runtime["runtime_targets"], json!([new_addr.to_string()]));
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)] // One HTTP catalog rename and owned-file lifecycle proof.
fn diversion_catalog_http_uses_single_transaction_for_source_rename() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    std::fs::create_dir_all(fixture.0.join("srs")).unwrap();
    std::fs::write(
        fixture.0.join("srs/special_50.json"),
        br#"{"opaque":{"enabled":false,"name":"opaque","type":"legacy","future":{"x":1}}}"#,
    )
    .unwrap();
    std::fs::write(fixture.0.join("rule/shared.txt"), "ads.example\n").unwrap();
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let (status, body) = http_request(
            api,
            "GET",
            "/plugins/special_route_50/config",
            "",
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let sources: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sources[0]["name"], "opaque");
        assert_eq!(sources[0]["future"]["x"], 1);

        let source = |name: &str, file: &str| {
            json!({
                "name": name,
                "type": "special_50",
                "enabled": true,
                "files": file,
                "auto_update": false,
                "enable_regexp": false
            })
            .to_string()
        };
        let (status, body) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/local",
            &source("local", "rule/shared.txt"),
        )
        .await;
        assert_eq!(status, 201, "{body}");

        let generation = host.control().generation();
        let (status, body) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/local",
            &source("renamed", "rule/shared.txt"),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(host.control().generation(), generation + 1);
        let catalog: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.0.join("srs/special_50.json")).unwrap(),
        )
        .unwrap();
        assert!(catalog.get("local").is_none());
        assert_eq!(catalog["renamed"]["files"], "rule/shared.txt");
        assert_eq!(catalog["opaque"]["future"]["x"], 1);

        let catalog_before_invalid = std::fs::read(fixture.0.join("srs/special_50.json")).unwrap();
        let invalid_file = source("missing", "rule/does-not-exist.txt");
        let (status, _) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/missing",
            &invalid_file,
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(
            api,
            "DELETE",
            "/plugins/special_route_50/config/opaque",
            "",
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/new",
            r#"{"name":"new","type":"special_50","enabled":true,"files":"rule/shared.txt","unsupported":false}"#,
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/opaque",
            r#"{"name":"opaque","type":"legacy","enabled":false,"future":{"x":2}}"#,
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(
            api,
            "PUT",
            "/plugins/special_route_50/config/%2e%2e%2frule",
            &source("escape", "rule/shared.txt"),
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(api, "GET", "/plugins/special_route_50/update", "").await;
        assert_eq!(status, 405);
        assert_eq!(
            std::fs::read(fixture.0.join("srs/special_50.json")).unwrap(),
            catalog_before_invalid
        );

        let (status, body) = http_request(
            api,
            "DELETE",
            "/plugins/special_route_50/config/renamed",
            "",
        )
        .await;
        assert_eq!(status, 204, "{body}");
        assert!(fixture.0.join("rule/shared.txt").exists());
        let (status, body) = http_request(
            api,
            "GET",
            "/plugins/special_route_50/config",
            "",
        )
        .await;
        assert_eq!(status, 200);
        let sources: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sources.as_array().unwrap().len(), 1);
        assert_eq!(sources[0]["name"], "opaque");

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn managed_manual_http_post_commits_rule_file_and_new_generation() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    std::fs::write(fixture.0.join("rule/special_50.txt"), "old.example\n").unwrap();
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let initial_generation = host.control().generation();

        let (status, body) = http_request(api, "GET", "/plugins/special_manual_50/show", "").await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("old.example"));

        let (status, body) = http_request(
            api,
            "POST",
            "/plugins/special_manual_50/post",
            r#"{"values":[" new.example ",""," # retained comment "]}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, "domain_set replaced with 1 entries");
        assert_eq!(host.control().generation(), initial_generation + 1);
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("rule/special_50.txt")).unwrap(),
            "new.example\n"
        );
        let (status, body) = http_request(api, "GET", "/plugins/special_manual_50/show", "").await;
        assert_eq!(status, 200);
        assert!(body.contains("new.example"));
        assert!(!body.contains("old.example"));

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn disconnected_manual_post_remains_owned_until_transaction_commit() {
    let group_port = free_pair();
    let fixture = Fixture::new(group_port, 15999);
    std::fs::write(fixture.0.join("rule/special_50.txt"), "before.example\n").unwrap();
    let api_port = free_pair();
    fixture.enable_api(api_port);
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let initial_generation = host.control().generation();
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));

        let body = r#"{"values":["disconnected.example"]}"#;
        let mut client = tokio::net::TcpStream::connect(api).await.unwrap();
        let request = format!(
            "POST /plugins/special_manual_50/post HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        client.write_all(request.as_bytes()).await.unwrap();
        drop(client);

        tokio::time::timeout(Duration::from_secs(3), async {
            while host.control().generation() == initial_generation {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the admitted transaction must finish after its client disconnects");
        assert_eq!(host.control().generation(), initial_generation + 1);
        assert_eq!(
            std::fs::read_to_string(fixture.0.join("rule/special_50.txt")).unwrap(),
            "disconnected.example\n"
        );

        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn capabilities_disable_managed_features_without_the_native_profile() {
    let fixture = Fixture::new(free_pair(), 15999);
    let api_port = free_pair();
    let dns_port = free_pair();
    std::fs::write(
        fixture.0.join("config.yaml"),
        format!(
            "api: {{http: '127.0.0.1:{api_port}'}}\nlog: {{level: error}}\nplugins:\n  - tag: default_forward\n    type: forward\n    args: {{upstreams: [{{tag: fallback, addr: 'udp://127.0.0.1:15999'}}]}}\n  - tag: main_entry\n    type: sequence\n    args: [{{exec: reject 3}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: main_entry, listen: '127.0.0.1:{dns_port}', enable_audit: false}}\n"
        ),
    )
    .unwrap();
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(stop.clone()));
        let (status, body) = http_request(api, "GET", "/api/v1/capabilities", "").await;
        assert_eq!(status, 200);
        let capabilities: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(capabilities["runtime"], "rust");
        assert_eq!(capabilities["special_groups"]["enabled"], false);
        assert_eq!(
            capabilities["special_groups"]["profile"],
            serde_json::Value::Null
        );
        assert_eq!(capabilities["upstream_protocols"], json!([]));
        assert_eq!(capabilities["rule_formats"], json!([]));
        assert_eq!(capabilities["endpoints"]["special_groups"]["post"], false);
        assert_eq!(capabilities["endpoints"]["upstream"]["config_post"], false);
        assert_eq!(capabilities["endpoints"]["diversion_sources"]["put"], false);
        assert_eq!(capabilities["endpoints"]["manual_rules"]["post"], false);
        let (status, body) = http_request(api, "GET", "/api/v1/special-groups", "").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            json!([])
        );
        let (status, _) = http_request(
            api,
            "POST",
            "/api/v1/special-groups",
            r#"{"slot":0,"name":"unsupported profile"}"#,
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = http_request(
            api,
            "POST",
            "/api/v1/upstream/config",
            r#"{"plugin_tag":"default_forward","upstreams":[]}"#,
        )
        .await;
        assert_eq!(status, 400);
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}
