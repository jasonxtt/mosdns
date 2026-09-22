//! Response header ID/RA patching and pure UDP/stream/HTTP framing helpers.
//!
//! Mirrors the frozen Go oracle contract
//! (`pkg/query_context/rust_bridge/ttl.go::PatchResponseHeader` and
//! `framing.go::FrameResponse`), which in turn mirror
//! `pkg/server_handler/packRawResponse`: the ID is a full big-endian write to
//! wire bytes 0-1 and the RA bit is ORed into byte 3 bit 7 without touching
//! any other byte. UDP and HTTP frames pass the response through
//! byte-identical; stream/TCP ("UrlPath == \"\"") gets a two-byte big-endian
//! length prefix and rejects a response longer than 65535; HTTP (DoH,
//! "UrlPath != \"\"") has no prefix and no size gate. Every result is a
//! caller-owned copy and the input is never modified.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramingError {
    /// The response is longer than `dns::MaxMsgSize` (65535) and the chosen
    /// framing (UDP or stream) cannot represent it.
    TooLarge,
    /// The framing mode is not a valid transport.
    UnknownMode,
}

/// Error returned by [`patch_response_id_ra`] when the packet is not a valid
/// DNS response header: shorter than 12 bytes, or not a response (QR clear).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderError {
    /// The packet is shorter than the 12-byte DNS header.
    TooShort,
    /// The packet's QR bit is clear, so it is not a response header.
    NotResponse,
}

/// The minimum response metadata needed by a transport before a full DNS
/// record walk. This intentionally does not expose or parse RR/OPT sections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    pub id: u16,
    pub qr: bool,
    pub truncated: bool,
}

/// The transport framing for [`frame_response`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameMode {
    Udp,
    Stream,
    Http,
}

/// A small protocol-error response constructor for an already parsed
/// one-question query. It intentionally does not parse or normalize DNS
/// names; callers must supply the [`QueryHeader`] and [`QuestionInfo`] from
/// [`crate::parse_query`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseBuildError {
    InvalidRcode(u8),
}

/// Builds a QR/RA response carrying the original ID and question with no
/// answer, authority, or additional records. The question name is the
/// self-contained, uncompressed form produced by [`crate::parse_query`].
///
/// This is deliberately limited to the two native-host protocol-error forms
/// (SERVFAIL and REFUSED in the current caller). It is not a general DNS
/// message builder and does not inspect a second copy of the query wire.
pub fn synthesize_response(
    query: &crate::QueryHeader,
    question: &crate::QuestionInfo,
    rcode: u8,
) -> Result<Vec<u8>, ResponseBuildError> {
    if rcode > 0x0f {
        return Err(ResponseBuildError::InvalidRcode(rcode));
    }
    let flags = 0x8000 | 0x0080 | u16::from(rcode);
    let mut response = Vec::with_capacity(12 + question.qname_wire.len() + 4);
    response.extend_from_slice(&query.id.to_be_bytes());
    response.extend_from_slice(&flags.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&question.qname_wire);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&question.qclass.to_be_bytes());
    Ok(response)
}

impl FrameMode {
    /// Builds a [`FrameMode`] from its numeric discriminator, returning
    /// `None` for any value with no defined transport.
    #[must_use]
    pub const fn from_discriminant(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Udp),
            1 => Some(Self::Stream),
            2 => Some(Self::Http),
            _ => None,
        }
    }
}

/// Inspects only the fixed DNS response header.
///
/// The caller-owned wire is never modified. A complete response validator
/// remains responsible for question and record semantics.
pub fn inspect_response_header(packet: &[u8]) -> Result<ResponseHeader, HeaderError> {
    if packet.len() < 12 {
        return Err(HeaderError::TooShort);
    }
    let qr = packet[2] & 0x80 != 0;
    if !qr {
        return Err(HeaderError::NotResponse);
    }
    Ok(ResponseHeader {
        id: u16::from_be_bytes([packet[0], packet[1]]),
        qr,
        truncated: packet[2] & 0x02 != 0,
    })
}

/// Returns a caller-owned copy of `packet` with the ID written over bytes 0-1
/// (big-endian) and the RA bit set in byte 3 bit 7. Every other byte is
/// preserved verbatim, and the input is never modified.
///
/// The packet must be a DNS response header: at least 12 bytes and with the
/// QR bit (byte 2 bit 7) set. This mirrors the Go oracle, where the method
/// lives on an already-validated `ResponseSnapshot`; a malformed input here
/// returns a typed error instead of producing partial output or panicking.
///
/// # Errors
///
/// Returns [`HeaderError::TooShort`] for a packet shorter than the header and
/// [`HeaderError::NotResponse`] for a QR-clear packet.
pub fn patch_response_id_ra(packet: &[u8], id: u16) -> Result<Vec<u8>, HeaderError> {
    inspect_response_header(packet)?;
    let mut out = packet.to_vec();
    out[0..2].copy_from_slice(&id.to_be_bytes());
    out[3] |= 0x80;
    Ok(out)
}

