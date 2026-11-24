mod assets;
mod jobs;
mod mp4;
pub use jobs::{available_space, preflight, required_export_bytes, Job};
mod project;
pub use project::{ExportFormat, ProjectBaseline};
mod rar;
mod zip;
// Portable project model and media contracts. Only `DesktopFfmpeg` spawns processes.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub narrator: String,
    pub description: String,
    pub genre: String,
    pub date: String,
    pub copyright: String,
    pub publisher: String,
    pub book_language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    /// Absolute FFmpeg stream index; None selects the first audio stream in legacy projects.
    #[serde(default)]
    pub stream_index: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub start_ms: u64,
    pub title: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub format: ExportFormat,
    pub language: String,
    pub theme: String,
    pub bitrate_kbps: u32,
    #[serde(default = "default_remux")]
    pub lossless_remux: bool,
}
fn default_remux() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            format: ExportFormat::M4b,
            language: "en".into(),
            theme: "system".into(),
            bitrate_kbps: 160,
            lossless_remux: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub name: String,
    pub version: u32,
    pub metadata: Metadata,
    pub tracks: Vec<Track>,
    pub chapters: Vec<Chapter>,
    pub cover: Option<PathBuf>,
    pub settings: Settings,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: 1,
            metadata: Metadata::default(),
            tracks: vec![],
            chapters: vec![],
            cover: None,
            settings: Settings::default(),
        }
    }
}
impl Project {
    pub fn named(name: &str) -> Result<Self> {
        ensure!(!name.trim().is_empty(), "Project name is required");
        let mut project = Self {
            name: name.trim().to_owned(),
            ..Self::default()
        };
        project.metadata.title = project.name.clone();
        Ok(project)
    }
    pub fn duration_ms(&self) -> u64 {
        self.tracks
            .iter()
            .fold(0u64, |sum, t| sum.saturating_add(t.duration_ms))
    }
    pub fn estimated_bytes(&self) -> u64 {
        if self.settings.format == ExportFormat::Flac {
            return self.duration_ms().saturating_mul(192);
        }
        self.duration_ms()
            .saturating_mul(u64::from(self.settings.bitrate_kbps))
            / 8
    }
    pub fn reset_chapters(&mut self) {
        let mut start = 0u64;
        self.chapters = self
            .tracks
            .iter()
            .map(|t| {
                let c = Chapter {
                    start_ms: start,
                    title: t
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                };
                start = start.saturating_add(t.duration_ms);
                c
            })
            .collect();
    }
    pub fn import(&mut self, backend: &dyn MediaBackend, paths: Vec<PathBuf>) -> Result<()> {
        self.import_with_job(backend, paths, &Job::default())
    }
    pub fn import_with_job(
        &mut self,
        backend: &dyn MediaBackend,
        paths: Vec<PathBuf>,
        job: &Job,
    ) -> Result<()> {
        self.import_observed(backend, paths, job, &mut |_| {})
    }

