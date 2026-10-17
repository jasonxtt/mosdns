//! Standard special-group state participates in the one effective configuration.
use mosdns_native_host::load_and_compile;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Tree(PathBuf);
impl Tree {
    fn new(name: &str, proxies: &str, owned: bool) -> Self {
        let root = std::env::temp_dir().join(format!("m1e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for directory in ["sub_config", "webinfo", "rule", "cache", "srs"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        std::fs::write(root.join("sub_config/special_groups.yaml"), "plugins:\n  - tag: special_upstream_matcher\n    type: domain_mapper\n    args: {rules: []}\n  - tag: sequence_special\n    type: sequence\n    args: []\n").unwrap();
        std::fs::write(root.join("rule/switch4.txt"), "A").unwrap();
        std::fs::write(
            root.join("config.yaml"),
            format!(
                r"log: {{level: error}}
include: [sub_config/special_groups.yaml]
plugins:
{proxies}
  - tag: switch4
    type: switch4
    args: {{initial_value: rule/switch4.txt}}
  - tag: normal
    type: aliapi
    args:
      upstreams: [{{tag: original, addr: 'tcp://127.0.0.1:15500'{source_proxy}}}]
  - tag: main
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $sequence_special
      - exec: $normal
  - tag: server
    type: udp_server
    args: {{listen: '127.0.0.1:15535', entry: main}}
",
                source_proxy = if owned { "" } else { ", proxy: $p" }
            ),
        )
        .unwrap();
        let entries = if owned {
            r#""upstream_sources":[],"owned_upstreams":[{"tag":"own","enabled":true,"protocol":"tcp","addr":"127.0.0.1:15501","use_socks_proxy":true}]"#
        } else {
            r#""upstream_sources":[{"kind":"group","plugin_tag":"normal"}],"owned_upstreams":[]"#
        };
        std::fs::write(
            root.join("webinfo/special_upstream_groups.json"),
            format!(r#"[{{"slot":50,"name":"test",{entries}}}]"#),
        )
        .unwrap();
        Self(root)
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const ONE: &str = "  - tag: p\n    type: socks5_proxy\n    args: {addr: '127.0.0.1:17890'}";
const TWO: &str = "  - tag: p\n    type: socks5_proxy\n    args: {addr: '127.0.0.1:17890'}\n  - tag: other\n    type: socks5_proxy\n    args: {addr: '127.0.0.1:17891'}";

#[test]
fn owned_proxy_requires_exactly_one_object_and_never_falls_back() {
    for (name, proxies, reason) in [
        ("missing", "", "no socks5_proxy"),
        ("ambiguous", TWO, "ambiguous"),
    ] {
        let tree = Tree::new(name, proxies, true);
        let error = load_and_compile(&tree.0.join("config.yaml"))
            .err()
            .expect("proxy must be refused");
        assert!(error.to_string().contains(reason), "{error}");
    }
    let tree = Tree::new("sole", ONE, true);
    let compiled = load_and_compile(&tree.0.join("config.yaml")).unwrap();
    assert!(
        compiled
            .settings_report()
            .iter()
            .any(|item| item.object == "special_upstream_50"
                && item.value.contains("127.0.0.1:17890")),
        "missing effective group proxy"
    );
}

#[test]
fn a_reference_inherits_its_source_proxy_even_with_several_objects() {
    let tree = Tree::new("inherit", TWO, false);
    let compiled = load_and_compile(&tree.0.join("config.yaml")).unwrap();
    assert!(
        compiled
            .settings_report()
            .iter()
            .any(|item| item.object == "special_upstream_50"
                && item.value.contains("127.0.0.1:17890")
                && !item.value.contains("17891"))
    );
}

#[test]
fn sole_empty_proxy_and_inherited_bootstrap_keep_their_meaning() {
    let tree = Tree::new("empty-proxy", &ONE.replace("127.0.0.1:17890", ""), true);
    load_and_compile(&tree.0.join("config.yaml")).unwrap();
    let tree = Tree::new("bootstrap", TWO, false);
    let path = tree.0.join("config.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "      upstreams:",
            "      bootstrap: '127.0.0.1:15502'\n      bootstrap_version: 4\n      upstreams:",
        )
        .replace("tcp://127.0.0.1:15500", "tcp://upstream.example.com:15500")
        .replace(
            "proxy: $p",
            "proxy: $p, bootstrap: '', bootstrap_version: 0",
        );
    std::fs::write(&path, yaml).unwrap();
    let compiled = load_and_compile(&path).unwrap();
    let entry = &compiled
        .forward_definitions
        .iter()
        .find(|forward| forward.tag == "special_upstream_50")
        .unwrap()
        .entries[0];
    assert_eq!(
        entry.target.bootstrap,
        Some("127.0.0.1:15502".parse().unwrap())
    );
    assert_eq!(entry.target.bootstrap_version, Some(4));
    assert!(
        compiled
            .settings_report()
            .iter()
            .any(|item| item.object == "special_upstream_50"
                && item.value.contains("upstream.example.com"))
    );
}

#[test]
fn whole_group_dominates_single_references_and_runtime_tags_do_not_collide() {
    let tree = Tree::new("dedupe", TWO, false);
    std::fs::write(tree.0.join("webinfo/special_upstream_groups.json"), r#"[
      {"slot":50,"name":"mixed","owned_upstreams":[{"tag":"original","enabled":true,"protocol":"tcp","addr":"127.0.0.1:15501","use_proxy":false}],
       "upstream_sources":[{"kind":"upstream","plugin_tag":"normal","upstream_tag":"original"},{"kind":"group","plugin_tag":"normal"},{"kind":"group","plugin_tag":"normal"}]},
      {"slot":51,"name":"same-source","upstream_sources":[{"kind":"group","plugin_tag":"normal"}]}
    ]"#).unwrap();
    let compiled = load_and_compile(&tree.0.join("config.yaml")).unwrap();
    let mixed = compiled
        .settings_report()
        .iter()
        .find(|item| item.object == "special_upstream_50" && item.field == "upstreams")
        .unwrap();
    assert_eq!(mixed.value.matches("tcp://").count(), 2, "{}", mixed.value);
    assert!(mixed.value.find("15501").unwrap() < mixed.value.find("15500").unwrap());
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
async fn http(
    port: u16,
    method: &str,
    path: &str,
    value: serde_json::Value,
) -> (u16, serde_json::Value) {
    let body = if value.is_null() {
        String::new()
    } else {
        value.to_string()
    };
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap_or_else(|_| serde_json::Value::String(body.to_owned())),
    )
}
async fn query(port: u16, name: &str) -> Vec<u8> {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut wire = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        wire.push(u8::try_from(label.len()).unwrap());
        wire.extend_from_slice(label.as_bytes());
    }
    wire.extend_from_slice(&[0, 0, 1, 0, 1]);
    socket.send_to(&wire, ("127.0.0.1", port)).await.unwrap();
    let mut response = vec![0; 1500];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut response))
        .await
        .unwrap()
        .unwrap();
    response.truncate(len);
    response
}
async fn stub(
    marker: u8,
    cancel: mosdns_upstream_core::TransportCancellation,
) -> (u16, tokio::task::JoinHandle<()>) {
    let (port, task, _) = counted_stub(marker, cancel).await;
    (port, task)
}
async fn counted_stub(
    marker: u8,
    cancel: mosdns_upstream_core::TransportCancellation,
) -> (
    u16,
    tokio::task::JoinHandle<()>,
    std::rc::Rc<std::cell::Cell<usize>>,
) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let calls = count.clone();
    let task = tokio::task::spawn_local(async move {
        let mut bytes = vec![0; 1500];
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                result = socket.recv_from(&mut bytes) => {
                    let (len, peer) = result.unwrap();
                    calls.set(calls.get() + 1);
                    let mut answer = bytes[..len].to_vec(); answer[2] = 0x81; answer[3] = 0x80; answer[7] = 1;
                    answer.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 1, 44, 0, 4, 192, 0, 2, marker]);
                    socket.send_to(&answer, peer).await.unwrap();
                }
            }
        }
    });
    (port, task, count)
}

