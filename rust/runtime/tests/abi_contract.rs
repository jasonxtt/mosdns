//! ABI contract tests against the single `mosdns-runtime` static library.
//!
//! These exercise the same `extern "C"` symbols that the Go bridge calls, so
//! they cover the full C-ABI boundary through the delegation layer.
//! The cache-core implementation tests (panic catch, etc.) remain in that
//! crate's `#[cfg(test)]` module.

use mosdns_runtime::{
    cache_abi_capabilities, domain_matcher_close, domain_matcher_create, domain_matcher_len,
    domain_matcher_match, ip_matcher_close, ip_matcher_create, ip_matcher_len, ip_matcher_match,
};

use mosdns_cache_core::{
    ABI_VERSION, BorrowedSlice, CAPABILITY_CACHE, CAPABILITY_LIFECYCLE, CacheConfig,
    LookupIntoResult, LookupResult, LookupState, OwnedBuffer, Status, WritableSlice,
    cache_abi_version, cache_buffer_release, cache_close, cache_create, cache_flush, cache_len,
    cache_lookup, cache_lookup_into, cache_store,
};
use std::sync::Arc;
use std::thread;

const RESPONSE_TTL_OFFSET: usize = 25;

fn response_wire(ttl: u32) -> Vec<u8> {
    let mut response = vec![
        0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0, 1, b'x', 0, 0, 1, 0, 1, 0xc0, 0x0c, 0, 1,
        0, 1, 0, 0, 0, 0, 0, 4, 192, 0, 2, 1,
    ];
    response[RESPONSE_TTL_OFFSET..RESPONSE_TTL_OFFSET + 4].copy_from_slice(&ttl.to_be_bytes());
    response
}

fn response_ttl(response: &[u8]) -> u32 {
    u32::from_be_bytes(
        response[RESPONSE_TTL_OFFSET..RESPONSE_TTL_OFFSET + 4]
            .try_into()
            .expect("four-byte TTL"),
    )
}

// --- Matcher ABI tests ---

#[test]
fn matcher_domain_create_match_close_roundtrip() {
    let rules = b"full:exact.example\ndomain:example.com\n";
    let key = BorrowedSlice::from_slice(rules);
    let mut handle = 0;
    assert_eq!(
        unsafe { domain_matcher_create(key, 0, &raw mut handle) },
        Status::Ok
    );
    assert_ne!(handle, 0);

    // Match an exact domain.
    let d = b"exact.example";
    let mut matched = false;
    assert_eq!(
        unsafe { domain_matcher_match(handle, BorrowedSlice::from_slice(d), &raw mut matched,) },
        Status::Ok
    );
    assert!(matched);

    // Match a suffix domain.
    let d2 = b"sub.example.com";
    let mut matched2 = false;
    assert_eq!(
        unsafe { domain_matcher_match(handle, BorrowedSlice::from_slice(d2), &raw mut matched2,) },
        Status::Ok
    );
    assert!(matched2);

    // Non-match.
    let d3 = b"unrelated.org";
    let mut matched3 = false;
    assert_eq!(
        unsafe { domain_matcher_match(handle, BorrowedSlice::from_slice(d3), &raw mut matched3,) },
        Status::Ok
    );
    assert!(!matched3);

    // Len.
    let mut len = 0;
    assert_eq!(
        unsafe { domain_matcher_len(handle, &raw mut len) },
        Status::Ok
    );
    assert_eq!(len, 2);

    // Close.
    assert_eq!(domain_matcher_close(handle), Status::Ok);
    assert_eq!(domain_matcher_close(handle), Status::Closed);
}

#[test]
fn matcher_domain_mixed_regexp_and_keyword_rules_are_compiled_once() {
    let rules = b"regexp:^ads\\.example$
keyword:telemetry
";
    let mut handle = 0;
    assert_eq!(
        unsafe { domain_matcher_create(BorrowedSlice::from_slice(rules), 0, &raw mut handle) },
        Status::Ok
    );

    let mut len = 0;
    assert_eq!(
        unsafe { domain_matcher_len(handle, &raw mut len) },
        Status::Ok
    );
    assert_eq!(len, 2, "one regexp and one keyword entry must be published");

    let mut regexp_match = false;
    assert_eq!(
        unsafe {
            domain_matcher_match(
                handle,
                BorrowedSlice::from_slice(b"ads.example"),
                &raw mut regexp_match,
            )
        },
        Status::Ok
    );
    assert!(regexp_match);

    let mut keyword_match = false;
    assert_eq!(
        unsafe {
            domain_matcher_match(
                handle,
                BorrowedSlice::from_slice(b"host.telemetry.example"),
                &raw mut keyword_match,
            )
        },
        Status::Ok
    );
    assert!(keyword_match);
    assert_eq!(domain_matcher_close(handle), Status::Ok);
}

