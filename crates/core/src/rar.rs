//! Bounded single-volume RAR import. The decoder opens by path, so preflight
//! the wire blocks before it builds its catalog, then stream by catalog ID.
use crate::{
    assets::{check_unix_kind, ArchiveBatch, MAX_BYTES, MAX_FILE},
    Job,
};
use anyhow::{bail, ensure, Context, Result};
use rar_rs::{
    wire::{self, FileHeader},
    ArchiveReader, ExtractOptions, OpenOptions,
};
use std::{
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_DICTIONARY: u64 = 64 * 1024 * 1024;
const MAX_HEADERS: u64 = 16 * 1024 * 1024;
const MAX_BUFFERED_PACKED: u64 = 32 * 1024 * 1024;
const MAX_LEGACY_SOLID_OUTPUT: u64 = 64 * 1024 * 1024;
const MAX_KDF_STRENGTH: u8 = 16;
const MAX_ENCRYPTED_MEMBERS: usize = 512;

fn vint(bytes: &mut &[u8]) -> Result<u64> {
    let (value, size) = wire::vint::decode_from_slice(bytes, 0)?;
    *bytes = &bytes[size..];
    Ok(value)
}

fn take<'a>(bytes: &mut &'a [u8], size: usize) -> Result<&'a [u8]> {
    let (head, rest) = bytes
        .split_at_checked(size)
        .context("Truncated RAR header")?;
    *bytes = rest;
    Ok(head)
}

fn header_key(block: &wire::RawBlock) -> Result<wire::EncryptionParams> {
    ensure!(block.flags == 0, "Invalid RAR header encryption block");
    let mut body = block.header_data.as_slice();
    vint(&mut body)?;
    vint(&mut body)?;
    ensure!(
        vint(&mut body)? == 0,
        "Unsupported RAR header encryption version"
    );
    let flags = vint(&mut body)?;
    ensure!(flags <= 1, "Unsupported RAR header encryption flags");
    let strength = take(&mut body, 1)?[0];
    ensure!(
        strength <= MAX_KDF_STRENGTH,
        "RAR password derivation exceeds import limit"
    );
    let salt = take(&mut body, 16)?.try_into()?;
    let checksum = if flags == 1 {
        Some(take(&mut body, 12)?.try_into()?)
    } else {
        None
    };
    ensure!(body.is_empty(), "Unexpected RAR encryption header data");
    Ok(wire::EncryptionParams {
        version: 0,
        flags: flags as u8,
        strength,
        salt,
        iv: [0; 16],
        checksum,
        iterations: 1u32 << strength,
    })
}

fn modern_file(block: &wire::RawBlock) -> Result<(FileHeader, bool)> {
    let mut body = block.header_data.as_slice();
    vint(&mut body)?;
    let flags = vint(&mut body)?;
    let extra_size = if flags & 1 == 0 {
        0
    } else {
        usize::try_from(vint(&mut body)?)?
    };
    if flags & 2 != 0 {
        vint(&mut body)?;
    }
    let file_flags = vint(&mut body)?;
    ensure!(
        file_flags & !7 == 0,
        "Unsupported RAR member flags or unknown unpacked size"
    );
    // Size, attributes, and optional fixed timestamp/checksum precede the name.
    vint(&mut body)?;
    vint(&mut body)?;
    take(
        &mut body,
        4 * usize::try_from((file_flags & 6).count_ones())?,
    )?;
    vint(&mut body)?;
    vint(&mut body)?;
    let name_len = usize::try_from(vint(&mut body)?)?;
    ensure!(name_len <= 4096, "RAR filename exceeds 4096 bytes");
    let name = std::str::from_utf8(take(&mut body, name_len)?)?;
    ensure!(body.len() == extra_size, "Invalid RAR file extra area");
    let mut encrypted = false;
    while !body.is_empty() {
        let length = usize::try_from(vint(&mut body)?)?;
        let mut record = take(&mut body, length)?;
        match vint(&mut record)? {
            1 => {
                ensure!(!encrypted, "Duplicate RAR encryption record");
                let params = wire::EncryptionParams::from_extra_bytes(record)?;
                ensure!(
                    params.strength <= MAX_KDF_STRENGTH,
                    "RAR password derivation exceeds import limit"
                );
                encrypted = true;
            }
            5 => bail!("RAR links and file redirections are forbidden"),
            _ => {}
        }
    }
    let header = FileHeader::from_raw(block, block.data_offset)?;
    ensure!(
        header.name == name,
        "RAR decoder interpreted the member name differently"
    );
    ensure!(
        header.comp_method <= 5 && header.comp_version <= 1,
        "Unsupported RAR compression method"
    );
    ensure!(
        header.is_directory || header.crc32_val.is_some() || header.hash_value.is_some(),
        "RAR file lacks an integrity checksum"
    );
    Ok((header, encrypted))
}

