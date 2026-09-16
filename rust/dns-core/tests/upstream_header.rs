use mosdns_dns_core::{HeaderError, inspect_response_header};

#[test]
fn response_header_inspection_is_minimal_and_typed() {
    assert_eq!(
        inspect_response_header(&[0; 11]),
        Err(HeaderError::TooShort)
    );

    let wire = [0xab, 0xcd, 0x82, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
    let header = inspect_response_header(&wire).expect("valid response header");
    assert_eq!(header.id, 0xabcd);
    assert!(header.qr);
    assert!(header.truncated);
}
