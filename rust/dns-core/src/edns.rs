//! EDNS/DO/ECS extraction from the single extra record of a supported query.
//!
//! Mirrors the frozen Go oracle contract in
//! `pkg/query_context/rust_bridge/edns_test.go` and miekg/dns's
//! `EDNS0_SUBNET` unpack semantics: with exactly one OPT record, report OPT
//! presence, the advertised UDP size, the DO bit, and the first ECS option's
//! family, source netmask/scope, and fixed-width address. An OPT owner may be
//! any wire-legal name (miekg's `UnpackRR` accepts a non-root owner), and an
//! ECS address is padded to the family width (4 for IPv4, 16 for IPv6) with
//! trailing zero bytes, truncating any extra wire bytes. Family 0 with a zero
//! netmask yields the 16-byte v4-mapped zero address (Go's
//! `net.IPv4(0,0,0,0)`), which Snapshot.EDNS() keeps because `To4()` only
//! applies to family 1. Malformed OPT RDLENGTH, option length, invalid
//! family/netmask/scope bounds, and undersized ECS payloads are
//! [`EdnsParseError`]. Non-OPT additional records are deliberately rejected
//! as unsupported after their owner, fixed fields, and declared body bounds
//! are checked; the Go adapter can then use its oracle fallback without
//! treating an unvalidated record as EDNS-absent.

/// Constant OPT fixed-fields length: type(2) + class(2) + ttl(4) + rdlength(2),
/// the bytes following the owner name in a wire RR (the owner is variable in
/// general; for OPT it is usually the single root label 0x00).
pub const OPT_RDLEN: usize = 10;

/// Wire defect inside the OPT record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdnsParseError {
    /// The extra record's RDLENGTH or the option list is truncated, or the
    /// owner name is truncated/illegal.
    Truncated,
    /// The first ECS option's data is shorter than family/netmask/scope.
    EcsTooShort,
    /// An option's length overruns the RDLENGTH or the packet.
    OptionOob,
    /// An ECS option has an unknown address family (not 0/1/2).
    UnknownFamily,
    /// An ECS family-0 option specifies a non-zero netmask.
    Family0NonzeroNetmask,
    /// An ECS netmask or scope exceeds the family's width (32 IPv4, 128 IPv6).
    BadNetmask,
    /// The additional record is well-framed but is not an OPT record. The
    /// Rust foundation does not decode non-OPT RDATA yet and rejects it safely
    /// so malformed or unsupported records never become EDNS-absent success.
    UnsupportedRecord,
}

/// The first EDNS0 client-subnet option, deep-copied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EcsInfo {
    pub family: u16,
    pub source_netmask: u8,
    pub source_scope: u8,
    /// The family-width address: exactly 4 bytes for family 1, 16 bytes for
    /// family 2, and the 16-byte v4-mapped zero `[::ffff:0.0.0.0]` for family
    /// 0 (Go's `net.IPv4(0,0,0,0)` representation, which Snapshot.EDNS()
    /// keeps as-is because the `To4()` narrowing only applies to family 1).
    /// Wire address bytes are padded with trailing zeros to the family width
    /// and truncated if longer, matching miekg's `EDNS0_SUBNET.unpack`.
    pub address: Vec<u8>,
}

/// The EDNS state of the single extra record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdnsInfo {
    pub has_opt: bool,
    /// The advertised UDP payload size as sent, without clamping.
    pub udp_size: u16,
    pub do_bit: bool,
    /// `Some` only when the OPT carries a client-subnet option.
    pub ecs: Option<EcsInfo>,
}

/// Extracts the EDNS state from a standalone extra-record buffer beginning at
/// the owner name. Compression pointers in this convenience form are resolved
/// relative to the supplied buffer; callers with a full DNS message must use
/// [`extract_edns_at`] so pointers retain message-relative semantics.
///
/// # Errors
///
/// Returns [`EdnsParseError`] for a malformed or unsupported extra record or an
/// invalid client-subnet option. A successful result is always an OPT record.
pub fn extract_edns(extra: &[u8]) -> Result<Option<EdnsInfo>, EdnsParseError> {
    extract_edns_at(extra, 0)
}

