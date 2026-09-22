//! Response validation and TTL observation/aging/replacement.
//!
//! Byte-for-byte mirrors of the frozen Go oracle
//! (`pkg/query_context/rust_bridge/ttl.go`) and the existing `cache-core`
//! wire walk. The declared question/answer/authority/extra counts are walked
//! in header order, names and record bounds must be wire-legal, and OPT
//! records (type 41) are skipped for TTL purposes. Aging subtracts a
//! whole-second delta with saturating arithmetic (floor 0); replacement sets
//! every non-OPT record's TTL. All transforms produce a caller-owned copy;
//! malformed input never produces partial output or mutates its input.

/// Wire defect in a DNS response: shorter than a header, QR clear,
/// illegal name/compression, truncated question type/class, truncated record
/// fixed field, or truncated record rdata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    TooShort,
    NotResponse,
    BadName,
    TruncatedQuestion,
    TruncatedRecord,
    InvalidRecordData,
}

/// DNS section containing a declared resource record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseSection {
    Answer,
    Authority,
    Additional,
}

/// The observed TTL state of a response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TtlInfo {
    /// Smallest TTL over every non-OPT record; 0 when none exists.
    pub minimal_ttl: u32,
    /// Number of non-OPT records whose TTL was observed.
    pub record_count: u32,
}

/// The response question, expanded from any compression pointers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseQuestion {
    pub qname_wire: Vec<u8>,
    pub qtype: u16,
    pub qclass: u16,
}

/// Metadata needed by a native cache admission decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseMetadata {
    pub rcode: u16,
    pub opcode: u8,
    pub answer_count: u16,
    pub question: Option<ResponseQuestion>,
    pub has_opt: bool,
    pub truncated: bool,
}

const DNS_HEADER_LEN: usize = 12;
const RR_FIXED_LEN: usize = 10;
const TYPE_OPT: u16 = 41;
const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;

/// Returns ordinary A and AAAA addresses from the Answer section only.
///
/// All declared records are still walked, so a truncated Authority or
/// Additional record rejects the response rather than publishing a partial
/// observation. A and AAAA records must have their exact wire RDATA lengths
/// wherever they occur; CNAME and every other record type are deliberately
/// ignored as address candidates.
///
/// # Errors
///
/// Returns [`ResponseError`] when the response cannot be fully and safely
/// observed.
pub fn observe_answer_addresses(packet: &[u8]) -> Result<Vec<std::net::IpAddr>, ResponseError> {
    let mut addresses = Vec::new();
    walk_records(packet, |record| {
        let expected_length = match record.rrtype {
            TYPE_A => Some(4),
            TYPE_AAAA => Some(16),
            _ => None,
        };
        if let Some(expected_length) = expected_length {
            if record.rdata.len() != expected_length {
                return Err(ResponseError::InvalidRecordData);
            }
            if record.section == ResponseSection::Answer {
                let address = match record.rrtype {
                    TYPE_A => std::net::IpAddr::V4(std::net::Ipv4Addr::new(
                        record.rdata[0],
                        record.rdata[1],
                        record.rdata[2],
                        record.rdata[3],
                    )),
                    TYPE_AAAA => {
                        let mut octets = [0_u8; 16];
                        octets.copy_from_slice(record.rdata);
                        std::net::IpAddr::V6(std::net::Ipv6Addr::from(octets))
                    }
                    _ => unreachable!("address type was checked above"),
                };
                addresses.push(address);
            }
        }
        Ok(())
    })?;
    Ok(addresses)
}

/// Validates a response and returns its raw TTL observation.
///
/// Mirrors the Go `ResponseSnapshot.ObserveTTL` and the cache walk's
/// `validate_response`+TTL scan. Trailing bytes after the last declared record
/// are tolerated (matching the Go oracle and the live unpack path).
///
/// # Errors
///
/// Returns [`ResponseError`] on any wire defect.
pub fn validate_response(packet: &[u8]) -> Result<TtlInfo, ResponseError> {
    observe_response_ttl(packet)
}

