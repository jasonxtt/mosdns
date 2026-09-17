//! Public contract tests for the pure bootstrap DNS wire codec
//! (`mosdns_dns_core::resolver`).
//!
//! Slice 0 of the endpoint-resolution task: query construction, EDNS(0)
//! advertisement, injected query IDs, exact response correlation, bounded
//! record walking, CNAME chain policy, address selection, and effective TTL.
//! No sockets, no clocks, no external DNS.

use std::net::IpAddr;

use mosdns_dns_core::{
    AddressFamily, CnameChainPolicy, QueryIdSource, RESOLVER_DEFAULT_MAX_CNAME_LINKS,
    RESOLVER_MAX_CNAME_LINKS, RESOLVER_UDP_PAYLOAD_SIZE, ResolverWireError, SelectedAddress,
    build_resolver_query, extract_edns_at, parse_query, parse_resolver_response,
};

const HEADER_LEN: usize = 12;
const TYPE_A: u16 = 1;
const TYPE_CNAME: u16 = 5;
const TYPE_AAAA: u16 = 28;
const CLASS_IN: u16 = 1;

const QNAME: [&str; 3] = ["bootstrap", "example", "org"];

// ---------------------------------------------------------------------------
// Wire fixtures
// ---------------------------------------------------------------------------

fn name(labels: &[&str]) -> Vec<u8> {
    let mut wire = Vec::new();
    for label in labels {
        wire.push(u8::try_from(label.len()).expect("label fits"));
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire
}

/// A 14-bit compression pointer to `offset`.
fn ptr(offset: usize) -> Vec<u8> {
    let high = u8::try_from(offset >> 8).expect("offset fits the 14-bit pointer field");
    vec![0xc0 | high, u8::try_from(offset & 0xff).expect("low byte")]
}

fn rr(owner: &[u8], rrtype: u16, ttl: u32, rdata: &[u8]) -> Vec<u8> {
    let mut wire = owner.to_vec();
    wire.extend_from_slice(&rrtype.to_be_bytes());
    wire.extend_from_slice(&CLASS_IN.to_be_bytes());
    wire.extend_from_slice(&ttl.to_be_bytes());
    wire.extend_from_slice(
        &u16::try_from(rdata.len())
            .expect("rdata fits")
            .to_be_bytes(),
    );
    wire.extend_from_slice(rdata);
    wire
}

fn a_rdata(ip: [u8; 4]) -> Vec<u8> {
    ip.to_vec()
}

fn aaaa_rdata(ip: [u8; 16]) -> Vec<u8> {
    ip.to_vec()
}

fn question(labels: &[&str], qtype: u16) -> Vec<u8> {
    let mut wire = name(labels);
    wire.extend_from_slice(&qtype.to_be_bytes());
    wire.extend_from_slice(&CLASS_IN.to_be_bytes());
    wire
}

/// Offset of the first answer record for `QNAME` responses.
fn first_answer_offset() -> usize {
    HEADER_LEN + name(&QNAME).len() + 4
}

/// Offset of the first CNAME's rdata (the second name in the answer section).
fn first_cname_rdata_offset() -> usize {
    // owner(2) + type/class/ttl/rdlength(10)
    first_answer_offset() + 12
}

const FLAG_QR: u16 = 0x8000;
const FLAG_TC: u16 = 0x0200;

fn response(
    id: u16,
    flags: u16,
    question_wire: &[u8],
    answers: &[Vec<u8>],
    authorities: &[Vec<u8>],
    extras: &[Vec<u8>],
) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&flags.to_be_bytes());
    wire.extend_from_slice(&u16::from(!question_wire.is_empty()).to_be_bytes());
    wire.extend_from_slice(&u16::try_from(answers.len()).expect("an fits").to_be_bytes());
    wire.extend_from_slice(
        &u16::try_from(authorities.len())
            .expect("ns fits")
            .to_be_bytes(),
    );
    wire.extend_from_slice(&u16::try_from(extras.len()).expect("ar fits").to_be_bytes());
    wire.extend_from_slice(question_wire);
    for section in [answers, authorities, extras] {
        for record in section {
            wire.extend_from_slice(record);
        }
    }
    wire
}

