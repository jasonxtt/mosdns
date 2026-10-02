//! Catalog-level behavior for several native caches in one host.
//!
//! These tests drive the real UDP listener against a controlled loopback
//! upstream. They assert user-visible DNS behavior only: which answers come
//! back, how often the upstream was reached, and which stores hold a value.
//! Nothing here mocks the cache publication path, the generation gate or the
//! catalog itself.

use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{inspect_response_header, validate_response};
use mosdns_native_host::{
    CacheId, CacheKind, CacheTestClock, HostAssembly, HostOptions, UdpServer, compile_yaml,
};
use mosdns_upstream_core::TransportCancellation;

/// A loopback upstream that answers every query with one fixed A record.
struct MockUpstream {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockUpstream {
    fn start() -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").expect("mock upstream bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("mock timeout");
        let address = socket.local_addr().expect("mock local address");
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_count = Arc::clone(&count);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut input = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut input) else {
                    continue;
                };
                thread_count.fetch_add(1, Ordering::SeqCst);
                let response = response_for(&input[..length]);
                socket.send_to(&response, peer).expect("mock response");
            }
        });
        Self {
            address,
            count,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock upstream join");
        }
    }
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = vec![
        (id >> 8) as u8,
        (id & 0xff) as u8,
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
    ];
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).expect("label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    packet
}

/// Echoes the question, sets RA, and answers with one fixed A record.
fn response_for(query: &[u8]) -> Vec<u8> {
    let question_end = question_end(query);
    let mut response = Vec::with_capacity(question_end + 16);
    response.extend_from_slice(&query[..question_end]);
    response[2] = 0x81;
    response[3] = 0x80;
    response[6] = 0;
    response[7] = 1;
    response.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    response.extend_from_slice(&30_u32.to_be_bytes());
    response.extend_from_slice(&[0x00, 0x04, 198, 51, 100, 7]);
    response
}

fn question_end(query: &[u8]) -> usize {
    let mut index = 12;
    while query[index] != 0 {
        index += 1 + usize::from(query[index]);
    }
    index + 5
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

/// Runs the requests sequentially against a freshly bound listener and returns
/// every response together with the observed upstream request count.
fn run_batch(yaml: &str, requests: Vec<Vec<u8>>) -> (Vec<Vec<u8>>, usize) {
    let (responses, requests, _) = run_batch_with_store(yaml, requests);
    (responses, requests)
}

/// Same as [`run_batch`], but also reports how many entries the store behind
/// [`CacheId(0)`] holds at the end, so a test can prove whether a publication
/// happened instead of inferring it from upstream counts alone.
fn run_batch_with_store(yaml: &str, requests: Vec<Vec<u8>>) -> (Vec<Vec<u8>>, usize, u64) {
    let (responses, requests, stored, _) = run_batch_with_audit(yaml, requests);
    (responses, requests, stored)
}

fn run_batch_with_audit(
    yaml: &str,
    requests: Vec<Vec<u8>>,
) -> (Vec<Vec<u8>>, usize, u64, mosdns_native_host::AuditSnapshot) {
    let mock = MockUpstream::start();
    let prepared = yaml.replace("udp://127.0.0.1:15455", &format!("udp://{}", mock.address));
    let config = compile_yaml(&prepared).expect("catalog configuration must compile");
    let assembly = HostAssembly::with_options(
        config,
        HostOptions::default().with_cache_clock(Rc::new(CacheTestClock::new(1000))),
    )
    .expect("catalog assembly");
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let responses = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut collected = Vec::new();
        for request in requests {
            let one = tokio::task::spawn_blocking(move || client_request(listener, &request))
                .await
                .expect("client task");
            collected.push(one);
        }
        shutdown.cancel();
        task.await.expect("server task").expect("server shutdown");
        collected
    });
    let requests_seen = mock.requests();
    let stored = assembly
        .cache()
        .get(CacheId(0))
        .map_or(0, mosdns_native_host::NativeCacheAdapter::len);
    mock.stop();
    (responses, requests_seen, stored, assembly.audit_snapshot())
}

