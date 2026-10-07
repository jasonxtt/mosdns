//! Read-only decoders for the sing-box `.srs` rule-set subset that the
//! `sd_set_light`, `domain_set_light` and `si_set` providers consume.
//!
//! This mirrors the lenient Go readers: only domain items (full/suffix from the
//! succinct domain set, keyword, regexp) and IP-CIDR items are read, a header
//! that is not `SRS` v1..=3 + zlib is rejected, and anything after the header
//! that cannot be read ends decoding with the rules read so far (`complete`
//! is then `false`). Unlike the Go readers every size is bounded: the
//! decompressed stream is capped, declared lengths never pre-allocate, and the
//! succinct-set walk cannot loop or recurse without bound.

use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use flate2::bufread::ZlibDecoder;

const MAGIC: [u8; 3] = *b"SRS";
const MAX_VERSION: u8 = 3;
const ITEM_DOMAIN: u8 = 2;
const ITEM_KEYWORD: u8 = 3;
const ITEM_REGEX: u8 = 4;
const ITEM_IPCIDR: u8 = 6;
const ITEM_FINAL: u8 = 0xFF;

/// Decompressed bytes accepted from one file.
pub const MAX_INFLATED_BYTES: u64 = 64 * 1024 * 1024;
/// The longest reversed domain key the succinct-set walk will build.
const MAX_KEY_LEN: usize = 1024;
/// Rules, strings and prefixes one file may expand to. Real rule sets are two
/// orders of magnitude smaller; the budget keeps a file that is small on disk
/// (zero-length strings, an all-ones bitmap, wide IP ranges) from becoming
/// tens of millions of objects.
pub const MAX_ITEMS: usize = 2_000_000;
/// 64-bit words of one succinct-set bitmap (about 16.7 million bits).
const MAX_SET_WORDS: u64 = 1 << 18;
/// Label bytes of one succinct set, and key bytes the walk may materialize.
const MAX_SET_BYTES: u64 = 16 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 64 * 1024 * 1024;
const PREFIX_LABEL: u8 = b'\r';
const ROOT_LABEL: u8 = b'\n';

/// A header-level rejection (the Go readers' `ok == false`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SrsError {
    /// Not an SRS file (magic mismatch or too short).
    NotSrs,
    /// An SRS version newer than the supported maximum.
    UnsupportedVersion(u8),
    /// The zlib header or the rule count could not be read.
    BadStream,
}

impl std::fmt::Display for SrsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSrs => write!(f, "not an SRS file"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported SRS version {v}"),
            Self::BadStream => write!(f, "unreadable SRS stream"),
        }
    }
}

impl std::error::Error for SrsError {}

/// Domain rules in provider text form: `full:`, `domain:`, `keyword:`,
/// `regexp:` (regexp only when requested).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DomainSrs {
    pub rules: Vec<String>,
    /// `false` when decoding stopped before the declared rules were read.
    pub complete: bool,
}

/// IP prefixes decoded from the CIDR ranges of an SRS file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IpSrs {
    pub prefixes: Vec<(IpAddr, u8)>,
    pub complete: bool,
}

struct Body<R: BufRead> {
    reader: R,
    /// Objects this file may still expand to (see [`MAX_ITEMS`]).
    budget: usize,
    /// Key bytes this file may still materialize across all its sets (see
    /// [`MAX_KEY_BYTES`]); shared, so many small sets cannot add up.
    key_bytes: usize,
}

type Inflated<'a> = BufReader<io::Take<ZlibDecoder<&'a [u8]>>>;

fn open(bytes: &[u8]) -> Result<(Body<Inflated<'_>>, u64), SrsError> {
    if bytes.len() < 4 || bytes[..3] != MAGIC {
        return Err(SrsError::NotSrs);
    }
    let version = bytes[3];
    if version > MAX_VERSION {
        return Err(SrsError::UnsupportedVersion(version));
    }
    let inflater = ZlibDecoder::new(&bytes[4..]).take(MAX_INFLATED_BYTES);
    let mut body = Body {
        reader: BufReader::new(inflater),
        budget: MAX_ITEMS,
        key_bytes: MAX_KEY_BYTES,
    };
    let count = body.uvarint().map_err(|_| SrsError::BadStream)?;
    Ok((body, count))
}

