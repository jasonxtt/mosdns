use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};

fn query(id: u16, flags: u16, qname: &[u8], qtype: u16, qclass: u16, arcount: u16) -> Vec<u8> {
    let mut wire = Vec::from([
        (id >> 8) as u8,
        u8::try_from(id & 0x00ff).expect("id low byte"),
        (flags >> 8) as u8,
        u8::try_from(flags & 0x00ff).expect("flags low byte"),
        0,
        1,
        0,
        0,
        0,
        0,
        (arcount >> 8) as u8,
        u8::try_from(arcount & 0x00ff).expect("additional count low byte"),
    ]);
    wire.extend_from_slice(qname);
    wire.extend_from_slice(&qtype.to_be_bytes());
    wire.extend_from_slice(&qclass.to_be_bytes());
    if arcount == 1 {
        wire.extend_from_slice(&[0, 0x00, 0x29, 0x04, 0xb0, 0, 0, 0, 0, 0, 0]);
    }
    wire
}

fn answer(query: &[u8], ttl: u32, rcode: u8) -> Vec<u8> {
    let (_, question) = mosdns_dns_core::parse_query(query).expect("query");
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut wire = Vec::from([
        (id >> 8) as u8,
        u8::try_from(id & 0x00ff).expect("id low byte"),
        0x81,
        0x80 | rcode,
        0,
        1,
        0,
        u8::from(rcode == 0),
        0,
        0,
        0,
        0,
    ]);
    wire.extend_from_slice(&question.qname_wire);
    wire.extend_from_slice(&question.qtype.to_be_bytes());
    wire.extend_from_slice(&question.qclass.to_be_bytes());
    if rcode == 0 {
        wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1]);
        wire.extend_from_slice(&ttl.to_be_bytes());
        wire.extend_from_slice(&[0, 4, 192, 0, 2, 1]);
    }
    wire
}

fn empty_noerror(query: &[u8]) -> Vec<u8> {
    let (header, question) = mosdns_dns_core::parse_query(query).expect("query");
    let mut wire = Vec::from([
        (header.id >> 8) as u8,
        u8::try_from(header.id & 0x00ff).expect("id low byte"),
        0x81,
        0x80,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    wire.extend_from_slice(&question.qname_wire);
    wire.extend_from_slice(&question.qtype.to_be_bytes());
    wire.extend_from_slice(&question.qclass.to_be_bytes());
    wire
}

fn response_with_opt(query: &[u8]) -> Vec<u8> {
    let mut wire = answer(query, 60, 0);
    wire[10] = 0;
    wire[11] = 1;
    wire.extend_from_slice(&[0, 0, 0x29, 0x04, 0xb0, 0, 0, 0, 0, 0, 0]);
    wire
}

fn response_with_authority_and_additional_ttls(query: &[u8]) -> Vec<u8> {
    let mut wire = answer(query, 60, 0);
    wire[8] = 0;
    wire[9] = 1;
    wire[10] = 0;
    wire[11] = 1;
    for ttl in [7_u32, 2_u32] {
        wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1]);
        wire.extend_from_slice(&ttl.to_be_bytes());
        wire.extend_from_slice(&[0, 4, 192, 0, 2, 2]);
    }
    wire
}

fn response_with_compressed_question(query: &[u8]) -> Vec<u8> {
    let (_, question) = mosdns_dns_core::parse_query(query).expect("query");
    let target = 34_u8;
    let mut wire = Vec::from([
        query[0], query[1], 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0, 0xc0, target, 0, 1, 0, 1, 0xc0,
        target, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1,
    ]);
    wire.extend_from_slice(&question.qname_wire);
    wire
}

