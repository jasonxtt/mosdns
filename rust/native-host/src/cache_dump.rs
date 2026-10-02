//! Bounded native codec for the existing mosdns_cache_v2 persistence format.
use crate::cache::CacheAdapterError;
use flate2::{Compression, GzBuilder, bufread::GzDecoder};
use mosdns_cache_core::NativeSnapshotEntry;
use std::io::{Read, Write};

pub(crate) const MAX_COMPRESSED: usize = 16 * 1024 * 1024;
const MAX_OWNED: usize = 64 * 1024 * 1024;
const MAX_BLOCK: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const NAME: &[u8] = b"mosdns_cache_v2";
fn bad() -> CacheAdapterError {
    CacheAdapterError::Dump("invalid or oversized cache dump".into())
}
fn charge(budget: &mut usize, count: usize) -> Result<(), CacheAdapterError> {
    *budget = budget
        .checked_add(count)
        .filter(|n| *n <= MAX_OWNED)
        .ok_or_else(bad)?;
    Ok(())
}
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn field(out: &mut Vec<u8>, number: u64, bytes: &[u8]) {
    varint(out, number * 8 + 2);
    varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}
fn encode_entry(entry: &NativeSnapshotEntry) -> Result<Vec<u8>, CacheAdapterError> {
    let [stored, msg, cache] = entry.wall_times.ok_or_else(bad)?;
    let mut out = Vec::new();
    field(&mut out, 1, &entry.key);
    field(&mut out, 2, &entry.response);
    for (number, value) in [(3, cache), (4, msg), (5, stored)] {
        varint(&mut out, number * 8);
        varint(&mut out, u64::try_from(value).map_err(|_| bad())?);
    }
    if !entry.domain_set.is_empty() {
        field(&mut out, 6, &entry.domain_set);
    }
    Ok(out)
}
pub(crate) fn encode(entries: &[NativeSnapshotEntry]) -> Result<Vec<u8>, CacheAdapterError> {
    if entries.len() > MAX_ENTRIES {
        return Err(bad());
    }
    let mut writer = GzBuilder::new()
        .filename(NAME)
        .write(Vec::new(), Compression::default());
    let mut budget = 0;
    for entry in entries {
        let encoded = encode_entry(entry)?;
        let mut block = Vec::new();
        field(&mut block, 1, &encoded);
        if block.len() > MAX_BLOCK {
            return Err(bad());
        }
        charge(&mut budget, 8 + block.len())?;
        charge(
            &mut budget,
            entry.key.len() + entry.response.len() + entry.domain_set.len(),
        )?;
        writer
            .write_all(&(block.len() as u64).to_be_bytes())
            .map_err(|_| bad())?;
        writer.write_all(&block).map_err(|_| bad())?;
        if writer.get_ref().len() > MAX_COMPRESSED {
            return Err(bad());
        }
    }
    let bytes = writer.finish().map_err(|_| bad())?;
    if bytes.len() > MAX_COMPRESSED {
        return Err(bad());
    }
    Ok(bytes)
}
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn integer(&mut self) -> Result<u64, CacheAdapterError> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let byte = *self.bytes.get(self.pos).ok_or_else(bad)?;
            self.pos += 1;
            if shift == 63 && byte > 1 {
                return Err(bad());
            }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(bad())
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8], CacheAdapterError> {
        let end = self.pos.checked_add(size).ok_or_else(bad)?;
        let result = self.bytes.get(self.pos..end).ok_or_else(bad)?;
        self.pos = end;
        Ok(result)
    }
    fn data(&mut self) -> Result<&'a [u8], CacheAdapterError> {
        let size = usize::try_from(self.integer()?).map_err(|_| bad())?;
        self.take(size)
    }
    fn skip(&mut self, kind: u64) -> Result<(), CacheAdapterError> {
        match kind {
            0 => {
                self.integer()?;
            }
            1 => {
                self.take(8)?;
            }
            2 => {
                self.data()?;
            }
            5 => {
                self.take(4)?;
            }
            _ => return Err(bad()),
        }
        Ok(())
    }
}
fn decode_entry(
    bytes: &[u8],
    budget: &mut usize,
) -> Result<NativeSnapshotEntry, CacheAdapterError> {
    let mut input = Cursor { bytes, pos: 0 };
    let mut data = [Vec::new(), Vec::new(), Vec::new()];
    let mut times = [0_i64; 3];
    let mut seen = 0;
    while input.pos < bytes.len() {
        let tag = input.integer()?;
        let number = tag >> 3;
        let kind = tag & 7;
        if number == 0 {
            return Err(bad());
        }
        if number <= 6 {
            let bit = 1 << number;
            if seen & bit != 0 {
                return Err(bad());
            }
            seen |= bit;
            match number {
                1 | 2 | 6 if kind == 2 => {
                    let index = if number == 6 {
                        2
                    } else {
                        (number - 1) as usize
                    };
                    let value = input.data()?;
                    charge(budget, value.len())?;
                    data[index] = value.to_vec();
                }
                3..=5 if kind == 0 => {
                    let index = match number {
                        3 => 2,
                        4 => 1,
                        _ => 0,
                    };
                    times[index] = i64::try_from(input.integer()?).map_err(|_| bad())?;
                }
                _ => return Err(bad()),
            }
        } else {
            input.skip(kind)?;
        }
    }
    let [key, response, domain_set] = data;
    Ok(NativeSnapshotEntry {
        key,
        response,
        domain_set,
        times: [0; 3],
        wall_times: Some(times),
    })
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<NativeSnapshotEntry>, CacheAdapterError> {
    if bytes.len() > MAX_COMPRESSED {
        return Err(bad());
    }
    let mut reader = GzDecoder::new(bytes);
    if reader.header().and_then(|header| header.filename()) != Some(NAME) {
        return Err(bad());
    }
    let mut entries = Vec::new();
    let mut budget = 0;
    loop {
        let mut length = [0; 8];
        // Reading to EOF also verifies the complete CRC and gzip footer.
        let count = reader.read(&mut length[..1]).map_err(|_| bad())?;
        if count == 0 {
            break;
        }
        reader.read_exact(&mut length[1..]).map_err(|_| bad())?;
        let size = usize::try_from(u64::from_be_bytes(length)).map_err(|_| bad())?;
        if size > MAX_BLOCK {
            return Err(bad());
        }
        charge(&mut budget, 8 + size)?;
        let mut block = vec![0; size];
        reader.read_exact(&mut block).map_err(|_| bad())?;
        let mut cursor = Cursor {
            bytes: &block,
            pos: 0,
        };
        while cursor.pos < size {
            let tag = cursor.integer()?;
            if tag == 10 {
                if entries.len() >= MAX_ENTRIES {
                    return Err(bad());
                }
                entries.push(decode_entry(cursor.data()?, &mut budget)?);
            } else if tag >> 3 != 0 {
                cursor.skip(tag & 7)?;
            } else {
                return Err(bad());
            }
        }
    }
    if !reader.into_inner().is_empty() {
        return Err(bad());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gzip(payload: &[u8]) -> Vec<u8> {
        let mut writer = GzBuilder::new()
            .filename(NAME)
            .write(Vec::new(), Compression::default());
        writer.write_all(payload).unwrap();
        writer.finish().unwrap()
    }
    #[test]
    fn stream_owned_and_entry_limits_apply_before_any_merge() {
        let mut writer = GzBuilder::new()
            .filename(NAME)
            .write(Vec::new(), Compression::fast());
        let mut block = Vec::new();
        field(&mut block, 7, &vec![0; MAX_BLOCK - 16]);
        for _ in 0..65 {
            writer
                .write_all(&(block.len() as u64).to_be_bytes())
                .unwrap();
            writer.write_all(&block).unwrap();
        }
        assert!(decode(&writer.finish().unwrap()).is_err());
        let payload = [10, 0].repeat(MAX_ENTRIES + 1);
        let mut framed = (payload.len() as u64).to_be_bytes().to_vec();
        framed.extend_from_slice(&payload);
        assert!(decode(&gzip(&framed)).is_err());
        let item = NativeSnapshotEntry {
            key: vec![],
            response: vec![],
            domain_set: vec![],
            times: [0; 3],
            wall_times: Some([0; 3]),
        };
        assert!(encode(&vec![item; MAX_ENTRIES + 1]).is_err());
    }
    #[test]
    fn complete_footer_name_framing_limits_and_trailing_members_are_validated() {
        let empty = encode(&[]).unwrap();
        assert!(decode(&empty).unwrap().is_empty());
        for length in 0..empty.len() {
            assert!(decode(&empty[..length]).is_err());
        }
        let mut corrupt = empty.clone();
        let index = corrupt.len() - 8;
        corrupt[index] ^= 1;
        assert!(decode(&corrupt).is_err());
        let mut wrong_name = empty.clone();
        wrong_name[10] ^= 1;
        assert!(decode(&wrong_name).is_err());
        let mut trailing = empty.clone();
        trailing.extend_from_slice(&empty);
        assert!(decode(&trailing).is_err());
        assert!(decode(&gzip(&((MAX_BLOCK + 1) as u64).to_be_bytes())).is_err());
        assert!(decode(&gzip(&[0; 7])).is_err());
        assert!(decode(&vec![0; MAX_COMPRESSED + 1]).is_err());
        let mut malformed = vec![0; 8];
        malformed[7] = 2;
        malformed.extend_from_slice(&[10, 3]);
        assert!(decode(&gzip(&malformed)).is_err());
    }
}