/// Assembles one complete configuration from a plugin block and the entry
/// sequence's rule block. Both blocks are passed with their own indentation so
/// each test can state exactly the configuration it needs.
fn catalog_yaml(plugin_block: &str, entry_rules: &str) -> String {
    let mut yaml = String::from("log:\n  level: error\n\nplugins:\n");
    yaml.push_str(plugin_block);
    yaml.push_str("  - tag: phase5a_forward\n");
    yaml.push_str("    type: forward\n");
    yaml.push_str("    args:\n");
    yaml.push_str("      upstreams:\n");
    yaml.push_str("        - addr: \"udp://127.0.0.1:15455\"\n\n");
    yaml.push_str("  - tag: phase5a_entry\n");
    yaml.push_str("    type: sequence\n");
    yaml.push_str("    args:\n");
    yaml.push_str(entry_rules);
    yaml.push_str("  - tag: phase5a_udp\n");
    yaml.push_str("    type: udp_server\n");
    yaml.push_str("    args:\n");
    yaml.push_str("      entry: phase5a_entry\n");
    yaml.push_str("      listen: \"127.0.0.1:15355\"\n");
    yaml.push_str("      enable_audit: false\n");
    yaml
}

fn named_cache(tag: &str, size: u64) -> String {
    format!("  - tag: {tag}\n    type: cache\n    args:\n      size: {size}\n\n")
}

#[test]
fn two_named_caches_compile_to_two_distinct_stores_with_stable_ids() {
    let mut plugins = named_cache("cache_alpha", 64);
    plugins.push_str("  - tag: cache_beta\n    type: cache\n    args:\n      size: 128\n\n");
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_alpha\n      - exec: $cache_beta\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("two named caches must compile");
    assert_eq!(config.caches.len(), 2, "both named caches are compiled");
    assert_eq!(config.caches[0].tag, "cache_alpha");
    assert_eq!(config.caches[1].tag, "cache_beta");
    assert_eq!(config.caches[0].id.0, 0);
    assert_eq!(config.caches[1].id.0, 1);
    assert_ne!(
        config.caches[0].executable, config.caches[1].executable,
        "each named cache keeps its own dispatch identity"
    );
    assert!(config.caches.iter().all(|c| c.kind == CacheKind::Named));
    assert_eq!(config.caches[0].capacity, 64);
    assert_eq!(config.caches[1].capacity, 128, "per-cache size is honored");
    // Defaults: an absent dump_interval is the product default and there is no
    // implicit dump target.
    assert!(config.caches.iter().all(|c| c.dump_interval_secs == 600));
    assert!(config.caches.iter().all(|c| c.dump_file.is_none()));
    assert_eq!(config.named_caches().count(), 2);

    // Two chained named caches answer normally and reach the upstream once.
    let (responses, requests) = run_batch(&yaml, vec![query(1, "nested.example")]);
    assert_eq!(requests, 1, "a cold chained lookup forwards exactly once");
    assert_eq!(
        inspect_response_header(&responses[0]).expect("header").id,
        1
    );
    validate_response(&responses[0]).expect("valid chained response");
}

#[test]
fn nested_named_caches_publish_independently_and_the_outer_hit_short_circuits() {
    // cache_outer wraps a child sequence that owns cache_inner. The first
    // request misses both and forwards once; every later request is an outer
    // hit and must not reach the upstream at all.
    let mut plugins = named_cache("cache_outer", 64);
    plugins.push_str(&named_cache("cache_inner", 64));
    plugins.push_str("  - tag: inner_sequence\n    type: sequence\n    args:\n      - exec: $cache_inner\n      - exec: $phase5a_forward\n\n");
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_outer\n      - exec: $inner_sequence\n",
    );
    let (responses, requests) = run_batch(
        &yaml,
        vec![
            query(0x2001, "nest.example"),
            query(0x2002, "nest.example"),
            query(0x2003, "nest.example"),
        ],
    );
    assert_eq!(
        requests, 1,
        "a cold nested miss forwards once and every later request is a hit"
    );
    for (index, response) in responses.iter().enumerate() {
        validate_response(response).expect("valid nested response");
        assert_eq!(
            inspect_response_header(response).expect("header").id,
            0x2001 + u16::try_from(index).expect("index"),
            "each cached response keeps its own request id"
        );
        assert_eq!(response[3] & 0x0f, 0, "the cached answer is a NOERROR");
    }
}