#[test]
fn matcher_domain_invalid_mixed_batch_publishes_no_partial_handle() {
    let rules = b"regexp:^valid\\.example$
keyword:telemetry
regexp:[
";
    let mut handle = 0xdead_beef;
    assert_eq!(
        unsafe { domain_matcher_create(BorrowedSlice::from_slice(rules), 0, &raw mut handle) },
        Status::InvalidArgument
    );
    assert_eq!(handle, 0xdead_beef);

    let mut matched = false;
    assert_eq!(
        unsafe {
            domain_matcher_match(
                0xdead_beef,
                BorrowedSlice::from_slice(b"valid.example"),
                &raw mut matched,
            )
        },
        Status::Closed
    );
}

#[test]
fn matcher_empty_batches_create_zero_length_handles() {
    let mut domain_handle = 0;
    assert_eq!(
        unsafe { domain_matcher_create(BorrowedSlice::from_slice(&[]), 0, &raw mut domain_handle) },
        Status::Ok
    );
    let mut domain_len = u64::MAX;
    assert_eq!(
        unsafe { domain_matcher_len(domain_handle, &raw mut domain_len) },
        Status::Ok
    );
    assert_eq!(domain_len, 0);
    let mut domain_matched = true;
    assert_eq!(
        unsafe {
            domain_matcher_match(
                domain_handle,
                BorrowedSlice::from_slice(b"empty.example"),
                &raw mut domain_matched,
            )
        },
        Status::Ok
    );
    assert!(!domain_matched);
    assert_eq!(domain_matcher_close(domain_handle), Status::Ok);
    assert_eq!(domain_matcher_close(domain_handle), Status::Closed);

    let mut ip_handle = 0;
    assert_eq!(
        unsafe { ip_matcher_create(BorrowedSlice::from_slice(&[]), &raw mut ip_handle) },
        Status::Ok
    );
    let mut ip_len = u64::MAX;
    assert_eq!(
        unsafe { ip_matcher_len(ip_handle, &raw mut ip_len) },
        Status::Ok
    );
    assert_eq!(ip_len, 0);
    assert_eq!(ip_matcher_close(ip_handle), Status::Ok);
    assert_eq!(ip_matcher_close(ip_handle), Status::Closed);
}

#[test]
fn matcher_ip_create_match_close_roundtrip() {
    let data = b"10.0.0.0/8\n192.168.1.1\n";
    let key = BorrowedSlice::from_slice(data);
    let mut handle = 0;
    assert_eq!(
        unsafe { ip_matcher_create(key, &raw mut handle) },
        Status::Ok
    );
    assert_ne!(handle, 0);

    // Match.
    let a1 = b"10.1.2.3";
    let mut matched = false;
    assert_eq!(
        unsafe { ip_matcher_match(handle, BorrowedSlice::from_slice(a1), &raw mut matched) },
        Status::Ok
    );
    assert!(matched);

    // Non-match.
    let a2 = b"11.0.0.1";
    let mut matched2 = false;
    assert_eq!(
        unsafe { ip_matcher_match(handle, BorrowedSlice::from_slice(a2), &raw mut matched2) },
        Status::Ok
    );
    assert!(!matched2);

    // Len.
    let mut len = 0;
    assert_eq!(unsafe { ip_matcher_len(handle, &raw mut len) }, Status::Ok);
    assert_eq!(len, 2);

    // Close.
    assert_eq!(ip_matcher_close(handle), Status::Ok);
    assert_eq!(ip_matcher_close(handle), Status::Closed);
}

#[test]
fn matcher_domain_invalid_arguments() {
    assert_eq!(
        unsafe { domain_matcher_create(BorrowedSlice::from_slice(b""), 0, std::ptr::null_mut(),) },
        Status::InvalidArgument
    );
    assert_eq!(domain_matcher_close(0), Status::InvalidArgument);
}

#[test]
fn matcher_ip_invalid_arguments() {
    assert_eq!(
        unsafe { ip_matcher_create(BorrowedSlice::from_slice(b""), std::ptr::null_mut(),) },
        Status::InvalidArgument
    );
}

#[test]
fn matcher_capability_is_exposed() {
    let caps = cache_abi_capabilities();
    assert_ne!(caps & 1 << 3, 0, "MATCHER capability bit must be set");
}

#[test]
fn matcher_namespaces_reject_cache_handles() {
    let config = CacheConfig {
        capacity: 1,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };
    let mut cache_handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut cache_handle) },
        Status::Ok
    );

    let mut matched = false;
    assert_eq!(
        unsafe {
            domain_matcher_match(
                cache_handle,
                BorrowedSlice::from_slice(b"example.com"),
                &raw mut matched,
            )
        },
        Status::Closed
    );
    assert_eq!(
        unsafe {
            ip_matcher_match(
                cache_handle,
                BorrowedSlice::from_slice(b"192.0.2.1"),
                &raw mut matched,
            )
        },
        Status::Closed
    );

    let mut domain_handle = 0;
    assert_eq!(
        unsafe {
            domain_matcher_create(
                BorrowedSlice::from_slice(b"full:example.com"),
                0,
                &raw mut domain_handle,
            )
        },
        Status::Ok
    );
    let mut ip_handle = 0;
    assert_eq!(
        unsafe {
            ip_matcher_create(
                BorrowedSlice::from_slice(b"192.0.2.0/24"),
                &raw mut ip_handle,
            )
        },
        Status::Ok
    );

    let mut cross_match = false;
    assert_eq!(
        unsafe {
            ip_matcher_match(
                domain_handle,
                BorrowedSlice::from_slice(b"192.0.2.1"),
                &raw mut cross_match,
            )
        },
        Status::Closed
    );
    assert_eq!(
        unsafe {
            domain_matcher_match(
                ip_handle,
                BorrowedSlice::from_slice(b"example.com"),
                &raw mut cross_match,
            )
        },
        Status::Closed
    );

    let mut wrong_len = 0;
    assert_eq!(
        unsafe { cache_len(domain_handle, &raw mut wrong_len) },
        Status::Closed
    );
    assert_eq!(
        unsafe { cache_len(ip_handle, &raw mut wrong_len) },
        Status::Closed
    );
    assert_eq!(cache_flush(domain_handle), Status::Closed);
    assert_eq!(cache_flush(ip_handle), Status::Closed);

    assert_eq!(domain_matcher_close(domain_handle), Status::Ok);
    assert_eq!(domain_matcher_close(domain_handle), Status::Closed);
    assert_eq!(ip_matcher_close(ip_handle), Status::Ok);
    assert_eq!(ip_matcher_close(ip_handle), Status::Closed);

    assert_eq!(domain_matcher_close(cache_handle), Status::Closed);
    assert_eq!(ip_matcher_close(cache_handle), Status::Closed);
    assert_eq!(cache_close(cache_handle), Status::Ok);
}

