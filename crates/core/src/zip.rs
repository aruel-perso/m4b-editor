//! Bound and validate ZIP metadata before handing the file to the native decoder.
use crate::{
    assets::{check_unix_kind, ArchiveBatch, MAX_BYTES},
    Job,
};
use ::zip::{
    read::{ArchiveOffset, Config},
    CompressionMethod, ZipArchive, ZipReadOptions,
};
use anyhow::{ensure, Context, Result};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

const MAX_DIRECTORY: u64 = 32 * 1024 * 1024;
const EOCD: &[u8; 4] = b"PK\x05\x06";
const EOCD64: &[u8; 4] = b"PK\x06\x06";
const LOCATOR: &[u8; 4] = b"PK\x06\x07";

fn u16le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

struct Directory {
    offset: u64,
    size: u64,
    count: u64,
    end: u64,
}

fn directory(file: &mut File, limit: usize) -> Result<Directory> {
    let len = file.metadata()?.len();
    let tail_start = len.saturating_sub(65_557);
    file.seek(SeekFrom::Start(tail_start))?;
    let mut tail = vec![0; (len - tail_start) as usize];
    file.read_exact(&mut tail)?;
    let position = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail[i..].starts_with(EOCD) && i + 22 + usize::from(u16le(&tail, i + 20)) == tail.len()
        })
        .context("ZIP end record missing")?;
    let record = &tail[position..];
    let eocd_offset = tail_start + position as u64;
    ensure!(
        u16le(record, 4) == 0 && u16le(record, 6) == 0 && u16le(record, 8) == u16le(record, 10),
        "Multi-volume ZIP unsupported"
    );
    let count = u64::from(u16le(record, 10));
    let size = u64::from(u32le(record, 12));
    let offset = u64::from(u32le(record, 16));

    let mut footer_offset = eocd_offset;
    let mut result = Directory {
        offset,
        size,
        count,
        end: eocd_offset + 22,
    };
    if eocd_offset >= 20 {
        file.seek(SeekFrom::Start(eocd_offset - 20))?;
        let mut locator = [0; 20];
        file.read_exact(&mut locator)?;
        if locator.starts_with(LOCATOR) {
            ensure!(
                u32le(&locator, 4) == 0 && u32le(&locator, 16) == 1,
                "Multi-volume ZIP64 unsupported"
            );
            footer_offset = u64le(&locator, 8);
            ensure!(
                footer_offset.checked_add(56) == Some(eocd_offset - 20),
                "Unsupported ZIP64 end record"
            );
            file.seek(SeekFrom::Start(footer_offset))?;
            let mut record = [0; 56];
            file.read_exact(&mut record)?;
            ensure!(
                record.starts_with(EOCD64) && u64le(&record, 4) == 44,
                "Invalid ZIP64 end record"
            );
            ensure!(
                u32le(&record, 16) == 0
                    && u32le(&record, 20) == 0
                    && u64le(&record, 24) == u64le(&record, 32),
                "Multi-volume ZIP64 unsupported"
            );
            result.offset = u64le(&record, 48);
            result.size = u64le(&record, 40);
            result.count = u64le(&record, 32);
            ensure!(
                (offset == u64::from(u32::MAX) || offset == result.offset)
                    && (size == u64::from(u32::MAX) || size == result.size)
                    && (count == u64::from(u16::MAX) || count == result.count),
                "ZIP end records disagree"
            );
        }
    }
    ensure!(
        result.count <= limit as u64 && result.size <= MAX_DIRECTORY,
        "ZIP metadata exceeds import limits"
    );
    ensure!(
        result.offset.checked_add(result.size) == Some(footer_offset),
        "Invalid ZIP directory bounds"
    );
    Ok(result)
}

struct Record {
    name: String,
    central: u64,
    flags: u16,
    method: u16,
}

