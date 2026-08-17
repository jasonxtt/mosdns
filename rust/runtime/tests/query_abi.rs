//! Query Snapshot v1 ABI contract tests.

use mosdns_cache_core::{BorrowedSlice, Status, WritableSlice, cache_close};
use mosdns_runtime::{
    QueryInspectResult, QuerySnapshotInput, cache_abi_capabilities, domain_matcher_close,
    ip_matcher_close, query_abi_capabilities, query_abi_version, query_snapshot_close,
    query_snapshot_create, query_snapshot_inspect, query_snapshot_required_len,
};

const QUERY_CAPABILITY_SNAPSHOT: u64 = 1 << 5;
const QUERY_CAPABILITY_INSPECT: u64 = 1 << 6;

#[test]
fn query_abi_version_and_capability_are_exposed() {
    assert_eq!(query_abi_version(), 1);
    let capabilities = query_abi_capabilities();
    assert_ne!(capabilities & QUERY_CAPABILITY_SNAPSHOT, 0);
    assert_ne!(capabilities & QUERY_CAPABILITY_INSPECT, 0);
    let all_capabilities = cache_abi_capabilities();
    assert_ne!(all_capabilities & QUERY_CAPABILITY_SNAPSHOT, 0);
    assert_ne!(all_capabilities & QUERY_CAPABILITY_INSPECT, 0);
}

#[test]
fn query_abi_records_are_fixed_width() {
    assert_eq!(std::mem::size_of::<QuerySnapshotInput>(), 48);
    assert_eq!(std::mem::size_of::<QueryInspectResult>(), 80);
}

fn valid_query() -> Vec<u8> {
    vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e', b'x',
        b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00, 0x01,
    ]
}

fn query_with_opt_ecs() -> Vec<u8> {
    let mut wire = valid_query();
    wire[10] = 0;
    wire[11] = 1;
    // root owner, OPT, UDP size 1232, DO bit, ECS family 1 / mask 24 / scope 0
    wire.extend_from_slice(&[
        0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x80, 0x00, 0x00, 0x0b, 0x00, 0x08, 0x00, 0x07,
        0x00, 0x01, 0x18, 0x00, 192, 0, 2,
    ]);
    wire
}

fn query_with_compressed_opt_owner() -> Vec<u8> {
    let mut wire = valid_query();
    wire[10] = 0;
    wire[11] = 1;
    // The owner pointer targets the question name at absolute packet offset
    // 12. This must be resolved against the full DNS message, not the
    // additional-record subslice.
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0, 0]);
    wire
}

fn overlong_query_name() -> Vec<u8> {
    let mut wire = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for _ in 0..4 {
        wire.push(63);
        wire.extend([b'x'; 63]);
    }
    wire.push(0);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire
}

fn input(wire: &[u8]) -> QuerySnapshotInput {
    QuerySnapshotInput {
        struct_size: u32::try_from(std::mem::size_of::<QuerySnapshotInput>())
            .expect("query input record size fits u32"),
        version: 1,
        flags: 0,
        reserved: 0,
        query_wire: BorrowedSlice::from_slice(wire),
        from_udp: 1,
        transport_mode: 0,
        advertised_udp_size: 1232,
        reserved_tail: 0,
        pre_fast_flags: 0x1122_3344_5566_7788,
    }
}

#[test]
fn valid_snapshot_can_be_created_and_closed_exactly_once() {
    let wire = valid_query();
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut handle) },
        Status::Ok
    );
    assert!(handle >= 0x4000_0000_0000_0000);
    assert_eq!(query_snapshot_close(handle), Status::Ok);
    assert_eq!(query_snapshot_close(handle), Status::Closed);
}

