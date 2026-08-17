//! Versioned query snapshot ABI records and capability negotiation.

use mosdns_cache_core::{BorrowedSlice, Status, WritableSlice};
use mosdns_dns_core::{EdnsInfo, QueryHeader, QuestionInfo, extract_edns_at, parse_query};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

/// Capability bit indicating the immutable query snapshot registry.
pub const CAPABILITY_QUERY_SNAPSHOT: u64 = 1 << 5;
/// Capability bit indicating the caller-buffer inspect operation.
pub const CAPABILITY_QUERY_INSPECT: u64 = 1 << 6;

pub const QUERY_ABI_VERSION: u32 = 1;
pub const QUERY_RESULT_VERSION: u32 = 1;

/// UDP transport discriminator.
pub const QUERY_TRANSPORT_UDP: u8 = 0;
/// Stream/TCP transport discriminator.
pub const QUERY_TRANSPORT_STREAM: u8 = 1;
/// HTTP/DoH transport discriminator.
pub const QUERY_TRANSPORT_HTTP: u8 = 2;

const QUERY_HANDLE_BASE: u64 = 0x4000_0000_0000_0000;
static NEXT_QUERY_HANDLE: AtomicU64 = AtomicU64::new(QUERY_HANDLE_BASE);

struct QuerySnapshot {
    wire: Vec<u8>,
    header: QueryHeader,
    question: QuestionInfo,
    edns: Option<EdnsInfo>,
    flags: u32,
    from_udp: bool,
    transport_mode: u8,
    advertised_udp_size: u16,
    pre_fast_flags: u64,
}

fn query_table() -> &'static RwLock<HashMap<u64, QuerySnapshot>> {
    static TABLE: OnceLock<RwLock<HashMap<u64, QuerySnapshot>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn boundary(operation: impl FnOnce() -> Status) -> Status {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(Status::Panic)
}

/// Fixed-width v1 input record. The byte slice descriptor is passed by value;
/// the runtime copies the bytes before publishing a handle.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct QuerySnapshotInput {
    pub struct_size: u32,
    pub version: u32,
    pub flags: u32,
    pub reserved: u32,
    pub query_wire: BorrowedSlice,
    pub from_udp: u8,
    pub transport_mode: u8,
    pub advertised_udp_size: u16,
    pub reserved_tail: u32,
    pub pre_fast_flags: u64,
}

/// Fixed-width v1 result metadata. `output` carries the caller-owned query
/// wire copy; the normalized fields remain valid even when that buffer is too
/// small, and `required_len` reports the exact retry size.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryInspectResult {
    pub status: Status,
    pub version: u32,
    pub flags: u32,
    pub id: u16,
    pub qtype: u16,
    pub qclass: u16,
    pub advertised_udp_size: u16,
    pub edns_udp_size: u16,
    pub ecs_family: u16,
    pub ecs_source_netmask: u8,
    pub ecs_source_scope: u8,
    pub from_udp: u8,
    pub transport_mode: u8,
    pub has_opt: u8,
    pub do_bit: u8,
    pub ecs_present: u8,
    pub reserved: u8,
    pub qname_len: u64,
    pub pre_fast_flags: u64,
    pub required_len: u64,
    pub written_len: u64,
    pub ecs_address: [u8; 16],
}

impl QueryInspectResult {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            status: Status::Internal,
            version: QUERY_RESULT_VERSION,
            flags: 0,
            id: 0,
            qtype: 0,
            qclass: 0,
            advertised_udp_size: 0,
            edns_udp_size: 0,
            ecs_family: 0,
            ecs_source_netmask: 0,
            ecs_source_scope: 0,
            from_udp: 0,
            transport_mode: QUERY_TRANSPORT_UDP,
            has_opt: 0,
            do_bit: 0,
            ecs_present: 0,
            reserved: 0,
            qname_len: 0,
            pre_fast_flags: 0,
            required_len: 0,
            written_len: 0,
            ecs_address: [0; 16],
        }
    }
}

