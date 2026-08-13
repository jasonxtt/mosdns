// DNS wire walking adapted from KixDNS `src/proto_utils.rs` at commit
// 2da3a2d (GPL-3.0): https://github.com/olicesx/kixdns/blob/2da3a2d/src/proto_utils.rs
// Local changes: strict bounds validation, error returns, and copy-before-patch
// semantics so malformed cache entries cannot expose partially modified bytes.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WireError;

const DNS_HEADER_LEN: usize = 12;
const RR_FIXED_LEN: usize = 10;
const TYPE_OPT: u16 = 41;

pub(crate) fn age_ttls(packet: &[u8], elapsed_secs: u32) -> Result<Vec<u8>, WireError> {
    patch_ttls(packet, |ttl| ttl.saturating_sub(elapsed_secs))
}

pub(crate) fn set_ttls(packet: &[u8], ttl: u32) -> Result<Vec<u8>, WireError> {
    patch_ttls(packet, |_| ttl)
}

pub(crate) fn validate_response(packet: &[u8]) -> Result<(), WireError> {
    visit_ttl_offsets(packet, |_| Ok(()))
}

fn patch_ttls(packet: &[u8], transform: impl Fn(u32) -> u32) -> Result<Vec<u8>, WireError> {
    let mut patched = packet.to_vec();
    visit_ttl_offsets(packet, |offset| {
        let ttl = read_u32(packet, offset)?;
        patched[offset..offset + 4].copy_from_slice(&transform(ttl).to_be_bytes());
        Ok(())
    })?;
    Ok(patched)
}

fn visit_ttl_offsets(
    packet: &[u8],
    mut visitor: impl FnMut(usize) -> Result<(), WireError>,
) -> Result<(), WireError> {
    if packet.len() < DNS_HEADER_LEN || packet[2] & 0x80 == 0 {
        return Err(WireError);
    }

    let question_count = usize::from(read_u16(packet, 4)?);
    let record_count = usize::from(read_u16(packet, 6)?)
        .checked_add(usize::from(read_u16(packet, 8)?))
        .and_then(|count| count.checked_add(usize::from(read_u16(packet, 10).ok()?)))
        .ok_or(WireError)?;

    let mut position = DNS_HEADER_LEN;
    for _ in 0..question_count {
        position = skip_name(packet, position)?;
        position = position.checked_add(4).ok_or(WireError)?;
        if position > packet.len() {
            return Err(WireError);
        }
    }

    for _ in 0..record_count {
        position = skip_name(packet, position)?;
        let fixed_end = position.checked_add(RR_FIXED_LEN).ok_or(WireError)?;
        if fixed_end > packet.len() {
            return Err(WireError);
        }
        let record_type = read_u16(packet, position)?;
        if record_type != TYPE_OPT {
            visitor(position + 4)?;
        }
        let data_len = usize::from(read_u16(packet, position + 8)?);
        position = fixed_end.checked_add(data_len).ok_or(WireError)?;
        if position > packet.len() {
            return Err(WireError);
        }
    }
    Ok(())
}

fn skip_name(packet: &[u8], mut position: usize) -> Result<usize, WireError> {
    loop {
        let label = *packet.get(position).ok_or(WireError)?;
        match label {
            0 => return position.checked_add(1).ok_or(WireError),
            value if value & 0xc0 == 0xc0 => {
                let second = *packet.get(position + 1).ok_or(WireError)?;
                let target = usize::from(value & 0x3f) << 8 | usize::from(second);
                if target >= packet.len() {
                    return Err(WireError);
                }
                return position.checked_add(2).ok_or(WireError);
            }
            value if value & 0xc0 != 0 || value > 63 => return Err(WireError),
            value => {
                position = position
                    .checked_add(1 + usize::from(value))
                    .ok_or(WireError)?;
                if position > packet.len() {
                    return Err(WireError);
                }
            }
        }
    }
}

