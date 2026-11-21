//! The Windows GUI owns an OS file lock scoped to the user and logon session.
//! The local mailbox carries one project offer and a coalesced activation.
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_REQUEST: u64 = 65_536;

pub struct Primary {
    lock: File,
    root: PathBuf,
}

impl Drop for Primary {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}

impl Primary {
    pub fn activate(&self) -> bool {
        fs::remove_file(self.root.join("activate")).is_ok()
    }

    pub fn project(&self) -> Option<PathBuf> {
        let _mailbox = mailbox(&self.root).ok()?;
        let path = self.root.join("project");
        let mut request = File::open(&path).ok()?;
        let mut data = Vec::new();
        let result = (&mut request).take(MAX_REQUEST + 1).read_to_end(&mut data);
        drop(request);
        if fs::remove_file(path).is_err() || result.is_err() || data.len() as u64 > MAX_REQUEST {
            return None;
        }
        let path = PathBuf::from(String::from_utf8(data).ok()?);
        validate_project(&path).then_some(path)
    }
}

fn validate_project(path: &Path) -> bool {
    path.is_absolute()
        && path.is_file()
        && path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("m4bproj") || ext.eq_ignore_ascii_case("json")
        })
}

fn mailbox(root: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("mailbox.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

fn send_project(root: &Path, project: &Path) -> io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match mailbox(root) {
            Ok(_mailbox) => {
                return write_atomic(root, "project", project.as_os_str().as_encoded_bytes())
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn write_atomic(root: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(data)?;
    temp.persist(root.join(name))?;
    Ok(())
}

fn claim(root: &Path, project: Option<&Path>) -> io::Result<Option<Primary>> {
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("owner.lock"))?;
    match lock.try_lock() {
        Ok(()) => {
            if let Some(project) = project {
                send_project(root, project)?;
            }
            write_atomic(root, "pid", std::process::id().to_string().as_bytes())?;
            Ok(Some(Primary {
                lock,
                root: root.into(),
            }))
        }
        Err(TryLockError::WouldBlock) => {
            // The second process can grant the primary foreground permission.
            #[cfg(windows)]
            if let Ok(file) = File::open(root.join("pid")) {
                let mut pid = String::new();
                if file.take(16).read_to_string(&mut pid).is_ok() {
                    if let Ok(pid) = pid.parse::<u32>() {
                        if pid != 0 && pid != u32::MAX {
                            unsafe { AllowSetForegroundWindow(pid) };
                        }
                    }
                }
            }
            if let Some(project) = project {
                send_project(root, project)?;
            }
            write_atomic(root, "activate", b"")?;
            Ok(None)
        }
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn AllowSetForegroundWindow(pid: u32) -> i32;
}

#[cfg(windows)]
pub fn start() -> io::Result<Option<Primary>> {
    #[link(name = "kernel32")]
    extern "system" {
        fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    }
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let project = std::env::args_os()
        .nth(1)
        .and_then(|path| dunce::canonicalize(path).ok())
        .filter(|path| {
            validate_project(path)
                && path
                    .to_str()
                    .is_some_and(|text| text.len() as u64 <= MAX_REQUEST)
        });
    claim(
        &super::state_directory().join(format!("instance-{session}")),
        project.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_forwards_path_and_released_lock_allows_restart() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Livre é.m4bproj");
        fs::write(&project, b"{}").unwrap();
        let primary = claim(dir.path(), None).unwrap().unwrap();
        assert!(claim(dir.path(), Some(&project)).unwrap().is_none());
        assert!(claim(dir.path(), None).unwrap().is_none());
        assert!(primary.activate());
        assert!(!primary.activate());
        assert_eq!(primary.project(), Some(project));
        assert!(primary.project().is_none());
        drop(primary);
        assert!(claim(dir.path(), None).unwrap().is_some());
    }
}