/// Extracts the EDNS state from the single extra record beginning at
/// `extra_offset` in the full DNS `packet`.
///
/// Any wire-legal owner name is accepted (miekg's `UnpackRR` accepts a
/// non-root OPT owner); the fixed fields (type/class/ttl/rdlength) are read
/// from the owner-name end. Owner compression pointers are followed against
/// the full packet with a bounded budget; self-pointers, pointer cycles, and
/// out-of-packet targets are rejected. A malformed OPT (truncated fields,
/// option length overruns, illegal owner) or an undersized/invalid ECS payload
/// is [`EdnsParseError`]. A non-OPT record is rejected as
/// [`EdnsParseError::UnsupportedRecord`] after generic body bounds are checked.
///
/// # Errors
///
/// Returns [`EdnsParseError`] for a malformed or unsupported extra record or
/// an invalid client-subnet option. A successful result is always an OPT
/// record.
pub fn extract_edns_at(
    packet: &[u8],
    extra_offset: usize,
) -> Result<Option<EdnsInfo>, EdnsParseError> {
    let owner_len = owner_end(packet, extra_offset).ok_or(EdnsParseError::Truncated)?;
    let fixed_end = owner_len
        .checked_add(OPT_RDLEN)
        .ok_or(EdnsParseError::Truncated)?;
    let fixed = packet
        .get(owner_len..fixed_end)
        .ok_or(EdnsParseError::Truncated)?;
    let rrtype = u16::from_be_bytes([fixed[0], fixed[1]]);
    let udp_size = u16::from_be_bytes([fixed[2], fixed[3]]);
    let ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
    let rdlength = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
    let body_start = fixed_end;
    let body_end = body_start
        .checked_add(rdlength)
        .ok_or(EdnsParseError::Truncated)?;
    let body = packet
        .get(body_start..body_end)
        .ok_or(EdnsParseError::Truncated)?;
    if rrtype != 41 {
        return Err(EdnsParseError::UnsupportedRecord);
    }
    let do_bit = ttl & 0x8000 != 0;
    let ecs = extract_first_ecs(body)?;
    Ok(Some(EdnsInfo {
        has_opt: true,
        udp_size,
        do_bit,
        ecs,
    }))
}

/// Returns one past the end of the wire owner name beginning at `start` in
/// `packet`, in wire-record terms: for a name ending in a compression pointer,
/// two bytes past that first pointer; otherwise past the terminating root
/// label.
///
/// Compression pointers are followed exactly like miekg's
/// `UnpackDomainName`, with targets relative to the full DNS message rather
/// than to an additional-record subslice. Returns `None` when the owner is
/// truncated or its encoding is illegal (including a compression loop).
fn owner_end(packet: &[u8], start: usize) -> Option<usize> {
    const MAX_POINTERS: usize = 255_usize.div_ceil(2) - 2; // miekg's budget
    let mut label_budget = 255; // max wire name octets
    let mut pos = start;
    let mut first_pointer_end: Option<usize> = None;
    let mut pointers = 0usize;
    loop {
        let label = *packet.get(pos)?;
        match label & 0xc0 {
            0 => {
                if label == 0 {
                    return Some(first_pointer_end.unwrap_or(pos.checked_add(1)?));
                }
                if label_budget <= usize::from(label) + 1 {
                    return None;
                }
                label_budget -= usize::from(label) + 1;
                pos = pos.checked_add(1 + usize::from(label))?;
                if pos > packet.len() {
                    return None;
                }
            }
            0xc0 => {
                if pointers == MAX_POINTERS {
                    return None; // too many compression pointers (cycle)
                }
                let second = *packet.get(pos + 1)?;
                let target = usize::from(label & 0x3f) << 8 | usize::from(second);
                if target >= packet.len() || target == pos {
                    return None; // out-of-packet target or self-pointer
                }
                if first_pointer_end.is_none() {
                    first_pointer_end = Some(pos.checked_add(2)?);
                }
                pointers += 1;
                pos = target;
            }
            _ => return None, // 0x40 and 0x80 are reserved
        }
    }
}