#[test]
fn the_same_cache_dispatched_twice_in_one_scope_answers_normally() {
    // A repeated dispatch used to fail closed. It must now build a second
    // frame, publish at both boundaries, and still answer the normal upstream
    // result on the first request and a hit on the second.
    let plugins = named_cache("cache_repeat", 64);
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_repeat\n      - exec: $cache_repeat\n      - exec: $phase5a_forward\n",
    );
    let (responses, requests) = run_batch(
        &yaml,
        vec![
            query(0x3001, "repeat.example"),
            query(0x3002, "repeat.example"),
        ],
    );
    assert_eq!(requests, 1, "the duplicate dispatch still forwards once");
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(
            response[3] & 0x0f,
            0,
            "a repeated cache dispatch must not fail closed"
        );
        assert_eq!(
            inspect_response_header(response).expect("header").id,
            0x3001 + u16::try_from(index).expect("index")
        );
        validate_response(response).expect("valid repeated response");
    }
}

#[test]
fn an_inline_cache_callsite_owns_a_private_instance_without_a_public_identity() {
    let yaml = catalog_yaml(
        "",
        "      - exec: cache 32\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("an inline cache callsite must compile");
    assert_eq!(config.caches.len(), 1, "the inline form compiles one cache");
    let quick = &config.caches[0];
    assert_eq!(quick.kind, CacheKind::Quick);
    assert_eq!(quick.capacity, 32);
    assert_eq!(
        quick.lazy_cache_ttl_secs, 0,
        "an inline cache never enables lazy retention"
    );
    assert!(
        quick.public_tag().is_none(),
        "a quick callsite has no management identity"
    );
    assert_eq!(
        config.named_caches().count(),
        0,
        "quick caches never appear in the named catalog"
    );

    let (responses, requests) = run_batch(
        &yaml,
        vec![
            query(0x4001, "quick.example"),
            query(0x4002, "quick.example"),
        ],
    );
    assert_eq!(requests, 1, "the inline cache is consulted and does hit");
    assert_eq!(
        inspect_response_header(&responses[1]).expect("header").id,
        0x4002
    );
}

#[test]
fn each_inline_cache_callsite_is_a_separate_instance() {
    let yaml = catalog_yaml(
        "",
        "      - exec: cache 16\n      - exec: cache 16\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("two inline callsites must compile");
    assert_eq!(config.caches.len(), 2);
    assert!(
        config
            .caches
            .iter()
            .all(|cache| cache.kind == CacheKind::Quick),
        "both instances are private quick caches"
    );
    assert_ne!(
        config.caches[0].executable, config.caches[1].executable,
        "every callsite dispatches its own executable"
    );
    assert_ne!(
        config.caches[0].tag, config.caches[1].tag,
        "every callsite has a distinct private identity"
    );
    // The first callsite's hit short-circuits the second, so the upstream is
    // reached exactly once for two identical requests.
    let (_, requests) = run_batch(
        &yaml,
        vec![query(0x5001, "two.example"), query(0x5002, "two.example")],
    );
    assert_eq!(requests, 1);
}

#[test]
fn a_quick_cache_and_a_named_cache_coexist_and_stay_distinguishable() {
    let plugins = named_cache("cache_named", 64);
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_named\n      - exec: cache 8\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("mixed named and quick caches must compile");
    assert_eq!(config.caches.len(), 2);
    assert_eq!(config.caches[0].kind, CacheKind::Named);
    assert_eq!(config.caches[1].kind, CacheKind::Quick);
    assert_eq!(config.named_caches().count(), 1);
    assert_eq!(
        config.named_caches().next().expect("named cache").tag,
        "cache_named"
    );
    let (responses, requests) = run_batch(
        &yaml,
        vec![
            query(0x6001, "mixed.example"),
            query(0x6002, "mixed.example"),
        ],
    );
    assert_eq!(requests, 1);
    validate_response(&responses[0]).expect("valid mixed response");
}

#[test]
fn every_configured_named_cache_is_listed_in_configuration_order() {
    let mut plugins = named_cache("zeta", 1);
    plugins.push_str(&named_cache("alpha", 2));
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $zeta\n      - exec: $alpha\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("declaration order is stable");
    let tags: Vec<&str> = config
        .named_caches()
        .map(|cache| cache.tag.as_str())
        .collect();
    assert_eq!(tags, vec!["zeta", "alpha"], "config order is preserved");
    assert_eq!(config.caches[0].capacity, 1);
    assert_eq!(config.caches[1].capacity, 2);
}

/// Two names must not share an entry, and the cached response must echo the
/// question of the request it answers.
#[test]
fn distinct_names_do_not_share_a_cache_entry() {
    let plugins = named_cache("cache_iso", 64);
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_iso\n      - exec: $phase5a_forward\n",
    );
    let first = query(0x7001, "one.example");
    let second = query(0x7002, "two.example");
    let (responses, requests) =
        run_batch(&yaml, vec![first.clone(), second.clone(), first, second]);
    assert_eq!(requests, 2, "each distinct name misses once");
    // Each cached answer must echo the question of the request it answers.
    assert_eq!(
        &responses[2][12..question_end(&responses[2])],
        b"\x03one\x07example\x00\x00\x01\x00\x01"
    );
    assert_eq!(
        &responses[3][12..question_end(&responses[3])],
        b"\x03two\x07example\x00\x00\x01\x00\x01"
    );
}

/// A fallback whose primary is a cache tag. `exit_after_forward` controls
/// whether the fallback's successor produces its response and then stops
/// through `exit`.
fn fallback_cache_target_yaml(exit_after_forward: bool) -> String {
    let mut plugins =
        String::from("  - tag: cache_target\n    type: cache\n    args:\n      size: 64\n\n");
    plugins.push_str("  - tag: fb\n    type: fallback\n    args:\n      primary: $cache_target\n      secondary: $forward_secondary\n\n");
    plugins.push_str("  - tag: forward_secondary\n    type: forward\n    args:\n      upstreams:\n        - addr: \"udp://127.0.0.1:15454\"\n\n");
    let mut entry = String::from("      - exec: $fb\n      - exec: $phase5a_forward\n");
    if exit_after_forward {
        entry.push_str("      - exec: exit\n");
    }
    catalog_yaml(&plugins, &entry)
}

#[test]
fn a_fallback_cache_target_publishes_only_after_a_natural_successor_completion() {
    // The successor ([$phase5a_forward]) produces a response and then the
    // enclosing scope ends normally. That is a real completion, so the cache
    // target's miss must be published.
    let yaml = fallback_cache_target_yaml(false);
    let (responses, requests, stored) = run_batch_with_store(
        &yaml,
        vec![query(0x8001, "fb.example"), query(0x8002, "fb.example")],
    );
    assert_eq!(
        requests, 1,
        "the published entry must serve the second request from cache"
    );
    assert_eq!(stored, 1, "a natural completion must publish the entry");
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(response[3] & 0x0f, 0, "the fallback answer is a NOERROR");
        assert_eq!(
            inspect_response_header(response).expect("header").id,
            0x8001 + u16::try_from(index).expect("index")
        );
        validate_response(response).expect("valid fallback response");
    }
}

