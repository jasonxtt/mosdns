//! Client-specific UDP presentation, after execution and full cache publication.
use hickory_proto::op::Message;
use hickory_proto::rr::{Record, RecordType};
use hickory_proto::serialize::binary::{BinEncodable, BinEncoder};
use hickory_proto::{ProtoError, ProtoErrorKind};

/// Admission already validated the question. Malformed/unsupported OPT profiles
/// keep the ordinary 512-byte error-response budget rather than supplying one.
pub(crate) fn payload_limit(query: &[u8]) -> usize {
    if query.get(10..12) != Some(&[0, 1]) {
        return 512;
    }
    let mut end = 12;
    loop {
        let Some(&label) = query.get(end) else {
            return 512;
        };
        end += 1;
        if label == 0 {
            break;
        }
        if label & 0xc0 == 0xc0 {
            end += 1;
            break;
        }
        end += usize::from(label);
    }
    mosdns_dns_core::extract_edns_at(query, end + 4)
        .ok()
        .flatten()
        .map_or(512, |opt| usize::from(opt.udp_size).max(512))
}

/// Preserve a retry path when typed decoding/encoding rejects an oversized
/// response. Admission validated the original question; only this rare error
/// path reparses it, expanding any pointers before dropping the other sections.
pub(crate) fn prepare_response(
    wire: Vec<u8>,
    limit: usize,
    original_query: &[u8],
) -> Result<(Vec<u8>, bool), ProtoError> {
    let header: Option<[u8; 12]> = wire.get(..12).and_then(|bytes| bytes.try_into().ok());
    match limit_response(wire, limit) {
        Ok(result) => Ok(result),
        Err(error) => {
            let Some(header) = header else {
                return Err(error);
            };
            let (_, question) = mosdns_dns_core::parse_query(original_query)
                .map_err(|_| ProtoError::from("invalid admitted UDP question"))?;
            let mut output = Vec::with_capacity(12 + question.qname_wire.len() + 4);
            output.extend_from_slice(&header);
            output[2] |= 0x02;
            output[4..6].copy_from_slice(&1_u16.to_be_bytes());
            output[6..12].fill(0);
            output.extend_from_slice(&question.qname_wire);
            output.extend_from_slice(&question.qtype.to_be_bytes());
            output.extend_from_slice(&question.qclass.to_be_bytes());
            Ok((output, true))
        }
    }
}