/// Returns the first ECS option's decoded value, or None if the OPT has no
/// ECS. The first ECS option wins; unknown options are skipped.
fn extract_first_ecs(options: &[u8]) -> Result<Option<EcsInfo>, EdnsParseError> {
    let mut cursor = 0;
    while cursor + 4 <= options.len() {
        let code = u16::from_be_bytes([options[cursor], options[cursor + 1]]);
        let len = usize::from(u16::from_be_bytes([
            options[cursor + 2],
            options[cursor + 3],
        ]));
        let end = cursor + 4 + len;
        if end > options.len() {
            return Err(EdnsParseError::OptionOob);
        }
        if code == 8 {
            let data = &options[cursor + 4..end];
            if data.len() < 4 {
                return Err(EdnsParseError::EcsTooShort);
            }
            return decode_ecs(data);
        }
        cursor = end;
    }
    // Truncated options past the end but within RDLENGTH is malformed.
    if cursor != options.len() {
        return Err(EdnsParseError::Truncated);
    }
    Ok(None)
}

/// Decodes an ECS option data (family/netmask/scope/address) into the
/// fixed-width address representation, validating family and netmask/scope
/// bounds exactly as miekg/dns `EDNS0_SUBNET.unpack` does.
fn decode_ecs(data: &[u8]) -> Result<Option<EcsInfo>, EdnsParseError> {
    let family = u16::from_be_bytes([data[0], data[1]]);
    let source_netmask = data[2];
    let source_scope = data[3];
    let wire_addr = &data[4..];
    let address = match family {
        0 => {
            // miekg: only a zero netmask family-0 is accepted; e.Address is
            // net.IPv4(0,0,0,0), which Go represents as the 16-byte
            // v4-mapped value. Snapshot.EDNS() only narrows via To4() when
            // Family == 1, so family 0 keeps the 16-byte mapped form.
            if source_netmask != 0 {
                return Err(EdnsParseError::Family0NonzeroNetmask);
            }
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]
        }
        1 => {
            if source_netmask > 32 || source_scope > 32 {
                return Err(EdnsParseError::BadNetmask);
            }
            let mut addr = [0u8; 4];
            addr[..wire_addr.len().min(4)].copy_from_slice(&wire_addr[..wire_addr.len().min(4)]);
            addr.to_vec()
        }
        2 => {
            if source_netmask > 128 || source_scope > 128 {
                return Err(EdnsParseError::BadNetmask);
            }
            let mut addr = [0u8; 16];
            addr[..wire_addr.len().min(16)].copy_from_slice(&wire_addr[..wire_addr.len().min(16)]);
            addr.to_vec()
        }
        _ => return Err(EdnsParseError::UnknownFamily),
    };
    Ok(Some(EcsInfo {
        family,
        source_netmask,
        source_scope,
        address,
    }))
}

#[cfg(test)]
mod tests {
    use super::{EdnsParseError, extract_edns};

    /// Builds an OPT RR with an explicit owner name prefix.
    fn opt_with_owner(owner: &[u8], udp: u16, ttl: u32, options: &[u8]) -> Vec<u8> {
        let mut b = owner.to_vec();
        b.extend_from_slice(&[0x00, 0x29]); // type OPT
        b.extend_from_slice(&udp.to_be_bytes());
        b.extend_from_slice(&ttl.to_be_bytes());
        b.extend_from_slice(&((options.len() as u16).to_be_bytes()));
        b.extend_from_slice(options);
        b
    }