#[test]
fn group_create_and_delete_keep_an_unrelated_cache_warm() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let host = tokio::task::LocalSet::new().block_on(&runtime, async {
        let tree = Tree::new("cache", ONE, true);
        std::fs::write(tree.0.join("webinfo/special_upstream_groups.json"), "[]").unwrap();
        let cancel = mosdns_upstream_core::TransportCancellation::new();
        let (upstream, peer, calls) = counted_stub(10, cancel.clone()).await;
        let api = port(); let unrelated = port(); let main = port();
        let path = tree.0.join("config.yaml");
        let yaml = std::fs::read_to_string(&path).unwrap()
            .replace("include:", &format!("api: {{http: '127.0.0.1:{api}'}}\ninclude:"))
            .replace("tcp://127.0.0.1:15500", &format!("udp://127.0.0.1:{upstream}"))
            .replace("127.0.0.1:15535", &format!("127.0.0.1:{main}"));
        std::fs::write(&path, format!("{yaml}\n  - tag: unrelated_cache\n    type: cache\n    args: {{size: 16}}\n  - tag: isolated\n    type: sequence\n    args: [{{exec: $unrelated_cache}}, {{exec: $normal}}]\n  - tag: isolated_server\n    type: udp_server\n    args: {{listen: '127.0.0.1:{unrelated}', entry: isolated}}\n")).unwrap();
        let host = mosdns_native_host::HostAssembly::from_config_file(&path).unwrap();
        let bound = host.bind_host().await.unwrap();
        let scope = mosdns_upstream_core::TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(scope.clone()));
        assert_eq!(query(unrelated, "warm.example.com").await.last(), Some(&10));
        assert_eq!(calls.get(), 1);
        let (status, result) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"name":"new","upstream_sources":[{"kind":"group","plugin_tag":"normal"}]})).await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(query(unrelated, "warm.example.com").await.last(), Some(&10));
        assert_eq!(calls.get(), 1, "create discarded the unrelated cache");
        assert_eq!(http(api, "DELETE", "/api/v1/special-groups/50", serde_json::Value::Null).await.0, 204);
        assert_eq!(query(unrelated, "warm.example.com").await.last(), Some(&10));
        assert_eq!(calls.get(), 1, "delete discarded the unrelated cache");
        scope.cancel(); serving.await.unwrap().unwrap();
        cancel.cancel(); peer.await.unwrap();
        host
    });
    drop(host);
}