/// A correlated NOERROR response with the standard question section.
fn correlated(
    id: u16,
    flags: u16,
    answers: &[Vec<u8>],
    authorities: &[Vec<u8>],
    extras: &[Vec<u8>],
) -> Vec<u8> {
    response(
        id,
        FLAG_QR | flags,
        &question(&QNAME, TYPE_A),
        answers,
        authorities,
        extras,
    )
}

/// Query name copied out of a built query so response tests use the same bytes
/// callers obtain from `parse_query`.
fn qname_wire() -> Vec<u8> {
    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("query builds");
    parse_query(&query).expect("query parses").1.qname_wire
}

/// Deterministic injected query-ID source.
struct SequenceIds {
    next: u16,
}

impl SequenceIds {
    const fn new() -> Self {
        Self { next: 1 }
    }
}

impl QueryIdSource for SequenceIds {
    fn next_id(&mut self) -> u16 {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        id
    }
}

fn parse(
    packet: &[u8],
    family: AddressFamily,
    qname: &[u8],
    id: u16,
) -> Result<SelectedAddress, ResolverWireError> {
    parse_resolver_response(packet, family, qname, id, &CnameChainPolicy::default())
}

fn expect_ok(packet: &[u8], family: AddressFamily, qname: &[u8], id: u16) -> SelectedAddress {
    parse(packet, family, qname, id).unwrap_or_else(|err| panic!("expected success, got {err:?}"))
}

// ---------------------------------------------------------------------------
// Query construction
// ---------------------------------------------------------------------------

#[test]
fn builds_a_query_for_normalized_fqdn() {
    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");

    let (header, parsed) = parse_query(&query).expect("valid query shape");
    assert_eq!(header.id, 1);
    assert!(!header.qr);
    assert_eq!(header.opcode, 0);
    assert_eq!(header.qdcount, 1);
    assert_eq!(header.ancount, 0);
    assert_eq!(header.nscount, 0);
    assert_eq!(header.arcount, 1, "exactly one OPT record");

    assert_eq!(parsed.qname_wire, name(&QNAME));
    assert_eq!(parsed.qtype, TYPE_A);
    assert_eq!(parsed.qclass, CLASS_IN);

    // RD must be set and QR clear in the raw flags word.
    let flags = u16::from_be_bytes([query[2], query[3]]);
    assert_eq!(flags & FLAG_QR, 0);
    assert_ne!(flags & 0x0100, 0, "RD set");
}

#[test]
fn builds_aaaa_query_for_ipv6_family() {
    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv6, &mut ids)
        .expect("built");
    let (_, parsed) = parse_query(&query).expect("valid query shape");
    assert_eq!(parsed.qtype, TYPE_AAAA);
}

#[test]
fn appends_root_opt_advertising_1200_without_do_bit() {
    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");

    let extra_offset = HEADER_LEN + name(&QNAME).len() + 4;
    assert_eq!(query[extra_offset], 0x00, "OPT owner is the root label");
    let edns = extract_edns_at(&query, extra_offset)
        .expect("extra record parses")
        .expect("OPT present");
    assert!(edns.has_opt);
    assert_eq!(edns.udp_size, RESOLVER_UDP_PAYLOAD_SIZE);
    assert_eq!(edns.udp_size, 1200);
    assert!(!edns.do_bit, "DNSSEC OK bit is not advertised");
    assert!(edns.ecs.is_none(), "no client subnet is added");
}

#[test]
fn uses_injected_query_ids_deterministically() {
    let mut ids = SequenceIds::new();
    let first = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");
    let second = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");
    assert_eq!(u16::from_be_bytes([first[0], first[1]]), 1);
    assert_eq!(u16::from_be_bytes([second[0], second[1]]), 2);
    // Everything except the ID is byte-identical for the same input.
    assert_eq!(first[2..], second[2..]);
}