fn validate_input(
    input: QuerySnapshotInput,
) -> Result<(Vec<u8>, QueryHeader, QuestionInfo, Option<EdnsInfo>), Status> {
    if input.struct_size as usize != std::mem::size_of::<QuerySnapshotInput>()
        || input.version != QUERY_ABI_VERSION
        || input.flags != 0
        || input.reserved != 0
        || input.reserved_tail != 0
        || input.from_udp > 1
        || input.transport_mode > QUERY_TRANSPORT_HTTP
    {
        return Err(Status::InvalidArgument);
    }
    // SAFETY: the caller owns the borrowed slice for the duration of this ABI call.
    let query = unsafe { input.query_wire.as_slice() }?;
    let (header, question) = parse_query(query).map_err(|_| Status::InvalidArgument)?;
    let question_end = 12usize
        .checked_add(question.qname_wire.len())
        .and_then(|end| end.checked_add(4))
        .ok_or(Status::InvalidArgument)?;
    if question_end > query.len() {
        return Err(Status::InvalidArgument);
    }
    let edns = if header.arcount == 1 {
        extract_edns_at(query, question_end).map_err(|_| Status::InvalidArgument)?
    } else {
        None
    };
    Ok((query.to_vec(), header, question, edns))
}

fn result_for_snapshot(snapshot: &QuerySnapshot) -> QueryInspectResult {
    let mut result = QueryInspectResult::empty();
    result.flags = snapshot.flags;
    result.id = snapshot.header.id;
    result.qtype = snapshot.question.qtype;
    result.qclass = snapshot.question.qclass;
    result.advertised_udp_size = snapshot.advertised_udp_size;
    result.from_udp = u8::from(snapshot.from_udp);
    result.transport_mode = snapshot.transport_mode;
    result.pre_fast_flags = snapshot.pre_fast_flags;
    result.qname_len = snapshot.question.qname_wire.len() as u64;
    result.required_len = snapshot.wire.len() as u64;
    if let Some(edns) = &snapshot.edns {
        result.has_opt = u8::from(edns.has_opt);
        result.edns_udp_size = edns.udp_size;
        result.do_bit = u8::from(edns.do_bit);
        if let Some(ecs) = &edns.ecs {
            result.ecs_present = 1;
            result.ecs_family = ecs.family;
            result.ecs_source_netmask = ecs.source_netmask;
            result.ecs_source_scope = ecs.source_scope;
            let len = ecs.address.len().min(result.ecs_address.len());
            result.ecs_address[..len].copy_from_slice(&ecs.address[..len]);
        }
    }
    result
}

/// Creates an immutable query snapshot handle from a versioned input record.
///
/// # Safety
///
/// `input.query_wire` must satisfy [`BorrowedSlice`]'s pointer contract and
/// `out_handle` must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_create(
    input: QuerySnapshotInput,
    out_handle: *mut u64,
) -> Status {
    boundary(|| {
        if out_handle.is_null() {
            return Status::InvalidArgument;
        }
        let Ok((wire, header, question, edns)) = validate_input(input) else {
            return Status::InvalidArgument;
        };
        let Ok(mut table) = query_table().write() else {
            return Status::Internal;
        };
        let handle = loop {
            let candidate = NEXT_QUERY_HANDLE.fetch_add(1, Ordering::Relaxed);
            if candidate < QUERY_HANDLE_BASE {
                return Status::Internal;
            }
            if !table.contains_key(&candidate) {
                break candidate;
            }
        };
        table.insert(
            handle,
            QuerySnapshot {
                wire,
                header,
                question,
                edns,
                flags: input.flags,
                from_udp: input.from_udp != 0,
                transport_mode: input.transport_mode,
                advertised_udp_size: input.advertised_udp_size,
                pre_fast_flags: input.pre_fast_flags,
            },
        );
        // SAFETY: the caller provided writable storage for the handle.
        unsafe { out_handle.write(handle) };
        Status::Ok
    })
}

/// Returns the exact caller-buffer length required by one snapshot inspect.
///
/// # Safety
///
/// `out_len` must point to writable `u64` storage for this call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_required_len(handle: u64, out_len: *mut u64) -> Status {
    boundary(|| {
        if handle == 0 || out_len.is_null() {
            return Status::InvalidArgument;
        }
        let Ok(table) = query_table().read() else {
            return Status::Internal;
        };
        let Some(snapshot) = table.get(&handle) else {
            return Status::Closed;
        };
        let Ok(required) = u64::try_from(snapshot.wire.len()) else {
            return Status::Internal;
        };
        // SAFETY: the caller provided writable storage for the length.
        unsafe { out_len.write(required) };
        Status::Ok
    })
}

