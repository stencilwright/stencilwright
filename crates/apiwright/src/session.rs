//! Adapter session: a raw-DOM browser bound to a site's map.
//!
//! The trust posture: this runs against **raw** (unmasked) DOM — it is the
//! runtime half of the toolchain. Adapters build site-specific flows from the
//! interaction + extraction primitives here; navigation and recognition come
//! from the [`PlaceGraph`] the adapter supplies (embedded) or loads from disk.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use stencil_browser::{Page, RawAccess, Session};
use stencil_core::paths;
use stencil_places::PlaceGraph;

use crate::daemon;
use crate::visibility::{SurfacePolicy, SurfaceTrigger, Visibility};

/// Runtime configuration for an adapter session.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Site name — selects the per-user profile under `~/.stencilwright/<site>/`.
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
/// Created by adapters; wraps a `stencil_browser` raw [`Session`] plus the
/// site's [`PlaceGraph`].
pub struct AdapterSession {
    cfg: RuntimeConfig,
    session: Session,
    graph: PlaceGraph,
    #[allow(dead_code)]
    site_dir: PathBuf,
}

impl AdapterSession {
    /// Open using the site's map under `~/.stencilwright/<site>/` (the
    /// non-standalone path — mostly useful for development against a freshly
    /// mapped site).
    pub async fn open(cfg: RuntimeConfig) -> Result<Self> {
        let site_dir = paths::site_dir(&cfg.site);
        let graph = PlaceGraph::from_dir(&site_dir).context("loading site map from disk")?;
        Self::open_with_map(cfg, graph).await
    }

    /// Open with a caller-provided map — the **standalone** path: the adapter
    /// passes its `include_str!`-embedded [`PlaceGraph`]. The map travels in the
    /// binary; only the per-user Chrome profile (auth session) lives on disk
    /// under `~/.stencilwright/<site>/profile`.
    pub async fn open_with_map(cfg: RuntimeConfig, graph: PlaceGraph) -> Result<Self> {
        let site_dir = paths::site_dir(&cfg.site);
        daemon::preflight(&site_dir)?;
        let spawn_dir = site_dir.clone();
        let visibility = cfg.visibility;
        let session = Session::attach(&site_dir, move || daemon::spawn(&spawn_dir, visibility))
            .await
            .context("attaching browser daemon")?;
        match visibility {
            Visibility::Headed => session
                .page()
                .surface_window()
                .await
                .context("surfacing headed browser window")?,
            Visibility::Offscreen => session
                .page()
                .hide_window()
                .await
                .context("hiding off-screen browser window")?,
        }
        Ok(Self {
            cfg,
            session,
            graph,
            site_dir,
        })
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.cfg
    }

    /// The selector mapped to element `name` at `place` (site-wide elements
    /// included), or `None` if the map doesn't define it. Adapters resolve their
    /// extraction selectors by *name* through this, so the selectors live in the
    /// map — and a missing name is a fail-fast signal of a stale/incomplete map.
    pub fn element_selector(&self, place: &str, name: &str) -> Option<String> {
        self.graph
            .elements_at(place)
            .into_iter()
            .find(|e| e.name == name)
            .map(|e| e.selector.clone())
    }

    /// The navigation URL of a mapped place (its `url` field), if any.
    pub fn place_url(&self, place: &str) -> Option<String> {
        self.graph.place(place).and_then(|p| p.url.clone())
    }

    fn page(&self) -> Page {
        self.session.page()
    }

    /// Navigate to a mapped place: recognize-first, navigate on miss, auto-fill
    /// interactive intermediaries (login) from the map. Runs raw; the masked
    /// dump the runner writes as a side effect is irrelevant here and ignored.
    pub async fn goto_place(&self, place: &str) -> Result<()> {
        let page = self.page();
        self.graph
            .place_goto(&page, place)
            .await
            .with_context(|| format!("navigating to place '{place}'"))?;
        Ok(())
    }

    /// Raw navigation to an explicit URL (bypasses place validation).
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.page().goto(url).await
    }

    /// Raw text of every node matching `selector`, in document order.
    pub async fn extract_text(&self, selector: &str) -> Result<Vec<String>> {
        let snippets = self
            .page()
            .selector_text_raw(RawAccess::acknowledged(), selector)
            .await?;
        Ok(snippets.into_iter().map(|s| s.text().to_string()).collect())
    }

    /// Raw value of `attr` on every node matching `selector` (missing → empty),
    /// in document order — for data carried in attributes rather than text, such
    /// as a result row's permalink `href`.
    pub async fn extract_attr(&self, selector: &str, attr: &str) -> Result<Vec<String>> {
        self.page()
            .selector_attr_raw(RawAccess::acknowledged(), selector, attr)
            .await
    }

    /// Wait until at least one node matches `selector`, or `timeout` elapses
    /// (an error). Adapters use this to wait for results to render — or to
    /// conclude "no matches" when it times out.
    pub async fn wait_for(&self, selector: &str, timeout: Duration) -> Result<()> {
        self.page().wait_for(selector, timeout).await
    }

    /// Full raw (unmasked) page HTML — for development and analysis. Adapters
    /// pull specific data with [`Self::extract_text`] / [`Self::extract_attr`];
    /// this is the whole document.
    pub async fn dump_raw(&self) -> Result<String> {
        Ok(self
            .page()
            .dump_raw(RawAccess::acknowledged())
            .await?
            .0)
    }

    /// Evaluate JavaScript in the page and return its JSON result. Raw — for
    /// development/analysis (reading network timings, app state, or a direct
    /// `fetch` against the site's own API).
    pub async fn evaluate(&self, js: &str) -> Result<serde_json::Value> {
        self.page().evaluate(js).await
    }

    // --- interaction primitives ------------------------------------------
    // Adapters compose site-specific flows from these (e.g. Acme's
    // "type the query, then press Enter twice").

    pub async fn click(&self, selector: &str) -> Result<()> {
        self.page().click(selector).await
    }

    /// Click bypassing actionability checks (present-but-unclickable controls).
    pub async fn click_force(&self, selector: &str) -> Result<()> {
        self.page().click_force(selector).await
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        self.page().fill(selector, value).await
    }

    /// Type with real per-character key events (for rich editors that ignore
    /// `fill`, like Acme's Quill-based search box).
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        self.page().type_text(selector, text).await
    }

    /// Press a key on `selector`.
    pub async fn press(&self, selector: &str, key: &str) -> Result<()> {
        self.page().press(selector, key).await
    }

    /// Press a key on the page's focused element (no selector) — e.g. submit a
    /// focused, hard-to-select search box with `"Enter"`.
    pub async fn key(&self, key: &str) -> Result<()> {
        self.page().key(key).await
    }

    /// Bring the live browser to a visible, focused window. Idempotent. Headed
    /// by default is already visible, but this also corrects an existing daemon
    /// that a previous off-screen run left hidden.
    pub async fn surface(&self) -> Result<()> {
        self.page()
            .surface_window()
            .await
            .context("surfacing browser window")?;
        Ok(())
    }

    /// Surface only if the configured [`SurfacePolicy`] opts into `trigger`.
    pub async fn maybe_surface(&self, trigger: SurfaceTrigger) -> Result<bool> {
        if self.cfg.visibility == Visibility::Offscreen && self.cfg.surface.surfaces(trigger) {
            self.surface().await?;
            return Ok(true);
        }
        Ok(false)
    }
}
