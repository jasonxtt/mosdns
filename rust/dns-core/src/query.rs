//! Strict query header/question validation.
//!
//! This mirrors the frozen Go oracle contract in
//! `pkg/query_context/rust_bridge/validation_test.go`: the header must be a
//! non-response (QR clear) with opcode QUERY and exactly one question, no
//! answer or authority records, and at most one extra record. The question
//! name must be wire-legal under the same label/compression rules the cache
//! wire walk enforces, and the question type/class must fit inside the packet.
//!
//! Malformed-wire cases return [`QueryParseError`]; wire-legal messages that
//! are not a supported query shape return [`QueryUnsupportedError`].

/// Wire defect in a DNS query: shorter than a header, illegal question name,
/// or truncated question type/class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryParseError {
    TooShort,
    BadName,
}

/// Wire-legal message that is not a supported query: QR set, non-QUERY
/// opcode, question count != 1, or non-empty answer/authority/extra.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryUnsupportedError {
    ResponseBit,
    Opcode(u8),
    QuestionCount(u16),
    NonEmptySection,
}

/// The fixed 12-byte DNS header fields a query shares.
///
/// `qr`, `opcode`, the four counts, and the raw flags word are all derived
/// from the wire bytes; this struct lets the later ABI slice carry exactly the
/// header state the Go snapshot contract exposes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryHeader {
    pub id: u16,
    pub qr: bool,
    pub opcode: u8,
    pub qdcount: u16,
    pub ancount: u16,
    pub nscount: u16,
    pub arcount: u16,
}

/// The single decoded question.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestionInfo {
    /// The question name in wire label form, copied from the input.
    pub qname_wire: Vec<u8>,
    pub qtype: u16,
    pub qclass: u16,
}

const DNS_HEADER_LEN: usize = 12;
const OPCODE_QUERY: u8 = 0;

/// Parses and validates a single DNS question at `offset` from `packet`,
/// returning the question [`QuestionInfo`] and the offset just past it (the
/// question's type/class fields).
///
/// The name must be wire-legal under the cache-style label/compression rules
/// (labels ≤ 63 bytes, compression pointers with an in-packet target, the
/// pointer itself is the name). The question type/class bytes must fit inside
/// the packet; if they do not, this is [`QueryParseError::TooShort`] (the
/// miekg tolerances of a zero class or a truncated class are *not* accepted,
/// matching the strict Go oracle contract).
///
/// # Errors
///
/// Returns [`QueryParseError::BadName`] for an illegal name and
/// [`QueryParseError::TooShort`] when the packet ends inside the name or
/// inside the type/class fields.
pub fn parse_question(packet: &[u8], offset: usize) -> Result<QuestionInfo, QueryParseError> {
    let next = question_name_end(packet, offset)?;
    let end = next.checked_add(4).ok_or(QueryParseError::TooShort)?;
    if end > packet.len() {
        return Err(QueryParseError::TooShort);
    }
    let qname_wire = expanded_question_name(packet, offset)?;
    let qtype = u16::from_be_bytes([packet[next], packet[next + 1]]);
    let qclass = u16::from_be_bytes([packet[next + 2], packet[next + 3]]);
    Ok(QuestionInfo {
        qname_wire,
        qtype,
        qclass,
    })
}

/// Strict query header/question validation, mirroring the Go oracle.
///
/// Accepts a message with QR clear, opcode QUERY, exactly one question, empty
/// answer/authority, and at most one extra record. Returns the decoded header
/// and question. A wire defect returns [`QueryParseError`]; a wire-legal but
/// unsupported shape returns [`QueryUnsupportedError`].
///
/// # Errors
///
/// Errors are classified as described on the two error enums.
pub fn parse_query(packet: &[u8]) -> Result<(QueryHeader, QuestionInfo), QueryError> {
    if packet.len() < DNS_HEADER_LEN {
        return Err(QueryError::Parse(QueryParseError::TooShort));
    }
    let id = u16::from_be_bytes([packet[0], packet[1]]);
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    let qr = flags & 0x8000 != 0;
    let opcode = ((flags >> 11) & 0x0f) as u8;
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    let ancount = u16::from_be_bytes([packet[6], packet[7]]);
    let nscount = u16::from_be_bytes([packet[8], packet[9]]);
    let arcount = u16::from_be_bytes([packet[10], packet[11]]);

    if qr {
        return Err(QueryError::Unsupported(QueryUnsupportedError::ResponseBit));
    }
    if opcode != OPCODE_QUERY {
        return Err(QueryError::Unsupported(QueryUnsupportedError::Opcode(
            opcode,
        )));
    }
    if qdcount != 1 {
        return Err(QueryError::Unsupported(
            QueryUnsupportedError::QuestionCount(qdcount),
        ));
    }
    if ancount > 0 || nscount > 0 || arcount > 1 {
        return Err(QueryError::Unsupported(
            QueryUnsupportedError::NonEmptySection,
        ));
    }
    let question = parse_question(packet, DNS_HEADER_LEN)?;
    let header = QueryHeader {
        id,
        qr,
        opcode,
        qdcount,
        ancount,
        nscount,
        arcount,
    };
    Ok((header, question))
}