/// Frames a validated response for the transport `mode`, mirroring
/// `pkg/server_handler/packRawResponse`'s `streamTransport` decision.
///
/// UDP and HTTP pass the response through byte-identical (caller-owned copy);
/// stream prefixes it with a two-byte big-endian length. A response longer
/// than 65535 fails with [`FramingError::TooLarge`] for UDP and stream (HTTP
/// is the `streamTransport=false` branch and is never length-checked). The
/// input is never modified.
///
/// # Errors
///
/// Returns [`FramingError::TooLarge`] for an oversized UDP/stream response and
/// [`FramingError::UnknownMode`] for an invalid mode.
pub fn frame_response(packet: &[u8], mode: FrameMode) -> Result<Vec<u8>, FramingError> {
    match mode {
        FrameMode::Udp | FrameMode::Stream => {
            const MAX_MSG: usize = 65535;
            if packet.len() > MAX_MSG {
                return Err(FramingError::TooLarge);
            }
            if mode == FrameMode::Stream {
                let len: u16 = packet
                    .len()
                    .try_into()
                    .map_err(|_| FramingError::TooLarge)?;
                let mut out = Vec::with_capacity(2 + packet.len());
                out.extend_from_slice(&len.to_be_bytes());
                out.extend_from_slice(packet);
                Ok(out)
            } else {
                Ok(packet.to_vec())
            }
        }
        FrameMode::Http => Ok(packet.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameMode, FramingError, HeaderError, ResponseBuildError, frame_response,
        patch_response_id_ra, synthesize_response,
    };
    use crate::{QueryHeader, QuestionInfo};

    const MAX: usize = 65535;

    fn resp_wire(answers: &[Vec<u8>]) -> Vec<u8> {
        let mut b: Vec<u8> = vec![
            0x12,
            0x34,
            0x81,
            0x80,
            0x00,
            0x01,
            answers.len() as u8,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        b.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        b.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00, 0x01]);
        for a in answers {
            b.extend_from_slice(a);
        }
        b
    }

    fn a(ttl: u32, ip: &[u8]) -> Vec<u8> {
        let mut b = vec![0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01];
        b.extend_from_slice(&ttl.to_be_bytes());
        b.extend_from_slice(&[0x00, 0x04]);
        b.extend_from_slice(ip);
        b
    }

    #[test]
    fn synthesizes_associated_servfail_and_refused_responses() {
        let query = QueryHeader {
            id: 0xcafe,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        };
        let question = QuestionInfo {
            qname_wire: vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0],
            qtype: 1,
            qclass: 1,
        };
        for rcode in [2, 5] {
            let response = synthesize_response(&query, &question, rcode).unwrap();
            assert_eq!(&response[0..2], &0xcafe_u16.to_be_bytes());
            assert_eq!(response[2] & 0x80, 0x80);
            assert_eq!(response[3] & 0x80, 0x80);
            assert_eq!(
                u16::from_be_bytes([response[2], response[3]]) & 0x000f,
                u16::from(rcode)
            );
            assert_eq!(&response[4..12], &[0, 1, 0, 0, 0, 0, 0, 0]);
            assert_eq!(
                &response[12..12 + question.qname_wire.len()],
                &question.qname_wire[..]
            );
            assert_eq!(&response[12 + question.qname_wire.len()..], &[0, 1, 0, 1]);
        }
        assert_eq!(
            synthesize_response(&query, &question, 16),
            Err(ResponseBuildError::InvalidRcode(16))
        );
    }

    #[test]
    fn synthesizes_a_self_contained_question_from_compressed_query_input() {
        // The query name points into the query header at offset 2. That is
        // accepted by the query parser, but the target bytes would not be a
        // valid target at offset 2 after the question is copied into a new
        // response with different flags.
        let mut query = vec![
            0x00, 0x12, 0x01, 0x00, // ID and RD flags; offset 2 is a one-byte label.
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        query.extend_from_slice(&[0xc0, 0x02, 0x00, 0x01, 0x00, 0x01]);
        let (header, question) = crate::parse_query(&query).expect("compressed query parses");
        assert_eq!(question.qname_wire, vec![1, 0, 0]);

        let response = synthesize_response(&header, &question, 2).expect("SERVFAIL response");
        assert_eq!(
            crate::validate_response(&response),
            Ok(crate::TtlInfo {
                minimal_ttl: 0,
                record_count: 0,
            })
        );
    }

    #[test]
    fn patches_id_big_endian_into_bytes_0_1() {
        let wire = resp_wire(&[a(60, &[192, 0, 2, 1])]);
        let out = patch_response_id_ra(&wire, 0x1235).unwrap();
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), 0x1235);
        // QR and every byte after the header is untouched.
        assert_eq!(out[2] & 0x80, 0x80);
        assert_eq!(&out[12..], &wire[12..]);
    }

    #[test]
    fn patches_ra_bit_only() {
        let mut wire = resp_wire(&[]);
        wire[3] &= 0x7f; // clear RA
        let out = patch_response_id_ra(&wire, 0xcafe).unwrap();
        assert_eq!(out[3] & 0x80, 0x80);
        // Everything except byte3 bit7 identical.
        assert_eq!(out[2], wire[2]);
        assert_eq!(out[4..], wire[4..]);
    }

    #[test]
    fn patch_does_not_touch_other_flags() {
        let mut wire = resp_wire(&[a(60, &[1, 2, 3, 4])]);
        wire[2] |= 0x02; // TC
        let out = patch_response_id_ra(&wire, 1).unwrap();
        assert_eq!(out[2] & 0x02, 0x02);
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), 1);
        assert_eq!(out[3] & 0x80, 0x80);
    }

    #[test]
    fn patch_zero_id_allowed() {
        let wire = resp_wire(&[a(60, &[1, 2, 3, 4])]);
        let out = patch_response_id_ra(&wire, 0).unwrap();
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 0);
    }

    #[test]
    fn patch_is_a_copy_not_aliased() {
        let wire = resp_wire(&[a(60, &[1, 2, 3, 4])]);
        let mut out = patch_response_id_ra(&wire, 0x1111).unwrap();
        let want_id = u16::from_be_bytes([out[0], out[1]]);
        out[0] ^= 0xff;
        // Mutating out must not reach the input.
        assert_eq!(wire[0], 0x12);
        assert_eq!(
            u16::from_be_bytes([out[0], out[1]]) != want_id,
            out[0] != 0x11
        );
    }

    #[test]
    fn patch_rejects_short_input() {
        // A packet shorter than the 12-byte DNS header must return a typed
        // error, not panic (the Go oracle operates on an already-validated
        // snapshot; here malformed input is rejected before any output).
        for short in [&[0u8; 0][..], &[0u8; 6][..], &[0u8; 11][..]] {
            assert_eq!(
                patch_response_id_ra(short, 0x1234),
                Err(HeaderError::TooShort)
            );
        }
    }

    #[test]
    fn patch_rejects_qr_clear() {
        // A wire-legal query (QR clear) is not a response header: the patch
        // must reject it (matching the Go oracle's response-only contract)
        // instead of mutating a query's flags.
        let mut q = resp_wire(&[]);
        q[2] &= 0x7f; // clear QR
        assert_eq!(patch_response_id_ra(&q, 1), Err(HeaderError::NotResponse));
    }

    #[test]
    fn patch_error_produces_no_partial_output_and_no_mutation() {
        // On error, the caller's bytes must be untouched (the implementation
        // copies only after validation).
        let short: [u8; 6] = [0x12, 0x34, 0x81, 0x80, 0, 0];
        let before = short;
        assert_eq!(
            patch_response_id_ra(&short, 0xffff),
            Err(HeaderError::TooShort)
        );
        assert_eq!(short, before);
    }

    #[test]
    fn udp_and_http_pass_through_without_prefix() {
        let wire = resp_wire(&[a(60, &[192, 0, 2, 1])]);
        for mode in [FrameMode::Udp, FrameMode::Http] {
            let out = frame_response(&wire, mode).unwrap();
            assert_eq!(out, wire);
        }
    }

    #[test]
    fn stream_prefixes_length_big_endian() {
        let wire = resp_wire(&[a(60, &[192, 0, 2, 1])]);
        let out = frame_response(&wire, FrameMode::Stream).unwrap();
        assert_eq!(out.len(), 2 + wire.len());
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), wire.len() as u16);
        assert_eq!(&out[2..], &wire[..]);
    }

    #[test]
    fn stream_and_udp_reject_over_max() {
        let big = vec![0u8; MAX + 10];
        for mode in [FrameMode::Udp, FrameMode::Stream] {
            assert_eq!(frame_response(&big, mode), Err(FramingError::TooLarge));
        }
    }

    #[test]
    fn http_passes_through_over_max() {
        // DNS-over-HTTP has no prefix and no length gate (streamTransport=false
        // when UrlPath != ""), so an oversized body still frames byte-identical.
        let big = vec![0u8; MAX + 10];
        let out = frame_response(&big, FrameMode::Http).unwrap();
        assert_eq!(out, big);
    }

    #[test]
    fn zero_length_response_is_framed() {
        // A 12-byte header-only response is a valid frame; its body length is 0.
        let empty: Vec<u8> = vec![0x12, 0x34, 0x81, 0x80, 0, 0, 0, 0, 0, 0, 0, 0];
        let out = frame_response(&empty, FrameMode::Stream).unwrap();
        assert_eq!(out.len(), 2 + 12);
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), 12);
    }

    #[test]
    fn unknown_mode_is_not_constructible() {
        // There is no defined transport for discriminant 9; the public
        // from_discriminant API is the safe way to obtain a mode, and an
        // invalid mode must never be constructible via the enum discriminant.
        assert!(FrameMode::from_discriminant(9).is_none());
        assert!(FrameMode::from_discriminant(2).is_some());
    }

    #[test]
    fn framing_input_not_modified() {
        let wire = resp_wire(&[a(60, &[192, 0, 2, 1])]);
        let before = wire.clone();
        let _ = frame_response(&wire, FrameMode::Udp).unwrap();
        let _ = frame_response(&wire, FrameMode::Stream).unwrap();
        assert_eq!(wire, before);
    }
}
