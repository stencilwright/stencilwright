//! # apiwright — adapter runtime core library
//!
//! apiwright is the **runtime half** of the stencilwright toolchain. Where
//! [`stencilwright`] *maps* a site (with masking, for safe collaborative
//! development), apiwright *drives* a mapped site against **raw DOM** and
//! exposes a clean programmatic API. Service adapters (e.g. `adapter-example`)
//! are built on apiwright.
//!
//! Two ideas define apiwright:
//!
//! 1. **It consumes maps, it does not make them.** A site's
//!    `~/.stencilwright/<site>/{places,elements,values}.toml` is produced once
//!    (collaboratively, masked) with `stencilwright`; apiwright loads it and
//!    runs against the live site with the `raw` feature on.
//! 2. **Nothing surprising.** Automation acts in the user's name, so the live
//!    browser is either visible ([`Visibility::Headed`]) or off-screen but
//!    *surfaceable on demand* ([`Visibility::Offscreen`]). It is never truly
//!    headless: a real window always exists and can be brought forward for
//!    awareness, consent, a login, or a captcha. See [`visibility`].
//!
//! See `specs/01-apiwright.md`.
//!
//! [`stencilwright`]: https://github.com/stencilwright/stencilwright

// Re-export the stencil crates adapters build against, so an adapter depends on
// `apiwright` alone.
pub use stencil_browser;
pub use stencil_core;
pub use stencil_places;
pub use stencil_secrets;

pub mod daemon;
pub mod session;
pub mod visibility;

pub use daemon::run_if_daemon;
pub use session::{AdapterSession, RuntimeConfig};
pub use visibility::{SurfacePolicy, SurfaceTrigger, Visibility};
