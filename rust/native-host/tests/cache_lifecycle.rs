use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};

fn query(name: &[u8]) -> Vec<u8> {
    let mut wire = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    wire.extend_from_slice(name);
    wire.extend_from_slice(&[0, 1, 0, 1]);
    wire
}

#[test]
fn product_key_preserves_case_and_escaped_labels_with_independent_flags() {
    let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).unwrap();
    let mut q = query(&[3, b'A', b'.', 0, 0]);
    assert_eq!(
        cache.key_for_query(&q).unwrap().unwrap(),
        b"\x00\x00\x01\x08A\\.\\000."
    );
    q[3] = 0x30;
    assert_eq!(cache.key_for_query(&q).unwrap().unwrap()[0], 3);
    q[3] = 0;
    q[11] = 1;
    q.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 128, 0, 0, 0]);
    assert_eq!(cache.key_for_query(&q).unwrap().unwrap()[0], 4);
    q[3] = 0x30;
    assert_eq!(cache.key_for_query(&q).unwrap().unwrap()[0], 7);
}

fn response(q: &[u8], ttl: u32, opt: bool) -> Vec<u8> {
    let (_, question) = mosdns_dns_core::parse_query(q).unwrap();
    let mut wire = vec![0, 1, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, u8::from(opt)];
    wire.extend_from_slice(&question.qname_wire);
    wire.extend_from_slice(&[0, 1, 0, 1]);
    wire.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1]);
    wire.extend_from_slice(&ttl.to_be_bytes());
    wire.extend_from_slice(&[0, 4, 192, 0, 2, 1]);
    if opt {
        wire.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 128, 0, 0, 0]);
    }
    wire
}

#[test]
fn opt_is_removed_from_cache_copy_and_domain_set_survives_wall_rollback() {
    let clock = CacheTestClock::new(10);
    clock.set_wall(1_700_000_000);
    let cache = NativeCacheAdapter::for_test(clock.clone()).unwrap();
    let q = query(&[1, b'A', 0]);
    let wire = response(&q, 60, true);
    assert!(
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish_with_domain(&wire, "route-A")
            .unwrap()
    );
    assert_eq!(wire[11], 1, "miss response must retain OPT");
    clock.set_wall(1);
    clock.advance(10);
    let hit = cache.lookup_entry(&q).unwrap().unwrap();
    assert_eq!(hit.domain_set, "route-A");
    assert_eq!(hit.response[11], 0);
    assert_eq!(
        mosdns_dns_core::observe_response_ttl(&hit.response)
            .unwrap()
            .minimal_ttl,
        50
    );
    clock.advance(50);
    assert!(cache.lookup(&q).unwrap().is_none());
}

#[test]
fn ecs_malformed_options_and_unsupported_versions_bypass_without_key_aliasing() {
    let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).unwrap();
    let plain = query(&[1, b'a', 0]);
    for opt in [
        vec![0, 0, 41, 4, 208, 0, 1, 0, 0, 0, 0], // EDNS version 1
        vec![0, 0, 41, 4, 208, 1, 0, 0, 0, 0, 0], // extended rcode
        vec![0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 4, 0, 8, 0, 0], // ECS even empty
        vec![0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 1, 1], // bad option framing
    ] {
        let mut q = plain.clone();
        q[11] = 1;
        q.extend_from_slice(&opt);
        assert!(cache.key_for_query(&q).unwrap().is_none());
    }
}

#[test]
fn lazy_retention_is_from_storage_and_excluded_answers_do_not_replace_entries() {
    let clock = CacheTestClock::new(100);
    let cache =
        NativeCacheAdapter::with_options_and_clock(8, 20, std::rc::Rc::new(clock.clone())).unwrap();
    let q = query(&[1, b'a', 0]);
    assert!(
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&response(&q, 10, false))
            .unwrap()
    );
    clock.advance(10);
    let hit = cache.lookup_entry(&q).unwrap().unwrap();
    assert_eq!(hit.state, mosdns_cache_core::LookupState::Lazy);
    assert_eq!(
        mosdns_dns_core::observe_response_ttl(&hit.response)
            .unwrap()
            .minimal_ttl,
        5
    );
    clock.advance(10);
    assert!(cache.lookup(&q).unwrap().is_none());
    let excluded = NativeCacheAdapter::for_test(clock)
        .unwrap()
        .with_exclusions(&["192.0.2.0/24".into(), "invalid".into()]);
    assert!(
        !excluded
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&response(&q, 60, false))
            .unwrap()
    );
    assert!(excluded.lookup(&q).unwrap().is_none());
}

