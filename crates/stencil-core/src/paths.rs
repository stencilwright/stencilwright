//! Filesystem layout for per-site stencilwright state.

use std::path::PathBuf;

/// Default root for all stencilwright site mappings.
pub fn root_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".stencilwright")
}

/// Resolve a site name to `~/.stencilwright/<site>/`.
pub fn site_dir(site: &str) -> PathBuf {
    root_dir().join(site)
}
