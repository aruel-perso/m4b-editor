//! Crash recovery for the current project in each running instance.
use m4b_editor_core::Project;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Prompt {
    pub path: PathBuf,
    pub name: anyhow::Result<String>,
    pub date: Option<String>,
}

impl Prompt {
    pub fn new(path: PathBuf) -> Self {
        let date = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(utc_date);
        let name = Project::load(&path).map(|project| project.name);
        Self { path, name, date }
    }
}

pub fn resolve(path: &Path) -> std::io::Result<()> {
    std::fs::write(path.with_extension("resolved"), b"Recovery resolved")
}

fn utc_date(modified: SystemTime) -> Option<String> {
    let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let mut days = seconds / 86_400;
    for year in 1970..=9999 {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let year_days = 365 + u64::from(leap);
        if days >= year_days {
            days -= year_days;
            continue;
        }
        let months = [
            31,
            28 + u64::from(leap),
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        for (index, length) in months.into_iter().enumerate() {
            if days < length {
                let (month, day) = (index + 1, days + 1);
                let (hour, minute) = (seconds / 3600 % 24, seconds / 60 % 60);
                return Some(format!(
                    "{year}-{month:02}-{day:02} {hour:02}:{minute:02} UTC"
                ));
            }
            days -= length;
        }
    }
    None
}

pub struct Recovery {
    directory: PathBuf,
    preserve_on_drop: bool,
}
impl Recovery {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let directory = tempfile::Builder::new()
            .prefix("instance-")
            .tempdir_in(root)?
            .keep();
        std::fs::write(directory.join("owner"), std::process::id().to_string())?;
        Ok(Self {
            directory,
            preserve_on_drop: false,
        })
    }
    pub fn slot(&self) -> PathBuf {
        self.directory.join("current.m4bproj")
    }
    pub fn clear(&self) {
        let _ = std::fs::remove_file(self.slot());
    }
    pub fn preserve_on_exit(&mut self) {
        self.preserve_on_drop = true;
    }
    pub fn discover(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let directory = entry.path();
                if !directory.is_dir() {
                    continue;
                }
                let pid = std::fs::read_to_string(directory.join("owner"))
                    .ok()
                    .and_then(|p| p.parse().ok());
                // Older versions removed the owner on normal exit but left stale slots.
                if pid.is_none_or(|pid| pid == 0 || alive(pid)) {
                    continue;
                }
                if let Ok(slots) = std::fs::read_dir(directory) {
                    found.extend(slots.flatten().map(|s| s.path()).filter(|p| {
                        p.extension().is_some_and(|e| e == "m4bproj")
                            && !p.with_extension("resolved").exists()
                            && Project::load(p).is_ok()
                    }));
                }
            }
        }
        found.sort();
        found
    }
}
impl Drop for Recovery {
    fn drop(&mut self) {
        if self.preserve_on_drop || std::thread::panicking() {
            return;
        }
        self.clear();
        let _ = std::fs::remove_file(self.directory.join("owner"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
    }
    unsafe {
        let handle = OpenProcess(0x1000, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let result = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        result != 0 && code == 259
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autosave_timestamp_includes_leap_day_and_utc_time() {
        let timestamp = UNIX_EPOCH + std::time::Duration::from_secs(1_709_210_096);
        assert_eq!(utc_date(timestamp).as_deref(), Some("2024-02-29 12:34 UTC"));
    }

    #[test]
    fn only_abandoned_valid_unresolved_snapshots_are_discovered() {
        let root = tempfile::tempdir().unwrap();
        let clean = Recovery::new(root.path()).unwrap();
        let interrupted = Recovery::new(root.path()).unwrap();
        let project = Project::named("Book").unwrap();
        project.save_new(&clean.slot()).unwrap();
        project.save_new(&interrupted.slot()).unwrap();
        assert_ne!(clean.slot(), interrupted.slot());
        assert!(Recovery::discover(root.path()).is_empty());
        let clean_slot = clean.slot();
        clean.clear(); // Successful project save
        assert!(!clean_slot.exists());
        project.save_new(&clean_slot).unwrap();
        drop(clean); // Normal close
        assert!(!clean_slot.exists());
        let abandoned = interrupted.slot();
        #[cfg(unix)]
        let dead_pid = i32::MAX.to_string();
        #[cfg(windows)]
        let dead_pid = u32::MAX.to_string();
        std::fs::write(interrupted.directory.join("owner"), dead_pid).unwrap();
        let stale = root.path().join("instance-legacy-clean");
        std::fs::create_dir(&stale).unwrap();
        project.save_new(&stale.join("stale.m4bproj")).unwrap();
        assert_eq!(Recovery::discover(root.path()), vec![abandoned.clone()]);
        resolve(&abandoned).unwrap();
        assert!(Recovery::discover(root.path()).is_empty());
        assert_eq!(Project::load(&abandoned).unwrap().name, "Book");
        std::fs::remove_file(abandoned.with_extension("resolved")).unwrap();
        std::fs::write(&abandoned, b"invalid").unwrap();
        assert!(Recovery::discover(root.path()).is_empty());
    }
}