#[test]
fn query_encoding_is_stable_across_calls() {
    let mut ids = SequenceIds::new();
    let mut expected = vec![
        0x00, 0x01, // id
        0x01, 0x00, // RD, QR clear, opcode QUERY, rcode 0
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    ];
    expected.extend_from_slice(&name(&["example", "org"]));
    expected.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
    expected.extend_from_slice(&[
        0x00, // root OPT owner
        0x00, 0x29, // type OPT
        0x04, 0xb0, // 1200
        0x00, 0x00, 0x00, 0x00, // extended rcode/version/flags
        0x00, 0x00, // no options
    ]);

    let query = build_resolver_query("example.org", AddressFamily::Ipv4, &mut ids).expect("built");
    assert_eq!(query, expected);
}

#[test]
fn rejects_empty_and_overlong_names() {
    let mut ids = SequenceIds::new();
    for bad in [
        "",
        ".",
        "example..org",
        "-example.org",
        "example-.org",
        "exa mple.org",
    ] {
        assert!(
            build_resolver_query(bad, AddressFamily::Ipv4, &mut ids).is_err(),
            "{bad:?} must be rejected"
        );
    }

    let long_label = "a".repeat(64);
    assert!(build_resolver_query(&long_label, AddressFamily::Ipv4, &mut ids).is_err());

    // 4 * 63 + separators = 255 octets > 253.
    let long_name = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(63),
    ]
    .join(".");
    assert!(build_resolver_query(&long_name, AddressFamily::Ipv4, &mut ids).is_err());

    // The trailing root dot is accepted and normalized away.
    let dotted = build_resolver_query("bootstrap.example.org.", AddressFamily::Ipv4, &mut ids)
        .expect("built");
    let (_, parsed) = parse_query(&dotted).expect("valid query shape");
    assert_eq!(parsed.qname_wire, name(&QNAME), "one trailing dot removed");
}

// ---------------------------------------------------------------------------
// Successful correlation and selection
// ---------------------------------------------------------------------------

#[test]
fn accepts_matching_a_answer() {
    let packet = correlated(
        0x1234,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 0x1234);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 1]));
    assert_eq!(selected.family, AddressFamily::Ipv4);
    assert_eq!(selected.ttl, 600);
    assert_eq!(selected.cname_chain_len, 0);
}

#[test]
fn accepts_matching_aaaa_answer() {
    let packet = response(
        0x1234,
        FLAG_QR,
        &question(&QNAME, TYPE_AAAA),
        &[rr(
            &ptr(HEADER_LEN),
            TYPE_AAAA,
            900,
            &aaaa_rdata([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        )],
        &[],
        &[],
    );
    let selected = expect_ok(&packet, AddressFamily::Ipv6, &qname_wire(), 0x1234);
    assert_eq!(
        selected.address,
        "2001:db8::1".parse::<IpAddr>().expect("valid ip")
    );
    assert_eq!(selected.family, AddressFamily::Ipv6);
    assert_eq!(selected.ttl, 900);
}

#[test]
fn selects_first_matching_address_in_wire_order() {
    // An unrelated owner first, then two matching A records: the first
    // matching address in wire order wins.
    let unrelated = rr(
        &name(&["other", "example", "org"]),
        TYPE_A,
        10,
        &a_rdata([203, 0, 113, 9]),
    );
    let packet = correlated(
        7,
        0,
        &[
            unrelated,
            rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1])),
            rr(&ptr(HEADER_LEN), TYPE_A, 20, &a_rdata([192, 0, 2, 2])),
        ],
        &[],
        &[],
    );
    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 7);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 1]));
    assert_eq!(selected.ttl, 600, "the unrelated record's TTL is not used");
}