fn records(directory: &Directory, bytes: &[u8], job: &Job) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    let mut offset = 0;
    for _ in 0..directory.count {
        job.check()?;
        let header = bytes
            .get(offset..offset + 46)
            .context("Truncated ZIP directory")?;
        ensure!(
            header.starts_with(b"PK\x01\x02") && u16le(header, 34) == 0,
            "Invalid ZIP directory record"
        );
        let flags = u16le(header, 8);
        let method = u16le(header, 10);
        ensure!(
            flags & !0x080f == 0 && (matches!(method, 0 | 8) || (method == 99 && flags & 1 != 0)),
            "Unsupported ZIP compression or encryption"
        );
        let name_len = usize::from(u16le(header, 28));
        ensure!(name_len <= 4096, "ZIP filename exceeds 4096 bytes");
        let start = offset + 46;
        let end = start + name_len;
        let name = std::str::from_utf8(bytes.get(start..end).context("Truncated ZIP name")?)?;
        records.push(Record {
            name: name.to_owned(),
            central: directory.offset + offset as u64,
            flags,
            method,
        });
        offset = end + usize::from(u16le(header, 30)) + usize::from(u16le(header, 32));
    }
    ensure!(offset == bytes.len(), "ZIP directory size mismatch");
    Ok(records)
}

// During catalog creation only expose the checked directory and footer. This
// prevents alternate EOCD candidates in payloads or comments from expanding
// the library's scan or its preallocated catalog.
struct DecoderInput<'a> {
    file: File,
    directory: u64,
    metadata: Option<Vec<u8>>,
    position: u64,
    navigating: &'a std::cell::Cell<bool>,
    job: &'a Job,
}

impl Read for DecoderInput<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.job.check().map_err(io::Error::other)?;
        if self.navigating.get() {
            let metadata = self.metadata.as_ref().unwrap();
            let n = if self.position < self.directory {
                let n = bytes
                    .len()
                    .min(usize::try_from(self.directory - self.position).unwrap_or(usize::MAX));
                bytes[..n].fill(0);
                n
            } else {
                let offset = usize::try_from(self.position - self.directory).unwrap_or(usize::MAX);
                metadata.get(offset..).unwrap_or_default().read(bytes)?
            };
            self.position += n as u64;
            Ok(n)
        } else {
            self.metadata.take();
            self.file.seek(SeekFrom::Start(self.position))?;
            let n = self.file.read(bytes)?;
            self.position += n as u64;
            Ok(n)
        }
    }
}

impl Seek for DecoderInput<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.job.check().map_err(io::Error::other)?;
        self.position = match position {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => {
                let len = if self.navigating.get() {
                    self.directory + self.metadata.as_ref().unwrap().len() as u64
                } else {
                    self.file.metadata()?.len()
                };
                len.checked_add_signed(offset)
            }
        }
        .ok_or_else(|| io::Error::other("ZIP seek is out of bounds"))?;
        Ok(self.position)
    }
}

fn local_header(
    file: &mut File,
    record: &Record,
    offset: u64,
    compressed: u64,
    directory: u64,
) -> Result<u64> {
    ensure!(
        offset.checked_add(30).is_some_and(|end| end <= directory),
        "ZIP local header overlaps directory"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0; 30];
    file.read_exact(&mut header)?;
    ensure!(
        header.starts_with(b"PK\x03\x04")
            && u16le(&header, 6) == record.flags
            && u16le(&header, 8) == record.method,
        "ZIP local/central flags or compression mismatch"
    );
    ensure!(
        usize::from(u16le(&header, 26)) == record.name.len(),
        "ZIP local filename length mismatch"
    );
    let mut name = vec![0; record.name.len()];
    file.read_exact(&mut name)?;
    ensure!(
        name == record.name.as_bytes(),
        "ZIP local/central filename mismatch"
    );
    let start = offset + 30 + name.len() as u64 + u64::from(u16le(&header, 28));
    ensure!(
        start
            .checked_add(compressed)
            .is_some_and(|end| end <= directory),
        "ZIP data overlaps directory"
    );
    Ok(start + compressed)
}

