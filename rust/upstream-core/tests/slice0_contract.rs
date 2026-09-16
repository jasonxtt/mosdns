use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use mosdns_dns_core::ResponseHeader;
use mosdns_upstream_core::{
    CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, ExchangeResponse,
    LifecycleState, SideEffectState, Transport, TransportCancellation, Upstream, UpstreamError,
};

fn endpoint(transport: Transport) -> mosdns_upstream_core::Endpoint {
    mosdns_upstream_core::Endpoint::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53),
        transport,
    )
    .expect("test endpoint is valid")
}

fn valid_query() -> Vec<u8> {
    vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e', b'x',
        b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00, 0x01,
    ]
}

fn context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(30),
        TransportCancellation::new(),
    )
}

#[test]
fn invalid_queries_fail_before_the_socket_boundary() {
    let upstream = Upstream::new(endpoint(Transport::Tcp));

    for query in [Vec::new(), vec![0; 12]] {
        assert!(matches!(
            ExchangeRequest::new(&query),
            Err(UpstreamError::InvalidRequest(_))
        ));
    }

    let mut unframeable = valid_query();
    unframeable.resize(usize::from(u16::MAX) + 1, 0);
    let request = ExchangeRequest::new(&unframeable).expect("trailing bytes are still a query");
    assert!(matches!(
        upstream.prepare_exchange(request, context()),
        Err(UpstreamError::FrameTooLarge)
    ));
}

#[test]
fn request_borrows_read_only_query_and_stably_exposes_original_id() {
    let mut query = valid_query();
    let before = query.clone();
    {
        let request = ExchangeRequest::new(&query).expect("valid query");
        assert_eq!(request.query(), before.as_slice());
        assert_eq!(request.request_id(), 0x1234);
        assert_eq!(query, before);
    }
    query[0] = 0xab;
}

#[test]
fn returned_response_wire_is_owned_by_the_response() {
    let response = ExchangeResponse::new(vec![0x12, 0x34], 0x1234, 0x1234, Transport::Tcp, false);
    assert_eq!(response.wire(), &[0x12, 0x34]);
    let owned = response.into_wire();
    assert_eq!(owned, vec![0x12, 0x34]);
}

#[test]
fn side_effect_state_is_closed_and_connect_is_not_sent() {
    assert_eq!(SideEffectState::NotSent, SideEffectState::NotSent);
    assert_ne!(SideEffectState::NotSent, SideEffectState::MaybeSent);
    assert_ne!(SideEffectState::MaybeSent, SideEffectState::Sent);
    assert_eq!(
        UpstreamError::Connect.side_effect(),
        SideEffectState::NotSent
    );
    assert_eq!(
        UpstreamError::Runtime(SideEffectState::MaybeSent).side_effect(),
        SideEffectState::MaybeSent
    );
}

#[test]
fn cancellation_wins_when_deadline_is_ready_at_the_same_time() {
    let token = TransportCancellation::new();
    token.cancel();
    let deadline = Instant::now();
    let context = ExchangeContext::new(deadline, token);

    assert_eq!(
        context.check_at(deadline, SideEffectState::Sent),
        Err(UpstreamError::Cancelled(SideEffectState::Sent))
    );
}

#[test]
fn deadline_is_a_distinct_typed_error_when_not_cancelled() {
    let context = ExchangeContext::new(Instant::now(), TransportCancellation::new());
    assert_eq!(
        context.check_at(Instant::now(), SideEffectState::NotSent),
        Err(UpstreamError::DeadlineExceeded(SideEffectState::NotSent))
    );
}

#[test]
fn cancellation_children_observe_parent_without_requiring_sequence_core() {
    let parent = TransportCancellation::new();
    let child = parent.child_token();
    child.cancel();
    assert!(child.is_cancelled());
    assert!(!parent.is_cancelled());

    parent.cancel();
    assert!(child.is_cancelled());
}

#[test]
fn lifecycle_is_open_closing_closed_and_close_is_idempotent() {
    let upstream = Upstream::new(endpoint(Transport::Udp));
    assert_eq!(upstream.lifecycle_state(), LifecycleState::Open);
    assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
    assert_eq!(upstream.lifecycle_state(), LifecycleState::Closing);

    let query = valid_query();
    let request = ExchangeRequest::new(&query).expect("valid query");
    assert!(matches!(
        upstream.prepare_exchange(request, context()),
        Err(UpstreamError::Closed(SideEffectState::NotSent))
    ));

    assert_eq!(upstream.begin_close(), CloseTransition::AlreadyClosing);
    upstream.finish_close();
    assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
    assert_eq!(upstream.close(), CloseResult::AlreadyClosed);

    let query = valid_query();
    let request = ExchangeRequest::new(&query).expect("valid query");
    assert!(matches!(
        upstream.prepare_exchange(request, context()),
        Err(UpstreamError::Closed(SideEffectState::NotSent))
    ));
}

#[test]
fn error_side_effects_preserve_the_last_tracked_state() {
    assert_eq!(
        UpstreamError::Cancelled(SideEffectState::MaybeSent).side_effect(),
        SideEffectState::MaybeSent
    );
    assert_eq!(
        UpstreamError::DeadlineExceeded(SideEffectState::Sent).side_effect(),
        SideEffectState::Sent
    );
    assert_eq!(
        UpstreamError::Closed(SideEffectState::MaybeSent).side_effect(),
        SideEffectState::MaybeSent
    );
}

#[test]
fn header_helper_exposes_only_header_metadata() {
    let mut response = vec![0x12, 0x34, 0x82, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
    let header: ResponseHeader =
        mosdns_dns_core::inspect_response_header(&response).expect("response header");
    assert_eq!(header.id, 0x1234);
    assert!(header.qr);
    assert!(header.truncated);

    response[2] = 0;
    assert_eq!(
        mosdns_dns_core::inspect_response_header(&response),
        Err(mosdns_dns_core::HeaderError::NotResponse)
    );
}