impl<R: BufRead> Body<R> {
    fn byte(&mut self) -> io::Result<u8> {
        let mut b = [0u8; 1];
        self.reader.read_exact(&mut b)?;
        Ok(b[0])
    }

    fn uvarint(&mut self) -> io::Result<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        for index in 0..10 {
            let b = self.byte()?;
            if b < 0x80 {
                if index == 9 && b > 1 {
                    break;
                }
                return Ok(value | (u64::from(b) << shift));
            }
            value |= u64::from(b & 0x7f) << shift;
            shift += 7;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "uvarint overflow",
        ))
    }

    /// Reads `len` raw bytes without trusting `len` for allocation.
    fn bytes(&mut self, len: u64) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let read = (&mut self.reader).take(len).read_to_end(&mut out)?;
        if read as u64 != len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(out)
    }

    fn spend(&mut self, count: u64) -> io::Result<()> {
        match usize::try_from(count) {
            Ok(count) if count <= self.budget => {
                self.budget -= count;
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SRS expands beyond the rule budget",
            )),
        }
    }

    fn u64_slice(&mut self) -> io::Result<Vec<u64>> {
        let len = self.uvarint()?;
        if len > MAX_SET_WORDS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "set bitmap too large",
            ));
        }
        let mut out = Vec::new();
        for _ in 0..len {
            let mut b = [0u8; 8];
            self.reader.read_exact(&mut b)?;
            out.push(u64::from_be_bytes(b));
        }
        Ok(out)
    }

    fn byte_slice(&mut self) -> io::Result<Vec<u8>> {
        let len = self.uvarint()?;
        self.bytes(len)
    }

    /// A byte slice whose length is capped before it is read.
    fn capped_byte_slice(&mut self, max: u64) -> io::Result<Vec<u8>> {
        let len = self.uvarint()?;
        if len > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byte slice too large",
            ));
        }
        self.bytes(len)
    }

    fn string_slice(&mut self) -> io::Result<Vec<String>> {
        let len = self.uvarint()?;
        self.spend(len)?;
        let mut out = Vec::new();
        for _ in 0..len {
            let bytes = self.byte_slice()?;
            out.push(String::from_utf8_lossy(&bytes).into_owned());
        }
        Ok(out)
    }
}

/// Decodes the domain items of an SRS file.
///
/// # Errors
///
/// Returns [`SrsError`] when the header is not a supported SRS stream.
pub fn decode_domain_srs(bytes: &[u8], enable_regexp: bool) -> Result<DomainSrs, SrsError> {
    let (mut body, count) = open(bytes)?;
    let mut out = DomainSrs {
        rules: Vec::new(),
        complete: false,
    };
    for _ in 0..count {
        if read_rule(&mut body, &mut |item, body| {
            domain_item(item, body, enable_regexp, &mut out.rules)
        })
        .is_err()
        {
            return Ok(out);
        }
    }
    out.complete = true;
    Ok(out)
}

/// Decodes the IP-CIDR items of an SRS file.
///
/// # Errors
///
/// Returns [`SrsError`] when the header is not a supported SRS stream.
pub fn decode_ip_srs(bytes: &[u8]) -> Result<IpSrs, SrsError> {
    let (mut body, count) = open(bytes)?;
    let mut out = IpSrs {
        prefixes: Vec::new(),
        complete: false,
    };
    for _ in 0..count {
        if read_rule(&mut body, &mut |item, body| {
            ip_item(item, body, &mut out.prefixes)
        })
        .is_err()
        {
            return Ok(out);
        }
    }
    out.complete = true;
    Ok(out)
}

/// What an item reader tells the rule loop.
enum Next {
    Continue,
    /// Stop this rule (the Go `default:` / final handling).
    EndRule,
}

