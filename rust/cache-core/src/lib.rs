//! Experimental `MosDNS` Rust cache core.
//!
//! The C ABI uses integer handles backed by a process-local registry. This
//! makes duplicate and concurrent close deterministic without dereferencing a
//! pointer after its allocation has been freed.
//!
//! This crate is an implementation `rlib` linked by `mosdns-runtime` (the sole
//! `staticlib`). All `extern "C"` symbols are exported by `mosdns-runtime`.

#![allow(clippy::missing_safety_doc, clippy::missing_errors_doc)]

use bytes::Bytes;
use dashmap::DashMap;
use moka::sync::Cache;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

mod wire;

pub const ABI_VERSION: u32 = 1;
pub const CAPABILITY_LIFECYCLE: u64 = 1 << 0;
pub const CAPABILITY_CACHE: u64 = 1 << 1;
pub const CAPABILITY_LOOKUP_INTO: u64 = 1 << 2;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static HANDLES: OnceLock<DashMap<u64, CacheState>> = OnceLock::new();

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    Closed = 2,
    Panic = 3,
    Internal = 4,
    BufferTooSmall = 5,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CacheConfig {
    pub capacity: u64,
    pub lazy_cache_ttl_secs: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug)]
pub struct OwnedBuffer {
    pub ptr: *mut u8,
    pub len: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BorrowedSlice {
    pub ptr: *const u8,
    pub len: u64,
}

impl BorrowedSlice {
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self {
                ptr: std::ptr::null(),
                len: 0,
            };
        }
        let Ok(len) = u64::try_from(bytes.len()) else {
            return Self {
                ptr: std::ptr::null(),
                len: u64::MAX,
            };
        };
        Self {
            ptr: bytes.as_ptr(),
            len,
        }
    }

    /// Views caller-owned bytes for the duration of an ABI call.
    ///
    /// # Safety
    ///
    /// For non-empty input, `ptr` must be readable for exactly `len` bytes and
    /// remain live for the returned lifetime.
    pub unsafe fn as_slice(&self) -> Result<&[u8], Status> {
        if self.ptr.is_null() {
            return if self.len == 0 {
                Ok(&[])
            } else {
                Err(Status::InvalidArgument)
            };
        }
        if self.len == 0 {
            return Err(Status::InvalidArgument);
        }
        let len = usize::try_from(self.len).map_err(|_| Status::InvalidArgument)?;
        // SAFETY: The caller upholds the pointer/length contract.
        Ok(unsafe { std::slice::from_raw_parts(self.ptr, len) })
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct WritableSlice {
    pub ptr: *mut u8,
    pub len: u64,
}

impl WritableSlice {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
        }
    }

    #[must_use]
    pub fn from_slice(bytes: &mut [u8]) -> Self {
        if bytes.is_empty() {
            return Self::empty();
        }
        let Ok(len) = u64::try_from(bytes.len()) else {
            return Self {
                ptr: std::ptr::null_mut(),
                len: u64::MAX,
            };
        };
        Self {
            ptr: bytes.as_mut_ptr(),
            len,
        }
    }

    pub unsafe fn as_mut_slice(&mut self) -> Result<&mut [u8], Status> {
        if self.ptr.is_null() {
            return if self.len == 0 {
                Ok(&mut [])
            } else {
                Err(Status::InvalidArgument)
            };
        }
        if self.len == 0 {
            return Err(Status::InvalidArgument);
        }
        let len = usize::try_from(self.len).map_err(|_| Status::InvalidArgument)?;
        // SAFETY: The caller guarantees exclusive writable access for this call.
        Ok(unsafe { std::slice::from_raw_parts_mut(self.ptr, len) })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupState {
    Miss = 0,
    Fresh = 1,
    Lazy = 2,
}

#[repr(C)]
#[derive(Debug)]
pub struct LookupResult {
    pub status: Status,
    pub state: LookupState,
    pub stored_at_unix: i64,
    pub message_expires_at_unix: i64,
    pub response: OwnedBuffer,
    pub domain_set: OwnedBuffer,
}

impl LookupResult {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            status: Status::Internal,
            state: LookupState::Miss,
            stored_at_unix: 0,
            message_expires_at_unix: 0,
            response: OwnedBuffer::empty(),
            domain_set: OwnedBuffer::empty(),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct LookupIntoResult {
    pub status: Status,
    pub state: LookupState,
    pub stored_at_unix: i64,
    pub message_expires_at_unix: i64,
    pub response_len: u64,
    pub domain_set_len: u64,
}

impl LookupIntoResult {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            status: Status::Internal,
            state: LookupState::Miss,
            stored_at_unix: 0,
            message_expires_at_unix: 0,
            response_len: 0,
            domain_set_len: 0,
        }
    }
}