async fn failed_saves_preserve_files(tree: &Tree, api: u16, state: &[u8], generated: &[u8]) {
    // The second rename fails after derived YAML was replaced: roll it back.
    let blocked_temp = tree.0.join(format!(
        "webinfo/special_upstream_groups.json.tmp-{}",
        std::process::id()
    ));
    std::fs::create_dir(&blocked_temp).unwrap();
    let (status, _) = http(
        api,
        "POST",
        "/api/v1/special-groups",
        serde_json::json!({"slot":50,"name":"failed-save"}),
    )
    .await;
    assert_eq!(status, 500);
    assert_eq!(
        std::fs::read(tree.0.join("webinfo/special_upstream_groups.json")).unwrap(),
        state
    );
    assert_eq!(
        std::fs::read(tree.0.join("sub_config/special_groups.yaml")).unwrap(),
        generated
    );
    std::fs::remove_dir(blocked_temp).unwrap();
    // Saving a proxied owned entry is refused with a useful reason at 0/2 objects.
    let config_text = std::fs::read_to_string(tree.0.join("config.yaml")).unwrap();
    for (objects, reason) in [("", "no socks5_proxy"), (TWO, "ambiguous")] {
        let yaml = config_text.replace(ONE, objects).replace("proxy: $p, ", "");
        std::fs::write(tree.0.join("config.yaml"), yaml).unwrap();
        let (status, result) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"name":"bad-proxy","upstreams":[{"tag":"bad","enabled":true,"protocol":"tcp","addr":"127.0.0.1:17800","use_proxy":true}]})).await;
        assert_eq!(status, 400, "{result}");
        assert!(
            result["error"].as_str().unwrap().contains(reason),
            "{result}"
        );
        assert_eq!(
            std::fs::read(tree.0.join("webinfo/special_upstream_groups.json")).unwrap(),
            state
        );
    }
    std::fs::write(tree.0.join("config.yaml"), config_text).unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let (status, _) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"slot":51,"name":"port","listen_port":occupied.local_addr().unwrap().port(),"custom_port_only":true})).await;
    assert_eq!(status, 409);
    assert_eq!(
        std::fs::read(tree.0.join("webinfo/special_upstream_groups.json")).unwrap(),
        state
    );
    assert_eq!(
        std::fs::read(tree.0.join("sub_config/special_groups.yaml")).unwrap(),
        generated
    );
    drop(occupied);
}

