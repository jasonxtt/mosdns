//! Minimal, bounded archive readers and writers for the updater.
//!
//! No `zip`/`tar` crate is available offline, and the updater only needs three small
//! things: pull the `mosdns` executable out of a release `tar.gz`, read a configuration
//! package (a zip of stored or deflated files), and write a stored zip for the
//! configuration export. Every reader bounds entry counts and sizes before it
//! allocates, rejects anything it does not understand, and never trusts a name.

use std::io::Read;

use flate2::Crc;
use flate2::read::{DeflateDecoder, GzDecoder};

const BLOCK: usize = 512;

/// The regular file named `wanted` (compared by its last path component) from a
/// gzip-compressed tar stream.
///
/// `max_file` bounds the file; `max_scan` bounds how much decompressed data is looked at
/// altogether, so a hostile stream cannot make the reader work without end.
pub(crate) fn extract_tar_gz_file(
    data: &[u8],
    wanted: &str,
    max_file: u64,
    max_scan: u64,
) -> Result<Vec<u8>, String> {
    let mut stream = GzDecoder::new(data).take(max_scan.saturating_add(1));
    let mut scanned: u64 = 0;
    let mut long_name: Option<String> = None;
    loop {
        let mut header = [0_u8; BLOCK];
        if !read_exact_or_end(&mut stream, &mut header)? {
            return Err(format!("`{wanted}` not found in the archive"));
        }
        scanned += BLOCK as u64;
        if header.iter().all(|byte| *byte == 0) {
            return Err(format!("`{wanted}` not found in the archive"));
        }
        let size = parse_octal(&header[124..136])?;
        let kind = header[156];
        let padded = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        scanned = scanned.saturating_add(padded);
        if scanned > max_scan {
            return Err("archive is larger than the allowed scan size".to_owned());
        }
        match kind {
            // GNU long name: the data is the next entry's name.
            b'L' => {
                if size > 4096 {
                    return Err("archive entry name is too long".to_owned());
                }
                let mut name = read_block_data(&mut stream, size, padded)?;
                while name.last() == Some(&0) {
                    name.pop();
                }
                long_name = Some(String::from_utf8(name).map_err(|_| "archive name is not UTF-8")?);
            }
            b'0' | 0 => {
                let name = match long_name.take() {
                    Some(name) => name,
                    None => header_name(&header),
                };
                let base = name.rsplit('/').next().unwrap_or(&name);
                if base == wanted {
                    if size == 0 || size > max_file {
                        return Err(format!(
                            "`{wanted}` has an unacceptable size ({size} bytes)"
                        ));
                    }
                    return read_block_data(&mut stream, size, padded);
                }
                skip(&mut stream, padded)?;
            }
            // Extended headers, links, directories and everything else: not the file.
            _ => {
                long_name = None;
                skip(&mut stream, padded)?;
            }
        }
    }
}

fn header_name(header: &[u8; BLOCK]) -> String {
    let field = |range: std::ops::Range<usize>| {
        let bytes = &header[range];
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    let name = field(0..100);
    let ustar = &header[257..262] == b"ustar";
    let prefix = if ustar {
        field(345..500)
    } else {
        String::new()
    };
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn parse_octal(field: &[u8]) -> Result<u64, String> {
    // A base-256 size (high bit set) means a file over 8 GiB: never a release binary.
    if field.first().is_some_and(|byte| byte & 0x80 != 0) {
        return Err("archive entry is too large".to_owned());
    }
    let text: String = field
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| "archive header is malformed".to_owned())
}

/// Fills `buffer`; `Ok(false)` at a clean end of the stream.
fn read_exact_or_end(stream: &mut impl Read, buffer: &mut [u8]) -> Result<bool, String> {
    let mut filled = 0;
    while filled < buffer.len() {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err("archive is truncated".to_owned()),
            Ok(read) => filled += read,
            Err(error) => return Err(format!("archive read failed: {error}")),
        }
    }
    Ok(true)
}

fn read_block_data(stream: &mut impl Read, size: u64, padded: u64) -> Result<Vec<u8>, String> {
    let size = usize::try_from(size).map_err(|_| "archive entry is too large")?;
    let mut data = vec![0_u8; size];
    stream
        .read_exact(&mut data)
        .map_err(|error| format!("archive is truncated: {error}"))?;
    skip(stream, padded - size as u64)?;
    Ok(data)
}

