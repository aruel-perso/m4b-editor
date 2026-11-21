//! Shared cancellation, deadlines and bounded subprocess capture.
use crate::*;
use std::{io::Read, process::Output, time::Instant};
#[derive(Clone, Debug)]
pub struct Job {
    pub cancel: Cancel,
    pub deadline: Option<Instant>,
}
impl Default for Job {
    fn default() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            deadline: None,
        }
    }
}
impl Job {
    pub fn new(cancel: Cancel, timeout: Duration) -> Self {
        Self {
            cancel,
            deadline: Some(Instant::now() + timeout),
        }
    }
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.cancel.load(AtomicOrdering::Relaxed),
            "Operation cancelled"
        );
        ensure!(
            self.deadline.is_none_or(|d| Instant::now() < d),
            "Operation deadline exceeded"
        );
        Ok(())
    }
}
pub(crate) fn capture(command: &mut Command, job: &Job, limit: usize) -> Result<Output> {
    let job = Job {
        cancel: job.cancel.clone(),
        deadline: Some(
            job.deadline
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(120))
                .min(Instant::now() + Duration::from_secs(120)),
        ),
    };
    job.check()?;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Start bounded media process")?;
    fn drain(
        mut pipe: impl Read + Send + 'static,
        limit: usize,
    ) -> std::thread::JoinHandle<std::io::Result<(Vec<u8>, bool)>> {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buf = [0; 8192];
            let mut overflow = false;
            loop {
                let n = pipe.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let keep = n.min(limit.saturating_sub(kept.len()));
                kept.extend_from_slice(&buf[..keep]);
                overflow |= keep < n;
            }
            Ok((kept, overflow))
        })
    }
    let out = drain(child.stdout.take().unwrap(), limit);
    let err = drain(child.stderr.take().unwrap(), 65536);
    let result = loop {
        if let Err(e) = job.check() {
            let _ = child.kill();
            let _ = child.wait();
            break Err(e);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.into());
            }
        }
    };
    if !readers_finished(&out, &err) {
        result?;
        bail!("Media process pipes did not close after exit");
    }
    let (stdout, overflow) = out
        .join()
        .map_err(|_| anyhow::anyhow!("Output reader failed"))??;
    let (stderr, _) = err
        .join()
        .map_err(|_| anyhow::anyhow!("Error reader failed"))??;
    let status = result?;
    ensure!(!overflow, "Process output exceeds limit");
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
/// Includes both decoded stereo PCM staging and the final file, plus reserve.
pub fn required_export_bytes(p: &Project) -> u64 {
    if p.can_remux() {
        return p.tracks.iter().map(|t| &t.path).chain(p.cover.iter()).fold(
            64 * 1024 * 1024,
            |sum, path| {
                sum.saturating_add(
                    fs::metadata(path)
                        .map(|m| m.len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(2),
                )
            },
        );
    }
    p.duration_ms()
        .saturating_mul(192 + 192)
        .saturating_add(64 * 1024 * 1024)
}
pub fn preflight(p: &Project, directory: &Path) -> Result<()> {
    let available = available_space(directory)?;
    ensure!(
        available >= required_export_bytes(p),
        "Insufficient disk space: require {} bytes, available {} bytes",
        required_export_bytes(p),
        available
    );
    Ok(())
}
#[cfg(unix)]
pub fn available_space(path: &Path) -> Result<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    ensure!(
        unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0,
        "Cannot inspect free disk space: {}",
        std::io::Error::last_os_error()
    );
    let stat = unsafe { stat.assume_init() };
    Ok((u128::from(stat.f_bavail) * u128::from(stat.f_frsize)).min(u128::from(u64::MAX)) as u64)
}
#[cfg(windows)]
pub fn available_space(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    ensure!(
        unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0,
        "Cannot inspect free disk space"
    );
    Ok(available)
}
pub(crate) fn copy_file(source: &Path, destination: &Path, job: &Job) -> Result<()> {
    use std::io::Read;
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let mut buffer = [0; 65536];
    loop {
        job.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n])?;
    }
    output.sync_all()?;
    Ok(())
}
#[derive(Default)]
struct EncoderPipes {
    media_time_us: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    stderr_tail: Vec<u8>,
    last_activity: Option<Instant>,
}

fn diagnostic_category(stderr: &str) -> &'static str {
    // Never carry arbitrary subprocess text into persistent diagnostics.
    let text = stderr.to_ascii_lowercase();
    if text.contains("no space left on device") || text.contains("disk full") {
        "disk_full"
    } else if text.contains("permission denied") {
        "permission_denied"
    } else if text.contains("invalid data found") {
        "invalid_input"
    } else if text.contains("input/output error") {
        "io_error"
    } else if text.is_empty() {
        "none"
    } else {
        "redacted"
    }
}