async fn special_cache_hit_keeps_audit_label(api: u16, dns: u16) {
    assert_eq!(
        http(api, "POST", "/api/v1/audit/start", serde_json::json!({}))
            .await
            .0,
        200
    );
    for _ in 0..2 {
        query(dns, "chosen.example.com").await;
        let (_, logs) = http(
            api,
            "GET",
            "/api/v2/audit/logs?page=1&limit=1",
            serde_json::Value::Null,
        )
        .await;
        assert_eq!(logs["logs"][0]["domain_set"], "特殊上游50", "{logs}");
        assert_eq!(logs["logs"][0]["effective_tag"], "特殊上游50", "{logs}");
        assert_eq!(
            logs["logs"][0]["final_sequence"], "sequence_special_50",
            "{logs}"
        );
    }
}

#[test]
fn standard_http_routes_dns_ports_rollback_refresh_and_restart() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let assemblies = tokio::task::LocalSet::new().block_on(&runtime, async {
        let tree = Tree::new("http", ONE, true);
        std::fs::write(tree.0.join("webinfo/special_upstream_groups.json"), "[]").unwrap();
        let cancel = mosdns_upstream_core::TransportCancellation::new();
        let (a, stub_a) = stub(10, cancel.clone()).await;
        let (b, stub_b) = stub(20, cancel.clone()).await;
        let dns = port(); let api = port(); let dedicated = port();
        std::fs::write(tree.0.join("config.yaml"), format!(r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
include: [sub_config/special_groups.yaml]
plugins:
{ONE}
  - tag: switch4
    type: switch4
    args: {{initial_value: rule/switch4.txt}}
  - tag: normal
    type: aliapi
    args: {{proxy: $p, upstreams: [{{tag: original, addr: 'udp://127.0.0.1:{a}'}}]}}
  - tag: entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $sequence_special
      - exec: $normal
  - tag: server
    type: udp_server
    args: {{listen: '127.0.0.1:{dns}', entry: entry, enable_audit: true}}
")).unwrap();
        let host = mosdns_native_host::HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
        let bound = host.bind_host().await.unwrap();
        let scope = mosdns_upstream_core::TransportCancellation::new();
        let serve_scope = scope.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(serve_scope).await.unwrap(); });
        let (_, caps) = http(api, "GET", "/api/v1/capabilities", serde_json::Value::Null).await;
        assert_eq!(caps["ui_operations"]["groups.upstreams"]["supported"], true, "{caps}");
        let (status, sources) = http(api, "GET", "/api/v1/upstream/sources", serde_json::Value::Null).await;
        assert_eq!(status, 200); assert_eq!(sources[0]["plugin_tag"], "normal");
        let entry = serde_json::json!({"tag":"owned","enabled":true,"protocol":"udp","addr":format!("127.0.0.1:{b}"),"use_socks_proxy":false});
        let (status, saved) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"name":"main","upstreams":[entry]})).await;
        assert_eq!(status, 200, "{saved}"); assert_eq!(saved["slot"], 50); assert_eq!(saved["effective_upstream_count"], 1);
        let (status, _) = http(api, "POST", "/api/v1/upstream/config", serde_json::json!({"plugin_tag":"special_upstream_50","upstreams":[]})).await;
        assert_eq!(status, 400, "private entries must be maintained through group management");
        let (status, result) = http(api, "POST", "/plugins/special_manual_50/post", serde_json::json!({"values":["full:chosen.example.com"]})).await;
        assert_eq!(status, 200, "{result}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if query(dns, "chosen.example.com").await.last() == Some(&20) { break; }
            assert!(tokio::time::Instant::now() < deadline, "manual-rule rebuild never applied");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(query(dns, "other.example.com").await.last(), Some(&10));
        special_cache_hit_keeps_audit_label(api, dns).await;
        let (status, saved) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"name":"port","listen_port":dedicated,"custom_port_only":true,"upstream_sources":[{"kind":"group","plugin_tag":"normal"}]})).await;
        assert_eq!(status, 200, "{saved}"); assert_eq!(saved["slot"], 51);
        assert_eq!(query(dedicated, "chosen.example.com").await.last(), Some(&10));
        // The dedicated UDP listener gets (and reports) an enlarged receive buffer too.
        let listed_buffers = |state: &serde_json::Value| {
            state["udp_receive_buffers"].as_array().unwrap().iter()
                .filter(|entry| entry["listen"].as_str().unwrap().ends_with(&format!(":{dedicated}")))
                .map(|entry| entry["bytes"].as_u64().unwrap())
                .collect::<Vec<_>>()
        };
        let buffers = listed_buffers(&health(api).await.1);
        assert!(buffers.len() == 1 && buffers[0] > 0, "{buffers:?}");
        let state = std::fs::read(tree.0.join("webinfo/special_upstream_groups.json")).unwrap();
        let generated = std::fs::read(tree.0.join("sub_config/special_groups.yaml")).unwrap();
        failed_saves_preserve_files(&tree, api, &state, &generated).await;
        let (status, _) = http(api, "POST", "/api/v1/upstream/config", serde_json::json!({"plugin_tag":"normal","upstreams":[{"tag":"changed","enabled":true,"protocol":"udp","addr":format!("127.0.0.1:{b}")}]})).await;
        assert_eq!(status, 200);
        assert_eq!(query(dedicated, "fresh.example.com").await.last(), Some(&20));
        let (_, listed) = http(api, "GET", "/api/v1/special-groups", serde_json::Value::Null).await;
        assert_eq!(listed[1]["upstream_active"], true);
        // No usable references: warning, no listener and no main-flow diversion.
        let (status, inactive) = http(api, "POST", "/api/v1/special-groups", serde_json::json!({"name":"inactive","upstream_sources":[{"kind":"upstream","plugin_tag":"missing","upstream_tag":"no"}]})).await;
        assert_eq!(status, 200); assert_eq!(inactive["upstream_active"], false); assert!(inactive["upstream_warnings"].as_array().unwrap().len() >= 2);
        let (status, _) = http(api, "DELETE", "/api/v1/special-groups/50", serde_json::Value::Null).await;
        assert_eq!(status, 204); assert!(!tree.0.join("rule/special_50.txt").exists()); assert!(!tree.0.join("cache/cache_special_50.dump").exists());
        let (status, _) = http(api, "DELETE", "/api/v1/special-groups/51", serde_json::Value::Null).await;
        assert_eq!(status, 204);
        std::net::TcpListener::bind(("127.0.0.1", dedicated)).unwrap();
        std::net::UdpSocket::bind(("127.0.0.1", dedicated)).unwrap();
        assert!(listed_buffers(&health(api).await.1).is_empty(), "a closed listener is no longer listed");
        scope.cancel(); serving.await.unwrap();
        // A stale derived file cannot resurrect a deleted group on restart.
        std::fs::write(tree.0.join("sub_config/special_groups.yaml"), generated).unwrap();
        let restart = mosdns_native_host::HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
        let rebound = restart.bind_host().await.unwrap();
        assert_eq!(load_and_compile(&tree.0.join("config.yaml")).unwrap().listeners.len(), 1);
        let stopped = mosdns_upstream_core::TransportCancellation::new();
        stopped.cancel(); rebound.serve(stopped).await.unwrap();
        cancel.cancel(); stub_a.await.unwrap(); stub_b.await.unwrap();
        (host, restart)
    });
    drop(assemblies);
}