pub(crate) fn extract(
    source: &Path,
    password: Option<&str>,
    batch: &mut ArchiveBatch<'_>,
) -> Result<()> {
    let job = batch.job;
    let mut file = File::open(source)?;
    let directory = directory(&mut file, batch.remaining_entries())?;
    file.seek(SeekFrom::Start(directory.offset))?;
    let mut metadata = vec![0; (directory.end - directory.offset) as usize];
    for chunk in metadata.chunks_mut(65_536) {
        job.check()?;
        file.read_exact(chunk)?;
    }
    let records = records(&directory, &metadata[..directory.size as usize], job)?;
    // The verified EOCD abuts the directory (or the fixed ZIP64 data).
    // Hide its arbitrary comment and reject competing end-record signatures.
    let eocd = metadata.len() - 22;
    ensure!(
        !metadata[..eocd].windows(4).any(|sig| sig == EOCD),
        "Ambiguous ZIP end record"
    );
    metadata[eocd + 20..].fill(0);
    let navigating = std::cell::Cell::new(true);
    let mut zip = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        DecoderInput {
            file: file.try_clone()?,
            directory: directory.offset,
            metadata: Some(metadata),
            position: 0,
            navigating: &navigating,
            job,
        },
    )
    .context("Read ZIP directory")?;
    navigating.set(false);
    ensure!(
        zip.len() == records.len()
            && !zip.has_duplicate_names()
            && zip.central_directory_start() == directory.offset,
        "ZIP directory changed or contains duplicate names"
    );

    let mut members = Vec::new();
    let mut ranges = Vec::new();
    let mut packed = 0u64;
    for (index, record) in records.iter().enumerate() {
        job.check()?;
        let entry = zip.by_index_data(index)?;
        ensure!(
            entry.name_raw() == record.name.as_bytes()
                && entry.name()?.as_ref() == record.name
                && entry.central_header_start() == record.central,
            "Conflicting ZIP names or directory records"
        );
        ensure!(
            matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ),
            "Unsupported ZIP compression"
        );
        let directory_entry = record.name.ends_with('/');
        ensure!(
            !entry.is_symlink() && entry.external_attributes() & 0x0448 == 0,
            "ZIP links, reparse points or special files are forbidden"
        );
        if let Some(mode) = entry.unix_mode() {
            check_unix_kind(mode.into(), directory_entry)?;
        }
        let size = entry.size();
        packed = packed
            .checked_add(entry.compressed_size())
            .context("ZIP packed size overflow")?;
        ensure!(
            packed <= MAX_BYTES + MAX_DIRECTORY,
            "ZIP packed data exceeds import limit"
        );
        let path = batch.register(&record.name, directory_entry, size)?;
        let end = local_header(
            &mut file,
            record,
            entry.header_start(),
            entry.compressed_size(),
            directory.offset,
        )?;
        ranges.push((entry.header_start(), end));
        if directory_entry {
            ensure!(
                entry.compressed_size() == 0 || entry.compression() == CompressionMethod::Deflated,
                "ZIP directory contains unexpected data"
            );
        }
        members.push((path, size, directory_entry));
    }
    ranges.sort_unstable();
    ensure!(
        ranges.windows(2).all(|pair| pair[0].1 <= pair[1].0),
        "Overlapping ZIP members"
    );
    batch.start()?;
    for (index, (path, size, is_dir)) in members.into_iter().enumerate() {
        job.check()?;
        let mut entry = zip
            .by_index_with_options(
                index,
                ZipReadOptions::new()
                    .password(password.map(str::as_bytes))
                    .enforce_header_consistency(true),
            )
            .context("Read ZIP member (check password, encryption and compression)")?;
        if is_dir {
            // Check CRC/authentication and require an empty decoded stream.
            ensure!(entry.read(&mut [0])? == 0, "ZIP directory contains data");
        } else {
            batch.stage(&path, size, |writer| {
                io::copy(&mut entry, writer)?;
                Ok(())
            })?;
        }
    }
    Ok(())
}
