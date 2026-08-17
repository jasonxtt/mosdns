//! Matcher ABI handle management for the MosDNS runtime.
//!
//! Handles are typed integers backed by an `RwLock<HashMap>`. Queries hold
//! only a read lock during match — concurrent lookups proceed in parallel.
//! Separate registries for domain and IP keep the handle spaces independent.
//! Panic containment uses the same `catch_unwind` pattern as cache-core.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use mosdns_cache_core::{BorrowedSlice, Status, WritableSlice};
use mosdns_matcher_core::{IpPrefixList, MixMatcher, ValuedDomainMatcher};

/// Capability bit indicating that matcher ABI functions are available.
pub const CAPABILITY_MATCHER: u64 = 1 << 3;
/// Capability bit indicating that the valued matcher ABI is available.
pub const CAPABILITY_VALUED_MATCHER: u64 = 1 << 4;

// --- Domain matcher handles ---

// Handle namespaces start at non-overlapping offsets so a cache handle (≈ 1)
// can never collide with a domain or IP handle.
const DOMAIN_HANDLE_BASE: u64 = 0x1000_0000_0000_0000;
const IP_HANDLE_BASE: u64 = 0x2000_0000_0000_0000;
static NEXT_DOMAIN_HANDLE: AtomicU64 = AtomicU64::new(DOMAIN_HANDLE_BASE);

type DomainState = MixMatcher<()>;

fn domain_table() -> &'static RwLock<HashMap<u64, DomainState>> {
    static TABLE: OnceLock<RwLock<HashMap<u64, DomainState>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Creates a domain matcher from a batch of newline-separated rules.
///
/// # Safety
///
/// `rules` must satisfy `BorrowedSlice`'s pointer contract and `out_handle`
/// must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn domain_matcher_create(
    rules: BorrowedSlice,
    default_type: u32,
    out_handle: *mut u64,
) -> Status {
    boundary(|| {
        if out_handle.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(rule_bytes) = (unsafe { rules.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(rule_text) = std::str::from_utf8(rule_bytes) else {
            return Status::InvalidArgument;
        };

        let mut matcher = MixMatcher::new();
        match default_type {
            0 => matcher.set_default("domain"),
            1 => matcher.set_default("full"),
            _ => return Status::InvalidArgument,
        }

        // Build the candidate in one pass. The matcher is local to this FFI
        // call and is only inserted into the handle table after every rule
        // has been accepted, so an invalid batch cannot publish a partial
        // snapshot.
        for line in rule_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if matcher.add(line, ()).is_err() {
                return Status::InvalidArgument;
            }
        }

        let handle = NEXT_DOMAIN_HANDLE.fetch_add(1, Ordering::Relaxed);
        domain_table().write().unwrap().insert(handle, matcher);
        unsafe { out_handle.write(handle) };
        Status::Ok
    })
}

/// Check a domain against a domain matcher handle.
///
/// # Safety
///
/// `domain` must satisfy `BorrowedSlice`'s pointer contract and `out_match`
/// must point to writable `bool` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn domain_matcher_match(
    handle: u64,
    domain: BorrowedSlice,
    out_match: *mut bool,
) -> Status {
    boundary(|| {
        if out_match.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(domain_bytes) = (unsafe { domain.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(domain_str) = std::str::from_utf8(domain_bytes) else {
            return Status::InvalidArgument;
        };
        if !domain_str.is_ascii() {
            return Status::InvalidArgument;
        }
        let table = domain_table().read().unwrap();
        let Some(matcher) = table.get(&handle) else {
            return Status::Closed;
        };
        let matched = matcher.r#match(domain_str).is_some();
        unsafe { out_match.write(matched) };
        Status::Ok
    })
}

/// Returns the number of rules in a domain matcher.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn domain_matcher_len(handle: u64, out_len: *mut u64) -> Status {
    boundary(|| {
        if out_len.is_null() {
            return Status::InvalidArgument;
        }
        let table = domain_table().read().unwrap();
        let Some(matcher) = table.get(&handle) else {
            return Status::Closed;
        };
        let len = u64::try_from(matcher.len()).unwrap_or(u64::MAX);
        unsafe { out_len.write(len) };
        Status::Ok
    })
}

/// Destroys a domain matcher handle.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn domain_matcher_close(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let mut table = domain_table().write().unwrap();
        if table.remove(&handle).is_some() {
            Status::Ok
        } else {
            Status::Closed
        }
    })
}

// --- IP matcher handles ---

static NEXT_IP_HANDLE: AtomicU64 = AtomicU64::new(IP_HANDLE_BASE);

type IpState = IpPrefixList;

