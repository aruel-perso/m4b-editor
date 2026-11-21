use crate::zip;
use crate::*;
use std::io::{Read, Write};

pub(crate) const MAX_FILES: usize = 4096;
pub(crate) const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) fn safe_name(name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && name.len() <= 4096
            && !name.contains('\\')
            && !name.contains(':')
            && !name.contains('\0'),
        "Unsafe archive path: {name:?}"
    );
    ensure!(
        !name.starts_with('/') && name.split('/').all(|c| !matches!(c, "." | "..")),
        "Unsafe archive path: {name:?}"
    );
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    ensure!(
        !trimmed.is_empty() && trimmed.split('/').all(|p| !p.is_empty()),
        "Empty archive path component"
    );
    ensure!(
        trimmed.split('/').count() <= 64,
        "Archive path is too deeply nested"
    );
    for part in trimmed.split('/') {
        ensure!(
            part.encode_utf16().count() <= 255
                && !part.ends_with(['.', ' '])
                && !part.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)),
            "Unsafe Windows archive path: {name:?}"
        );
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        ensure!(
            ![
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9", "CONIN$", "CONOUT$", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"
            ]
            .contains(&stem.as_str()),
            "Reserved Windows path: {name:?}"
        );
    }
    Ok(Path::new(name).components().collect())
}
pub(crate) fn audio(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        [
            "mka", "mkv", "mp4", "m4b", "m4a", "mp3", "wav", "flac", "ogg", "opus", "aac",
        ]
        .contains(&e.to_ascii_lowercase().as_str())
    })
}
pub(crate) fn copy_limited(mut input: impl Read, path: &Path, total: &mut u64) -> Result<()> {
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut size = 0;
    let mut buf = [0u8; 65536];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        *total += n as u64;
        ensure!(
            size <= MAX_FILE && *total <= MAX_BYTES,
            "Import exceeds limits: 2 GiB/file, 8 GiB/batch"
        );
        out.write_all(&buf[..n])?;
    }
    out.sync_all()?;
    Ok(())
}
impl Project {
    /// Copies a whole batch into persistent assets; failures leave the project unchanged.
    /// The caller owns the asset directory and must retain it alongside the project.
    pub fn import_assets(
        &mut self,
        backend: &dyn MediaBackend,
        paths: Vec<PathBuf>,
        assets: &Path,
    ) -> Result<()> {
        self.import_assets_with(backend, paths, assets, &Job::default(), None)
    }
    pub fn import_assets_with(
        &mut self,
        backend: &dyn MediaBackend,
        paths: Vec<PathBuf>,
        assets: &Path,
        job: &Job,
        password: Option<&str>,
    ) -> Result<()> {
        self.import_assets_observed(backend, paths, assets, job, password, &mut |_| {})
    }