#[test]
fn selects_the_first_matching_address_even_when_a_later_one_differs() {
    // The first match has a long TTL and the second a short one; selecting by
    // TTL or by "last wins" would produce a different observable result.
    let packet = correlated(
        8,
        0,
        &[
            rr(&ptr(HEADER_LEN), TYPE_A, 900, &a_rdata([192, 0, 2, 11])),
            rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 22])),
        ],
        &[],
        &[],
    );
    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 8);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 11]));
    assert_eq!(selected.ttl, 900, "the selected record's own TTL");
}

#[test]
fn an_owner_outside_the_chain_never_selects() {
    // A malformed owner (a pointer running off the end of the packet) fails the
    // message; a well-formed owner that is simply not the question name or a
    // chain link is ignored and leaves the response without a usable answer.
    let qname = qname_wire();

    let off_packet = correlated(
        0x12,
        0,
        &[rr(&[0xc0, 0xff], TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&off_packet, AddressFamily::Ipv4, &qname, 0x12),
        Err(ResolverWireError::Malformed)
    );

    let root_owner = correlated(
        0x12,
        0,
        &[rr(&[0x00], TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&root_owner, AddressFamily::Ipv4, &qname, 0x12),
        Err(ResolverWireError::NoUsableAnswer)
    );
}

#[test]
fn follows_bounded_cname_chain_and_takes_minimum_ttl() {
    // QNAME -> CNAME mid.example.org (ttl 60) -> A 192.0.2.5 (ttl 600). The
    // CNAME is the shorter-lived link, so the raw minimum is 60 and the floor
    // lifts it to 300; using the address TTL alone would report 600.
    let mut answers = vec![rr(
        &ptr(HEADER_LEN),
        TYPE_CNAME,
        60,
        &name(&["mid", "example", "org"]),
    )];
    answers.push(rr(
        &ptr(first_cname_rdata_offset()),
        TYPE_A,
        600,
        &a_rdata([192, 0, 2, 5]),
    ));
    let packet = correlated(0x2222, 0, &answers, &[], &[]);

    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 0x2222);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 5]));
    assert_eq!(
        selected.ttl, 300,
        "min over the used CNAME and address, clamped"
    );
    assert_eq!(selected.cname_chain_len, 1);

    // The mirror case: the address is the shorter-lived link.
    let mut mirror = vec![rr(
        &ptr(HEADER_LEN),
        TYPE_CNAME,
        900,
        &name(&["mid", "example", "org"]),
    )];
    mirror.push(rr(
        &ptr(first_cname_rdata_offset()),
        TYPE_A,
        420,
        &a_rdata([192, 0, 2, 6]),
    ));
    let packet = correlated(0x2223, 0, &mirror, &[], &[]);
    assert_eq!(
        expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 0x2223).ttl,
        420
    );
}

#[test]
fn accepts_uncompressed_owner_equal_to_qname() {
    let packet = correlated(
        3,
        0,
        &[rr(&name(&QNAME), TYPE_A, 600, &a_rdata([198, 51, 100, 7]))],
        &[],
        &[],
    );
    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname_wire(), 3);
    assert_eq!(selected.address, IpAddr::from([198, 51, 100, 7]));
}

#[test]
fn effective_ttl_is_clamped_at_both_bounds() {
    let policy = CnameChainPolicy::default();
    assert_eq!(policy.min_ttl(), 300);
    assert_eq!(policy.max_ttl(), 604_800);
    assert_eq!(policy.max_cname_links(), RESOLVER_DEFAULT_MAX_CNAME_LINKS);

    let low = correlated(
        4,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 60, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        expect_ok(&low, AddressFamily::Ipv4, &qname_wire(), 4).ttl,
        300,
        "clamped up to the five-minute floor"
    );

    let high = correlated(
        5,
        0,
        &[rr(
            &ptr(HEADER_LEN),
            TYPE_A,
            0x00ff_ffff,
            &a_rdata([192, 0, 2, 1]),
        )],
        &[],
        &[],
    );
    assert_eq!(
        expect_ok(&high, AddressFamily::Ipv4, &qname_wire(), 5).ttl,
        604_800,
        "clamped down to the seven-day ceiling"
    );

    let exact = correlated(
        6,
        0,
        &[rr(
            &ptr(HEADER_LEN),
            TYPE_A,
            604_800,
            &a_rdata([192, 0, 2, 1]),
        )],
        &[],
        &[],
    );
    assert_eq!(
        expect_ok(&exact, AddressFamily::Ipv4, &qname_wire(), 6).ttl,
        604_800
    );
}

