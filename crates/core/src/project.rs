use crate::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    M4b,
    Mp3,
    Flac,
}
impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::M4b => "m4b",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
        }
    }
    pub fn encoder(self) -> &'static str {
        match self {
            Self::M4b => "aac",
            Self::Mp3 => "libmp3lame",
            Self::Flac => "flac",
        }
    }
    pub fn codec(self) -> &'static str {
        match self {
            Self::M4b => "aac",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
        }
    }
}
impl Project {
    /// Associate chapters by half-open track intervals before any edit. Invalid
    /// timelines are rejected before mutation rather than silently regenerated.
    pub(crate) fn chapter_anchors(&self) -> Result<Vec<Vec<Chapter>>> {
        let mut result = vec![Vec::new(); self.tracks.len()];
        ensure!(
            self.chapters
                .windows(2)
                .all(|w| w[0].start_ms < w[1].start_ms),
            "Ambiguous chapter ordering; fix chapter starts before editing tracks"
        );
        for chapter in &self.chapters {
            let mut start = 0u64;
            let mut found = false;
            for (i, t) in self.tracks.iter().enumerate() {
                let end = start
                    .checked_add(t.duration_ms)
                    .context("Duration overflow")?;
                if chapter.start_ms >= start && chapter.start_ms < end {
                    result[i].push(Chapter {
                        start_ms: chapter.start_ms - start,
                        title: chapter.title.clone(),
                    });
                    found = true;
                    break;
                }
                start = end;
            }
            ensure!(
                found,
                "Chapter outside source audio; fix chapter before editing tracks"
            );
        }
        Ok(result)
    }
    pub fn reorder_tracks(&mut self, order: &[usize]) -> Result<()> {
        ensure!(
            order.len() == self.tracks.len(),
            "Invalid track permutation"
        );
        let mut seen = vec![false; order.len()];
        for &i in order {
            ensure!(i < seen.len() && !seen[i], "Invalid track permutation");
            seen[i] = true;
        }
        self.retain_tracks(order)
    }
    pub fn remove_track(&mut self, index: usize) -> Result<()> {
        ensure!(index < self.tracks.len(), "Invalid track index");
        let order: Vec<_> = (0..self.tracks.len()).filter(|&i| i != index).collect();
        self.retain_tracks(&order)
    }
    fn retain_tracks(&mut self, order: &[usize]) -> Result<()> {
        let anchors = self.chapter_anchors()?;
        let mut tracks = Vec::new();
        let mut chapters = Vec::new();
        let mut start = 0u64;
        for &i in order {
            if tracks.is_empty() && anchors[i].first().is_none_or(|c| c.start_ms != 0) {
                chapters.push(Chapter {
                    start_ms: 0,
                    title: self.tracks[i]
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                });
            }
            for c in &anchors[i] {
                chapters.push(Chapter {
                    start_ms: start.checked_add(c.start_ms).context("Duration overflow")?,
                    title: c.title.clone(),
                });
            }
            start = start
                .checked_add(self.tracks[i].duration_ms)
                .context("Duration overflow")?;
            tracks.push(self.tracks[i].clone());
        }
        self.tracks = tracks;
        self.chapters = chapters;
        Ok(())
    }
    /// Publish a complete portable directory without replacing an existing path.
    /// Files are staged in a sibling directory, then published atomically with
    /// a platform no-replace rename. Failure removes only the private stage.
    pub fn copy_to(&self, destination: &Path) -> Result<PathBuf> {
        self.copy_to_with_job(destination, &Job::default())
    }
    pub fn copy_to_with_job(&self, destination: &Path, job: &Job) -> Result<PathBuf> {
        job.check()?;
        self.validate()?;
        let bytes = self
            .tracks
            .iter()
            .map(|t| &t.path)
            .chain(self.cover.iter())
            .try_fold(0u64, |sum, path| -> Result<u64> {
                sum.checked_add(fs::metadata(path)?.len())
                    .context("Portable copy size overflow")
            })?;
        ensure!(
            available_space(parent_dir(destination))? >= bytes.saturating_add(1024 * 1024),
            "Insufficient disk space for portable copy"
        );
        ensure!(!destination.exists(), "Portable destination must not exist");
        let stage = tempfile::Builder::new()
            .prefix(".portable-")
            .tempdir_in(parent_dir(destination))?;
        let root = stage.path();
        let result = (|| {
            let mut portable = self.clone();
            let media = root.join("media");
            fs::create_dir(&media)?;
            for (i, t) in portable.tracks.iter_mut().enumerate() {
                let name = format!("track-{i}.{}", safe_extension(&t.path));
                crate::jobs::copy_file(&t.path, &media.join(&name), job)?;
                t.path = PathBuf::from("media").join(name);
            }
            if let Some(cover) = &mut portable.cover {
                let name = format!("cover.{}", safe_extension(cover));
                crate::jobs::copy_file(cover, &media.join(&name), job)?;
                *cover = PathBuf::from("media").join(name);
            }
            let project = root.join("book.m4bproj");
            portable.save_new(&project)?;
            Project::load(&project)?.validate()?;
            job.check()?;
            publish_directory(root, destination)?;
            Ok(destination.join("book.m4bproj"))
        })();
        result
    }
}
impl DesktopFfmpeg {
    /// Encode a bounded book excerpt, including cross-track transitions, through
    /// the same selected encoder and validation path as a complete export.
    pub fn encoded_preview(
        &self,
        project: &Project,
        output: &Path,
        start_ms: u64,
        duration_ms: u64,
        cancel: Cancel,
        event: &mut dyn FnMut(ExportEvent),
    ) -> Result<()> {
        project.validate()?;
        ensure!(
            duration_ms > 0 && duration_ms <= 30_000,
            "Preview duration must be 1–30000 ms"
        );
        ensure!(
            start_ms < project.duration_ms(),
            "Preview starts outside book"
        );
        ensure!(!output.exists(), "Preview output exists");
        let stage = tempfile::tempdir_in(parent_dir(output))?;
        let end_ms = start_ms
            .saturating_add(duration_ms)
            .min(project.duration_ms());
        let mut excerpt = Project {
            metadata: project.metadata.clone(),
            settings: project.settings.clone(),
            ..Project::default()
        };
        excerpt.settings.lossless_remux = false;
        let mut cursor = 0u64;
        for (i, t) in project.tracks.iter().enumerate() {
            let end = cursor
                .checked_add(t.duration_ms)
                .context("Duration overflow")?;
            let from = cursor.max(start_ms);
            let to = end.min(end_ms);
            if to > from {
                let path = stage.path().join(format!("preview-{i}.wav"));
                let args = vec![
                    "-n".into(),
                    "-ss".into(),
                    format!("{:.3}", (from - cursor) as f64 / 1000.).into(),
                    "-i".into(),
                    t.path.as_os_str().into(),
                    "-t".into(),
                    format!("{:.3}", (to - from) as f64 / 1000.).into(),
                    "-map".into(),
                    t.stream_index
                        .map(|i| format!("0:{i}"))
                        .unwrap_or_else(|| "0:a:0".into())
                        .into(),
                    "-vn".into(),
                    "-c:a".into(),
                    "pcm_s16le".into(),
                    path.as_os_str().into(),
                ];
                self.run(
                    &args,
                    &cancel,
                    event,
                    0. ..0.,
                    to - from,
                    super::ENCODER_ALLOCATION_LIMIT,
                )?;
                excerpt.tracks.push(self.probe(&path)?.track);
            }
            cursor = end;
        }
        excerpt.reset_chapters();
        self.export(&excerpt, output, cancel, event)
    }
}