/// The configuration of the cleanup tests: one proxy object, one forward group
/// and one DNS listener, with the management API.
fn cleanup_config(tree: &Tree, api: u16, dns: u16, upstream: u16) {
    std::fs::write(
        tree.0.join("config.yaml"),
        format!(
            r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
include: [sub_config/special_groups.yaml]
plugins:
{ONE}
  - tag: switch4
    type: switch4
    args: {{initial_value: rule/switch4.txt}}
  - tag: normal
    type: aliapi
    args: {{proxy: $p, upstreams: [{{tag: original, addr: 'udp://127.0.0.1:{upstream}'}}]}}
  - tag: entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $sequence_special
      - exec: $normal
  - tag: server
    type: udp_server
    args: {{listen: '127.0.0.1:{dns}', entry: entry, enable_audit: true}}
"
        ),
    )
    .unwrap();
}

async fn health(api: u16) -> (u16, serde_json::Value) {
    http(api, "GET", "/api/v1/system/health", serde_json::Value::Null).await
}

/// The fault goes away (the name is a plain file again); the next save retries.
async fn retried_by_the_next_save(
    api: u16,
    blocker: &std::path::Path,
    sources: &serde_json::Value,
) {
    std::fs::remove_dir_all(blocker).unwrap();
    std::fs::write(blocker, "stale").unwrap();
    let (status, saved) = http(
        api,
        "POST",
        "/api/v1/special-groups",
        serde_json::json!({"name":"two","upstream_sources":sources}),
    )
    .await;
    assert_eq!((status, saved["slot"].as_u64()), (200, Some(50)), "{saved}");
    // Slot 50 is in use again, so its files are not orphans and must stay.
    assert!(blocker.exists(), "an active group's file is never swept");
    assert_eq!(
        health(api).await.1["pending_cleanup"],
        serde_json::json!([])
    );
    let (status, _) = http(
        api,
        "DELETE",
        "/api/v1/special-groups/50",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, 204);
    assert!(!blocker.exists(), "the retried cleanup removed the file");
    assert_eq!(
        health(api).await.1["pending_cleanup"],
        serde_json::json!([])
    );
}

