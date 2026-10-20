use std::time::Duration;

use hickory_proto::op::{Edns, Message, Query};
use hickory_proto::rr::{Name, RecordType, rdata::opt::EdnsCode};
use mosdns_native_host::{
    AuditRecord, FailureProvenance, HostAssembly, HostOptions, LocalFailureKind,
    QueryTerminalOutcome, ResponseSource, ResponseState, TcpServer, UdpServer, compile_yaml,
};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

fn config(peer: std::net::SocketAddr, tcp: bool, path: &str) -> String {
    let rules = match path {
        "ecs" => "[{exec: '$ecs'}, {exec: '$up'}]",
        "quick_ecs" => "[{exec: 'ecs 192.0.2.1'}, {exec: '$up'}]",
        "redirect" => "[{exec: '$rewrite'}, {exec: '$up'}]",
        "fallback" => "[{exec: '$choice'}]",
        "preference" => "[{exec: prefer_ipv4}, {exec: '$up'}]",
        "empty" => "[{exec: 'mark 1'}]",
        "exit" => "[{exec: exit}]",
        "drop" => "[{exec: '$local'}, {exec: drop_resp}]",
        "forward" => "[{exec: '$up'}]",
        "sleep" => "[{exec: '$ecs'}, {exec: 'sleep 1000'}]",
        _ => panic!("unknown test path"),
    };
    let transport = if tcp { "tcp" } else { "udp" };
    format!(
        "log: {{level: error}}
plugins:
  - tag: up
    type: forward
    args: {{upstreams: [{{addr: 'udp://{peer}', upstream_query_timeout: 6000}}]}}
  - tag: ecs
    type: ecs_handler
    args: {{preset: '192.0.2.1'}}
  - tag: rewrite
    type: redirect
    args: {{rules: ['budget.example target.example']}}
  - tag: choice
    type: fallback
    args: {{primary: '$up', secondary: '$up', threshold: 1, always_standby: true}}
  - tag: local
    type: hosts
    args: {{entries: ['budget.example 192.0.2.1']}}
  - tag: main
    type: sequence
    args: {rules}
  - tag: dns
    type: {transport}_server
    args: {{entry: main, listen: '127.0.0.1:19909', enable_audit: true}}
"
    )
}

async fn read_tcp(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let size = stream.read_u16().await?;
    let mut wire = vec![0; usize::from(size)];
    stream.read_exact(&mut wire).await?;
    Ok(wire)
}

fn run(path: &str, tcp: bool, edns: bool, cancel: bool) {
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_nonblocking(true).unwrap();
    let yaml = config(peer.local_addr().unwrap(), tcp, path);
    let host = HostAssembly::with_options(
        compile_yaml(&yaml).unwrap(),
        HostOptions::with_deadline(if cancel {
            Duration::from_secs(3)
        } else {
            Duration::from_millis(80)
        }),
    )
    .unwrap();
    host.block_on(async {
        let peer = UdpSocket::from_std(peer).unwrap();
        let stop = TransportCancellation::new();
        let (address, serving) = if tcp {
            let server = TcpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let address = server.local_addr().unwrap();
            let shutdown = stop.clone();
            (
                address,
                tokio::task::spawn_local(async move { server.serve(shutdown).await.unwrap() }),
            )
        } else {
            let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let address = server.local_addr().unwrap();
            let shutdown = stop.clone();
            (
                address,
                tokio::task::spawn_local(async move { server.serve(shutdown).await.unwrap() }),
            )
        };
        let query = query(path, edns);
        let wire = query.to_vec().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut stream = if tcp {
            Some(TcpStream::connect(address).await.unwrap())
        } else {
            None
        };
        if let Some(stream) = &mut stream {
            stream
                .write_u16(u16::try_from(wire.len()).unwrap())
                .await
                .unwrap();
            stream.write_all(&wire).await.unwrap();
        } else {
            udp.send_to(&wire, address).await.unwrap();
        }
        if cancel {
            let mut packet = [0; 4096];
            tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut packet))
                .await
                .unwrap()
                .unwrap();
            stop.cancel();
        }
        let response = tokio::time::timeout(Duration::from_millis(600), async {
            if let Some(stream) = &mut stream {
                read_tcp(stream).await
            } else {
                let mut packet = vec![0; 4096];
                let len = udp.recv(&mut packet).await?;
                packet.truncate(len);
                Ok(packet)
            }
        })
        .await;
        stop.cancel();
        serving.await.unwrap();
        let records = host.audit_snapshot().records;
        assert_eq!(records.len(), 1, "{path} tcp={tcp}");
        let record = &records[0];
        assert_eq!(host.metrics_snapshot().in_flight, 0);
        if cancel {
            assert!(
                !matches!(response, Ok(Ok(_))),
                "canceled query sent a response"
            );
            assert_eq!(record.terminal_outcome, QueryTerminalOutcome::Canceled);
            assert_eq!(record.response, ResponseState::NoResponse);
            return;
        }
        let response = response
            .expect("live request must answer")
            .expect("DNS frame, not EOF");
        assert_answer(
            &Message::from_vec(&response).unwrap(),
            &query,
            record,
            path,
            edns,
        );
    });
}