fn skip(stream: &mut impl Read, mut count: u64) -> Result<(), String> {
    let mut sink = [0_u8; 8192];
    while count > 0 {
        let want = usize::try_from(count.min(sink.len() as u64)).unwrap_or(sink.len());
        stream
            .read_exact(&mut sink[..want])
            .map_err(|error| format!("archive is truncated: {error}"))?;
        count -= want as u64;
    }
    Ok(())
}

/// Bounds for [`read_zip`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ZipLimits {
    pub(crate) max_entries: usize,
    pub(crate) max_entry_bytes: u64,
    pub(crate) max_total_bytes: u64,
}

/// One file of a zip (directories are not returned).
#[derive(Debug)]
pub(crate) struct ZipFile {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
}

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn le16(data: &[u8], at: usize) -> Result<u16, String> {
    data.get(at..at + 2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .ok_or_else(|| "zip is truncated".to_owned())
}

fn le32(data: &[u8], at: usize) -> Result<u32, String> {
    data.get(at..at + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| "zip is truncated".to_owned())
}

/// Reads every regular file of a zip (stored or deflate, no encryption, no zip64).
pub(crate) fn read_zip(data: &[u8], limits: ZipLimits) -> Result<Vec<ZipFile>, String> {
    // The end-of-central-directory record sits in the last 64 KiB + 22 bytes.
    let tail_start = data.len().saturating_sub(65_557);
    let eocd = (tail_start..data.len().saturating_sub(21))
        .rev()
        .find(|&at| le32(data, at) == Ok(EOCD))
        .ok_or("not a zip file")?;
    let disk = le16(data, eocd + 4)?;
    let cd_disk = le16(data, eocd + 6)?;
    let entries = le16(data, eocd + 10)?;
    let cd_size = le32(data, eocd + 12)?;
    let cd_offset = le32(data, eocd + 16)?;
    if disk != 0 || cd_disk != 0 || entries == 0xFFFF || cd_offset == 0xFFFF_FFFF {
        return Err("multi-disk or zip64 archives are not supported".to_owned());
    }
    let entries = usize::from(entries);
    if entries > limits.max_entries {
        return Err(format!("zip has more than {} entries", limits.max_entries));
    }
    let cd_offset = cd_offset as usize;
    let cd_end = cd_offset
        .checked_add(cd_size as usize)
        .filter(|end| *end <= data.len())
        .ok_or("zip central directory is out of range")?;
    let mut at = cd_offset;
    let mut files = Vec::new();
    let mut total: u64 = 0;
    for _ in 0..entries {
        if at + 46 > cd_end || le32(data, at)? != CENTRAL {
            return Err("zip central directory is malformed".to_owned());
        }
        let made_by = le16(data, at + 4)?;
        let flags = le16(data, at + 8)?;
        let method = le16(data, at + 10)?;
        let crc = le32(data, at + 16)?;
        let compressed = u64::from(le32(data, at + 20)?);
        let size = u64::from(le32(data, at + 24)?);
        let name_len = usize::from(le16(data, at + 28)?);
        let extra_len = usize::from(le16(data, at + 30)?);
        let comment_len = usize::from(le16(data, at + 32)?);
        let attributes = le32(data, at + 38)?;
        let local = le32(data, at + 42)? as usize;
        let name_bytes = data
            .get(at + 46..at + 46 + name_len)
            .ok_or("zip is truncated")?;
        at += 46 + name_len + extra_len + comment_len;
        if at > cd_end {
            return Err("zip central directory is malformed".to_owned());
        }
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| "zip entry name is not UTF-8".to_owned())?
            .to_owned();
        if flags & 0x0001 != 0 {
            return Err(format!("zip entry `{name}` is encrypted"));
        }
        if name.ends_with('/') {
            continue;
        }
        // Unix mode in the high half of the attributes when the archive was made on Unix.
        if made_by >> 8 == 3 {
            let mode = attributes >> 16;
            let kind = mode & 0o170_000;
            if kind != 0 && kind != 0o100_000 {
                return Err(format!("zip entry `{name}` is not a regular file"));
            }
        }
        if size > limits.max_entry_bytes {
            return Err(format!("zip entry `{name}` is too large"));
        }
        total = total.saturating_add(size);
        if total > limits.max_total_bytes {
            return Err("zip is larger than the allowed total size".to_owned());
        }
        if le32(data, local)? != LOCAL {
            return Err(format!("zip entry `{name}` has no local header"));
        }
        let local_name = usize::from(le16(data, local + 26)?);
        let local_extra = usize::from(le16(data, local + 28)?);
        let start = local + 30 + local_name + local_extra;
        let end = start
            .checked_add(usize::try_from(compressed).map_err(|_| "zip is too large")?)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| format!("zip entry `{name}` is truncated"))?;
        let body = &data[start..end];
        let content = match method {
            0 => {
                if compressed != size {
                    return Err(format!("zip entry `{name}` has inconsistent sizes"));
                }
                body.to_vec()
            }
            8 => {
                let mut out = Vec::with_capacity(usize::try_from(size.min(1 << 20)).unwrap_or(0));
                DeflateDecoder::new(body)
                    .take(size + 1)
                    .read_to_end(&mut out)
                    .map_err(|error| format!("zip entry `{name}` cannot be inflated: {error}"))?;
                out
            }
            other => {
                return Err(format!(
                    "zip entry `{name}` uses unsupported method {other}"
                ));
            }
        };
        if content.len() as u64 != size {
            return Err(format!(
                "zip entry `{name}` does not match its declared size"
            ));
        }
        let mut check = Crc::new();
        check.update(&content);
        if check.sum() != crc {
            return Err(format!("zip entry `{name}` fails its CRC check"));
        }
        files.push(ZipFile {
            name,
            data: content,
        });
    }
    Ok(files)
}