type ItemFn<'a, R> = dyn FnMut(u8, &mut Body<R>) -> io::Result<Next> + 'a;

/// One headless rule: mode 0 is a default rule, mode 1 a logical group whose
/// sub-rules are flattened; any other mode reads nothing (as in Go).
fn read_rule<R: BufRead>(body: &mut Body<R>, item: &mut ItemFn<'_, R>) -> io::Result<()> {
    read_rule_depth(body, item, 0)
}

fn read_rule_depth<R: BufRead>(
    body: &mut Body<R>,
    item: &mut ItemFn<'_, R>,
    depth: u32,
) -> io::Result<()> {
    match body.byte()? {
        0 => loop {
            let id = body.byte()?;
            if id == ITEM_FINAL {
                return Ok(());
            }
            if matches!(item(id, body)?, Next::EndRule) {
                return Ok(());
            }
        },
        1 => {
            if depth >= 16 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "rule nesting"));
            }
            let _mode = body.byte()?;
            let n = body.uvarint()?;
            for _ in 0..n {
                read_rule_depth(body, item, depth + 1)?;
            }
            let _invert = body.byte()?;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn domain_item<R: BufRead>(
    item: u8,
    body: &mut Body<R>,
    enable_regexp: bool,
    rules: &mut Vec<String>,
) -> io::Result<Next> {
    match item {
        ITEM_DOMAIN => {
            let set = SuccinctSet::read(body)?;
            // `dump` charges every key against the file-wide budgets before
            // it is materialized.
            let (domains, suffixes) = set.dump(&mut body.budget, &mut body.key_bytes)?;
            rules.extend(domains.into_iter().map(|d| format!("full:{d}")));
            rules.extend(suffixes.into_iter().map(|d| format!("domain:{d}")));
            Ok(Next::Continue)
        }
        ITEM_KEYWORD => {
            let list = body.string_slice()?;
            rules.extend(list.into_iter().map(|d| format!("keyword:{d}")));
            Ok(Next::Continue)
        }
        ITEM_REGEX => {
            let list = body.string_slice()?;
            if enable_regexp {
                rules.extend(list.into_iter().map(|d| format!("regexp:{d}")));
            }
            Ok(Next::Continue)
        }
        _ => Ok(Next::EndRule),
    }
}

fn ip_item<R: BufRead>(
    item: u8,
    body: &mut Body<R>,
    prefixes: &mut Vec<(IpAddr, u8)>,
) -> io::Result<Next> {
    if item != ITEM_IPCIDR {
        return Ok(Next::EndRule);
    }
    let version = body.byte()?;
    if version != 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "ipset version"));
    }
    let mut len = [0u8; 8];
    body.reader.read_exact(&mut len)?;
    let len = u64::from_be_bytes(len);
    for _ in 0..len {
        body.spend(1)?;
        let from = body.capped_byte_slice(16)?;
        let to = body.capped_byte_slice(16)?;
        if let (Some(from), Some(to)) = (addr(&from), addr(&to)) {
            let before = prefixes.len();
            range_to_prefixes(from, to, prefixes);
            // One unit was spent for the range itself; a wide range can
            // expand to up to 2 * 127 prefixes.
            body.spend((prefixes.len() - before).saturating_sub(1) as u64)?;
        }
    }
    Ok(Next::Continue)
}

fn addr(bytes: &[u8]) -> Option<IpAddr> {
    match bytes.len() {
        4 => Some(IpAddr::V4(Ipv4Addr::new(
            bytes[0], bytes[1], bytes[2], bytes[3],
        ))),
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(bytes);
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

/// Minimal CIDR cover of `[from, to]`; ranges of mixed families or with
/// `from > to` contribute nothing.
fn range_to_prefixes(from: IpAddr, to: IpAddr, out: &mut Vec<(IpAddr, u8)>) {
    match (from, to) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let (mut lo, hi) = (u128::from(u32::from(a)), u128::from(u32::from(b)));
            cover(&mut lo, hi, 32, |start, bits| {
                out.push((IpAddr::V4(Ipv4Addr::from(start as u32)), bits));
            });
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let (mut lo, hi) = (u128::from(a), u128::from(b));
            cover(&mut lo, hi, 128, |start, bits| {
                out.push((IpAddr::V6(Ipv6Addr::from(start)), bits));
            });
        }
        _ => {}
    }
}

