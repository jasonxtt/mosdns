use mosdns_native_host::compile_yaml;
fn config(enabled: bool, rules: &str, extra: &str) -> String {
    format!(
        "log: {{level: error}}\nplugins:\n  - tag: cache\n    type: cache\n    args: {{enable_ecs: {enabled}}}\n  - tag: ecs\n    type: ecs_handler\n    args: {{send: true}}\n  - tag: up\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:19907'}}]}}\n{extra}  - tag: main\n    type: sequence\n    args:\n{rules}\n  - tag: listener\n    type: udp_server\n    args: {{entry: main, listen: '127.0.0.1:19908', enable_audit: false}}\n"
    )
}
#[test]
fn ecs_cache_is_explicitly_opt_in_and_rejects_unsafe_placement() {
    assert!(
        compile_yaml(&config(
            true,
            "      - exec: $ecs\n      - exec: $cache\n      - exec: $up",
            ""
        ))
        .is_ok()
    );
    for enabled in [true, false] {
        assert!(
            compile_yaml(&config(
                enabled,
                "      - exec: $cache\n      - exec: $ecs\n      - exec: $up",
                ""
            ))
            .is_err()
        );
    }
}

#[test]
fn placement_covers_control_flow_and_own_boundary() {
    let child = "  - tag: child\n    type: sequence\n    args: [{exec: '$ecs'}]\n";
    for enabled in [false, true] {
        for cache in ["$cache", "cache"] {
            for effect in [
                "$ecs",
                "ecs 192.0.2.1",
                "$child",
                "jump $child",
                "goto $child",
                "try $child",
                "prefer_ipv4",
            ] {
                let after = if effect == "prefer_ipv4" {
                    "      - exec: $ecs\n"
                } else {
                    ""
                };
                let yaml = config(
                    enabled,
                    &format!(
                        "      - exec: {cache}\n      - exec: {effect}\n{after}      - exec: $up"
                    ),
                    child,
                );
                let error = compile_yaml(&yaml)
                    .err()
                    .expect("unsafe cache rejected")
                    .to_string();
                assert!(
                    error.contains("cache") && error.contains("ecs"),
                    "{effect}: {error}"
                );
            }
            let yaml = config(
                enabled,
                &format!(
                    "      - exec: {cache}\n      - matches: '!client_ip 192.0.2.0/24'\n        exec: $up"
                ),
                "",
            );
            assert!(compile_yaml(&yaml).is_err());
            let fallback = "  - tag: choice\n    type: fallback\n    args: {primary: '$child', secondary: '$up'}\n";
            assert!(
                compile_yaml(&config(
                    enabled,
                    &format!("      - exec: {cache}\n      - exec: $choice"),
                    &format!("{child}{fallback}")
                ))
                .is_err()
            );
            let recursive = "  - tag: child\n    type: sequence\n    args: [{exec: '$loop'}, {exec: '$ecs'}]\n  - tag: loop\n    type: sequence\n    args: [{exec: '$child'}]\n";
            assert!(
                compile_yaml(&config(
                    enabled,
                    &format!("      - exec: {cache}\n      - exec: $child"),
                    recursive
                ))
                .is_err()
            );
            assert!(
                compile_yaml(&config(
                    enabled,
                    &format!("      - exec: {cache}\n      - exec: reject 0\n      - exec: $ecs"),
                    ""
                ))
                .is_ok()
            );
            assert!(
                compile_yaml(&config(
                    enabled,
                    &format!("      - exec: {cache}\n      - exec: ecs\n      - exec: $up"),
                    ""
                ))
                .is_ok()
            );
            // Inline cache ends at its own list boundary; a later outer policy
            // must not invalidate that already completed publication scope.
            assert!(
                compile_yaml(&config(
                    enabled,
                    &format!("      - exec: ['{cache}', '$up']\n      - exec: $ecs"),
                    ""
                ))
                .is_ok()
            );
        }
    }
    let inherited =
        "  - tag: choice\n    type: fallback\n    args: {primary: '$cache', secondary: '$up'}\n";
    assert!(
        compile_yaml(&config(
            true,
            "      - exec: $choice\n      - exec: $ecs\n      - exec: $up",
            inherited
        ))
        .is_err()
    );
}