/// A stored (uncompressed) zip built in memory, for the configuration export.
#[derive(Default)]
pub(crate) struct ZipWriter {
    buffer: Vec<u8>,
    entries: Vec<(String, u32, u32, u32)>,
}

impl ZipWriter {
    /// Bytes written so far (local headers and file data).
    pub(crate) fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Adds a file. Fails when the archive would exceed what plain zip can address.
    pub(crate) fn add(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        let size = u32::try_from(data.len()).map_err(|_| format!("`{name}` is too large"))?;
        let offset = u32::try_from(self.buffer.len()).map_err(|_| "archive is too large")?;
        if self.entries.len() >= 0xFFFE {
            return Err("archive has too many files".to_owned());
        }
        let mut crc = Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let name_len = u16::try_from(name.len()).map_err(|_| "file name is too long")?;
        let buffer = &mut self.buffer;
        buffer.extend_from_slice(&LOCAL.to_le_bytes());
        buffer.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        buffer.extend_from_slice(&0x0800_u16.to_le_bytes()); // UTF-8 names
        buffer.extend_from_slice(&0_u16.to_le_bytes()); // stored
        buffer.extend_from_slice(&0_u16.to_le_bytes()); // time
        buffer.extend_from_slice(&0x0021_u16.to_le_bytes()); // date 1980-01-01
        buffer.extend_from_slice(&crc.to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
        buffer.extend_from_slice(&name_len.to_le_bytes());
        buffer.extend_from_slice(&0_u16.to_le_bytes()); // extra
        buffer.extend_from_slice(name.as_bytes());
        buffer.extend_from_slice(data);
        self.entries.push((name.to_owned(), crc, size, offset));
        Ok(())
    }

    /// Appends the central directory and returns the archive.
    pub(crate) fn finish(mut self) -> Result<Vec<u8>, String> {
        let cd_offset = u32::try_from(self.buffer.len()).map_err(|_| "archive is too large")?;
        let entries = std::mem::take(&mut self.entries);
        let buffer = &mut self.buffer;
        for (name, crc, size, offset) in &entries {
            buffer.extend_from_slice(&CENTRAL.to_le_bytes());
            buffer.extend_from_slice(&0x031E_u16.to_le_bytes()); // made by Unix, 3.0
            buffer.extend_from_slice(&20_u16.to_le_bytes());
            buffer.extend_from_slice(&0x0800_u16.to_le_bytes());
            buffer.extend_from_slice(&0_u16.to_le_bytes());
            buffer.extend_from_slice(&0_u16.to_le_bytes());
            buffer.extend_from_slice(&0x0021_u16.to_le_bytes());
            buffer.extend_from_slice(&crc.to_le_bytes());
            buffer.extend_from_slice(&size.to_le_bytes());
            buffer.extend_from_slice(&size.to_le_bytes());
            buffer.extend_from_slice(&(name.len() as u16).to_le_bytes());
            buffer.extend_from_slice(&0_u16.to_le_bytes()); // extra
            buffer.extend_from_slice(&0_u16.to_le_bytes()); // comment
            buffer.extend_from_slice(&0_u16.to_le_bytes()); // disk
            buffer.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
            buffer.extend_from_slice(&((0o100_644_u32) << 16).to_le_bytes());
            buffer.extend_from_slice(&offset.to_le_bytes());
            buffer.extend_from_slice(name.as_bytes());
        }
        let cd_size = u32::try_from(buffer.len())
            .ok()
            .and_then(|end| end.checked_sub(cd_offset))
            .ok_or("archive is too large")?;
        buffer.extend_from_slice(&EOCD.to_le_bytes());
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        buffer.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&cd_size.to_le_bytes());
        buffer.extend_from_slice(&cd_offset.to_le_bytes());
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        Ok(self.buffer)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::{DeflateEncoder, GzEncoder};

    use super::*;

    const LIMITS: ZipLimits = ZipLimits {
        max_entries: 16,
        max_entry_bytes: 1 << 20,
        max_total_bytes: 4 << 20,
    };

    fn tar_header(name: &str, size: usize, kind: u8) -> [u8; BLOCK] {
        let mut header = [0_u8; BLOCK];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000755");
        header[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        header[156] = kind;
        header[257..262].copy_from_slice(b"ustar");
        header
    }

    fn tar_gz(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (name, kind, data) in entries {
            tar.extend_from_slice(&tar_header(name, data.len(), *kind));
            tar.extend_from_slice(data);
            tar.resize(tar.len().div_ceil(BLOCK) * BLOCK, 0);
        }
        tar.extend_from_slice(&[0_u8; 2 * BLOCK]);
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn the_executable_is_found_by_its_last_path_component_and_other_entries_are_skipped() {
        let archive = tar_gz(&[
            ("./", b'5', b""),
            ("pax", b'x', b"30 mtime=1\n"),
            ("README", b'0', &[1; 700]),
            ("./bin/mosdns", b'0', b"ELF-binary"),
        ]);
        assert_eq!(
            extract_tar_gz_file(&archive, "mosdns", 1 << 20, 1 << 20).unwrap(),
            b"ELF-binary"
        );
        assert!(
            extract_tar_gz_file(&archive, "missing", 1 << 20, 1 << 20)
                .unwrap_err()
                .contains("not found")
        );
    }

    #[test]
    fn a_directory_or_link_named_like_the_executable_is_not_the_executable() {
        let archive = tar_gz(&[("mosdns", b'5', b""), ("x/mosdns", b'2', b"")]);
        assert!(extract_tar_gz_file(&archive, "mosdns", 1 << 20, 1 << 20).is_err());
    }

    #[test]
    fn size_and_scan_limits_and_truncation_are_errors_not_allocations() {
        let archive = tar_gz(&[("mosdns", b'0', &[7; 4000])]);
        assert!(
            extract_tar_gz_file(&archive, "mosdns", 3999, 1 << 20)
                .unwrap_err()
                .contains("size")
        );
        assert!(extract_tar_gz_file(&archive, "mosdns", 1 << 20, 1024).is_err());
        assert!(
            extract_tar_gz_file(&archive[..archive.len() / 2], "mosdns", 1 << 20, 1 << 20).is_err()
        );
        assert!(extract_tar_gz_file(b"not gzip", "mosdns", 1 << 20, 1 << 20).is_err());
        // An absurd declared size in the header is refused before anything is read.
        let mut header = tar_header("mosdns", 0, b'0');
        header[124] = 0x80;
        let mut tar = header.to_vec();
        tar.extend_from_slice(&[0_u8; 2 * BLOCK]);
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&tar).unwrap();
        let archive = gz.finish().unwrap();
        assert!(extract_tar_gz_file(&archive, "mosdns", 1 << 20, 1 << 20).is_err());
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// A deflate-method zip the way the `zip` command writes one (for reader tests).
    fn zip_with(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, method) in entries {
            let mut crc = Crc::new();
            crc.update(data);
            let body = if *method == 8 {
                deflate(data)
            } else {
                data.to_vec()
            };
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL.to_le_bytes());
            out.extend_from_slice(&20_u16.to_le_bytes());
            out.extend_from_slice(&0_u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0_u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            central.extend_from_slice(&CENTRAL.to_le_bytes());
            central.extend_from_slice(&0x031E_u16.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.sum().to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&((0o100_644_u32) << 16).to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    #[test]
    fn stored_and_deflated_entries_are_read_and_directories_are_skipped() {
        let big = vec![b'x'; 5000];
        let archive = zip_with(&[
            ("dir/", b"", 0),
            ("a.txt", b"alpha", 0),
            ("sub/b.yaml", &big, 8),
        ]);
        let files = read_zip(&archive, LIMITS).unwrap();
        let names: Vec<_> = files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "sub/b.yaml"]);
        assert_eq!(files[0].data, b"alpha");
        assert_eq!(files[1].data, big);
    }

    #[test]
    fn the_writer_output_is_read_back_and_names_keep_their_directories() {
        let mut writer = ZipWriter::default();
        writer.add("config_custom.yaml", b"log: {}\n").unwrap();
        writer.add("sub_config/中文.yaml", b"x").unwrap();
        writer.add("empty", b"").unwrap();
        let archive = writer.finish().unwrap();
        let files = read_zip(&archive, LIMITS).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].name, "sub_config/中文.yaml");
        assert_eq!(files[0].data, b"log: {}\n");
        assert!(files[2].data.is_empty());
    }

    #[test]
    fn hostile_zips_are_refused() {
        let archive = zip_with(&[("a", b"hello", 0)]);
        assert!(read_zip(b"", LIMITS).is_err());
        assert!(read_zip(b"PK not a zip", LIMITS).is_err());
        assert!(read_zip(&archive[..archive.len() - 5], LIMITS).is_err());
        // Too many entries, an entry over its limit, a total over its limit.
        let many: Vec<(String, Vec<u8>, u16)> = (0..20)
            .map(|n| (format!("f{n}"), vec![b'a'; 4], 0))
            .collect();
        let refs: Vec<(&str, &[u8], u16)> = many
            .iter()
            .map(|(n, d, m)| (n.as_str(), d.as_slice(), *m))
            .collect();
        assert!(
            read_zip(&zip_with(&refs), LIMITS)
                .unwrap_err()
                .contains("more than")
        );
        let big = vec![b'z'; 2000];
        let tight = ZipLimits {
            max_entry_bytes: 1000,
            ..LIMITS
        };
        assert!(
            read_zip(&zip_with(&[("big", &big, 8)]), tight)
                .unwrap_err()
                .contains("too large")
        );
        let tight = ZipLimits {
            max_total_bytes: 1500,
            ..LIMITS
        };
        assert!(
            read_zip(&zip_with(&[("a", &big, 0), ("b", &big, 0)]), tight)
                .unwrap_err()
                .contains("total")
        );
        // A corrupted payload byte fails the CRC; a wrong declared size fails the size check.
        let mut corrupt = archive.clone();
        let at = corrupt
            .windows(5)
            .position(|window| window == b"hello")
            .unwrap();
        corrupt[at] = b'j';
        assert!(read_zip(&corrupt, LIMITS).unwrap_err().contains("CRC"));
        // A deflate bomb is cut at the declared size.
        let bomb = vec![0_u8; 900_000];
        let mut archive = zip_with(&[("bomb", &bomb, 8)]);
        // Lie about the uncompressed size in both headers (30 + name(4) bytes in; central at the end).
        let declared = 100_u32.to_le_bytes();
        let local = archive
            .windows(4)
            .position(|w| w == LOCAL.to_le_bytes())
            .unwrap();
        archive[local + 22..local + 26].copy_from_slice(&declared);
        let central = archive
            .windows(4)
            .position(|w| w == CENTRAL.to_le_bytes())
            .unwrap();
        archive[central + 24..central + 28].copy_from_slice(&declared);
        assert!(read_zip(&archive, LIMITS).is_err());
    }
}