fn cover(lo: &mut u128, hi: u128, width: u32, mut emit: impl FnMut(u128, u8)) {
    if *lo > hi {
        return;
    }
    loop {
        // The largest aligned block starting at `lo` that stays within `hi`.
        let align = if *lo == 0 {
            width
        } else {
            lo.trailing_zeros().min(width)
        };
        let span = (hi - *lo).wrapping_add(1);
        let fit = if span == 0 {
            128
        } else {
            127 - span.leading_zeros()
        };
        let host_bits = align.min(fit);
        emit(*lo, (width - host_bits) as u8);
        let block = if host_bits >= 128 {
            u128::MAX
        } else {
            (1u128 << host_bits) - 1
        };
        let last = *lo + block;
        if last >= hi {
            return;
        }
        *lo = last + 1;
    }
}

/// The succinct domain set of sing's `domain.Matcher` (read side only).
struct SuccinctSet {
    leaves: Vec<u64>,
    label_bitmap: Vec<u64>,
    labels: Vec<u8>,
}

impl SuccinctSet {
    fn read<R: BufRead>(body: &mut Body<R>) -> io::Result<Self> {
        let _reserved = body.byte()?;
        Ok(Self {
            leaves: body.u64_slice()?,
            label_bitmap: body.u64_slice()?,
            labels: body.capped_byte_slice(MAX_SET_BYTES)?,
        })
    }

    fn bit(words: &[u64], index: usize) -> bool {
        words
            .get(index >> 6)
            .is_some_and(|w| w & (1 << (index & 63)) != 0)
    }

    /// Keys of the set in traversal order (sing's `succinctSet.keys`). Each
    /// key is charged against the caller's file-wide object and byte budgets.
    fn keys(&self, items: &mut usize, bytes: &mut usize) -> io::Result<Vec<Vec<u8>>> {
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "malformed succinct set");
        let total_bits = self.label_bitmap.len() * 64;
        // `starts[n]` is the bitmap position where node `n`'s labels begin;
        // `zeros_before[i]` counts zero bits below position `i` per word.
        let mut starts: Vec<u32> = vec![0];
        let mut zeros_before: Vec<u32> = Vec::with_capacity(self.label_bitmap.len() + 1);
        let mut zeros = 0u32;
        for (word_index, word) in self.label_bitmap.iter().enumerate() {
            zeros_before.push(zeros);
            zeros += word.count_zeros();
            let mut ones = *word;
            while ones != 0 {
                let position = word_index * 64 + ones.trailing_zeros() as usize;
                starts.push(u32::try_from(position + 1).map_err(|_| bad())?);
                ones &= ones - 1;
            }
        }
        zeros_before.push(zeros);
        let count_zeros = |i: usize| -> usize {
            let word = i >> 6;
            let below = if i & 63 == 0 {
                0
            } else {
                (self.label_bitmap[word] & ((1u64 << (i & 63)) - 1)).count_zeros() as usize
                    - (64 - (i & 63))
            };
            zeros_before[word] as usize + below
        };

