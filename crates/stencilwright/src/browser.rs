//! Shared browser/session helpers for stencilwright commands.

use std::path::PathBuf;

use anyhow::Result;
use stencil_browser::Session;
use stencil_places::PlaceGraph;

use crate::session;

pub(crate) struct SiteRuntime {
    pub site_dir: PathBuf,
    pub graph: PlaceGraph,
}

pub(crate) fn load_site(site: &str) -> Result<SiteRuntime> {
    let site_dir = session::site_dir(site);
    let graph = PlaceGraph::from_dir(&site_dir)?;
    Ok(SiteRuntime { site_dir, graph })
}

pub(crate) async fn attach(site: &str) -> Result<Session> {
    let site_dir = session::site_dir(site);
    if stencil_browser::daemon::live_socket(&site_dir)?.is_none() {
        session::preflight(&site_dir).await?;
    }
    let site_dir_for_spawn = site_dir.clone();
    Session::attach(&site_dir, move || {
        session::spawn_subprocess(&site_dir_for_spawn)
    })
    .await
}