fn lookup(handle: u64, key: &[u8], now: i64) -> LookupResult {
    let mut result = LookupResult::empty();
    let key = BorrowedSlice::from_slice(key);
    let status = unsafe { cache_lookup(handle, key, now, &raw mut result) };
    assert_eq!(status, result.status);
    result
}

fn release_lookup(result: LookupResult) {
    assert_eq!(unsafe { cache_buffer_release(result.response) }, Status::Ok);
    assert_eq!(
        unsafe { cache_buffer_release(result.domain_set) },
        Status::Ok
    );
}

#[test]
fn lifecycle_is_versioned_and_idempotent() {
    assert_eq!(cache_abi_version(), ABI_VERSION);
    assert_ne!(cache_abi_capabilities() & CAPABILITY_LIFECYCLE, 0);

    let config = CacheConfig {
        capacity: 16,
        lazy_cache_ttl_secs: 60,
        flags: 0,
    };
    let mut handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );
    assert_ne!(handle, 0);

    let mut len = u64::MAX;
    assert_eq!(unsafe { cache_len(handle, &raw mut len) }, Status::Ok);
    assert_eq!(len, 0);
    assert_eq!(cache_close(handle), Status::Ok);
    assert_eq!(cache_close(handle), Status::Closed);
    assert_eq!(unsafe { cache_len(handle, &raw mut len) }, Status::Closed);
}

