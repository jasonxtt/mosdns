//! Strict native ECS policy profile, separate from the historical EDNS parser.
use mosdns_sequence_core::ExecutorError;
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct EcsPolicy {
    pub forward: bool,
    pub send: bool,
    pub preset: Option<IpAddr>,
    pub mask4: u8,
    pub mask6: u8,
    pub active: bool,
}
impl Default for EcsPolicy {
    fn default() -> Self {
        Self {
            forward: false,
            send: false,
            preset: None,
            mask4: 24,
            mask6: 48,
            active: true,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Subnet {
    pub family: u16,
    pub source: u8,
    pub scope: u8,
    pub address: Vec<u8>,
}
impl Subnet {
    pub(crate) fn from_ip(ip: IpAddr, mask4: u8, mask6: u8) -> Self {
        let ip = unmap(ip);
        let (family, source, mut address) = match ip {
            IpAddr::V4(ip) => (1, mask4, ip.octets().to_vec()),
            IpAddr::V6(ip) => (2, mask6, ip.octets().to_vec()),
        };
        for (i, byte) in address.iter_mut().enumerate() {
            let bits = usize::from(source).saturating_sub(i * 8).min(8);
            *byte &= if bits == 0 { 0 } else { u8::MAX << (8 - bits) };
        }
        Self {
            family,
            source,
            scope: 0,
            address,
        }
    }
    pub(crate) fn decode(data: &[u8], query: bool) -> Result<Self, ExecutorError> {
        if data.len() < 4 {
            return Err(error("short ECS"));
        }
        let family = u16::from_be_bytes([data[0], data[1]]);
        let source = data[2];
        let scope = data[3];
        let width = match family {
            1 => 32,
            2 => 128,
            _ => return Err(error("unsupported ECS family")),
        };
        if source > width
            || scope > source
            || (query && scope != 0)
            || data.len() != 4 + usize::from(source).div_ceil(8)
        {
            return Err(error("invalid ECS prefix/scope/address length"));
        }
        let mut address = vec![0; usize::from(width) / 8];
        address[..data.len() - 4].copy_from_slice(&data[4..]);
        if source % 8 != 0 && address[usize::from(source) / 8] & (u8::MAX >> (source % 8)) != 0 {
            return Err(error("unmasked ECS address"));
        }
        Ok(Self {
            family,
            source,
            scope,
            address,
        })
    }
    pub(crate) fn option(&self) -> Vec<u8> {
        let mut data = self.family.to_be_bytes().to_vec();
        data.extend_from_slice(&[self.source, self.scope]);
        data.extend_from_slice(&self.address[..usize::from(self.source).div_ceil(8)]);
        let mut option = vec![0, 8];
        option.extend_from_slice(&u16::try_from(data.len()).unwrap_or(0).to_be_bytes());
        option.extend_from_slice(&data);
        option
    }
}
pub(crate) fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}
fn error(message: &str) -> ExecutorError {
    ExecutorError::new(message)
}

pub(crate) struct QueryOpt {
    pub offset: usize,
    pub fixed: Vec<u8>,
    pub other: Vec<u8>,
    pub ecs: Option<Subnet>,
}
/// Admission has already passed parse_query; validate the supported OPT and all options.
pub(crate) fn query_opt(raw: &[u8]) -> Result<Option<QueryOpt>, ExecutorError> {
    let (header, _) = mosdns_dns_core::parse_query(raw).map_err(|_| error("invalid ECS query"))?;
    let mut pos = 12;
    loop {
        let byte = *raw.get(pos).ok_or_else(|| error("short question"))?;
        pos += 1;
        if byte == 0 {
            break;
        }
        if byte & 0xc0 == 0xc0 {
            pos += 1;
            break;
        }
        pos += usize::from(byte);
    }
    pos += 4;
    if header.arcount == 0 {
        if pos != raw.len() {
            return Err(error("trailing query bytes"));
        }
        return Ok(None);
    }
    let opt = raw.get(pos..).ok_or_else(|| error("missing OPT"))?;
    if header.arcount != 1
        || opt.len() < 11
        || opt[..3] != [0, 0, 41]
        || opt[5] != 0
        || opt[6] != 0
        || opt[7] & 0x7f != 0
        || opt[8] != 0
        || opt.len() != 11 + usize::from(u16::from_be_bytes([opt[9], opt[10]]))
    {
        return Err(error("unsupported OPT profile"));
    }
    let mut cursor = 11;
    let mut other = Vec::new();
    let mut ecs = None;
    while cursor < opt.len() {
        let start = cursor;
        let fixed = opt
            .get(cursor..cursor + 4)
            .ok_or_else(|| error("short EDNS option"))?;
        let code = u16::from_be_bytes([fixed[0], fixed[1]]);
        let len = usize::from(u16::from_be_bytes([fixed[2], fixed[3]]));
        cursor += 4;
        let data = opt
            .get(cursor..cursor + len)
            .ok_or_else(|| error("short EDNS payload"))?;
        cursor += len;
        if code == 8 {
            if ecs.is_some() {
                return Err(error("duplicate ECS"));
            }
            ecs = Some(Subnet::decode(data, true)?);
        } else {
            other.extend_from_slice(&opt[start..cursor]);
        }
    }
    Ok(Some(QueryOpt {
        offset: pos,
        fixed: opt[..9].to_vec(),
        other,
        ecs,
    }))
}
pub(crate) fn replace_query_ecs(
    raw: &[u8],
    selected: Option<&Subnet>,
) -> Result<Vec<u8>, ExecutorError> {
    let opt = query_opt(raw)?;
    let offset = opt.as_ref().map_or(raw.len(), |o| o.offset);
    let mut wire = raw[..offset].to_vec();
    if opt.is_none() && selected.is_none() {
        return Ok(wire);
    }
    let (fixed, mut options) = opt.map_or_else(
        || (vec![0, 0, 41, 4, 208, 0, 0, 0, 0], Vec::new()),
        |o| (o.fixed, o.other),
    );
    if let Some(selected) = selected {
        options.extend_from_slice(&selected.option());
    }
    wire[10..12].copy_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&fixed);
    wire.extend_from_slice(
        &u16::try_from(options.len())
            .map_err(|_| error("oversized OPT"))?
            .to_be_bytes(),
    );
    wire.extend_from_slice(&options);
    if wire.len() > 65535 {
        return Err(error("oversized ECS query"));
    }
    Ok(wire)
}

/// Consume only the returned supplier's ECS. Temporarily rename ECS option codes
/// before decoding, preserving offsets while rejecting malformed ECS independently
/// of the DNS answer. Re-encoding rebuilds every name compression pointer.
pub(crate) fn response_wire(
    raw: &[u8],
    incoming: &[u8],
    echo: Option<&Subnet>,
    network_supplier: bool,
) -> Result<Vec<u8>, ExecutorError> {
    use hickory_proto::rr::rdata::opt::{EdnsCode, EdnsOption};
    mosdns_dns_core::validate_response(raw).map_err(|_| error("invalid ECS response DNS"))?;
    let mut pos = 12;
    for _ in 0..u16::from_be_bytes([raw[4], raw[5]]) {
        pos = name_end(raw, pos)? + 4;
    }
    let records = [6, 8, 10]
        .into_iter()
        .map(|i| usize::from(u16::from_be_bytes([raw[i], raw[i + 1]])))
        .sum::<usize>();
    let mut ecs_offsets = Vec::new();
    let mut codes = std::collections::BTreeSet::new();
    let mut subnet = None;
    let mut valid = true;
    let mut opt_count = 0;
    for _ in 0..records {
        pos = name_end(raw, pos)?;
        let fields = raw
            .get(pos..pos + 10)
            .ok_or_else(|| error("short response RR"))?;
        let kind = u16::from_be_bytes([fields[0], fields[1]]);
        let len = usize::from(u16::from_be_bytes([fields[8], fields[9]]));
        pos += 10;
        let end = pos + len;
        if kind == 41 {
            opt_count += 1;
            while pos < end {
                let fixed = raw
                    .get(pos..pos + 4)
                    .filter(|_| pos + 4 <= end)
                    .ok_or_else(|| error("short response option"))?;
                let code = u16::from_be_bytes([fixed[0], fixed[1]]);
                let len = usize::from(u16::from_be_bytes([fixed[2], fixed[3]]));
                codes.insert(code);
                let data = raw
                    .get(pos + 4..pos + 4 + len)
                    .filter(|_| pos + 4 + len <= end)
                    .ok_or_else(|| error("short response option data"))?;
                if code == 8 {
                    ecs_offsets.push(pos);
                    match Subnet::decode(data, false) {
                        Ok(value) if subnet.is_none() => subnet = Some(value),
                        _ => valid = false,
                    }
                }
                pos += 4 + len;
            }
        }
        pos = end;
    }
    let temporary = (65000..=u16::MAX)
        .rev()
        .find(|code| !codes.contains(code))
        .ok_or_else(|| error("no temporary EDNS code"))?;
    let mut sanitized = raw.to_vec();
    for offset in &ecs_offsets {
        sanitized[*offset..*offset + 2].copy_from_slice(&temporary.to_be_bytes());
    }
    let mut message = hickory_proto::op::Message::from_vec(&sanitized)
        .map_err(|_| error("invalid response decode"))?;
    if let Some(opt) = message.extensions_mut().as_mut() {
        opt.options_mut().remove(EdnsCode::Unknown(temporary));
    }
    let original = query_opt(incoming)?;
    if let Some(original) = original {
        let opt = message
            .extensions_mut()
            .get_or_insert_with(hickory_proto::op::Edns::new);
        opt.set_max_payload(u16::from_be_bytes([original.fixed[3], original.fixed[4]]));
        opt.set_dnssec_ok(original.fixed[7] & 0x80 != 0);
        if network_supplier && valid && opt_count == 1 && ecs_offsets.len() == 1 {
            if let (Some(expected), Some(actual)) = (echo, subnet) {
                if actual.family == expected.family
                    && actual.source == expected.source
                    && actual.address == expected.address
                {
                    let option = actual.option();
                    opt.options_mut()
                        .insert(EdnsOption::Unknown(8, option[4..].to_vec()));
                }
            }
        }
    } else {
        *message.extensions_mut() = None;
    }
    message
        .to_vec()
        .map_err(|_| error("response ECS encode failed"))
}
fn name_end(raw: &[u8], mut pos: usize) -> Result<usize, ExecutorError> {
    loop {
        let byte = *raw.get(pos).ok_or_else(|| error("short DNS name"))?;
        pos += 1;
        if byte == 0 {
            return Ok(pos);
        }
        if byte & 0xc0 == 0xc0 {
            raw.get(pos).ok_or_else(|| error("short DNS pointer"))?;
            return Ok(pos + 1);
        }
        if byte & 0xc0 != 0 {
            return Err(error("invalid DNS label"));
        }
        pos += usize::from(byte);
    }
}
