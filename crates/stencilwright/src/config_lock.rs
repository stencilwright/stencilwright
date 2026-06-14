//! Site-local advisory lock for mapping config mutations.

use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Holds an exclusive advisory lock on `<site>/.config.lock`.
///
/// The lock spans the read-modify-write cycle for TOML-mutating CLI
/// commands. It is process-scoped and released automatically if the CLI
/// exits.
pub(crate) struct ConfigLock {
    fd: i32,
    _file: fs::File,
}

impl ConfigLock {
    pub(crate) fn lock(site_dir: &Path) -> Result<Self> {
        let path = site_dir.join(".config.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening config lock {}", path.display()))?;
        let fd = file.as_raw_fd();
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if rc != 0 {
            return Err(anyhow!(
                "flock LOCK_EX failed for {}: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
        Ok(Self { fd, _file: file })
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.fd, libc::LOCK_UN);
        }
    }
}

pub(crate) fn write_atomic(path: &Path, body: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("config path has no file name: {}", path.display()))?
        .to_string_lossy();
    let counter = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ));
    fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))
}