    pub fn import_assets_observed(
        &mut self,
        backend: &dyn MediaBackend,
        paths: Vec<PathBuf>,
        assets: &Path,
        job: &Job,
        password: Option<&str>,
        event: &mut dyn FnMut(ImportEvent),
    ) -> Result<()> {
        job.check()?;
        ensure!(!paths.is_empty(), "Select at least one file");
        fs::create_dir_all(assets).context("Create project asset directory")?;
        let assets = dunce::canonicalize(assets)?;
        let stage = tempfile::Builder::new()
            .prefix(".import-")
            .tempdir_in(&assets)?;
        let mut media = Vec::new();
        let mut total = 0;
        let mut count: usize = 0;
        for (index, source) in paths.iter().enumerate() {
            job.check()?;
            event(ImportEvent::Inspecting);
            ensure!(
                fs::metadata(source)?.is_file(),
                "Import source must be a regular file"
            );
            let ext = source
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let root = stage.path().join(index.to_string());
            fs::create_dir(&root)?;
            if ext == "rar" || ext == "zip" {
                let mut batch = ArchiveBatch::new(&root, &mut total, &mut count, job, event);
                if ext == "rar" {
                    crate::rar::extract(source, password, &mut batch)?;
                } else {
                    zip::extract(source, password, &mut batch)?;
                }
                media.extend(batch.media);
            } else {
                ensure!(audio(source), "Unsupported import: {}", source.display());
                count += 1;
                ensure!(count <= MAX_FILES, "Import exceeds 4096 files");
                event(ImportEvent::Extracting { done: 0, total: 1 });
                let dest = root.join(source.file_name().context("Missing filename")?);
                copy_limited(
                    CheckedRead {
                        input: fs::File::open(source)?,
                        job,
                    },
                    &dest,
                    &mut total,
                )?;
                media.push(dest);
                event(ImportEvent::Extracting { done: 1, total: 1 });
            }
        }
        ensure!(!media.is_empty(), "Archive contains no audio files");
        let mut next = self.clone();
        next.import_observed(backend, media, job, event)?;
        job.check()?;
        // Persist the successful batch with a unique name, preserving prior batches.
        let old = stage.path().to_path_buf();
        let committed = stage.keep();
        for track in &mut next.tracks {
            if let Ok(relative) = track.path.strip_prefix(&old) {
                track.path = committed.join(relative);
            }
        }
        if let Some(cover) = &mut next.cover {
            if let Ok(relative) = cover.strip_prefix(&old) {
                *cover = committed.join(relative);
            }
        }
        *self = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_paths() {
        for name in [
            "../a.mp3",
            "/a.mp3",
            "C:/a.mp3",
            "a\\b.mp3",
            "a/../b",
            "./x",
            "a//b.mp3",
            "a///",
            "NUL.mp3",
            "LPT¹.mp3",
            "CONIN$.mp3",
            "a./b.mp3",
            "a /b.mp3",
        ] {
            assert!(safe_name(name).is_err(), "{name}");
        }
        assert!(safe_name("book/02.mp3").is_ok());
    }
    #[test]
    fn legacy_metadata_defaults() {
        let m: Metadata = serde_json::from_str(
            r#"{"title":"Old","author":"A","narrator":"N","description":"D"}"#,
        )
        .unwrap();
        assert!(m.genre.is_empty());
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    struct Backend;
    impl MediaBackend for Backend {
        fn probe(&self, path: &Path) -> Result<Probe> {
            ensure!(fs::read(path)? == b"audio", "Invalid test audio");
            Ok(Probe {
                track: Track {
                    path: dunce::canonicalize(path)?,
                    duration_ms: 1000,
                    codec: "test".into(),
                    sample_rate: 48000,
                    channels: 2,
                    stream_index: None,
                },
                metadata: Metadata::default(),
                chapters: vec![],
                has_cover: false,
            })
        }
        fn encoders(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn export(
            &self,
            _: &Project,
            _: &Path,
            _: Cancel,
            _: &mut dyn FnMut(ExportEvent),
        ) -> Result<()> {
            bail!("unused")
        }
    }
    fn archive(path: &Path, entries: &[(&str, &[u8], u32)]) {
        let mut data = Vec::new();
        let mut central = Vec::new();
        for (name, bytes, mode) in entries {
            let compressed = name.contains("deflate");
            let payload = if compressed {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(bytes).unwrap();
                encoder.finish().unwrap()
            } else {
                bytes.to_vec()
            };
            let method = if compressed { 8u16 } else { 0u16 };
            let offset = data.len() as u32;
            let crc = crc32fast::hash(bytes);
            data.extend(b"PK\x03\x04");
            data.extend(20u16.to_le_bytes());
            data.extend([0; 2]);
            data.extend(method.to_le_bytes());
            data.extend([0; 4]);
            data.extend(crc.to_le_bytes());
            data.extend((payload.len() as u32).to_le_bytes());
            data.extend((bytes.len() as u32).to_le_bytes());
            data.extend((name.len() as u16).to_le_bytes());
            data.extend(0u16.to_le_bytes());
            data.extend(name.as_bytes());
            data.extend(&payload);
            central.extend(b"PK\x01\x02");
            central.extend([20, 3]);
            central.extend(20u16.to_le_bytes());
            central.extend([0; 2]);
            central.extend(method.to_le_bytes());
            central.extend([0; 4]);
            central.extend(crc.to_le_bytes());
            central.extend((payload.len() as u32).to_le_bytes());
            central.extend((bytes.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 8]);
            central.extend((mode << 16).to_le_bytes());
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let offset = data.len() as u32;
        let size = central.len() as u32;
        data.extend(central);
        data.extend(b"PK\x05\x06");
        data.extend([0; 4]);
        data.extend((entries.len() as u16).to_le_bytes());
        data.extend((entries.len() as u16).to_le_bytes());
        data.extend(size.to_le_bytes());
        data.extend(offset.to_le_bytes());
        data.extend([0; 2]);
        fs::write(path, data).unwrap();
    }
    #[test]
    fn tagged_archive_uses_consensus_while_preserving_edits_and_order() {
        let temp = tempfile::tempdir().unwrap();
        let backend = DesktopFfmpeg::default();
        let tagged_track = |name, artist, pixel| {
            let image = temp.path().join("cover.ppm");
            let mut ppm = b"P6\n1 1\n255\n".to_vec();
            ppm.extend_from_slice(pixel);
            fs::write(&image, ppm).unwrap();
            let output = temp.path().join(name);
            assert!(command(&backend.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=duration=0.2",
                    "-i"
                ])
                .arg(image)
                .args([
                    "-map",
                    "0:a",
                    "-map",
                    "1:v",
                    "-c:a",
                    "libmp3lame",
                    "-c:v",
                    "png",
                    "-disposition:v",
                    "attached_pic",
                    "-metadata",
                    "album=Tagged book",
                    "-metadata",
                    artist,
                    "-metadata",
                    "genre=Audiobook",
                    "-metadata",
                    "language=French",
                    "-metadata"
                ])
                .arg(format!("title={name}"))
                .arg(&output)
                .status()
                .unwrap()
                .success());
            fs::read(output).unwrap()
        };
        let a = tagged_track("track2.mp3", "artist=Artist A", &[255, 0, 0]);
        let b = tagged_track("track10.mp3", "artist=Artist B", &[255, 0, 0]);
        let c = tagged_track("track12.mp3", "artist=Artist B", &[0, 0, 255]);
        let input = temp.path().join("tagged.zip");
        archive(
            &input,
            &[("track10.mp3", &b, 0o100644), ("track2.mp3", &a, 0o100644)],
        );
        let assets = temp.path().join("assets");
        let mut project = Project::default();
        project.metadata.genre = "Edited genre".into();
        let mut events = Vec::new();
        project
            .import_assets_observed(
                &backend,
                vec![input.clone()],
                &assets,
                &Job::default(),
                None,
                &mut |event| events.push(event),
            )
            .unwrap();
        assert_eq!(project.metadata.title, "Tagged book");
        assert_eq!(project.metadata.genre, "Edited genre");
        assert!(project.metadata.book_language.is_empty());
        project.validate().unwrap();
        assert!(project.metadata.author.is_empty());
        assert_eq!(project.cover.as_ref(), Some(&project.tracks[0].path));
        assert!(project.tracks[0].path.ends_with("track2.mp3"));
        assert!(matches!(events.first(), Some(ImportEvent::Inspecting)));
        assert!(events
            .iter()
            .any(|event| matches!(event, ImportEvent::Extracting { done: 2, total: 2 })));
        assert!(events
            .iter()
            .any(|event| matches!(event, ImportEvent::Probing { done: 2, total: 2 })));
        assert!(matches!(
            events.last(),
            Some(ImportEvent::ComparingCovers { done: 2, total: 2 })
        ));

        project.chapters[0].title = "Edited chapter".into();
        let previous_cover = project.cover.clone();
        archive(
            &input,
            &[("track12.mp3", &c, 0o100644), ("track2.mp3", &a, 0o100644)],
        );
        project
            .import_assets(&backend, vec![input.clone()], &assets)
            .unwrap();
        assert_eq!(project.chapters[0].title, "Edited chapter");
        assert_eq!(project.cover, previous_cover);

        let mut conflicting = Project::default();
        conflicting
            .import_assets(&backend, vec![input.clone()], &assets)
            .unwrap();
        assert!(conflicting.cover.is_none());
        let cancel = Job::default();
        let before = fs::read_dir(&assets).unwrap().count();
        let mut cancelled = Project::default();
        assert!(cancelled
            .import_assets_observed(
                &backend,
                vec![input],
                &assets,
                &cancel,
                None,
                &mut |event| if matches!(event, ImportEvent::Probing { done: 1, .. }) {
                    cancel.cancel.store(true, AtomicOrdering::Relaxed);
                }
            )
            .is_err());
        assert!(cancelled.tracks.is_empty());
        assert_eq!(fs::read_dir(&assets).unwrap().count(), before);
    }

    #[test]
    fn persistent_transactional_import_and_named_project() {
        assert!(Project::named("  ").is_err());
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("book.zip");
        let assets = dir.path().join("book.assets");
        archive(
            &zip,
            &[
                ("part10-deflate.mp3", b"audio", 0o100644),
                ("part2.mp3", b"audio", 0o100644),
            ],
        );
        let mut p = Project::named(" Book ").unwrap();
        p.import_assets(&Backend, vec![zip.clone()], &assets)
            .unwrap();
        assert_eq!(p.name, "Book");
        assert!(p.tracks[0].path.ends_with("part2.mp3"));
        fs::remove_file(&zip).unwrap();
        let project = dir.path().join("book.json");
        p.save_new(&project).unwrap();
        let loaded = Project::load(&project).unwrap();
        assert!(loaded.tracks.iter().all(|t| t.path.is_file()));
        let before = format!("{p:?}");
        let batches = fs::read_dir(&assets).unwrap().count();
        archive(
            &zip,
            &[
                ("valid.mp3", b"audio", 0o100644),
                ("bad.mp3", b"invalid", 0o100644),
            ],
        );
        assert!(p.import_assets(&Backend, vec![zip], &assets).is_err());
        assert_eq!(format!("{p:?}"), before);
        assert_eq!(fs::read_dir(&assets).unwrap().count(), batches);
    }
    #[test]
    fn hostile_archives_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("hostile.zip");
        let assets = dir.path().join("assets");
        let mut p = Project::default();
        for entries in [
            vec![("../escape.mp3", b"audio".as_slice(), 0o100644)],
            vec![("/escape.mp3", b"audio".as_slice(), 0o100644)],
            vec![("link.mp3", b"audio".as_slice(), 0o120777)],
            vec![
                ("a.mp3", b"audio".as_slice(), 0o100644),
                ("A.mp3", b"audio".as_slice(), 0o100644),
            ],
            vec![("notes.txt", b"audio".as_slice(), 0o100644)],
            vec![("CON.mp3", b"audio".as_slice(), 0o100644)],
        ] {
            archive(&zip, &entries);
            assert!(p
                .import_assets(&Backend, vec![zip.clone()], &assets)
                .is_err());
            assert!(p.tracks.is_empty());
            assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
        }
    }
    fn write_rar(path: &Path, options: rar_rs::WriterOptions, redirect: Option<u64>) {
        let entries =
            rar_rs::EntryWriteOptions::new().compression_level(rar_rs::CompressionLevel::STORE);
        let mut writer = rar_rs::ArchiveWriter::create_with(path, options).unwrap();
        writer
            .add_bytes("book/track10.mp3", b"audio", entries)
            .unwrap();
        writer
            .add_bytes("book/track2.mp3", b"audio", entries)
            .unwrap();
        if let Some(kind) = redirect {
            writer
                .add_redirect("book/link.mp3", kind, "book/track2.mp3")
                .unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn rar_password_integrity_and_cancellation_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let rar = dir.path().join("book.rar");
        let assets = dir.path().join("assets");
        let mut project = Project::default();
        for encrypted_headers in [false, true] {
            write_rar(
                &rar,
                rar_rs::WriterOptions::new()
                    .password("fixture")
                    .encrypt_headers(encrypted_headers),
                None,
            );
            for password in [None, Some("wrong")] {
                assert!(project
                    .import_assets_with(
                        &Backend,
                        vec![rar.clone()],
                        &assets,
                        &Job::default(),
                        password
                    )
                    .is_err());
                assert!(project.tracks.is_empty());
                assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
            }
            let job = Job::default();
            let mut staged = false;
            let result = project.import_assets_observed(
                &Backend,
                vec![rar.clone()],
                &assets,
                &job,
                Some("fixture"),
                &mut |event| {
                    if matches!(event, ImportEvent::Extracting { done: 1, total: 2 }) {
                        staged = true;
                        job.cancel.store(true, AtomicOrdering::Relaxed);
                    }
                },
            );
            assert!(staged, "{result:?}");
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
            assert!(project.tracks.is_empty());
        }

        project
            .import_assets_with(
                &Backend,
                vec![rar.clone()],
                &assets,
                &Job::default(),
                Some("fixture"),
            )
            .unwrap();
        assert!(project.tracks[0].path.ends_with("track2.mp3"));
        assert!(project.tracks[1].path.ends_with("track10.mp3"));

        write_rar(&rar, rar_rs::WriterOptions::new(), None);
        let reader = rar_rs::ArchiveReader::open(&rar).unwrap();
        let offset = reader.entries().nth(1).unwrap().data_offset() as usize;
        drop(reader);
        let mut damaged = fs::read(&rar).unwrap();
        damaged[offset] ^= 1;
        fs::write(&rar, damaged).unwrap();
        let error = project
            .import_assets(&Backend, vec![rar], &assets)
            .unwrap_err();
        assert!(
            format!("{error:#}").to_lowercase().contains("crc"),
            "{error:#}"
        );
        assert_eq!(project.tracks.len(), 2);
        assert_eq!(fs::read_dir(&assets).unwrap().count(), 1);
    }

    #[test]
    fn rar_rejects_redirects_and_ancestor_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let rar = dir.path().join("book.rar");
        let assets = dir.path().join("assets");
        let mut project = Project::default();
        for kind in 1..=5 {
            write_rar(&rar, rar_rs::WriterOptions::new(), Some(kind));
            let error = project
                .import_assets(&Backend, vec![rar.clone()], &assets)
                .unwrap_err();
            assert!(format!("{error:#}").contains("redirections"), "{error:#}");
        }
        let mut writer = rar_rs::ArchiveWriter::create(&rar).unwrap();
        for name in ["track.mp3", "TRACK.mp3/child.mp3"] {
            writer
                .add_bytes(name, b"audio", rar_rs::EntryWriteOptions::new())
                .unwrap();
        }
        writer.finish().unwrap();
        let error = project
            .import_assets(&Backend, vec![rar.clone()], &assets)
            .unwrap_err();
        assert!(format!("{error:#}").contains("file ancestor"), "{error:#}");
        assert!(project.tracks.is_empty());
        assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
    }

    #[test]
    fn separate_input_roots_keep_natural_track_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in ["track10.mp3", "track2.mp3", "track1.mp3"] {
            let path = dir.path().join(name);
            fs::write(&path, b"audio").unwrap();
            paths.push(path);
        }
        let mut project = Project::default();
        project
            .import_assets(&Backend, paths, &dir.path().join("assets"))
            .unwrap();
        for (track, expected) in
            project
                .tracks
                .iter()
                .zip(["track1.mp3", "track2.mp3", "track10.mp3"])
        {
            assert!(track.path.ends_with(expected));
        }
    }
    #[test]
    fn quotas_and_corrupt_zip() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("bad.zip");
        let assets = dir.path().join("assets");
        let mut p = Project::default();
        archive(&zip, &[("a.mp3", b"audio", 0o100644)]);
        let original = fs::read(&zip).unwrap();
        let central = original
            .windows(4)
            .position(|b| b == b"PK\x01\x02")
            .unwrap();
        let mut too_many = original.clone();
        let end = too_many.len() - 22;
        too_many[end + 8..end + 10].copy_from_slice(&4097u16.to_le_bytes());
        too_many[end + 10..end + 12].copy_from_slice(&4097u16.to_le_bytes());
        fs::write(&zip, too_many).unwrap();
        assert!(p
            .import_assets(&Backend, vec![zip.clone()], &assets)
            .is_err());
        let mut oversized = original.clone();
        oversized[central + 24..central + 28]
            .copy_from_slice(&((MAX_FILE + 1) as u32).to_le_bytes());
        fs::write(&zip, oversized).unwrap();
        assert!(p
            .import_assets(&Backend, vec![zip.clone()], &assets)
            .is_err());
        for (position, value) in [
            (end + 12, 1u32),
            (central + 42, central as u32),
            (central + 20, u32::MAX - 1),
        ] {
            let mut malformed = original.clone();
            malformed[position..position + 4].copy_from_slice(&value.to_le_bytes());
            fs::write(&zip, malformed).unwrap();
            assert!(p
                .import_assets(&Backend, vec![zip.clone()], &assets)
                .is_err());
            assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
        }
        let mut corrupt = original;
        corrupt[35] ^= 1;
        fs::write(&zip, corrupt).unwrap();
        assert!(p.import_assets(&Backend, vec![zip], &assets).is_err());
        assert_eq!(fs::read_dir(&assets).unwrap().count(), 0);
    }
}