impl OwnedBuffer {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
        }
    }

    #[must_use]
    pub fn copy_from_slice(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::empty();
        }
        Self::from_vec(bytes.to_vec())
    }

    #[must_use]
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return Self::empty();
        }
        let Ok(len) = u64::try_from(bytes.len()) else {
            return Self::empty();
        };
        let ptr = Box::into_raw(bytes.into_boxed_slice()).cast::<u8>();
        Self { ptr, len }
    }

    /// Views the bytes without taking ownership.
    ///
    /// # Safety
    ///
    /// This buffer must still own a live allocation created by
    /// [`Self::copy_from_slice`], and no mutable reference to that allocation
    /// may exist for the returned lifetime.
    #[must_use]
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            return &[];
        }
        let Ok(len) = usize::try_from(self.len) else {
            return &[];
        };
        // SAFETY: The caller upholds the allocation and aliasing contract.
        unsafe { std::slice::from_raw_parts(self.ptr, len) }
    }
}

struct CacheState {
    config: CacheConfig,
    entries: Cache<Bytes, Arc<CacheEntry>>,
}

#[derive(Debug)]
struct CacheEntry {
    response: Bytes,
    domain_set: Bytes,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
}

struct LookupData {
    state: LookupState,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    response: Vec<u8>,
    domain_set: Bytes,
}

fn lookup_data(
    state: &CacheState,
    key: &[u8],
    now_unix: i64,
) -> Result<Option<LookupData>, Status> {
    let Some(entry) = state.entries.get(key) else {
        return Ok(None);
    };
    if now_unix >= entry.cache_expires_at_unix {
        state.entries.invalidate(key);
        state.entries.run_pending_tasks();
        return Ok(None);
    }

    let (lookup_state, response) = if now_unix < entry.message_expires_at_unix {
        let elapsed = now_unix
            .saturating_sub(entry.stored_at_unix)
            .clamp(0, i64::from(u32::MAX));
        let elapsed = u32::try_from(elapsed).unwrap_or(u32::MAX);
        (LookupState::Fresh, wire::age_ttls(&entry.response, elapsed))
    } else if state.config.lazy_cache_ttl_secs > 0 {
        (LookupState::Lazy, wire::set_ttls(&entry.response, 5))
    } else {
        return Ok(None);
    };
    let response = response.map_err(|_| {
        state.entries.invalidate(key);
        Status::Internal
    })?;
    Ok(Some(LookupData {
        state: lookup_state,
        stored_at_unix: entry.stored_at_unix,
        message_expires_at_unix: entry.message_expires_at_unix,
        response,
        domain_set: entry.domain_set.clone(),
    }))
}

fn handles() -> &'static DashMap<u64, CacheState> {
    HANDLES.get_or_init(DashMap::new)
}

fn boundary(operation: impl FnOnce() -> Status) -> Status {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(Status::Panic)
}

// -- Public API (Rust ABI, called by mosdns-runtime's extern "C" wrappers) --

#[must_use]
pub fn cache_abi_version() -> u32 {
    ABI_VERSION
}