/// Returns final wire and whether records were removed (audit must be rebuilt).
/// Already fitting wire keeps its allocation and exact encoding. Only oversized
/// responses pay for typed decoding/compression; the full cached wire is owned
/// elsewhere and is never modified here.
pub(crate) fn limit_response(wire: Vec<u8>, limit: usize) -> Result<(Vec<u8>, bool), ProtoError> {
    if wire.len() > usize::from(u16::MAX) {
        return Err(ProtoError::from("DNS response exceeds wire limit"));
    }
    let limit = limit.max(512);
    if wire.len() <= limit {
        return Ok((wire, false));
    }
    let message = Message::from_vec(&wire)?;
    if message
        .signature()
        .last()
        .or_else(|| message.additionals().last())
        .is_some_and(|record| record.record_type() == RecordType::TSIG)
    {
        // A signed response cannot be repacked without breaking its MAC.
        return Ok((wire, false));
    }
    let opt = message
        .extensions()
        .as_ref()
        .map(|edns| Record::from(edns).to_bytes())
        .transpose()?;
    let budget = limit
        .checked_sub(opt.as_ref().map_or(0, Vec::len))
        .ok_or_else(|| ProtoError::from("OPT exceeds UDP budget"))?;
    let mut output = Vec::with_capacity(limit.min(wire.len()));
    let mut encoder = BinEncoder::new(&mut output);
    encoder.emit_vec(&wire[..12])?;
    for question in message.queries() {
        question.emit(&mut encoder)?;
    }
    if encoder.len() > budget {
        return Err(ProtoError::from("question and OPT exceed UDP budget"));
    }
    let mut counts = [0_u16; 3];
    let mut removed = false;
    'sections: for (section, records) in [
        message.answers(),
        message.name_servers(),
        message.additionals(),
    ]
    .into_iter()
    .enumerate()
    {
        let signatures = if section == 2 {
            message.signature()
        } else {
            &[]
        };
        for record in records.iter().chain(signatures) {
            let offset = encoder.offset();
            let emitted = record.emit(&mut encoder);
            match emitted {
                Ok(()) if encoder.len() <= budget => counts[section] += 1,
                Ok(()) => {
                    encoder.set_offset(offset);
                    encoder.trim();
                    removed = true;
                    break 'sections;
                }
                Err(error) if matches!(error.kind(), ProtoErrorKind::MaxBufferSizeExceeded(_)) => {
                    encoder.set_offset(offset);
                    encoder.trim();
                    removed = true;
                    break 'sections;
                }
                Err(error) => return Err(error),
            }
        }
    }
    // Nothing after a rejected RR may use its discarded compression pointers.
    // OPT's root owner and opaque options have no name-pointer dependencies.
    if let Some(opt) = opt {
        encoder.emit_vec(&opt)?;
        counts[2] += 1;
    }
    for (index, count) in counts.into_iter().enumerate() {
        output[6 + index * 2..8 + index * 2].copy_from_slice(&count.to_be_bytes());
    }
    if removed {
        output[2] |= 0x02;
    }
    Ok((output, removed))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use super::*;
    use hickory_proto::op::{Edns, MessageType, Query};
    use hickory_proto::rr::{
        DNSClass, Name, RData,
        rdata::{A, NULL},
    };

    fn message() -> Message {
        let mut message = Message::new();
        message
            .set_id(7)
            .set_message_type(MessageType::Response)
            .add_query(Query::query(
                Name::from_ascii("large.example.com.").unwrap(),
                RecordType::A,
            ));
        message
    }

    fn address() -> Record {
        Record::from_rdata(
            Name::from_ascii("large.example.com.").unwrap(),
            60,
            RData::A(A::new(192, 0, 2, 7)),
        )
    }

    fn opaque(size: usize) -> Record {
        Record::from_rdata(
            Name::root(),
            60,
            RData::NULL(if size == 0 {
                NULL::new()
            } else {
                NULL::with(vec![7; size])
            }),
        )
    }

    fn hex(wire: &[u8]) -> String {
        wire.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn check(name: &str, wire: Vec<u8>, limit: usize) -> (Vec<u8>, bool) {
        let result = limit_response(wire.clone(), limit).unwrap();
        if let Ok(oracle) = std::env::var("MOSDNS_UDP_TRUNCATION_ORACLE") {
            let mut child = Command::new(oracle)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let request = serde_json::json!({"name":name,"wire":hex(&wire),"size":limit});
            writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            for (index, field) in ["answer", "authority", "additional"]
                .into_iter()
                .enumerate()
            {
                let count = u16::from_be_bytes([result.0[6 + index * 2], result.0[7 + index * 2]]);
                assert_eq!(
                    u64::from(count),
                    expected[field].as_u64().unwrap(),
                    "{name}: {field}"
                );
            }
            assert_eq!(
                result.0[2] & 2 != 0,
                expected["tc"].as_bool().unwrap(),
                "{name}: TC"
            );
            if let Ok(path) = std::env::var("MOSDNS_UDP_TRUNCATION_REPORT") {
                let row = serde_json::json!({"case":name,"rust_bytes":result.0.len(),"go_bytes":expected["size"],
                    "byte_equal":hex(&result.0)==expected["wire"].as_str().unwrap(),"fields_equal":true,
                    "budget":limit,"tc":result.0[2]&2!=0});
                writeln!(
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .unwrap(),
                    "{row}"
                )
                .unwrap();
            }
        }
        result
    }

    #[test]
    fn fits_and_exact_boundary_preserve_wire_and_allocation() {
        let mut message = message();
        message.add_answer(opaque(0));
        let overhead = message.to_vec().unwrap().len();
        message.answers_mut().clear();
        message.add_answer(opaque(512 - overhead));
        let wire = message.to_vec().unwrap();
        assert_eq!(wire.len(), 512);
        let pointer = wire.as_ptr();
        let (wire, changed) = limit_response(wire, 512).unwrap();
        assert_eq!(pointer, wire.as_ptr());
        assert!(!changed);
        assert_eq!(check("exact_512", wire.clone(), 512).0, wire);
    }

    #[test]
    fn compression_can_fit_every_record_without_setting_tc() {
        let mut message = message();
        for _ in 0..20 {
            message.add_answer(address());
        }
        let mut wire = Vec::new();
        let mut encoder = BinEncoder::new(&mut wire);
        encoder.set_canonical_names(true);
        message.emit(&mut encoder).unwrap();
        assert!(wire.len() > 512);
        let (wire, changed) = check("compression_only", wire, 512);
        assert!(!changed);
        assert!(wire.len() <= 512);
        let response = Message::from_vec(&wire).unwrap();
        assert_eq!(response.answers().len(), 20);
        assert!(!response.truncated());
    }

    #[test]
    fn oversized_answer_stops_later_sections_too() {
        let mut message = message();
        message
            .add_answer(opaque(600))
            .add_name_server(address())
            .add_additional(address());
        let (wire, changed) = check("single_rr_too_large", message.to_vec().unwrap(), 512);
        let response = Message::from_vec(&wire).unwrap();
        assert!(changed && response.truncated());
        assert!(response.answers().is_empty());
        assert!(response.name_servers().is_empty());
        assert!(response.additionals().is_empty());
    }

    #[test]
    fn dropping_only_additional_sets_tc() {
        let mut message = message();
        message
            .add_answer(address())
            .add_name_server(address())
            .add_additional(opaque(600));
        let (wire, _) = check("additional_only", message.to_vec().unwrap(), 512);
        let response = Message::from_vec(&wire).unwrap();
        assert_eq!(response.answers().len(), 1);
        assert_eq!(response.name_servers().len(), 1);
        assert!(response.additionals().is_empty());
        assert!(response.truncated());
    }

    #[test]
    fn opt_is_reserved_and_preserved_even_when_the_answer_would_fit_alone() {
        let mut message = message();
        message.add_answer(opaque(0));
        let overhead = message.to_vec().unwrap().len();
        message.answers_mut().clear();
        message.add_answer(opaque(512 - overhead));
        let mut opt = Edns::new();
        opt.set_max_payload(4096).set_dnssec_ok(true);
        message.set_edns(opt);
        let (wire, _) = check("reserve_opt", message.to_vec().unwrap(), 512);
        let response = Message::from_vec(&wire).unwrap();
        assert!(response.truncated());
        assert!(response.answers().is_empty());
        let opt = response.extensions().as_ref().unwrap();
        assert_eq!(opt.max_payload(), 4096);
        assert!(opt.flags().dnssec_ok);
        assert!(wire.len() <= 512);
    }

    #[test]
    fn budgets_512_1232_4096_keep_section_prefixes() {
        let mut message = message();
        for _ in 0..200 {
            message.add_answer(address());
        }
        let mut counts = Vec::new();
        for limit in [512, 1232, 4096] {
            let (wire, _) = check(&format!("budget_{limit}"), message.to_vec().unwrap(), limit);
            assert!(wire.len() <= limit);
            let response = Message::from_vec(&wire).unwrap();
            assert_eq!(response.truncated(), limit != 4096);
            counts.push(response.answers().len());
        }
        assert!(counts[0] < counts[1] && counts[1] < counts[2]);
        assert_eq!(counts[2], 200);
    }

    #[test]
    fn upstream_tc_is_never_cleared() {
        let mut message = message();
        message.set_truncated(true);
        for _ in 0..96 {
            message.add_answer(address());
        }
        for limit in [512, 4096] {
            let (wire, _) = check(
                &format!("existing_tc_{limit}"),
                message.to_vec().unwrap(),
                limit,
            );
            assert!(Message::from_vec(&wire).unwrap().truncated());
        }
    }

    #[test]
    fn tsig_response_is_not_repacked_or_truncated() {
        let mut message = message();
        for _ in 0..96 {
            message.add_answer(address());
        }
        let mut data = b"\x0bhmac-sha256\0".to_vec();
        data.extend([0; 16]);
        let mut signature = Record::from_rdata(
            Name::from_ascii("key.example.com.").unwrap(),
            0,
            RData::Unknown {
                code: RecordType::TSIG,
                rdata: NULL::with(data),
            },
        );
        signature.set_dns_class(DNSClass::ANY);
        message.add_additional(signature);
        let original = message.to_vec().unwrap();
        let (wire, changed) = check("tsig", original.clone(), 512);
        assert_eq!(wire, original);
        assert!(!changed);
    }

    #[test]
    fn original_client_opt_controls_limit_with_a_512_floor() {
        for size in [0, 100, 511, 512, 1232, 4096] {
            let mut query = message();
            query.set_message_type(MessageType::Query);
            let mut opt = Edns::new();
            opt.set_max_payload(size);
            query.set_edns(opt);
            assert_eq!(
                payload_limit(&query.to_vec().unwrap()),
                usize::from(size).max(512)
            );
        }
        assert_eq!(payload_limit(&message().to_vec().unwrap()), 512);
    }

    #[test]
    fn undecodable_oversized_response_falls_back_to_original_question_and_header() {
        let mut query = message();
        query.set_message_type(MessageType::Query);
        let query = query.to_vec().unwrap();
        let mut wire = message().to_vec().unwrap();
        wire[3] |= 0x83; // Preserve RA and NXDOMAIN from the response header.
        wire[10..12].copy_from_slice(&1_u16.to_be_bytes());
        // Wire-bounded CNAME RDATA with an invalid label: accepted by the generic
        // response/cache walker, rejected by Hickory's typed decoder.
        wire.extend_from_slice(&[0, 0, 5, 0, 1, 0, 0, 0, 60, 2, 88]);
        wire.extend_from_slice(&[255; 600]);
        assert!(wire.len() > 512);
        assert!(mosdns_dns_core::validate_response(&wire).is_ok());
        assert!(Message::from_vec(&wire).is_err());
        let mut expected = wire[..12].to_vec();
        expected[2] |= 2;
        expected[6..12].fill(0);
        expected.extend_from_slice(&query[12..]);
        let (output, changed) = prepare_response(wire, 512, &query).unwrap();
        assert!(changed);
        assert_eq!(output, expected);
        assert!(Message::from_vec(&output).unwrap().truncated());
    }

    #[test]
    fn oversized_opt_uses_the_same_minimal_tc_fallback() {
        let mut query = message();
        query.set_message_type(MessageType::Query);
        let query = query.to_vec().unwrap();
        let mut wire = message().to_vec().unwrap();
        wire[10..12].copy_from_slice(&1_u16.to_be_bytes());
        // The OPT fits alone, but leaves no room for the question at 512.
        wire.extend_from_slice(&[0, 0, 41, 16, 0, 0, 0, 0, 0, 1, 218]);
        wire.extend_from_slice(&[0xfd, 0xe9, 1, 214]);
        wire.extend_from_slice(&[7; 470]);
        assert!(Message::from_vec(&wire).is_ok());
        assert!(limit_response(wire.clone(), 512).is_err());
        let (output, changed) = prepare_response(wire, 512, &query).unwrap();
        let response = Message::from_vec(&output).unwrap();
        assert!(changed && response.truncated());
        assert_eq!(&output[12..], &query[12..]);
        assert_eq!(&output[6..12], &[0; 6]);
        assert!(output.len() <= 512);
    }
}