fn name(labels: &[&str]) -> Vec<u8> {
    let mut wire = Vec::new();
    for label in labels {
        wire.push(u8::try_from(label.len()).expect("DNS label length"));
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire
}

#[test]
fn native_adapter_ages_owned_hits_without_mutating_the_entry() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock.clone()).expect("adapter");
    let first_query = query(0x1001, 0x0100, &name(&["Example", "org"]), 1, 1, 0);
    let token = adapter
        .begin_store(&first_query)
        .expect("begin store")
        .expect("cacheable query");
    assert!(token.publish(&answer(&first_query, 4, 0)).expect("publish"));

    clock.set(102);
    let second_query = query(0x1002, 0x0100, &name(&["Example", "org"]), 1, 1, 0);
    let mut first_hit = adapter
        .lookup(&second_query)
        .expect("lookup")
        .expect("fresh hit");
    assert_eq!([first_hit[0], first_hit[1]], [0x10, 0x02]);
    assert!(first_hit.windows(4).any(|window| window == [0, 0, 0, 2]));
    first_hit[0] = 0xff;

    clock.set(103);
    let second_hit = adapter
        .lookup(&second_query)
        .expect("lookup")
        .expect("second fresh hit");
    assert_eq!([second_hit[0], second_hit[1]], [0x10, 0x02]);
    assert!(second_hit.windows(4).any(|window| window == [0, 0, 0, 1]));

    clock.set(104);
    assert!(adapter.lookup(&second_query).expect("expiry").is_none());
}

#[test]
fn cache_key_separates_security_bits_type_class_and_non_in_queries() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock).expect("adapter");
    let qname = name(&["same", "example"]);
    let plain = query(1, 0x0100, &qname, 1, 1, 0);
    let ad = query(2, 0x0120, &qname, 1, 1, 0);
    let cd = query(3, 0x0110, &qname, 1, 1, 0);
    let aaaa = query(4, 0x0100, &qname, 28, 1, 0);
    let ch = query(5, 0x0100, &qname, 1, 3, 0);
    let token = adapter
        .begin_store(&plain)
        .expect("begin")
        .expect("eligible");
    assert!(token.publish(&answer(&plain, 60, 0)).expect("publish"));
    assert!(adapter.lookup(&plain).expect("plain").is_some());
    assert!(adapter.lookup(&ad).expect("ad").is_none());
    assert!(adapter.lookup(&cd).expect("cd").is_none());
    assert!(adapter.lookup(&aaaa).expect("type").is_none());
    assert!(adapter.lookup(&ch).expect("class").is_none());
    assert!(adapter.begin_store(&ch).expect("class begin").is_none());
}

#[test]
fn edns_queries_bypass_lookup_and_publication() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock).expect("adapter");
    let plain = query(1, 0x0100, &name(&["edns", "example"]), 1, 1, 0);
    let edns = query(2, 0x0100, &name(&["edns", "example"]), 1, 1, 1);
    let token = adapter
        .begin_store(&plain)
        .expect("begin")
        .expect("eligible");
    assert!(token.publish(&answer(&plain, 60, 0)).expect("publish"));
    assert!(adapter.lookup(&edns).expect("EDNS lookup").is_none());
    assert!(adapter.begin_store(&edns).expect("EDNS begin").is_none());
}

#[test]
fn compressed_query_name_reuses_the_decoded_key() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock).expect("adapter");
    let plain = query(1, 0x0100, &name(&["compressed", "example"]), 1, 1, 0);
    let mut compressed = query(2, 0x0100, &[0xc0, 0x12], 1, 1, 0);
    compressed.extend_from_slice(&name(&["compressed", "example"]));
    let token = adapter
        .begin_store(&plain)
        .expect("begin")
        .expect("eligible");
    assert!(token.publish(&answer(&plain, 60, 0)).expect("publish"));
    assert!(
        adapter
            .lookup(&compressed)
            .expect("compressed lookup")
            .is_some()
    );
}

#[test]
fn retention_is_separate_from_answer_ttl_and_expires_at_exact_boundaries() {
    let cases = [
        ("positive-short", 4_u32, 0_u8, 4_u64),
        ("positive-capped", 600_u32, 0_u8, 300_u64),
        ("nxdomain", 0_u32, 3_u8, 30_u64),
        ("upstream-servfail", 0_u32, 2_u8, 5_u64),
        ("zero-ttl", 0_u32, 0_u8, 5_u64),
    ];
    for (label, ttl, rcode, retention) in cases {
        let clock = CacheTestClock::new(100);
        let adapter = NativeCacheAdapter::for_test(clock.clone()).expect(label);
        let query = query(0x4000, 0x0100, &name(&[label, "example"]), 1, 1, 0);
        let response = answer(&query, ttl, rcode);
        let token = adapter
            .begin_store(&query)
            .expect("begin")
            .expect("eligible");
        assert!(token.publish(&response).expect("publish"));
        clock.set(100 + retention - 1);
        assert!(adapter.lookup(&query).expect("before expiry").is_some());
        clock.set(100 + retention);
        assert!(adapter.lookup(&query).expect("at expiry").is_none());
    }

    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock.clone()).expect("empty");
    let query = query(0x4001, 0x0100, &name(&["empty", "example"]), 1, 1, 0);
    let token = adapter
        .begin_store(&query)
        .expect("begin")
        .expect("eligible");
    assert!(
        token
            .publish(&empty_noerror(&query))
            .expect("empty publish")
    );
    clock.set(104);
    assert!(
        adapter
            .lookup(&query)
            .expect("empty before expiry")
            .is_some()
    );
    clock.set(105);
    assert!(adapter.lookup(&query).expect("empty expiry").is_none());
}