#[derive(Default)]
struct Scan {
    entries: usize,
    encrypted: usize,
    archive_header: bool,
}

impl Scan {
    fn member(&mut self, limit: usize, encrypted: bool, password: Option<&str>) -> Result<()> {
        ensure!(
            self.archive_header && self.entries < limit,
            "RAR catalog exceeds the entry limit or has no main header"
        );
        self.entries += 1;
        if encrypted {
            ensure!(
                password.is_some(),
                "Encrypted RAR member requires a password"
            );
            self.encrypted += 1;
            ensure!(
                self.encrypted <= MAX_ENCRYPTED_MEMBERS,
                "RAR has too many encrypted members"
            );
        }
        Ok(())
    }
}

// The wire reader caps any one RAR5 block at 2 MiB before allocating. This
// wrapper additionally limits cumulative header I/O and checks cancellation
// inside each read while allowing seeks over packed member bodies.
struct HeaderInput<'a> {
    file: &'a mut File,
    remaining: u64,
    job: &'a Job,
}

impl Read for HeaderInput<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.job.check().map_err(io::Error::other)?;
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other("RAR exceeds header byte limit"));
        }
        let size = self.file.read(bytes)?;
        self.remaining -= size as u64;
        Ok(size)
    }
}

impl Seek for HeaderInput<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

fn scan_modern(
    input: &mut HeaderInput<'_>,
    end: u64,
    limit: usize,
    password: Option<&str>,
) -> Result<usize> {
    let mut scan = Scan::default();
    let mut keys = None;
    for index in 0..limit + 128 {
        let block = wire::read_block(input, keys.as_ref())
            .context("Read RAR5 block header (check password and archive integrity)")?
            .context("RAR5 end block missing")?;
        ensure!(
            block.data_end <= end && block.flags & 0x18 == 0,
            "Split or truncated RAR block"
        );
        ensure!(
            scan.archive_header || matches!(block.block_type, 1 | 4),
            "RAR main header must precede other blocks"
        );
        match block.block_type {
            1 => {
                // WinRAR and rar-rs mark the main header skippable (0x04).
                ensure!(
                    !scan.archive_header && block.flags & !5 == 0,
                    "Invalid RAR main header"
                );
                let mut body = block.raw.header_data.as_slice();
                vint(&mut body)?;
                let extra = if vint(&mut body)? & 1 != 0 {
                    usize::try_from(vint(&mut body)?)?
                } else {
                    0
                };
                ensure!(vint(&mut body)? & 3 == 0, "Multi-volume RAR unsupported");
                ensure!(body.len() == extra, "Invalid RAR main header extra area");
                scan.archive_header = true;
            }
            2 => {
                let (header, encrypted) = modern_file(&block.raw)?;
                scan.member(limit, encrypted, password)?;
                // rar-rs streams output but aggregates modern packed input.
                ensure!(
                    header.packed_size <= MAX_BUFFERED_PACKED,
                    "RAR packed member exceeds 32 MiB decoder memory limit"
                );
                ensure!(
                    !header.is_directory || header.unpacked_size == 0 && header.packed_size == 0,
                    "RAR directory contains data"
                );
            }
            3 => {
                // RAR5 uses dependent service blocks for NTFS streams.
                // Independent service records (quick-open, comments, recovery)
                // are skipped in a full catalog scan.
                ensure!(
                    block.flags & 0x20 == 0,
                    "RAR alternate streams are unsupported"
                );
            }
            4 => {
                ensure!(index == 0, "Misplaced RAR encryption header");
                let params = header_key(&block.raw)?;
                let password = password.context("RAR header encryption requires a password")?;
                keys = Some(
                    params
                        .derive_and_verify(password)?
                        .context("Wrong RAR password")?
                        .key,
                );
            }
            5 => {
                ensure!(
                    scan.archive_header && block.flags & !4 == 0 && block.data_end == end,
                    "Invalid RAR end block or trailing data"
                );
                let mut body = block.raw.header_data.as_slice();
                vint(&mut body)?;
                vint(&mut body)?;
                ensure!(vint(&mut body)? & 1 == 0, "Multi-volume RAR unsupported");
                return Ok(scan.entries);
            }
            _ => ensure!(block.flags & 4 != 0, "Unsupported RAR block type"),
        }
        input.seek(SeekFrom::Start(block.data_end))?;
    }
    bail!("RAR has too many metadata blocks")
}