#[test]
fn original_wall_timestamps_are_preserved_and_opt_removal_rebuilds_later_pointers() {
    let clock = CacheTestClock::new(7);
    clock.set_wall(1000);
    let cache = NativeCacheAdapter::for_test(clock.clone()).unwrap();
    let q = query(&[1, b'a', 0]);
    let mut wire = response(&q, 60, true);
    let opt_start = wire.len() - 11;
    wire[11] = 2;
    wire.extend_from_slice(&[
        0xc0,
        12,
        0,
        5,
        0,
        1,
        0,
        0,
        0,
        30,
        0,
        2,
        0xc0,
        u8::try_from(opt_start).unwrap(),
    ]);
    assert!(
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&wire)
            .unwrap()
    );
    let snapshot = cache.snapshot().unwrap();
    assert_eq!(snapshot[0].times, [7, 37, 37]); // minimum includes additional CNAME
    assert_eq!(snapshot[0].wall_times, Some([1000, 1030, 1030]));
    clock.set_wall(1);
    clock.advance(5);
    assert_eq!(
        cache.snapshot().unwrap()[0].wall_times,
        Some([1000, 1030, 1030])
    );
    let hit = cache.lookup(&q).unwrap().unwrap();
    assert_eq!(hit[11], 1);
    let parsed = hickory_proto::op::Message::from_vec(&hit).unwrap();
    assert!(parsed.extensions().is_none());
    assert_eq!(parsed.additionals()[0].data().to_string(), ".");
}

#[test]
fn oversized_text_keys_bypass_and_checked_wall_expiry_overflow_does_not_store() {
    let clock = CacheTestClock::new(1);
    let cache = NativeCacheAdapter::for_test(clock.clone()).unwrap();
    let mut name = Vec::new();
    for _ in 0..4 {
        name.push(60);
        name.extend_from_slice(&[b'.'; 60]);
    }
    name.push(0);
    assert!(cache.key_for_query(&query(&name)).unwrap().is_none());
    let q = query(&[1, b'a', 0]);
    clock.set_wall(i64::MAX as u64);
    assert!(
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&response(&q, 60, false))
            .is_err()
    );
    assert!(cache.lookup(&q).unwrap().is_none());
}

#[test]
fn refresh_owner_merges_keys_limits_concurrency_and_cancels_drains_on_stop() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async {
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(0)).unwrap();
        let q = query(&[1, b'a', 0]);
        assert!(
            cache
                .start_refresh(&q, |_, _, _| async {
                    std::future::pending::<()>().await;
                })
                .unwrap()
        );
        assert!(
            !cache
                .start_refresh(&q, |_, _, _| -> std::future::Pending<()> {
                    panic!("follower must not construct work");
                })
                .unwrap()
        );
        for n in 0..255_u16 {
            let label = n.to_string();
            let mut name = vec![u8::try_from(label.len()).unwrap()];
            name.extend_from_slice(label.as_bytes());
            name.push(0);
            assert!(
                cache
                    .start_refresh(&query(&name), |_, _, _| async {
                        std::future::pending::<()>().await;
                    })
                    .unwrap()
            );
        }
        assert_eq!(cache.pending_refreshes(), 256);
        assert!(
            !cache
                .start_refresh(
                    &query(&[1, b'z', 0]),
                    |_, _, _| -> std::future::Pending<()> {
                        panic!("full owner must not construct work");
                    }
                )
                .unwrap()
        );
        cache.stop_refreshes().await.unwrap();
        assert_eq!(cache.pending_refreshes(), 0);
        assert!(
            !cache
                .start_refresh(&q, |_, _, _| -> std::future::Pending<()> {
                    panic!("stopped owner");
                })
                .unwrap()
        );
    });
}

#[test]
fn refresh_deadline_releases_ownership_without_overwriting_the_old_lazy_value() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let clock = CacheTestClock::new(0);
        let cache =
            NativeCacheAdapter::with_options_and_clock(8, 90, std::rc::Rc::new(clock.clone()))
                .unwrap();
        let q = query(&[1, b'a', 0]);
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&response(&q, 1, false))
            .unwrap();
        clock.advance(2);
        assert!(
            cache
                .start_refresh(&q, |_, _, deadline| {
                    assert!(
                        deadline.saturating_duration_since(std::time::Instant::now())
                            <= std::time::Duration::from_secs(5)
                    );
                    async {
                        std::future::pending::<()>().await;
                    }
                })
                .unwrap()
        );
        tokio::time::sleep(std::time::Duration::from_millis(5100)).await;
        assert_eq!(cache.pending_refreshes(), 0);
        assert_eq!(
            cache.lookup_entry(&q).unwrap().unwrap().state,
            mosdns_cache_core::LookupState::Lazy
        );
        cache.stop_refreshes().await.unwrap();
    });
}