#[test]
fn an_undeletable_group_artifact_after_a_save_is_pending_not_fatal() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let host = tokio::task::LocalSet::new().block_on(&runtime, async {
        let tree = Tree::new("cleanup-save", ONE, true);
        std::fs::write(tree.0.join("webinfo/special_upstream_groups.json"), "[]").unwrap();
        let cancel = mosdns_upstream_core::TransportCancellation::new();
        let (a, stub_a) = stub(10, cancel.clone()).await;
        let (dns, api) = (port(), port());
        cleanup_config(&tree, api, dns, a);
        let host = mosdns_native_host::HostAssembly::from_config_file(&tree.0.join("config.yaml"))
            .unwrap();
        let bound = host.bind_host().await.unwrap();
        let scope = mosdns_upstream_core::TransportCancellation::new();
        let serve_scope = scope.clone();
        let serving = tokio::task::spawn_local(async move {
            bound.serve(serve_scope).await.unwrap();
        });
        let sources = serde_json::json!([{"kind":"group","plugin_tag":"normal"}]);
        let (status, saved) = http(
            api,
            "POST",
            "/api/v1/special-groups",
            serde_json::json!({"name":"one","upstream_sources":sources}),
        )
        .await;
        assert_eq!((status, saved["slot"].as_u64()), (200, Some(50)), "{saved}");
        assert_eq!(
            health(api).await.1["pending_cleanup"],
            serde_json::json!([])
        );

        // The group's manual file cannot be removed: a directory has its name.
        let blocker = tree.0.join("rule/special_50.txt");
        let _ = std::fs::remove_file(&blocker);
        std::fs::create_dir_all(blocker.join("inner")).unwrap();
        let (status, _) = http(
            api,
            "DELETE",
            "/api/v1/special-groups/50",
            serde_json::Value::Null,
        )
        .await;
        assert_eq!(
            status, 204,
            "a committed deletion is not undone by its cleanup"
        );
        let (status, state) = health(api).await;
        assert_eq!(status, 200, "the host stays ready: {state}");
        assert_eq!(state["ready"], true);
        assert_eq!(
            state["pending_cleanup"],
            serde_json::json!(["rule/special_50.txt"])
        );
        assert_eq!(
            query(dns, "after.example.com").await.last(),
            Some(&10),
            "DNS keeps answering"
        );
        let (_, listed) = http(
            api,
            "GET",
            "/api/v1/special-groups",
            serde_json::Value::Null,
        )
        .await;
        assert_eq!(
            listed,
            serde_json::json!([]),
            "the deletion stays committed"
        );

        retried_by_the_next_save(api, &blocker, &sources).await;
        scope.cancel();
        serving.await.unwrap();
        cancel.cancel();
        stub_a.await.unwrap();
        host
    });
    drop(host);
}