#[must_use]
pub fn cache_abi_capabilities() -> u64 {
    CAPABILITY_LIFECYCLE | CAPABILITY_CACHE | CAPABILITY_LOOKUP_INTO
}

/// Creates a cache handle.
///
/// # Safety
///
/// `config` must point to a readable [`CacheConfig`] and `out_handle` must
/// point to writable `u64` storage for the duration of this call.
#[must_use]
pub unsafe fn cache_create(config: *const CacheConfig, out_handle: *mut u64) -> Status {
    boundary(|| {
        if config.is_null() || out_handle.is_null() {
            return Status::InvalidArgument;
        }
        let config = unsafe { *config };
        if config.capacity == 0 {
            return Status::InvalidArgument;
        }

        let handle = loop {
            let candidate = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            if candidate != 0 && !handles().contains_key(&candidate) {
                break candidate;
            }
        };
        handles().insert(
            handle,
            CacheState {
                config,
                entries: Cache::builder().max_capacity(config.capacity).build(),
            },
        );
        unsafe { out_handle.write(handle) };
        Status::Ok
    })
}

#[must_use]
pub fn cache_close(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        if handles().remove(&handle).is_some() {
            Status::Ok
        } else {
            Status::Closed
        }
    })
}

/// Releases a buffer previously returned by this library.
///
/// # Safety
///
/// A non-empty `buffer` must be the original, not-yet-released value returned
/// by this library. It must not be copied, modified, or released twice.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub unsafe fn cache_buffer_release(buffer: OwnedBuffer) -> Status {
    boundary(|| {
        if buffer.ptr.is_null() {
            return if buffer.len == 0 {
                Status::Ok
            } else {
                Status::InvalidArgument
            };
        }
        if buffer.len == 0 {
            return Status::InvalidArgument;
        }
        let Ok(len) = usize::try_from(buffer.len) else {
            return Status::InvalidArgument;
        };
        let slice = std::ptr::slice_from_raw_parts_mut(buffer.ptr, len);
        drop(unsafe { Box::from_raw(slice) });
        Status::Ok
    })
}

/// Returns the number of entries currently owned by a cache handle.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for the duration of this call.
pub unsafe fn cache_len(handle: u64, out_len: *mut u64) -> Status {
    boundary(|| {
        if handle == 0 || out_len.is_null() {
            return Status::InvalidArgument;
        }
        let Some(state) = handles().get(&handle) else {
            return Status::Closed;
        };
        state.entries.run_pending_tasks();
        let len = state.entries.entry_count();
        unsafe { out_len.write(len) };
        Status::Ok
    })
}