// ---------------------------------------------------------------------------
// Correlation failures
// ---------------------------------------------------------------------------

#[test]
fn rejects_mismatched_id() {
    let packet = correlated(
        0x1234,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&packet, AddressFamily::Ipv4, &qname_wire(), 0x1235),
        Err(ResolverWireError::MismatchedId)
    );
}

#[test]
fn rejects_question_mismatch() {
    let qname = qname_wire();
    let answer = rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]));

    let other_name = response(
        9,
        FLAG_QR,
        &question(&["other", "example", "org"], TYPE_A),
        std::slice::from_ref(&answer),
        &[],
        &[],
    );
    assert_eq!(
        parse(&other_name, AddressFamily::Ipv4, &qname, 9),
        Err(ResolverWireError::QuestionMismatch)
    );

    let other_type = response(
        9,
        FLAG_QR,
        &question(&QNAME, TYPE_AAAA),
        std::slice::from_ref(&answer),
        &[],
        &[],
    );
    assert_eq!(
        parse(&other_type, AddressFamily::Ipv4, &qname, 9),
        Err(ResolverWireError::QuestionMismatch)
    );

    // The question section is echoed in its original case; comparison is
    // case-insensitive.
    let upper = response(
        9,
        FLAG_QR,
        &question(&["BOOTSTRAP", "EXAMPLE", "ORG"], TYPE_A),
        std::slice::from_ref(&answer),
        &[],
        &[],
    );
    let selected = expect_ok(&upper, AddressFamily::Ipv4, &qname, 9);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 1]));

    // A response with no question section cannot be correlated.
    let no_question = response(9, FLAG_QR, &[], &[answer], &[], &[]);
    assert_eq!(
        parse(&no_question, AddressFamily::Ipv4, &qname, 9),
        Err(ResolverWireError::QuestionMismatch)
    );

    // A newline-bearing name must not be normalized into a match.
    let control = "bootstra\n.example.org";
    let mut ids = SequenceIds::new();
    assert!(build_resolver_query(control, AddressFamily::Ipv4, &mut ids).is_err());
}

#[test]
fn rejects_query_packets_and_non_query_opcodes() {
    let qname = qname_wire();
    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");
    assert_eq!(
        parse(&query, AddressFamily::Ipv4, &qname, 1),
        Err(ResolverWireError::NotAResponse)
    );

    // IQUERY (opcode 1) with QR set is not a standard QUERY response.
    let iquery = correlated(
        1,
        0x0800,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&iquery, AddressFamily::Ipv4, &qname, 1),
        Err(ResolverWireError::UnexpectedOpcode(1))
    );
}

#[test]
fn rejects_non_noerror_rcodes() {
    let qname = qname_wire();
    // FORMERR(1), SERVFAIL(2), NXDOMAIN(3), NOTIMP(4), REFUSED(5).
    for rcode in [1u16, 2, 3, 4, 5] {
        let packet = correlated(0x4321, rcode, &[], &[], &[]);
        assert_eq!(
            parse(&packet, AddressFamily::Ipv4, &qname, 0x4321),
            Err(ResolverWireError::Rcode(u8::try_from(rcode).expect("fits"))),
            "rcode {rcode}"
        );
    }
}

#[test]
fn rejects_truncated_flagged_responses() {
    let qname = qname_wire();
    let packet = correlated(
        0x5555,
        FLAG_TC,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&packet, AddressFamily::Ipv4, &qname, 0x5555),
        Err(ResolverWireError::Truncated)
    );
}