#[test]
fn an_undeletable_orphan_at_startup_is_pending_not_fatal() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let host = tokio::task::LocalSet::new().block_on(&runtime, async {
        let tree = Tree::new("cleanup-start", ONE, true);
        std::fs::write(tree.0.join("webinfo/special_upstream_groups.json"), "[]").unwrap();
        let cancel = mosdns_upstream_core::TransportCancellation::new();
        let (a, stub_a) = stub(10, cancel.clone()).await;
        let (dns, api) = (port(), port());
        cleanup_config(&tree, api, dns, a);
        // One orphan that can be removed and one that cannot.
        std::fs::write(tree.0.join("cache/cache_special_61.dump"), "x").unwrap();
        std::fs::create_dir_all(tree.0.join("srs/special_62.json/inner")).unwrap();
        let host = mosdns_native_host::HostAssembly::from_config_file(&tree.0.join("config.yaml"))
            .unwrap();
        let bound = host.bind_host().await.expect("startup continues");
        let scope = mosdns_upstream_core::TransportCancellation::new();
        let serve_scope = scope.clone();
        let serving = tokio::task::spawn_local(async move {
            bound.serve(serve_scope).await.unwrap();
        });
        assert!(!tree.0.join("cache/cache_special_61.dump").exists());
        let (status, state) = health(api).await;
        assert_eq!(status, 200, "{state}");
        assert_eq!(
            state["pending_cleanup"],
            serde_json::json!(["srs/special_62.json"])
        );
        assert_eq!(query(dns, "start.example.com").await.last(), Some(&10));
        // The kernel receive buffer of the UDP listener is visible (and was enlarged).
        let buffers = state["udp_receive_buffers"].as_array().expect("array");
        assert_eq!(buffers.len(), 1, "{state}");
        assert!(
            buffers[0]["listen"]
                .as_str()
                .unwrap()
                .ends_with(&format!(":{dns}"))
        );
        assert!(buffers[0]["bytes"].as_u64().unwrap() > 0, "{state}");
        scope.cancel();
        serving.await.unwrap();
        cancel.cancel();
        stub_a.await.unwrap();
        host
    });
    drop(host);
}
