//! Session daemon: spawns headed Chrome with a persistent profile,
//! exposes a Unix socket for short-lived clients to RPC into.
//!
//! Submodules carve the concerns:
//!
//! - [`session`] — the `.session` file: [`SessionInfo`],
//!   [`DaemonStatus`], [`live_socket`], [`status`], [`stop`].
//! - [`lifecycle`] — the spawn protocol: [`ensure_running`].
//! - [`run`] — the daemon body itself: Playwright launch, shutdown
//!   `select!`, RPC stub.
//!
//! Trust boundary: stencilwright uses the daemon's `dump_masked` RPC,
//! which resolves values in daemon memory and returns only masked
//! HTML. Raw-oriented RPCs remain library internals for the
//! `raw`-feature path; `dump_raw` lives in `page.rs` behind
//! `#[cfg(feature = "raw")]` and is only reachable from
//! `apiwright`-shaped binaries.

use std::time::{SystemTime, UNIX_EPOCH};

mod dispatch;
mod lifecycle;
pub mod run;
mod session;

pub use lifecycle::ensure_running;
pub use run::run;
pub use session::{DaemonStatus, SessionInfo, live_socket, status, stop};

/// Wall-clock seconds since Unix epoch. We store it in `.session` so
/// `status` can report uptime without pulling in chrono.
pub(super) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `kill(pid, 0)` runs the kernel's existence/permission check
/// without delivering a signal — returns 0 if the pid is alive.
pub(super) fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