// ---------------------------------------------------------------------------
// Answer shape failures
// ---------------------------------------------------------------------------

#[test]
fn rejects_wrong_family_answers() {
    let qname = qname_wire();
    let aaaa_only = response(
        0x0a,
        FLAG_QR,
        &question(&QNAME, TYPE_A),
        &[rr(
            &ptr(HEADER_LEN),
            TYPE_AAAA,
            600,
            &aaaa_rdata([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        )],
        &[],
        &[],
    );
    assert_eq!(
        parse(&aaaa_only, AddressFamily::Ipv4, &qname, 0x0a),
        Err(ResolverWireError::NoUsableAnswer)
    );

    let a_only = response(
        0x0a,
        FLAG_QR,
        &question(&QNAME, TYPE_AAAA),
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&a_only, AddressFamily::Ipv6, &qname, 0x0a),
        Err(ResolverWireError::NoUsableAnswer)
    );
}

#[test]
fn rejects_empty_and_cname_only_answers() {
    let qname = qname_wire();

    let empty = correlated(0x0b, 0, &[], &[], &[]);
    assert_eq!(
        parse(&empty, AddressFamily::Ipv4, &qname, 0x0b),
        Err(ResolverWireError::NoUsableAnswer)
    );

    // A well-formed CNAME chain that never reaches an address is NoData; this
    // codec does not issue a second query.
    let only_cname = correlated(
        0x0b,
        0,
        &[rr(
            &ptr(HEADER_LEN),
            TYPE_CNAME,
            900,
            &name(&["mid", "example", "org"]),
        )],
        &[],
        &[],
    );
    assert_eq!(
        parse(&only_cname, AddressFamily::Ipv4, &qname, 0x0b),
        Err(ResolverWireError::NoUsableAnswer)
    );
}

#[test]
fn rejects_cname_loops_and_overlong_chains() {
    let qname = qname_wire();

    // Self-referential CNAME target: QNAME -> CNAME pointing at QNAME.
    let self_referential = correlated(
        0x0c,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_CNAME, 60, &ptr(HEADER_LEN))],
        &[],
        &[],
    );
    assert_eq!(
        parse(&self_referential, AddressFamily::Ipv4, &qname, 0x0c),
        Err(ResolverWireError::InvalidCnameChain)
    );

    // Two-link cycle: QNAME -> b, b -> QNAME. The second record's owner sits at
    // the end of the first record's rdata, and its target points back at the
    // first record's owner.
    let b_off = first_answer_offset() + 12; // the second record owns the first CNAME's target
    let cycle = correlated(
        0x0c,
        0,
        &[
            rr(
                &ptr(HEADER_LEN),
                TYPE_CNAME,
                60,
                &name(&["b", "example", "org"]),
            ),
            rr(&ptr(b_off), TYPE_CNAME, 60, &ptr(HEADER_LEN)),
        ],
        &[],
        &[],
    );
    assert_eq!(
        parse(&cycle, AddressFamily::Ipv4, &qname, 0x0c),
        Err(ResolverWireError::InvalidCnameChain)
    );
}

/// Builds a response whose answer section holds `links` CNAMEs rooted at the
/// question name and ending in one A record.
fn cname_chain(links: usize) -> Vec<u8> {
    let mut answers = Vec::new();
    // Each record's owner points at the previous record's rdata (the QNAME for
    // the first link), so the chain is linked in wire order.
    let mut start = first_answer_offset();
    let mut previous_rdata = HEADER_LEN; // the QNAME the chain is rooted at
    for index in 0..links {
        let label = format!("n{index}");
        let labels = [label.as_str(), "example", "org"];
        let target = name(&labels);
        answers.push(rr(&ptr(previous_rdata), TYPE_CNAME, 900, &target));
        previous_rdata = start + 12;
        start += 12 + target.len();
    }
    answers.push(rr(
        &ptr(previous_rdata),
        TYPE_A,
        600,
        &a_rdata([192, 0, 2, 77]),
    ));
    correlated(0x0d, 0, &answers, &[], &[])
}