    fn import_observed(
        &mut self,
        backend: &dyn MediaBackend,
        mut paths: Vec<PathBuf>,
        job: &Job,
        event: &mut dyn FnMut(ImportEvent),
    ) -> Result<()> {
        job.check()?;
        self.chapter_anchors()?;
        paths.sort_by(|a, b| {
            natural_cmp(
                &a.file_name().unwrap_or(a.as_os_str()).to_string_lossy(),
                &b.file_name().unwrap_or(b.as_os_str()).to_string_lossy(),
            )
            .then_with(|| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()))
        });
        let mut probes = Vec::with_capacity(paths.len());
        event(ImportEvent::Probing {
            done: 0,
            total: paths.len(),
        });
        for path in &paths {
            probes.push(backend.probe_with_job(path, job)?);
            event(ImportEvent::Probing {
                done: probes.len(),
                total: paths.len(),
            });
        }
        job.check()?;
        let mut next = self.clone();
        if self.tracks.is_empty() && self.chapters.is_empty() && probes.len() == 1 {
            next.chapters.clone_from(&probes[0].chapters);
        }
        for (field, tag) in [
            (
                &mut next.metadata.title,
                (|m: &Metadata| m.title.as_str()) as fn(&Metadata) -> &str,
            ),
            (&mut next.metadata.author, |m| &m.author),
            (&mut next.metadata.narrator, |m| &m.narrator),
            (&mut next.metadata.description, |m| &m.description),
            (&mut next.metadata.genre, |m| &m.genre),
            (&mut next.metadata.date, |m| &m.date),
            (&mut next.metadata.copyright, |m| &m.copyright),
            (&mut next.metadata.publisher, |m| &m.publisher),
        ] {
            if field.is_empty() && self.tracks.is_empty() {
                let mut values = probes
                    .iter()
                    .map(|p| tag(&p.metadata).trim())
                    .filter(|value| !value.is_empty() && *value != "und");
                if let Some(value) = values.next() {
                    if values.all(|other| other == value) {
                        *field = value.to_owned();
                    }
                }
            }
        }
        // Audio tags may contain names, two-letter codes or multiple languages.
        // Do not guess a conversion or import a value that prevents saving.
        if next.metadata.book_language.is_empty() && self.tracks.is_empty() {
            let mut values = probes
                .iter()
                .map(|probe| probe.metadata.book_language.trim())
                .filter(|value| !value.is_empty() && *value != "und");
            if let Some(value) = values.next() {
                if value.len() == 3
                    && value.bytes().all(|byte| byte.is_ascii_lowercase())
                    && values.all(|other| other == value)
                {
                    next.metadata.book_language = value.to_owned();
                }
            }
        }
        if next.cover.is_none() && self.tracks.is_empty() {
            let covers: Vec<_> = probes.iter().filter(|probe| probe.has_cover).collect();
            if let Some(first) = covers.first() {
                let mut same = covers.len() == 1;
                if covers.len() > 1 {
                    event(ImportEvent::ComparingCovers {
                        done: 0,
                        total: covers.len(),
                    });
                    let fingerprint = backend.cover_fingerprint(&first.track.path, job);
                    same = fingerprint.is_some();
                    for (index, cover) in covers.iter().enumerate().skip(1) {
                        job.check()?;
                        if !same || backend.cover_fingerprint(&cover.track.path, job) != fingerprint
                        {
                            same = false;
                            break;
                        }
                        event(ImportEvent::ComparingCovers {
                            done: index + 1,
                            total: covers.len(),
                        });
                    }
                }
                if same {
                    next.cover = Some(first.track.path.clone());
                }
            }
        }
        job.check()?;
        let embedded_chapters = self.tracks.is_empty() && !next.chapters.is_empty();
        let mut start = self.duration_ms();
        for probe in probes {
            let track = probe.track;
            if !embedded_chapters {
                next.chapters.push(Chapter {
                    start_ms: start,
                    title: track
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                });
            }
            start = start
                .checked_add(track.duration_ms)
                .context("Duration overflow")?;
            next.tracks.push(track);
        }
        *self = next;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported project version {}",
            self.version
        );
        ensure!(!self.tracks.is_empty(), "No audio tracks");
        ensure!(
            self.metadata.book_language.is_empty()
                || (self.metadata.book_language.len() == 3
                    && self
                        .metadata
                        .book_language
                        .bytes()
                        .all(|b| b.is_ascii_lowercase())),
            "Book language must be a lowercase ISO 639-2 code (for example fra or eng)"
        );
        ensure!(
            self.tracks
                .iter()
                .try_fold(0u64, |sum, t| sum.checked_add(t.duration_ms))
                .is_some(),
            "Project duration overflow"
        );
        ensure!(
            [48, 64, 96, 128, 160, 192, 256, 320].contains(&self.settings.bitrate_kbps),
            "Unsupported AAC bitrate"
        );
        ensure!(
            self.tracks
                .iter()
                .all(|t| t.duration_ms > 0 && t.path.is_file()),
            "Missing or empty source track"
        );
        ensure!(
            self.chapters.first().map(|c| c.start_ms) == Some(0),
            "First chapter must start at 0"
        );
        ensure!(
            self.chapters
                .windows(2)
                .all(|w| w[0].start_ms < w[1].start_ms),
            "Chapter starts must strictly increase"
        );
        ensure!(
            self.chapters
                .iter()
                .all(|c| c.start_ms < self.duration_ms()),
            "Chapter outside audio duration"
        );
        if let Some(p) = &self.cover {
            ensure!(p.is_file(), "Cover does not exist");
        }
        Ok(())
    }
    pub fn can_remux(&self) -> bool {
        self.settings.format == ExportFormat::M4b
            && self.settings.lossless_remux
            && self.tracks.len() == 1
            && self.tracks[0].codec == "aac"
            && self.tracks[0]
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("m4b") || e.eq_ignore_ascii_case("m4a"))
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.write_project(path, true)
    }
    /// Creates a project atomically and refuses an existing destination.
    pub fn save_new(&self, path: &Path) -> Result<()> {
        self.write_project(path, false)
    }
    fn write_project(&self, path: &Path, replace: bool) -> Result<()> {
        if let Ok(target) = dunce::canonicalize(path) {
            for source in self.tracks.iter().map(|t| &t.path).chain(self.cover.iter()) {
                ensure!(
                    dunce::canonicalize(source).ok().as_ref() != Some(&target),
                    "Cannot save project over a source or cover"
                );
            }
        }
        let parent = parent_dir(path);
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temp, self)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        if replace {
            temp.persist(path).map_err(|e| e.error)?;
        } else {
            temp.persist_noclobber(path).map_err(|e| e.error)?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_bytes(path, &fs::read(path)?)
    }
    fn load_bytes(path: &Path, bytes: &[u8]) -> Result<Self> {
        let mut p: Self = serde_json::from_slice(bytes)?;
        ensure!(p.version == 1, "Unsupported project version");
        let base = dunce::canonicalize(path)?
            .parent()
            .context("Project has no parent")?
            .to_path_buf();
        for t in &mut p.tracks {
            if t.path.is_relative() {
                t.path = base.join(&t.path);
            }
        }
        if let Some(c) = &mut p.cover {
            if c.is_relative() {
                *c = base.join(&*c);
            }
        }
        Ok(p)
    }
}
fn parent_dir(p: &Path) -> &Path {
    p.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x = a[si..i]
                .iter()
                .position(|c| *c != b'0')
                .map(|n| &a[si + n..i])
                .unwrap_or(&[]);
            let y = b[sj..j]
                .iter()
                .position(|c| *c != b'0')
                .map(|n| &b[sj + n..j])
                .unwrap_or(&[]);
            let c = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
            if c != Ordering::Equal {
                return c;
            }
        } else {
            let c = a[i].cmp(&b[j]);
            if c != Ordering::Equal {
                return c;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| a.cmp(b))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub language: String,
}
#[derive(Clone, Debug)]
pub struct Probe {
    pub track: Track,
    pub metadata: Metadata,
    pub chapters: Vec<Chapter>,
    pub has_cover: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum ImportEvent {
    Inspecting,
    Extracting { done: usize, total: usize },
    Probing { done: usize, total: usize },
    ComparingCovers { done: usize, total: usize },
}

#[derive(Clone, Copy, Debug)]
pub enum ExportPhase {
    CheckingSources,
    PreparingTrack(usize),
    Encoding,
    CheckingOutput,
    VerifyingAudio,
    Publishing,
}

#[derive(Clone, Debug)]
pub enum ExportEvent {
    Log(String),
    Progress(f32),
    Phase(ExportPhase),
    Activity {
        media_time_us: u64,
        stdout_bytes: u64,
        stderr_bytes: u64,
        stderr_category: &'static str,
        idle_ms: u64,
        exit_code: Option<i32>,
        outcome: &'static str,
    },
}
pub type Cancel = Arc<AtomicBool>;
pub trait MediaBackend: Send + Sync {
    fn probe(&self, path: &Path) -> Result<Probe>;
    fn probe_with_job(&self, path: &Path, job: &Job) -> Result<Probe> {
        job.check()?;
        let result = self.probe(path)?;
        job.check()?;
        Ok(result)
    }
    /// A fingerprint only when the embedded artwork can be read within limits.
    fn cover_fingerprint(&self, _path: &Path, _job: &Job) -> Option<[u8; 32]> {
        None
    }
    fn encoders(&self) -> Result<Vec<String>>;
    fn export(
        &self,
        project: &Project,
        output: &Path,
        cancel: Cancel,
        event: &mut dyn FnMut(ExportEvent),
    ) -> Result<()>;
}
#[derive(Clone, Debug)]
pub struct DesktopFfmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub job: Job,
}
impl Default for DesktopFfmpeg {
    fn default() -> Self {
        let directory = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        Self::discover_in(directory.as_deref())
    }
}
impl DesktopFfmpeg {
    /// Prefer the packaged tools beside the executable, independent of CWD/PATH.
    /// PATH fallback supports developer builds and system-provided tools on Linux.
    pub fn discover_in(directory: Option<&Path>) -> Self {
        fn locate(directory: Option<&Path>, name: &str) -> PathBuf {
            let executable = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            };
            if let Some(candidate) = directory.map(|dir| dir.join(&executable)) {
                if candidate.is_file() {
                    return candidate;
                }
            }
            executable.into()
        }
        Self {
            ffmpeg: locate(directory, "ffmpeg"),
            ffprobe: locate(directory, "ffprobe"),
            job: Job::default(),
        }
    }
}
fn command(path: &Path) -> Command {
    let mut c = Command::new(path);
    c.stdin(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
impl DesktopFfmpeg {
    /// Opens an M4B as a single source while retaining its chapter timeline and tags.
    pub fn open(&self, path: &Path) -> Result<Project> {
        let info = self.probe(path)?;
        let mut p = Project {
            metadata: info.metadata,
            tracks: vec![info.track],
            chapters: info.chapters,
            ..Project::default()
        };
        if info.has_cover {
            p.cover = Some(p.tracks[0].path.clone());
        }
        if p.chapters.is_empty() {
            p.reset_chapters();
        }
        Ok(p)
    }
    /// Small preview extracted off the UI thread; one scaled frame only.
    pub fn extract_cover_preview(&self, input: &Path, output: &Path) -> Result<()> {
        ensure!(!output.exists(), "Cover destination exists");
        let status = jobs::capture(
            command(&self.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-n",
                    "-threads",
                    "1",
                    "-max_alloc",
                    "67108864",
                    "-i",
                ])
                .arg(input)
                .args([
                    "-map",
                    "0:v:0",
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=560:560:force_original_aspect_ratio=decrease",
                    "-threads",
                    "1",
                    "-fs",
                    "16777216",
                ])
                .arg(output),
            &self.job,
            65536,
        )?
        .status;
        ensure!(status.success(), "Cover extraction failed");
        Ok(())
    }
    pub fn extract_cover(&self, input: &Path, output: &Path) -> Result<()> {
        ensure!(!output.exists(), "Cover destination exists");
        let s = jobs::capture(
            command(&self.ffmpeg)
                .args(["-v", "error", "-nostdin", "-n", "-i"])
                .arg(input)
                .args(["-map", "0:v:0", "-frames:v", "1"])
                .arg(output),
            &self.job,
            65536,
        )?
        .status;
        ensure!(s.success(), "Cover extraction failed");
        Ok(())
    }
    fn run(
        &self,
        args: &[std::ffi::OsString],
        cancel: &Cancel,
        event: &mut dyn FnMut(ExportEvent),
        base: f32,
        span: f32,
        duration_ms: u64,
    ) -> Result<()> {
        if cancel.load(AtomicOrdering::Relaxed) {
            bail!("Export cancelled");
        }
        let child = command(&self.ffmpeg)
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "warning",
                "-threads",
                "1",
                "-max_alloc",
                "67108864",
                "-filter_threads",
                "1",
                "-filter_complex_threads",
                "1",
                "-progress",
                "pipe:1",
                "-nostats",
            ])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Start ffmpeg (install it or configure its path)")?;
        jobs::watch_encoder(child, &self.job, cancel, |activity| {
            if let ExportEvent::Activity { media_time_us, .. } = activity {
                if media_time_us > 0 {
                    event(ExportEvent::Progress(
                        base + span
                            * (media_time_us as f32 / (duration_ms.max(1) as f32 * 1000.)).min(1.),
                    ));
                }
            }
            event(activity);
        })
    }
}
fn tag(v: &serde_json::Value, key: &str) -> String {
    v.as_object()
        .and_then(|object| {
            object
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .and_then(|(_, v)| v.as_str())
        })
        .unwrap_or_default()
        .into()
}
fn vorbis_chapters(tags: &serde_json::Value) -> Vec<Chapter> {
    let mut chapters = Vec::new();
    for index in 1..=4096 {
        let key = format!("CHAPTER{index:03}");
        let start = tag(tags, &key);
        if start.is_empty() {
            break;
        }
        let parts: Vec<_> = start.split(':').collect();
        if parts.len() != 3 {
            return Vec::new();
        }
        let parse = || -> Option<u64> {
            let hours = parts[0].parse::<u64>().ok()?;
            let minutes = parts[1].parse::<u64>().ok()?;
            let seconds = parts[2].parse::<f64>().ok()?;
            if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
                return None;
            }
            hours
                .checked_mul(3600000)?
                .checked_add(minutes * 60000)?
                .checked_add((seconds * 1000.).round() as u64)
        };
        let Some(start_ms) = parse() else {
            return Vec::new();
        };
        chapters.push(Chapter {
            start_ms,
            title: tag(tags, &format!("{key}NAME")),
        });
    }
    chapters
}
fn meta_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('=', "\\=")
        .replace(';', "\\;")
        .replace('#', "\\#")
        .replace('\r', "")
        .replace('\n', "\\\n")
}
impl DesktopFfmpeg {
    fn probe_document(&self, path: &Path) -> Result<(PathBuf, serde_json::Value)> {
        let path =
            dunce::canonicalize(path).with_context(|| format!("Read source {}", path.display()))?;
        let o = jobs::capture(
            command(&self.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_format",
                    "-show_streams",
                    "-show_chapters",
                    "-of",
                    "json",
                ])
                .arg(&path),
            &self.job,
            16 * 1024 * 1024,
        )?;
        ensure!(
            o.status.success(),
            "ffprobe: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&o.stdout)?;
        Ok((path, v))
    }
    pub fn audio_streams(&self, path: &Path) -> Result<Vec<AudioStream>> {
        let (_, v) = self.probe_document(path)?;
        Ok(v["streams"]
            .as_array()
            .context("No streams")?
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .map(|s| AudioStream {
                index: s["index"].as_u64().unwrap_or(0) as u32,
                codec: tag(s, "codec_name"),
                sample_rate: tag(s, "sample_rate").parse().unwrap_or(0),
                channels: s["channels"].as_u64().unwrap_or(0) as u32,
                language: tag(&s["tags"], "language"),
            })
            .collect())
    }
    pub fn probe_stream(&self, path: &Path, stream_index: Option<u32>) -> Result<Probe> {
        let (path, v) = self.probe_document(path)?;
        let streams = v["streams"].as_array().context("No streams")?;
        let audio = streams
            .iter()
            .find(|s| {
                s["codec_type"] == "audio"
                    && stream_index.is_none_or(|i| s["index"].as_u64() == Some(u64::from(i)))
            })
            .context("No audio stream")?;
        let duration = audio["duration"]
            .as_str()
            .or(v["format"]["duration"].as_str())
            .context("Unknown duration")?
            .parse::<f64>()?;
        ensure!(
            duration.is_finite() && duration > 0. && duration < 1e9,
            "Invalid duration"
        );
        let tags = &v["format"]["tags"];
        let field = |name| {
            let value = tag(tags, name);
            if value.is_empty() {
                tag(&audio["tags"], name)
            } else {
                value
            }
        };
        let mut chapters: Vec<Chapter> = v["chapters"]
            .as_array()
            .map(|cs| {
                cs.iter()
                    .map(|c| Chapter {
                        start_ms: (c["start_time"]
                            .as_str()
                            .unwrap_or("0")
                            .parse::<f64>()
                            .unwrap_or(0.)
                            * 1000.)
                            .round() as u64,
                        title: tag(&c["tags"], "title"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if chapters.is_empty() {
            chapters = vorbis_chapters(tags);
        }
        Ok(Probe {
            track: Track {
                path,
                duration_ms: (duration * 1000.).round() as u64,
                codec: tag(audio, "codec_name"),
                sample_rate: tag(audio, "sample_rate").parse().unwrap_or(0),
                channels: audio["channels"].as_u64().unwrap_or(0) as u32,
                stream_index: audio["index"].as_u64().map(|v| v as u32),
            },
            metadata: Metadata {
                title: {
                    let album = field("album");
                    if chapters.is_empty() && !album.is_empty() {
                        album
                    } else {
                        field("title")
                    }
                },
                author: {
                    let artist = field("artist");
                    let album_artist = field("album_artist");
                    if chapters.is_empty() && !album_artist.is_empty() {
                        album_artist
                    } else {
                        artist
                    }
                },
                narrator: field("composer"),
                description: {
                    let description = field("description");
                    if description.is_empty() {
                        field("comment")
                    } else {
                        description
                    }
                },
                genre: field("genre"),
                date: field("date"),
                copyright: field("copyright"),
                publisher: field("publisher"),
                book_language: {
                    let language = tag(&audio["tags"], "language");
                    if language.is_empty() || language == "und" {
                        field("language")
                    } else {
                        language
                    }
                },
            },
            chapters,
            has_cover: streams
                .iter()
                .any(|s| s["disposition"]["attached_pic"] == 1),
        })
    }
}
impl MediaBackend for DesktopFfmpeg {
    fn probe_with_job(&self, path: &Path, job: &Job) -> Result<Probe> {
        let mut backend = self.clone();
        backend.job = job.clone();
        backend.probe(path)
    }
    fn probe(&self, path: &Path) -> Result<Probe> {
        self.probe_stream(path, None)
    }
    fn cover_fingerprint(&self, path: &Path, job: &Job) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let mut backend = self.clone();
        backend.job = job.clone();
        let (path, document) = backend.probe_document(path).ok()?;
        let index = document["streams"]
            .as_array()?
            .iter()
            .find(|stream| stream["disposition"]["attached_pic"] == 1)?["index"]
            .as_u64()?;
        // Bundled FFmpeg provides the image2 muxer. Treat the pipe as a single output.
        let output = jobs::capture(
            command(&self.ffmpeg)
                .args(["-v", "error", "-max_alloc", "67108864", "-i"])
                .arg(path)
                .args([
                    "-map",
                    &format!("0:{index}"),
                    "-c",
                    "copy",
                    "-frames:v",
                    "1",
                    "-f",
                    "image2",
                    "-update",
                    "1",
                    "pipe:1",
                ]),
            job,
            8 * 1024 * 1024,
        )
        .ok()?;
        (output.status.success() && !output.stdout.is_empty())
            .then(|| Sha256::digest(output.stdout).into())
    }
    fn encoders(&self) -> Result<Vec<String>> {
        let o = jobs::capture(
            command(&self.ffmpeg).args(["-hide_banner", "-encoders"]),
            &self.job,
            4 * 1024 * 1024,
        )?;
        ensure!(o.status.success(), "Cannot query encoders");
        Ok(String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let mut s = l.split_whitespace();
                let flags = s.next()?;
                let name = s.next()?;
                if flags.starts_with('A') && name != "=" {
                    Some(name.into())
                } else {
                    None
                }
            })
            .collect())
    }
    fn export(
        &self,
        project: &Project,
        output: &Path,
        cancel: Cancel,
        event: &mut dyn FnMut(ExportEvent),
    ) -> Result<()> {
        let mut backend = self.clone();
        backend.job.cancel = cancel.clone();
        if backend.job.deadline.is_none() {
            backend.job.deadline = Some(std::time::Instant::now() + Duration::from_secs(86400));
        }
        backend.export_impl(project, output, cancel, event)
    }
}
impl DesktopFfmpeg {
    fn export_impl(
        &self,
        p: &Project,
        output: &Path,
        cancel: Cancel,
        event: &mut dyn FnMut(ExportEvent),
    ) -> Result<()> {
        self.job.check()?;
        p.validate()?;
        ensure!(
            !output.exists(),
            "Output already exists; refusing overwrite"
        );
        ensure!(
            output
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(p.settings.format.extension())),
            "Output extension must match selected format"
        );
        let remux = p.can_remux();
        if !remux {
            ensure!(
                self.encoders()?
                    .iter()
                    .any(|e| e == p.settings.format.encoder()),
                "This FFmpeg lacks the selected encoder"
            );
        }
        let parent = dunce::canonicalize(parent_dir(output))?;
        let output = parent.join(output.file_name().context("Invalid output")?);
        preflight(p, &parent)?;
        // Same filesystem as destination: hard-link publishes atomically with no replace.
        let stage = tempfile::Builder::new()
            .prefix(".m4b-editor-")
            .tempdir_in(&parent)?;
        event(ExportEvent::Phase(ExportPhase::CheckingSources));
        let mut total = 0u64;
        for t in &p.tracks {
            let fresh = self.probe_stream(&t.path, t.stream_index)?.track;
            ensure!(
                fresh.duration_ms.abs_diff(t.duration_ms) <= 100,
                "Source duration changed; reimport {}",
                t.path.display()
            );
            ensure!(
                !remux || fresh.codec == "aac",
                "Source codec changed; reimport source"
            );
            total = total
                .checked_add(fresh.duration_ms)
                .context("Duration overflow")?;
        }
        let mut export_chapters = p.chapters.clone();
        if !remux {
            let mut list = String::new();
            let mut decoded_durations = Vec::new();
            let mut elapsed = 0u64;
            for (i, t) in p.tracks.iter().enumerate() {
                event(ExportEvent::Phase(ExportPhase::PreparingTrack(i + 1)));
                event(ExportEvent::Log(format!(
                    "Preparing track {}: {}",
                    i + 1,
                    t.path.display()
                )));
                let wav = stage.path().join(format!("track{i}.wav"));
                let args = vec![
                    "-n".into(),
                    "-i".into(),
                    t.path.as_os_str().into(),
                    "-map".into(),
                    t.stream_index
                        .map(|i| format!("0:{i}"))
                        .unwrap_or_else(|| "0:a:0".into())
                        .into(),
                    "-vn".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-ac".into(),
                    "2".into(),
                    "-c:a".into(),
                    "pcm_s16le".into(),
                    "-rf64".into(),
                    "auto".into(),
                    wav.as_os_str().into(),
                ];
                self.run(
                    &args,
                    &cancel,
                    event,
                    0.45 * elapsed as f32 / total as f32,
                    0.45 * t.duration_ms as f32 / total as f32,
                    t.duration_ms,
                )?;
                decoded_durations.push(self.probe(&wav)?.track.duration_ms);
                list.push_str(&format!("file 'track{i}.wav'\n"));
                elapsed += t.duration_ms;
            }
            // Compressed stream durations can include encoder delay/padding. Use the
            // decoded samples for the output timeline, retaining chapter offsets
            // within their source track rather than accumulating padding as gaps.
            export_chapters.clear();
            for c in &p.chapters {
                let (mut source_start, mut decoded_start) = (0u64, 0u64);
                for (track, decoded) in p.tracks.iter().zip(&decoded_durations) {
                    if c.start_ms < source_start + track.duration_ms {
                        let offset = c.start_ms - source_start;
                        ensure!(
                            offset < *decoded,
                            "Chapter '{}' lies in discarded source padding; adjust its start",
                            c.title
                        );
                        export_chapters.push(Chapter {
                            start_ms: decoded_start + offset,
                            title: c.title.clone(),
                        });
                        break;
                    }
                    source_start += track.duration_ms;
                    decoded_start += decoded;
                }
            }
            total = decoded_durations.iter().sum();
            if total != p.duration_ms() {
                event(ExportEvent::Log(format!(
                    "Chapter timeline aligned to decoded audio: {total} ms (source headers: {} ms)",
                    p.duration_ms()
                )));
            }
            fs::write(stage.path().join("concat.txt"), list)?;
        }
        let mut meta = String::from(";FFMETADATA1\n");
        for (key, val) in [
            ("title", &p.metadata.title),
            ("artist", &p.metadata.author),
            ("album_artist", &p.metadata.author),
            ("composer", &p.metadata.narrator),
            ("description", &p.metadata.description),
            ("genre", &p.metadata.genre),
            ("date", &p.metadata.date),
            ("copyright", &p.metadata.copyright),
            ("publisher", &p.metadata.publisher),
            ("language", &p.metadata.book_language),
            ("album", &p.metadata.title),
        ] {
            meta.push_str(&format!("{key}={}\n", meta_escape(val)));
        }
        if p.settings.format == ExportFormat::Flac {
            for (i, c) in export_chapters.iter().enumerate() {
                let ms = c.start_ms;
                meta.push_str(&format!(
                    "CHAPTER{:03}={:02}:{:02}:{:02}.{:03}\nCHAPTER{:03}NAME={}\n",
                    i + 1,
                    ms / 3600000,
                    ms / 60000 % 60,
                    ms / 1000 % 60,
                    ms % 1000,
                    i + 1,
                    meta_escape(&c.title)
                ));
            }
        }
        for (i, c) in export_chapters.iter().enumerate() {
            let end = export_chapters
                .get(i + 1)
                .map(|c| c.start_ms)
                .unwrap_or(total);
            meta.push_str(&format!(
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={end}\ntitle={}\n",
                c.start_ms,
                meta_escape(&c.title)
            ));
        }
        let metadata = stage.path().join("metadata.txt");
        fs::write(&metadata, meta)?;
        let temp = stage
            .path()
            .join(format!("output.{}", p.settings.format.extension()));
        let mut args: Vec<std::ffi::OsString> = if remux {
            vec![
                "-n".into(),
                "-i".into(),
                p.tracks[0].path.as_os_str().into(),
            ]
        } else {
            vec![
                "-n".into(),
                "-f".into(),
                "concat".into(),
                "-safe".into(),
                "1".into(),
                "-i".into(),
                stage.path().join("concat.txt").into_os_string(),
            ]
        };
        args.extend([
            "-f".into(),
            "ffmetadata".into(),
            "-i".into(),
            metadata.into_os_string(),
        ]);
        if let Some(cover) = &p.cover {
            args.extend(["-i".into(), cover.as_os_str().into()]);
        }
        args.extend([
            "-map".into(),
            if remux {
                p.tracks[0]
                    .stream_index
                    .map(|i| format!("0:{i}"))
                    .unwrap_or_else(|| "0:a:0".into())
            } else {
                "0:a:0".into()
            }
            .into(),
            "-map_metadata".into(),
            "1".into(),
            "-map_chapters".into(),
            "1".into(),
        ]);
        args.extend([
            "-c:a".into(),
            if remux {
                "copy"
            } else {
                p.settings.format.encoder()
            }
            .into(),
        ]);
        if !remux && p.settings.format != ExportFormat::Flac {
            args.extend([
                "-b:a".into(),
                format!("{}k", p.settings.bitrate_kbps).into(),
            ]);
        }
        if p.cover.is_some() {
            args.extend([
                "-map".into(),
                "2:v:0".into(),
                "-c:v".into(),
                "mjpeg".into(),
                "-disposition:v:0".into(),
                "attached_pic".into(),
            ]);
        }
        args.extend([
            "-threads:a".into(),
            "1".into(),
            "-threads:v".into(),
            "1".into(),
        ]);
        if !p.metadata.book_language.is_empty() {
            args.extend([
                "-metadata:s:a:0".into(),
                format!("language={}", p.metadata.book_language).into(),
            ]);
        }
        if p.settings.format == ExportFormat::M4b {
            args.extend(["-f".into(), "ipod".into()]);
        } else {
            args.extend(["-f".into(), p.settings.format.extension().into()]);
        }
        args.push(temp.as_os_str().into());
        event(ExportEvent::Log(
            if remux {
                "Lossless AAC remux and chapters"
            } else {
                match p.settings.format {
                    ExportFormat::M4b => "Encoding AAC and chapters",
                    ExportFormat::Mp3 => "Encoding MP3 and chapters",
                    ExportFormat::Flac => "Encoding FLAC and chapters",
                }
            }
            .into(),
        ));
        event(ExportEvent::Phase(ExportPhase::Encoding));
        let base = if remux { 0. } else { 0.45 };
        self.run(&args, &cancel, event, base, 0.95 - base, total)?;
        event(ExportEvent::Phase(ExportPhase::CheckingOutput));
        if p.settings.format == ExportFormat::M4b {
            mp4::publisher(&temp, &p.metadata.publisher)?;
        }
        let check = self.probe(&temp)?;
        ensure!(
            check.track.codec == p.settings.format.codec(),
            "Output codec validation failed"
        );
        ensure!(
            check.track.duration_ms.abs_diff(total) < 500,
            "Output duration validation failed"
        );
        for (name, actual, expected) in [
            ("title", &check.metadata.title, &p.metadata.title),
            ("author", &check.metadata.author, &p.metadata.author),
            ("narrator", &check.metadata.narrator, &p.metadata.narrator),
            (
                "description",
                &check.metadata.description,
                &p.metadata.description,
            ),
            ("genre", &check.metadata.genre, &p.metadata.genre),
            ("date", &check.metadata.date, &p.metadata.date),
            (
                "copyright",
                &check.metadata.copyright,
                &p.metadata.copyright,
            ),
            (
                "publisher",
                &check.metadata.publisher,
                &p.metadata.publisher,
            ),
        ] {
            ensure!(
                actual == expected,
                "Output {} metadata validation failed: {name}",
                p.settings.format.extension()
            );
        }
        ensure!(
            p.metadata.book_language.is_empty()
                || check.metadata.book_language == p.metadata.book_language,
            "Output book language validation failed"
        );
        ensure!(
            (check.chapters.len() == export_chapters.len()
                && check.chapters.iter().zip(&export_chapters).all(|(a, b)| a
                    .start_ms
                    .abs_diff(b.start_ms)
                    <= 2
                    && a.title == b.title)),
            "Output chapter validation failed"
        );
        ensure!(
            p.cover.is_none() || check.has_cover,
            "Output cover validation failed"
        );
        // Decode the entire encoded stream to catch corrupt/truncated packets before publishing.
        let args = vec![
            "-xerror".into(),
            "-i".into(),
            temp.as_os_str().into(),
            "-map".into(),
            "0:a:0".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        event(ExportEvent::Phase(ExportPhase::VerifyingAudio));
        self.run(&args, &cancel, event, 0.95, 0.04, total)?;
        event(ExportEvent::Phase(ExportPhase::Publishing));
        ensure!(!cancel.load(AtomicOrdering::Relaxed), "Export cancelled");
        // FlushFileBuffers on Windows requires a write-capable handle.
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp)?
            .sync_all()?;
        self.job.check()?;
        fs::hard_link(&temp,
                    &output).context("Atomic no-overwrite publication failed (destination filesystem must support hard links)")?;
        event(ExportEvent::Progress(1.));
        event(ExportEvent::Log(format!("Saved {}", output.display())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_order() {
        let mut a = vec!["track10", "track2", "track1"];
        a.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(a, vec!["track1", "track2", "track10"]);
        assert_eq!(natural_cmp("a01", "a1x"), Ordering::Less);
        assert_eq!(
            natural_cmp("a999999999999999999999", "a10"),
            Ordering::Greater
        );
    }
    #[test]
    fn persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        let mut p = Project::default();
        p.metadata.title = "Écoute\n=;#".into();
        p.save(&path).unwrap();
        assert_eq!(
            Project::load(&path).unwrap().metadata.title,
            p.metadata.title
        );
        p.metadata.title = "Updated".into();
        assert!(p.save_new(&path).is_err());
        assert_ne!(Project::load(&path).unwrap().metadata.title, "Updated");
        p.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap().metadata.title, "Updated");
    }
    #[test]
    fn invalid_chapters() {
        let mut p = Project::default();
        assert!(p.validate().is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.wav");
        fs::write(&path, b"source bytes").unwrap();
        p.tracks.push(Track {
            path: path.clone(),
            duration_ms: 2000,
            codec: "pcm_s16le".into(),
            sample_rate: 48000,
            channels: 2,
            stream_index: None,
        });
        p.reset_chapters();
        assert!(p.validate().is_ok());
        assert!(p.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"source bytes");
        p.chapters[0].start_ms = 1;
        assert!(p.validate().is_err());
        p.chapters[0].start_ms = 0;
        p.chapters.push(Chapter {
            start_ms: 2000,
            title: "outside".into(),
        });
        assert!(p.validate().is_err());
        p.chapters[1].start_ms = 0;
        assert!(p.validate().is_err());
        p.chapters[1].start_ms = 1000;
        assert!(p.validate().is_ok());
    }
}

#[cfg(test)]
mod alpha_regressions {
    use super::*;
    #[test]
    fn new_default_and_legacy_bitrate_assets_roundtrip() {
        assert_eq!(Project::default().settings.bitrate_kbps, 160);
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::named("Legacy").unwrap();
        project.settings.bitrate_kbps = 96;
        project.cover = Some(PathBuf::from("book.assets/cover.png"));
        let legacy = dir.path().join("book.json");
        project.save(&legacy).unwrap();
        let loaded = Project::load(&legacy).unwrap();
        assert_eq!(loaded.settings.bitrate_kbps, 96);
        assert_eq!(loaded.cover, Some(dir.path().join("book.assets/cover.png")));
        let modern = dir.path().join("book.m4bproj");
        loaded.save(&modern).unwrap();
        assert_eq!(Project::load(&modern).unwrap().settings.bitrate_kbps, 96);
        assert!(legacy.exists());
    }
}