#[test]
fn a_fallback_cache_target_never_publishes_a_successor_that_exited() {
    // The same configuration with a trailing `exit`: the successor still
    // produces a response, but it stopped instead of completing. A cache
    // reached through an ordinary sequence dispatch is invalidated by
    // `MachineStep::ScopeAborted` in exactly this situation, so the direct
    // cache target must not store anything either.
    let yaml = fallback_cache_target_yaml(true);
    let (responses, requests, stored) = run_batch_with_store(
        &yaml,
        vec![
            query(0x8101, "fbexit.example"),
            query(0x8102, "fbexit.example"),
        ],
    );
    assert_eq!(
        stored, 0,
        "an exited successor must not publish, exactly like a ScopeAborted boundary"
    );
    assert_eq!(
        requests, 2,
        "with nothing cached each request must reach the upstream again"
    );
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(response[3] & 0x0f, 0, "the answer itself is unaffected");
        assert_eq!(
            inspect_response_header(response).expect("header").id,
            0x8101 + u16::try_from(index).expect("index")
        );
        validate_response(response).expect("valid fallback response");
    }
}

#[test]
fn cache_hit_restores_the_successor_domain_set_in_the_final_query_audit() {
    let mut plugins = named_cache("cache_a", 64);
    plugins.push_str(
        "  - tag: routed\n    type: domain_set\n    args:\n      exps: [full:metadata.example]\n",
    );
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_a\n      - matches: qname $routed\n        exec: $phase5a_forward\n",
    )
    .replace("enable_audit: false", "enable_audit: true");
    let (_, requests, _, audit) = run_batch_with_audit(
        &yaml,
        vec![
            query(0x9101, "metadata.example"),
            query(0x9102, "metadata.example"),
        ],
    );
    assert_eq!(requests, 1);
    assert_eq!(audit.records.len(), 2);
    assert_eq!(audit.records[0].domain_set.as_deref(), Some("routed"));
    assert_eq!(audit.records[1].domain_set.as_deref(), Some("routed"));
    assert_eq!(
        audit.records[1].cache_status,
        mosdns_native_host::CacheStatus::Hit
    );
}