fn ip_table() -> &'static RwLock<HashMap<u64, IpState>> {
    static TABLE: OnceLock<RwLock<HashMap<u64, IpState>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Creates an IP matcher from a batch of newline-separated IP prefixes.
///
/// # Safety
///
/// `data` must satisfy `BorrowedSlice`'s pointer contract and `out_handle`
/// must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ip_matcher_create(data: BorrowedSlice, out_handle: *mut u64) -> Status {
    boundary(|| {
        if out_handle.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(data_bytes) = (unsafe { data.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(data_text) = std::str::from_utf8(data_bytes) else {
            return Status::InvalidArgument;
        };

        let mut list = IpPrefixList::new();
        // Validate every line; reject the entire batch on the first invalid line.
        for line in data_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parsed = line
                .parse::<std::net::IpAddr>()
                .map(|addr| {
                    let bits = if addr.is_ipv4() { 32 } else { 128 };
                    (addr, bits)
                })
                .or_else(|_| parse_cidr(line).ok_or(()));
            let (addr, bits) = match parsed {
                Ok(v) => v,
                Err(_) => return Status::InvalidArgument,
            };
            list.append(addr, bits);
        }
        list.rebuild();

        let handle = NEXT_IP_HANDLE.fetch_add(1, Ordering::Relaxed);
        ip_table().write().unwrap().insert(handle, list);
        unsafe { out_handle.write(handle) };
        Status::Ok
    })
}

fn parse_cidr(s: &str) -> Option<(std::net::IpAddr, u8)> {
    let (addr_str, bits_str) = s.split_once('/')?;
    let addr: std::net::IpAddr = addr_str.parse().ok()?;
    let bits: u8 = bits_str.parse().ok()?;
    if addr.is_ipv4() && bits > 32 {
        return None;
    }
    if addr.is_ipv6() && bits > 128 {
        return None;
    }
    Some((addr, bits))
}

/// Check an IP address against an IP matcher handle.
///
/// # Safety
///
/// `addr` must satisfy `BorrowedSlice`'s pointer contract and `out_match`
/// must point to writable `bool` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ip_matcher_match(
    handle: u64,
    addr_text: BorrowedSlice,
    out_match: *mut bool,
) -> Status {
    boundary(|| {
        if out_match.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(addr_bytes) = (unsafe { addr_text.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(s) = std::str::from_utf8(addr_bytes) else {
            return Status::InvalidArgument;
        };
        let Ok(addr) = s.parse::<std::net::IpAddr>() else {
            return Status::InvalidArgument;
        };
        let table = ip_table().read().unwrap();
        let Some(list) = table.get(&handle) else {
            return Status::Closed;
        };
        let matched = list.contains(addr);
        unsafe { out_match.write(matched) };
        Status::Ok
    })
}

/// Returns the number of prefix entries in an IP matcher.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ip_matcher_len(handle: u64, out_len: *mut u64) -> Status {
    boundary(|| {
        if out_len.is_null() {
            return Status::InvalidArgument;
        }
        let table = ip_table().read().unwrap();
        let Some(list) = table.get(&handle) else {
            return Status::Closed;
        };
        let len = u64::try_from(list.len()).unwrap_or(u64::MAX);
        unsafe { out_len.write(len) };
        Status::Ok
    })
}

/// Destroys an IP matcher handle.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn ip_matcher_close(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let mut table = ip_table().write().unwrap();
        if table.remove(&handle).is_some() {
            Status::Ok
        } else {
            Status::Closed
        }
    })
}

// --- Valued domain matcher handles ---

const VALUED_HANDLE_BASE: u64 = 0x3000_0000_0000_0000;
static NEXT_VALUED_HANDLE: AtomicU64 = AtomicU64::new(VALUED_HANDLE_BASE);

fn valued_table() -> &'static RwLock<HashMap<u64, ValuedDomainMatcher>> {
    static TABLE: OnceLock<RwLock<HashMap<u64, ValuedDomainMatcher>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Result metadata returned by one valued matcher lookup.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValuedMatchResult {
    pub status: Status,
    pub matched: u32,
    pub required_len: u64,
}

impl ValuedMatchResult {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            status: Status::Internal,
            matched: 0,
            required_len: 0,
        }
    }
}

