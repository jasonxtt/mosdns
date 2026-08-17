//! `MosDNS` single Rust runtime static library.

#![allow(clippy::pedantic)]
//!
//! This is the only `staticlib` in the workspace. It exposes cache and matcher
//! ABI symbols as `extern "C"` functions through a thin delegation layer; the
//! implementation lives in the internal `cache-core` and `matcher-core`
//! libraries.

mod matcher;
mod query;

use mosdns_cache_core::{
    BorrowedSlice, CacheConfig, LookupIntoResult, LookupResult, OwnedBuffer, Status, WritableSlice,
};

pub use matcher::*;
pub use query::*;

// --- ABI version / capabilities ---

#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn cache_abi_version() -> u32 {
    mosdns_cache_core::ABI_VERSION
}

#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn cache_abi_capabilities() -> u64 {
    mosdns_cache_core::CAPABILITY_LIFECYCLE
        | mosdns_cache_core::CAPABILITY_CACHE
        | mosdns_cache_core::CAPABILITY_LOOKUP_INTO
        | matcher::CAPABILITY_MATCHER
        | matcher::CAPABILITY_VALUED_MATCHER
        | query::CAPABILITY_QUERY_SNAPSHOT
        | query::CAPABILITY_QUERY_INSPECT
}

// --- Lifecycle ---

/// Creates a cache handle.
///
/// # Safety
///
/// `config` must point to a readable [`CacheConfig`] and `out_handle` must
/// point to writable `u64` storage for the duration of this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_create(config: *const CacheConfig, out_handle: *mut u64) -> Status {
    unsafe { mosdns_cache_core::cache_create(config, out_handle) }
}

#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn cache_close(handle: u64) -> Status {
    mosdns_cache_core::cache_close(handle)
}

/// Returns the number of entries in a cache handle.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_len(handle: u64, out_len: *mut u64) -> Status {
    unsafe { mosdns_cache_core::cache_len(handle, out_len) }
}

#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn cache_flush(handle: u64) -> Status {
    mosdns_cache_core::cache_flush(handle)
}

// --- Store / Lookup ---

/// Stores an entry under a canonical `MosDNS` cache key.
///
/// # Safety
///
/// Every non-empty borrowed slice must remain readable for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_store(
    handle: u64,
    key: BorrowedSlice,
    response: BorrowedSlice,
    domain_set: BorrowedSlice,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
) -> Status {
    unsafe {
        mosdns_cache_core::cache_store(
            handle,
            key,
            response,
            domain_set,
            stored_at_unix,
            message_expires_at_unix,
            cache_expires_at_unix,
        )
    }
}

/// Looks up a canonical `MosDNS` cache key.
///
/// # Safety
///
/// `key` must satisfy [`BorrowedSlice`]'s pointer contract and `out_result`
/// must point to writable [`LookupResult`] storage for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_lookup(
    handle: u64,
    key: BorrowedSlice,
    now_unix: i64,
    out_result: *mut LookupResult,
) -> Status {
    unsafe { mosdns_cache_core::cache_lookup(handle, key, now_unix, out_result) }
}

/// Writes hit payloads into caller-owned buffers.
///
/// # Safety
///
/// Borrowed input and writable output slices must remain valid for this call,
/// and `out_result` must point to writable [`LookupIntoResult`] storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_lookup_into(
    handle: u64,
    key: BorrowedSlice,
    now_unix: i64,
    response_out: WritableSlice,
    domain_set_out: WritableSlice,
    out_result: *mut LookupIntoResult,
) -> Status {
    unsafe {
        mosdns_cache_core::cache_lookup_into(
            handle,
            key,
            now_unix,
            response_out,
            domain_set_out,
            out_result,
        )
    }
}

// --- Buffer release ---

/// Releases a buffer previously returned by this library.
///
/// # Safety
///
/// A non-empty `buffer` must be the original, not-yet-released value returned
/// by this library. It must not be copied, modified, or released twice.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cache_buffer_release(buffer: OwnedBuffer) -> Status {
    unsafe { mosdns_cache_core::cache_buffer_release(buffer) }
}