fn scan_legacy(
    input: &mut HeaderInput<'_>,
    end: u64,
    limit: usize,
    password: Option<&str>,
) -> Result<usize> {
    let mut scan = Scan::default();
    let mut solid = false;
    let mut largest = 0;
    let mut ended = false;
    for _ in 0..limit + 128 {
        input.job.check()?;
        let start = input.stream_position()?;
        if start == end && scan.archive_header {
            // Early RAR versions may omit the end-of-archive record.
            ended = true;
            break;
        }
        let mut prefix = [0u8; 7];
        input.read_exact(&mut prefix)?;
        let flags = u16::from_le_bytes([prefix[3], prefix[4]]);
        let size = u16::from_le_bytes([prefix[5], prefix[6]]);
        ensure!(
            size >= 7 && start + u64::from(size) <= end,
            "Invalid legacy RAR header size"
        );
        let mut rest = vec![0; usize::from(size) - 7];
        input.read_exact(&mut rest)?;
        let long = flags & 0x8000 != 0;
        let packed = if long {
            u64::from(u32::from_le_bytes(
                rest.get(..4)
                    .context("Truncated legacy RAR packed size")?
                    .try_into()?,
            ))
        } else {
            0
        };
        ensure!(
            scan.archive_header || prefix[2] == 0x73,
            "Legacy RAR main header must precede other blocks"
        );
        match prefix[2] {
            0x73 => {
                ensure!(
                    !scan.archive_header && !long && size >= 13 && flags & 1 == 0,
                    "Invalid or multi-volume legacy RAR"
                );
                ensure!(
                    flags & 0x80 == 0,
                    "Legacy RAR header encryption is unsupported"
                );
                scan.archive_header = true;
                solid = flags & 0x08 != 0;
            }
            0x74 => {
                ensure!(
                    long && flags & 3 == 0 && rest.len() >= 25,
                    "Invalid or split legacy RAR member"
                );
                let name_len = usize::from(u16::from_le_bytes(rest[19..21].try_into()?));
                let mut unpacked = u64::from(u32::from_le_bytes(rest[4..8].try_into()?));
                let name_offset = if flags & 0x100 == 0 {
                    25
                } else {
                    let high = rest
                        .get(25..29)
                        .context("Truncated legacy RAR large member")?;
                    // rar-rs advances its legacy catalog scan using only
                    // the low packed-size word from the envelope.
                    ensure!(high == [0; 4], "Legacy RAR packed size exceeds 32 bits");
                    let high = rest
                        .get(29..33)
                        .context("Truncated legacy RAR large member")?;
                    unpacked |= u64::from(u32::from_le_bytes(high.try_into()?)) << 32;
                    33
                };
                let name = rest
                    .get(name_offset..name_offset + name_len)
                    .context("Truncated legacy RAR name")?;
                ensure!(
                    name_len <= 4096 && (flags & 0x200 != 0 || !name.contains(&0)),
                    "Invalid legacy RAR filename"
                );
                let encrypted = flags & 4 != 0;
                ensure!(
                    (0x30..=0x35).contains(&rest[18]),
                    "Unsupported legacy RAR compression method"
                );
                scan.member(limit, encrypted, password)?;
                solid |= flags & 0x10 != 0;
                largest = largest.max(unpacked);
                // The decoder buffers packed legacy compressed/encrypted members.
                ensure!(
                    (rest[18] == 0x30 && !encrypted) || packed <= MAX_BUFFERED_PACKED,
                    "Legacy RAR compressed member exceeds memory limit"
                );
            }
            0x7b => {
                ensure!(
                    scan.archive_header
                        && !long
                        && flags & 1 == 0
                        && start + u64::from(size) == end,
                    "Invalid or multi-volume legacy RAR end"
                );
                ended = true;
                break;
            }
            // Recovery and comments are metadata and are never extracted.
            0x75..=0x79 => {}
            // Legacy NEWSUB (0x7a) can represent NTFS ADS or other attributes.
            _ => bail!("Unsupported legacy RAR service block"),
        }
        let next = (start + u64::from(size))
            .checked_add(packed)
            .context("RAR payload offset overflow")?;
        ensure!(next <= end, "Truncated legacy RAR payload");
        input.seek(SeekFrom::Start(next))?;
    }
    ensure!(ended, "RAR has too many metadata blocks");
    // The legacy solid decoder materializes an entire member before writing
    // to the quota-checked sink. This includes unflagged chain heads.
    ensure!(
        !solid || largest <= MAX_LEGACY_SOLID_OUTPUT,
        "Legacy solid RAR member exceeds 64 MiB expanded memory limit"
    );
    Ok(scan.entries)
}