        let mut result = Vec::new();
        // (node id, bitmap cursor, key length on entry)
        let mut stack: Vec<(usize, usize, usize)> = vec![(0, 0, 0)];
        let mut key: Vec<u8> = Vec::new();
        let mut visited_entries = 0usize;
        while let Some((node, cursor, key_len)) = stack.pop() {
            key.truncate(key_len);
            // A node with a leaf bit contributes its key once, on first visit.
            if cursor == *starts.get(node).ok_or_else(bad)? as usize
                && Self::bit(&self.leaves, node)
            {
                *items = items.checked_sub(1).ok_or_else(bad)?;
                *bytes = bytes.checked_sub(key.len()).ok_or_else(bad)?;
                result.push(key.clone());
            }
            let position = cursor;
            // Find the next label of this node.
            if position >= total_bits || Self::bit(&self.label_bitmap, position) {
                continue;
            }
            visited_entries += 1;
            if visited_entries > total_bits {
                return Err(bad());
            }
            let label = *self.labels.get(position - node).ok_or_else(bad)?;
            // Resume this node at the next label after the child is done.
            stack.push((node, position + 1, key_len));
            let child = count_zeros(position + 1);
            if child <= node || child >= starts.len() {
                return Err(bad());
            }
            if key_len + 1 > MAX_KEY_LEN {
                return Err(bad());
            }
            key.push(label);
            stack.push((child, starts[child] as usize, key_len + 1));
        }
        Ok(result)
    }

    /// `domain.Matcher.Dump`: full domains and suffixes, each sorted.
    fn dump(&self, items: &mut usize, bytes: &mut usize) -> io::Result<(Vec<String>, Vec<String>)> {
        let mut domains: BTreeSet<String> = BTreeSet::new();
        let mut prefixes: BTreeSet<String> = BTreeSet::new();
        let mut suffixes: Vec<String> = Vec::new();
        for raw in self.keys(items, bytes)? {
            // Keys are stored reversed; undo it rune-wise like Go.
            let text = String::from_utf8(raw)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 key"))?;
            let key: String = text.chars().rev().collect();
            let Some(first) = key.bytes().next() else {
                continue;
            };
            if first == PREFIX_LABEL {
                prefixes.insert(key[1..].to_owned());
            } else if first == ROOT_LABEL {
                suffixes.push(key[1..].to_owned());
            } else {
                domains.insert(key);
            }
        }
        for raw in prefixes {
            if let Some(root) = raw.strip_prefix('.') {
                if domains.remove(root) {
                    suffixes.push(root.to_owned());
                    continue;
                }
            }
            suffixes.push(raw);
        }
        suffixes.sort();
        Ok((domains.into_iter().collect(), suffixes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_srs_and_future_versions() {
        assert_eq!(decode_domain_srs(b"hello", false), Err(SrsError::NotSrs));
        assert_eq!(decode_domain_srs(b"SR", false), Err(SrsError::NotSrs));
        assert_eq!(
            decode_domain_srs(b"SRS\x09", false),
            Err(SrsError::UnsupportedVersion(9))
        );
        assert_eq!(decode_ip_srs(b"SRS\x01not-zlib"), Err(SrsError::BadStream));
    }

    #[test]
    fn range_cover_is_minimal_and_exact() {
        let mut out = Vec::new();
        range_to_prefixes(
            "10.0.0.1".parse().unwrap(),
            "10.0.0.6".parse().unwrap(),
            &mut out,
        );
        let got: Vec<String> = out.iter().map(|(a, b)| format!("{a}/{b}")).collect();
        assert_eq!(
            got,
            ["10.0.0.1/32", "10.0.0.2/31", "10.0.0.4/31", "10.0.0.6/32"]
        );
        let mut out = Vec::new();
        range_to_prefixes(
            "0.0.0.0".parse().unwrap(),
            "255.255.255.255".parse().unwrap(),
            &mut out,
        );
        assert_eq!(out, [("0.0.0.0".parse().unwrap(), 0)]);
        let mut out = Vec::new();
        range_to_prefixes("::".parse().unwrap(), "::ffff".parse().unwrap(), &mut out);
        assert_eq!(out, [("::".parse().unwrap(), 112)]);
        let mut out = Vec::new();
        range_to_prefixes(
            "10.0.0.5".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
            &mut out,
        );
        range_to_prefixes(
            "10.0.0.1".parse().unwrap(),
            "::1".parse().unwrap(),
            &mut out,
        );
        assert!(out.is_empty());
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn srs(body: &[u8]) -> Vec<u8> {
        let mut out = b"SRS\x03".to_vec();
        out.extend(zlib(body));
        out
    }

    #[test]
    fn keyword_and_regexp_items_and_the_regexp_switch() {
        // 1 rule: default, keyword ["ad"], regexp ["^a.*$"], final.
        let mut body = vec![1, 0, ITEM_KEYWORD, 1, 2, b'a', b'd', ITEM_REGEX, 1, 5];
        body.extend(b"^a.*$");
        body.push(ITEM_FINAL);
        let file = srs(&body);
        let with = decode_domain_srs(&file, true).unwrap();
        assert_eq!(with.rules, ["keyword:ad", "regexp:^a.*$"]);
        assert!(with.complete);
        let without = decode_domain_srs(&file, false).unwrap();
        assert_eq!(without.rules, ["keyword:ad"]);
    }

    #[test]
    fn truncated_bodies_return_what_was_read_and_are_incomplete() {
        let body = vec![2, 0, ITEM_KEYWORD, 1, 2, b'a', b'd', ITEM_FINAL];
        let got = decode_domain_srs(&srs(&body), true).unwrap();
        assert_eq!(got.rules, ["keyword:ad"]);
        assert!(!got.complete, "the second declared rule is missing");
    }

    #[test]
    fn oversized_declared_lengths_do_not_allocate_or_hang() {
        // keyword list claiming 2^60 strings with no data behind it.
        let mut body = vec![1, 0, ITEM_KEYWORD];
        body.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10]);
        let got = decode_domain_srs(&srs(&body), true).unwrap();
        assert!(got.rules.is_empty() && !got.complete);
        // succinct set with a huge bitmap length and no data.
        let mut body = vec![1, 0, ITEM_DOMAIN, 0, 0xff, 0xff, 0xff, 0xff, 0x0f];
        let got = decode_domain_srs(&srs(&body), true).unwrap();
        assert!(got.rules.is_empty() && !got.complete);
        body.clear();
    }

    /// A set whose 255 keys all share a 1000-byte prefix (255,255 key bytes).
    fn long_prefix_set() -> SuccinctSet {
        let mut bits: Vec<bool> = Vec::new();
        for _ in 0..1000 {
            bits.extend([false, true]); // a chain node with one label
        }
        bits.extend(std::iter::repeat_n(false, 255)); // the fan-out node
        bits.push(true);
        bits.extend(std::iter::repeat_n(true, 255)); // leaf nodes: no labels
        let mut label_bitmap = vec![0u64; bits.len().div_ceil(64)];
        for (index, bit) in bits.iter().enumerate() {
            if *bit {
                label_bitmap[index / 64] |= 1 << (index % 64);
            }
        }
        let mut leaves = vec![0u64; 1256usize.div_ceil(64)];
        for node in 1001..1256 {
            leaves[node / 64] |= 1 << (node % 64);
        }
        let mut labels = vec![b'a'; 1000];
        labels.extend(0..=254u8);
        SuccinctSet {
            leaves,
            label_bitmap,
            labels,
        }
    }

    #[test]
    fn key_bytes_are_one_budget_across_all_sets_of_a_file() {
        let set = long_prefix_set();
        let (mut items, mut bytes) = (MAX_ITEMS, 300_000usize);
        assert_eq!(set.keys(&mut items, &mut bytes).unwrap().len(), 255);
        assert_eq!(bytes, 300_000 - 255 * 1001);
        // A second set no longer fits the same file-wide budget.
        assert!(set.keys(&mut items, &mut bytes).is_err());
        // Alone, with the real budget, many such sets are bounded: 64 MiB
        // allows 261 of them, never 1,024.
        let (mut items, mut bytes) = (MAX_ITEMS, MAX_KEY_BYTES);
        let mut accepted = 0;
        while set.keys(&mut items, &mut bytes).is_ok() {
            accepted += 1;
        }
        assert!((200..300).contains(&accepted), "{accepted}");
    }

    #[test]
    fn a_compression_bomb_is_cut_at_the_inflation_cap() {
        // One keyword list whose first string claims 2^40 bytes, followed by
        // far more zeros than the cap allows.
        let mut body = vec![1, 0, ITEM_KEYWORD, 1, 0x80, 0x80, 0x80, 0x80, 0x80, 0x20];
        body.resize(body.len() + (MAX_INFLATED_BYTES as usize) + 4096, 0);
        let got = decode_domain_srs(&srs(&body), true).unwrap();
        assert!(got.rules.is_empty() && !got.complete);
    }

    fn uvarint_bytes(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while value >= 0x80 {
            out.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
        out
    }

    /// An SRS holding one rule made of `item` bytes.
    fn one_rule(items: &[u8]) -> Vec<u8> {
        let mut body = vec![1, 0];
        body.extend_from_slice(items);
        body.push(ITEM_FINAL);
        srs(&body)
    }

    #[test]
    fn a_flood_of_zero_length_strings_hits_the_rule_budget() {
        // 3,000,000 empty keywords: a few MB inflated, a huge object count.
        let mut items = vec![ITEM_KEYWORD];
        items.extend(uvarint_bytes(3_000_000));
        items.resize(items.len() + 3_000_000, 0);
        let got = decode_domain_srs(&one_rule(&items), true).unwrap();
        assert!(got.rules.is_empty() && !got.complete);
        // Just under the budget is accepted.
        let mut items = vec![ITEM_KEYWORD];
        items.extend(uvarint_bytes(500_000));
        items.resize(items.len() + 500_000, 0);
        let got = decode_domain_srs(&one_rule(&items), true).unwrap();
        assert_eq!(got.rules.len(), 500_000);
        assert!(got.complete);
    }

    #[test]
    fn an_oversized_or_all_ones_bitmap_is_refused_before_it_expands() {
        // Reserved, leaves [], bitmap with MAX_SET_WORDS + 1 words.
        let mut items = vec![ITEM_DOMAIN, 0, 0];
        items.extend(uvarint_bytes(262_145));
        items.resize(items.len() + 262_145 * 8, 0xff);
        items.push(0);
        let got = decode_domain_srs(&one_rule(&items), true).unwrap();
        assert!(got.rules.is_empty() && !got.complete);
        // At the cap, an all-ones bitmap has no labels to read and yields no
        // keys; it must not allocate per bit beyond the cap.
        let mut items = vec![ITEM_DOMAIN, 0, 0];
        items.extend(uvarint_bytes(262_144));
        items.resize(items.len() + 262_144 * 8, 0xff);
        items.push(0);
        let got = decode_domain_srs(&one_rule(&items), true).unwrap();
        assert!(got.rules.is_empty());
    }

    #[test]
    fn wide_ip_ranges_hit_the_prefix_budget() {
        // 100,000 ranges 0.0.0.1 - 255.255.255.254, 62 prefixes each.
        let mut items = vec![ITEM_IPCIDR, 1];
        items.extend(100_000u64.to_be_bytes());
        for _ in 0..100_000 {
            items.extend([4, 0, 0, 0, 1, 4, 255, 255, 255, 254]);
        }
        let got = decode_ip_srs(&one_rule(&items)).unwrap();
        assert!(!got.complete);
        assert!(
            got.prefixes.len() <= MAX_ITEMS + 256,
            "{}",
            got.prefixes.len()
        );
    }

    #[test]
    fn a_cyclic_succinct_set_fails_instead_of_looping() {
        // Reserved, leaves [1], bitmap [0] (all zeros: every label has a child
        // that points back), labels of 64 bytes.
        let mut body = vec![1, 0, ITEM_DOMAIN, 0, 1];
        body.extend(1u64.to_be_bytes());
        body.push(1);
        body.extend(0u64.to_be_bytes());
        body.push(64);
        body.extend([b'a'; 64]);
        body.push(ITEM_FINAL);
        let got = decode_domain_srs(&srs(&body), true).unwrap();
        assert!(!got.complete);
    }
}
