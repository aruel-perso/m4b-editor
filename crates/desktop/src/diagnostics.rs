//! Local diagnostics accept only typed events. Error strings, paths and metadata
//! belong in the in-memory journal and never cross this persistence boundary.
use m4b_editor_core::{ExportEvent, ExportPhase, ImportEvent, Project};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LIMIT: u64 = 256 * 1024;
static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
static JOB_ID: AtomicU64 = AtomicU64::new(1);

struct Log {
    directory: PathBuf,
    limit: u64,
}

impl Log {
    fn path(&self, index: usize) -> PathBuf {
        self.directory.join(format!("diagnostic-{index}.log"))
    }

    fn lock(&self) -> anyhow::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("rotation.lock"))?;
        file.try_lock()?;
        Ok(file)
    }

    fn record(&self, fields: impl std::fmt::Display) -> anyhow::Result<()> {
        let _lock = self.lock()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let entry = format!(
            "utc_unix_ms={} pid={} {}\n",
            now.as_millis(),
            std::process::id(),
            fields
        );
        anyhow::ensure!(
            entry.len() as u64 <= self.limit,
            "Diagnostic record exceeds log capacity"
        );
        let current = self.path(0);
        if current.metadata().map_or(0, |m| m.len()) + entry.len() as u64 > self.limit {
            let oldest = self.path(2);
            if oldest.exists() {
                fs::remove_file(oldest)?;
            }
            for index in (0..2).rev() {
                let file = self.path(index);
                if file.exists() {
                    fs::rename(file, self.path(index + 1))?;
                }
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(current)?;
        file.write_all(entry.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    fn export(&self, destination: &Path) -> anyhow::Result<()> {
        let _lock = self.lock()?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut copy = tempfile::NamedTempFile::new_in(parent)?;
        for index in (0..3).rev() {
            match File::open(self.path(index)) {
                Ok(file) => {
                    use std::io::Read;
                    io::copy(&mut file.take(self.limit), copy.as_file_mut())?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        copy.persist_noclobber(destination)?;
        Ok(())
    }
}

fn record(fields: impl std::fmt::Display) {
    if let Some(log) = LOG.get().and_then(|log| log.lock().ok()) {
        let _ = log.record(fields);
    }
}

fn log_directory(platform: &str, state: &Path, home: Option<PathBuf>) -> PathBuf {
    if platform == "macos" {
        if let Some(home) = home.filter(|path| path.is_absolute()) {
            return home.join("Library/Logs/m4b-editor/log");
        }
    }
    state.join("log")
}

fn directory() -> PathBuf {
    log_directory(
        std::env::consts::OS,
        &super::state_directory(),
        if cfg!(test) {
            None
        } else {
            std::env::var_os("HOME").map(PathBuf::from)
        },
    )
}

fn start_log(directory: PathBuf, old: &Path) -> anyhow::Result<Log> {
    if !directory.exists() && old.is_dir() {
        if let Some(parent) = directory.parent() {
            fs::create_dir_all(parent)?;
            let _ = fs::rename(old, &directory);
        }
    }
    fs::create_dir_all(&directory)?;
    let log = Log {
        directory,
        limit: LIMIT,
    };
    let _ = log.record(format_args!(
        "application_start version={} os={} arch={} profile={}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    ));
    Ok(log)
}

pub fn init() {
    static INIT: Mutex<()> = Mutex::new(());
    let Ok(_guard) = INIT.lock() else {
        return;
    };
    if LOG.get().is_some() {
        return;
    }
    let Ok(log) = start_log(directory(), &super::state_directory().join("logs")) else {
        return;
    };
    if LOG.set(Mutex::new(log)).is_err() {
        return;
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(log) = LOG.get().and_then(|log| log.try_lock().ok()) {
            let _ = log.record("rust_panic payload=redacted");
        }
        previous_hook(info);
    }));
}

pub fn shutdown() {
    record("application_exit");
}

pub fn operation_error() {
    record("severity=error event=desktop_operation_failed");
}

pub fn project_saved() {
    record("event=project_saved");
}

pub fn project_loaded() {
    record("event=project_loaded");
}

pub fn export(destination: &Path) -> anyhow::Result<()> {
    LOG.get()
        .and_then(|log| log.lock().ok())
        .ok_or_else(|| anyhow::anyhow!("Diagnostics unavailable"))?
        .export(destination)
}

pub fn open_folder() -> io::Result<()> {
    let directory = directory();
    fs::create_dir_all(&directory)?;
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(directory).spawn()?;
    Ok(())
}

pub struct Job {
    id: u64,
    started: Instant,
    next_activity: Instant,
    active: bool,
}

impl Job {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            id: JOB_ID.fetch_add(1, Ordering::Relaxed),
            started: now,
            next_activity: now,
            active: true,
        }
    }

    fn record(&self, fields: impl std::fmt::Display) {
        record(format_args!(
            "job={} elapsed_ms={} {}",
            self.id,
            self.started.elapsed().as_millis(),
            fields
        ));
    }

    pub fn import(inputs: usize, password_supplied: bool) -> Self {
        let job = Self::new();
        job.record(format_args!(
            "import_start inputs={inputs} password_supplied={password_supplied}"
        ));
        job
    }

    pub fn export(project: &Project, replacement: bool) -> Self {
        let job = Self::new();
        job.record(format_args!(
            "export_start format={} bitrate_kbps={} remux={} tracks={} duration_ms={} cover={} replacement={}",
            project.settings.format.extension(), project.settings.bitrate_kbps, project.can_remux(),
            project.tracks.len(), project.duration_ms(), project.cover.is_some(), replacement
        ));
        job
    }

    pub fn import_event(&mut self, event: ImportEvent) {
        match event {
            ImportEvent::Inspecting => self.record("phase=inspecting"),
            ImportEvent::Extracting { done, total }
            | ImportEvent::Probing { done, total }
            | ImportEvent::ComparingCovers { done, total }
                if done == 0 || done == total || Instant::now() >= self.next_activity =>
            {
                let phase = match event {
                    ImportEvent::Extracting { .. } => "extracting",
                    ImportEvent::Probing { .. } => "probing",
                    ImportEvent::ComparingCovers { .. } => "comparing_covers",
                    ImportEvent::Inspecting => unreachable!(),
                };
                self.record(format_args!("phase={phase} done={done} total={total}"));
                self.next_activity = Instant::now() + Duration::from_secs(1);
            }
            _ => {}
        }
    }

    pub fn export_event(&mut self, event: &ExportEvent, progress: f32) {
        match *event {
            ExportEvent::Phase(phase) => {
                match phase {
                    ExportPhase::CheckingSources => self.record("phase=checking_sources"),
                    ExportPhase::PreparingTrack(track) => {
                        self.record(format_args!("phase=preparing_track track={track}"))
                    }
                    ExportPhase::Encoding => self.record("phase=encoding"),
                    ExportPhase::CheckingOutput => self.record("phase=checking_output"),
                    ExportPhase::VerifyingAudio => self.record("phase=verifying_audio"),
                    ExportPhase::Publishing => self.record("phase=publishing"),
                }
                self.next_activity = Instant::now();
            }
            ExportEvent::Activity {
                media_time_us,
                stdout_bytes,
                stderr_bytes,
                stderr_category,
                idle_ms,
                exit_code,
                outcome,
            } if outcome != "running" || Instant::now() >= self.next_activity => {
                let severity = match (outcome, stderr_category) {
                    ("failed", _) => "error",
                    ("running" | "completed", "none") => "info",
                    _ => "warning",
                };
                self.record(format_args!(
                    "severity={severity} encoder={outcome} progress={progress:.4} media_time_us={media_time_us} idle_ms={idle_ms} stdout_bytes={stdout_bytes} stderr_bytes={stderr_bytes} stderr_category={stderr_category} exit_code={exit_code:?}"
                ));
                self.next_activity = Instant::now() + Duration::from_secs(5);
            }
            _ => {}
        }
    }

    pub fn finish(mut self, result: &anyhow::Result<()>, cancelled: bool) {
        let outcome = match result {
            Ok(()) => "completed",
            Err(_) if cancelled => "cancelled",
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .and_then(io::Error::raw_os_error)
                        .is_some_and(|code| code == if cfg!(windows) { 112 } else { 28 })
                }) =>
            {
                "disk_full"
            }
            Err(error)
                if error.chain().any(|cause| {
                    let text = cause.to_string();
                    text == "Operation deadline exceeded" || text == "Encoder deadline exceeded"
                }) =>
            {
                "deadline"
            }
            Err(_) => "failed",
        };
        self.record(format_args!("job_end outcome={outcome}"));
        self.active = false;
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.active {
            self.record("job_end outcome=interrupted");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_paths_create_and_rotate_startup_logs() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("LocalAppData");
        let xdg = temp.path().join("xdg");
        let home = temp.path().join("home");
        for (platform, with_xdg, expected) in [
            ("windows", true, local.join("m4b-editor/log")),
            ("macos", true, home.join("Library/Logs/m4b-editor/log")),
            ("linux", true, xdg.join("m4b-editor/log")),
            ("linux", false, home.join(".local/state/m4b-editor/log")),
        ] {
            let env = |key: &str| match key {
                "LOCALAPPDATA" => Some(local.clone().into_os_string()),
                "HOME" => Some(home.clone().into_os_string()),
                "XDG_STATE_HOME" if with_xdg => Some(xdg.clone().into_os_string()),
                _ => None,
            };
            let state = crate::configured_state_root(platform, env)
                .unwrap()
                .join("m4b-editor");
            let directory = log_directory(platform, &state, env("HOME").map(PathBuf::from));
            assert_eq!(directory, expected);
            let log = start_log(directory.clone(), &state.join("logs")).unwrap();
            assert!(fs::read_to_string(log.path(0))
                .unwrap()
                .contains("application_start"));
            fs::write(log.path(0), vec![b'x'; LIMIT as usize]).unwrap();
            let reopened = start_log(directory, &state.join("logs")).unwrap();
            assert_eq!(reopened.path(1).metadata().unwrap().len(), LIMIT);
            assert!(fs::read_to_string(reopened.path(0))
                .unwrap()
                .contains("application_start"));
        }
    }

    #[test]
    fn bounded_rotation_and_export_in_time_order() {
        let temp = tempfile::tempdir().unwrap();
        let log = Log {
            directory: temp.path().into(),
            limit: 64,
        };
        for n in 0..10 {
            log.record(format_args!("sequence={n}")).unwrap();
        }
        let output = temp.path().join("copy.log");
        log.export(&output).unwrap();
        let exported = fs::read_to_string(&output).unwrap();
        let lines: Vec<_> = exported.lines().collect();
        assert!(lines.len() <= 6);
        assert!(lines.last().unwrap().ends_with("sequence=9"));
        assert!(lines
            .windows(2)
            .all(|pair| pair[0].split('=').next_back() < pair[1].split('=').next_back()));
        assert!(log.export(&output).is_err());
        assert!((0..3).all(|i| log.path(i).metadata().unwrap().len() <= 64));
    }

    #[cfg(unix)]
    #[test]
    fn failed_export_logs_encoder_category_and_exit_without_stderr_text() {
        use m4b_editor_core::{DesktopFfmpeg, MediaBackend};
        use std::{
            os::unix::fs::PermissionsExt,
            sync::{atomic::AtomicBool, Arc},
        };

        init();
        let dir = tempfile::tempdir().unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tone440.wav");
        let mut backend = DesktopFfmpeg::default();
        let mut project = Project::default();
        project.import(&backend, vec![fixture]).unwrap();
        let fake = dir.path().join("ffmpeg");
        fs::write(
            &fake,
            r#"#!/bin/sh
if [ "$2" = -encoders ]; then
    printf 'A aac\n'
    exit 0
fi
printf 'out_time_us=630000\n'
printf 'Permission denied: C:\\Users\\private\\raw-stderr-private.mp3\n' >&2
exit 23
"#,
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        backend.ffmpeg = fake;
        let mut log_job = Job::export(&project, false);
        let output = dir.path().join("failed.m4b");
        let result = backend.export(
            &project,
            &output,
            Arc::new(AtomicBool::new(false)),
            &mut |event| log_job.export_event(&event, 0.),
        );
        assert!(result
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("raw-stderr-private.mp3"));
        assert!(!output.exists());
        log_job.finish(&result, false);
        let log = fs::read_to_string(directory().join("diagnostic-0.log")).unwrap();
        assert!(log.contains("encoder=failed"));
        assert!(log.contains("stderr_category=permission_denied exit_code=Some(23)"));
        assert!(!log.contains("raw-stderr-private.mp3"));
    }

    #[test]
    fn job_logs_allowlisted_failure_information_only() {
        init();
        let mut job = Job::import(2, true);
        job.import_event(ImportEvent::Probing { done: 1, total: 2 });
        job.export_event(
            &ExportEvent::Log("C:\\Users\\private\\secret.mp3".into()),
            0.,
        );
        job.finish(
            &Err(anyhow::anyhow!("https://private?token=super-secret")),
            false,
        );
        let log = fs::read_to_string(directory().join("diagnostic-0.log")).unwrap();
        assert!(log.contains("phase=probing done=1 total=2"));
        assert!(log.contains("job_end outcome=failed"));
        assert!(!log.contains("private"));
        assert!(!log.contains("super-secret"));
    }
}
