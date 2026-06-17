//! Daemon body. The detached subprocess calls [`run`] and stays here
//! until SIGTERM, SIGINT, the user closes the Chrome window, or the
//! accept loop dies.
//!
//! CP3 ships only a stub RPC handler — every connection gets a
//! placeholder reply. CP4 wires real Page dispatch.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use playwright_rs::{
    Playwright, api::launch_options::IgnoreDefaultArgs,
    protocol::browser_context::BrowserContextOptions, protocol::page::Page as PwPage,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tracing::{info, warn};

use super::dispatch::{DaemonState, handle_request};
use super::now_unix_secs;
use super::session::{SessionInfo, write_session_atomic};
use crate::rpc::{Request, Response};
use crate::site_config;

use super::window;

/// How the daemon should present its real Chrome window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowPresentation {
    /// Visible headed browser window.
    #[default]
    Headed,
    /// Real headed browser window positioned away from the user's workspace.
    Offscreen,
}

/// Daemon startup options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunOptions {
    pub presentation: WindowPresentation,
}

impl RunOptions {
    pub fn headed() -> Self {
        Self {
            presentation: WindowPresentation::Headed,
        }
    }

    pub fn offscreen() -> Self {
        Self {
            presentation: WindowPresentation::Offscreen,
        }
    }
}

/// Daemon entry point. Blocks (in the tokio runtime) until shutdown.
pub async fn run(site_dir: PathBuf) -> Result<()> {
    run_with_options(site_dir, RunOptions::default()).await
}

/// Daemon entry point with explicit startup options.
pub async fn run_with_options(site_dir: PathBuf, options: RunOptions) -> Result<()> {
    // Detach from controlling terminal so the daemon survives the
    // launching shell. Errors are non-fatal: we may already be a
    // process-group leader (e.g. under launchd).
    unsafe {
        libc::setsid();
    }

    info!(site = %site_dir.display(), pid = std::process::id(), "daemon starting");

    let profile_dir = site_dir.join("profile");
    if !profile_dir.is_dir() {
        bail!(
            "profile dir not found at {}; run `stencilwright init <site>` first",
            profile_dir.display()
        );
    }
    let op_config = site_config::load_op_config(&site_dir)?;

    let pw = Playwright::launch()
        .await
        .context("playwright_rs::Playwright::launch() failed")?;

    let mut opts = BrowserContextOptions::builder()
        .channel("chrome".into())
        .headless(false)
        // Keep Chrome's process sandbox ON. Playwright defaults chromium_sandbox
        // to false, which injects `--no-sandbox` and its "Stability and security
        // will suffer" infobar. We drive your real authenticated sessions on the
        // desktop, where the sandbox works and is exactly what should be
        // isolating page renderers — so we opt back in.
        .chromium_sandbox(true)
        .ignore_default_args(IgnoreDefaultArgs::Array(vec!["--enable-automation".into()]));

    if options.presentation == WindowPresentation::Offscreen {
        opts = opts.args(window::offscreen_launch_args());
    }

    let opts = opts.build();

    let ctx = pw
        .chromium()
        .launch_persistent_context_with_options(profile_dir.to_string_lossy().into_owned(), opts)
        .await
        .context("launch_persistent_context_with_options failed (is system Chrome installed?)")?;

    let page = match ctx.pages().into_iter().next() {
        Some(p) => p,
        None => ctx
            .new_page()
            .await
            .context("opening initial Page failed")?,
    };
    let _ = page.goto("about:blank", None).await;

    if options.presentation == WindowPresentation::Offscreen {
        window::hide(&page)
            .await
            .context("positioning Chrome window off-screen")?;
    }

    let sock_path = site_dir.join(".session.sock");
    let _ = fs::remove_file(&sock_path); // defensive — bind fails on EADDRINUSE
    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("binding {}", sock_path.display()))?;

    let info = SessionInfo {
        pid: std::process::id(),
        sock: sock_path.to_string_lossy().into_owned(),
        started_at: now_unix_secs(),
    };
    write_session_atomic(&site_dir.join(".session"), &info)?;
    info!(sock = %sock_path.display(), "daemon ready");

    let outcome = wait_for_shutdown(listener, page.clone(), op_config).await;
    info!(?outcome, "daemon shutting down");

    let _ = ctx.close().await;
    let _ = fs::remove_file(site_dir.join(".session"));
    let _ = fs::remove_file(&sock_path);

    Ok(())
}

#[derive(Debug)]
enum ShutdownReason {
    Sigterm,
    Sigint,
    ChromeClosed,
    AcceptLoopDied,
}

async fn wait_for_shutdown(
    listener: UnixListener,
    page: PwPage,
    op_config: stencil_secrets::OpConfig,
) -> ShutdownReason {
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!("failed to install SIGTERM handler: {e}");
            return ShutdownReason::AcceptLoopDied;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            warn!("failed to install SIGINT handler: {e}");
            return ShutdownReason::AcceptLoopDied;
        }
    };

    let state_for_clients = DaemonState::new(page.clone(), op_config);
    let chrome_closed = async move {
        loop {
            if page.is_closed() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };

    let accept_loop = async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_client(stream, state_for_clients.clone()));
                }
                Err(e) => {
                    warn!("accept error: {e}");
                    return;
                }
            }
        }
    };

    tokio::select! {
        _ = sigterm.recv() => ShutdownReason::Sigterm,
        _ = sigint.recv() => ShutdownReason::Sigint,
        _ = chrome_closed => ShutdownReason::ChromeClosed,
        _ = accept_loop => ShutdownReason::AcceptLoopDied,
    }
}

/// One client connection. Reads JSON-line requests, dispatches each
/// to the held Page, writes JSON-line responses. Loops until EOF.
async fn handle_client(stream: UnixStream, state: DaemonState) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(e) => {
                warn!("rpc read error: {e}");
                return;
            }
        };
        if n == 0 {
            return; // client disconnected
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(&state, req).await,
            Err(e) => Response::err(0, format!("parse error: {e}")),
        };
        let mut out = match serde_json::to_string(&resp) {
            Ok(s) => s,
            Err(e) => {
                warn!("response serialization failed: {e}");
                return;
            }
        };
        out.push('\n');
        if write_half.write_all(out.as_bytes()).await.is_err() {
            return;
        }
    }
}