/// Inspects one immutable snapshot and copies its query wire into caller
/// storage. A short buffer reports the required length without writing bytes.
///
/// # Safety
///
/// `output` must satisfy [`WritableSlice`]'s pointer contract and
/// `out_result` must point to writable [`QueryInspectResult`] storage for this
/// call.
#[must_use]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_inspect(
    handle: u64,
    mut output: WritableSlice,
    out_result: *mut QueryInspectResult,
) -> Status {
    boundary(|| {
        if out_result.is_null() {
            return Status::InvalidArgument;
        }
        if handle == 0 {
            let mut result = QueryInspectResult::empty();
            result.status = Status::InvalidArgument;
            // SAFETY: null was rejected above.
            unsafe { out_result.write(result) };
            return Status::InvalidArgument;
        }
        let Ok(table) = query_table().read() else {
            let mut result = QueryInspectResult::empty();
            result.status = Status::Internal;
            // SAFETY: null was rejected above.
            unsafe { out_result.write(result) };
            return Status::Internal;
        };
        let Some(snapshot) = table.get(&handle) else {
            let mut result = QueryInspectResult::empty();
            result.status = Status::Closed;
            // SAFETY: null was rejected above.
            unsafe { out_result.write(result) };
            return Status::Closed;
        };
        let mut result = result_for_snapshot(snapshot);
        // SAFETY: the caller owns this output buffer for the duration of the call.
        let Ok(output_bytes) = (unsafe { output.as_mut_slice() }) else {
            result.status = Status::InvalidArgument;
            unsafe { out_result.write(result) };
            return Status::InvalidArgument;
        };
        if output_bytes.len() < snapshot.wire.len() {
            result.status = Status::BufferTooSmall;
            unsafe { out_result.write(result) };
            return Status::BufferTooSmall;
        }
        output_bytes[..snapshot.wire.len()].copy_from_slice(&snapshot.wire);
        result.status = Status::Ok;
        result.written_len = result.required_len;
        unsafe { out_result.write(result) };
        Status::Ok
    })
}

/// Closes one exact query snapshot handle. Handles are never reused.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn query_snapshot_close(handle: u64) -> Status {
    boundary(|| {
        if handle == 0 {
            return Status::InvalidArgument;
        }
        let Ok(mut table) = query_table().write() else {
            return Status::Internal;
        };
        if table.remove(&handle).is_some() {
            Status::Ok
        } else {
            Status::Closed
        }
    })
}

/// Returns the query snapshot ABI version.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn query_abi_version() -> u32 {
    QUERY_ABI_VERSION
}

/// Returns the query snapshot capability bits.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn query_abi_capabilities() -> u64 {
    CAPABILITY_QUERY_SNAPSHOT | CAPABILITY_QUERY_INSPECT
}

#[cfg(test)]
mod tests {
    use super::{
        QUERY_ABI_VERSION, QUERY_TRANSPORT_UDP, QuerySnapshotInput, boundary, query_snapshot_close,
        query_snapshot_create, query_table,
    };
    use mosdns_cache_core::{BorrowedSlice, Status};
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    fn valid_input(wire: &[u8]) -> QuerySnapshotInput {
        QuerySnapshotInput {
            struct_size: u32::try_from(std::mem::size_of::<QuerySnapshotInput>()).unwrap(),
            version: QUERY_ABI_VERSION,
            flags: 0,
            reserved: 0,
            query_wire: BorrowedSlice::from_slice(wire),
            from_udp: 1,
            transport_mode: QUERY_TRANSPORT_UDP,
            advertised_udp_size: 512,
            reserved_tail: 0,
            pre_fast_flags: 0,
        }
    }

    #[test]
    fn panic_boundary_maps_panics_to_status() {
        assert_eq!(boundary(|| panic!("query ABI test panic")), Status::Panic);
    }

    #[test]
    fn close_waits_for_an_in_flight_registry_read() {
        let wire = vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, b'x',
            0x00, 0x00, 0x01, 0x00, 0x01,
        ];
        let mut handle = 0;
        assert_eq!(
            unsafe { query_snapshot_create(valid_input(&wire), &raw mut handle) },
            Status::Ok
        );
        let read_guard = query_table().read().unwrap();
        let (sender, receiver) = sync_channel(0);
        std::thread::spawn(move || {
            sender.send(query_snapshot_close(handle)).unwrap();
        });
        assert!(receiver.recv_timeout(Duration::from_millis(20)).is_err());
        drop(read_guard);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Status::Ok)
        );
    }
}