#[test]
fn enforces_the_chain_length_bound() {
    let qname = qname_wire();

    let default_links = usize::from(RESOLVER_DEFAULT_MAX_CNAME_LINKS);
    let at_bound = cname_chain(default_links);
    let selected = expect_ok(&at_bound, AddressFamily::Ipv4, &qname, 0x0d);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 77]));
    assert_eq!(selected.cname_chain_len, RESOLVER_DEFAULT_MAX_CNAME_LINKS);
    assert_eq!(selected.ttl, 600, "min over the whole chain");

    let over_bound = cname_chain(default_links + 1);
    assert_eq!(
        parse(&over_bound, AddressFamily::Ipv4, &qname, 0x0d),
        Err(ResolverWireError::InvalidCnameChain)
    );

    // A stricter reviewed policy rejects a chain the default accepts.
    let strict = CnameChainPolicy::new(1, 300, 604_800).expect("valid policy");
    let two_links = cname_chain(2);
    assert_eq!(
        parse_resolver_response(&two_links, AddressFamily::Ipv4, &qname, 0x0d, &strict),
        Err(ResolverWireError::InvalidCnameChain)
    );
    let one_link = cname_chain(1);
    assert!(parse_resolver_response(&one_link, AddressFamily::Ipv4, &qname, 0x0d, &strict).is_ok());

    // Policy construction is bound-checked and ordered.
    assert!(
        CnameChainPolicy::new(RESOLVER_MAX_CNAME_LINKS, 300, 604_800).is_err(),
        "the maximum accepted link count stays below the hard bound"
    );
    assert!(CnameChainPolicy::new(1, 0, 604_800).is_err(), "min ttl > 0");
    assert!(CnameChainPolicy::new(1, 600, 300).is_err(), "min <= max");
}

#[test]
fn rejects_a_dangling_cname_in_the_chain() {
    let qname = qname_wire();
    // The A record's owner is not the CNAME target, so the chain never
    // reaches an address belonging to QNAME.
    let packet = correlated(
        0x0e,
        0,
        &[
            rr(
                &ptr(HEADER_LEN),
                TYPE_CNAME,
                900,
                &name(&["mid", "example", "org"]),
            ),
            rr(
                &name(&["other", "example", "org"]),
                TYPE_A,
                600,
                &a_rdata([192, 0, 2, 1]),
            ),
        ],
        &[],
        &[],
    );
    assert_eq!(
        parse(&packet, AddressFamily::Ipv4, &qname, 0x0e),
        Err(ResolverWireError::NoUsableAnswer)
    );
}

#[test]
fn ignores_unrelated_authority_records_and_trailing_bytes() {
    let qname = qname_wire();
    let mut packet = correlated(
        0x0f,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[rr(
            &name(&["ns", "example", "org"]),
            TYPE_A,
            30,
            &a_rdata([203, 0, 113, 1]),
        )],
        &[],
    );
    packet.extend_from_slice(&[0xff, 0x00]);
    let selected = expect_ok(&packet, AddressFamily::Ipv4, &qname, 0x0f);
    assert_eq!(selected.address, IpAddr::from([192, 0, 2, 1]));
    assert_eq!(selected.ttl, 600, "authority TTL is not used");
}

