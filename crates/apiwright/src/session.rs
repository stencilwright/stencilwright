//! Adapter session: a raw-DOM browser bound to a site's map.
//!
//! **Skeleton.** The method bodies are `todo!()` until the runtime is fleshed
//! out alongside the first adapter (`adapter-example`). The shapes here are the
//! contract that `specs/01-apiwright.md` describes.

use crate::visibility::{SurfacePolicy, SurfaceTrigger, Visibility};

/// Runtime configuration for an adapter session.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Site name — selects the map under `~/.stencilwright/<site>/`.
    pub site: String,
    /// Headed (default) or off-screen-but-surfaceable.
    pub visibility: Visibility,
    /// When an off-screen session auto-surfaces.
    pub surface: SurfacePolicy,
}

impl RuntimeConfig {
    pub fn new(site: impl Into<String>) -> Self {
        Self {
            site: site.into(),
            visibility: Visibility::default(),
            surface: SurfacePolicy::default(),
        }
    }

    /// Run off-screen; the window can still be surfaced on demand or per policy.
    pub fn offscreen(mut self) -> Self {
        self.visibility = Visibility::Offscreen;
        self
    }

    pub fn surface_policy(mut self, policy: SurfacePolicy) -> Self {
        self.surface = policy;
        self
    }
}

/// A live, map-bound browser session driven against **raw** (unmasked) DOM.
///
/// Created by adapters; not constructed directly by end users. Wraps a
/// `stencil_browser` raw `Session` plus the site's `stencil_places::PlaceGraph`.
pub struct AdapterSession {
    cfg: RuntimeConfig,
}

impl AdapterSession {
    /// Attach to (auto-starting) the site daemon and load its map.
    pub async fn open(cfg: RuntimeConfig) -> anyhow::Result<Self> {
        // TODO: stencil_browser::Session::attach(&cfg.site) with raw access,
        // load PlaceGraph::from_dir(~/.stencilwright/<site>), honor cfg.visibility.
        let _ = &cfg;
        todo!("wire stencil-browser (raw) Session + stencil-places PlaceGraph")
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.cfg
    }

    /// Bring the live browser to a visible, focused window. Idempotent.
    pub async fn surface(&self) -> anyhow::Result<()> {
        todo!("move window on-screen + focus")
    }

    /// Surface only if the configured [`SurfacePolicy`] opts into `trigger`.
    pub async fn maybe_surface(&self, trigger: SurfaceTrigger) -> anyhow::Result<bool> {
        if self.cfg.visibility == Visibility::Offscreen && self.cfg.surface.surfaces(trigger) {
            self.surface().await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Navigate to a mapped place (recognize-first, navigate on miss). Surfaces
    /// automatically on a login / captcha / unrecognized landing per policy.
    pub async fn goto_place(&self, _place: &str) -> anyhow::Result<()> {
        todo!("PlaceGraph::place_goto against raw DOM, with surface-on-trigger")
    }

    /// Raw text of every node matching `selector` at the current place.
    pub async fn extract_text(&self, _selector: &str) -> anyhow::Result<Vec<String>> {
        todo!("Page::selector_text_raw")
    }
}