/// Observes the response header, question and declared record types in one
/// bounded wire walk. The helper intentionally does not impose the native
/// cache's policy; callers decide whether the metadata is eligible.
pub fn observe_response_metadata(packet: &[u8]) -> Result<ResponseMetadata, ResponseError> {
    if packet.len() < DNS_HEADER_LEN {
        return Err(ResponseError::TooShort);
    }
    if packet[2] & 0x80 == 0 {
        return Err(ResponseError::NotResponse);
    }

    let qdcount = usize::from(u16::from_be_bytes([packet[4], packet[5]]));
    let question = if qdcount == 1 {
        let (end, qname_wire) = read_name(packet, DNS_HEADER_LEN)?;
        let fields_end = end.checked_add(4).ok_or(ResponseError::TooShort)?;
        let fields = packet
            .get(end..fields_end)
            .ok_or(ResponseError::TruncatedQuestion)?;
        Some(ResponseQuestion {
            qname_wire,
            qtype: u16::from_be_bytes([fields[0], fields[1]]),
            qclass: u16::from_be_bytes([fields[2], fields[3]]),
        })
    } else {
        None
    };

    let mut has_opt = false;
    let mut extended_rcode = 0_u16;
    walk_records(packet, |record| {
        if record.rrtype == TYPE_OPT {
            has_opt = true;
            extended_rcode = u16::from((record.ttl >> 24) as u8);
        }
        Ok(())
    })?;

    Ok(ResponseMetadata {
        rcode: (u16::from(packet[3] & 0x0f)) | (extended_rcode << 4),
        opcode: (packet[2] >> 3) & 0x0f,
        answer_count: u16::from_be_bytes([packet[6], packet[7]]),
        question,
        has_opt,
        truncated: packet[2] & 0x02 != 0,
    })
}

/// Reports the minimal non-OPT TTL and count, mirroring the Go oracle.
///
/// # Errors
///
/// Returns [`ResponseError`] on any wire defect.
pub fn observe_response_ttl(packet: &[u8]) -> Result<TtlInfo, ResponseError> {
    // A single bounded walk collects both the non-OPT record count and the
    // minimal TTL; the walk's Result is propagated, so this production path
    // contains no unwrap/expect and cannot panic on malformed input.
    let mut count: u32 = 0;
    let mut min = u32::MAX;
    walk_records(packet, |record| {
        let ttl = record.ttl;
        if record.rrtype == TYPE_OPT {
            return Ok(());
        }
        if ttl < min {
            min = ttl;
        }
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        return Ok(TtlInfo {
            minimal_ttl: 0,
            record_count: 0,
        });
    }
    Ok(TtlInfo {
        minimal_ttl: min,
        record_count: count,
    })
}

/// Returns a caller-owned copy with every non-OPT TTL reduced by `elapsed`
/// using saturating arithmetic (floor 0, not the decoded-path clamp of 1).
/// OPT records are left byte-identical.
///
/// # Errors
///
/// Returns [`ResponseError`] on any wire defect; the input is never modified.
pub fn age_response_ttls(packet: &[u8], elapsed: u32) -> Result<Vec<u8>, ResponseError> {
    patch_ttls(packet, |ttl| ttl.saturating_sub(elapsed))
}

/// Returns a caller-owned copy with every non-OPT TTL set to `ttl`
/// (0 is legal). OPT records are left byte-identical.
///
/// # Errors
///
/// Returns [`ResponseError`] on any wire defect; the input is never modified.
pub fn replace_response_ttls(packet: &[u8], ttl: u32) -> Result<Vec<u8>, ResponseError> {
    patch_ttls(packet, |_| ttl)
}

fn patch_ttls(packet: &[u8], transform: impl Fn(u32) -> u32) -> Result<Vec<u8>, ResponseError> {
    let mut patched = packet.to_vec();
    walk_records(packet, |record| {
        if record.rrtype == TYPE_OPT {
            return Ok(());
        }
        let offset = record.ttl_offset;
        let ttl = record.ttl;
        patched[offset..offset + 4].copy_from_slice(&transform(ttl).to_be_bytes());
        Ok(())
    })?;
    Ok(patched)
}

/// Visits the TTL offset of every non-OPT record; returns the count visited.
struct RecordMetadata<'a> {
    section: ResponseSection,
    rrtype: u16,
    ttl: u32,
    ttl_offset: usize,
    rdata: &'a [u8],
}

