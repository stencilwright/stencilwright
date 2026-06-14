//! CLI side of the daemon lifecycle: host checks, daemon subprocess
//! spawning, and the `session start/stop/status` handlers.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use stencil_browser::daemon::{self, DaemonStatus};
use stencil_core::paths;

const CHROME_APP_PATH: &str = "/Applications/Google Chrome.app";

/// `session start` — auto-starts the daemon if it isn't running.
pub async fn start(site_dir: &Path) -> Result<()> {
    if daemon::live_socket(site_dir)?.is_none() {
        preflight(site_dir).await?;
    }
    let site_dir_owned = site_dir.to_path_buf();
    let sock = daemon::ensure_running(site_dir, move || spawn_subprocess(&site_dir_owned)).await?;
    let status = daemon::status(site_dir)?;
    match status {
        DaemonStatus::Running { pid, uptime_secs } => {
            println!(
                "daemon ready (pid={pid}, uptime={uptime_secs}s, sock={})",
                sock.display()
            );
        }
        other => {
            println!("daemon socket={} status={other:?}", sock.display());
        }
    }
    Ok(())
}

/// `session stop` — SIGTERM the daemon and wait for cleanup.
pub async fn stop(site_dir: &Path) -> Result<()> {
    if !site_dir.exists() {
        bail!("{} does not exist; nothing to stop", site_dir.display());
    }
    daemon::stop(site_dir).await?;
    println!("daemon stopped");
    Ok(())
}

/// `session status` — print human-readable state.
pub async fn status(site_dir: &Path) -> Result<()> {
    if !site_dir.exists() {
        println!(
            "not running (site dir {} does not exist)",
            site_dir.display()
        );
        return Ok(());
    }
    match daemon::status(site_dir)? {
        DaemonStatus::NotRunning => println!("not running"),
        DaemonStatus::Running { pid, uptime_secs } => {
            println!("running (pid={pid}, uptime={uptime_secs}s)")
        }
        DaemonStatus::Stale { pid } => {
            println!(
                "stale (.session records pid={pid} but it isn't alive); run `session stop` to clean up"
            )
        }
    }
    Ok(())
}

/// Verify the host has enough browser/session state before spawning.
/// Surfaces clear errors so the user doesn't have to grep .session.log.
pub(crate) async fn preflight(site_dir: &Path) -> Result<()> {
    if !site_dir.is_dir() {
        bail!(
            "{} not found; run `stencilwright init <site>` first",
            site_dir.display()
        );
    }
    let profile = site_dir.join("profile");
    if !profile.is_dir() {
        bail!("{} missing; rerun `stencilwright init`", profile.display());
    }
    if !Path::new(CHROME_APP_PATH).exists() {
        bail!(
            "Google Chrome not found at {CHROME_APP_PATH}. \
             Install from https://www.google.com/chrome/."
        );
    }
    Ok(())
}

/// Fork-detach the daemon subprocess. We rely on the daemon body
/// calling `setsid` itself rather than doing it pre-exec here.
pub(crate) fn spawn_subprocess(site_dir: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("std::env::current_exe()")?;
    // Canonicalize so the daemon is robust to any future CWD change.
    let site_abs = site_dir
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", site_dir.display()))?;
    let log_path = site_abs.join(".session.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log2 = log.try_clone().with_context(|| "cloning log file handle")?;

    let mut command = Command::new(&exe);
    command
        .arg("daemon")
        .arg(&site_abs)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    scrub_secret_provider_sessions(&mut command);
    command
        .spawn()
        .with_context(|| format!("spawning {} daemon {}", exe.display(), site_abs.display()))?;
    Ok(())
}

fn scrub_secret_provider_sessions(command: &mut Command) {
    // Avoid turning an already-authenticated caller shell into the
    // daemon's implicit provider session. Daemon-created auth cannot
    // flow back to the parent process environment either way.
    scrub_secret_provider_sessions_from(command, std::env::vars_os().map(|(key, _)| key));
}

fn scrub_secret_provider_sessions_from<I>(command: &mut Command, keys: I)
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    for key in keys {
        if key.to_string_lossy().starts_with("OP_SESSION") {
            command.env_remove(key);
        }
    }
}

/// Resolve `~/.stencilwright/<site>`.
pub fn site_dir(site: &str) -> PathBuf {
    paths::site_dir(site)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn daemon_spawn_scrubs_onepassword_session_env_only() {
        let mut command = Command::new("stencilwright");
        command.env("OP_ACCOUNT", "my.1password.com");

        super::scrub_secret_provider_sessions_from(
            &mut command,
            ["OP_SESSION_my", "OP_ACCOUNT"].map(std::ffi::OsString::from),
        );

        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect::<Vec<_>>();

        assert!(env.contains(&("OP_SESSION_my".to_string(), None)));
        assert!(env.contains(&(
            "OP_ACCOUNT".to_string(),
            Some("my.1password.com".to_string())
        )));
    }
}
