//! Public API smoke tests: the documented crate-root surface must be usable
//! from `mosdns_dns_core` without module-path dependencies (the Go oracle
//! atoms are re-exported so the later ABI slice depends on this crate, not on
//! module paths).

use mosdns_dns_core::{QueryError, parse_query};

fn valid_query() -> Vec<u8> {
    // id=0x1234, flags=0x10 (RD set, QR clear), opcode QUERY, qd=1.
    let mut wire = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00, 0x01]); // A IN
    wire
}

#[test]
fn crate_root_query_parse_is_usable() {
    let (header, question) = parse_query(&valid_query()).expect("valid query");
    assert!(!header.qr);
    assert_eq!(header.qdcount, 1);
    assert_eq!(question.qtype, 1);
}

#[test]
fn crate_root_query_error_is_typed() {
    // A response (QR set) must surface as Unsupported(ResponseBit) via the
    // crate-root QueryError re-export.
    let mut resp = valid_query();
    resp[2] |= 0x80;
    match parse_query(&resp) {
        Err(QueryError::Unsupported(_)) => {}
        other => panic!("expected Unsupported(ResponseBit), got {other:?}"),
    }
}