#[test]
fn concurrent_cache_classifies_fresh_lazy_and_expired_entries() {
    assert_ne!(cache_abi_capabilities() & CAPABILITY_CACHE, 0);
    let config = CacheConfig {
        capacity: 16,
        lazy_cache_ttl_secs: 300,
        flags: 0,
    };
    let mut handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );

    let key = b"exact-mosdns-key";
    let response = response_wire(60);
    let domain_set = b"contract-set";
    assert_eq!(
        unsafe {
            cache_store(
                handle,
                BorrowedSlice::from_slice(key),
                BorrowedSlice::from_slice(&response),
                BorrowedSlice::from_slice(domain_set),
                100,
                160,
                400,
            )
        },
        Status::Ok
    );

    let fresh = lookup(handle, key, 120);
    assert_eq!(fresh.status, Status::Ok);
    assert_eq!(fresh.state, LookupState::Fresh);
    assert_eq!(fresh.stored_at_unix, 100);
    assert_eq!(fresh.message_expires_at_unix, 160);
    assert_eq!(response_ttl(unsafe { fresh.response.as_slice() }), 40);
    assert_eq!(unsafe { fresh.domain_set.as_slice() }, domain_set);
    release_lookup(fresh);

    let lazy = lookup(handle, key, 200);
    assert_eq!(lazy.status, Status::Ok);
    assert_eq!(lazy.state, LookupState::Lazy);
    assert_eq!(response_ttl(unsafe { lazy.response.as_slice() }), 5);
    release_lookup(lazy);

    let expired = lookup(handle, key, 401);
    assert_eq!(expired.status, Status::Ok);
    assert_eq!(expired.state, LookupState::Miss);
    release_lookup(expired);

    let mut len = u64::MAX;
    assert_eq!(unsafe { cache_len(handle, &raw mut len) }, Status::Ok);
    assert_eq!(len, 0);
    assert_eq!(cache_flush(handle), Status::Ok);
    assert_eq!(cache_close(handle), Status::Ok);
}