#[test]
fn invalid_snapshot_input_publishes_no_handle() {
    let wire = valid_query();
    let mut cases = Vec::new();
    let mut bad_version = input(&wire);
    bad_version.version = 99;
    cases.push(bad_version);
    let mut bad_size = input(&wire);
    bad_size.struct_size = 0;
    cases.push(bad_size);
    let mut bad_flags = input(&wire);
    bad_flags.flags = 1;
    cases.push(bad_flags);
    let mut bad_reserved = input(&wire);
    bad_reserved.reserved = 1;
    cases.push(bad_reserved);
    let mut bad_reserved_tail = input(&wire);
    bad_reserved_tail.reserved_tail = 1;
    cases.push(bad_reserved_tail);
    let mut bad_transport = input(&wire);
    bad_transport.transport_mode = 3;
    cases.push(bad_transport);
    let mut bad_from_udp = input(&wire);
    bad_from_udp.from_udp = 2;
    cases.push(bad_from_udp);
    for bad in cases {
        let mut handle = 0xfeed_beef;
        assert_eq!(
            unsafe { query_snapshot_create(bad, &raw mut handle) },
            Status::InvalidArgument
        );
        assert_eq!(handle, 0xfeed_beef);
    }

    let malformed = [0_u8; 12];
    let mut malformed_handle = 0xdead_beef;
    assert_eq!(
        unsafe { query_snapshot_create(input(&malformed), &raw mut malformed_handle) },
        Status::InvalidArgument
    );
    assert_eq!(malformed_handle, 0xdead_beef);

    let mut malformed_opt = valid_query();
    malformed_opt[11] = 1;
    malformed_opt.extend_from_slice(&[0x00, 0x00, 0x29]);
    let mut malformed_opt_handle = 0xcafe;
    assert_eq!(
        unsafe { query_snapshot_create(input(&malformed_opt), &raw mut malformed_opt_handle) },
        Status::InvalidArgument
    );
    assert_eq!(malformed_opt_handle, 0xcafe);

    let mut overlong_handle = 0xbeef;
    assert_eq!(
        unsafe { query_snapshot_create(input(&overlong_query_name()), &raw mut overlong_handle) },
        Status::InvalidArgument
    );
    assert_eq!(overlong_handle, 0xbeef);

    let mut malformed_txt = valid_query();
    malformed_txt[10] = 0;
    malformed_txt[11] = 1;
    malformed_txt.extend_from_slice(&[
        0x00, 0x00, 0x10, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x14, 0x03, b'a', b'b', b'c',
    ]);
    let mut malformed_txt_handle = 0xabcd;
    assert_eq!(
        unsafe { query_snapshot_create(input(&malformed_txt), &raw mut malformed_txt_handle) },
        Status::InvalidArgument
    );
    assert_eq!(malformed_txt_handle, 0xabcd);

    let mut valid_txt = valid_query();
    valid_txt[10] = 0;
    valid_txt[11] = 1;
    valid_txt.extend_from_slice(&[
        0x00, 0x00, 0x10, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04, 0x03, b'a', b'b', b'c',
    ]);
    let mut valid_txt_handle = 0xdcba;
    assert_eq!(
        unsafe { query_snapshot_create(input(&valid_txt), &raw mut valid_txt_handle) },
        Status::InvalidArgument
    );
    assert_eq!(valid_txt_handle, 0xdcba);
}

#[test]
fn compressed_additional_owner_uses_full_packet_offsets() {
    let wire = query_with_compressed_opt_owner();
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut handle) },
        Status::Ok
    );
    assert_eq!(query_snapshot_close(handle), Status::Ok);
}

#[test]
fn inspect_reports_normalized_fields_and_caller_owned_wire_copy() {
    let mut wire = valid_query();
    let original = wire.clone();
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut handle) },
        Status::Ok
    );
    wire[0] ^= 0xff;

    let mut required = 0;
    assert_eq!(
        unsafe { query_snapshot_required_len(handle, &raw mut required) },
        Status::Ok
    );
    assert_eq!(required, original.len() as u64);

    let mut output = vec![0xa5; original.len() + 4];
    let mut result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                handle,
                WritableSlice::from_slice(&mut output),
                &raw mut result,
            )
        },
        Status::Ok
    );
    assert_eq!(result.status, Status::Ok);
    assert_eq!(result.id, 0x1234);
    assert_eq!(result.qtype, 1);
    assert_eq!(result.qclass, 1);
    assert_eq!(result.qname_len, 13);
    assert_eq!(result.from_udp, 1);
    assert_eq!(result.transport_mode, 0);
    assert_eq!(result.advertised_udp_size, 1232);
    assert_eq!(result.pre_fast_flags, 0x1122_3344_5566_7788);
    assert_eq!(result.required_len, wire.len() as u64);
    assert_eq!(result.written_len, wire.len() as u64);
    assert_eq!(&output[..original.len()], &original);
    assert!(output[original.len()..].iter().all(|&byte| byte == 0xa5));

    output[0] ^= 0xff;
    let mut second = vec![0; original.len()];
    let mut second_result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                handle,
                WritableSlice::from_slice(&mut second),
                &raw mut second_result,
            )
        },
        Status::Ok
    );
    assert_eq!(second, original);
    assert_eq!(query_snapshot_close(handle), Status::Ok);
}

#[test]
fn short_inspect_buffer_reports_required_length_without_writing() {
    let wire = valid_query();
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut handle) },
        Status::Ok
    );
    let mut output = vec![0x5a; wire.len() - 1];
    let before = output.clone();
    let mut result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                handle,
                WritableSlice::from_slice(&mut output),
                &raw mut result,
            )
        },
        Status::BufferTooSmall
    );
    assert_eq!(result.status, Status::BufferTooSmall);
    assert_eq!(result.required_len, wire.len() as u64);
    assert_eq!(result.written_len, 0);
    assert_eq!(output, before);
    assert_eq!(query_snapshot_close(handle), Status::Ok);
}

