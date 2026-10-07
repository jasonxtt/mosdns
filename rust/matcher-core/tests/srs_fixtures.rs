//! The SRS decoders against files written by the reference sing library
//! (`tools/m1a1-oracle/mksrs`).

use mosdns_matcher_core::{decode_domain_srs, decode_ip_srs};

const DOMAINS: &[u8] =
    include_bytes!("../../native-host/tests/fixtures/m1_a1/tree/srs/domains.srs");
const IPS: &[u8] = include_bytes!("../../native-host/tests/fixtures/m1_a1/tree/srs/ips.srs");

#[test]
fn domain_srs_yields_full_suffix_keyword_and_regexp_rules() {
    let all = decode_domain_srs(DOMAINS, true).unwrap();
    assert!(all.complete);
    assert_eq!(
        all.rules,
        [
            "full:exact.srs.example",
            "full:other.srs.example",
            "domain:example.cn",
            "domain:suffix.srs.example",
            "keyword:kwsrs",
            r"regexp:^re[0-9]+\.srs\.example$",
        ]
    );
    let without = decode_domain_srs(DOMAINS, false).unwrap();
    assert_eq!(without.rules.len(), all.rules.len() - 1);
    assert!(
        without
            .rules
            .iter()
            .all(|rule| !rule.starts_with("regexp:"))
    );
}

#[test]
fn ip_srs_yields_the_minimal_prefix_cover_of_each_range() {
    let got = decode_ip_srs(IPS).unwrap();
    assert!(got.complete);
    let text: Vec<String> = got
        .prefixes
        .iter()
        .map(|(address, bits)| format!("{address}/{bits}"))
        .collect();
    assert_eq!(
        text,
        [
            "10.0.0.0/24",
            "192.0.2.5/32",
            "192.0.2.6/31",
            "192.0.2.8/31",
            "2001:db8::/112",
        ]
    );
}

#[test]
fn every_truncation_of_a_valid_file_is_bounded() {
    for bytes in [DOMAINS, IPS] {
        for cut in 0..bytes.len() {
            // Never panics or hangs; short files are rejected or incomplete.
            let _ = decode_domain_srs(&bytes[..cut], true);
            let _ = decode_ip_srs(&bytes[..cut]);
        }
    }
}

#[test]
fn flipping_any_byte_never_panics() {
    for bytes in [DOMAINS, IPS] {
        for index in 0..bytes.len() {
            let mut damaged = bytes.to_vec();
            damaged[index] ^= 0xff;
            let _ = decode_domain_srs(&damaged, true);
            let _ = decode_ip_srs(&damaged);
        }
    }
}
