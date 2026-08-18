use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_sequence_core::{
    DnsResponseInspector, ExecutionState, OwnedResponseWire, ResponseError, ResponseState,
    RoutingState, SynthesizedResponse,
};

fn state() -> ExecutionState {
    ExecutionState::new(
        QueryHeader {
            id: 0x1234,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        QuestionInfo {
            qname_wire: vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0],
            qtype: 1,
            qclass: 1,
        },
    )
}

fn complete_response_wire() -> Vec<u8> {
    let mut wire = vec![
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01,
    ];
    wire.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0, 0, 1, 0, 1]);
    wire.extend_from_slice(&[
        0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 1,
    ]);
    wire.extend_from_slice(&[
        0xc0, 0x0c, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x02, 0xc0, 0x0c,
    ]);
    wire.extend_from_slice(&[
        0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    wire
}

#[test]
fn state_snapshot_is_typed_owned_and_deterministic() {
    let mut state = state();
    state.marks.extend([49, 7, 7]);
    state.fast_flags = 1 << 48;
    state.routing = RoutingState {
        domain_set: Some("domain".to_owned()),
        matched_group: Some("group".to_owned()),
        final_sequence: Some("sequence".to_owned()),
        final_upstream: Some("upstream".to_owned()),
        final_upstream_targets: Some("target-a,target-b".to_owned()),
        selected_upstream: Some("selected".to_owned()),
        matched_rule_source: Some("source".to_owned()),
    };

    let snapshot = state.snapshot();
    assert_eq!(snapshot.query.header.id, 0x1234);
    assert_eq!(
        snapshot.query.question.qname_wire,
        vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]
    );
    assert_eq!(snapshot.marks, vec![7, 49]);
    assert_eq!(snapshot.fast_flags, 1 << 48);
    assert_eq!(snapshot.routing, state.routing);
    assert_eq!(snapshot.response, ResponseState::None);
}

#[test]
fn response_setters_replace_and_clear_each_response_form() {
    let wire = complete_response_wire();
    let mut state = state();

    state.set_raw_response(wire.clone());
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);

    state
        .set_synthesized_response(15)
        .expect("valid synthesized response");
    assert_eq!(state.response.synthesized_rcode(), Some(15));

    state.set_raw_response(wire.clone());
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    state.set_response(ResponseState::Synthesized(
        SynthesizedResponse::new(0x0fff).expect("valid configured RCODE"),
    ));
    assert_eq!(state.response.synthesized_rcode(), Some(0x0fff));

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);
}

#[test]
fn synthesized_rcode_boundaries_are_validated_without_partial_transition() {
    let mut state = state();
    for rcode in [0, 15, 0x0fff] {
        state
            .set_synthesized_response(rcode)
            .expect("configured RCODE is in range");
        assert_eq!(state.response.synthesized_rcode(), Some(rcode));
    }

    let previous = state.response.clone();
    assert_eq!(
        state.set_synthesized_response(0x1000),
        Err(ResponseError::InvalidRcode(0x1000))
    );
    assert_eq!(state.response, previous);
}

#[test]
fn raw_inspection_is_ttl_only_non_consuming_and_preserves_complete_wire() {
    let wire = complete_response_wire();
    let mut state = state();
    state.set_raw_response(wire.clone());

    let inspection = state
        .inspect_response(&DnsResponseInspector)
        .expect("valid response")
        .expect("raw response is inspectable");
    assert_eq!(inspection.ttl.minimal_ttl, 60);
    assert_eq!(inspection.ttl.record_count, 2);
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    let second = state
        .inspect_response(&DnsResponseInspector)
        .expect("inspection remains repeatable")
        .expect("raw response remains inspectable");
    assert_eq!(second, inspection);
    assert_eq!(state.response, ResponseState::Raw(OwnedResponseWire(wire)));
}

#[test]
fn non_raw_inspection_does_not_change_none_or_synthesized_state() {
    let mut state = state();
    assert_eq!(state.inspect_response(&DnsResponseInspector), Ok(None));
    assert_eq!(state.response, ResponseState::None);

    state
        .set_synthesized_response(5)
        .expect("valid synthesized response");
    assert_eq!(state.inspect_response(&DnsResponseInspector), Ok(None));
    assert_eq!(state.response.synthesized_rcode(), Some(5));
}

#[test]
fn malformed_raw_inspection_returns_typed_error_and_retains_wire_until_clear() {
    let wire = vec![0x80];
    let mut state = state();
    state.set_raw_response(wire.clone());

    let error = state
        .inspect_response(&DnsResponseInspector)
        .expect_err("malformed wire must be observable");
    assert!(matches!(error, ResponseError::MalformedRawResponse { .. }));
    assert_eq!(state.response, ResponseState::Raw(OwnedResponseWire(wire)));

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);
}
