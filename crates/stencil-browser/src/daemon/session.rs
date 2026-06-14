//! `.session` file: data model + read/write + the public queries
//! that operate on the file (`live_socket`, `status`, `stop`).
//!
//! `.session` is a per-site JSON file recording the daemon's pid,
//! socket path, and start time. It's the rendezvous point between
//! the daemon and short-lived clients.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{is_pid_alive, now_unix_secs};

const STOP_GRACE: Duration = Duration::from_secs(5);

/// Persisted daemon record at `<site>/.session`. Sensitive only by
/// virtue of containing the socket path (which is also on disk
/// alongside) — written 0o600 defensively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub pid: u32,
    pub sock: String,
    pub started_at: u64,
}

/// Daemon liveness summary returned by [`status`].
#[derive(Debug, Clone)]
pub enum DaemonStatus {
    Running {
        pid: u32,
        uptime_secs: u64,
    },
    NotRunning,
    /// `.session` present but the recorded pid is not alive.
    Stale {
        pid: u32,
    },
}

/// Return the daemon's socket path if `.session` is present, the
/// recorded pid is alive, and the socket file exists. Stale records
/// are NOT cleaned up here — that's `ensure_running`'s job under the
/// per-site lock.
pub fn live_socket(site_dir: &Path) -> Result<Option<PathBuf>> {
    let session_path = site_dir.join(".session");
    if !session_path.exists() {
        return Ok(None);
    }
    let info = read_session(&session_path)?;
    if !is_pid_alive(info.pid) {
        return Ok(None);
    }
    let sock_path = PathBuf::from(&info.sock);
    if !sock_path.exists() {
        return Ok(None);
    }
    Ok(Some(sock_path))
}

/// Liveness + uptime (or `NotRunning` / `Stale`).
pub fn status(site_dir: &Path) -> Result<DaemonStatus> {
    let session_path = site_dir.join(".session");
    if !session_path.exists() {
        return Ok(DaemonStatus::NotRunning);
    }
    let info = read_session(&session_path)?;
    if !is_pid_alive(info.pid) {
        return Ok(DaemonStatus::Stale { pid: info.pid });
    }
    Ok(DaemonStatus::Running {
        pid: info.pid,
        uptime_secs: now_unix_secs().saturating_sub(info.started_at),
    })
}

/// SIGTERM the daemon and wait briefly for it to clean its files.
/// Idempotent: returns `Ok(())` if no daemon is recorded.
pub async fn stop(site_dir: &Path) -> Result<()> {
    let session_path = site_dir.join(".session");
    if !session_path.exists() {
        return Ok(());
    }
    let info = read_session(&session_path)?;
    if !is_pid_alive(info.pid) {
        // Stale record: just clean up.
        let _ = fs::remove_file(&session_path);
        let _ = fs::remove_file(site_dir.join(".session.sock"));
        return Ok(());
    }
    unsafe {
        if libc::kill(info.pid as i32, libc::SIGTERM) != 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("kill SIGTERM pid={}", info.pid));
        }
    }
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if !session_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Daemon didn't unwind in time — force-remove its artifacts.
    let _ = fs::remove_file(&session_path);
    let _ = fs::remove_file(site_dir.join(".session.sock"));
    Ok(())
}

pub(super) fn read_session(path: &Path) -> Result<SessionInfo> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {} as SessionInfo", path.display()))
}

/// Write atomically via tmp-file + rename, mode 0o600.
pub(super) fn write_session_atomic(path: &Path, info: &SessionInfo) -> Result<()> {
    let tmp = path.with_extension("session.tmp");
    let body = serde_json::to_vec_pretty(info)?;
    fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
    let mut perms = fs::metadata(&tmp)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&tmp, perms)?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