#[cfg(target_os = "linux")]
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    ensure!(
        unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } == 0,
        "Publish portable project: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
#[cfg(target_os = "macos")]
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    ensure!(
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) } == 0,
        "Publish portable project: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
#[cfg(windows)]
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination)?;
    Ok(())
}
impl Project {
    /// Rebind one track to an explicitly selected audio stream, retaining every
    /// track-relative chapter. Reject a chapter outside the replacement stream.
    pub fn select_stream(
        &mut self,
        backend: &DesktopFfmpeg,
        index: usize,
        stream: u32,
    ) -> Result<()> {
        let old = self.tracks.get(index).context("Invalid track index")?;
        let replacement = backend.probe_stream(&old.path, Some(stream))?.track;
        let anchors = self.chapter_anchors()?;
        ensure!(anchors[index].iter().all(|c| c.start_ms < replacement.duration_ms), "A custom chapter lies outside the selected stream; adjust chapters before changing streams");
        let mut next = self.clone();
        next.tracks[index] = replacement;
        let mut start = 0u64;
        let mut chapters = Vec::new();
        for (track, relative) in next.tracks.iter().zip(anchors) {
            for chapter in relative {
                chapters.push(Chapter {
                    start_ms: start
                        .checked_add(chapter.start_ms)
                        .context("Duration overflow")?,
                    title: chapter.title,
                });
            }
            start = start
                .checked_add(track.duration_ms)
                .context("Duration overflow")?;
        }
        next.chapters = chapters;
        *self = next;
        Ok(())
    }
}

