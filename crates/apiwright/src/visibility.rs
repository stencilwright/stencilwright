//! Visibility & consent model.
//!
//! An adapter's browser runs **headed**, or **off-screen with the ability to
//! surface** (snap to a visible, focused window) on demand. The guiding
//! principle: automation in the user's name must never be *surprising* —
//! anything the user should witness (a login, a captcha, an unrecognized page,
//! a consent checkpoint, or simply "show me") brings the live browser forward.
//!
//! apiwright is deliberately **never fully headless**. A real OS window always
//! exists; "off-screen" only means it isn't currently presented. This keeps
//! captcha-solving, 2FA, and ad-hoc oversight always possible.

/// How the adapter's browser window is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Visible, focused window. The default for anything acting in the user's
    /// name.
    #[default]
    Headed,
    /// Running but not presented (positioned off-screen / minimized / on a
    /// virtual display). Can be surfaced on demand or automatically per
    /// [`SurfacePolicy`]. Never truly headless.
    Offscreen,
}

/// Events that should pull an [`Visibility::Offscreen`] browser to the
/// foreground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceTrigger {
    /// A login / auth wall was recognized.
    Login,
    /// A captcha / human-verification challenge was recognized.
    Captcha,
    /// The runner landed on a page no map recognizes.
    Unrecognized,
    /// A consent checkpoint before an action the user should witness.
    Consent,
    /// The user (or calling code) explicitly asked to watch.
    Requested,
}

/// Which triggers auto-surface an off-screen session. Conservative by default:
/// surface for anything that needs a human or that the user would plainly want
/// to see happen.
#[derive(Debug, Clone, Copy)]
pub struct SurfacePolicy {
    pub on_login: bool,
    pub on_captcha: bool,
    pub on_unrecognized: bool,
    pub on_consent: bool,
}

impl Default for SurfacePolicy {
    fn default() -> Self {
        Self { on_login: true, on_captcha: true, on_unrecognized: true, on_consent: true }
    }
}

impl SurfacePolicy {
    /// Never auto-surface (fully unattended). The session can still be surfaced
    /// explicitly via [`SurfaceTrigger::Requested`].
    pub fn unattended() -> Self {
        Self { on_login: false, on_captcha: false, on_unrecognized: false, on_consent: false }
    }

    /// Whether `trigger` should bring an off-screen window forward.
    /// [`SurfaceTrigger::Requested`] always surfaces.
    pub fn surfaces(&self, trigger: SurfaceTrigger) -> bool {
        match trigger {
            SurfaceTrigger::Login => self.on_login,
            SurfaceTrigger::Captcha => self.on_captcha,
            SurfaceTrigger::Unrecognized => self.on_unrecognized,
            SurfaceTrigger::Consent => self.on_consent,
            SurfaceTrigger::Requested => true,
        }
    }
}