#[test]
fn positive_lazy_retention_is_accepted_for_named_caches_only() {
    let plugins = named_cache("cache_a", 64).replace(
        "      size: 64\n",
        "      size: 64\n      lazy_cache_ttl: 60\n",
    );
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_a\n      - exec: $phase5a_forward\n",
    );
    let config = compile_yaml(&yaml).expect("lazy lifecycle is supported");
    assert_eq!(config.caches[0].lazy_cache_ttl_secs, 60);
}

#[test]
#[allow(clippy::too_many_lines)] // One real lifecycle scenario, matching existing HTTP proof tests.
fn lazy_refresh_outlives_the_client_merges_followers_and_leaves_audit_unchanged() {
    let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    let peer = socket.local_addr().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let count = Arc::new(AtomicUsize::new(0));
    let (release_work, stop_work, count_work) = (release.clone(), stop.clone(), count.clone());
    let worker = thread::spawn(move || {
        let mut wire = vec![0; 65535];
        let mut pending = Vec::new();
        while !stop_work.load(Ordering::SeqCst) {
            if let Ok((len, client)) = socket.recv_from(&mut wire) {
                let n = count_work.fetch_add(1, Ordering::SeqCst);
                let response = response_for(&wire[..len]);
                if n == 0 {
                    socket.send_to(&response, client).unwrap();
                } else {
                    pending.push((response, client));
                }
            }
            if release_work.load(Ordering::SeqCst) {
                for (response, client) in pending.drain(..) {
                    socket.send_to(&response, client).unwrap();
                }
            }
        }
    });
    let plugins = named_cache("cache_a", 64).replace(
        "      size: 64\n",
        "      size: 64\n      lazy_cache_ttl: 90\n",
    );
    let plugins = format!(
        "{plugins}{}",
        named_cache("cache_b", 64).replace(
            "      size: 64\n",
            "      size: 64\n      lazy_cache_ttl: 90\n"
        )
    );
    let yaml = catalog_yaml(
        &plugins,
        "      - exec: $cache_a\n      - exec: $cache_b\n      - exec: $phase5a_forward\n",
    )
    .replace("udp://127.0.0.1:15455", &format!("udp://{peer}"))
    .replace("enable_audit: false", "enable_audit: true");
    let clock = CacheTestClock::new(100);
    let assembly = HostAssembly::with_options(
        compile_yaml(&yaml).unwrap(),
        HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
    )
    .unwrap();
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let addr = server.local_addr().unwrap();
    let shutdown = TransportCancellation::new();
    assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let first = query(1, "lazy.example");
        tokio::task::spawn_blocking(move || client_request(addr, &first))
            .await
            .unwrap();
        clock.advance(31);
        for id in 2..=3 {
            let q = query(id, "lazy.example");
            let answer = tokio::task::spawn_blocking(move || client_request(addr, &q))
                .await
                .unwrap();
            assert_eq!(
                mosdns_dns_core::observe_response_ttl(&answer)
                    .unwrap()
                    .minimal_ttl,
                5
            );
        }
        let end = std::time::Instant::now() + Duration::from_secs(1);
        while count.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "one actual refresh despite followers"
        );
        assert_eq!(
            assembly
                .cache()
                .get(CacheId(0))
                .unwrap()
                .pending_refreshes(),
            1
        );
        assert_eq!(
            assembly
                .cache()
                .get(CacheId(1))
                .unwrap()
                .pending_refreshes(),
            0,
            "nested lazy cache stays inline on the refresh root"
        );
        assert_eq!(assembly.audit_snapshot().records.len(), 3);
        release.store(true, Ordering::SeqCst);
        let end = std::time::Instant::now() + Duration::from_secs(1);
        while assembly
            .cache()
            .get(CacheId(0))
            .unwrap()
            .pending_refreshes()
            != 0
            && std::time::Instant::now() < end
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            assembly
                .cache()
                .get(CacheId(0))
                .unwrap()
                .pending_refreshes(),
            0
        );
        let q = query(4, "lazy.example");
        let answer = tokio::task::spawn_blocking(move || client_request(addr, &q))
            .await
            .unwrap();
        assert_eq!(
            mosdns_dns_core::observe_response_ttl(&answer)
                .unwrap()
                .minimal_ttl,
            30
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(
            assembly.audit_snapshot().records.len(),
            4,
            "background work creates no client audit"
        );
        assert_eq!(
            assembly.metrics_snapshot().admitted_total,
            4,
            "background creates no client admission"
        );
        assert_eq!(
            assembly
                .metrics_snapshot()
                .forward_attempts_by_upstream
                .values()
                .map(|m| m.attempts_total)
                .sum::<u64>(),
            2,
            "actual refresh upstream work has its own metrics"
        );
        shutdown.cancel();
        server_task.await.unwrap().unwrap();
        assert_eq!(
            assembly
                .cache()
                .get(CacheId(0))
                .unwrap()
                .pending_refreshes(),
            0
        );
    });
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap();
}

