//! Spawn protocol: get a daemon up if there isn't one, serializing
//! concurrent starts via `flock` on `.session.lock`.
//!
//! [`ensure_running`] takes a closure that knows how to spawn the
//! daemon subprocess (typically `Command::new(current_exe).arg("daemon")
//! .arg(site_dir).spawn()`). We don't bake that knowledge in here
//! because `stencil-browser` doesn't know its host binary.

use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use super::session::live_socket;

const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(10);

/// Ensure a daemon is running. Returns the socket path.
///
/// Sequence: live-check (fast path) → flock → re-check → cleanup
/// stale artifacts → invoke spawner → poll for socket.
pub async fn ensure_running<F>(site_dir: &Path, spawn_daemon: F) -> Result<PathBuf>
where
    F: FnOnce() -> Result<()>,
{
    if let Some(sock) = live_socket(site_dir)? {
        return Ok(sock);
    }

    let lock_path = site_dir.join(".session.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening lock file {}", lock_path.display()))?;
    let _guard = FlockGuard::lock_exclusive(&lock_file)?;

    // Another concurrent starter may have finished while we waited.
    if let Some(sock) = live_socket(site_dir)? {
        return Ok(sock);
    }

    let _ = fs::remove_file(site_dir.join(".session"));
    let _ = fs::remove_file(site_dir.join(".session.sock"));

    spawn_daemon()?;

    wait_for_live_session(site_dir, DAEMON_START_TIMEOUT).await
}

async fn wait_for_live_session(site_dir: &Path, timeout: Duration) -> Result<PathBuf> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(sock) = live_socket(site_dir)? {
            return Ok(sock);
        }
        if Instant::now() >= deadline {
            bail!(
                "daemon failed to come up within {:?}; check {}",
                timeout,
                site_dir.join(".session.log").display()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// RAII flock wrapper. Holds an exclusive advisory lock on the file
/// descriptor; releases on drop.
struct FlockGuard {
    fd: i32,
}

impl FlockGuard {
    fn lock_exclusive(file: &fs::File) -> Result<Self> {
        let fd = file.as_raw_fd();
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if rc != 0 {
            return Err(anyhow!(
                "flock LOCK_EX failed: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self { fd })
    }
}

impl Drop for FlockGuard {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.fd, libc::LOCK_UN);
        }
    }
}