struct CheckedRead<'a, R> {
    input: R,
    job: &'a Job,
}
impl<R: Read> Read for CheckedRead<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.job.check().map_err(std::io::Error::other)?;
        self.input.read(buffer)
    }
}

/// Shared validation and staging for native archive readers.
pub(crate) struct ArchiveBatch<'a> {
    root: &'a Path,
    total: &'a mut u64,
    count: &'a mut usize,
    pub job: &'a Job,
    event: &'a mut dyn FnMut(ImportEvent),
    paths: std::collections::HashMap<String, bool>,
    declared: u64,
    files: usize,
    media: Vec<PathBuf>,
}

impl<'a> ArchiveBatch<'a> {
    fn new(
        root: &'a Path,
        total: &'a mut u64,
        count: &'a mut usize,
        job: &'a Job,
        event: &'a mut dyn FnMut(ImportEvent),
    ) -> Self {
        Self {
            root,
            total,
            count,
            job,
            event,
            paths: Default::default(),
            declared: 0,
            files: 0,
            media: Vec::new(),
        }
    }

    pub fn remaining_entries(&self) -> usize {
        MAX_FILES - *self.count
    }

    pub fn register(&mut self, name: &str, directory: bool, size: u64) -> Result<PathBuf> {
        self.job.check()?;
        ensure!(
            self.remaining_entries() > 0,
            "Import exceeds 4096-entry limit"
        );
        *self.count += 1;
        let path = safe_name(name)?;
        let key = name.trim_end_matches('/').to_uppercase();
        ensure!(
            self.paths.insert(key, directory).is_none(),
            "Duplicate archive path: {name}"
        );
        ensure!(size <= MAX_FILE, "Archive entry exceeds 2 GiB: {name}");
        ensure!(
            !directory || size == 0,
            "Directory entry contains data: {name}"
        );
        self.declared += size;
        ensure!(
            self.declared <= MAX_BYTES - *self.total,
            "Archive exceeds 8 GiB batch expanded limit"
        );
        if !directory {
            ensure!(audio(&path), "Unsupported archive member: {name}");
            self.files += 1;
        }
        Ok(path)
    }