#[test]
fn dump_paths_use_declaration_base_and_duplicate_normalized_targets_are_rejected() {
    let yaml = "log: {level: error}\nplugins:\n  - tag: a\n    type: cache\n    args: {dump_file: ./state/../a.dump}\n  - tag: f\n    type: forward\n    args: {upstreams: [{addr: udp://127.0.0.1:5301}]}\n  - tag: main\n    type: sequence\n    args: [{exec: $a}, {exec: $f}]\n  - tag: dns\n    type: udp_server\n    args: {listen: '127.0.0.1:5311', entry: main, enable_audit: false}\n";
    let config =
        mosdns_native_host::compile_yaml_with_base(yaml, std::path::Path::new("/tmp/cache-base"))
            .unwrap();
    assert_eq!(
        config.caches[0].dump_file.as_deref(),
        Some(std::path::Path::new("/tmp/cache-base/a.dump"))
    );
    let duplicate = yaml.replace(
        "  - tag: f",
        "  - tag: b\n    type: cache\n    args: {dump_file: a.dump}\n  - tag: f",
    );
    assert!(
        mosdns_native_host::compile_yaml_with_base(
            &duplicate,
            std::path::Path::new("/tmp/cache-base")
        )
        .is_err()
    );
}

fn local_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn temporary_dump() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "cache-native-s5-{}-{}.dump",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}
async fn wait_persist(gate: &mosdns_native_host::PersistGate) {
    for _ in 0..1000 {
        if gate.arrived() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("persistence did not enter blocking gate");
}

#[test]
fn dump_roundtrip_restores_original_age_lazy_and_domain_with_all_or_nothing_validation() {
    tokio::task::LocalSet::new().block_on(&local_runtime(), async {
        let clock = CacheTestClock::new(0);
        clock.set_wall(1000);
        let source =
            NativeCacheAdapter::with_options_and_clock(64, 90, std::rc::Rc::new(clock.clone()))
                .unwrap();
        let q = query(&[1, b'A', 0]);
        source
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish_with_domain(&response(&q, 30, false), "domain-A")
            .unwrap();
        let bytes = source.dump().unwrap();
        let restart = CacheTestClock::new(2);
        restart.set_wall(1040);
        let target =
            NativeCacheAdapter::with_options_and_clock(64, 90, std::rc::Rc::new(restart.clone()))
                .unwrap();
        target.import_dump(bytes.clone()).await.unwrap();
        let hit = target.lookup_entry(&q).unwrap().unwrap();
        assert_eq!(hit.state, mosdns_cache_core::LookupState::Lazy);
        assert_eq!(hit.domain_set, "domain-A");
        assert_eq!(
            target.snapshot().unwrap()[0].wall_times,
            Some([1000, 1030, 1090])
        );
        let old = target.begin_store(&q).unwrap().unwrap();
        let mut corrupt = bytes.clone();
        let index = corrupt.len() - 8;
        corrupt[index] ^= 1;
        assert!(target.import_dump(corrupt).await.is_err());
        assert_eq!(
            target.lookup_entry(&q).unwrap().unwrap().domain_set,
            "domain-A"
        );
        assert!(
            old.publish(&response(&q, 30, false)).unwrap(),
            "invalid import keeps generation"
        );
        restart.set_wall(999);
        assert!(
            target.import_dump(bytes.clone()).await.is_err(),
            "future stored rejects entire dump"
        );
        restart.set_wall(1040);
        let no_lazy =
            NativeCacheAdapter::with_options_and_clock(64, 0, std::rc::Rc::new(restart.clone()))
                .unwrap();
        no_lazy.import_dump(bytes.clone()).await.unwrap();
        assert!(no_lazy.lookup(&q).unwrap().is_none());
        restart.set_wall(1090);
        let expired =
            NativeCacheAdapter::with_options_and_clock(64, 90, std::rc::Rc::new(restart)).unwrap();
        expired.import_dump(bytes).await.unwrap();
        assert!(expired.lookup(&q).unwrap().is_none());
    });
}

#[test]
fn durable_flush_failure_keeps_file_memory_generation_and_commit_survives_request_cancellation() {
    tokio::task::LocalSet::new().block_on(&local_runtime(), async {
        use mosdns_native_host::{PersistFault, PersistGate};
        let path = temporary_dump();
        let clock = CacheTestClock::new(1000);
        let cache = NativeCacheAdapter::for_test(clock)
            .unwrap()
            .with_persistence(Some(path.clone()), 600);
        let q = query(&[1, b'a', 0]);
        let wire = response(&q, 60, false);
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&wire)
            .unwrap();
        cache.save().await.unwrap();
        let original = std::fs::read(&path).unwrap();
        for fault in [PersistFault::WriteTemp, PersistFault::Rename] {
            let old = cache.begin_store(&q).unwrap().unwrap();
            cache.inject_persist_fault(fault);
            assert!(cache.flush().await.is_err());
            assert!(cache.lookup(&q).unwrap().is_some());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert!(old.publish(&wire).unwrap());
        }
        let old = cache.begin_store(&q).unwrap().unwrap();
        let blocked = cache.begin_store(&q).unwrap().unwrap();
        let gate = PersistGate::new();
        cache.inject_persist_gate(gate.clone());
        let job_cache = cache.clone();
        let request = tokio::task::spawn_local(async move { job_cache.flush().await });
        wait_persist(&gate).await;
        assert!(
            cache.lookup(&q).unwrap().is_some(),
            "readers see old consistent snapshot"
        );
        assert!(
            !blocked.publish(&wire).unwrap(),
            "publication cannot race durable commit"
        );
        request.abort();
        let _ = request.await;
        gate.release();
        for _ in 0..1000 {
            if cache.snapshot().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(cache.snapshot().unwrap().is_empty());
        assert!(!old.publish(&wire).unwrap());
        let restart = NativeCacheAdapter::for_test(CacheTestClock::new(1000)).unwrap();
        restart
            .import_dump(std::fs::read(&path).unwrap())
            .await
            .unwrap();
        assert!(restart.snapshot().unwrap().is_empty());
        cache
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish(&wire)
            .unwrap();
        assert!(cache.lookup(&q).unwrap().is_some());
        std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn import_invalidates_old_publications_and_save_keeps_concurrent_mutations_dirty() {
    tokio::task::LocalSet::new().block_on(&local_runtime(), async {
        let path = temporary_dump();
        let cache = NativeCacheAdapter::for_test(CacheTestClock::new(1000))
            .unwrap()
            .with_persistence(Some(path.clone()), 600);
        let q = query(&[1, b'a', 0]);
        let q2 = query(&[1, b'b', 0]);
        let old = cache.begin_store(&q).unwrap().unwrap();
        let source = NativeCacheAdapter::for_test(CacheTestClock::new(1000)).unwrap();
        source
            .begin_store(&q)
            .unwrap()
            .unwrap()
            .publish_with_domain(&response(&q, 60, false), "imported")
            .unwrap();
        cache.import_dump(source.dump().unwrap()).await.unwrap();
        assert!(!old.publish(&response(&q, 60, false)).unwrap());
        assert_eq!(
            cache.lookup_entry(&q).unwrap().unwrap().domain_set,
            "imported"
        );
        let gate = mosdns_native_host::PersistGate::new();
        cache.inject_persist_gate(gate.clone());
        let saved = cache.clone();
        let save = tokio::task::spawn_local(async move { saved.save().await });
        wait_persist(&gate).await;
        cache
            .begin_store(&q2)
            .unwrap()
            .unwrap()
            .publish(&response(&q2, 60, false))
            .unwrap();
        gate.release();
        save.await.unwrap().unwrap();
        assert!(
            cache.is_dirty(),
            "snapshot only cleans its captured revision"
        );
        let target = NativeCacheAdapter::for_test(CacheTestClock::new(1000)).unwrap();
        target
            .import_dump(std::fs::read(&path).unwrap())
            .await
            .unwrap();
        assert!(target.lookup(&q).unwrap().is_some());
        assert!(target.lookup(&q2).unwrap().is_none());
        cache.save().await.unwrap();
        assert!(!cache.is_dirty());
        std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn real_go_v2_fixture_import_and_native_export_preserve_product_key_and_dns_wire() {
    tokio::task::LocalSet::new().block_on(&local_runtime(), async {
        let clock = CacheTestClock::new(10);
        clock.set_wall(1_700_000_010);
        let cache =
            NativeCacheAdapter::with_options_and_clock(64, 90, std::rc::Rc::new(clock)).unwrap();
        cache
            .import_dump(include_bytes!("fixtures/cache-go-v2.gz").to_vec())
            .await
            .unwrap();
        let mut q = query(&[3, b'A', b'.', 0, 0]);
        q[3] = 0x30;
        q[11] = 1;
        q.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 128, 0, 0, 0]);
        let hit = cache.lookup_entry(&q).unwrap().unwrap();
        assert_eq!(hit.domain_set, "go-domain");
        assert_eq!(
            mosdns_dns_core::observe_response_ttl(&hit.response)
                .unwrap()
                .minimal_ttl,
            50
        );
        assert_eq!(
            mosdns_dns_core::observe_answer_addresses(&hit.response).unwrap(),
            ["192.0.2.7".parse::<std::net::IpAddr>().unwrap()]
        );
        if let Ok(path) = std::env::var("MOSDNS_CACHE_INTEROP_EXPORT") {
            std::fs::write(path, cache.dump().unwrap()).unwrap();
        }
    });
}
