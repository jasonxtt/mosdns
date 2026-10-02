use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};

#[test]
fn actual_go_ecs_dump_normalizes_collisions_and_roundtrips() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let clock = CacheTestClock::new(100);
        clock.set_wall(2_000_000_000);
        let cache = NativeCacheAdapter::for_test(clock.clone())
            .unwrap()
            .with_ecs(true);
        let fixture = include_bytes!("fixtures/cache-go-ecs-v2.gz").to_vec();
        let disabled = NativeCacheAdapter::for_test(clock.clone()).unwrap();
        assert!(disabled.import_dump(fixture.clone()).await.is_err());
        assert!(disabled.snapshot().unwrap().is_empty());
        cache.import_dump(fixture).await.unwrap();
        let entries = cache.snapshot().unwrap();
        assert_eq!(entries.len(), 4);
        for (suffix, last) in [
            ("192.0.2.0/24/0", 12),
            ("203.0.0.0/16/0", 13),
            ("[2001:db8:1234::]/48/0", 20),
            ("192.0.2.0/120/0", 30),
        ] {
            let entry = entries
                .iter()
                .find(|e| e.key.ends_with(suffix.as_bytes()))
                .unwrap();
            assert_eq!(*entry.response.last().unwrap(), last);
            assert_eq!(entry.domain_set, b"go-ecs");
            assert_eq!(
                entry.wall_times,
                Some([2_000_000_000, 2_000_000_060, 2_000_000_120])
            );
        }
        let dump = cache.dump().unwrap();
        if let Ok(path) = std::env::var("TASK_ECS_NATIVE_EXPORT") {
            std::fs::write(path, &dump).unwrap();
        }
        let restart = NativeCacheAdapter::for_test(clock).unwrap().with_ecs(true);
        restart.import_dump(dump).await.unwrap();
        assert_eq!(restart.snapshot().unwrap().len(), 4);
        assert!(
            restart
                .snapshot()
                .unwrap()
                .iter()
                .all(|e| e.domain_set == b"go-ecs")
        );
    });
}