pub(crate) fn watch_encoder(
    mut child: std::process::Child,
    job: &Job,
    cancel: &Cancel,
    mut event: impl FnMut(ExportEvent),
) -> Result<()> {
    use std::{
        io::{BufRead, BufReader},
        sync::Mutex,
    };
    const TAIL_LIMIT: usize = 16_384;
    let state = Arc::new(Mutex::new(EncoderPipes::default()));
    let out_state = state.clone();
    let stdout = child.stdout.take().expect("piped encoder stdout");
    let stderr = child.stderr.take().expect("piped encoder stderr");
    let out = std::thread::spawn(move || -> std::io::Result<()> {
        let mut input = BufReader::new(stdout);
        let mut data = Vec::new();
        loop {
            data.clear();
            let n = input.by_ref().take(4096).read_until(b'\n', &mut data)?;
            if n == 0 {
                return Ok(());
            }
            let mut state = out_state.lock().unwrap();
            state.stdout_bytes += n as u64;
            state.last_activity = Some(Instant::now());
            if let Some(value) = std::str::from_utf8(&data)
                .ok()
                .and_then(|line| line.trim_end().strip_prefix("out_time_us="))
                .and_then(|value| value.parse::<u64>().ok())
            {
                state.media_time_us = state.media_time_us.max(value);
            }
        }
    });
    let err_state = state.clone();
    let err = std::thread::spawn(move || -> std::io::Result<()> {
        let mut input = stderr;
        let mut buf = [0; 8192];
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            let mut state = err_state.lock().unwrap();
            state.stderr_bytes += n as u64;
            state.last_activity = Some(Instant::now());
            let discard = (state.stderr_tail.len() + n).saturating_sub(TAIL_LIMIT);
            state.stderr_tail.drain(..discard);
            state.stderr_tail.extend_from_slice(&buf[..n]);
        }
    });
    let start = Instant::now();
    let snapshot = |outcome, exit_code| {
        let state = state.lock().unwrap();
        ExportEvent::Activity {
            media_time_us: state.media_time_us,
            stdout_bytes: state.stdout_bytes,
            stderr_bytes: state.stderr_bytes,
            stderr_category: diagnostic_category(&String::from_utf8_lossy(&state.stderr_tail)),
            idle_ms: state.last_activity.unwrap_or(start).elapsed().as_millis() as u64,
            exit_code,
            outcome,
        }
    };
    let mut next_report = start;
    let (status, stop) = loop {
        let check = if cancel.load(AtomicOrdering::Relaxed) {
            Err(anyhow::anyhow!("Export cancelled"))
        } else {
            job.check()
        };
        if let Err(error) = check {
            let _ = child.kill();
            break (child.wait().ok(), Some(error));
        }
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                break (child.wait().ok(), Some(error.into()));
            }
        }
        if Instant::now() >= next_report {
            event(snapshot("running", None));
            next_report = Instant::now() + Duration::from_millis(250);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A descendant may have inherited a pipe. Never wait indefinitely for EOF.
    let drained = readers_finished(&out, &err);
    if drained {
        let out = out
            .join()
            .map_err(|_| anyhow::anyhow!("Encoder progress reader failed"));
        let err = err
            .join()
            .map_err(|_| anyhow::anyhow!("Encoder error reader failed"));
        if stop.is_none()
            && (out.and_then(|r| Ok(r?)).is_err() || err.and_then(|r| Ok(r?)).is_err())
        {
            event(snapshot("pipe_error", status.and_then(|s| s.code())));
            bail!("Encoder pipe read failed");
        }
    }
    let outcome = if cancel.load(AtomicOrdering::Relaxed) {
        "cancelled"
    } else if job
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        "deadline"
    } else if stop.is_some() || status.is_none_or(|s| !s.success()) {
        "failed"
    } else if !drained {
        "pipe_timeout"
    } else {
        "completed"
    };
    event(snapshot(outcome, status.and_then(|s| s.code())));
    if let Some(error) = stop {
        return Err(error);
    }
    job.check()?;
    ensure!(!cancel.load(AtomicOrdering::Relaxed), "Export cancelled");
    ensure!(drained, "Encoder pipes did not close after exit");
    let status = status.context("Missing FFmpeg exit status")?;
    ensure!(
        status.success(),
        "FFmpeg failed ({status}): {}",
        String::from_utf8_lossy(&state.lock().unwrap().stderr_tail)
    );
    Ok(())
}

fn readers_finished<A, B>(a: &std::thread::JoinHandle<A>, b: &std::thread::JoinHandle<B>) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !a.is_finished() || !b.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn deadline_reaps_and_drains_large_stderr() {
        let mut c = Command::new("sh");
        c.args(["-c", "exec sleep 30"]);
        let start = Instant::now();
        assert!(capture(
            &mut c,
            &Job::new(Arc::new(AtomicBool::new(false)), Duration::from_millis(50)),
            1024
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut c = Command::new("sh");
        c.args(["-c", "head -c 100000 /dev/zero >&2"]);
        assert!(capture(&mut c, &Job::default(), 1024)
            .unwrap()
            .status
            .success());
    }

    fn scripted_encoder(script: &str) -> std::process::Child {
        Command::new("sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn encoder_flood_keeps_tail_progress_and_exit() {
        // A single very long diagnostic line and a deliberately slow observer
        // must not back up the child or displace the final useful error.
        let child = scripted_encoder(
            "head -c 2000000 /dev/zero >&2; printf 'out_time_us=630000\n'; printf 'Permission denied\n' >&2; exit 23"
        );
        let mut final_event = None;
        let error = watch_encoder(child, &Job::default(), &Job::default().cancel, |activity| {
            std::thread::sleep(Duration::from_millis(100));
            final_event = Some(activity);
        })
        .unwrap_err();
        let error = error.to_string();
        assert!(error.contains("Permission denied"));
        assert!(error.len() <= 16_500);
        assert!(matches!(
            final_event,
            Some(ExportEvent::Activity {
                media_time_us: 630000,
                stderr_bytes: 2_000_018,
                exit_code: Some(23),
                outcome: "failed",
                stderr_category: "permission_denied",
                ..
            })
        ));
    }

    #[test]
    fn encoder_timeout_is_surfaced_and_reaped() {
        let child = scripted_encoder("printf 'out_time_us=630000\n'; exec sleep 20");
        let job = Job::new(Job::default().cancel, Duration::from_millis(100));
        let start = Instant::now();
        let mut final_event = None;
        let error = watch_encoder(child, &job, &job.cancel, |activity| {
            final_event = Some(activity)
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "Operation deadline exceeded");
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(matches!(
            final_event,
            Some(ExportEvent::Activity {
                outcome: "deadline",
                media_time_us: 630000,
                ..
            })
        ));
    }
}