#[test]
fn detached_successor_owns_its_state_and_rebinds_exactly_one_64_fuel_root() {
    use mosdns_sequence_core::{
        CancellationToken, ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep,
        RootFuelHandle,
    };
    let mut rules = String::from("      - exec: $cache_a\n");
    for _ in 0..70 {
        rules.push_str("      - exec: $phase5a_forward\n");
    }
    let config = compile_yaml(&catalog_yaml(&named_cache("cache_a", 64), &rules)).unwrap();
    let q = query(1, "fuel.example");
    let (header, question) = mosdns_dns_core::parse_query(&q).unwrap();
    let cancellation = CancellationToken::new();
    let client_fuel = RootFuelHandle::new(1);
    let mut client = config
        .new_machine(
            ExecutionState::new(header, question),
            ExecutionControl::with_shared_budget(client_fuel.clone(), cancellation.clone()),
        )
        .unwrap();
    assert!(matches!(client.step().unwrap(), MachineStep::Dispatch(_)));
    let recipe = client.capture_successor().unwrap();
    client.state_mut().set_raw_response(response_for(&q));
    cancellation.cancel();
    drop(client);
    assert_eq!(client_fuel.remaining(), 0);
    let refresh_fuel = RootFuelHandle::new(64);
    let mut refresh = recipe
        .bind(
            &config.program,
            ExecutionControl::with_shared_budget(refresh_fuel.clone(), CancellationToken::new()),
        )
        .unwrap();
    assert!(
        matches!(
            refresh.state().response,
            mosdns_sequence_core::ResponseState::None
        ),
        "capture precedes stale response"
    );
    let mut step = refresh.step().unwrap();
    for index in 0..64 {
        let MachineStep::Dispatch(dispatch) = step else {
            panic!("expected external");
        };
        let next = refresh.resume(dispatch.executable(), Ok(ExecutorOutcome::Continue));
        if index == 63 {
            assert!(next.is_err(), "all nested work uses the finite shared root");
            return;
        }
        step = next.unwrap();
    }
    panic!("budget must be exhausted");
}