#[test]
fn inspect_reports_edns_do_and_fixed_width_ecs() {
    let wire = query_with_opt_ecs();
    let mut snapshot_input = input(&wire);
    snapshot_input.advertised_udp_size = 4096;
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(snapshot_input, &raw mut handle) },
        Status::Ok
    );
    let mut output = vec![0; wire.len()];
    let mut result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                handle,
                WritableSlice::from_slice(&mut output),
                &raw mut result,
            )
        },
        Status::Ok
    );
    assert_eq!(result.has_opt, 1);
    assert_eq!(result.do_bit, 1);
    assert_eq!(result.advertised_udp_size, 4096);
    assert_eq!(result.edns_udp_size, 1232);
    assert_eq!(result.ecs_present, 1);
    assert_eq!(result.ecs_family, 1);
    assert_eq!(result.ecs_source_netmask, 24);
    assert_eq!(result.ecs_source_scope, 0);
    assert_eq!(&result.ecs_address[..4], &[192, 0, 2, 0]);
    assert_eq!(query_snapshot_close(handle), Status::Ok);
}

#[test]
fn query_abi_rejects_null_and_overflow_descriptors() {
    let wire = valid_query();
    let mut null_wire = input(&wire);
    null_wire.query_wire = BorrowedSlice {
        ptr: std::ptr::null(),
        len: 1,
    };
    let mut handle = 0x1111;
    assert_eq!(
        unsafe { query_snapshot_create(null_wire, &raw mut handle) },
        Status::InvalidArgument
    );
    assert_eq!(handle, 0x1111);

    let mut overflow_wire = input(&wire);
    overflow_wire.query_wire = BorrowedSlice {
        ptr: std::ptr::dangling(),
        len: u64::MAX,
    };
    assert_eq!(
        unsafe { query_snapshot_create(overflow_wire, &raw mut handle) },
        Status::InvalidArgument
    );
    assert_eq!(handle, 0x1111);
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), std::ptr::null_mut()) },
        Status::InvalidArgument
    );

    let mut valid_handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut valid_handle) },
        Status::Ok
    );
    let mut invalid_output_result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                valid_handle,
                WritableSlice {
                    ptr: std::ptr::null_mut(),
                    len: 1,
                },
                &raw mut invalid_output_result,
            )
        },
        Status::InvalidArgument
    );
    assert_eq!(invalid_output_result.status, Status::InvalidArgument);
    let mut overflow_output_result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                valid_handle,
                WritableSlice {
                    ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
                    len: u64::MAX,
                },
                &raw mut overflow_output_result,
            )
        },
        Status::InvalidArgument
    );
    assert_eq!(overflow_output_result.status, Status::InvalidArgument);
    assert_eq!(
        unsafe {
            query_snapshot_inspect(valid_handle, WritableSlice::empty(), std::ptr::null_mut())
        },
        Status::InvalidArgument
    );
    assert_eq!(query_snapshot_close(valid_handle), Status::Ok);

    assert_eq!(query_snapshot_close(0), Status::InvalidArgument);
    assert_eq!(
        unsafe { query_snapshot_required_len(0, std::ptr::null_mut()) },
        Status::InvalidArgument
    );
}

#[test]
fn query_handles_use_a_disjoint_namespace_and_are_not_reused() {
    let wire = valid_query();
    let mut first = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut first) },
        Status::Ok
    );
    assert!(first >= 0x4000_0000_0000_0000);
    assert_eq!(cache_close(first), Status::Closed);
    assert_eq!(domain_matcher_close(first), Status::Closed);
    assert_eq!(ip_matcher_close(first), Status::Closed);
    assert_eq!(query_snapshot_close(first), Status::Ok);

    let mut second = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut second) },
        Status::Ok
    );
    assert_ne!(first, second);
    assert_eq!(query_snapshot_close(first), Status::Closed);
    let mut required = 0;
    assert_eq!(
        unsafe { query_snapshot_required_len(second, &raw mut required) },
        Status::Ok
    );
    assert_eq!(required, wire.len() as u64);
    assert_eq!(query_snapshot_close(second), Status::Ok);
}

#[test]
fn closed_handle_inspect_reports_closed_without_touching_output() {
    let wire = valid_query();
    let mut handle = 0;
    assert_eq!(
        unsafe { query_snapshot_create(input(&wire), &raw mut handle) },
        Status::Ok
    );
    assert_eq!(query_snapshot_close(handle), Status::Ok);
    let mut output = vec![0x7b; wire.len()];
    let before = output.clone();
    let mut result = QueryInspectResult::empty();
    assert_eq!(
        unsafe {
            query_snapshot_inspect(
                handle,
                WritableSlice::from_slice(&mut output),
                &raw mut result,
            )
        },
        Status::Closed
    );
    assert_eq!(result.status, Status::Closed);
    assert_eq!(output, before);
}