fn walk_records<'a>(
    packet: &'a [u8],
    mut visitor: impl FnMut(RecordMetadata<'a>) -> Result<(), ResponseError>,
) -> Result<usize, ResponseError> {
    if packet.len() < DNS_HEADER_LEN {
        return Err(ResponseError::TooShort);
    }
    if packet[2] & 0x80 == 0 {
        return Err(ResponseError::NotResponse);
    }

    let qdcount = usize::from(u16::from_be_bytes([packet[4], packet[5]]));
    let ancount = usize::from(u16::from_be_bytes([packet[6], packet[7]]));
    let nscount = usize::from(u16::from_be_bytes([packet[8], packet[9]]));
    let arcount = usize::from(u16::from_be_bytes([packet[10], packet[11]]));

    let mut position = DNS_HEADER_LEN;
    for _ in 0..qdcount {
        position = skip_name(packet, position).ok_or(ResponseError::BadName)?;
        position = position.checked_add(4).ok_or(ResponseError::TooShort)?;
        if position > packet.len() {
            return Err(ResponseError::TooShort);
        }
    }

    let mut visits = 0;
    let sections = [
        (ResponseSection::Answer, ancount),
        (ResponseSection::Authority, nscount),
        (ResponseSection::Additional, arcount),
    ];
    for (section, count) in sections {
        for _ in 0..count {
            position = skip_name(packet, position).ok_or(ResponseError::BadName)?;
            let fixed_end = position
                .checked_add(RR_FIXED_LEN)
                .ok_or(ResponseError::TruncatedRecord)?;
            if fixed_end > packet.len() {
                return Err(ResponseError::TruncatedRecord);
            }
            let rrtype = u16::from_be_bytes([packet[position], packet[position + 1]]);
            let ttl = read_u32(packet, position + 4);
            let data_len = usize::from(u16::from_be_bytes([
                packet[position + 8],
                packet[position + 9],
            ]));
            let rdata_end = fixed_end
                .checked_add(data_len)
                .ok_or(ResponseError::TruncatedRecord)?;
            let rdata = packet
                .get(fixed_end..rdata_end)
                .ok_or(ResponseError::TruncatedRecord)?;
            visitor(RecordMetadata {
                section,
                rrtype,
                ttl,
                ttl_offset: position + 4,
                rdata,
            })?;
            if rrtype != TYPE_OPT {
                visits += 1;
            }
            position = rdata_end;
            if position > packet.len() {
                return Err(ResponseError::TruncatedRecord);
            }
        }
    }
    Ok(visits)
}

/// Reads and expands one DNS name while preserving ASCII case.
fn read_name(packet: &[u8], offset: usize) -> Result<(usize, Vec<u8>), ResponseError> {
    const MAX_NAME_OCTETS: usize = 255;
    const MAX_POINTERS: usize = 255_usize.div_ceil(2) - 2;

    let mut name = Vec::with_capacity(32);
    let mut position = offset;
    let mut written_end = None;
    let mut pointers = 0;
    loop {
        let label = *packet.get(position).ok_or(ResponseError::BadName)?;
        match label & 0xc0 {
            0 => {
                if label == 0 {
                    name.push(0);
                    let end = written_end
                        .unwrap_or(position.checked_add(1).ok_or(ResponseError::BadName)?);
                    return Ok((end, name));
                }
                let length = usize::from(label);
                let start = position.checked_add(1).ok_or(ResponseError::BadName)?;
                let end = start.checked_add(length).ok_or(ResponseError::BadName)?;
                let bytes = packet.get(start..end).ok_or(ResponseError::BadName)?;
                if name.len() + length + 1 > MAX_NAME_OCTETS {
                    return Err(ResponseError::BadName);
                }
                name.push(label);
                name.extend_from_slice(bytes);
                position = end;
            }
            0xc0 => {
                if pointers == MAX_POINTERS {
                    return Err(ResponseError::BadName);
                }
                let second = *packet.get(position + 1).ok_or(ResponseError::BadName)?;
                let target = usize::from(label & 0x3f) << 8 | usize::from(second);
                if target >= packet.len() || target == position {
                    return Err(ResponseError::BadName);
                }
                if written_end.is_none() {
                    written_end = Some(position.checked_add(2).ok_or(ResponseError::BadName)?);
                }
                pointers += 1;
                position = target;
            }
            _ => return Err(ResponseError::BadName),
        }
    }
}