/// Stores one entry under an already canonicalised `MosDNS` cache key.
///
/// # Safety
///
/// Every non-empty borrowed slice must remain readable for this call.
#[must_use]
pub unsafe fn cache_store(
    handle: u64,
    key: BorrowedSlice,
    response: BorrowedSlice,
    domain_set: BorrowedSlice,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let Ok(key) = (unsafe { key.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(response) = (unsafe { response.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(domain_set) = (unsafe { domain_set.as_slice() }) else {
            return Status::InvalidArgument;
        };
        if key.is_empty()
            || response.is_empty()
            || wire::validate_response(response).is_err()
            || message_expires_at_unix < stored_at_unix
            || cache_expires_at_unix < stored_at_unix
        {
            return Status::InvalidArgument;
        }
        let Some(state) = handles().get(&handle) else {
            return Status::Closed;
        };
        state.entries.insert(
            Bytes::copy_from_slice(key),
            Arc::new(CacheEntry {
                response: Bytes::copy_from_slice(response),
                domain_set: Bytes::copy_from_slice(domain_set),
                stored_at_unix,
                message_expires_at_unix,
                cache_expires_at_unix,
            }),
        );
        Status::Ok
    })
}

/// Looks up one already canonicalised `MosDNS` cache key.
///
/// # Safety
///
/// `key` must satisfy [`BorrowedSlice`]'s pointer contract and `out_result`
/// must point to writable [`LookupResult`] storage for this call.
pub unsafe fn cache_lookup(
    handle: u64,
    key: BorrowedSlice,
    now_unix: i64,
    out_result: *mut LookupResult,
) -> Status {
    boundary(|| {
        if handle == 0 || out_result.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(key_bytes) = (unsafe { key.as_slice() }) else {
            return Status::InvalidArgument;
        };
        if key_bytes.is_empty() {
            return Status::InvalidArgument;
        }
        let Some(state) = handles().get(&handle) else {
            return Status::Closed;
        };
        let mut result = LookupResult::empty();
        result.status = Status::Ok;
        match lookup_data(&state, key_bytes, now_unix) {
            Ok(Some(data)) => {
                result.state = data.state;
                result.stored_at_unix = data.stored_at_unix;
                result.message_expires_at_unix = data.message_expires_at_unix;
                result.response = OwnedBuffer::from_vec(data.response);
                result.domain_set = OwnedBuffer::copy_from_slice(&data.domain_set);
            }
            Ok(None) => {}
            Err(status) => {
                result.status = status;
                unsafe { out_result.write(result) };
                return status;
            }
        }
        unsafe { out_result.write(result) };
        Status::Ok
    })
}

/// Looks up one key and writes hit payloads into caller-owned buffers.
///
/// # Safety
///
/// Borrowed input and writable output slices must remain valid for this call,
/// and `out_result` must point to writable [`LookupIntoResult`] storage.
pub unsafe fn cache_lookup_into(
    handle: u64,
    key: BorrowedSlice,
    now_unix: i64,
    mut response_out: WritableSlice,
    mut domain_set_out: WritableSlice,
    out_result: *mut LookupIntoResult,
) -> Status {
    boundary(|| {
        if handle == 0 || out_result.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(key_bytes) = (unsafe { key.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(response_buffer) = (unsafe { response_out.as_mut_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(domain_set_buffer) = (unsafe { domain_set_out.as_mut_slice() }) else {
            return Status::InvalidArgument;
        };
        if key_bytes.is_empty() {
            return Status::InvalidArgument;
        }
        let Some(state) = handles().get(&handle) else {
            return Status::Closed;
        };
        let mut result = LookupIntoResult::empty();
        result.status = Status::Ok;
        match lookup_data(&state, key_bytes, now_unix) {
            Ok(Some(data)) => {
                result.state = data.state;
                result.stored_at_unix = data.stored_at_unix;
                result.message_expires_at_unix = data.message_expires_at_unix;
                result.response_len = u64::try_from(data.response.len()).unwrap_or(u64::MAX);
                result.domain_set_len = u64::try_from(data.domain_set.len()).unwrap_or(u64::MAX);
                if response_buffer.len() < data.response.len()
                    || domain_set_buffer.len() < data.domain_set.len()
                {
                    result.status = Status::BufferTooSmall;
                    unsafe { out_result.write(result) };
                    return Status::BufferTooSmall;
                }
                response_buffer[..data.response.len()].copy_from_slice(&data.response);
                domain_set_buffer[..data.domain_set.len()].copy_from_slice(&data.domain_set);
            }
            Ok(None) => {}
            Err(status) => {
                result.status = status;
                unsafe { out_result.write(result) };
                return status;
            }
        }
        unsafe { out_result.write(result) };
        Status::Ok
    })
}

#[must_use]
pub fn cache_flush(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let Some(state) = handles().get(&handle) else {
            return Status::Closed;
        };
        state.entries.invalidate_all();
        state.entries.run_pending_tasks();
        Status::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::{Status, boundary};

    #[test]
    fn panic_is_contained_at_boundary() {
        assert_eq!(boundary(|| panic!("ffi test panic")), Status::Panic);
    }
}