/// Creates a valued domain matcher from a versioned length-safe rule batch.
///
/// # Safety
///
/// `rules` must satisfy `BorrowedSlice`'s pointer contract and `out_handle`
/// must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn valued_domain_matcher_create(
    rules: BorrowedSlice,
    out_handle: *mut u64,
) -> Status {
    boundary(|| {
        if out_handle.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(rule_bytes) = (unsafe { rules.as_slice() }) else {
            return Status::InvalidArgument;
        };
        let Ok(matcher) = ValuedDomainMatcher::build_encoded(rule_bytes) else {
            return Status::InvalidArgument;
        };

        let handle = NEXT_VALUED_HANDLE.fetch_add(1, Ordering::Relaxed);
        let Ok(mut table) = valued_table().write() else {
            return Status::Internal;
        };
        table.insert(handle, matcher);
        unsafe { out_handle.write(handle) };
        Status::Ok
    })
}

/// Matches a domain and writes the versioned result payload into caller-owned
/// storage. A short output buffer returns `BufferTooSmall` and reports the
/// exact required length without writing a partial payload.
///
/// # Safety
///
/// `domain` must satisfy `BorrowedSlice`'s pointer contract, `output` must
/// satisfy `WritableSlice`'s pointer contract, and `out_result` must point to
/// writable `ValuedMatchResult` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn valued_domain_matcher_match(
    handle: u64,
    domain: BorrowedSlice,
    mut output: WritableSlice,
    out_result: *mut ValuedMatchResult,
) -> Status {
    boundary(|| {
        if out_result.is_null() {
            return Status::InvalidArgument;
        }
        if handle == 0 {
            write_valued_result(out_result, Status::InvalidArgument, 0, 0);
            return Status::InvalidArgument;
        }
        let Ok(domain_bytes) = (unsafe { domain.as_slice() }) else {
            write_valued_result(out_result, Status::InvalidArgument, 0, 0);
            return Status::InvalidArgument;
        };
        let Ok(domain_str) = std::str::from_utf8(domain_bytes) else {
            write_valued_result(out_result, Status::InvalidArgument, 0, 0);
            return Status::InvalidArgument;
        };
        if !domain_str.is_ascii() {
            write_valued_result(out_result, Status::InvalidArgument, 0, 0);
            return Status::InvalidArgument;
        }
        let Ok(output_bytes) = (unsafe { output.as_mut_slice() }) else {
            write_valued_result(out_result, Status::InvalidArgument, 0, 0);
            return Status::InvalidArgument;
        };

        let Ok(table) = valued_table().read() else {
            write_valued_result(out_result, Status::Internal, 0, 0);
            return Status::Internal;
        };
        let Some(matcher) = table.get(&handle) else {
            write_valued_result(out_result, Status::Closed, 0, 0);
            return Status::Closed;
        };
        let Some(result) = matcher.r#match(domain_str) else {
            write_valued_result(out_result, Status::Ok, 0, 0);
            return Status::Ok;
        };

        let Ok(encoded) = mosdns_matcher_core::encode_valued_match_result(&result) else {
            write_valued_result(out_result, Status::Internal, 0, 0);
            return Status::Internal;
        };
        let Ok(required_len) = u64::try_from(encoded.len()) else {
            write_valued_result(out_result, Status::Internal, 0, 0);
            return Status::Internal;
        };
        if output_bytes.len() < encoded.len() {
            write_valued_result(out_result, Status::BufferTooSmall, 1, required_len);
            return Status::BufferTooSmall;
        }
        output_bytes[..encoded.len()].copy_from_slice(&encoded);
        write_valued_result(out_result, Status::Ok, 1, required_len);
        Status::Ok
    })
}

/// Returns the number of unique rule strings in a valued matcher.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn valued_domain_matcher_len(handle: u64, out_len: *mut u64) -> Status {
    boundary(|| {
        if out_len.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(table) = valued_table().read() else {
            return Status::Internal;
        };
        let Some(matcher) = table.get(&handle) else {
            return Status::Closed;
        };
        let Ok(len) = u64::try_from(matcher.len()) else {
            return Status::Internal;
        };
        unsafe { out_len.write(len) };
        Status::Ok
    })
}

/// Destroys a valued domain matcher handle.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn valued_domain_matcher_close(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let Ok(mut table) = valued_table().write() else {
            return Status::Internal;
        };
        if table.remove(&handle).is_some() {
            Status::Ok
        } else {
            Status::Closed
        }
    })
}

fn write_valued_result(
    out_result: *mut ValuedMatchResult,
    status: Status,
    matched: u32,
    required_len: u64,
) {
    // SAFETY: The caller contract is checked by the public ABI function before
    // this helper is called.
    unsafe {
        out_result.write(ValuedMatchResult {
            status,
            matched,
            required_len,
        });
    }
}

// --- Panic boundary ---

fn boundary(operation: impl FnOnce() -> Status) -> Status {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(Status::Panic)
}
