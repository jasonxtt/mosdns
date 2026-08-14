use mosdns_cache_core::{BorrowedSlice, Status, WritableSlice};
use mosdns_matcher_core::{ValuedRule, decode_valued_match_result, encode_valued_rule_batch};
use mosdns_runtime::{
    ValuedMatchResult, cache_abi_capabilities, valued_domain_matcher_close,
    valued_domain_matcher_create, valued_domain_matcher_len, valued_domain_matcher_match,
};

fn sample_batch() -> Vec<u8> {
    encode_valued_rule_batch(&[
        ValuedRule::new("domain:example.com", 1, [10], "base", "source"),
        ValuedRule::new("keyword:child", 2, [20], "keyword", "keyword-source"),
    ])
    .expect("sample valued batch")
}

#[test]
fn valued_result_abi_layout_is_fixed_width() {
    assert_eq!(std::mem::size_of::<ValuedMatchResult>(), 16);
    assert_eq!(std::mem::align_of::<ValuedMatchResult>(), 8);
}

#[test]
fn valued_matcher_reports_required_size_then_writes_result() {
    let batch = sample_batch();
    let mut handle = 0;
    assert_eq!(
        unsafe { valued_domain_matcher_create(BorrowedSlice::from_slice(&batch), &raw mut handle) },
        Status::Ok
    );
    assert_ne!(handle, 0);

    let mut result = ValuedMatchResult::empty();
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                handle,
                BorrowedSlice::from_slice(b"child.example.com."),
                WritableSlice::empty(),
                &raw mut result,
            )
        },
        Status::BufferTooSmall
    );
    assert_eq!(result.status, Status::BufferTooSmall);
    assert_eq!(result.matched, 1);
    assert!(result.required_len > 0);

    let output_len = usize::try_from(result.required_len).expect("result length fits usize");
    let mut output = vec![0; output_len];
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                handle,
                BorrowedSlice::from_slice(b"child.example.com."),
                WritableSlice::from_slice(&mut output),
                &raw mut result,
            )
        },
        Status::Ok
    );
    assert_eq!(result.status, Status::Ok);
    assert_eq!(result.matched, 1);
    let decoded = decode_valued_match_result(&output).expect("encoded valued result");
    assert_eq!(decoded.fast_marks, 3);
    assert_eq!(decoded.ctx_marks, vec![10, 20]);
    assert_eq!(decoded.joined_tags, "base|keyword");

    assert_eq!(
        unsafe { valued_domain_matcher_len(handle, &raw mut result.required_len) },
        Status::Ok
    );
    assert_eq!(result.required_len, 2);
    assert_eq!(valued_domain_matcher_close(handle), Status::Ok);
    assert_eq!(valued_domain_matcher_close(handle), Status::Closed);
}

#[test]
fn valued_matcher_rejects_malformed_batches_and_keeps_typed_handles() {
    for malformed in [vec![0xff], vec![1, 1, 0, 0, 0], {
        let mut bytes = sample_batch();
        bytes.push(0);
        bytes
    }] {
        let mut handle = 123;
        assert_eq!(
            unsafe {
                valued_domain_matcher_create(BorrowedSlice::from_slice(&malformed), &raw mut handle)
            },
            Status::InvalidArgument
        );
        assert_eq!(handle, 123);
    }
    assert_eq!(valued_domain_matcher_close(1), Status::Closed);
    assert_eq!(
        cache_abi_capabilities() & (1 << 4),
        1 << 4,
        "valued matcher capability must be negotiated explicitly"
    );
}

#[test]
fn valued_matcher_handles_empty_miss_invalid_output_and_wrong_namespace() {
    assert_eq!(
        unsafe {
            valued_domain_matcher_create(BorrowedSlice::from_slice(&[]), std::ptr::null_mut())
        },
        Status::InvalidArgument
    );
    assert_eq!(
        unsafe { valued_domain_matcher_len(0, std::ptr::null_mut()) },
        Status::InvalidArgument
    );
    assert_eq!(valued_domain_matcher_close(0), Status::InvalidArgument);

    let empty_batch = encode_valued_rule_batch(&[]).expect("empty valued batch");
    let mut valued_handle = 0;
    assert_eq!(
        unsafe {
            valued_domain_matcher_create(
                BorrowedSlice::from_slice(&empty_batch),
                &raw mut valued_handle,
            )
        },
        Status::Ok
    );

    let mut miss = ValuedMatchResult::empty();
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                valued_handle,
                BorrowedSlice::from_slice(b"missing.example"),
                WritableSlice::empty(),
                &raw mut miss,
            )
        },
        Status::Ok
    );
    assert_eq!(miss.status, Status::Ok);
    assert_eq!(miss.matched, 0);
    assert_eq!(miss.required_len, 0);

    let mut invalid_output = ValuedMatchResult::empty();
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                valued_handle,
                BorrowedSlice::from_slice(b"missing.example"),
                WritableSlice {
                    ptr: std::ptr::null_mut(),
                    len: 1,
                },
                &raw mut invalid_output,
            )
        },
        Status::InvalidArgument
    );
    assert_eq!(invalid_output.status, Status::InvalidArgument);

    let mut invalid_domain = ValuedMatchResult::empty();
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                valued_handle,
                BorrowedSlice::from_slice(&[0xff]),
                WritableSlice::empty(),
                &raw mut invalid_domain,
            )
        },
        Status::InvalidArgument
    );
    assert_eq!(invalid_domain.status, Status::InvalidArgument);

    let mut domain_handle = 0;
    assert_eq!(
        unsafe {
            mosdns_runtime::domain_matcher_create(
                BorrowedSlice::from_slice(b"domain:example.com"),
                0,
                &raw mut domain_handle,
            )
        },
        Status::Ok
    );
    assert_eq!(valued_domain_matcher_close(domain_handle), Status::Closed);
    let mut wrong_namespace = ValuedMatchResult::empty();
    assert_eq!(
        unsafe {
            valued_domain_matcher_match(
                domain_handle,
                BorrowedSlice::from_slice(b"example.com"),
                WritableSlice::empty(),
                &raw mut wrong_namespace,
            )
        },
        Status::Closed
    );
    assert_eq!(wrong_namespace.status, Status::Closed);
    assert_eq!(
        mosdns_runtime::domain_matcher_close(domain_handle),
        Status::Ok
    );
    assert_eq!(valued_domain_matcher_close(valued_handle), Status::Ok);
}

#[test]
fn valued_matcher_supports_concurrent_reads_during_lifecycle() {
    use std::sync::Arc;
    use std::thread;

    let batch = sample_batch();
    let mut handle = 0;
    assert_eq!(
        unsafe { valued_domain_matcher_create(BorrowedSlice::from_slice(&batch), &raw mut handle) },
        Status::Ok
    );
    let handle = Arc::new(handle);
    let workers = (0..8)
        .map(|_| {
            let handle = Arc::clone(&handle);
            thread::spawn(move || {
                for _ in 0..100 {
                    let mut output = [0_u8; 128];
                    let mut result = ValuedMatchResult::empty();
                    assert_eq!(
                        unsafe {
                            valued_domain_matcher_match(
                                *handle,
                                BorrowedSlice::from_slice(b"child.example.com"),
                                WritableSlice::from_slice(&mut output),
                                &raw mut result,
                            )
                        },
                        Status::Ok
                    );
                    assert_eq!(result.matched, 1);
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("valued reader thread");
    }
    assert_eq!(valued_domain_matcher_close(*handle), Status::Ok);
    assert_eq!(valued_domain_matcher_close(*handle), Status::Closed);
}
