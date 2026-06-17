//! Daemon hosting for adapter binaries.
//!
//! The raw browser daemon is the same long-lived, real-Chrome process
//! `stencil-browser` provides. An adapter binary starts one by re-exec'ing
//! *itself* as `<exe> daemon <site_dir>` (the daemon can't be a separate binary
//! because it must share the adapter's `raw`-enabled `stencil-browser` build).
//!
//! - [`run_if_daemon`] handles that re-exec at the top of `main`.
//! - [`spawn`] is what [`crate::AdapterSession::open`] uses to fork one.
//! - [`preflight`] ensures the per-user profile dir + Chrome exist.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use stencil_browser::daemon::{RunOptions, WindowPresentation};

use crate::visibility::Visibility;

const CHROME_APP_PATH: &str = "/Applications/Google Chrome.app";
const OFFSCREEN_ARG: &str = "--offscreen";

/// Call at the very top of `main`. If this process was re-exec'd as the session
/// daemon (`<exe> daemon <site_dir>`), run it to completion and return
/// `Ok(true)`; otherwise return `Ok(false)` so the caller proceeds as the CLI.
pub async fn run_if_daemon() -> Result<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("daemon")) {
        return Ok(false);
    }
    let dir = args
        .next()
        .context("`daemon` subcommand requires a site-directory argument")?;
    let options = parse_daemon_options(args)?;
    stencil_browser::daemon::run_with_options(PathBuf::from(dir), options).await?;
    Ok(true)
}

/// Ensure the per-user site directory + Chrome profile exist and Chrome is
/// installed. The *map* is embedded in the adapter; this directory only holds
/// the browser profile (auth session) and the daemon socket — never the map.
pub(crate) fn preflight(site_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(site_dir.join("profile"))
        .with_context(|| format!("creating {}/profile", site_dir.display()))?;
    if !Path::new(CHROME_APP_PATH).exists() {
        bail!(
            "Google Chrome not found at {CHROME_APP_PATH}. \
             Install from https://www.google.com/chrome/."
        );
    }
    Ok(())
}

/// Fork-detach the daemon by re-exec'ing the current binary as
/// `<exe> daemon <site_dir>`. Mirrors stencilwright's spawner; the daemon body
/// calls `setsid` itself. `OP_SESSION*` is scrubbed so a caller's shell auth
/// state isn't silently reused by the daemon-owned secret path.
pub(crate) fn spawn(site_dir: &Path, visibility: Visibility) -> Result<()> {
    let exe = std::env::current_exe().context("std::env::current_exe()")?;
    let site_abs = site_dir
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", site_dir.display()))?;
    let log_path = site_abs.join(".session.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log2 = log.try_clone().context("cloning log file handle")?;

    let mut command = Command::new(&exe);
    command.arg("daemon").arg(&site_abs);
    if visibility == Visibility::Offscreen {
        command.arg(OFFSCREEN_ARG);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("OP_SESSION") {
            command.env_remove(key);
        }
    }
    command
        .spawn()
        .with_context(|| format!("spawning {} daemon {}", exe.display(), site_abs.display()))?;
    Ok(())
}

fn parse_daemon_options(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<RunOptions> {
    let mut options = RunOptions::headed();
    for arg in args {
        let arg = arg
            .to_str()
            .with_context(|| format!("daemon option is not valid UTF-8: {arg:?}"))?;
        match arg {
            OFFSCREEN_ARG => {
                options.presentation = WindowPresentation::Offscreen;
            }
            other => bail!("unknown daemon option: {other}"),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn parse_daemon_options_defaults_to_headed() {
        let options = parse_daemon_options(Vec::<OsString>::new()).unwrap();
        assert_eq!(options, RunOptions::headed());
    }

    #[test]
    fn parse_daemon_options_accepts_offscreen() {
        let options = parse_daemon_options([OsString::from(OFFSCREEN_ARG)]).unwrap();
        assert_eq!(options, RunOptions::offscreen());
    }

    #[test]
    fn parse_daemon_options_rejects_unknown_flags() {
        let err = parse_daemon_options([OsString::from("--headless")]).unwrap_err();
        assert!(err.to_string().contains("unknown daemon option"));
    }
}
