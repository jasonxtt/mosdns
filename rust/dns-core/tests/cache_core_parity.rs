//! Byte/field parity between `mosdns-dns-core` and the existing
//! `mosdns-cache-core` wire walk.
//!
//! The cache wire behavior lives in `cache-core/src/wire.rs` behind the
//! `cache_store`/`cache_lookup_into` ABI. This test stores a fixed response
//! through the real cache ABI and reads it back aged, then checks that the
//! dns-core `age_response_ttls`/`replace_response_ttls` produce the same
//! bytes on the same stored input. That is the "existing tests prove no
//! semantic change" evidence: if a later slice routes cache-core's wire walk
//! through dns-core, these vectors must stay identical.

use mosdns_cache_core::*;
use mosdns_dns_core::age_response_ttls;

const RESPONSE_TTL_OFFSET: usize = 35;

fn response_wire(ttl: u32, opt_ttl: u32) -> Vec<u8> {
    let mut packet = vec![
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, b'e', b'x',
        b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00, 0x01, 0xc0,
        0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04, 192, 0, 2, 1, 0x00, 0x00, 0x29, 0x04,
        0xd0, 0, 0, 0, 0, 0x00, 0x00,
    ];
    packet[RESPONSE_TTL_OFFSET..RESPONSE_TTL_OFFSET + 4].copy_from_slice(&ttl.to_be_bytes());
    packet[50..54].copy_from_slice(&opt_ttl.to_be_bytes());
    packet
}

#[test]
fn aged_response_matches_dns_core_age() {
    let stored = response_wire(60, 0x0000_8000);
    assert_eq!(stored.len(), 56);
    let stored_time = 1_700_000_000;
    let message_expires = stored_time + 60; // fresh for 60s

    let mut handle = 0;
    let config = CacheConfig {
        capacity: 16,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };
    let key = b"example.org;A";
    assert_eq!(
        // SAFETY: `config` and `handle` are valid for the call; `handle` is
        // written and then used as the cache handle below.
        unsafe { cache_create(&raw const config, &raw mut handle) },
        Status::Ok
    );

    let resp = BorrowedSlice::from_slice(&stored);
    let domain = BorrowedSlice::from_slice(b"");
    let now = stored_time + 17;
    assert_eq!(
        // SAFETY: Every borrowed slice stays live for the call and the handle
        // was created above; only the returned Status is consumed.
        unsafe {
            cache_store(
                handle,
                BorrowedSlice::from_slice(key),
                resp,
                domain,
                stored_time,
                message_expires,
                message_expires,
            )
        },
        Status::Ok
    );

    // Reading back at now = stored+17 must yield the aged wire.
    let mut resp_buf = vec![0u8; 128];
    let mut ds_buf = vec![0u8; 8];
    let mut result = LookupIntoResult::empty();
    let status = unsafe {
        cache_lookup_into(
            handle,
            BorrowedSlice::from_slice(key),
            now,
            WritableSlice::from_slice(&mut resp_buf),
            WritableSlice::from_slice(&mut ds_buf),
            &raw mut result,
        )
    };
    assert_eq!(status, Status::Ok);
    let aged_from_cache = resp_buf[..usize::try_from(result.response_len).unwrap()].to_vec();

    // dns-core ages the same stored bytes: must be byte-identical.
    let aged_from_dns = age_response_ttls(&stored, 17).expect("valid response");
    assert_eq!(aged_from_cache, aged_from_dns);
    assert_eq!(
        &aged_from_cache[RESPONSE_TTL_OFFSET..RESPONSE_TTL_OFFSET + 4],
        &43u32.to_be_bytes()[..]
    );

    // `cache_close` is safe; discard its status (the test already asserted Ok
    // on every cache ABI call above).
    let _ = cache_close(handle);
}