#[cfg(unix)]
fn decoder_path(file: &File, _: &Path) -> Result<PathBuf> {
    use std::os::fd::AsRawFd;
    Ok(Path::new("/dev/fd").join(file.as_raw_fd().to_string()))
}

#[cfg(not(unix))]
fn decoder_path(_: &File, source: &Path) -> Result<PathBuf> {
    let source = dunce::canonicalize(source)?;
    let parent = source.parent().context("RAR source directory missing")?;
    ensure!(
        std::fs::read_dir(parent)?.take(16_385).count() <= 16_384,
        "RAR source directory has too many entries"
    );
    ensure!(
        rar_rs::discover_volumes(&source) == [source.clone()],
        "Multi-volume RAR unsupported"
    );
    Ok(source)
}

fn unchanged(file: &File, original: &Metadata) -> Result<()> {
    let current = file.metadata()?;
    ensure!(
        current.len() == original.len() && current.modified()? == original.modified()?,
        "RAR source changed during import"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            (current.ctime(), current.ctime_nsec()) == (original.ctime(), original.ctime_nsec()),
            "RAR source changed during import"
        );
    }
    Ok(())
}

pub(crate) fn extract(
    source: &Path,
    password: Option<&str>,
    batch: &mut ArchiveBatch<'_>,
) -> Result<()> {
    let job = batch.job;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        open.share_mode(1); // FILE_SHARE_READ, deny writes and replacement while importing.
    }
    let mut file = open.open(source)?;
    let original = file.metadata()?;
    ensure!(
        original.is_file() && original.len() <= MAX_BYTES + MAX_HEADERS,
        "RAR source exceeds packed import limit"
    );
    let decoder_path = decoder_path(&file, source)?;
    let mut magic = [0; 8];
    file.read_exact(&mut magic)
        .context("Invalid RAR signature")?;
    let modern = &magic == b"Rar!\x1a\x07\x01\x00";
    if !modern {
        ensure!(
            &magic[..7] == b"Rar!\x1a\x07\x00",
            "Invalid RAR signature or unsupported archive generation"
        );
        file.seek(SeekFrom::Start(7))?;
    }
    let mut input = HeaderInput {
        file: &mut file,
        remaining: MAX_HEADERS,
        job,
    };
    let count = if modern {
        scan_modern(
            &mut input,
            original.len(),
            batch.remaining_entries(),
            password,
        )?
    } else {
        scan_legacy(
            &mut input,
            original.len(),
            batch.remaining_entries(),
            password,
        )?
    };
    job.check()?;
    unchanged(&file, &original)?;
    let mut options = OpenOptions::new();
    if let Some(password) = password {
        options = options.password(password);
    }
    let mut reader = ArchiveReader::open_with(decoder_path, options).context("Read RAR catalog")?;
    unchanged(&file, &original)?;
    ensure!(
        reader.entries().len() == count && reader.streams().is_empty(),
        "RAR catalog differs from validated headers or contains alternate streams"
    );
    reader.set_cancel_flag(Some(job.cancel.clone()));
    let mut members = Vec::new();
    for entry in reader.entries() {
        job.check()?;
        let directory = entry.is_dir();
        ensure!(
            entry.redirect().is_none() && !entry.name().contains('\u{fffd}'),
            "RAR link or invalid filename"
        );
        match (modern, entry.host_os_raw()) {
            (true, 0) | (false, 0 | 2) => ensure!(
                entry.attributes() & 0x0448 == 0,
                "RAR reparse points and special files are forbidden"
            ),
            (true, 1) => check_unix_kind(entry.attributes(), directory)?,
            (false, 3) => {
                // rar-rs uses the high word for legacy Unix directories.
                // Check either mode placement for a link or special file.
                check_unix_kind(entry.attributes() >> 16, directory)?;
                check_unix_kind(entry.attributes(), directory)?;
            }
            _ => bail!("Unsupported RAR member host attributes"),
        }
        ensure!(
            entry.method() == 0
                || entry.dict_size_bytes().unwrap_or_else(|| 131_072u64
                    .checked_shl(entry.comp_dict_size().into())
                    .unwrap_or(u64::MAX))
                    <= MAX_DICTIONARY,
            "RAR dictionary exceeds 64 MiB"
        );
        let name = entry.name().replace('\\', "/");
        let path = batch.register(&name, directory, entry.size())?;
        if !directory {
            members.push((entry.id(), path, entry.size()));
        }
    }
    batch.start()?;
    for (id, path, size) in members {
        batch.stage(&path, size, |writer| {
            let bytes = reader.copy_entry_to_with_options(
                id,
                writer,
                ExtractOptions {
                    max_dict_size: Some(MAX_DICTIONARY),
                    max_metadata_bytes: Some(MAX_HEADERS),
                    max_unpacked_bytes: Some(MAX_FILE),
                    max_total_unpacked_bytes: Some(MAX_BYTES),
                    threads: Some(1),
                    ..ExtractOptions::default()
                },
            )?;
            ensure!(bytes == size, "RAR decoder returned a different size");
            Ok(())
        })?;
    }
    unchanged(&file, &original)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(file: &mut File, fields: &[u64], bytes: &[u8]) {
        use std::io::Write;
        let mut body: Vec<_> = fields
            .iter()
            .flat_map(|&value| wire::vint::encode(value))
            .collect();
        body.extend(bytes);
        let mut header = wire::vint::encode(body.len() as u64);
        header.extend(body);
        file.write_all(&crc32fast::hash(&header).to_le_bytes())
            .unwrap();
        file.write_all(&header).unwrap();
    }

    #[test]
    fn preflight_enforces_catalog_and_header_budgets() {
        use std::io::Write;
        let mut file = tempfile::tempfile().unwrap();
        let job = Job::default();
        block(&mut file, &[1, 0, 0], &[]);
        for _ in 0..3 {
            // Zero-size directory, which still consumes a catalog slot.
            block(&mut file, &[2, 0, 1, 0, 0, 0, 1, 1], b"d");
        }
        block(&mut file, &[5, 0, 0], &[]);
        file.flush().unwrap();
        let end = file.stream_position().unwrap();
        file.rewind().unwrap();
        let mut input = HeaderInput {
            file: &mut file,
            remaining: MAX_HEADERS,
            job: &job,
        };
        assert_eq!(scan_modern(&mut input, end, 3, None).unwrap(), 3);
        input.rewind().unwrap();
        assert!(scan_modern(&mut input, end, 2, None)
            .unwrap_err()
            .to_string()
            .contains("entry limit"));
        input.rewind().unwrap();
        input.remaining = 12;
        assert!(
            format!("{:#}", scan_modern(&mut input, end, 3, None).unwrap_err())
                .contains("header byte limit")
        );
    }

    #[test]
    fn legacy_preflight_bounds_solid_output_and_keeps_decoder_offsets() {
        use std::io::Write;

        let job = Job::default();
        for (packed_high, unpacked, message) in [
            (1_u32, 0_u32, "packed size exceeds 32 bits"),
            (
                0,
                MAX_LEGACY_SOLID_OUTPUT as u32 + 1,
                "expanded memory limit",
            ),
        ] {
            let mut file = tempfile::tempfile().unwrap();
            let main = [0, 0, 0x73, 0x08, 0, 13, 0, 0, 0, 0, 0, 0, 0];
            let name = b"track.mp3";
            let mut header = [0; 40];
            let size = (header.len() + name.len()) as u16;
            header[2] = 0x74;
            header[3..5].copy_from_slice(&0x8100_u16.to_le_bytes());
            header[5..7].copy_from_slice(&size.to_le_bytes());
            header[11..15].copy_from_slice(&unpacked.to_le_bytes());
            header[25] = 0x30;
            header[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            header[32..36].copy_from_slice(&packed_high.to_le_bytes());
            file.write_all(&main).unwrap();
            file.write_all(&header).unwrap();
            file.write_all(name).unwrap();
            let end = file.stream_position().unwrap();
            file.rewind().unwrap();
            let mut input = HeaderInput {
                file: &mut file,
                remaining: MAX_HEADERS,
                job: &job,
            };
            let error = scan_legacy(&mut input, end, 1, None).unwrap_err();
            assert!(error.to_string().contains(message), "{error:#}");
        }
    }
}