    /// Builds an OPT RR with the standard single-root-label owner.
    fn opt(udp: u16, ttl: u32, options: &[u8]) -> Vec<u8> {
        opt_with_owner(&[0x00], udp, ttl, options)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn ecs(family: u16, mask: u8, scope: u8, addr: &[u8]) -> Vec<u8> {
        let mut data = vec![0x00, 0x08];
        let len = 4 + addr.len();
        data.extend_from_slice(&(len as u16).to_be_bytes());
        data.extend_from_slice(&family.to_be_bytes());
        data.push(mask);
        data.push(scope);
        data.extend_from_slice(addr);
        data
    }

    #[test]
    fn absent_without_opt() {
        // A non-OPT extra (TXT) is rejected safely; the Go adapter falls back
        // to its oracle instead of treating an unvalidated RR as absent EDNS.
        let txt = vec![
            0x00, // root owner
            0x00, 0x10, // TXT
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x03, b'a', b'b', b'c',
        ];
        assert_eq!(extract_edns(&txt), Err(EdnsParseError::UnsupportedRecord));
    }

    #[test]
    fn unsupported_extra_is_rejected_safely() {
        // A valid non-OPT extra (TXT) is outside the Rust foundation's RDATA
        // decoder. Reject it explicitly so the caller uses the Go oracle.
        let txt = vec![
            0x00, // root owner
            0x00, 0x10, // TXT
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x03, b'a', b'b', b'c',
        ];
        assert_eq!(extract_edns(&txt), Err(EdnsParseError::UnsupportedRecord));
    }

    #[test]
    fn malformed_non_opt_body_is_not_accepted() {
        // TXT RDLENGTH claims 20 bytes but only three bytes of RDATA exist.
        // The generic RR body bounds must be checked before the unsupported
        // non-OPT result is returned.
        let txt = vec![
            0x00, // root owner
            0x00, 0x10, // TXT
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x03, b'a', b'b', b'c',
        ];
        assert_eq!(extract_edns(&txt), Err(EdnsParseError::Truncated));
    }

    #[test]
    fn detects_presence_udp_and_do() {
        let w = opt(1232, 0x8000, &[]);
        let e = extract_edns(&w).unwrap().unwrap();
        assert!(e.has_opt);
        assert_eq!(e.udp_size, 1232);
        assert!(e.do_bit);
        assert!(e.ecs.is_none());
    }

    #[test]
    fn non_root_owner_opt_is_detected() {
        // miekg's UnpackRR accepts an OPT whose owner is any wire-legal name
        // (e.g. "example.org"), not just the single root label. The owner ends
        // at 13 (7+1 + 3+1 + 1 root), then type/class/ttl/rdlength follow.
        let owner = [
            0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00,
        ];
        let w = opt_with_owner(&owner, 1232, 0x8000, &[]);
        let e = extract_edns(&w)
            .expect("non-root owner OPT must be extractable")
            .expect("must be an OPT");
        assert!(e.has_opt);
        assert_eq!(e.udp_size, 1232);
        assert!(e.do_bit);
        assert!(e.ecs.is_none());
    }

    #[test]
    fn truncated_or_illegal_owner_is_an_error() {
        // Owner label claims 64 bytes but only 3 exist → truncated.
        let truncated = [0x40, b'a', b'b'];
        assert_eq!(extract_edns(&truncated), Err(EdnsParseError::Truncated));
        // Owner pointer to an out-of-slice target → truncated.
        let ptr = [0xc0, 0x40];
        assert_eq!(extract_edns(&ptr), Err(EdnsParseError::Truncated));
    }

    #[test]
    fn self_pointer_owner_is_rejected() {
        // A compression pointer at offset 0 pointing to offset 0 would loop
        // forever; miekg's UnpackDomainName rejects it ("too many compression
        // pointers"). The walker must follow in-slice targets with a bounded
        // budget and reject this self-pointer (and any cycle) instead of
        // accepting it as an OPT owner.
        let mut w = [0xc0, 0x00]; // self-pointer at 0 -> 0
        // Fixed fields after the (rejected) pointer; construction must fail.
        let extra = [w[0], w[1], 0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0, 0];
        assert_eq!(extract_edns(&extra), Err(EdnsParseError::Truncated));
        let _ = &mut w;
    }

    #[test]
    fn pointer_owner_resolves_to_in_slice_name() {
        // In this standalone RR buffer, a forward pointer owner whose target
        // terminates in a root label: extra[0..2] is the owner pointer
        // 0xc0 0x0c (target 12); the
        // fixed fields sit at 2..12; the target name "example.org" (13 bytes,
        // root-terminated) starts at 12. The walker follows the pointer to 12,
        // reads the terminating name, and returns the first-pointer end (2) so
        // the fixed fields parse; this is exactly what miekg's
        // UnpackDomainName does (its end is two bytes past the pointer).
        let w = vec![
            0xc0, 0x0c, // owner: pointer to 12 (start of the target name)
            0x00, 0x29, // type OPT
            0x04, 0xd0, // class = udp 1232
            0, 0, 0, 0, // ttl 0
            0, 0, // rdlength 0
            0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00,
        ];
        let e = extract_edns(&w)
            .expect("in-slice pointer owner must be extractable")
            .expect("must be an OPT");
        assert!(e.has_opt);
        assert_eq!(e.udp_size, 1232);
    }

    #[test]
    fn pointer_owner_uses_full_packet_base() {
        // The additional owner points to the question name at absolute offset
        // 12. Passing only query[question_end..] would incorrectly interpret
        // that target relative to the additional slice and reject it.
        let mut packet = vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        let extra_offset = packet.len();
        packet.extend_from_slice(&[
            0xc0, 0x0c, // owner pointer to the question name in the full packet
            0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0, 0,
        ]);
        let e = super::extract_edns_at(&packet, extra_offset)
            .expect("full-packet owner pointer must be extractable")
            .expect("must be an OPT");
        assert!(e.has_opt);
        assert_eq!(e.udp_size, 1232);
    }

    #[test]
    fn extracts_ipv4_ecs() {
        // ecsOpt(1,24,0,1,2,3): wire addr 3 bytes → padded to 4 with a zero.
        let w = opt(1232, 0, &ecs(1, 24, 0, &[1, 2, 3]));
        let e = extract_edns(&w).unwrap().unwrap();
        let ecs = e.ecs.expect("ecs");
        assert_eq!(ecs.family, 1);
        assert_eq!(ecs.source_netmask, 24);
        assert_eq!(ecs.source_scope, 0);
        assert_eq!(ecs.address, vec![1, 2, 3, 0]);
    }

    #[test]
    fn extracts_ipv6_ecs() {
        // ecsOpt(2,56,0,1..7): wire addr 7 bytes → padded to 16 with zeros.
        let w = opt(1232, 0, &ecs(2, 56, 0, &[1, 2, 3, 4, 5, 6, 7]));
        let e = extract_edns(&w).unwrap().unwrap();
        let ecs = e.ecs.expect("ecs");
        assert_eq!(ecs.family, 2);
        assert_eq!(
            ecs.address,
            vec![1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn ipv4_extra_wire_bytes_truncated() {
        // 5 wire bytes for family 1: only the first 4 are kept.
        let w = opt(1232, 0, &ecs(1, 24, 0, &[1, 2, 3, 4, 5]));
        let e = extract_edns(&w).unwrap().unwrap();
        assert_eq!(e.ecs.expect("ecs").address, vec![1, 2, 3, 4]);
    }

    #[test]
    fn family0_zero_netmask_yields_mapped_zero_address() {
        // miekg: family 0 netmask 0 → e.Address = net.IPv4(0,0,0,0). Go's
        // net.IPv4 returns a 16-byte v4-mapped value
        // [0,0,0,0,0,0,0,0,0,0,0xff,0xff,0,0,0,0], and Snapshot.EDNS() only
        // applies To4() when Family == 1, so family 0 keeps the 16-byte mapped
        // form.
        let w = opt(1232, 0, &ecs(0, 0, 0, &[]));
        let e = extract_edns(&w).unwrap().unwrap();
        let ecs = e.ecs.expect("ecs");
        assert_eq!(ecs.family, 0);
        assert_eq!(
            ecs.address,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]
        );
    }

    #[test]
    fn family0_nonzero_netmask_is_an_error() {
        let w = opt(1232, 0, &ecs(0, 8, 0, &[]));
        assert_eq!(extract_edns(&w), Err(EdnsParseError::Family0NonzeroNetmask));
    }

    #[test]
    fn family_netmask_scope_bounds_are_enforced() {
        // v4 netmask 33 and v4 scope 33 are each out of range.
        assert_eq!(
            extract_edns(&opt(1232, 0, &ecs(1, 33, 0, &[1]))),
            Err(EdnsParseError::BadNetmask)
        );
        assert_eq!(
            extract_edns(&opt(1232, 0, &ecs(1, 0, 33, &[1]))),
            Err(EdnsParseError::BadNetmask)
        );
        // v6 netmask 129 out of range.
        assert_eq!(
            extract_edns(&opt(1232, 0, &ecs(2, 129, 0, &[1]))),
            Err(EdnsParseError::BadNetmask)
        );
        // boundary values are accepted: v4 /32, v6 /128.
        assert!(
            extract_edns(&opt(1232, 0, &ecs(1, 32, 0, &[1, 2, 3, 4])))
                .unwrap()
                .is_some()
        );
        assert!(
            extract_edns(&opt(1232, 0, &ecs(2, 128, 0, &[1; 16])))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn unknown_family_is_a_typed_error() {
        let w = opt(1232, 0, &ecs(9, 24, 0, &[1, 2, 3]));
        assert_eq!(extract_edns(&w), Err(EdnsParseError::UnknownFamily));
    }

    #[test]
    fn first_ecs_wins_and_unknown_option_skipped() {
        let mut opts: Vec<u8> = vec![0x00, 0x20, 0x00, 0x00]; // code 0x20, len 0
        opts.extend(&ecs(1, 24, 0, &[1, 2, 3]));
        let w = opt(1232, 0, &opts);
        let e = extract_edns(&w).unwrap().unwrap();
        let info = e.ecs.expect("ecs");
        assert_eq!(info.family, 1);
        assert_eq!(info.address, vec![1, 2, 3, 0]);
        // First ECS wins: a second ECS after the first is ignored.
        let mut both = ecs(2, 56, 0, &[1]);
        both.extend(&ecs(1, 24, 0, &[9]));
        let e2 = extract_edns(&opt(1232, 0, &both)).unwrap().unwrap();
        assert_eq!(e2.ecs.expect("ecs").family, 2);
    }

    #[test]
    fn malformed_rdlength_is_an_error() {
        // RDLENGTH claims 8 bytes but none follow.
        let mut w = opt(1232, 0, &[]);
        w[9] = 0x00;
        w[10] = 0x08;
        assert_eq!(extract_edns(&w), Err(EdnsParseError::Truncated));
    }

    #[test]
    fn option_length_overflow_is_an_error() {
        // ECS option length 100 but only 7 bytes follow.
        let mut w = opt(1232, 0, &ecs(1, 24, 0, &[1, 2, 3]));
        w[13] = 0x00;
        w[14] = 0x64;
        assert_eq!(extract_edns(&w), Err(EdnsParseError::OptionOob));
    }

    #[test]
    fn undersized_ecs_is_an_error() {
        // ECS option length 2: shorter than family/netmask/scope.
        let w = opt(1232, 0, &[0x00, 0x08, 0x00, 0x02, 0x00, 0x01]);
        assert_eq!(extract_edns(&w), Err(EdnsParseError::EcsTooShort));
    }

    #[test]
    fn malformed_opt_does_not_mutate_input() {
        let wire = opt(1232, 0, &ecs(1, 24, 0, &[1, 2, 3]));
        let before = wire.clone();
        assert!(extract_edns(&wire).is_ok());
        assert_eq!(wire, before);
    }
}