/// Error enum for [`parse_query`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryError {
    Parse(QueryParseError),
    Unsupported(QueryUnsupportedError),
}

impl From<QueryParseError> for QueryError {
    fn from(err: QueryParseError) -> Self {
        Self::Parse(err)
    }
}

/// Follows one compression pointer: validates the two-byte wire form and
/// returns the target offset. `None` means the pointer bytes are truncated or
/// the target lies outside the packet.
fn pointer_target(packet: &[u8], offset: usize) -> Option<usize> {
    let second = *packet.get(offset + 1)?;
    let label = *packet.get(offset)?;
    let target = usize::from(label & 0x3f) << 8 | usize::from(second);
    if target >= packet.len() {
        return None;
    }
    Some(target)
}

/// Advances past exactly one wire name at `offset`, returning the offset just
/// past the whole name as written (in wire-record terms: the offset of the
/// next field). For a name that begins with compression pointers this is the
/// offset just past the first pointer, exactly as `dns.UnpackDomainName`
/// reports `off1`; the labels themselves are found by following the pointer
/// chain.
///
/// This is the query oracle's walker and mirrors miekg's
/// `UnpackDomainName` (`pkg/query_context` uses it for the Go query snapshot):
/// labels must be ≤ 63 bytes and the expanded wire name must stay within the
/// DNS 255-byte limit, compression pointers are *followed* with a
/// bounded budget of 126 pointers (miekg's `maxCompressionPointers`, the
/// `(255+1)/2 - 2` guard), and a chain that revisits a pointer — a
/// self-pointer or a multi-pointer cycle such as 12 -> 14 -> 12 — exceeds the
/// budget and is rejected. Unlike the response/cache walk (which never follows
/// pointers and cannot see loops), the query contract rejects these loops, as
/// the Go oracle does.
///
/// `None` means the packet ends inside the name or the encoding is illegal
/// (including a pointer loop).
fn skip_name(packet: &[u8], offset: usize) -> Option<usize> {
    const MAX_DOMAIN_NAME_WIRE_OCTETS: usize = 255;
    // miekg's maxCompressionPointers = (maxDomainNameWireOctets+1)/2 - 2 with
    // maxDomainNameWireOctets=255; clippy wants div_ceil for the +1 rounding.
    const MAX_POINTERS: usize = 255_usize.div_ceil(2) - 2;
    let mut domain_budget = MAX_DOMAIN_NAME_WIRE_OCTETS;
    let mut pos = offset;
    let mut end_after_first_pointer: Option<usize> = None;
    let mut pointers = 0usize;
    loop {
        let label = *packet.get(pos)?;
        match label & 0xc0 {
            0 => {
                if label == 0 {
                    // End of name: the wire end is past the first pointer if
                    // any was followed, else just past this root label.
                    return Some(end_after_first_pointer.unwrap_or(pos.checked_add(1)?));
                }
                // 0x40 cannot occur (masked to 0), so this is a 1..=63 label.
                let label_wire_len = usize::from(label) + 1;
                if domain_budget <= label_wire_len {
                    return None;
                }
                domain_budget -= label_wire_len;
                pos = pos.checked_add(1 + usize::from(label))?;
                if pos > packet.len() {
                    return None;
                }
            }
            0xc0 => {
                if pointers == MAX_POINTERS {
                    return None; // "too many compression pointers"
                }
                let target = pointer_target(packet, pos)?;
                // A pointer whose jump lands immediately where we already are
                // is a degenerate self-loop; the budget above would also catch
                // it after 127 iterations, but this short-circuits it.
                if target == pos {
                    return None;
                }
                if end_after_first_pointer.is_none() {
                    end_after_first_pointer = Some(pos.checked_add(2)?);
                }
                pointers += 1;
                pos = target;
            }
            _ => return None, // 0x40 and 0x80 are reserved
        }
    }
}

/// `parse_question`'s name walker: rejects a self-referential pointer.
///
/// The cache-wire `skip_name` (in `response`) deliberately accepts
/// self-pointers (its contract is only that the target lies inside the
/// packet), and the frozen response-TTL contract keeps that behavior. The
/// query oracle is separate: miekg rejects the self-referential question name
/// and pointer cycles via the compression-pointer budget, and this dedicated
/// walker preserves that distinction.
fn question_name_end(packet: &[u8], offset: usize) -> Result<usize, QueryParseError> {
    skip_name(packet, offset).ok_or(QueryParseError::BadName)
}