    pub fn start(&mut self) -> Result<()> {
        for name in self.paths.keys() {
            self.job.check()?;
            for (offset, _) in name.match_indices('/') {
                ensure!(
                    self.paths.get(&name[..offset]) != Some(&false),
                    "Archive path has a file ancestor"
                );
            }
        }
        (self.event)(ImportEvent::Extracting {
            done: 0,
            total: self.files,
        });
        Ok(())
    }

    pub fn stage(
        &mut self,
        path: &Path,
        size: u64,
        extract: impl FnOnce(&mut dyn Write) -> Result<()>,
    ) -> Result<()> {
        self.job.check()?;
        ensure!(
            size <= MAX_BYTES - *self.total,
            "Archive output exceeds batch quota"
        );
        let dest = self.root.join(path);
        fs::create_dir_all(dest.parent().context("Archive member parent")?)?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)?;
        let mut writer = MemberWriter {
            file,
            job: self.job,
            remaining: size,
        };
        extract(&mut writer)
            .with_context(|| format!("Extract archive member {}", path.display()))?;
        ensure!(
            writer.remaining == 0,
            "Archive member ended before its declared size"
        );
        writer.file.sync_all()?;
        *self.total += size;
        self.media.push(dest);
        (self.event)(ImportEvent::Extracting {
            done: self.media.len(),
            total: self.files,
        });
        Ok(())
    }
}

struct MemberWriter<'a> {
    file: fs::File,
    job: &'a Job,
    remaining: u64,
}

impl Write for MemberWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.job.check().map_err(std::io::Error::other)?;
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Archive output exceeds declared member size",
            ));
        }
        let written = self.file.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

pub(crate) fn check_unix_kind(mode: u64, directory: bool) -> Result<()> {
    let kind = mode & 0o170000;
    ensure!(
        kind == 0 || kind == if directory { 0o040000 } else { 0o100000 },
        "Archive links or special files are forbidden"
    );
    Ok(())
}