fn read_u16(packet: &[u8], offset: usize) -> Result<u16, WireError> {
    let bytes: [u8; 2] = packet
        .get(offset..offset + 2)
        .ok_or(WireError)?
        .try_into()
        .map_err(|_| WireError)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(packet: &[u8], offset: usize) -> Result<u32, WireError> {
    let bytes: [u8; 4] = packet
        .get(offset..offset + 4)
        .ok_or(WireError)?
        .try_into()
        .map_err(|_| WireError)?;
    Ok(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{age_ttls, set_ttls};

    const ANSWER_TTL_OFFSET: usize = 35;
    const OPT_TTL_OFFSET: usize = 50;

    fn response_with_compressed_answer_and_opt(answer_ttl: u32, opt_ttl: u32) -> Vec<u8> {
        let mut packet = vec![
            0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00,
            0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04, 192, 0, 2, 1, 0x00,
            0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0x00, 0x00,
        ];
        packet[ANSWER_TTL_OFFSET..ANSWER_TTL_OFFSET + 4].copy_from_slice(&answer_ttl.to_be_bytes());
        packet[OPT_TTL_OFFSET..OPT_TTL_OFFSET + 4].copy_from_slice(&opt_ttl.to_be_bytes());
        packet
    }

    fn ttl_at(packet: &[u8], offset: usize) -> u32 {
        u32::from_be_bytes(
            packet[offset..offset + 4]
                .try_into()
                .expect("four-byte TTL"),
        )
    }

    #[test]
    fn ages_compressed_answer_and_preserves_opt_flags() {
        let packet = response_with_compressed_answer_and_opt(60, 0x0000_8000);
        let aged = age_ttls(&packet, 17).expect("valid response");

        assert_eq!(ttl_at(&aged, ANSWER_TTL_OFFSET), 43);
        assert_eq!(ttl_at(&aged, OPT_TTL_OFFSET), 0x0000_8000);
        assert_eq!(
            packet[ANSWER_TTL_OFFSET..ANSWER_TTL_OFFSET + 4],
            60_u32.to_be_bytes()
        );
    }

    #[test]
    fn ttl_aging_saturates_at_zero() {
        let packet = response_with_compressed_answer_and_opt(3, 0);
        let aged = age_ttls(&packet, 10).expect("valid response");
        assert_eq!(ttl_at(&aged, ANSWER_TTL_OFFSET), 0);
    }

    #[test]
    fn lazy_ttl_replacement_skips_opt() {
        let packet = response_with_compressed_answer_and_opt(60, 0x0000_8000);
        let aged = set_ttls(&packet, 5).expect("valid response");
        assert_eq!(ttl_at(&aged, ANSWER_TTL_OFFSET), 5);
        assert_eq!(ttl_at(&aged, OPT_TTL_OFFSET), 0x0000_8000);
    }

    #[test]
    fn accepts_empty_nxdomain_and_servfail_responses() {
        for flags in [[0x81, 0x83], [0x81, 0x82]] {
            let packet = vec![
                0x12, 0x34, flags[0], flags[1], 0, 1, 0, 0, 0, 0, 0, 0, 1, b'x', 0, 0, 1, 0, 1,
            ];
            assert_eq!(age_ttls(&packet, 1).expect("valid empty response"), packet);
        }
    }

    #[test]
    fn rejects_truncated_rr_without_returning_partial_output() {
        let mut packet = response_with_compressed_answer_and_opt(60, 0);
        packet.truncate(40);
        assert!(age_ttls(&packet, 1).is_err());
        assert_eq!(ttl_at(&packet, ANSWER_TTL_OFFSET), 60);
    }

    #[test]
    fn rejects_invalid_label_encoding() {
        let mut packet = response_with_compressed_answer_and_opt(60, 0);
        packet[12] = 0x80;
        assert!(set_ttls(&packet, 5).is_err());
    }
}