/// Returns a self-contained, uncompressed question name. `QuestionInfo` is
/// later allowed to outlive the original query packet, so retaining a pointer
/// whose target belonged to that packet would make response synthesis unsafe.
fn expanded_question_name(packet: &[u8], offset: usize) -> Result<Vec<u8>, QueryParseError> {
    const MAX_DOMAIN_NAME_WIRE_OCTETS: usize = 255;
    const MAX_POINTERS: usize = 255_usize.div_ceil(2) - 2;

    let mut expanded = Vec::new();
    let mut pos = offset;
    let mut pointers = 0usize;
    loop {
        let label = *packet.get(pos).ok_or(QueryParseError::TooShort)?;
        match label & 0xc0 {
            0 => {
                if label == 0 {
                    expanded.push(0);
                    return Ok(expanded);
                }
                let label_len = usize::from(label);
                let end = pos
                    .checked_add(1 + label_len)
                    .ok_or(QueryParseError::TooShort)?;
                if end > packet.len()
                    || expanded.len() + 1 + label_len > MAX_DOMAIN_NAME_WIRE_OCTETS
                {
                    return Err(QueryParseError::BadName);
                }
                expanded.push(label);
                expanded.extend_from_slice(&packet[pos + 1..end]);
                pos = end;
            }
            0xc0 => {
                if pointers == MAX_POINTERS {
                    return Err(QueryParseError::BadName);
                }
                let target = pointer_target(packet, pos).ok_or(QueryParseError::BadName)?;
                if target == pos {
                    return Err(QueryParseError::BadName);
                }
                pointers += 1;
                pos = target;
            }
            _ => return Err(QueryParseError::BadName),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{QueryError, QueryParseError, QueryUnsupportedError, parse_query};

    fn hdr(flags: u8, opcode: u8, qd: u16, an: u16, ns: u16, ar: u16) -> Vec<u8> {
        let mut b = vec![0x12, 0x34, flags | opcode << 3, 0, 0, 0, 0, 0, 0, 0, 0, 0];
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

    fn question(labels: &[&str], qtype: u16, qclass: u16) -> Vec<u8> {
        let mut b = name(labels);
        b.extend_from_slice(&qtype.to_be_bytes());
        b.extend_from_slice(&qclass.to_be_bytes());
        b
    }

    fn valid_query() -> Vec<u8> {
        let mut q = hdr(0x01, 0, 1, 0, 0, 0);
        q.extend(question(&["example", "org"], 1, 1));
        q
    }

    #[test]
    fn accepts_valid_queries() {
        let mut with_opt = hdr(0x01, 0, 1, 0, 0, 1);
        with_opt.extend(question(&["example", "org"], 1, 1));
        with_opt.extend_from_slice(&[0x00, 0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0, 0]);

        let mut root_name = hdr(0x01, 0, 1, 0, 0, 0);
        root_name.push(0);
        root_name.extend_from_slice(&[0x00, 0x1c, 0x00, 0x01]); // AAAA IN

        let mut unknown_qtype = hdr(0x01, 0, 1, 0, 0, 0);
        unknown_qtype.extend(question(&["example", "org"], 0xff, 0xfe));

        let mut compressed = hdr(0x01, 0, 1, 0, 0, 0);
        compressed.extend(name(&["example"]));
        compressed.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);

        let mut trailing = valid_query();
        trailing.extend_from_slice(&[0xff, 0x00]);

        let cases = vec![
            valid_query(),
            with_opt,
            root_name,
            unknown_qtype,
            compressed,
            trailing,
        ];
        for wire in cases {
            let (_, q) = parse_query(&wire).unwrap_or_else(|e| panic!("rejected: {e:?}"));
            assert!(!q.qname_wire.is_empty());
        }
    }

    #[test]
    fn rejects_malformed_wire() {
        let mut self_ptr = hdr(0x01, 0, 1, 0, 0, 0);
        self_ptr.extend_from_slice(&[0xc0, 0x0c]);

        let mut ptr_out_of_bounds = hdr(0x01, 0, 1, 0, 0, 0);
        ptr_out_of_bounds.extend_from_slice(&[0xc0, 0x14]);

        let mut long_label = hdr(0x01, 0, 1, 0, 0, 0);
        long_label.extend_from_slice(&[0x40]);
        long_label.extend([0; 64]);

        let no_name_bytes = hdr(0x01, 0, 1, 0, 0, 0);

        let mut name_only = hdr(0x01, 0, 1, 0, 0, 0);
        name_only.extend(name(&["example"]));

        let mut missing_qclass = hdr(0x01, 0, 1, 0, 0, 0);
        missing_qclass.extend(name(&["example"]));
        missing_qclass.extend_from_slice(&[0x00, 0x01]);

        // Two-pointer compression cycle: at offset 12 the name is a pointer to
        // 14; at offset 14 the label is a pointer to 12. miekg/dns follows the
        // chain and trips "too many compression pointers" (the same pointer is
        // revisited each iteration; the budget is 126), producing
        // ErrMalformedQuery → BadName. The response/cache walk (which never
        // follows pointers) leaves this class accepted; the query oracle must
        // reject it.
        let mut two_ptr_cycle = hdr(0x01, 0, 1, 0, 0, 0);
        two_ptr_cycle.extend_from_slice(&[
            0xc0, 0x0e, // at 12: pointer to 14
            0xc0, 0x0c, // at 14: pointer to 12
            0x00, 0x01, 0x00, 0x01, // qtype A, qclass IN
        ]);

        let cases: Vec<(Vec<u8>, QueryParseError)> = vec![
            (Vec::new(), QueryParseError::TooShort),
            (vec![1, 2, 3, 4, 5, 6], QueryParseError::TooShort),
            (self_ptr, QueryParseError::BadName),
            (two_ptr_cycle, QueryParseError::BadName),
            (ptr_out_of_bounds, QueryParseError::BadName),
            (long_label, QueryParseError::BadName),
            (no_name_bytes, QueryParseError::BadName),
            (name_only, QueryParseError::TooShort),
            (missing_qclass, QueryParseError::TooShort),
        ];
        for (i, (wire, want)) in cases.into_iter().enumerate() {
            match parse_query(&wire) {
                Err(QueryError::Parse(got)) => assert_eq!(got, want, "case {i}"),
                other => panic!("case {i} expected Parse({want:?}), got {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_names_over_dns_wire_limit() {
        let mut overlong = hdr(0x01, 0, 1, 0, 0, 0);
        for _ in 0..4 {
            overlong.push(63);
            overlong.extend([b'x'; 63]);
        }
        overlong.push(0);
        overlong.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        assert_eq!(
            parse_query(&overlong),
            Err(QueryError::Parse(QueryParseError::BadName))
        );
    }

    #[test]
    fn rejects_unsupported_shapes() {
        let mut qr_set = valid_query();
        qr_set[2] |= 0x80;

        let mut opcode_status = hdr(0x01, 2, 1, 0, 0, 0);
        opcode_status.extend(question(&["example", "org"], 1, 1));

        let mut opcode15 = hdr(0x01, 15, 1, 0, 0, 0);
        opcode15.extend(question(&["example", "org"], 1, 1));

        let qd0 = hdr(0x01, 0, 0, 0, 0, 0);

        let mut qd2 = hdr(0x01, 0, 2, 0, 0, 0);
        qd2.extend(question(&["example", "org"], 1, 1));
        qd2.extend(question(&["www", "example", "org"], 1, 1));

        let mut with_answer = hdr(0x01, 0, 1, 1, 0, 0);
        with_answer.extend(question(&["example", "org"], 1, 1));
        with_answer.extend_from_slice(&[
            0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0x3c, 0, 4, 1, 2, 3, 4,
        ]);

        let mut ar2 = hdr(0x01, 0, 1, 0, 0, 2);
        ar2.extend(question(&["example", "org"], 1, 1));

        // an=65535 with ns=1: separate count checks, sum would wrap to 0.
        let mut an_max_ns_one = hdr(0x01, 0, 1, 0xffff, 1, 0);
        an_max_ns_one.extend(question(&["example", "org"], 1, 1));

        let cases: Vec<(Vec<u8>, QueryUnsupportedError)> = vec![
            (qr_set, QueryUnsupportedError::ResponseBit),
            (opcode_status, QueryUnsupportedError::Opcode(2)),
            (opcode15, QueryUnsupportedError::Opcode(15)),
            (qd0, QueryUnsupportedError::QuestionCount(0)),
            (qd2, QueryUnsupportedError::QuestionCount(2)),
            (with_answer, QueryUnsupportedError::NonEmptySection),
            (ar2, QueryUnsupportedError::NonEmptySection),
            (an_max_ns_one, QueryUnsupportedError::NonEmptySection),
        ];
        for (i, (wire, want)) in cases.into_iter().enumerate() {
            match parse_query(&wire) {
                Err(QueryError::Unsupported(got)) => assert_eq!(got, want, "case {i}"),
                other => panic!("case {i} expected Unsupported({want:?}), got {other:?}"),
            }
        }
    }
}