fn safe_extension(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("bin")
        .to_ascii_lowercase()
}

/// Content baseline of the exact project bytes loaded or saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectBaseline([u8; 32]);
fn project_digest(bytes: &[u8]) -> ProjectBaseline {
    use sha2::{Digest, Sha256};
    ProjectBaseline(Sha256::digest(bytes).into())
}
fn project_lock(path: &Path) -> Result<fs::File> {
    let parent = dunce::canonicalize(parent_dir(path))?;
    let name = path.file_name().context("Project has no filename")?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(".m4b-lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(lock_name))?;
    lock.lock()?;
    Ok(lock)
}
impl Project {
    pub fn load_with_baseline(path: &Path) -> Result<(Self, ProjectBaseline)> {
        let canonical = dunce::canonicalize(path)?;
        let path = canonical.as_path();
        // Atomic project publication gives readers a complete snapshot. Only
        // writers need the sidecar lock; opening a read-only project must work.
        let bytes = fs::read(path)?;
        Ok((Self::load_bytes(path, &bytes)?, project_digest(&bytes)))
    }
    /// Compare and replace while holding the same cross-process lock. `None`
    /// means an explicitly consented Save As replacement.
    pub fn save_with_baseline(
        &self,
        path: &Path,
        expected: Option<&ProjectBaseline>,
    ) -> Result<ProjectBaseline> {
        let canonical = dunce::canonicalize(path).ok();
        let path = canonical.as_deref().unwrap_or(path);
        let _lock = project_lock(path)?;
        if let Some(expected) = expected {
            let current =
                fs::read(path).context("Project changed or was removed since it was loaded")?;
            ensure!(
                &project_digest(&current) == expected,
                "Project changed since it was loaded; use Save As or reload"
            );
        }
        self.save(path)?;
        Ok(project_digest(&fs::read(path)?))
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    #[test]
    fn loading_snapshot_does_not_require_a_writable_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.m4bproj");
        Project::default().save(&path).unwrap();
        // A directory occupying the sidecar name also prevents its creation
        // when tests run as root, where chmod alone cannot prove read-only use.
        fs::create_dir(dir.path().join(".book.m4bproj.m4b-lock")).unwrap();
        let original = fs::read(&path).unwrap();
        let (project, baseline) = Project::load_with_baseline(&path).unwrap();
        assert!(project.save_with_baseline(&path, Some(&baseline)).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn concurrent_saves_accept_only_one_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.m4bproj");
        Project::default().save(&path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut handles = Vec::new();
        for title in ["first", "second"] {
            let (mut project, baseline) = Project::load_with_baseline(&path).unwrap();
            project.metadata.title = title.into();
            let path = path.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                project.save_with_baseline(&path, Some(&baseline)).is_ok()
            }));
        }
        barrier.wait();
        let successes = handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
        assert!(
            ["first", "second"].contains(&Project::load(&path).unwrap().metadata.title.as_str())
        );
    }

    #[test]
    fn stale_instances_and_external_edits_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.m4bproj");
        Project::default().save(&path).unwrap();
        let (mut first, a) = Project::load_with_baseline(&path).unwrap();
        let (mut second, b) = Project::load_with_baseline(&path).unwrap();
        first.metadata.title = "first".into();
        first.save_with_baseline(&path, Some(&a)).unwrap();
        let newer = fs::read(&path).unwrap();
        second.metadata.title = "second".into();
        assert!(second.save_with_baseline(&path, Some(&b)).is_err());
        assert_eq!(fs::read(&path).unwrap(), newer);
        let (_, current) = Project::load_with_baseline(&path).unwrap();
        fs::write(&path, b"external change").unwrap();
        assert!(second.save_with_baseline(&path, Some(&current)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external change");
        second.save_with_baseline(&path, None).unwrap();
        assert_eq!(Project::load(&path).unwrap().metadata.title, "second");
    }
}