fn query(path: &str, edns: bool) -> Message {
    let mut query = Message::new();
    query
        .set_id(0x1234)
        .set_recursion_desired(true)
        .add_query(Query::query(
            Name::from_ascii("budget.example.").unwrap(),
            if path == "preference" {
                RecordType::AAAA
            } else {
                RecordType::A
            },
        ));
    if edns {
        let mut opt = Edns::new();
        opt.set_max_payload(1232).set_dnssec_ok(true);
        query.set_edns(opt);
    }
    query
}

fn assert_answer(
    response: &Message,
    query: &Message,
    record: &AuditRecord,
    path: &str,
    edns: bool,
) {
    let rcode = if matches!(path, "empty" | "exit" | "drop") {
        5
    } else {
        2
    };
    assert_eq!(u16::from(response.response_code()), rcode, "{path}");
    assert_eq!(response.id(), query.id());
    assert_eq!(
        response.queries(),
        query.queries(),
        "restore admission question"
    );
    assert!(response.answers().is_empty());
    if matches!(
        path,
        "ecs" | "quick_ecs" | "redirect" | "fallback" | "preference" | "sleep"
    ) {
        assert_eq!(response.extensions().is_some(), edns);
        if let Some(opt) = response.extensions() {
            assert_eq!(opt.max_payload(), 1232);
            assert!(opt.flags().dnssec_ok);
            assert!(opt.options().get(EdnsCode::Subnet).is_none());
        }
        assert_eq!(
            record.failure_provenance,
            Some(FailureProvenance::LocalFailure(
                LocalFailureKind::RequestBudgetExceeded
            ))
        );
    }
    assert_eq!(record.terminal_outcome, QueryTerminalOutcome::SendSucceeded);
    assert_eq!(
        record.response,
        ResponseState::Dns {
            rcode,
            source: ResponseSource::Local
        }
    );
    assert!(record.selected_upstream.is_none());
    assert!(
        record
            .upstream_diagnostics
            .as_ref()
            .unwrap()
            .selected
            .is_none()
    );
}

macro_rules! budget_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            for tcp in [false, true] {
                for edns in [false, true] {
                    run($path, tcp, edns, false);
                }
            }
        }
    };
}

#[test]
fn ecs_udp_budget_answers_servfail() {
    for edns in [false, true] {
        run("ecs", false, edns, false);
    }
}

#[test]
fn ecs_tcp_budget_answers_servfail() {
    for edns in [false, true] {
        run("ecs", true, edns, false);
    }
}

budget_test!(quick_ecs_budget_answers_servfail, "quick_ecs");
budget_test!(redirect_budget_answers_servfail, "redirect");
budget_test!(fallback_budget_answers_servfail, "fallback");
budget_test!(preference_budget_answers_servfail, "preference");
budget_test!(scoped_sleep_budget_answers_servfail, "sleep");
budget_test!(ordinary_forward_failure_still_answers_servfail, "forward");
budget_test!(empty_sequence_still_answers_refused, "empty");
budget_test!(exit_still_answers_refused, "exit");
budget_test!(drop_response_still_answers_refused, "drop");

#[test]
fn policy_root_shutdown_still_sends_nothing() {
    for path in ["ecs", "redirect", "fallback", "preference"] {
        for tcp in [false, true] {
            run(path, tcp, true, true);
        }
    }
}
