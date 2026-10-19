use std::cell::Cell;
use std::rc::Rc;

use hickory_proto::op::{Edns, Message, MessageType, Query};
use hickory_proto::rr::{Name, RData, Record, RecordType, rdata::A};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use crate::{CacheStatus, HostAssembly, ListenerKind, TcpServer, UdpServer};

fn query(id: u16, size: Option<u16>) -> Vec<u8> {
    let mut message = Message::new();
    message
        .set_id(id)
        .set_recursion_desired(true)
        .add_query(Query::query(
            Name::from_ascii("large.example.com.").unwrap(),
            RecordType::A,
        ));
    if let Some(size) = size {
        let mut opt = Edns::new();
        opt.set_max_payload(size);
        message.set_edns(opt);
    }
    message.to_vec().unwrap()
}

fn answer(query: &[u8]) -> Vec<u8> {
    let request = Message::from_vec(query).unwrap();
    let mut message = Message::new();
    message
        .set_id(request.id())
        .set_message_type(MessageType::Response)
        .set_recursion_desired(true)
        .set_recursion_available(true)
        .add_queries(request.queries().iter().cloned());
    for index in 1..=96 {
        message.add_answer(Record::from_rdata(
            request.queries()[0].name().clone(),
            60,
            RData::A(A::new(192, 0, 2, index)),
        ));
    }
    message.to_vec().unwrap()
}

#[test]
fn udp_budget_is_per_client_while_cache_and_tcp_keep_the_full_response() {
    run_cached_response(false);
}

#[test]
fn udp_decode_failure_sends_tc_and_audits_it_while_tcp_keeps_full_cache() {
    run_cached_response(true);
}

fn decode_fixture(wire: &[u8], rejected_additional: bool) -> Message {
    if rejected_additional {
        assert!(Message::from_vec(wire).is_err());
        assert_eq!(&wire[10..12], &[0, 1]);
        let mut answers = wire[..wire.len() - 611].to_vec();
        answers[10..12].fill(0);
        Message::from_vec(&answers).unwrap()
    } else {
        Message::from_vec(wire).unwrap()
    }
}

fn run_cached_response(rejected_additional: bool) {
    let upstream = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let yaml = format!(
        r#"
log:
  level: error
plugins:
  - tag: cache
    type: cache
    args:
      size: 128
  - tag: upstream
    type: forward
    args:
      upstreams:
        - addr: "udp://{}"
  - tag: entry
    type: sequence
    args:
      - exec: $cache
      - matches: has_resp
        exec: accept
      - exec: $upstream
  - tag: udp
    type: udp_server
    args:
      entry: entry
      listen: "127.0.0.1:53001"
      enable_audit: true
  - tag: tcp
    type: tcp_server
    args:
      entry: entry
      listen: "127.0.0.1:53002"
      idle_timeout: 5
      enable_audit: true
"#,
        upstream.local_addr().unwrap()
    );
    let assembly = HostAssembly::from_yaml(&yaml).unwrap();
    assembly.block_on(async {
        let upstream = UdpSocket::from_std(upstream).unwrap();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let stop = TransportCancellation::new();
        let upstream_stop = stop.clone();
        let mock = tokio::task::spawn_local(async move {
            let mut buffer = [0_u8; 65535];
            loop {
                tokio::select! {
                    () = upstream_stop.cancelled() => break,
                    request = upstream.recv_from(&mut buffer) => {
                        let (size, peer) = request.unwrap();
                        observed.set(observed.get() + 1);
                        let mut response = answer(&buffer[..size]);
                        if rejected_additional {
                            response[10..12].copy_from_slice(&1_u16.to_be_bytes());
                            response.extend_from_slice(&[0, 0, 5, 0, 1, 0, 0, 0, 60, 2, 88]);
                            response.extend_from_slice(&[255; 600]);
                        }
                        upstream.send_to(&response, peer).await.unwrap();
                    }
                }
            }
        });
        let udp = UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let udp_addr = udp.local_addr().unwrap();
        let tcp_config = assembly
            .config()
            .listeners
            .iter()
            .find(|l| l.kind == ListenerKind::Tcp)
            .unwrap();
        let tcp =
            TcpServer::bind_listener(&assembly, tcp_config, "127.0.0.1:0".parse().unwrap(), false)
                .await
                .unwrap();
        let tcp_addr = tcp.local_addr().unwrap();
        let udp_stop = stop.clone();
        let tcp_stop = stop.clone();
        let udp_task = tokio::task::spawn_local(udp.serve(udp_stop));
        let tcp_task = tokio::task::spawn_local(tcp.serve(tcp_stop));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut buffer = [0_u8; 65535];
        let mut counts = Vec::new();
        for (index, budget) in [None, Some(1232), Some(4096), None].into_iter().enumerate() {
            let wire = query(u16::try_from(index).unwrap(), budget);
            client.send_to(&wire, udp_addr).await.unwrap();
            let (size, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.recv_from(&mut buffer),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                size <= usize::from(budget.unwrap_or(512)),
                "UDP response {size} exceeds client budget"
            );
            let response =
                decode_fixture(&buffer[..size], rejected_additional && budget == Some(4096));
            assert_eq!(response.id(), u16::try_from(index).unwrap());
            assert_eq!(response.truncated(), budget != Some(4096));
            // The cached supplier response has no OPT; EDNS admission changes
            // this client's budget without fabricating supplier options.
            if budget.is_none() {
                assert!(response.extensions().is_none());
            }
            counts.push(response.answers().len());
        }
        if rejected_additional {
            assert_eq!(counts, [0, 0, 96, 0]);
        } else {
            assert!(counts[0] < counts[1]);
        }
        assert!(counts[1] < counts[2]);
        assert_eq!(counts[2], 96);
        assert_eq!(counts[3], counts[0]);
        let mut client = TcpStream::connect(tcp_addr).await.unwrap();
        let wire = query(4, None);
        client
            .write_u16(u16::try_from(wire.len()).unwrap())
            .await
            .unwrap();
        client.write_all(&wire).await.unwrap();
        let size = client.read_u16().await.unwrap();
        client
            .read_exact(&mut buffer[..usize::from(size)])
            .await
            .unwrap();
        let response = decode_fixture(&buffer[..usize::from(size)], rejected_additional);
        assert!(!response.truncated());
        assert_eq!(response.answers().len(), 96);
        assert_eq!(
            calls.get(),
            1,
            "all later sizes and TCP must use the full cached response"
        );
        while assembly.metrics_snapshot().completed_total != 5 {
            tokio::task::yield_now().await;
        }
        let audit = assembly.audit_snapshot();
        assert_eq!(audit.records.len(), 5);
        for (index, record) in audit.records.iter().enumerate() {
            assert_eq!(record.response_details.flags.tc, matches!(index, 0 | 1 | 3));
            assert_eq!(
                record.response_details.answers.len(),
                if index < 4 { counts[index] } else { 96 }
            );
            if index > 0 {
                assert!(matches!(record.cache_status, CacheStatus::Hit));
            }
        }
        drop(client);
        stop.cancel();
        udp_task.await.unwrap().unwrap();
        tcp_task.await.unwrap().unwrap();
        mock.await.unwrap();
    });
}