#[test]
fn malformed_tc_opt_and_mismatched_questions_never_publish() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock).expect("adapter");
    let primary = query(0x5000, 0x0100, &name(&["gate", "example"]), 1, 1, 0);

    let tc = {
        let mut wire = answer(&primary, 60, 0);
        wire[2] |= 0x02;
        wire
    };
    let token = adapter
        .begin_store(&primary)
        .expect("TC begin")
        .expect("eligible");
    assert!(!token.publish(&tc).expect("TC admission"));
    assert!(adapter.lookup(&primary).expect("TC lookup").is_none());

    let token = adapter
        .begin_store(&primary)
        .expect("OPT begin")
        .expect("eligible");
    assert!(
        !token
            .publish(&response_with_opt(&primary))
            .expect("OPT admission")
    );
    assert!(adapter.lookup(&primary).expect("OPT lookup").is_none());

    let other_query = query(0x5001, 0x0100, &name(&["other", "example"]), 1, 1, 0);
    let token = adapter
        .begin_store(&primary)
        .expect("mismatch begin")
        .expect("eligible");
    assert!(
        !token
            .publish(&answer(&other_query, 60, 0))
            .expect("mismatch admission")
    );
    assert!(adapter.lookup(&primary).expect("mismatch lookup").is_none());

    let question_offset = 12 + name(&["gate", "example"]).len();
    for (label, offset, value) in [
        ("qtype", question_offset, 28_u8),
        ("qclass", question_offset + 2, 3_u8),
    ] {
        let mut wrong = answer(&primary, 60, 0);
        wrong[offset] = value;
        let token = adapter
            .begin_store(&primary)
            .expect(label)
            .expect("eligible");
        assert!(!token.publish(&wrong).expect("field mismatch admission"));
        assert!(
            adapter
                .lookup(&primary)
                .expect("field mismatch lookup")
                .is_none()
        );
    }

    let mut missing_question = answer(&primary, 60, 0);
    missing_question[4] = 0;
    missing_question[5] = 0;
    let token = adapter
        .begin_store(&primary)
        .expect("missing begin")
        .expect("eligible");
    assert!(
        !token
            .publish(&missing_question)
            .expect("missing question admission")
    );

    let mut non_query = answer(&primary, 60, 0);
    non_query[2] = 0x89;
    let token = adapter
        .begin_store(&primary)
        .expect("opcode begin")
        .expect("eligible");
    assert!(!token.publish(&non_query).expect("opcode admission"));
}

#[test]
fn ordinary_authority_and_additional_ttls_set_the_minimum_retention() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock.clone()).expect("adapter");
    let query = query(0x6000, 0x0100, &name(&["minimum", "example"]), 1, 1, 0);
    let token = adapter
        .begin_store(&query)
        .expect("begin")
        .expect("eligible");
    assert!(
        token
            .publish(&response_with_authority_and_additional_ttls(&query))
            .expect("publish")
    );
    clock.set(101);
    assert!(adapter.lookup(&query).expect("before minimum").is_some());
    clock.set(102);
    assert!(adapter.lookup(&query).expect("at minimum").is_none());
}

#[test]
fn compressed_response_question_is_eligible_after_decoding() {
    let clock = CacheTestClock::new(100);
    let adapter = NativeCacheAdapter::for_test(clock).expect("adapter");
    let query = query(0x6001, 0x0100, &name(&["compressed", "response"]), 1, 1, 0);
    let token = adapter
        .begin_store(&query)
        .expect("begin")
        .expect("eligible");
    assert!(
        token
            .publish(&response_with_compressed_question(&query))
            .expect("publish")
    );
    assert!(adapter.lookup(&query).expect("lookup").is_some());
}