#[test]
fn concurrent_stores_respect_capacity_without_a_global_cache_lock() {
    let config = CacheConfig {
        capacity: 32,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };
    let mut handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );
    let handle = Arc::new(handle);
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let handle = Arc::clone(&handle);
            thread::spawn(move || {
                for item in 0_u32..100 {
                    let key = format!("worker-{worker}-item-{item}").into_bytes();
                    let response = response_wire(item + 1);
                    assert_eq!(
                        unsafe {
                            cache_store(
                                *handle,
                                BorrowedSlice::from_slice(&key),
                                BorrowedSlice::from_slice(&response),
                                BorrowedSlice::from_slice(&[]),
                                100,
                                200,
                                200,
                            )
                        },
                        Status::Ok
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("cache worker panicked");
    }

    let mut len = 0;
    assert_eq!(unsafe { cache_len(*handle, &raw mut len) }, Status::Ok);
    assert!(
        len > 0 && len <= config.capacity,
        "unexpected cache len {len}"
    );
    assert_eq!(cache_close(*handle), Status::Ok);
}

#[test]
fn malformed_dns_wire_is_rejected_at_store_boundary() {
    let config = CacheConfig {
        capacity: 1,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };
    let mut handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );
    let truncated = [0_u8; 11];
    assert_eq!(
        unsafe {
            cache_store(
                handle,
                BorrowedSlice::from_slice(b"key"),
                BorrowedSlice::from_slice(&truncated),
                BorrowedSlice::from_slice(&[]),
                1,
                2,
                2,
            )
        },
        Status::InvalidArgument
    );
    assert_eq!(cache_close(handle), Status::Ok);
}

#[test]
fn lookup_into_reports_sizes_then_writes_caller_owned_buffers() {
    let config = CacheConfig {
        capacity: 1,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };
    let mut handle = 0;
    assert_eq!(
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );
    let response = response_wire(60);
    assert_eq!(
        unsafe {
            cache_store(
                handle,
                BorrowedSlice::from_slice(b"lookup-into"),
                BorrowedSlice::from_slice(&response),
                BorrowedSlice::from_slice(b"domain-set"),
                100,
                160,
                160,
            )
        },
        Status::Ok
    );

    let mut too_small = LookupIntoResult::empty();
    let mut tiny = [0_u8; 1];
    assert_eq!(
        unsafe {
            cache_lookup_into(
                handle,
                BorrowedSlice::from_slice(b"lookup-into"),
                110,
                WritableSlice::from_slice(&mut tiny),
                WritableSlice::empty(),
                &raw mut too_small,
            )
        },
        Status::BufferTooSmall
    );
    assert_eq!(too_small.response_len, response.len() as u64);
    assert_eq!(too_small.domain_set_len, 10);

    let response_len = usize::try_from(too_small.response_len).expect("response length fits usize");
    let domain_set_len =
        usize::try_from(too_small.domain_set_len).expect("domain-set length fits usize");
    let mut response_out = vec![0_u8; response_len];
    let mut domain_out = vec![0_u8; domain_set_len];
    let mut result = LookupIntoResult::empty();
    assert_eq!(
        unsafe {
            cache_lookup_into(
                handle,
                BorrowedSlice::from_slice(b"lookup-into"),
                110,
                WritableSlice::from_slice(&mut response_out),
                WritableSlice::from_slice(&mut domain_out),
                &raw mut result,
            )
        },
        Status::Ok
    );
    assert_eq!(result.state, LookupState::Fresh);
    assert_eq!(response_ttl(&response_out), 50);
    assert_eq!(&domain_out, b"domain-set");
    assert_eq!(cache_close(handle), Status::Ok);
}

#[test]
fn owned_buffer_uses_length_only_release_contract() {
    let payload = b"capacity must not be guessed";
    let buffer = OwnedBuffer::copy_from_slice(payload);
    assert!(!buffer.ptr.is_null());
    assert_eq!(buffer.len, payload.len() as u64);
    assert_eq!(unsafe { buffer.as_slice() }, payload);
    assert_eq!(unsafe { cache_buffer_release(buffer) }, Status::Ok);
    assert_eq!(
        unsafe { cache_buffer_release(OwnedBuffer::empty()) },
        Status::Ok
    );
}

#[test]
fn owned_buffer_can_take_vec_without_a_second_payload_copy() {
    let payload = b"ttl-patched-wire".to_vec();
    let buffer = OwnedBuffer::from_vec(payload);
    assert_eq!(buffer.len, 16);
    assert_eq!(unsafe { buffer.as_slice() }, b"ttl-patched-wire");
    assert_eq!(unsafe { cache_buffer_release(buffer) }, Status::Ok);
}

#[test]
fn invalid_arguments_return_status_instead_of_panicking() {
    assert_eq!(
        unsafe { cache_create(std::ptr::null(), std::ptr::null_mut()) },
        Status::InvalidArgument
    );
    assert_eq!(cache_close(0), Status::InvalidArgument);
    assert_eq!(
        unsafe { cache_len(0, std::ptr::null_mut()) },
        Status::InvalidArgument
    );
}

#[test]
fn checked_in_header_matches_abi_constants_and_entrypoints() {
    let header = include_str!("../include/mosdns_cache_core.h");
    for expected in [
        "#define MOSDNS_CACHE_ABI_VERSION 1u",
        "MOSDNS_CACHE_CAPABILITY_LIFECYCLE",
        "MOSDNS_CACHE_CAPABILITY_LOOKUP_INTO",
        "MOSDNS_CACHE_CAPABILITY_VALUED_MATCHER",
        "#define MOSDNS_VALUED_RULE_BATCH_VERSION 1u",
        "#define MOSDNS_VALUED_RESULT_VERSION 1u",
        "uint32_t cache_abi_version(void);",
        "uint64_t cache_abi_capabilities(void);",
        "MosdnsCacheStatus cache_create(",
        "MosdnsCacheStatus cache_close(uint64_t handle);",
        "MosdnsCacheStatus cache_len(uint64_t handle, uint64_t *out_len);",
        "MosdnsCacheStatus cache_store(",
        "MosdnsCacheStatus cache_lookup(",
        "MosdnsCacheStatus cache_lookup_into(",
        "MosdnsCacheStatus cache_flush(uint64_t handle);",
        "MosdnsCacheStatus cache_buffer_release(MosdnsCacheOwnedBuffer buffer);",
        "MosdnsCacheStatus valued_domain_matcher_create(",
        "MosdnsCacheStatus valued_domain_matcher_match(",
        "MosdnsCacheStatus valued_domain_matcher_len(uint64_t handle, uint64_t *out_len);",
        "MosdnsCacheStatus valued_domain_matcher_close(uint64_t handle);",
    ] {
        assert!(header.contains(expected), "header is missing {expected:?}");
    }
}
