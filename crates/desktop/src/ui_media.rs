use anyhow::{ensure, Context, Result};
use eframe::egui;
use m4b_editor_core::{Cancel, DesktopFfmpeg, ExportFormat, Project};
use serde::Deserialize;
use std::{
    fs::{self, File, Metadata},
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

pub fn check_m4b_header(path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("m4b")),
        "Expected an M4B extension"
    );
    ensure!(fs::metadata(path)?.is_file(), "Expected an M4B file");
    let mut file = File::open(path)?;
    let mut header = [0; 16];
    file.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header[..4].try_into()?);
    ensure!(
        &header[4..8] == b"ftyp"
            && size >= 16
            && size % 4 == 0
            && u64::from(size) <= file.metadata()?.len(),
        "Expected an MPEG-4 audio file type box"
    );
    Ok(())
}

pub fn open_m4b(backend: &DesktopFfmpeg, path: &Path) -> Result<Project> {
    check_m4b_header(path)?;
    backend.open(path)
}

pub fn recent_cover(backend: &DesktopFfmpeg, path: &Path) -> Option<egui::ColorImage> {
    #[derive(Deserialize)]
    struct CoverMetadata {
        version: u32,
        cover: Option<PathBuf>,
    }
    const PROJECT_LIMIT: u64 = 16 * 1024 * 1024;
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > PROJECT_LIMIT {
        return None;
    }
    let file = File::open(path).ok()?;
    let metadata: CoverMetadata = serde_json::from_reader(file.take(PROJECT_LIMIT + 1)).ok()?;
    if metadata.version != 1 {
        return None;
    }
    let mut cover = metadata.cover?;
    if cover.is_relative() {
        cover = dunce::canonicalize(path).ok()?.parent()?.join(cover);
    }
    load_cover(backend, &cover, 96).ok()
}

pub fn load_cover(backend: &DesktopFfmpeg, path: &Path, max_side: u32) -> Result<egui::ColorImage> {
    ensure!(fs::metadata(path)?.is_file(), "Cover must be a file");
    let dir = tempfile::tempdir()?;
    let extracted = dir.path().join("cover.png");
    let is_png = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let source = if is_png {
        path
    } else {
        backend.extract_cover_preview(path, &extracted)?;
        &extracted
    };
    let file = File::open(source)?;
    let cover_limit = 16 * 1024 * 1024;
    ensure!(
        file.metadata()?.len() <= cover_limit,
        "Cover exceeds 16 MiB"
    );
    let reader = std::io::BufReader::new(file.take(cover_limit + 1));
    let mut reader = image::ImageReader::new(reader).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    let image = if image.width() > max_side || image.height() > max_side {
        image.thumbnail(max_side, max_side)
    } else {
        image
    }
    .to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

pub fn bitrates(format: ExportFormat) -> &'static [u32] {
    // The backend encodes at 48 kHz, where these are all valid MPEG-1 Layer III rates.
    match format {
        ExportFormat::M4b | ExportFormat::Mp3 => &[48, 64, 96, 128, 160, 192, 256, 320],
        ExportFormat::Flac => &[],
    }
}

pub struct Replacement {
    path: PathBuf,
    original: Metadata,
}

impl Replacement {
    pub fn prepare<'a>(
        path: &Path,
        protected: impl IntoIterator<Item = &'a PathBuf>,
    ) -> Result<Self> {
        let original = path.symlink_metadata()?;
        ensure!(
            original.is_file(),
            "Cannot overwrite a non-regular destination"
        );
        let path = path.canonicalize()?;
        for source in protected {
            ensure!(
                source.canonicalize().ok().as_ref() != Some(&path),
                "Cannot overwrite a project, source track or cover"
            );
        }
        Ok(Self { path, original })
    }

    pub fn staging_directory(&self) -> Result<tempfile::TempDir> {
        Ok(tempfile::tempdir_in(
            self.path.parent().context("Missing output parent")?,
        )?)
    }

    pub fn publish(self, staged: &Path, cancel: &Cancel) -> Result<()> {
        let current = self.path.symlink_metadata()?;
        let same = current.is_file()
            && current.len() == self.original.len()
            && current.modified()? == self.original.modified()?
            && current.created().ok() == self.original.created().ok();
        #[cfg(unix)]
        let same = {
            use std::os::unix::fs::MetadataExt;
            same && current.dev() == self.original.dev() && current.ino() == self.original.ino()
        };
        ensure!(same, "Output changed since overwrite consent");
        ensure!(!cancel.load(Ordering::Relaxed), "Export cancelled");
        fs::rename(staged, self.path)?;
        Ok(())
    }
}