/// Skip one wire name, mirroring the cache walk (see `query::skip_name`'s
/// rules). `None` means the packet ends inside the name or the encoding is
/// illegal.
fn skip_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    loop {
        let label = *packet.get(offset)?;
        match label {
            0 => return offset.checked_add(1),
            value if value & 0xc0 == 0xc0 => {
                let second = *packet.get(offset + 1)?;
                let target = usize::from(value & 0x3f) << 8 | usize::from(second);
                if target >= packet.len() {
                    return None;
                }
                return offset.checked_add(2);
            }
            value if value & 0xc0 != 0 || value > 63 => return None,
            value => {
                offset = offset.checked_add(1 + usize::from(value))?;
                if offset > packet.len() {
                    return None;
                }
            }
        }
    }
}

/// Reads a big-endian u32 at `offset` (bounds guaranteed by the walk).
fn read_u32(packet: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        ResponseError, TtlInfo, age_response_ttls, observe_answer_addresses, observe_response_ttl,
        replace_response_ttls, validate_response,
    };

    const TYPE_OPT: u16 = 41;

    fn hdr(flags: u8, rcode: u8, qd: u16, an: u16, ns: u16, ar: u16) -> Vec<u8> {
        let mut b = vec![0x12, 0x34, flags, rcode | 0x80, 0, 0, 0, 0, 0, 0, 0, 0];
        b[4..6].copy_from_slice(&qd.to_be_bytes());
        b[6..8].copy_from_slice(&an.to_be_bytes());
        b[8..10].copy_from_slice(&ns.to_be_bytes());
        b[10..12].copy_from_slice(&ar.to_be_bytes());
        b
    }

    fn name(labels: &[&str]) -> Vec<u8> {
        let mut b = Vec::new();
        for l in labels {
            b.push(l.len() as u8);
            b.extend_from_slice(l.as_bytes());
        }
        b.push(0);
        b
    }

    fn question(labels: &[&str]) -> Vec<u8> {
        let mut b = name(labels);
        b.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
        b
    }

    fn a_rr(ttl: u32, ip: &[u8]) -> Vec<u8> {
        let mut b = vec![0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]; // ptr(12) A IN
        b.extend_from_slice(&ttl.to_be_bytes());
        b.extend_from_slice(&[0x00, 0x04]);
        b.extend_from_slice(ip);
        b
    }

    fn rr(rrtype: u16, rdata: &[u8]) -> Vec<u8> {
        let mut b = vec![0xc0, 0x0c]; // compressed owner
        b.extend_from_slice(&rrtype.to_be_bytes());
        b.extend_from_slice(&1_u16.to_be_bytes());
        b.extend_from_slice(&10_u32.to_be_bytes());
        b.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        b.extend_from_slice(rdata);
        b
    }

    fn opt(udp: u16, ttl: u32, options: &[u8]) -> Vec<u8> {
        let mut b = vec![
            0x00, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        b[3..5].copy_from_slice(&udp.to_be_bytes());
        b[5..9].copy_from_slice(&ttl.to_be_bytes());
        b[9..11].copy_from_slice(&((options.len() as u16).to_be_bytes()));
        b.extend_from_slice(options);
        b
    }

    fn resp_wire(answers: &[Vec<u8>], authorities: &[Vec<u8>], extras: &[Vec<u8>]) -> Vec<u8> {
        let mut w = hdr(
            0x81,
            0,
            1,
            answers.len() as u16,
            authorities.len() as u16,
            extras.len() as u16,
        );
        w.extend(question(&["example", "org"]));
        for sec in [answers, authorities, extras] {
            for rr in sec {
                w.extend_from_slice(rr);
            }
        }
        w
    }

    #[test]
    fn accepts_valid_responses() {
        let simple = resp_wire(&[a_rr(60, &[192, 0, 2, 1])], &[], &[]);
        let mixed = resp_wire(
            &[a_rr(60, &[192, 0, 2, 1]), a_rr(120, &[192, 0, 2, 2])],
            &[a_rr(30, &[198, 51, 100, 1])],
            &[opt(1232, 0x8000, &[])],
        );
        let empty = resp_wire(&[], &[], &[]);
        let only_opt = resp_wire(&[], &[], &[opt(1232, 0x8000, &[])]);
        let mut nxdomain = resp_wire(&[], &[], &[]);
        nxdomain[3] = 0x83; // RCODE NXDOMAIN

        let mut uncompressed = name(&["www", "example", "org"]);
        uncompressed.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        uncompressed.extend_from_slice(&60u32.to_be_bytes());
        uncompressed.extend_from_slice(&[0x00, 0x04, 192, 0, 2, 9]);

        let mut no_question = hdr(0x81, 0, 0, 1, 0, 0);
        no_question.extend(question(&["example", "org"]));
        no_question.extend_from_slice(&[0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 1, 2, 3, 4]);

        let mut self_ptr_question = hdr(0x81, 0, 1, 0, 0, 0);
        self_ptr_question.extend_from_slice(&[0xc0, 0x0c]);
        self_ptr_question.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let mut trailing = resp_wire(&[a_rr(60, &[1, 2, 3, 4])], &[], &[]);
        trailing.extend_from_slice(&[0xff, 0x00]);

        for wire in [
            simple,
            mixed,
            empty,
            only_opt,
            nxdomain,
            resp_wire(&[uncompressed], &[], &[]),
            no_question,
            self_ptr_question,
            trailing,
        ] {
            validate_response(&wire).unwrap_or_else(|e| panic!("rejected: {e:?}"));
        }
    }

    #[test]
    fn observes_only_answer_addresses_but_validates_every_declared_section() {
        let wire = resp_wire(
            &[
                rr(5, &[0xc0, 0x0c]), // CNAME is never an address candidate.
                a_rr(10, &[192, 0, 2, 10]),
                rr(
                    28,
                    &[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
                ),
            ],
            &[a_rr(10, &[198, 51, 100, 1])],
            &[rr(
                28,
                &[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
            )],
        );
        assert_eq!(
            observe_answer_addresses(&wire).expect("valid answer observation"),
            vec![
                "192.0.2.10".parse::<std::net::IpAddr>().expect("IPv4"),
                "2001:db8::1".parse::<std::net::IpAddr>().expect("IPv6"),
            ]
        );

        let malformed_answer = resp_wire(&[rr(1, &[192, 0, 2])], &[], &[]);
        assert_eq!(
            observe_answer_addresses(&malformed_answer),
            Err(ResponseError::InvalidRecordData)
        );
        let malformed_authority = resp_wire(&[], &[rr(28, &[0; 15])], &[]);
        assert_eq!(
            observe_answer_addresses(&malformed_authority),
            Err(ResponseError::InvalidRecordData)
        );

        let mut truncated_extra = resp_wire(&[a_rr(10, &[192, 0, 2, 10])], &[], &[]);
        truncated_extra[10..12].copy_from_slice(&1_u16.to_be_bytes());
        assert_eq!(
            observe_answer_addresses(&truncated_extra),
            Err(ResponseError::BadName)
        );
    }

    #[test]
    fn rejects_malformed_responses() {
        let mut q_long_label = hdr(0x81, 0, 1, 0, 0, 0);
        q_long_label.extend_from_slice(&[0x40]);
        q_long_label.extend([0; 64]);
        q_long_label.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let mut q_oob_ptr = hdr(0x81, 0, 1, 0, 0, 0);
        q_oob_ptr.extend_from_slice(&[0xc0, 0x20]);
        q_oob_ptr.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let mut q_truncated = hdr(0x81, 0, 1, 0, 0, 0);
        q_truncated.extend(name(&["example", "org"]));
        q_truncated.extend_from_slice(&[0x00, 0x01]);

        let mut bad_rr = vec![0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01];
        bad_rr.extend_from_slice(&60u32.to_be_bytes());
        bad_rr.extend_from_slice(&[0x00, 0xc8]);
        let record_length_overflow = resp_wire(&[bad_rr], &[], &[]);

        let mut declared_missing = resp_wire(&[a_rr(60, &[1, 2, 3, 4])], &[], &[]);
        declared_missing[6..8].copy_from_slice(&2u16.to_be_bytes());

        let mut bad_owner = vec![0x80];
        bad_owner.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        bad_owner.extend_from_slice(&60u32.to_be_bytes());
        bad_owner.extend_from_slice(&[0x00, 0x04, 1, 2, 3, 4]);
        let bad_owner_name = resp_wire(&[bad_owner], &[], &[]);

        let cases = vec![
            (Vec::new(), ResponseError::TooShort),
            (vec![1, 2, 3, 4, 5, 6], ResponseError::TooShort),
            // Header-only with QD=1: the question name has no bytes, so the
            // walk reports BadName (matching the Go oracle's "question name at
            // offset" classification; both are ErrMalformedResponse there).
            (hdr(0x81, 0, 1, 0, 0, 0), ResponseError::BadName),
            (q_long_label, ResponseError::BadName),
            (q_oob_ptr, ResponseError::BadName),
            (q_truncated, ResponseError::TooShort),
            (record_length_overflow, ResponseError::TruncatedRecord),
            // AN=2 but only one answer: the second record's name runs past the
            // end of the packet, so the walk reports BadName ("record name at
            // offset"), matching the Go oracle's classification.
            (declared_missing, ResponseError::BadName),
            (bad_owner_name, ResponseError::BadName),
        ];
        for (i, (wire, want)) in cases.into_iter().enumerate() {
            match validate_response(&wire) {
                Err(got) => assert_eq!(got, want, "case {i}"),
                Ok(info) => panic!("case {i} expected {want:?}, got info {info:?}"),
            }
        }
    }

    #[test]
    fn rejects_non_response() {
        // A wire-legal query is not a response (QR clear).
        let mut q = hdr(0x01, 0, 1, 0, 0, 0);
        q.extend(question(&["example", "org"]));
        assert_eq!(validate_response(&q), Err(ResponseError::NotResponse));
    }

    #[test]
    fn observes_minimal_ttl_and_count() {
        let wire = resp_wire(
            &[a_rr(60, &[192, 0, 2, 1]), a_rr(120, &[192, 0, 2, 2])],
            &[a_rr(30, &[198, 51, 100, 1])],
            &[opt(1232, 0x8000, &[])],
        );
        let info = observe_response_ttl(&wire).unwrap();
        assert_eq!(
            info,
            TtlInfo {
                minimal_ttl: 30,
                record_count: 3,
            }
        );
    }

    #[test]
    fn observes_zero_when_no_records() {
        let cases = vec![
            resp_wire(&[], &[], &[]),
            resp_wire(&[], &[], &[opt(1232, 0x8000, &[])]),
        ];
        for wire in cases {
            let info = observe_response_ttl(&wire).unwrap();
            assert_eq!(
                info,
                TtlInfo {
                    minimal_ttl: 0,
                    record_count: 0,
                }
            );
        }
    }

    #[test]
    fn ages_each_non_opt_and_saturates_at_zero() {
        // 17s age on ttls 60,120,30 -> 43,103,13; OPT DO flags preserved.
        let wire = resp_wire(
            &[a_rr(60, &[192, 0, 2, 1]), a_rr(120, &[192, 0, 2, 2])],
            &[a_rr(30, &[198, 51, 100, 1])],
            &[opt(1232, 0x8000, &[])],
        );
        let aged = age_response_ttls(&wire, 17).unwrap();
        let non_opt: Vec<u32> = ttl_offsets(&aged)
            .into_iter()
            .filter(|(_, is_opt)| !is_opt)
            .map(|(off, _)| u32::from_be_bytes(aged[off..off + 4].try_into().unwrap()))
            .collect();
        assert_eq!(non_opt, vec![43, 103, 13]);
        // OPT TTL bytes untouched (DO flags).
        let opt_off = ttl_offsets(&aged)
            .into_iter()
            .find(|(_, is_opt)| *is_opt)
            .map(|(off, _)| off)
            .expect("OPT present");
        assert_eq!(
            u32::from_be_bytes(aged[opt_off..opt_off + 4].try_into().unwrap()),
            0x8000
        );
        // The walk itself is unchanged by aging; input untouched.
        assert_eq!(
            ttl_offsets(&aged),
            ttl_offsets(&wire),
            "aging must not change record layout"
        );

        // Saturation: ttl 3 aged by 10 = 0.
        let wire2 = resp_wire(&[a_rr(3, &[192, 0, 2, 1])], &[], &[]);
        let aged2 = age_response_ttls(&wire2, 10).unwrap();
        let off2 = ttl_offsets(&aged2)[0].0;
        assert_eq!(
            u32::from_be_bytes(aged2[off2..off2 + 4].try_into().unwrap()),
            0
        );

        // Zero elapsed is a no-op.
        assert_eq!(age_response_ttls(&wire, 0).unwrap(), wire);
    }

    #[test]
    fn replaces_ttls_skipping_opt() {
        let wire = resp_wire(
            &[a_rr(60, &[192, 0, 2, 1]), a_rr(120, &[192, 0, 2, 2])],
            &[a_rr(30, &[198, 51, 100, 1])],
            &[opt(1232, 0x8000, &[])],
        );
        let out = replace_response_ttls(&wire, 5).unwrap();
        let non_opt: Vec<u32> = ttl_offsets(&out)
            .into_iter()
            .filter(|(_, is_opt)| !is_opt)
            .map(|(off, _)| u32::from_be_bytes(out[off..off + 4].try_into().unwrap()))
            .collect();
        assert_eq!(non_opt, vec![5, 5, 5]);
        // OPT DO flags preserved.
        let opt_off = ttl_offsets(&out)
            .into_iter()
            .find(|(_, is_opt)| *is_opt)
            .map(|(off, _)| off)
            .expect("OPT present");
        assert_eq!(
            u32::from_be_bytes(out[opt_off..opt_off + 4].try_into().unwrap()),
            0x8000
        );
    }

    /// Returns (ttl_offset, is_opt) for every record, walking the packet the
    /// same way the production walker does. Test helper only: asserts the
    /// production offsets map to the same layout on aging/replacement.
    fn ttl_offsets(packet: &[u8]) -> Vec<(usize, bool)> {
        let an = usize::from(u16::from_be_bytes([packet[6], packet[7]]));
        let ns = usize::from(u16::from_be_bytes([packet[8], packet[9]]));
        let ar = usize::from(u16::from_be_bytes([packet[10], packet[11]]));
        let mut pos = 12;
        // question: skip its name + 4.
        let mut p = pos;
        loop {
            let label = packet[p];
            if label == 0 {
                p += 1;
                break;
            }
            if label & 0xc0 == 0xc0 {
                p += 2;
                break;
            }
            p += 1 + usize::from(label);
        }
        pos = p + 4;
        let mut out = Vec::new();
        for _ in 0..an + ns + ar {
            let mut p = pos;
            loop {
                let label = packet[p];
                if label == 0 {
                    p += 1;
                    break;
                }
                if label & 0xc0 == 0xc0 {
                    p += 2;
                    break;
                }
                p += 1 + usize::from(label);
            }
            let rrtype = u16::from_be_bytes([packet[p], packet[p + 1]]);
            let rdlen = usize::from(u16::from_be_bytes([packet[p + 8], packet[p + 9]]));
            out.push((p + 4, rrtype == TYPE_OPT));
            pos = p + 10 + rdlen;
        }
        out
    }

    #[test]
    fn cache_wire_fixture_parity() {
        // Bit-for-bit copy of rust/cache-core wire.rs
        // response_with_compressed_answer_and_opt(60, 0x0000_8000) with answer
        // TTL at offset 35 and OPT TTL at offset 50. A direct age of 17 must
        // produce the same bytes cache-core produces (ttl 43 at 35, DO flags
        // 0x00008000 preserved at 50), and the input must remain untouched.
        let mut packet = vec![
            0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00,
            0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04, 192, 0, 2, 1, 0x00,
            0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0x00, 0x00,
        ];
        packet[35..39].copy_from_slice(&60u32.to_be_bytes());
        packet[50..54].copy_from_slice(&0x0000_8000u32.to_be_bytes());
        assert_eq!(packet.len(), 56);

        let aged = age_response_ttls(&packet, 17).unwrap();
        assert_eq!(&aged[35..39], &43u32.to_be_bytes()[..]);
        assert_eq!(&aged[50..54], &0x0000_8000u32.to_be_bytes()[..]);
        // Input untouched.
        assert_eq!(&packet[35..39], &60u32.to_be_bytes()[..]);
        assert_eq!(&packet[50..54], &0x0000_8000u32.to_be_bytes()[..]);

        let info = observe_response_ttl(&packet).unwrap();
        assert_eq!(
            info,
            TtlInfo {
                minimal_ttl: 60,
                record_count: 1,
            }
        );
    }

    #[test]
    fn rejects_truncated_rr_without_partial_output() {
        let mut packet = resp_wire(&[a_rr(60, &[192, 0, 2, 1])], &[], &[]);
        let ttl_before = u32::from_be_bytes(packet[35..39].try_into().unwrap());
        packet.truncate(40);
        assert_eq!(
            age_response_ttls(&packet, 1),
            Err(ResponseError::TruncatedRecord)
        );
        assert_eq!(
            u32::from_be_bytes(packet[35..39].try_into().unwrap()),
            ttl_before
        );
    }
}