fn query(ecs: Option<&[u8]>) -> Vec<u8> {
    let mut wire = vec![0, 45, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 0, 0, 1, 0, 1];
    if let Some(data) = ecs {
        wire[11] = 1;
        wire.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 0, 0]);
        wire.extend_from_slice(&u16::try_from(data.len() + 4).unwrap().to_be_bytes());
        wire.extend_from_slice(&[0, 8]);
        wire.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
        wire.extend_from_slice(data);
    }
    wire
}
fn response(raw: &[u8], last: u8) -> Vec<u8> {
    let mut wire = raw[..19].to_vec();
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire[7] = 1;
    wire[11] = 0;
    wire.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, last]);
    wire
}
#[test]
fn full_ecs_keys_partition_exact_network_family_and_mask() {
    use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};
    let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100))
        .unwrap()
        .with_ecs(true);
    let base = query(None);
    let plain = NativeCacheAdapter::for_test(CacheTestClock::new(100)).unwrap();
    assert_eq!(
        cache.key_for_query(&base).unwrap(),
        plain.key_for_query(&base).unwrap()
    );
    let queries = [
        query(Some(&[0, 1, 24, 0, 203, 0, 113])),
        query(Some(&[0, 1, 24, 0, 192, 0, 2])),
        query(Some(&[0, 1, 16, 0, 203, 0])),
        query(Some(&[0, 2, 24, 0, 0x20, 1, 0x0d])),
    ];
    for (i, raw) in queries.iter().enumerate() {
        let last = u8::try_from(i + 1).unwrap();
        assert!(plain.begin_store(raw).unwrap().is_none());
        assert!(
            cache
                .begin_store(raw)
                .unwrap()
                .unwrap()
                .publish(&response(raw, last))
                .unwrap()
        );
    }
    for (i, raw) in queries.iter().enumerate() {
        assert_eq!(
            *cache.lookup(raw).unwrap().unwrap().last().unwrap(),
            u8::try_from(i + 1).unwrap()
        );
    }
    let key = cache.key_for_query(&queries[0]).unwrap().unwrap();
    assert!(key.ends_with(b"\x10203.0.113.0/24/0"));
    let key6 = cache.key_for_query(&queries[3]).unwrap().unwrap();
    assert!(key6.ends_with(b"[2001:d00::]/24/0"));
    for invalid in [
        vec![0, 0, 0, 0],
        vec![0, 1, 24, 1, 203, 0, 113],
        vec![0, 1, 25, 0, 203, 0, 113, 1],
        vec![0, 1, 32, 0, 203],
    ] {
        assert!(cache.begin_store(&query(Some(&invalid))).unwrap().is_none());
    }
}

#[test]
fn jump_inherited_continuation_is_checked_but_child_boundary_is_respected() {
    let child = "  - tag: child\n    type: sequence\n    args: [{exec: '$cache'}, {exec: '$up'}]\n";
    assert!(
        compile_yaml(&config(
            true,
            "      - exec: jump $child\n      - exec: $ecs",
            child
        ))
        .is_err()
    );
    assert!(
        compile_yaml(&config(
            true,
            "      - exec: $child\n      - exec: $ecs",
            child
        ))
        .is_ok()
    );
    let inherited =
        "  - tag: choice\n    type: fallback\n    args: {primary: '$child', secondary: '$up'}\n";
    assert!(
        compile_yaml(&config(
            true,
            "      - exec: $choice\n      - exec: $ecs",
            &format!("{child}{inherited}")
        ))
        .is_err()
    );
}

#[test]
fn refresh_singleflight_uses_full_ecs_key() {
    use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};
    use std::{cell::Cell, rc::Rc};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100))
            .unwrap()
            .with_ecs(true);
        let a = query(Some(&[0, 1, 24, 0, 203, 0, 113]));
        let b = query(Some(&[0, 1, 24, 0, 192, 0, 2]));
        let builds = Rc::new(Cell::new(0));
        let mut releases = Vec::new();
        for raw in [&a, &b] {
            let (send, receive) = tokio::sync::oneshot::channel();
            let builds = builds.clone();
            let answer = response(raw, if raw == &a { 1 } else { 2 });
            assert!(
                cache
                    .start_refresh(raw, move |token, _, _| {
                        builds.set(builds.get() + 1);
                        async move {
                            receive.await.unwrap();
                            token.publish(&answer).unwrap();
                        }
                    })
                    .unwrap()
            );
            releases.push(send);
        }
        assert!(
            !cache
                .start_refresh(&a, |_, _, _| async { panic!("follower must not build") })
                .unwrap()
        );
        assert_eq!(builds.get(), 2);
        for send in releases {
            send.send(()).unwrap();
        }
        tokio::task::yield_now().await;
        assert_eq!(*cache.lookup(&a).unwrap().unwrap().last().unwrap(), 1);
        assert_eq!(*cache.lookup(&b).unwrap().unwrap().last().unwrap(), 2);
        cache.stop_refreshes().await.unwrap();
    });
}

#[test]
fn family2_mapped_string_matches_go_without_changing_wire_family() {
    use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};
    let cache = NativeCacheAdapter::for_test(CacheTestClock::new(100))
        .unwrap()
        .with_ecs(true);
    let mut payload = vec![0, 2, 120, 0];
    payload.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 192, 0, 2]);
    let key = cache
        .key_for_query(&query(Some(&payload)))
        .unwrap()
        .unwrap();
    assert!(key.ends_with(b"192.0.2.0/120/0"));
    assert_ne!(
        key,
        cache
            .key_for_query(&query(Some(&[0, 1, 24, 0, 192, 0, 2])))
            .unwrap()
            .unwrap()
    );
}