#[test]
fn rejects_malformed_message_bounds() {
    let qname = qname_wire();
    let good_answer = rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]));

    let mut truncated_header = correlated(0x10, 0, std::slice::from_ref(&good_answer), &[], &[]);
    truncated_header.truncate(11);

    let mut truncated_question = correlated(0x10, 0, std::slice::from_ref(&good_answer), &[], &[]);
    truncated_question[4..6].copy_from_slice(&1u16.to_be_bytes());
    truncated_question.truncate(HEADER_LEN + 2);

    // Declared RDLENGTH overruns the packet.
    let overrun_rr = rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]));
    let mut overrun = correlated(0x10, 0, &[overrun_rr], &[], &[]);
    let rdlen_offset = overrun.len() - 4 - 2;
    overrun[rdlen_offset..rdlen_offset + 2].copy_from_slice(&600u16.to_be_bytes());

    // Two answers declared, one present.
    let mut missing = correlated(0x10, 0, std::slice::from_ref(&good_answer), &[], &[]);
    missing[6..8].copy_from_slice(&2u16.to_be_bytes());

    // An authority record whose declared RDLENGTH overruns the packet still
    // fails the message even though authorities are otherwise ignored.
    let authority_overrun = correlated(
        0x10,
        0,
        std::slice::from_ref(&good_answer),
        &[rr(
            &name(&["ns", "example", "org"]),
            TYPE_A,
            30,
            &a_rdata([203, 0, 113, 1]),
        )],
        &[],
    );
    let authority_rdlen = authority_overrun.len() - 4 - 2;
    let mut authority_overrun = authority_overrun;
    authority_overrun[authority_rdlen..authority_rdlen + 2].copy_from_slice(&900u16.to_be_bytes());

    // Answer owner pointer outside the packet.
    let bad_pointer = correlated(
        0x10,
        0,
        &[rr(&[0xc0, 0xff], TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );

    // Illegal label length in an owner name.
    let mut bad_label = vec![0x40];
    bad_label.extend_from_slice(&[0; 64]);
    let bad_label = correlated(
        0x10,
        0,
        &[rr(&bad_label, TYPE_A, 600, &a_rdata([1, 2, 3, 4]))],
        &[],
        &[],
    );

    for (label, packet) in [
        ("truncated header", truncated_header),
        ("truncated question", truncated_question),
        ("rdlength overrun", overrun),
        ("missing declared answer", missing),
        ("owner pointer out of packet", bad_pointer),
        ("illegal label length", bad_label),
        ("authority rdlength overrun", authority_overrun),
    ] {
        assert_eq!(
            parse(&packet, AddressFamily::Ipv4, &qname, 0x10),
            Err(ResolverWireError::Malformed),
            "{label}"
        );
    }

    // An exactly-sized A rdata is required; 3 or 5 bytes are not an address.
    for wrong in [vec![192, 0, 2], vec![192, 0, 2, 1, 9]] {
        let packet = correlated(
            0x10,
            0,
            &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &wrong)],
            &[],
            &[],
        );
        assert_eq!(
            parse(&packet, AddressFamily::Ipv4, &qname, 0x10),
            Err(ResolverWireError::Malformed),
            "A rdata of {} bytes",
            wrong.len()
        );
    }

    for wrong in [vec![0u8; 15], vec![0u8; 17], Vec::new()] {
        // The question is echoed with the AAAA type the caller asked for; only
        // the answer's rdata length is wrong.
        let packet = response(
            0x10,
            FLAG_QR,
            &question(&QNAME, TYPE_AAAA),
            &[rr(&ptr(HEADER_LEN), TYPE_AAAA, 600, &wrong)],
            &[],
            &[],
        );
        assert_eq!(
            parse(&packet, AddressFamily::Ipv6, &qname, 0x10),
            Err(ResolverWireError::Malformed),
            "AAAA rdata of {} bytes",
            wrong.len()
        );
    }
}

#[test]
fn parses_are_pure_and_do_not_mutate_input() {
    let qname = qname_wire();
    let packet = correlated(
        0x11,
        0,
        &[rr(&ptr(HEADER_LEN), TYPE_A, 600, &a_rdata([192, 0, 2, 1]))],
        &[],
        &[],
    );
    let before = packet.clone();
    assert!(parse(&packet, AddressFamily::Ipv4, &qname, 0x11).is_ok());
    assert_eq!(packet, before);

    let mut ids = SequenceIds::new();
    let query = build_resolver_query("bootstrap.example.org", AddressFamily::Ipv4, &mut ids)
        .expect("built");
    let query_before = query.clone();
    let _ = parse_query(&query).expect("valid");
    assert_eq!(query, query_before);
}
